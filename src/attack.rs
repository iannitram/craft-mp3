//! Transient detection for window switching.
//!
//! A granule needs short blocks when a sudden rise in energy falls inside
//! its MDCT window, or the quantization noise of the long transform would
//! spread ahead of the attack as audible pre-echo. The detector measures
//! high-passed energy in 192-sample sub-blocks (a third of a granule) and
//! flags a sub-block whose energy jumps well above the recent past.
//!
//! The analysis filterbank delays the signal by about 240 samples, and a
//! short granule's three windows start a third of a granule before its
//! new half, so granule `g`'s short windows best cover input samples
//! `576 g - 432 .. 576 (g + 1) - 432`. The detector's sub-blocks are
//! aligned so that three of them make up each such region.

use std::collections::VecDeque;

/// Samples per sub-block.
const SUB_BLOCK: usize = 192;

/// Input samples by which the filterbank lags the PCM it is given.
const FILTERBANK_LAG: usize = 240;

/// End of granule 0's region, from which all others follow.
const FIRST_REGION_END: usize = 576 - FILTERBANK_LAG - SUB_BLOCK;

/// A sub-block is an attack when its energy exceeds the largest of the
/// previous three by this factor...
const ATTACK_RATIO: f32 = 10.0;

/// ...and its mean energy per sample is above this floor (about -60 dBFS
/// after the high-pass).
const ENERGY_FLOOR: f32 = 1e-6;

/// Streaming detector for one channel; yields one flag per granule.
#[derive(Clone, Default)]
pub(crate) struct AttackDetector {
    previous_sample: f32,
    /// Samples consumed so far.
    position: usize,
    energy: f32,
    count: usize,
    /// Mean energies of the last three sub-blocks, newest last.
    history: [f32; 3],
    region_has_attack: bool,
    flags: VecDeque<bool>,
}

impl AttackDetector {
    /// Where the current sub-block ends.
    fn sub_block_end(&self) -> usize {
        // Sub-block boundaries fall on FIRST_REGION_END + k * SUB_BLOCK
        // (k may be negative): 144, 336, 528, ...
        let first = FIRST_REGION_END % SUB_BLOCK;
        if self.position < first {
            first
        } else {
            first + ((self.position - first) / SUB_BLOCK + 1) * SUB_BLOCK
        }
    }

    pub(crate) fn push(&mut self, sample: f32) {
        // A first difference keeps the highs, where pre-echo is heard, and
        // ignores slow bass swells.
        let high = sample - self.previous_sample;
        self.previous_sample = sample;
        let end = self.sub_block_end();
        self.energy += high * high;
        self.count += 1;
        self.position += 1;
        if self.position < end {
            return;
        }

        let mean = self.energy / self.count as f32;
        let before = self.history.iter().copied().fold(0.0, f32::max);
        if mean > ENERGY_FLOOR && mean > ATTACK_RATIO * before {
            self.region_has_attack = true;
        }
        self.history = [self.history[1], self.history[2], mean];
        self.energy = 0.0;
        self.count = 0;

        if self.position >= FIRST_REGION_END
            && (self.position - FIRST_REGION_END).is_multiple_of(576)
        {
            self.flags.push_back(self.region_has_attack);
            self.region_has_attack = false;
        }
    }

    /// Whether granule `index` (counting from the oldest not yet popped)
    /// contains an attack, or `None` if its samples haven't all arrived.
    pub(crate) fn attack(&self, index: usize) -> Option<bool> {
        self.flags.get(index).copied()
    }

    /// Forgets the oldest granule's flag.
    pub(crate) fn pop(&mut self) {
        self.flags.pop_front();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(input: &[f32]) -> Vec<bool> {
        let mut d = AttackDetector::default();
        for &x in input {
            d.push(x);
        }
        (0..).map_while(|i| d.attack(i)).collect()
    }

    fn noise(len: usize, amplitude: f32) -> Vec<f32> {
        let mut seed = 9u32;
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5)
            })
            .collect()
    }

    #[test]
    fn one_flag_per_granule_region() {
        assert_eq!(flags(&vec![0.0; FIRST_REGION_END - 1]).len(), 0);
        assert_eq!(flags(&vec![0.0; FIRST_REGION_END]).len(), 1);
        assert_eq!(flags(&vec![0.0; FIRST_REGION_END + 576 * 3]).len(), 4);
    }

    #[test]
    fn steady_signals_and_silence_have_no_attacks() {
        assert!(flags(&vec![0.0; 576 * 10]).iter().all(|&a| !a));
        let steady = noise(576 * 10, 0.5);
        // The very first sub-block rises from silence; after that, nothing.
        assert!(flags(&steady)[1..].iter().all(|&a| !a));
        let sine: Vec<f32> = (0..576 * 10)
            .map(|n| (n as f32 * 0.3).sin() * 0.8)
            .collect();
        assert!(flags(&sine)[1..].iter().all(|&a| !a));
    }

    #[test]
    fn a_click_is_flagged_in_the_granule_that_contains_it() {
        let mut input = noise(576 * 10, 0.001);
        // A loud burst starting inside granule 5's region.
        let start = 576 * 5 - FILTERBANK_LAG + 300;
        for (x, b) in input[start..start + 200].iter_mut().zip(noise(200, 0.8)) {
            *x += b;
        }
        let f = flags(&input);
        let attacks: Vec<usize> = f
            .iter()
            .enumerate()
            .filter(|&(_, &a)| a)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(attacks, [5], "{f:?}");
    }

    #[test]
    fn quiet_onsets_below_the_floor_are_ignored() {
        let mut input = vec![0.0; 576 * 6];
        let start = 576 * 3;
        for (x, b) in input[start..].iter_mut().zip(noise(576 * 3, 0.0005)) {
            *x = b;
        }
        assert!(flags(&input).iter().all(|&a| !a));
    }
}
