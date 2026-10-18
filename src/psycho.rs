//! A small psychoacoustic model: how much quantization noise each band of
//! a granule can hide.
//!
//! The allowed noise in a band is the larger of
//! - the absolute threshold of hearing (Terhardt's approximation), with a
//!   full-scale sine taken as 96 dB SPL, and
//! - the masking threshold: band energies spread across neighbouring bands
//!   on the Bark scale (Schroeder's spreading function), lowered by an
//!   offset that depends on how tonal the band is (Johnston): noise masks
//!   noise 5.5 dB below it, but noise must stay 14.5 + z dB below a tone
//!   at z Bark. Tonality comes from the band's spectral flatness.
//!
//! Long blocks are modelled over their 22 bands; short blocks over the 13
//! bands of each window separately. Energies are in the MDCT domain, where
//! a full-scale sine carries an energy of about 1.0 per granule (long) or
//! per window (short).

use crate::layout::{Layout, Partition};
use crate::mdct::{BlockType, GRANULE_LINES};

/// Masking offset for a noise-like masker, in dB.
const NOISE_OFFSET_DB: f32 = 5.5;

/// Spectral flatness, in dB, at which a band counts as fully tonal.
const TONAL_FLATNESS_DB: f32 = -40.0;

/// Shape of the bit-pressure relaxation over Bark: higher values spare
/// more of the low bands.
const PRESSURE_EXPONENT: f64 = 3.0;

/// Level of a full-scale sine in dB SPL.
const FULL_SCALE_DB_SPL: f64 = 96.0;

/// Absolute threshold of hearing in dB SPL (Terhardt).
fn ath_db(freq_hz: f64) -> f64 {
    let khz = (freq_hz / 1000.0).max(0.01);
    3.64 * khz.powf(-0.8) - 6.5 * (-0.6 * (khz - 3.3).powi(2)).exp() + 1e-3 * khz.powi(4)
}

/// Critical-band rate in Bark (Zwicker).
fn bark(freq_hz: f64) -> f64 {
    13.0 * (0.00076 * freq_hz).atan() + 3.5 * (freq_hz / 7500.0).powi(2).atan()
}

/// Schroeder's spreading function, in linear power, for a masker `dz`
/// Bark below (positive) or above (negative) the masked band.
fn spreading(dz: f64) -> f64 {
    let x = dz + 0.474;
    let db = 15.81 + 7.5 * x - 17.5 * (1.0 + x * x).sqrt();
    10f64.powf(db / 10.0)
}

/// The model for one spectrum of `bands` bands (a long granule or one
/// short window).
struct Model {
    /// Threshold of hearing per band, as an energy.
    ath: Vec<f32>,
    /// `spread[masked * bands + masker]`.
    spread: Vec<f32>,
    /// Masking offset for a tonal band, in dB.
    tonal_offset_db: Vec<f32>,
    /// How strongly each band gives way under bit pressure, from 0 (lowest
    /// band) to 1 (highest).
    pressure_weight: Vec<f32>,
}

impl Model {
    /// `parts` are the partitions of one spectrum, lowest band first.
    fn new(parts: &[&Partition]) -> Self {
        let bands = parts.len();
        let ath = parts
            .iter()
            .map(|p| {
                // The quietest frequency in the band sets its threshold.
                let (lo, hi) = (p.lowest_hz, p.highest_hz);
                let steps = 16;
                let db = (0..=steps)
                    .map(|i| ath_db(lo + (hi - lo) * f64::from(i) / f64::from(steps)))
                    .fold(f64::INFINITY, f64::min);
                10f64.powf((db - FULL_SCALE_DB_SPL) / 10.0) as f32
            })
            .collect();
        let centre: Vec<f64> = parts.iter().map(|p| bark(p.centre_hz)).collect();
        let mut spread = vec![0.0; bands * bands];
        for masked in 0..bands {
            for masker in 0..bands {
                spread[masked * bands + masker] = spreading(centre[masked] - centre[masker]) as f32;
            }
        }
        let tonal_offset_db = centre.iter().map(|z| (14.5 + z) as f32).collect();
        let pressure_weight = centre
            .iter()
            .map(|z| ((z / 24.0).powf(PRESSURE_EXPONENT)).min(1.0) as f32)
            .collect();
        Self {
            ath,
            spread,
            tonal_offset_db,
            pressure_weight,
        }
    }

    /// Allowed noise per band for one spectrum; `lines[b]` are band `b`'s values.
    fn thresholds(&self, lines: &[&[f32]]) -> Vec<f32> {
        let bands = lines.len();
        let energy: Vec<f32> = lines
            .iter()
            .map(|l| l.iter().map(|x| x * x).sum())
            .collect();
        let tonality = tonality(lines);
        (0..bands)
            .map(|band| {
                let row = &self.spread[band * bands..(band + 1) * bands];
                let spread: f32 = row.iter().zip(&energy).map(|(s, e)| s * e).sum();
                let a = tonality[band];
                let offset_db = a * self.tonal_offset_db[band] + (1.0 - a) * NOISE_OFFSET_DB;
                (spread * 10f32.powf(-offset_db / 10.0)).max(self.ath[band])
            })
            .collect()
    }
}

/// How tonal each band is, from 0 (noise) to 1 (a pure tone), judged by the
/// spectral flatness of the band widened with its neighbours to at least 8
/// lines.
fn tonality(lines: &[&[f32]]) -> Vec<f32> {
    let bands = lines.len();
    (0..bands)
        .map(|band| {
            let (mut lo, mut hi) = (band, band + 1);
            let width = |lo: usize, hi: usize| lines[lo..hi].iter().map(|l| l.len()).sum::<usize>();
            while width(lo, hi) < 8 && (lo > 0 || hi < bands) {
                lo = lo.saturating_sub(1);
                hi = (hi + 1).min(bands);
            }
            let values = lines[lo..hi].iter().flat_map(|l| l.iter());
            let n = width(lo, hi) as f32;
            let arithmetic = values.clone().map(|x| x * x).sum::<f32>() / n;
            if arithmetic <= 0.0 {
                return 0.0;
            }
            let floor = arithmetic * 1e-6;
            let log_geometric = values.map(|x| (x * x + floor).ln()).sum::<f32>() / n;
            let flatness_db = 10.0 * (log_geometric.exp() / arithmetic).log10();
            (flatness_db / TONAL_FLATNESS_DB).clamp(0.0, 1.0)
        })
        .collect()
}

pub(crate) struct Psycho {
    long: Model,
    short: Model,
}

impl Psycho {
    pub(crate) fn new(layout: &Layout) -> Self {
        let long: Vec<&Partition> = layout.partitions(BlockType::Normal).iter().collect();
        let short: Vec<&Partition> = layout
            .partitions(BlockType::Short)
            .iter()
            .filter(|p| p.window == Some(0))
            .collect();
        Self {
            long: Model::new(&long),
            short: Model::new(&short),
        }
    }

    /// Allowed noise energy for each partition of a granule (see
    /// [`Layout::partitions`]); `xr` is in bitstream order.
    ///
    /// `pressure_db` relaxes the thresholds when bits are short: by up to
    /// that many dB in the top band and progressively less below, so the
    /// low bands, where noise is most audible, stay clean longest.
    pub(crate) fn thresholds(
        &self,
        layout: &Layout,
        xr: &[f32; GRANULE_LINES],
        block: BlockType,
        pressure_db: f32,
    ) -> Vec<f32> {
        let mut thr = self.unrelaxed(layout, xr, block);
        let model = if block == BlockType::Short {
            &self.short
        } else {
            &self.long
        };
        for (t, p) in thr.iter_mut().zip(layout.partitions(block)) {
            *t *= 10f32.powf(pressure_db * model.pressure_weight[p.band] / 10.0);
        }
        thr
    }

    fn unrelaxed(&self, layout: &Layout, xr: &[f32; GRANULE_LINES], block: BlockType) -> Vec<f32> {
        let parts = layout.partitions(block);
        if block != BlockType::Short {
            let lines: Vec<&[f32]> = parts.iter().map(|p| &xr[p.lines.clone()]).collect();
            return self.long.thresholds(&lines);
        }
        let mut out = vec![0.0; parts.len()];
        for window in 0..3 {
            let indices: Vec<usize> = (0..parts.len())
                .filter(|&i| parts[i].window == Some(window))
                .collect();
            let lines: Vec<&[f32]> = indices
                .iter()
                .map(|&i| &xr[parts[i].lines.clone()])
                .collect();
            for (&i, thr) in indices.iter().zip(self.short.thresholds(&lines)) {
                out[i] = thr;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hearing_is_most_sensitive_around_3_to_4_khz() {
        let best = (100..20_000)
            .step_by(50)
            .min_by(|&a, &b| ath_db(f64::from(a)).total_cmp(&ath_db(f64::from(b))));
        assert!((3000..4500).contains(&best.unwrap()));
        assert!(ath_db(50.0) > 30.0);
        assert!(ath_db(18_000.0) > 60.0);
    }

    #[test]
    fn spreading_is_0_db_in_band_and_falls_off_asymmetrically() {
        assert!((spreading(0.0) - 1.0).abs() < 0.01);
        // Masking reaches further up in frequency than down.
        assert!(spreading(2.0) > spreading(-2.0));
        assert!(spreading(5.0) < 1e-2);
    }

    fn setup() -> (Layout, Psycho) {
        let layout = Layout::new(44_100);
        let psycho = Psycho::new(&layout);
        (layout, psycho)
    }

    #[test]
    fn silence_is_held_to_the_threshold_of_hearing() {
        let (layout, p) = setup();
        assert_eq!(
            p.thresholds(&layout, &[0.0; GRANULE_LINES], BlockType::Normal, 0.0),
            p.long.ath
        );
        let short = p.thresholds(&layout, &[0.0; GRANULE_LINES], BlockType::Short, 0.0);
        for (i, part) in layout.partitions(BlockType::Short).iter().enumerate() {
            assert_eq!(short[i], p.short.ath[part.band]);
        }
    }

    #[test]
    fn a_loud_band_raises_its_own_and_neighbouring_thresholds() {
        let (layout, p) = setup();
        let parts = layout.partitions(BlockType::Normal);
        let band = 10;
        let mut xr = [0.0; GRANULE_LINES];
        for x in &mut xr[parts[band].lines.clone()] {
            *x = 0.1;
        }
        let energy: f32 = xr.iter().map(|x| x * x).sum();
        let thr = p.thresholds(&layout, &xr, BlockType::Normal, 0.0);
        // A flat band is noise-like: masked 5.5 dB down.
        let expected = energy * 10f32.powf(-NOISE_OFFSET_DB / 10.0);
        assert!(
            (thr[band] / expected - 1.0).abs() < 0.05,
            "{} vs {expected}",
            thr[band]
        );
        assert!(thr[band + 1] > p.long.ath[band + 1] && thr[band + 1] < thr[band]);
        assert!(thr[band - 1] > p.long.ath[band - 1] && thr[band - 1] < thr[band]);
        // Far away, only the threshold of hearing is left.
        assert_eq!(thr[0], p.long.ath[0]);
    }

    #[test]
    fn tones_get_a_lower_threshold_than_noise_of_the_same_energy() {
        let (layout, p) = setup();
        let band = 12;
        let range = layout.partitions(BlockType::Normal)[band].lines.clone();

        let mut noise = [0.0f32; GRANULE_LINES];
        let mut seed = 1u32;
        for x in &mut noise[range.clone()] {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *x = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
        }
        let energy: f32 = noise.iter().map(|x| x * x).sum();
        let mut tone = [0.0f32; GRANULE_LINES];
        tone[range.start + 5] = energy.sqrt();

        let tonality_of = |xr: &[f32; GRANULE_LINES]| -> Vec<f32> {
            let parts = layout.partitions(BlockType::Normal);
            let l: Vec<&[f32]> = parts.iter().map(|p| &xr[p.lines.clone()]).collect();
            tonality(&l)
        };
        assert!(tonality_of(&noise)[band] < 0.3);
        assert!(tonality_of(&tone)[band] > 0.9);
        let t_noise = p.thresholds(&layout, &noise, BlockType::Normal, 0.0)[band];
        let t_tone = p.thresholds(&layout, &tone, BlockType::Normal, 0.0)[band];
        assert!(t_tone < t_noise / 10.0, "tone {t_tone} vs noise {t_noise}");
    }

    #[test]
    fn short_windows_are_modelled_independently() {
        let (layout, p) = setup();
        let parts = layout.partitions(BlockType::Short);
        // Loud noise in window 2 only (an attack late in the granule).
        let mut xr = [0.0f32; GRANULE_LINES];
        let mut seed = 3u32;
        for part in parts.iter().filter(|p| p.window == Some(2)) {
            for x in &mut xr[part.lines.clone()] {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *x = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
            }
        }
        let thr = p.thresholds(&layout, &xr, BlockType::Short, 0.0);
        for (i, part) in parts.iter().enumerate() {
            if part.window == Some(2) {
                assert!(thr[i] > p.short.ath[part.band] * 10.0);
            } else {
                assert_eq!(thr[i], p.short.ath[part.band], "quiet windows keep the ATH");
            }
        }
    }

    #[test]
    fn bit_pressure_relaxes_high_bands_more_than_low_ones() {
        let (layout, p) = setup();
        let mut xr = [0.0f32; GRANULE_LINES];
        for (i, x) in xr.iter_mut().enumerate() {
            *x = 0.01 * ((i * 7919) % 13) as f32 / 13.0;
        }
        for block in [BlockType::Normal, BlockType::Short] {
            let base = p.thresholds(&layout, &xr, block, 0.0);
            let relaxed = p.thresholds(&layout, &xr, block, 20.0);
            let gain_db: Vec<f32> = base
                .iter()
                .zip(&relaxed)
                .map(|(b, r)| 10.0 * (r / b).log10())
                .collect();
            let parts = layout.partitions(block);
            let first = gain_db[0];
            let last = gain_db[parts.len() - 1];
            assert!(first < 0.5, "{block:?}: lowest band relaxed by {first} dB");
            assert!(
                last > 10.0 && last <= 20.0 + 1e-3,
                "{block:?}: top band relaxed by {last} dB"
            );
            // Never tighter, and rising with frequency.
            for pair in parts.windows(2).zip(gain_db.windows(2)) {
                let (bands, g) = pair;
                assert!(g[0] >= 0.0);
                if bands[1].band > bands[0].band {
                    assert!(g[1] >= g[0] - 1e-4);
                }
            }
        }
    }
}
