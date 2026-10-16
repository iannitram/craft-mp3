//! The hybrid filterbank's second stage: a windowed MDCT on each subband,
//! followed by the alias-reduction butterflies (ISO/IEC 11172-3, 2.4.3.4).
//!
//! Each granule turns 18 time slots of 32 subband samples into 576
//! frequency lines in subband order: subband 0's 18 lines first, then
//! subband 1's, and so on. Long blocks (normal, start and stop) give 18
//! lines per subband; short blocks give three windows of 6 lines each,
//! interleaved as `3 * k + window`, which is how the decoder lays them out
//! before its IMDCT.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::filterbank::SUBBANDS;

/// Subband samples per subband in one granule.
pub const SLOTS: usize = 18;

/// Frequency lines in one granule.
pub const GRANULE_LINES: usize = SUBBANDS * SLOTS;

const LONG: usize = 2 * SLOTS;
const SHORT: usize = 12;
const SHORT_LINES: usize = SHORT / 2;

/// The window shape of a granule (`block_type` in the side information).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockType {
    #[default]
    Normal = 0,
    /// Long window that switches to short at its end.
    Start = 1,
    /// Three short windows, for transients.
    Short = 2,
    /// Long window that switches back from short at its start.
    Stop = 3,
}

impl BlockType {
    /// The window shape for a granule, given the previous granule's shape
    /// and whether this granule and the next one contain an attack.
    pub fn next(prev: BlockType, attack_here: bool, attack_next: bool) -> BlockType {
        let prev_short = prev == BlockType::Short;
        match (attack_here, attack_next) {
            (true, _) => BlockType::Short,
            (false, true) if prev_short => BlockType::Short,
            (false, true) => BlockType::Start,
            (false, false) if prev_short => BlockType::Stop,
            (false, false) => BlockType::Normal,
        }
    }
}

/// Alias-reduction coefficients `c[i]` (Table B.9).
const ALIAS_C: [f64; 8] = [
    -0.6, -0.535, -0.33, -0.185, -0.095, -0.041, -0.0142, -0.0037,
];

/// The 36-point window for a long block type (2.4.3.4.10.1).
fn long_window(block: BlockType, n: usize) -> f64 {
    let long = || (PI / LONG as f64 * (n as f64 + 0.5)).sin();
    let short = |offset: usize| (PI / SHORT as f64 * ((n - offset) as f64 + 0.5)).sin();
    match block {
        BlockType::Normal | BlockType::Short => long(),
        BlockType::Start => match n {
            0..18 => long(),
            18..24 => 1.0,
            24..30 => short(18),
            _ => 0.0,
        },
        BlockType::Stop => match n {
            0..6 => 0.0,
            6..12 => short(6),
            12..18 => 1.0,
            _ => long(),
        },
    }
}

struct Tables {
    /// Window times MDCT kernel times the forward 2/N scale, `[k * 36 + n]`,
    /// for normal, start and stop blocks.
    long: [Box<[f32; SLOTS * LONG]>; 3],
    /// The same for one short window, `[k * 12 + n]`.
    short: [f32; SHORT_LINES * SHORT],
    cs: [f32; 8],
    ca: [f32; 8],
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        // The decoder's IMDCT is unscaled, so the forward transform carries
        // the whole 2/N normalisation.
        let long = [BlockType::Normal, BlockType::Start, BlockType::Stop].map(|block| {
            let mut kernel = Box::new([0.0; SLOTS * LONG]);
            for k in 0..SLOTS {
                for n in 0..LONG {
                    let basis =
                        (PI / (2 * LONG) as f64 * (2 * n + 1 + SLOTS) as f64 * (2 * k + 1) as f64)
                            .cos();
                    kernel[k * LONG + n] =
                        (long_window(block, n) * basis * 2.0 / SLOTS as f64) as f32;
                }
            }
            kernel
        });
        let mut short = [0.0; SHORT_LINES * SHORT];
        for k in 0..SHORT_LINES {
            for n in 0..SHORT {
                let window = (PI / SHORT as f64 * (n as f64 + 0.5)).sin();
                let basis = (PI / (2 * SHORT) as f64
                    * (2 * n + 1 + SHORT_LINES) as f64
                    * (2 * k + 1) as f64)
                    .cos();
                short[k * SHORT + n] = (window * basis * 2.0 / SHORT_LINES as f64) as f32;
            }
        }
        let mut cs = [0.0; 8];
        let mut ca = [0.0; 8];
        for (i, &c) in ALIAS_C.iter().enumerate() {
            let norm = (1.0 + c * c).sqrt();
            cs[i] = (1.0 / norm) as f32;
            ca[i] = (c / norm) as f32;
        }
        Tables {
            long,
            short,
            cs,
            ca,
        }
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

    /// Transforms one granule with the given window shape. `slots[t][sb]` is
    /// subband `sb` at time slot `t`, as produced by
    /// [`crate::filterbank::AnalysisFilterbank`].
    pub fn process(
        &mut self,
        slots: &[[f32; SUBBANDS]; SLOTS],
        block: BlockType,
        out: &mut [f32; GRANULE_LINES],
    ) {
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

            let mut z = [0.0f32; LONG];
            z[..SLOTS].copy_from_slice(&self.prev[sb]);
            z[SLOTS..].copy_from_slice(&cur);
            self.prev[sb] = cur;

            let lines = &mut out[sb * SLOTS..(sb + 1) * SLOTS];
            match block {
                BlockType::Short => {
                    // Three half-overlapping 12-point windows starting at 6, 12, 18.
                    for window in 0..3 {
                        let input = &z[6 + 6 * window..6 + 6 * window + SHORT];
                        for k in 0..SHORT_LINES {
                            let row = &t.short[k * SHORT..(k + 1) * SHORT];
                            lines[3 * k + window] = row.iter().zip(input).map(|(a, b)| a * b).sum();
                        }
                    }
                }
                _ => {
                    let kernel = match block {
                        BlockType::Start => &t.long[1],
                        BlockType::Stop => &t.long[2],
                        _ => &t.long[0],
                    };
                    for (k, line) in lines.iter_mut().enumerate() {
                        let row = &kernel[k * LONG..(k + 1) * LONG];
                        *line = row.iter().zip(&z).map(|(a, b)| a * b).sum();
                    }
                }
            }
        }

        if block == BlockType::Short {
            return;
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

        pub(crate) fn process(
            &mut self,
            lines: &[f32; GRANULE_LINES],
            block: BlockType,
        ) -> [[f32; SUBBANDS]; SLOTS] {
            let mut xr: Vec<f64> = lines.iter().map(|&x| f64::from(x)).collect();
            if block != BlockType::Short {
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
            }

            let mut slots = [[0.0f32; SUBBANDS]; SLOTS];
            for sb in 0..SUBBANDS {
                let x = &xr[sb * SLOTS..(sb + 1) * SLOTS];
                let mut y = [0.0f64; LONG];
                if block == BlockType::Short {
                    for window in 0..3 {
                        for n in 0..SHORT {
                            let sum: f64 = (0..SHORT_LINES)
                                .map(|k| {
                                    x[3 * k + window]
                                        * (PI / (2 * SHORT) as f64
                                            * (2 * n + 1 + SHORT_LINES) as f64
                                            * (2 * k + 1) as f64)
                                            .cos()
                                })
                                .sum();
                            let w = (PI / SHORT as f64 * (n as f64 + 0.5)).sin();
                            y[6 + 6 * window + n] += sum * w;
                        }
                    }
                } else {
                    for (n, y) in y.iter_mut().enumerate() {
                        let sum: f64 = (0..SLOTS)
                            .map(|k| {
                                x[k] * (PI / (2 * LONG) as f64
                                    * (2 * n + 1 + SLOTS) as f64
                                    * (2 * k + 1) as f64)
                                    .cos()
                            })
                            .sum();
                        *y = sum * long_window(block, n);
                    }
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
    fn block_type_sequence_follows_the_standard() {
        use BlockType::*;
        let attacks = [false, false, true, false, false, true, true, false, false];
        let mut prev = Normal;
        let mut seq = Vec::new();
        for (i, &here) in attacks.iter().enumerate() {
            let next = attacks.get(i + 1).copied().unwrap_or(false);
            prev = BlockType::next(prev, here, next);
            seq.push(prev);
        }
        assert_eq!(
            seq,
            [
                Normal, Start, Short, Stop, Start, Short, Short, Stop, Normal
            ]
        );
        // Every short block is entered through Start (or Short) and left through Stop (or Short).
        for pair in seq.windows(2) {
            if pair[1] == Short {
                assert!(matches!(pair[0], Start | Short));
            }
            if pair[0] == Short {
                assert!(matches!(pair[1], Stop | Short));
            }
        }
    }

    #[test]
    fn inverse_reconstructs_across_window_switches() {
        use BlockType::*;
        let blocks = [
            Normal, Start, Short, Short, Stop, Normal, Start, Short, Stop, Normal,
        ];
        let mut seed = 1;
        let granules: Vec<[[f32; SUBBANDS]; SLOTS]> = blocks
            .iter()
            .map(|_| {
                let mut g = [[0.0; SUBBANDS]; SLOTS];
                for x in g.iter_mut().flatten() {
                    *x = pseudo_random(&mut seed);
                }
                g
            })
            .collect();

        let mut mdct = Mdct::new();
        let mut inverse = Inverse::new();
        let mut lines = [0.0; GRANULE_LINES];
        for (i, (granule, &block)) in granules.iter().zip(&blocks).enumerate() {
            mdct.process(granule, block, &mut lines);
            let out = inverse.process(&lines, block);
            // Overlap-add completes a granule one granule later.
            if i > 0 {
                for (got, want) in out.iter().flatten().zip(granules[i - 1].iter().flatten()) {
                    assert!(
                        (got - want).abs() < 1e-5,
                        "granule {i} ({block:?}): {got} vs {want}"
                    );
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
            mdct.process(&slots, BlockType::Normal, &mut lines);
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
