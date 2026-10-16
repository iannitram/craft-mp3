//! The hybrid filterbank's second stage: a windowed MDCT on each subband,
//! followed by the alias-reduction butterflies (ISO/IEC 11172-3, 2.4.3.4).
//!
//! Each granule turns 18 time slots of 32 subband samples into 576
//! frequency lines, in the order the bitstream carries them for long
//! blocks: subband 0's 18 lines first, then subband 1's, and so on.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::filterbank::SUBBANDS;

/// Subband samples per subband in one granule.
pub const SLOTS: usize = 18;

/// Frequency lines in one granule.
pub const GRANULE_LINES: usize = SUBBANDS * SLOTS;

const LONG: usize = 2 * SLOTS;

/// Alias-reduction coefficients `c[i]` (Table B.9).
const ALIAS_C: [f64; 8] = [
    -0.6, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
];

struct Tables {
    /// Sine window times the MDCT kernel and the forward scale factor,
    /// `[k * 36 + n]`.
    kernel: Box<[f32; SLOTS * LONG]>,
    cs: [f32; 8],
    ca: [f32; 8],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut kernel = Box::new([0.0; SLOTS * LONG]);
        for k in 0..SLOTS {
            for n in 0..LONG {
                let window = (PI / LONG as f64 * (n as f64 + 0.5)).sin();
                let basis =
                    (PI / (2 * LONG) as f64 * (2 * n + 1 + SLOTS) as f64 * (2 * k + 1) as f64)
                        .cos();
                // The decoder's IMDCT is unscaled, so the forward transform
                // carries the whole 2/N normalisation.
                kernel[k * LONG + n] = (window * basis * 2.0 / SLOTS as f64) as f32;
            }
        }
        let mut cs = [0.0; 8];
        let mut ca = [0.0; 8];
        for (i, &c) in ALIAS_C.iter().enumerate() {
            let norm = (1.0 + c * c).sqrt();
            cs[i] = (1.0 / norm) as f32;
            ca[i] = (c / norm) as f32;
        }
        Tables { kernel, cs, ca }
    })
}

/// Streaming MDCT for one channel. Each call overlaps the new granule with
/// the previous one.
#[derive(Clone)]
pub struct Mdct {
    prev: [[f32; SLOTS]; SUBBANDS],
}

impl Default for Mdct {
    fn default() -> Self {
        Self::new()
    }
}

impl Mdct {
    pub fn new() -> Self {
        Self {
            prev: [[0.0; SLOTS]; SUBBANDS],
        }
    }

    /// Transforms one granule. `slots[t][sb]` is subband `sb` at time slot
    /// `t`, as produced by [`crate::filterbank::AnalysisFilterbank`].
    pub fn process(&mut self, slots: &[[f32; SUBBANDS]; SLOTS], out: &mut [f32; GRANULE_LINES]) {
        let t = tables();

        for sb in 0..SUBBANDS {
            let mut cur = [0.0f32; SLOTS];
            for (slot, cur) in cur.iter_mut().enumerate() {
                // Undo the spectral inversion of odd subbands, which the
                // decoder reapplies after its IMDCT.
                let invert = sb % 2 == 1 && slot % 2 == 1;
                *cur = if invert {
                    -slots[slot][sb]
                } else {
                    slots[slot][sb]
                };
            }

            let mut block = [0.0f32; LONG];
            block[..SLOTS].copy_from_slice(&self.prev[sb]);
            block[SLOTS..].copy_from_slice(&cur);
            self.prev[sb] = cur;

            let lines = &mut out[sb * SLOTS..(sb + 1) * SLOTS];
            for (k, line) in lines.iter_mut().enumerate() {
                let row = &t.kernel[k * LONG..(k + 1) * LONG];
                *line = row.iter().zip(&block).map(|(a, b)| a * b).sum();
            }
        }

        // Alias reduction: the transpose of the decoder's butterflies.
        for sb in 1..SUBBANDS {
            for i in 0..8 {
                let lower = sb * SLOTS - 1 - i;
                let upper = sb * SLOTS + i;
                let (a, b) = (out[lower], out[upper]);
                out[lower] = a * t.cs[i] + b * t.ca[i];
                out[upper] = b * t.cs[i] - a * t.ca[i];
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The decoder's side, written from the standard: alias butterflies,
    /// IMDCT, windowing, overlap-add and frequency inversion.
    pub(crate) struct Inverse {
        overlap: [[f64; SLOTS]; SUBBANDS],
    }

    impl Inverse {
        pub(crate) fn new() -> Self {
            Self {
                overlap: [[0.0; SLOTS]; SUBBANDS],
            }
        }

        pub(crate) fn process(&mut self, lines: &[f32; GRANULE_LINES]) -> [[f32; SUBBANDS]; SLOTS] {
            let mut xr: Vec<f64> = lines.iter().map(|&x| f64::from(x)).collect();
            for sb in 1..SUBBANDS {
                for (i, &c) in ALIAS_C.iter().enumerate() {
                    let norm = (1.0 + c * c).sqrt();
                    let (cs, ca) = (1.0 / norm, c / norm);
                    let lower = sb * SLOTS - 1 - i;
                    let upper = sb * SLOTS + i;
                    let (a, b) = (xr[lower], xr[upper]);
                    xr[lower] = a * cs - b * ca;
                    xr[upper] = b * cs + a * ca;
                }
            }

            let mut slots = [[0.0f32; SUBBANDS]; SLOTS];
            for sb in 0..SUBBANDS {
                let mut y = [0.0f64; LONG];
                for (n, y) in y.iter_mut().enumerate() {
                    let sum: f64 = (0..SLOTS)
                        .map(|k| {
                            xr[sb * SLOTS + k]
                                * (PI / (2 * LONG) as f64
                                    * (2 * n + 1 + SLOTS) as f64
                                    * (2 * k + 1) as f64)
                                    .cos()
                        })
                        .sum();
                    *y = sum * (PI / LONG as f64 * (n as f64 + 0.5)).sin();
                }
                for slot in 0..SLOTS {
                    let mut v = y[slot] + self.overlap[sb][slot];
                    if sb % 2 == 1 && slot % 2 == 1 {
                        v = -v;
                    }
                    slots[slot][sb] = v as f32;
                    self.overlap[sb][slot] = y[slot + SLOTS];
                }
            }
            slots
        }
    }

    fn pseudo_random(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1 << 24) as f32 - 0.5
    }

    #[test]
    fn inverse_reconstructs_the_previous_granule() {
        let mut seed = 1;
        let granules: Vec<[[f32; SUBBANDS]; SLOTS]> = (0..6)
            .map(|_| {
                let mut g = [[0.0; SUBBANDS]; SLOTS];
                for slot in g.iter_mut() {
                    for s in slot.iter_mut() {
                        *s = pseudo_random(&mut seed);
                    }
                }
                g
            })
            .collect();

        let mut mdct = Mdct::new();
        let mut inverse = Inverse::new();
        let mut lines = [0.0; GRANULE_LINES];
        for (i, granule) in granules.iter().enumerate() {
            mdct.process(granule, &mut lines);
            let out = inverse.process(&lines);
            // Overlap-add completes a granule one granule later.
            if i > 0 {
                for (got, want) in out.iter().flatten().zip(granules[i - 1].iter().flatten()) {
                    assert!((got - want).abs() < 1e-5, "granule {i}: {got} vs {want}");
                }
            }
        }
    }

    #[test]
    fn a_steady_tone_concentrates_in_a_few_lines() {
        // A cosine at the centre of line 100's frequency, fed as subband samples
        // through the real filterbank.
        use crate::filterbank::AnalysisFilterbank;
        let freq = (100.0 + 0.5) / GRANULE_LINES as f64 * 0.5; // cycles per sample
        let mut fb = AnalysisFilterbank::new();
        let mut mdct = Mdct::new();
        let mut lines = [0.0; GRANULE_LINES];
        let mut n = 0usize;
        for _ in 0..8 {
            let mut slots = [[0.0; SUBBANDS]; SLOTS];
            for slot in slots.iter_mut() {
                let mut input = [0.0; SUBBANDS];
                for x in input.iter_mut() {
                    *x = (0.5 * (2.0 * PI * freq * n as f64).cos()) as f32;
                    n += 1;
                }
                fb.process(&input, slot);
            }
            mdct.process(&slots, &mut lines);
        }
        let energy: Vec<f64> = lines.iter().map(|&x| f64::from(x) * f64::from(x)).collect();
        let total: f64 = energy.iter().sum();
        let near: f64 = energy[97..104].iter().sum();
        assert!(
            near / total > 0.99,
            "only {} of the energy near line 100",
            near / total
        );
    }
}
