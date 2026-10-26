//! How a granule's 576 lines divide into scale-factor bands, for long and
//! short blocks, in bitstream order.

use std::ops::Range;

use crate::mdct::{BlockType, GRANULE_LINES};
use crate::tables::*;

/// Long-block bands; the last one has no scale factor.
pub(crate) const LONG_BANDS: usize = 22;

/// Short-block bands per window; the last one has no scale factor.
pub(crate) const SHORT_BANDS: usize = 13;

/// One band of one window: the unit that gets its own step size.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Partition {
    /// Lines in bitstream order.
    pub(crate) lines: Range<usize>,
    /// Scale-factor band.
    pub(crate) band: usize,
    /// Short-block window, or `None` for long blocks.
    pub(crate) window: Option<usize>,
    /// Frequency of the band's middle, in Hz.
    pub(crate) centre_hz: f64,
    /// Frequency of the band's lowest line, in Hz.
    pub(crate) lowest_hz: f64,
    /// Frequency of the band's highest line, in Hz.
    pub(crate) highest_hz: f64,
}

/// Band layouts for one sample rate.
pub(crate) struct Layout {
    /// MPEG-2 low sampling frequency: one granule per frame and its own
    /// scale-factor packing.
    pub(crate) lsf: bool,
    pub(crate) long_bounds: &'static [usize; LONG_BANDS + 1],
    pub(crate) short_bounds: &'static [usize; SHORT_BANDS + 1],
    long: Vec<Partition>,
    short: Vec<Partition>,
}

impl Layout {
    pub(crate) fn new(sample_rate: u32) -> Self {
        let (long_bounds, short_bounds) = match sample_rate {
            16_000 => (&SFB_LONG_22050, &SFB_SHORT_16000),
            22_050 => (&SFB_LONG_22050, &SFB_SHORT_22050),
            24_000 => (&SFB_LONG_24000, &SFB_SHORT_24000),
            32_000 => (&SFB_LONG_32000, &SFB_SHORT_32000),
            48_000 => (&SFB_LONG_48000, &SFB_SHORT_48000),
            _ => (&SFB_LONG_44100, &SFB_SHORT_44100),
        };

        let hz = |line: f64, lines_per_spectrum: usize| {
            line * f64::from(sample_rate) / 2.0 / lines_per_spectrum as f64
        };
        let long = (0..LONG_BANDS)
            .map(|band| {
                let (lo, hi) = (long_bounds[band], long_bounds[band + 1]);
                Partition {
                    lines: lo..hi,
                    band,
                    window: None,
                    centre_hz: hz((lo + hi) as f64 / 2.0, GRANULE_LINES),
                    lowest_hz: hz(lo as f64 + 0.5, GRANULE_LINES),
                    highest_hz: hz(hi as f64 - 0.5, GRANULE_LINES),
                }
            })
            .collect();

        let window_lines = GRANULE_LINES / 3;
        let mut short = Vec::with_capacity(3 * SHORT_BANDS);
        for band in 0..SHORT_BANDS {
            let (lo, hi) = (short_bounds[band], short_bounds[band + 1]);
            for window in 0..3 {
                let start = 3 * lo + window * (hi - lo);
                short.push(Partition {
                    lines: start..start + hi - lo,
                    band,
                    window: Some(window),
                    centre_hz: hz((lo + hi) as f64 / 2.0, window_lines),
                    lowest_hz: hz(lo as f64 + 0.5, window_lines),
                    highest_hz: hz(hi as f64 - 0.5, window_lines),
                });
            }
        }

        Self {
            lsf: matches!(sample_rate, 16_000 | 22_050 | 24_000),
            long_bounds,
            short_bounds,
            long,
            short,
        }
    }

    /// The partitions of a granule with this block type, in bitstream order.
    pub(crate) fn partitions(&self, block: BlockType) -> &[Partition] {
        if block == BlockType::Short {
            &self.short
        } else {
            &self.long
        }
    }

    /// Reorders short-block lines from the MDCT's subband order
    /// (`subband * 18 + 3 * k + window`) to bitstream order (band by band,
    /// window by window).
    pub(crate) fn short_to_bitstream(
        &self,
        subband_order: &[f32; GRANULE_LINES],
    ) -> [f32; GRANULE_LINES] {
        let mut out = [0.0; GRANULE_LINES];
        for p in &self.short {
            let window = p.window.unwrap();
            let first = self.short_bounds[p.band];
            for (i, line) in p.lines.clone().enumerate() {
                let freq = first + i;
                out[line] = subband_order[(freq / 6) * 18 + (freq % 6) * 3 + window];
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_tile_the_granule_in_order() {
        for rate in [16_000, 22_050, 24_000, 32_000, 44_100, 48_000] {
            let layout = Layout::new(rate);
            for block in [BlockType::Normal, BlockType::Short] {
                let parts = layout.partitions(block);
                assert_eq!(parts.first().unwrap().lines.start, 0);
                assert_eq!(parts.last().unwrap().lines.end, GRANULE_LINES);
                for pair in parts.windows(2) {
                    assert_eq!(pair[0].lines.end, pair[1].lines.start);
                }
            }
            assert_eq!(layout.partitions(BlockType::Short).len(), 3 * SHORT_BANDS);
        }
    }

    #[test]
    fn short_reordering_is_a_permutation_grouped_by_window() {
        let layout = Layout::new(44_100);
        let mut subband_order = [0.0; GRANULE_LINES];
        for (i, x) in subband_order.iter_mut().enumerate() {
            *x = i as f32;
        }
        let out = layout.short_to_bitstream(&subband_order);
        let mut seen: Vec<usize> = out.iter().map(|&x| x as usize).collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..GRANULE_LINES).collect::<Vec<_>>());

        // Band 0 of window 1 holds that window's first four frequencies:
        // subband 0, k = 0..4, window 1.
        let p = &layout.partitions(BlockType::Short)[1];
        assert_eq!(p.window, Some(1));
        let got: Vec<usize> = out[p.lines.clone()].iter().map(|&x| x as usize).collect();
        assert_eq!(got, [1, 4, 7, 10]);
    }
}
