//! Polyphase analysis filterbank (ISO/IEC 11172-3, Annex C.1.3).
//!
//! Splits PCM into 32 equally wide frequency subbands, critically sampled:
//! every 32 input samples produce one sample in each subband.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::tables::WINDOW_D_2POW16;

/// Number of subbands.
pub const SUBBANDS: usize = 32;

const WINDOW_LEN: usize = 512;

/// Analysis window `C[i]` and the matrixing coefficients `M[i][k]`.
struct Tables {
    c: [f32; WINDOW_LEN],
    /// `M[i][k] = cos((2i + 1)(k - 16)π / 64)`, row-major `[i * 64 + k]`.
    m: Box<[f32; SUBBANDS * 64]>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut c = [0.0; WINDOW_LEN];
        for (c, &d) in c.iter_mut().zip(&WINDOW_D_2POW16) {
            *c = (f64::from(d) / f64::from(1 << 21)) as f32;
        }
        let mut m = Box::new([0.0; SUBBANDS * 64]);
        for i in 0..SUBBANDS {
            for k in 0..64 {
                let angle = (2 * i + 1) as f64 * (k as f64 - 16.0) * PI / 64.0;
                m[i * 64 + k] = angle.cos() as f32;
            }
        }
        Tables { c, m }
    })
}

/// Streaming analysis filterbank for one channel.
#[derive(Clone)]
pub struct AnalysisFilterbank {
    /// The last 512 input samples, newest first (`x[0]` is the latest).
    x: [f32; WINDOW_LEN],
}

impl Default for AnalysisFilterbank {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalysisFilterbank {
    pub fn new() -> Self {
        Self {
            x: [0.0; WINDOW_LEN],
        }
    }

    /// Consumes 32 PCM samples (oldest first, nominally in `[-1, 1)`) and
    /// writes one sample for each of the 32 subbands, lowest band first.
    pub fn process(&mut self, input: &[f32; SUBBANDS], out: &mut [f32; SUBBANDS]) {
        let t = tables();

        self.x.copy_within(0..WINDOW_LEN - SUBBANDS, SUBBANDS);
        for (x, &sample) in self.x[..SUBBANDS].iter_mut().zip(input.iter().rev()) {
            *x = sample;
        }

        // Window, then fold the 512 products into 64 partial sums.
        let mut y = [0.0f32; 64];
        for (i, y) in y.iter_mut().enumerate() {
            *y = (0..8).map(|j| t.c[i + 64 * j] * self.x[i + 64 * j]).sum();
        }

        for (i, out) in out.iter_mut().enumerate() {
            let row = &t.m[i * 64..(i + 1) * 64];
            *out = row.iter().zip(&y).map(|(m, y)| m * y).sum();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Reference polyphase synthesis (ISO/IEC 11172-3, Annex A, Figure A.2),
    /// written directly from the standard's equations to check the analysis
    /// side against.
    pub(crate) struct Synthesis {
        v: Vec<f64>,
        n: Vec<f64>,
    }

    impl Synthesis {
        pub(crate) fn new() -> Self {
            let mut n = vec![0.0; 64 * SUBBANDS];
            for i in 0..64 {
                for k in 0..SUBBANDS {
                    n[i * SUBBANDS + k] = ((16 + i) as f64 * (2 * k + 1) as f64 * PI / 64.0).cos();
                }
            }
            Self {
                v: vec![0.0; 1024],
                n,
            }
        }

        pub(crate) fn process(&mut self, s: &[f32; SUBBANDS]) -> [f32; SUBBANDS] {
            self.v.copy_within(0..1024 - 64, 64);
            for i in 0..64 {
                self.v[i] = (0..SUBBANDS)
                    .map(|k| self.n[i * SUBBANDS + k] * f64::from(s[k]))
                    .sum();
            }
            let mut u = [0.0; 512];
            for i in 0..8 {
                for j in 0..32 {
                    u[i * 64 + j] = self.v[i * 128 + j];
                    u[i * 64 + 32 + j] = self.v[i * 128 + 96 + j];
                }
            }
            let mut out = [0.0; SUBBANDS];
            for (j, out) in out.iter_mut().enumerate() {
                *out = (0..16)
                    .map(|i| u[j + 32 * i] * f64::from(WINDOW_D_2POW16[j + 32 * i]) / 65536.0)
                    .sum::<f64>() as f32;
            }
            out
        }
    }

    /// Analysis followed by synthesis delays the signal by this many samples.
    pub(crate) const ROUND_TRIP_DELAY: usize = 481;

    fn round_trip(input: &[f32]) -> Vec<f32> {
        let mut analysis = AnalysisFilterbank::new();
        let mut synthesis = Synthesis::new();
        let mut output = Vec::with_capacity(input.len());
        let mut bands = [0.0; SUBBANDS];
        for chunk in input.chunks_exact(SUBBANDS) {
            analysis.process(chunk.try_into().unwrap(), &mut bands);
            output.extend(synthesis.process(&bands));
        }
        output
    }

    fn sine(len: usize, freq_hz: f64, rate_hz: f64) -> Vec<f32> {
        (0..len)
            .map(|n| (0.5 * (2.0 * PI * freq_hz * n as f64 / rate_hz).sin()) as f32)
            .collect()
    }

    #[test]
    fn analysis_then_synthesis_reconstructs_the_input() {
        let input: Vec<f32> = sine(32 * 200, 1000.0, 44_100.0)
            .iter()
            .zip(sine(32 * 200, 7_350.0, 44_100.0))
            .map(|(a, b)| a + 0.5 * b)
            .collect();
        let output = round_trip(&input);

        let mut max_err = 0.0f32;
        for (n, &out) in output.iter().enumerate().skip(ROUND_TRIP_DELAY + 512) {
            max_err = max_err.max((out - input[n - ROUND_TRIP_DELAY]).abs());
        }
        // The MPEG filterbank is near, not exactly, perfect reconstruction.
        assert!(max_err < 1e-4, "max reconstruction error {max_err}");
    }

    #[test]
    fn a_sine_lands_in_its_own_subband() {
        let rate = 48_000.0;
        let band_width = rate / 2.0 / SUBBANDS as f64;
        for band in [0, 5, 17, 30] {
            let freq = (band as f64 + 0.5) * band_width;
            let input = sine(32 * 100, freq, rate);
            let mut analysis = AnalysisFilterbank::new();
            let mut energy = [0.0f64; SUBBANDS];
            let mut out = [0.0; SUBBANDS];
            for (i, chunk) in input.chunks_exact(SUBBANDS).enumerate() {
                analysis.process(chunk.try_into().unwrap(), &mut out);
                if i >= 20 {
                    for (e, &s) in energy.iter_mut().zip(&out) {
                        *e += f64::from(s) * f64::from(s);
                    }
                }
            }
            let total: f64 = energy.iter().sum();
            assert!(energy[band] / total > 0.999, "band {band}: {energy:?}");
        }
    }

    #[test]
    fn silence_stays_silent() {
        let mut analysis = AnalysisFilterbank::new();
        let mut out = [1.0; SUBBANDS];
        analysis.process(&[0.0; SUBBANDS], &mut out);
        assert_eq!(out, [0.0; SUBBANDS]);
    }
}
