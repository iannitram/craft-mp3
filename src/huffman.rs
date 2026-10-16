//! Huffman coding of quantized frequency lines (ISO/IEC 11172-3, 2.4.2.7).
//!
//! The big-values region is coded in pairs with one of 32 pair tables,
//! chosen per region; tables 16–31 code values of 15 and above with an
//! escape plus `linbits` extra bits. The count1 region is coded in quads
//! of values in `-1..=1` with table A or B.

use crate::bitstream::BitWriter;
use crate::huffman_tables::*;

/// A pair table: codes indexed by `x * width + y` for `x, y` in `0..width`.
#[derive(Clone, Copy)]
struct PairTable {
    codes: &'static [u32],
    lens: &'static [u8],
    width: u32,
    linbits: u32,
}

impl PairTable {
    /// The largest magnitude this table can code.
    fn max_value(self) -> u32 {
        if self.linbits > 0 {
            15 + (1 << self.linbits) - 1
        } else {
            self.width - 1
        }
    }
}

const fn table(
    codes: &'static [u32],
    lens: &'static [u8],
    width: u32,
    linbits: u32,
) -> Option<PairTable> {
    Some(PairTable {
        codes,
        lens,
        width,
        linbits,
    })
}

/// Tables 0, 4 and 14 have no codes: 0 means "all zero" and 4 and 14 are unused.
const PAIR_TABLES: [Option<PairTable>; 32] = [
    None,
    table(&CODES_1, &LENS_1, 2, 0),
    table(&CODES_2, &LENS_2, 3, 0),
    table(&CODES_3, &LENS_3, 3, 0),
    None,
    table(&CODES_5, &LENS_5, 4, 0),
    table(&CODES_6, &LENS_6, 4, 0),
    table(&CODES_7, &LENS_7, 6, 0),
    table(&CODES_8, &LENS_8, 6, 0),
    table(&CODES_9, &LENS_9, 6, 0),
    table(&CODES_10, &LENS_10, 8, 0),
    table(&CODES_11, &LENS_11, 8, 0),
    table(&CODES_12, &LENS_12, 8, 0),
    table(&CODES_13, &LENS_13, 16, 0),
    None,
    table(&CODES_15, &LENS_15, 16, 0),
    table(&CODES_16, &LENS_16, 16, 1),
    table(&CODES_16, &LENS_16, 16, 2),
    table(&CODES_16, &LENS_16, 16, 3),
    table(&CODES_16, &LENS_16, 16, 4),
    table(&CODES_16, &LENS_16, 16, 6),
    table(&CODES_16, &LENS_16, 16, 8),
    table(&CODES_16, &LENS_16, 16, 10),
    table(&CODES_16, &LENS_16, 16, 13),
    table(&CODES_24, &LENS_24, 16, 4),
    table(&CODES_24, &LENS_24, 16, 5),
    table(&CODES_24, &LENS_24, 16, 6),
    table(&CODES_24, &LENS_24, 16, 7),
    table(&CODES_24, &LENS_24, 16, 8),
    table(&CODES_24, &LENS_24, 16, 9),
    table(&CODES_24, &LENS_24, 16, 11),
    table(&CODES_24, &LENS_24, 16, 13),
];

/// The largest magnitude any table can code: 15 plus 13 linbits.
pub const MAX_VALUE: u32 = 15 + (1 << 13) - 1;

/// Non-escape tables, grouped by the value range they cover.
const SMALL_TABLES: [&[usize]; 6] = [&[1], &[2, 3], &[5, 6], &[7, 8, 9], &[10, 11, 12], &[13, 15]];

/// Bits needed to code `values` (an even number of lines) with table `t`.
///
/// # Panics
///
/// Panics if `t` has no codes or a value is out of the table's range.
fn pair_bits_with(t: PairTable, values: &[i32]) -> u32 {
    let mut bits = 0;
    for pair in values.chunks_exact(2) {
        let (x, y) = (pair[0].unsigned_abs(), pair[1].unsigned_abs());
        let (cx, cy) = (x.min(15), y.min(15));
        bits += u32::from(t.lens[(cx * t.width + cy) as usize]);
        if t.linbits > 0 {
            bits += t.linbits * (u32::from(x >= 15) + u32::from(y >= 15));
        }
        bits += u32::from(x != 0) + u32::from(y != 0);
    }
    bits
}

/// Bits needed to code `values` with pair table `table`; table 0 costs nothing.
pub fn pair_bits(table: usize, values: &[i32]) -> u32 {
    match PAIR_TABLES[table] {
        Some(t) => pair_bits_with(t, values),
        None => 0,
    }
}

/// Picks the cheapest pair table able to code `values`, returning it and its cost.
/// Returns table 0 when every value is zero.
///
/// # Panics
///
/// Panics if a magnitude exceeds [`MAX_VALUE`].
pub fn best_pair_table(values: &[i32]) -> (usize, u32) {
    let max = values.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
    assert!(max <= MAX_VALUE, "value {max} is too large to Huffman code");
    if max == 0 {
        return (0, 0);
    }

    let mut best = (usize::MAX, u32::MAX);
    let mut consider = |table: usize| {
        let bits = pair_bits(table, values);
        if bits < best.1 {
            best = (table, bits);
        }
    };

    if max < 16 {
        // The smallest range class that fits, and the next one up.
        let first = SMALL_TABLES
            .iter()
            .position(|group| PAIR_TABLES[group[0]].unwrap().max_value() >= max)
            .expect("tables 13 and 15 cover 0..=15");
        for group in &SMALL_TABLES[first..(first + 2).min(SMALL_TABLES.len())] {
            group.iter().copied().for_each(&mut consider);
        }
    } else {
        // In each escape family, the table with the fewest linbits that fits.
        for family in [16..24, 24..32] {
            if let Some(t) = family
                .clone()
                .find(|&t| PAIR_TABLES[t].unwrap().max_value() >= max)
            {
                consider(t);
            }
        }
    }
    best
}

/// Writes `values` (an even number of lines) with pair table `table`.
/// Table 0 writes nothing.
pub fn write_pairs(w: &mut BitWriter, table: usize, values: &[i32]) {
    let Some(t) = PAIR_TABLES[table] else {
        debug_assert!(values.iter().all(|&v| v == 0));
        return;
    };
    for pair in values.chunks_exact(2) {
        let (x, y) = (pair[0], pair[1]);
        let (ax, ay) = (x.unsigned_abs(), y.unsigned_abs());
        let (cx, cy) = (ax.min(15), ay.min(15));
        let index = (cx * t.width + cy) as usize;
        w.write_bits(t.codes[index], u32::from(t.lens[index]));
        if t.linbits > 0 && ax >= 15 {
            w.write_bits(ax - 15, t.linbits);
        }
        if ax != 0 {
            w.write_bit(x < 0);
        }
        if t.linbits > 0 && ay >= 15 {
            w.write_bits(ay - 15, t.linbits);
        }
        if ay != 0 {
            w.write_bit(y < 0);
        }
    }
}

fn quad_index(quad: &[i32]) -> usize {
    quad.iter()
        .fold(0, |acc, v| (acc << 1) | v.unsigned_abs() as usize)
}

/// Bits needed to code `values` (a multiple of 4 lines, each in `-1..=1`)
/// with count1 table A (`false`) or B (`true`).
pub fn quad_bits(table_b: bool, values: &[i32]) -> u32 {
    values
        .chunks_exact(4)
        .map(|quad| {
            let signs = quad.iter().filter(|&&v| v != 0).count() as u32;
            let code = if table_b {
                4
            } else {
                u32::from(QUAD_A_LENS[quad_index(quad)])
            };
            code + signs
        })
        .sum()
}

/// Writes `values` (a multiple of 4 lines, each in `-1..=1`) with count1
/// table A (`false`) or B (`true`).
pub fn write_quads(w: &mut BitWriter, table_b: bool, values: &[i32]) {
    for quad in values.chunks_exact(4) {
        let index = quad_index(quad);
        if table_b {
            w.write_bits(15 - index as u32, 4);
        } else {
            w.write_bits(QUAD_A_CODES[index], u32::from(QUAD_A_LENS[index]));
        }
        for &v in quad {
            if v != 0 {
                w.write_bit(v < 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads bits back MSB first.
    struct BitReader<'a> {
        bytes: &'a [u8],
        pos: usize,
    }

    impl BitReader<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.bytes[self.pos / 8] >> (7 - self.pos % 8)) & 1;
            self.pos += 1;
            u32::from(b)
        }

        fn bits(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |acc, _| (acc << 1) | self.bit())
        }

        /// Decodes one symbol by matching codes bit by bit.
        fn symbol(&mut self, codes: &[u32], lens: &[u8]) -> usize {
            let (mut code, mut len) = (0u32, 0u8);
            loop {
                code = (code << 1) | self.bit();
                len += 1;
                if let Some(i) = (0..codes.len()).find(|&i| lens[i] == len && codes[i] == code) {
                    return i;
                }
                assert!(len < 20, "no code matched");
            }
        }

        fn signed(&mut self, magnitude: u32) -> i32 {
            match magnitude {
                0 => 0,
                m if self.bit() == 1 => -(m as i32),
                m => m as i32,
            }
        }

        fn pairs(&mut self, table: usize, count: usize) -> Vec<i32> {
            let t = PAIR_TABLES[table].unwrap();
            let mut out = Vec::new();
            for _ in 0..count / 2 {
                let i = self.symbol(t.codes, t.lens) as u32;
                for mut m in [i / t.width, i % t.width] {
                    if t.linbits > 0 && m == 15 {
                        m += self.bits(t.linbits);
                    }
                    out.push(self.signed(m));
                }
            }
            out
        }

        fn quads(&mut self, table_b: bool, count: usize) -> Vec<i32> {
            let mut out = Vec::new();
            for _ in 0..count / 4 {
                let i = if table_b {
                    15 - self.bits(4) as usize
                } else {
                    self.symbol(&QUAD_A_CODES, &QUAD_A_LENS)
                };
                for shift in [3, 2, 1, 0] {
                    let m = (i >> shift) as u32 & 1;
                    out.push(self.signed(m));
                }
            }
            out
        }
    }

    fn pseudo_random(seed: &mut u32, max: u32) -> i32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let m = (*seed >> 8) % (max + 1);
        if (*seed >> 4) & 1 == 1 {
            -(m as i32)
        } else {
            m as i32
        }
    }

    #[test]
    fn every_table_round_trips_and_counts_its_bits_exactly() {
        let mut seed = 7;
        for (table, t) in PAIR_TABLES.iter().enumerate() {
            let Some(t) = *t else {
                continue;
            };
            let mut values: Vec<i32> = (0..400)
                .map(|_| pseudo_random(&mut seed, t.max_value()))
                .collect();
            // Make sure the extremes appear.
            values[0] = t.max_value() as i32;
            values[1] = -(t.max_value() as i32);
            values[2] = 0;
            values[3] = 0;

            let mut w = BitWriter::new();
            write_pairs(&mut w, table, &values);
            assert_eq!(
                w.bit_len() as u32,
                pair_bits(table, &values),
                "table {table}"
            );

            let bytes = w.into_bytes();
            let mut r = BitReader {
                bytes: &bytes,
                pos: 0,
            };
            assert_eq!(r.pairs(table, values.len()), values, "table {table}");
        }
    }

    #[test]
    fn quads_round_trip_with_both_tables() {
        let mut seed = 3;
        let values: Vec<i32> = (0..400).map(|_| pseudo_random(&mut seed, 1)).collect();
        for table_b in [false, true] {
            let mut w = BitWriter::new();
            write_quads(&mut w, table_b, &values);
            assert_eq!(w.bit_len() as u32, quad_bits(table_b, &values));
            let bytes = w.into_bytes();
            let mut r = BitReader {
                bytes: &bytes,
                pos: 0,
            };
            assert_eq!(r.quads(table_b, values.len()), values);
        }
    }

    #[test]
    fn best_table_fits_and_is_cheapest_of_its_candidates() {
        assert_eq!(best_pair_table(&[0, 0, 0, 0]), (0, 0));

        let small = [1, 0, -1, 1, 0, 0];
        let (t, bits) = best_pair_table(&small);
        assert!(PAIR_TABLES[t].unwrap().max_value() >= 1);
        assert_eq!(bits, pair_bits(t, &small));

        let large = [300, -2, 7, 15];
        let (t, bits) = best_pair_table(&large);
        assert!(t >= 16 && PAIR_TABLES[t].unwrap().max_value() >= 300);
        assert_eq!(bits, pair_bits(t, &large));

        let (t, _) = best_pair_table(&[MAX_VALUE as i32, 0]);
        assert!(t == 23 || t == 31);
    }

    #[test]
    #[should_panic(expected = "too large")]
    fn values_beyond_the_escape_range_panic() {
        best_pair_table(&[MAX_VALUE as i32 + 1, 0]);
    }
}
