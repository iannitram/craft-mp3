//! Simple, reproducible quality measures: proxies, not PEAQ. All work on one
//! channel of f32 samples with full scale at 1.0.

use std::f64::consts::PI;

/// Highest segmental SNR any frame (or the whole file) reports, in dB.
pub const SNR_CAP_DB: f64 = 100.0;
/// Frames whose input is quieter than this (dBFS, RMS) are not scored.
pub const QUIET_DBFS: f64 = -60.0;

/// Segmental SNR in dB: the mean over 20 ms frames of
/// `10 log10(signal / error)` per frame, each capped at [`SNR_CAP_DB`].
/// Frames of the input below [`QUIET_DBFS`] are skipped. `None` when every
/// frame is skipped.
pub fn segmental_snr_db(original: &[f32], decoded: &[f32], sample_rate: u32) -> Option<f64> {
    let frame = (f64::from(sample_rate) * 0.02).round() as usize;
    let n = original.len().min(decoded.len());
    let quiet = 10f64.powf(QUIET_DBFS / 10.0);
    let (mut sum, mut count) = (0.0, 0usize);
    for start in (0..n.saturating_sub(frame - 1)).step_by(frame) {
        let o = &original[start..start + frame];
        let d = &decoded[start..start + frame];
        let signal: f64 = o.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / frame as f64;
        if signal < quiet {
            continue;
        }
        let noise: f64 = o
            .iter()
            .zip(d)
            .map(|(&a, &b)| (f64::from(b) - f64::from(a)).powi(2))
            .sum::<f64>()
            / frame as f64;
        sum += (10.0 * (signal / noise.max(1e-30)).log10()).min(SNR_CAP_DB);
        count += 1;
    }
    (count > 0).then(|| sum / count as f64)
}

const FFT_SIZE: usize = 2048;
/// Power below this (a full-scale sine reads 1.0) counts as
/// silence: -120 dB.
const POWER_FLOOR: f64 = 1e-12;

fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                (cr, ci) = (cr * wr - ci * wi, cr * wi + ci * wr);
            }
        }
        len <<= 1;
    }
}

/// Power spectra (bins 0..=1024) of Hann-windowed frames, hop 1024.
fn power_spectra(x: &[f32]) -> Vec<Vec<f64>> {
    let window: Vec<f64> = (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / FFT_SIZE as f64).cos())
        .collect();
    let norm = window.iter().sum::<f64>().powi(2) / 4.0; // a full-scale sine reads 1.0
    (0..x.len().saturating_sub(FFT_SIZE - 1))
        .step_by(FFT_SIZE / 2)
        .map(|s| {
            let mut re: Vec<f64> = (0..FFT_SIZE)
                .map(|i| f64::from(x[s + i]) * window[i])
                .collect();
            let mut im = vec![0.0; FFT_SIZE];
            fft(&mut re, &mut im);
            (0..=FFT_SIZE / 2)
                .map(|k| (re[k] * re[k] + im[k] * im[k]) / norm)
                .collect()
        })
        .collect()
}

/// Log-spectral distance in dB below `cutoff_hz`: per frame the RMS over
/// bins of the dB difference of the two power spectra (floored at -120 dB),
/// averaged over frames. 0 for identical signals.
pub fn log_spectral_distance_db(
    original: &[f32],
    decoded: &[f32],
    sample_rate: u32,
    cutoff_hz: f64,
) -> f64 {
    let n = original.len().min(decoded.len());
    let (so, sd) = (power_spectra(&original[..n]), power_spectra(&decoded[..n]));
    let top = ((cutoff_hz / f64::from(sample_rate) * FFT_SIZE as f64) as usize).min(FFT_SIZE / 2);
    if so.is_empty() || top == 0 {
        return 0.0;
    }
    let total: f64 = so
        .iter()
        .zip(&sd)
        .map(|(a, b)| {
            let sq: f64 = (1..=top)
                .map(|k| {
                    let d = 10.0 * (a[k].max(POWER_FLOOR) / b[k].max(POWER_FLOOR)).log10();
                    d * d
                })
                .sum();
            (sq / top as f64).sqrt()
        })
        .sum();
    total / so.len() as f64
}

/// Bands used by [`effective_bandwidth_hz`].
const BANDWIDTH_BANDS: usize = 40;

/// Effective bandwidth in Hz: the upper edge of the highest of 40 equal
/// bands whose mean power in the decoded signal is within 60 dB of the
/// input's in that band. Bands where the input is itself silent are ignored.
pub fn effective_bandwidth_hz(original: &[f32], decoded: &[f32], sample_rate: u32) -> f64 {
    let n = original.len().min(decoded.len());
    let band_power = |x: &[f32]| {
        let spectra = power_spectra(&x[..n]);
        let mut power = vec![0.0f64; BANDWIDTH_BANDS];
        let per = (FFT_SIZE / 2) / BANDWIDTH_BANDS;
        for s in &spectra {
            for (b, p) in power.iter_mut().enumerate() {
                *p += s[b * per..(b + 1) * per].iter().sum::<f64>() / per as f64;
            }
        }
        let frames = spectra.len().max(1) as f64;
        power.iter_mut().for_each(|p| *p /= frames);
        power
    };
    let (po, pd) = (band_power(original), band_power(decoded));
    let mut top = 0;
    for b in 0..BANDWIDTH_BANDS {
        if po[b] > POWER_FLOOR && pd[b] >= po[b] * 1e-6 {
            top = b + 1;
        }
    }
    top as f64 * f64::from(sample_rate) / 2.0 / BANDWIDTH_BANDS as f64
}

/// The lag in `stated - radius..=stated + radius` at which `decoded` best
/// matches `original`, by cross-correlation over up to 32768 samples from
/// the middle of the input. `decoded[i + lag]` corresponds to `original[i]`.
pub fn find_delay(original: &[f32], decoded: &[f32], stated: usize, radius: usize) -> usize {
    let window = original.len().min(32_768);
    let start = (original.len() - window) / 2;
    let reference = &original[start..start + window];
    let mut best = (stated, f64::NEG_INFINITY);
    for lag in stated.saturating_sub(radius)..=stated + radius {
        let from = start + lag;
        if from + window > decoded.len() {
            break;
        }
        let r: f64 = reference
            .iter()
            .zip(&decoded[from..from + window])
            .map(|(&a, &b)| f64::from(a) * f64::from(b))
            .sum();
        if r > best.1 {
            best = (lag, r);
        }
    }
    best.0
}

/// `decoded` shifted back by `lag` and cut or zero-padded to `len` samples.
pub fn align(decoded: &[f32], lag: usize, len: usize) -> Vec<f32> {
    let mut out: Vec<f32> = decoded.iter().skip(lag).take(len).copied().collect();
    out.resize(len, 0.0);
    out
}
