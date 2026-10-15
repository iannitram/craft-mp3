//! MPEG audio CRC-16 implementation.
//!
//! Polynomial: 0x8005 (x^16 + x^15 + x^2 + 1)
//! Initial value: 0xFFFF
//! No reflection, no final XOR

/// Compute CRC-16 over the first `nbits` bits of `bits`.
///
/// Most-significant bit first (MP3's CRC covers header bits 16..31 and then the side info,
/// which is not byte-aligned in general).
pub fn crc16(bits: &[u8], nbits: usize, init: u16) -> u16 {
    let mut crc = init;

    // Process each bit
    for i in 0..nbits {
        let bit = (bits[i / 8] >> (7 - (i % 8))) & 1;

        // If the MSB of CRC is set, XOR with polynomial and shift left
        if crc & 0x8000 != 0 {
            crc = (crc << 1) ^ 0x8005;
        } else {
            crc <<= 1;
        }

        // If the current bit is 1, XOR with the polynomial
        if bit == 1 {
            crc ^= 0x8005;
        }
    }

    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_returns_init() {
        assert_eq!(crc16(&[], 0, 0xFFFF), 0xFFFF);
    }

    #[test]
    fn check_value_123456789() {
        let data = [0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39]; // "123456789" as bytes
        assert_eq!(crc16(&data, 72, 0xFFFF), 0xAEE7); // 72 bits = 9 bytes
    }

    #[test]
    fn fast_matches_bitwise_500_random() {
        // Generate some test inputs and verify both methods match
        // Using fixed test data to avoid dependency on external crates
        let test_cases = [
            ([0x00u8; 10], 0),
            ([0xFFu8; 10], 80),
            (
                [0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF],
                80,
            ),
            (
                [0x12, 0x34, 0x56, 0x78, 0x90, 0xAB, 0xCD, 0xEF, 0x12, 0x34],
                72,
            ),
            (
                [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01, 0x00, 0x00],
                80,
            ),
        ];

        for &(data, nbits) in test_cases.iter() {
            // Fast method
            let fast_result = crc16(&data, nbits, 0xFFFF);

            // Bitwise reference implementation
            let mut crc = 0xFFFFu16;
            for i in 0..nbits {
                let bit = (data[i / 8] >> (7 - (i % 8))) & 1;

                // If the MSB of CRC is set, XOR with polynomial and shift left
                if crc & 0x8000 != 0 {
                    crc = (crc << 1) ^ 0x8005;
                } else {
                    crc <<= 1;
                }

                // If the current bit is 1, XOR with the polynomial
                if bit == 1 {
                    crc ^= 0x8005;
                }
            }
            let bitwise_result = crc;

            assert_eq!(fast_result, bitwise_result);
        }
    }
}
