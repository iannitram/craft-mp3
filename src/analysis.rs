//! Measurement hooks for the benchmark harness (`examples/bench.rs`).
//!
//! Not part of the supported API: the module is `#[doc(hidden)]` and may
//! change in any release. It lets the harness judge any encoder's output
//! with this crate's own psychoacoustic model, which is the model the
//! noise-to-mask proxy is defined by.

use crate::encoder::{EncoderConfig, lowpass_cutoff_hz};
use crate::filterbank::{AnalysisFilterbank, SUBBANDS};
use crate::layout::Layout;
use crate::mdct::{BlockType, GRANULE_LINES, Mdct, SLOTS};
use crate::psycho::Psycho;

/// The lowpass, in Hz, this crate applies for `config`.
pub fn lowpass_hz(config: &EncoderConfig) -> f32 {
    lowpass_cutoff_hz(config)
}

/// Long-block MDCT spectra of a mono signal, one granule per entry.
fn spectra(signal: &[f32]) -> Vec<[f32; GRANULE_LINES]> {
    let mut filterbank = AnalysisFilterbank::new();
    let mut mdct = Mdct::new();
    let granules = signal.len().div_ceil(GRANULE_LINES);
    (0..granules)
        .map(|gr| {
            let mut slots = [[0.0f32; SUBBANDS]; SLOTS];
            for (t, slot) in slots.iter_mut().enumerate() {
                let start = (gr * SLOTS + t) * SUBBANDS;
                let mut input = [0.0f32; SUBBANDS];
                for (i, v) in input.iter_mut().enumerate() {
                    *v = signal.get(start + i).copied().unwrap_or(0.0);
                }
                filterbank.process(&input, slot);
            }
            let mut xr = [0.0f32; GRANULE_LINES];
            mdct.process(&slots, BlockType::Normal, &mut xr);
            xr
        })
        .collect()
}

/// Mean over granules and scale-factor bands of the error energy over this
/// crate's masking threshold for the original, in dB. Both signals are mono
/// and already aligned. Negative means the error sits under the threshold.
///
/// This is our model judging, so it compares encoders but says nothing
/// absolute about audibility.
pub fn noise_to_mask_db(sample_rate: u32, original: &[f32], decoded: &[f32]) -> f64 {
    let n = original.len().min(decoded.len());
    let original = &original[..n];
    let error: Vec<f32> = original.iter().zip(decoded).map(|(a, b)| b - a).collect();
    let layout = Layout::new(sample_rate);
    let psycho = Psycho::new(&layout);
    let (xo, xe) = (spectra(original), spectra(&error));
    let parts = layout.partitions(BlockType::Normal);
    let (mut sum, mut count) = (0.0f64, 0usize);
    for (o, e) in xo.iter().zip(&xe) {
        let thr = psycho.thresholds(&layout, o, BlockType::Normal, 0.0);
        for (p, t) in parts.iter().zip(&thr) {
            let energy: f64 = e[p.lines.clone()].iter().map(|x| f64::from(x * x)).sum();
            sum += 10.0 * (energy.max(1e-20) / f64::from(*t).max(1e-20)).log10();
            count += 1;
        }
    }
    if count == 0 { 0.0 } else { sum / count as f64 }
}
