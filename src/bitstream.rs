//! MSB-first bit packing into a byte buffer.
//!
//! Every field of an MP3 frame (header, side info, scale factors, Huffman
//! codes) is written most significant bit first, with no alignment between
//! fields. [`BitWriter`] is the single place that does that packing.

/// Appends bit fields, most significant bit first, to a growing byte buffer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BitWriter {
    bytes: Vec<u8>,
    /// Bits already used in the last byte of `bytes` (0 means byte-aligned).
    used: u32,
}

impl BitWriter {
    /// Creates an empty writer.
    #[cfg(test)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty writer with room for `bytes` bytes.
    pub fn with_capacity(bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(bytes),
            used: 0,
        }
    }

    /// Writes the low `n` bits of `value`, most significant first.
    ///
    /// # Panics
    ///
    /// Panics if `n > 32`, or if `value` has bits set above the low `n`.
    pub fn write_bits(&mut self, value: u32, n: u32) {
        assert!(n <= 32, "cannot write {n} bits at once");
        assert!(
            n == 32 || value >> n == 0,
            "value {value:#x} does not fit in {n} bits"
        );

        let mut remaining = n;
        while remaining > 0 {
            if self.used == 0 {
                self.bytes.push(0);
            }
            let free = 8 - self.used;
            let take = free.min(remaining);
            // The next `take` bits of `value`, counting from the top of the field.
            let chunk = (value >> (remaining - take)) & ((1 << take) - 1);
            let last = self.bytes.last_mut().expect("a byte was pushed above");
            *last |= (chunk << (free - take)) as u8;
            self.used = (self.used + take) % 8;
            remaining -= take;
        }
    }

    /// Writes a single bit.
    pub fn write_bit(&mut self, bit: bool) {
        self.write_bits(u32::from(bit), 1);
    }

    /// Total number of bits written so far.
    #[cfg(test)]
    pub fn bit_len(&self) -> usize {
        match self.used {
            0 => self.bytes.len() * 8,
            used => (self.bytes.len() - 1) * 8 + used as usize,
        }
    }

    /// Whether the next bit written will start a new byte.
    #[cfg(test)]
    pub fn is_byte_aligned(&self) -> bool {
        self.used == 0
    }

    /// Pads with zero bits up to the next byte boundary.
    pub fn align_to_byte(&mut self) {
        self.used = 0;
    }

    /// The bytes written so far; a partial last byte is zero-padded.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the writer, returning its bytes with the last byte zero-padded.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_writer() {
        let w = BitWriter::new();
        assert_eq!(w.bit_len(), 0);
        assert!(w.is_byte_aligned());
        assert!(w.as_bytes().is_empty());
    }

    #[test]
    fn whole_bytes() {
        let mut w = BitWriter::new();
        w.write_bits(0xAB, 8);
        w.write_bits(0xCDEF, 16);
        assert_eq!(w.as_bytes(), [0xAB, 0xCD, 0xEF]);
        assert_eq!(w.bit_len(), 24);
        assert!(w.is_byte_aligned());
    }

    #[test]
    fn fields_straddle_byte_boundaries() {
        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        w.write_bits(0b11001, 5);
        w.write_bits(0b1, 1);
        w.write_bits(0b0110011, 7);
        assert_eq!(w.as_bytes(), [0b1011_1001, 0b1011_0011]);
        assert_eq!(w.bit_len(), 16);
    }

    #[test]
    fn partial_byte_is_zero_padded() {
        let mut w = BitWriter::new();
        w.write_bits(0b11, 2);
        assert_eq!(w.bit_len(), 2);
        assert!(!w.is_byte_aligned());
        assert_eq!(w.as_bytes(), [0b1100_0000]);
    }

    #[test]
    fn single_bits() {
        let mut w = BitWriter::new();
        for bit in [true, false, true, true, false, false, true, false, true] {
            w.write_bit(bit);
        }
        assert_eq!(w.as_bytes(), [0b1011_0010, 0b1000_0000]);
        assert_eq!(w.bit_len(), 9);
    }

    #[test]
    fn full_32_bit_word_unaligned() {
        let mut w = BitWriter::new();
        w.write_bits(0b1, 1);
        w.write_bits(0xFFFF_FFFF, 32);
        assert_eq!(w.as_bytes(), [0xFF, 0xFF, 0xFF, 0xFF, 0x80]);
        assert_eq!(w.bit_len(), 33);
    }

    #[test]
    fn zero_width_write_is_a_no_op() {
        let mut w = BitWriter::new();
        w.write_bits(0, 0);
        assert_eq!(w.bit_len(), 0);
        assert!(w.as_bytes().is_empty());
    }

    #[test]
    fn align_to_byte() {
        let mut w = BitWriter::new();
        w.write_bits(0b1, 1);
        w.align_to_byte();
        assert!(w.is_byte_aligned());
        assert_eq!(w.bit_len(), 8);
        w.write_bits(0xFF, 8);
        assert_eq!(w.into_bytes(), [0x80, 0xFF]);

        // Aligning an already aligned writer adds nothing.
        let mut w = BitWriter::new();
        w.write_bits(0xAA, 8);
        w.align_to_byte();
        assert_eq!(w.bit_len(), 8);
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn value_wider_than_field_panics() {
        BitWriter::new().write_bits(0b100, 2);
    }

    #[test]
    #[should_panic(expected = "cannot write 33 bits")]
    fn more_than_32_bits_panics() {
        BitWriter::new().write_bits(0, 33);
    }
}
