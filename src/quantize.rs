//! Quantization of one granule of one channel, the rate loop that picks the
//! step size to fit a bit budget, and the perceptual search that picks
//! scale factors to meet a noise target (ISO/IEC 11172-3, Annex C.1.5.4).

use crate::bitstream::BitWriter;
use crate::huffman::{self, MAX_VALUE};
use crate::layout::{LONG_BANDS, Layout, SHORT_BANDS};
use crate::mdct::{BlockType, GRANULE_LINES};
use crate::tables::{PRETAB, SLEN};

/// Long-block bands that carry a scale factor.
pub(crate) const LONG_SCALEFACS: usize = LONG_BANDS - 1;

/// Short-block bands (per window) that carry a scale factor.
pub(crate) const SHORT_SCALEFACS: usize = SHORT_BANDS - 1;

/// Largest `part2_3_length` the side information can express.
pub(crate) const MAX_PART23_BITS: u32 = (1 << 12) - 1;

/// Largest subblock gain (3 bits).
const MAX_SUBBLOCK_GAIN: i32 = 7;

/// LAME's region split for long blocks, indexed by the number of
/// scale-factor bands the big-values region reaches into.
#[rustfmt::skip]
const SUBDIVISION: [(usize, usize); 23] = [
    (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 1), (1, 1), (1, 1), (1, 2), (2, 2), (2, 3), (2, 3),
    (3, 4), (3, 4), (3, 4), (4, 5), (4, 5), (4, 6), (5, 6), (5, 6), (5, 7), (6, 7), (6, 7),
];

/// The largest scale factor band `band` can hold when the first `split`
/// bands use the wider `slen1` field.
fn limit(band: usize, split: usize) -> i32 {
    if band < split { 15 } else { 7 }
}

/// Scale factors and subblock gains of one granule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScaleFactors {
    pub(crate) long: [u8; LONG_SCALEFACS],
    /// `short[band][window]`.
    pub(crate) short: [[u8; 3]; SHORT_SCALEFACS],
    /// Per-window gain for short blocks, in steps of 2^-2.
    pub(crate) subblock_gain: [u8; 3],
    /// `scalefac_compress`: an index into [`SLEN`] for MPEG-1, or MPEG-2's
    /// 9-bit packing of four field widths; it must fit the scale factors.
    pub(crate) compress: usize,
    /// Pack the scale factors the MPEG-2 way.
    pub(crate) lsf: bool,
    /// Scale factors step by 2^1 instead of 2^0.5.
    pub(crate) scale: bool,
    /// Adds [`PRETAB`] to long-block scale factors.
    pub(crate) preflag: bool,
}

impl ScaleFactors {
    /// Field width and count for each group of scale factors, in bitstream
    /// order (short blocks count each band's three windows).
    fn fields(&self, block: BlockType) -> [(u32, usize); 4] {
        let short = block == BlockType::Short;
        if !self.lsf {
            let (slen1, slen2) = SLEN[self.compress];
            return if short {
                [(slen1, 18), (slen2, 18), (0, 0), (0, 0)]
            } else {
                [(slen1, 11), (slen2, 10), (0, 0), (0, 0)]
            };
        }
        // ISO/IEC 13818-3, 2.4.3.2, without intensity stereo; only the
        // first form (scalefac_compress < 400, no preflag) is produced.
        let c = self.compress as u32;
        let slen = [(c >> 4) / 5, (c >> 4) % 5, (c % 16) >> 2, c % 4];
        let counts = if short { [9, 9, 9, 9] } else { [6, 5, 5, 5] };
        std::array::from_fn(|i| (slen[i], counts[i]))
    }

    /// Bits the scale factors take in the main data (no scfsi reuse).
    pub(crate) fn bits(&self, block: BlockType) -> u32 {
        self.fields(block)
            .iter()
            .map(|&(slen, count)| slen * count as u32)
            .sum()
    }

    /// The scale factors `block` sends, in bitstream order.
    fn values(&self, block: BlockType) -> Vec<u8> {
        if block == BlockType::Short {
            self.short.iter().flatten().copied().collect()
        } else {
            self.long.to_vec()
        }
    }

    /// The cheapest `scalefac_compress` whose field widths fit the scale
    /// factors `block` uses.
    pub(crate) fn fit_compress(&self, block: BlockType) -> Option<usize> {
        if self.lsf {
            return self.fit_compress_lsf(block);
        }
        let (low, high) = if block == BlockType::Short {
            let max = |bands: &[[u8; 3]]| bands.iter().flatten().copied().max().unwrap_or(0);
            (max(&self.short[..6]), max(&self.short[6..]))
        } else {
            let max = |bands: &[u8]| bands.iter().copied().max().unwrap_or(0);
            (max(&self.long[..11]), max(&self.long[11..]))
        };
        (0..SLEN.len())
            .filter(|&c| {
                let (slen1, slen2) = SLEN[c];
                u32::from(low) < 1 << slen1 && u32::from(high) < 1 << slen2
            })
            .min_by_key(|&c| {
                ScaleFactors {
                    compress: c,
                    ..*self
                }
                .bits(block)
            })
    }

    fn fit_compress_lsf(&self, block: BlockType) -> Option<usize> {
        debug_assert!(!self.preflag, "MPEG-2 preflag packing is not produced");
        let values = self.values(block);
        let counts = if block == BlockType::Short {
            [9, 9, 9, 9]
        } else {
            [6, 5, 5, 5]
        };
        let mut slen = [0u32; 4];
        let mut start = 0;
        for (i, &count) in counts.iter().enumerate() {
            let max = values[start..start + count]
                .iter()
                .copied()
                .max()
                .unwrap_or(0);
            slen[i] = u32::BITS - u32::from(max).leading_zeros();
            start += count;
        }
        // The first two groups have 5 widths (0..=4), the others 4 (0..=3).
        if slen[0] > 4 || slen[1] > 4 || slen[2] > 3 || slen[3] > 3 {
            return None;
        }
        Some((((slen[0] * 5 + slen[1]) << 4) + (slen[2] << 2) + slen[3]) as usize)
    }

    /// How much, in quarter powers of two, each line is amplified before
    /// quantization; lines are in bitstream order.
    fn line_shifts(&self, block: BlockType, layout: &Layout) -> [i32; GRANULE_LINES] {
        let step = if self.scale { 4 } else { 2 };
        let mut shifts = [0; GRANULE_LINES];
        for p in layout.partitions(block) {
            let shift = match p.window {
                Some(w) => {
                    let sf = self.short.get(p.band).map_or(0, |s| i32::from(s[w]));
                    8 * i32::from(self.subblock_gain[w]) + step * sf
                }
                None => {
                    let sf = self.long.get(p.band).map_or(0, |&s| i32::from(s));
                    let pre = if self.preflag {
                        i32::from(PRETAB[p.band])
                    } else {
                        0
                    };
                    step * (sf + pre)
                }
            };
            shifts[p.lines.clone()].fill(shift);
        }
        shifts
    }
}

/// One quantized granule of one channel and its side-information fields.
#[derive(Clone)]
pub(crate) struct Granule {
    pub(crate) ix: [i32; GRANULE_LINES],
    pub(crate) block: BlockType,
    pub(crate) global_gain: u32,
    pub(crate) scalefac: ScaleFactors,
    /// Number of pairs in the big-values region.
    pub(crate) big_values: usize,
    /// Number of quads in the count1 region.
    pub(crate) count1: usize,
    pub(crate) table_select: [usize; 3],
    pub(crate) region0_count: usize,
    pub(crate) region1_count: usize,
    pub(crate) count1_table_b: bool,
    /// Huffman-coded bits (part 3).
    pub(crate) part3_bits: u32,
}

impl Granule {
    /// A granule of silence: every line zero.
    pub(crate) fn silent(block: BlockType) -> Self {
        Self {
            ix: [0; GRANULE_LINES],
            block,
            global_gain: 210,
            scalefac: ScaleFactors::default(),
            big_values: 0,
            count1: 0,
            table_select: [0; 3],
            region0_count: 0,
            region1_count: 0,
            count1_table_b: false,
            part3_bits: 0,
        }
    }

    pub(crate) fn part2_3_length(&self) -> u32 {
        self.scalefac.bits(self.block) + self.part3_bits
    }

    /// Writes the scale factors and Huffman data.
    pub(crate) fn write_main_data(&self, w: &mut BitWriter, layout: &Layout) {
        let values = self.scalefac.values(self.block);
        let mut values = values.iter();
        for (slen, count) in self.scalefac.fields(self.block) {
            for &value in values.by_ref().take(count) {
                w.write_bits(u32::from(value), slen);
            }
        }

        let [r1, r2] = self.region_starts(layout);
        let bv_end = self.big_values * 2;
        huffman::write_pairs(w, self.table_select[0], &self.ix[..r1]);
        huffman::write_pairs(w, self.table_select[1], &self.ix[r1..r2]);
        huffman::write_pairs(w, self.table_select[2], &self.ix[r2..bv_end]);
        huffman::write_quads(
            w,
            self.count1_table_b,
            &self.ix[bv_end..bv_end + 4 * self.count1],
        );
    }

    /// Where regions 1 and 2 start, as the decoder computes them.
    fn region_starts(&self, layout: &Layout) -> [usize; 2] {
        let bv_end = self.big_values * 2;
        // Window-switched granules have an implicit split and no region 2:
        // after 3 short bands (36 lines) for short blocks, after 8 long
        // bands for start and stop blocks (36 lines in MPEG-1, 54 in MPEG-2).
        match self.block {
            BlockType::Short => return [(3 * layout.short_bounds[3]).min(bv_end), bv_end],
            BlockType::Start | BlockType::Stop => {
                return [layout.long_bounds[8].min(bv_end), bv_end];
            }
            BlockType::Normal => {}
        }
        let sfb = layout.long_bounds;
        let r1 = sfb[self.region0_count + 1].min(bv_end);
        let r2 = sfb[self.region0_count + self.region1_count + 2].min(bv_end);
        [r1, r2]
    }

    /// Splits `ix` into big-values, count1 and zero regions, chooses the
    /// Huffman tables and returns the part 3 bit count.
    ///
    /// For normal blocks with `search_regions`, every legal region split is
    /// tried; otherwise the fixed [`SUBDIVISION`] heuristic is used.
    fn code(&mut self, layout: &Layout, search_regions: bool) -> u32 {
        let ix = &self.ix;

        // Trailing zero pairs.
        let mut end = GRANULE_LINES;
        while end >= 2 && ix[end - 1] == 0 && ix[end - 2] == 0 {
            end -= 2;
        }
        // Then quads of magnitude at most 1.
        let mut count1 = 0;
        while end >= 4 && ix[end - 4..end].iter().all(|v| v.unsigned_abs() <= 1) {
            end -= 4;
            count1 += 1;
        }
        let bv_end = end;
        self.big_values = bv_end / 2;
        self.count1 = count1;

        let quads = &ix[bv_end..bv_end + 4 * count1];
        let (a, b) = (
            huffman::quad_bits(false, quads),
            huffman::quad_bits(true, quads),
        );
        self.count1_table_b = b < a;
        let count1_bits = a.min(b);

        let sfb = layout.long_bounds;
        let big_bits = if self.block != BlockType::Normal {
            self.set_regions(layout, 0, 0)
        } else if search_regions {
            self.search_regions(layout, bv_end)
        } else {
            let reach = (1..sfb.len())
                .find(|&i| sfb[i] >= bv_end)
                .unwrap_or(sfb.len() - 1);
            let (mut r0, mut r1) = SUBDIVISION[reach];
            while r0 > 0 && sfb[r0 + 1] > bv_end {
                r0 -= 1;
            }
            while r1 > 0 && sfb[r0 + r1 + 2] > bv_end {
                r1 -= 1;
            }
            self.set_regions(layout, r0, r1)
        };

        self.part3_bits = big_bits + count1_bits;
        self.part3_bits
    }

    /// Picks tables for the given region split and returns the big-values bits.
    fn set_regions(&mut self, layout: &Layout, r0: usize, r1: usize) -> u32 {
        self.region0_count = r0;
        self.region1_count = r1;
        let [s1, s2] = self.region_starts(layout);
        let bv_end = self.big_values * 2;
        let mut bits = 0;
        for (i, range) in [0..s1, s1..s2, s2..bv_end].into_iter().enumerate() {
            let (table, b) = huffman::best_pair_table(&self.ix[range]);
            self.table_select[i] = table;
            bits += b;
        }
        bits
    }

    fn search_regions(&mut self, layout: &Layout, bv_end: usize) -> u32 {
        let sfb = layout.long_bounds;
        let bound = |i: usize| sfb[i].min(bv_end);

        // prefix[t][b]: bits to code bands 0..b with table t, or a huge
        // value once some band holds a magnitude t can't code. Any region
        // made of whole bands then costs a subtraction per table.
        const UNUSABLE: u64 = 1 << 40;
        let mut prefix = [[0u64; LONG_BANDS + 1]; 32];
        for band in 0..LONG_BANDS {
            let values = &self.ix[bound(band)..bound(band + 1)];
            let max = values.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
            for (table, row) in prefix.iter_mut().enumerate() {
                let bits = match huffman::table_max_value(table) {
                    Some(limit) if limit >= max => u64::from(huffman::pair_bits(table, values)),
                    _ => UNUSABLE,
                };
                row[band + 1] = row[band] + bits;
            }
        }
        let cost = |a: usize, b: usize| prefix.iter().map(|row| row[b] - row[a]).min().unwrap();

        let mut best = (u64::MAX, 0, 0);
        for r0 in 0..16 {
            let head = cost(0, r0 + 1);
            for r1 in 0..8 {
                let split = r0 + r1 + 2;
                if split > LONG_BANDS {
                    break;
                }
                let bits = head + cost(r0 + 1, split) + cost(split, LONG_BANDS);
                if bits < best.0 {
                    best = (bits, r0, r1);
                }
                if bound(split) == bv_end {
                    break;
                }
            }
            if bound(r0 + 1) == bv_end {
                break;
            }
        }
        self.set_regions(layout, best.1, best.2)
    }
}

/// `n^(4/3)` for every magnitude the Huffman tables can code.
fn pow43(n: usize) -> f32 {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=MAX_VALUE)
            .map(|n| (n as f64).powf(4.0 / 3.0) as f32)
            .collect()
    })[n]
}

/// `|xr|^(3/4)` for each line, computed once per granule.
pub(crate) fn magnitudes34(xr: &[f32; GRANULE_LINES]) -> [f32; GRANULE_LINES] {
    xr.map(|x| x.abs().powf(0.75))
}

/// Quantizes `xr` with `global_gain` and per-line shifts into `ix`.
/// Returns `false` if a value exceeds what the Huffman tables can code.
fn quantize(
    xr: &[f32; GRANULE_LINES],
    xr34: &[f32; GRANULE_LINES],
    global_gain: u32,
    shifts: &[i32; GRANULE_LINES],
    ix: &mut [i32; GRANULE_LINES],
) -> bool {
    let mut i = 0;
    while i < GRANULE_LINES {
        // Runs of equal shift share one gain computation.
        let shift = shifts[i];
        let q = global_gain as i32 - 210 - shift;
        let gain = (-0.1875 * q as f32).exp2();
        while i < GRANULE_LINES && shifts[i] == shift {
            // nint(x^(3/4) - 0.0946), the standard's rounding bias.
            let magnitude = xr34[i] * gain + 0.4054;
            if magnitude > MAX_VALUE as f32 {
                return false;
            }
            let magnitude = magnitude as i32;
            ix[i] = if xr[i] < 0.0 { -magnitude } else { magnitude };
            i += 1;
        }
    }
    true
}

/// Quantizes a granule with the finest step size, no finer than `min_gain`,
/// whose Huffman data fits in `max_bits` (scale-factor bits included).
pub(crate) fn rate_loop(
    xr: &[f32; GRANULE_LINES],
    block: BlockType,
    scalefac: ScaleFactors,
    layout: &Layout,
    max_bits: u32,
    min_gain: u32,
) -> Granule {
    let max_bits = max_bits.min(MAX_PART23_BITS);
    if max_bits < scalefac.bits(block) {
        return rate_loop(
            xr,
            block,
            ScaleFactors::default(),
            layout,
            max_bits,
            min_gain,
        );
    }
    let xr34 = magnitudes34(xr);
    let shifts = scalefac.line_shifts(block, layout);
    let mut granule = Granule {
        scalefac,
        ..Granule::silent(block)
    };
    let part3_budget = max_bits - scalefac.bits(block);

    let try_gain = |gain: u32, granule: &mut Granule, search: bool| -> Option<u32> {
        granule.global_gain = gain;
        if !quantize(xr, &xr34, gain, &shifts, &mut granule.ix) {
            return None;
        }
        Some(granule.code(layout, search)).filter(|&bits| bits <= part3_budget)
    };

    // Smallest gain (finest quantization) that fits; bits fall as gain rises.
    let (mut lo, mut hi) = (min_gain.min(255), 255u32);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if try_gain(mid, &mut granule, false).is_some() {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    // The budget can be too small even at the coarsest step; then give up
    // the remaining lines (a decoder still accepts the frame).
    let mut gain = lo;
    while try_gain(gain, &mut granule, true).is_none() {
        if gain == 255 {
            return Granule::silent(block);
        }
        gain += 1;
    }
    granule
}

/// Quantization noise energy in lines `range` at exponent `exponent`
/// (global gain minus the lines' shift), or `None` if a value would exceed
/// the Huffman range.
fn band_noise(
    xr: &[f32; GRANULE_LINES],
    xr34: &[f32; GRANULE_LINES],
    exponent: i32,
    range: std::ops::Range<usize>,
) -> Option<f32> {
    let q = exponent - 210;
    let gain = (-0.1875 * q as f32).exp2();
    let step = (0.25 * q as f32).exp2();
    let mut noise = 0.0;
    for i in range {
        let magnitude = xr34[i] * gain + 0.4054;
        if magnitude > MAX_VALUE as f32 {
            return None;
        }
        let restored = pow43(magnitude as usize) * step;
        let err = xr[i].abs() - restored;
        noise += err * err;
    }
    Some(noise)
}

/// Quantizes a granule so that every partition's noise stays under
/// `allowed` (one value per [`Layout::partitions`] entry), using scale
/// factors and subblock gains to give each partition its own step size, and
/// as coarse a step as that allows. The result's bit count is the
/// granule's demand.
pub(crate) fn perceptual(
    xr: &[f32; GRANULE_LINES],
    block: BlockType,
    allowed: &[f32],
    layout: &Layout,
) -> Granule {
    let xr34 = magnitudes34(xr);
    let parts = layout.partitions(block);

    // The coarsest exponent each partition tolerates.
    let coarsest: Vec<i32> = parts
        .iter()
        .zip(allowed)
        .map(|(p, &allowed)| {
            let ok =
                |e: i32| band_noise(xr, &xr34, e, p.lines.clone()).is_none_or(|n| n <= allowed);
            if ok(255) {
                return 255;
            }
            if !ok(0) {
                return 0;
            }
            // Invariant: ok(lo) and !ok(hi).
            let (mut lo, mut hi) = (0, 255);
            while hi - lo > 1 {
                let mid = (lo + hi) / 2;
                if ok(mid) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            lo
        })
        .collect();

    let ceil_div = |a: i32, b: i32| (a + b - 1) / b;
    // Quantizes with scale factors stepping by 2^0.5 (`scale` false) or 2^1,
    // and reports whether their range kept any band finer than it needed.
    let candidate = |scale: bool| -> (Granule, bool) {
        let step = if scale { 4 } else { 2 };
        let mut scalefac = ScaleFactors {
            scale,
            lsf: layout.lsf,
            ..Default::default()
        };
        let (gain, limited) = if block == BlockType::Short {
            // Each window gets a base exponent: as coarse as its bands need,
            // but no coarser than its top band (which has no scale factor)
            // allows, or than scale factors can bring its finest band down to.
            let mut limited = false;
            let base: [i32; 3] = std::array::from_fn(|w| {
                let c = |band: usize| coarsest[band * 3 + w];
                let wanted = (0..SHORT_SCALEFACS)
                    .map(c)
                    .max()
                    .unwrap()
                    .min(c(SHORT_SCALEFACS));
                let reach = (0..SHORT_SCALEFACS)
                    .map(|b| c(b) + step * limit(b, 6))
                    .fold(wanted, i32::min);
                limited |= reach < wanted;
                reach
            });
            // Subblock gains take the global gain down to each window's base.
            let coarsest_base = base.iter().copied().max().unwrap();
            let finest_base = base.iter().copied().min().unwrap();
            let gain = coarsest_base.min(finest_base + 8 * MAX_SUBBLOCK_GAIN);
            for (w, &b) in base.iter().enumerate() {
                let sbg = ceil_div((gain - b).max(0), 8);
                scalefac.subblock_gain[w] = sbg as u8;
                let window_exponent = gain - 8 * sbg;
                for band in 0..SHORT_SCALEFACS {
                    let shift = (window_exponent - coarsest[band * 3 + w]).max(0);
                    scalefac.short[band][w] = ceil_div(shift, step).min(limit(band, 6)) as u8;
                }
            }
            (gain, limited)
        } else {
            // The top band has no scale factor, so the global gain can't be
            // coarser than it allows.
            let wanted = coarsest[..LONG_SCALEFACS]
                .iter()
                .copied()
                .max()
                .unwrap()
                .min(coarsest[LONG_SCALEFACS]);
            let gain = (0..LONG_SCALEFACS)
                .map(|b| coarsest[b] + step * limit(b, 11))
                .fold(wanted, i32::min);
            for (band, sf) in scalefac.long.iter_mut().enumerate() {
                let shift = (gain - coarsest[band]).max(0);
                *sf = ceil_div(shift, step).min(limit(band, 11)) as u8;
            }
            (gain, gain < wanted)
        };
        scalefac.compress = scalefac
            .fit_compress(block)
            .expect("scale factors are within the field limits");
        (
            rate_loop(
                xr,
                block,
                scalefac,
                layout,
                MAX_PART23_BITS,
                gain.max(0) as u32,
            ),
            limited,
        )
    };

    // Half-power-of-two steps fit the targets more closely; whole powers
    // reach further, which only helps when the finer kind ran out of range.
    let (fine, limited) = candidate(false);
    if !limited {
        return fine;
    }
    let (coarse, _) = candidate(true);
    if coarse.part2_3_length() < fine.part2_3_length() {
        coarse
    } else {
        fine
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spectrum(seed: u32, amplitude: f32) -> [f32; GRANULE_LINES] {
        let mut s = seed;
        let mut xr = [0.0; GRANULE_LINES];
        for (i, x) in xr.iter_mut().enumerate() {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (s >> 8) as f32 / (1 << 24) as f32 - 0.5;
            // Falling spectrum, like real audio.
            *x = amplitude * noise / (1.0 + i as f32 / 16.0);
        }
        xr
    }

    const BLOCKS: [BlockType; 4] = [
        BlockType::Normal,
        BlockType::Start,
        BlockType::Short,
        BlockType::Stop,
    ];

    #[test]
    fn rate_loop_respects_the_budget_and_uses_most_of_it() {
        let layout = Layout::new(44_100);
        for block in BLOCKS {
            for budget in [200, 800, 1500, 3000, 4095] {
                let xr = spectrum(9, 0.5);
                let g = rate_loop(&xr, block, ScaleFactors::default(), &layout, budget, 0);
                let used = g.part2_3_length();
                assert!(used <= budget, "{block:?}: {used} > {budget}");
                assert!(used > budget * 8 / 10, "{block:?}: only {used} of {budget}");
            }
        }
    }

    #[test]
    fn silence_codes_to_nothing() {
        let layout = Layout::new(44_100);
        let g = rate_loop(
            &[0.0; GRANULE_LINES],
            BlockType::Normal,
            ScaleFactors::default(),
            &layout,
            1000,
            0,
        );
        assert_eq!(g.part2_3_length(), 0);
        assert_eq!((g.big_values, g.count1), (0, 0));
    }

    #[test]
    fn written_length_matches_the_counted_length() {
        let layout = Layout::new(44_100);
        for block in BLOCKS {
            let xr = spectrum(4, 0.8);
            let mut sf = ScaleFactors::default();
            sf.long[3] = 5;
            sf.long[15] = 2;
            sf.short[2] = [1, 0, 3];
            sf.short[9] = [0, 2, 1];
            sf.subblock_gain = [0, 2, 1];
            sf.compress = sf.fit_compress(block).unwrap();
            let g = rate_loop(&xr, block, sf, &layout, 2500, 0);
            let mut w = BitWriter::new();
            g.write_main_data(&mut w, &layout);
            assert_eq!(w.bit_len() as u32, g.part2_3_length(), "{block:?}");
        }
    }

    #[test]
    fn quantization_round_trips_within_a_step() {
        let layout = Layout::new(44_100);
        let xr = spectrum(5, 0.3);
        let g = rate_loop(
            &xr,
            BlockType::Normal,
            ScaleFactors::default(),
            &layout,
            4095,
            0,
        );
        let step = (0.25 * (g.global_gain as f32 - 210.0)).exp2();
        for (&x, &q) in xr.iter().zip(&g.ix) {
            let restored = (q.unsigned_abs() as f32).powf(4.0 / 3.0) * step * (q.signum() as f32);
            // Error is bounded by the step at that magnitude.
            let bound = step * (q.unsigned_abs() as f32 + 1.0).powf(1.0 / 3.0) * 1.5;
            assert!((restored - x).abs() <= bound, "{x} -> {q} -> {restored}");
        }
    }

    #[test]
    fn region_search_never_costs_more_than_the_heuristic() {
        let layout = Layout::new(44_100);
        for seed in 0..20 {
            let xr = spectrum(seed, 0.6);
            let xr34 = magnitudes34(&xr);
            let mut g = Granule::silent(BlockType::Normal);
            assert!(quantize(&xr, &xr34, 180, &[0; GRANULE_LINES], &mut g.ix));
            let heuristic = g.code(&layout, false);
            let searched = g.code(&layout, true);
            assert!(searched <= heuristic, "{searched} > {heuristic}");
        }
    }

    #[test]
    fn region_search_finds_the_cheapest_split() {
        let layout = Layout::new(48_000);
        for seed in 0..10 {
            let xr = spectrum(seed, 0.4 + seed as f32 * 0.1);
            let xr34 = magnitudes34(&xr);
            let mut g = Granule::silent(BlockType::Normal);
            assert!(quantize(
                &xr,
                &xr34,
                170 + seed,
                &[0; GRANULE_LINES],
                &mut g.ix
            ));
            let searched = g.code(&layout, true);
            // Brute force over every legal split.
            let mut best = u32::MAX;
            for r0 in 0..16 {
                for r1 in 0..8 {
                    if r0 + r1 + 2 <= LONG_BANDS {
                        best = best.min(g.set_regions(&layout, r0, r1));
                    }
                }
            }
            let count1 = huffman::quad_bits(
                g.count1_table_b,
                &g.ix[g.big_values * 2..g.big_values * 2 + 4 * g.count1],
            );
            assert_eq!(searched, best + count1, "seed {seed}");
        }
    }

    #[test]
    fn fit_compress_picks_the_cheapest_wide_enough_fields() {
        let mut sf = ScaleFactors::default();
        assert_eq!(sf.fit_compress(BlockType::Normal), Some(0));
        sf.long[0] = 15;
        sf.long[20] = 7;
        assert_eq!(SLEN[sf.fit_compress(BlockType::Normal).unwrap()], (4, 3));
        sf.long[0] = 16;
        assert_eq!(sf.fit_compress(BlockType::Normal), None);
        // Short blocks look only at the short scale factors.
        assert_eq!(sf.fit_compress(BlockType::Short), Some(0));
        sf.short[7][2] = 3;
        assert_eq!(SLEN[sf.fit_compress(BlockType::Short).unwrap()], (0, 2));
    }

    #[test]
    fn perceptual_meets_every_partitions_noise_target() {
        let layout = Layout::new(44_100);
        for block in BLOCKS {
            let xr = spectrum(11, 0.5);
            let parts = layout.partitions(block);
            // A target 20 dB under each partition's energy, but not below 1e-9.
            let allowed: Vec<f32> = parts
                .iter()
                .map(|p| (xr[p.lines.clone()].iter().map(|x| x * x).sum::<f32>() * 0.01).max(1e-9))
                .collect();
            let g = perceptual(&xr, block, &allowed, &layout);
            let shifts = g.scalefac.line_shifts(block, &layout);
            for (p, &allowed) in parts.iter().zip(&allowed) {
                let q = g.global_gain as i32 - 210 - shifts[p.lines.start];
                let step = (0.25 * q as f32).exp2();
                let noise: f32 = p
                    .lines
                    .clone()
                    .map(|i| {
                        let restored = (g.ix[i].unsigned_abs() as f32).powf(4.0 / 3.0) * step;
                        (xr[i].abs() - restored).powi(2)
                    })
                    .sum();
                let (band, window) = (p.band, p.window);
                assert!(
                    noise <= allowed * 1.001,
                    "{block:?} band {band} window {window:?}: {noise} > {allowed}"
                );
            }
        }
    }
}
