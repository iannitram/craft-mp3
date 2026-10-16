//! Quantization of one granule of one channel, and the rate loop that picks
//! the step size to fit a bit budget (ISO/IEC 11172-3, Annex C.1.5.4).

use crate::bitstream::BitWriter;
use crate::huffman::{self, MAX_VALUE};
use crate::mdct::GRANULE_LINES;
use crate::tables::{PRETAB, SLEN};

/// Long-block scale-factor bands that carry a scale factor.
pub(crate) const SCALEFAC_BANDS: usize = 21;

/// Long-block scale-factor band boundaries for one sample rate.
pub(crate) type SfbBounds = [usize; 23];

/// Largest `part2_3_length` the side information can express.
pub(crate) const MAX_PART23_BITS: u32 = (1 << 12) - 1;

/// LAME's region split for long blocks, indexed by the number of
/// scale-factor bands the big-values region reaches into.
#[rustfmt::skip]
const SUBDIVISION: [(usize, usize); 23] = [
    (0, 0), (0, 0), (0, 0), (0, 0), (0, 0), (0, 1), (1, 1), (1, 1), (1, 2), (2, 2), (2, 3), (2, 3),
    (3, 4), (3, 4), (3, 4), (4, 5), (4, 5), (4, 6), (5, 6), (5, 6), (5, 7), (6, 7), (6, 7),
];

/// Scale factors of one long-block granule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ScaleFactors {
    pub(crate) values: [u8; SCALEFAC_BANDS],
    /// Index into [`SLEN`]; must be large enough for `values`.
    pub(crate) compress: usize,
    /// Scale factors step by 2^1 instead of 2^0.5.
    pub(crate) scale: bool,
    /// Adds [`PRETAB`] to the scale factors.
    pub(crate) preflag: bool,
}

impl ScaleFactors {
    /// Bits the scale factors take in the main data (no scfsi reuse).
    pub(crate) fn bits(&self) -> u32 {
        let (slen1, slen2) = SLEN[self.compress];
        11 * slen1 + 10 * slen2
    }

    /// The smallest `scalefac_compress` whose field widths fit `values`.
    pub(crate) fn fit_compress(values: &[u8; SCALEFAC_BANDS]) -> Option<usize> {
        let max_low = values[..11].iter().copied().max().unwrap_or(0);
        let max_high = values[11..].iter().copied().max().unwrap_or(0);
        (0..SLEN.len())
            .filter(|&c| {
                let (slen1, slen2) = SLEN[c];
                u32::from(max_low) < 1 << slen1 && u32::from(max_high) < 1 << slen2
            })
            .min_by_key(|&c| {
                ScaleFactors {
                    compress: c,
                    ..Default::default()
                }
                .bits()
            })
    }

    /// The amount, in quarter powers of two, that band `sfb` is amplified
    /// before quantization.
    fn shift(&self, sfb: usize) -> i32 {
        if sfb >= SCALEFAC_BANDS {
            return 0;
        }
        let pre = if self.preflag { PRETAB[sfb] } else { 0 };
        let step = if self.scale { 4 } else { 2 };
        step * i32::from(self.values[sfb] + pre)
    }
}

/// One quantized granule of one channel and its side-information fields.
#[derive(Clone)]
pub(crate) struct Granule {
    pub(crate) ix: [i32; GRANULE_LINES],
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
    pub(crate) fn silent() -> Self {
        Self {
            ix: [0; GRANULE_LINES],
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
        self.scalefac.bits() + self.part3_bits
    }

    /// Writes the scale factors and Huffman data.
    pub(crate) fn write_main_data(&self, w: &mut BitWriter, sfb: &SfbBounds) {
        let (slen1, slen2) = SLEN[self.scalefac.compress];
        for (band, &value) in self.scalefac.values.iter().enumerate() {
            w.write_bits(u32::from(value), if band < 11 { slen1 } else { slen2 });
        }

        let [r1, r2] = self.region_starts(sfb);
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
    fn region_starts(&self, sfb: &SfbBounds) -> [usize; 2] {
        let bv_end = self.big_values * 2;
        let r1 = sfb[self.region0_count + 1].min(bv_end);
        let r2 = sfb[self.region0_count + self.region1_count + 2].min(bv_end);
        [r1, r2]
    }

    /// Splits `ix` into big-values, count1 and zero regions, chooses the
    /// Huffman tables and returns the part 3 bit count.
    ///
    /// With `search_regions` every legal region split is tried; otherwise
    /// the fixed [`SUBDIVISION`] heuristic is used.
    fn code(&mut self, sfb: &SfbBounds, search_regions: bool) -> u32 {
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

        let big_bits = if search_regions {
            self.search_regions(sfb, bv_end)
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
            self.set_regions(sfb, r0, r1)
        };

        self.part3_bits = big_bits + count1_bits;
        self.part3_bits
    }

    /// Picks tables for the given region split and returns the big-values bits.
    fn set_regions(&mut self, sfb: &SfbBounds, r0: usize, r1: usize) -> u32 {
        self.region0_count = r0;
        self.region1_count = r1;
        let [s1, s2] = self.region_starts(sfb);
        let bv_end = self.big_values * 2;
        let mut bits = 0;
        for (i, range) in [0..s1, s1..s2, s2..bv_end].into_iter().enumerate() {
            let (table, b) = huffman::best_pair_table(&self.ix[range]);
            self.table_select[i] = table;
            bits += b;
        }
        bits
    }

    fn search_regions(&mut self, sfb: &SfbBounds, bv_end: usize) -> u32 {
        // Best table cost for every sfb-aligned segment [sfb[a], min(sfb[b], bv_end)).
        let bound = |i: usize| sfb[i].min(bv_end);
        let cost = |a: usize, b: usize| huffman::best_pair_table(&self.ix[bound(a)..bound(b)]).1;

        let mut best = (u32::MAX, 0, 0);
        for r0 in 0..16 {
            let head = cost(0, r0 + 1);
            for r1 in 0..8 {
                let split = r0 + r1 + 2;
                if split >= sfb.len() {
                    break;
                }
                let bits = head + cost(r0 + 1, split) + cost(split, sfb.len() - 1);
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
        self.set_regions(sfb, best.1, best.2)
    }
}

/// `|xr|^(3/4)` for each line, computed once per granule.
pub(crate) fn magnitudes34(xr: &[f32; GRANULE_LINES]) -> [f32; GRANULE_LINES] {
    xr.map(|x| x.abs().powf(0.75))
}

/// Quantizes `xr` with `global_gain` and scale factors into `ix`.
/// Returns `false` if a value exceeds what the Huffman tables can code.
fn quantize(
    xr: &[f32; GRANULE_LINES],
    xr34: &[f32; GRANULE_LINES],
    global_gain: u32,
    scalefac: &ScaleFactors,
    sfb: &SfbBounds,
    ix: &mut [i32; GRANULE_LINES],
) -> bool {
    for band in 0..sfb.len() - 1 {
        // Quarter-power exponent of the step size for this band.
        let q = global_gain as i32 - 210 - scalefac.shift(band);
        let gain = (-0.1875 * q as f32).exp2();
        for i in sfb[band]..sfb[band + 1] {
            // nint(x^(3/4) - 0.0946), the standard's rounding bias.
            let magnitude = xr34[i] * gain + 0.4054;
            if magnitude > MAX_VALUE as f32 {
                return false;
            }
            let magnitude = magnitude as i32;
            ix[i] = if xr[i] < 0.0 { -magnitude } else { magnitude };
        }
    }
    true
}

/// Quantizes a granule with the finest step size, no finer than `min_gain`,
/// whose Huffman data fits in `max_bits` (scale-factor bits included).
pub(crate) fn rate_loop(
    xr: &[f32; GRANULE_LINES],
    scalefac: ScaleFactors,
    sfb: &SfbBounds,
    max_bits: u32,
    min_gain: u32,
) -> Granule {
    let max_bits = max_bits.min(MAX_PART23_BITS);
    if max_bits < scalefac.bits() {
        return rate_loop(xr, ScaleFactors::default(), sfb, max_bits, min_gain);
    }
    let xr34 = magnitudes34(xr);
    let mut granule = Granule {
        scalefac,
        ..Granule::silent()
    };
    let part3_budget = max_bits - scalefac.bits();

    let try_gain = |gain: u32, granule: &mut Granule, search: bool| -> Option<u32> {
        granule.global_gain = gain;
        if !quantize(xr, &xr34, gain, &granule.scalefac, sfb, &mut granule.ix) {
            return None;
        }
        Some(granule.code(sfb, search)).filter(|&bits| bits <= part3_budget)
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
            return Granule::silent();
        }
        gain += 1;
    }
    granule
}

/// Quantization noise energy in lines `range` at band exponent `exponent`
/// (global gain minus the band's scale-factor shift), or `None` if a value
/// would exceed the Huffman range.
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
        let restored = (magnitude.floor()).powf(4.0 / 3.0) * step;
        let err = xr[i].abs() - restored;
        noise += err * err;
    }
    Some(noise)
}

/// Quantizes a granule so that every band's noise stays under `allowed`,
/// using scale factors to give each band its own step size, and as coarse
/// a step as that allows. The result's bit count is the granule's demand.
pub(crate) fn perceptual(
    xr: &[f32; GRANULE_LINES],
    allowed: &[f32; 22],
    sfb: &SfbBounds,
) -> Granule {
    let xr34 = magnitudes34(xr);

    // The coarsest exponent each band tolerates.
    let mut coarsest = [255i32; 22];
    for (band, c) in coarsest.iter_mut().enumerate() {
        let range = sfb[band]..sfb[band + 1];
        let ok =
            |e: i32| band_noise(xr, &xr34, e, range.clone()).is_none_or(|n| n <= allowed[band]);
        if ok(255) {
            continue;
        }
        let (mut lo, mut hi) = (0, 255);
        // Invariant: ok(lo) (or nothing is), !ok(hi).
        if !ok(lo) {
            *c = 0;
            continue;
        }
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if ok(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        *c = lo;
    }

    // The global gain is the coarsest any band needs, but the top band has
    // no scale factor, so it can't be finer than that band allows.
    let wanted = coarsest[..SCALEFAC_BANDS]
        .iter()
        .copied()
        .max()
        .unwrap()
        .min(coarsest[SCALEFAC_BANDS]);

    let candidates = [false, true].map(|scale| {
        let step = if scale { 4 } else { 2 };
        // Scale factors are at most 15 in bands 0-10 and 7 in 11-20.
        let limit = |band: usize| if band < 11 { 15 } else { 7 };
        let gain = (0..SCALEFAC_BANDS)
            .map(|b| coarsest[b] + step * limit(b))
            .fold(wanted, i32::min);
        let mut values = [0u8; SCALEFAC_BANDS];
        for (b, v) in values.iter_mut().enumerate() {
            let shift = (gain - coarsest[b]).max(0);
            *v = ((shift + step - 1) / step) as u8;
        }
        let compress =
            ScaleFactors::fit_compress(&values).expect("values are within the field limits");
        let scalefac = ScaleFactors {
            values,
            compress,
            scale,
            preflag: false,
        };
        rate_loop(xr, scalefac, sfb, MAX_PART23_BITS, gain.max(0) as u32)
    });
    let [fine, coarse] = candidates;
    if coarse.part2_3_length() < fine.part2_3_length() {
        coarse
    } else {
        fine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::SFB_LONG_44100;

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

    #[test]
    fn rate_loop_respects_the_budget_and_uses_most_of_it() {
        for budget in [200, 800, 1500, 3000, 4095] {
            let xr = spectrum(9, 0.5);
            let g = rate_loop(&xr, ScaleFactors::default(), &SFB_LONG_44100, budget, 0);
            assert!(
                g.part2_3_length() <= budget,
                "{} > {budget}",
                g.part2_3_length()
            );
            assert!(
                g.part2_3_length() > budget * 8 / 10,
                "only {} of {budget}",
                g.part2_3_length()
            );
        }
    }

    #[test]
    fn silence_codes_to_nothing() {
        let g = rate_loop(
            &[0.0; GRANULE_LINES],
            ScaleFactors::default(),
            &SFB_LONG_44100,
            1000,
            0,
        );
        assert_eq!(g.part2_3_length(), 0);
        assert_eq!((g.big_values, g.count1), (0, 0));
    }

    #[test]
    fn written_length_matches_the_counted_length() {
        let xr = spectrum(4, 0.8);
        let mut values = [0u8; SCALEFAC_BANDS];
        values[3] = 5;
        values[15] = 2;
        let sf = ScaleFactors {
            values,
            compress: ScaleFactors::fit_compress(&values).unwrap(),
            ..Default::default()
        };
        let g = rate_loop(&xr, sf, &SFB_LONG_44100, 2500, 0);
        let mut w = BitWriter::new();
        g.write_main_data(&mut w, &SFB_LONG_44100);
        assert_eq!(w.bit_len() as u32, g.part2_3_length());
    }

    #[test]
    fn quantization_round_trips_within_half_a_step() {
        let xr = spectrum(5, 0.3);
        let g = rate_loop(&xr, ScaleFactors::default(), &SFB_LONG_44100, 4095, 0);
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
        for seed in 0..20 {
            let xr = spectrum(seed, 0.6);
            let xr34 = magnitudes34(&xr);
            let mut g = Granule::silent();
            assert!(quantize(
                &xr,
                &xr34,
                180,
                &g.scalefac.clone(),
                &SFB_LONG_44100,
                &mut g.ix
            ));
            let heuristic = g.code(&SFB_LONG_44100, false);
            let searched = g.code(&SFB_LONG_44100, true);
            assert!(searched <= heuristic, "{searched} > {heuristic}");
        }
    }

    #[test]
    fn fit_compress_picks_the_cheapest_wide_enough_fields() {
        assert_eq!(ScaleFactors::fit_compress(&[0; SCALEFAC_BANDS]), Some(0));
        let mut v = [0u8; SCALEFAC_BANDS];
        v[0] = 15;
        v[20] = 7;
        let c = ScaleFactors::fit_compress(&v).unwrap();
        assert_eq!(SLEN[c], (4, 3));
        v[0] = 16;
        assert_eq!(ScaleFactors::fit_compress(&v), None);
    }
}
