//! The 32-bit MPEG-1 Layer III frame header and frame sizing.
//!
//! Layout (ISO/IEC 11172-3, 2.4.1.3), most significant bit first:
//!
//! | bits | field              |
//! |-----:|--------------------|
//! |   11 | sync word, all 1s  |
//! |    2 | version, `11` = MPEG-1 |
//! |    2 | layer, `01` = Layer III |
//! |    1 | protection, `0` = CRC follows |
//! |    4 | bitrate index      |
//! |    2 | sampling frequency |
//! |    1 | padding            |
//! |    1 | private            |
//! |    2 | mode               |
//! |    2 | mode extension     |
//! |    1 | copyright          |
//! |    1 | original           |
//! |    2 | emphasis           |

use std::fmt;

use crate::bitstream::BitWriter;

/// PCM samples per channel in one Layer III frame.
pub const SAMPLES_PER_FRAME: usize = 1152;

/// Length of the frame header in bytes.
pub const HEADER_LEN: usize = 4;

/// Length of the optional CRC-16 that follows the header, in bytes.
pub const CRC_LEN: usize = 2;

const SYNC: u32 = 0x7FF;
const VERSION_MPEG1: u32 = 0b11;
const LAYER_III: u32 = 0b01;

/// Bitrates in kbit/s for bitrate indices 1 through 14.
/// Index 0 (free format) and 15 (forbidden) are not supported.
const BITRATES_KBPS: [u16; 14] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];

/// A bitrate allowed in an MPEG-1 Layer III frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bitrate(u16);

impl Bitrate {
    /// Every supported bitrate, lowest first.
    pub fn all() -> impl Iterator<Item = Bitrate> {
        BITRATES_KBPS.iter().map(|&kbps| Bitrate(kbps))
    }

    /// Returns the bitrate for `kbps` if Layer III allows it.
    pub fn from_kbps(kbps: u32) -> Option<Self> {
        BITRATES_KBPS
            .iter()
            .find(|&&allowed| u32::from(allowed) == kbps)
            .map(|&allowed| Bitrate(allowed))
    }

    /// The bitrate in kbit/s.
    pub fn kbps(self) -> u32 {
        u32::from(self.0)
    }

    /// The bitrate in bit/s.
    pub fn bps(self) -> u32 {
        self.kbps() * 1000
    }

    fn index(self) -> u32 {
        let pos = BITRATES_KBPS.iter().position(|&kbps| kbps == self.0);
        pos.expect("Bitrate only holds table values") as u32 + 1
    }

    fn from_index(index: u32) -> Option<Self> {
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        BITRATES_KBPS.get(i).map(|&kbps| Bitrate(kbps))
    }
}

/// A sampling frequency allowed in MPEG-1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleRate {
    Hz32000,
    Hz44100,
    Hz48000,
}

impl SampleRate {
    /// Returns the sample rate for `hz` if MPEG-1 allows it.
    pub fn from_hz(hz: u32) -> Option<Self> {
        match hz {
            32_000 => Some(Self::Hz32000),
            44_100 => Some(Self::Hz44100),
            48_000 => Some(Self::Hz48000),
            _ => None,
        }
    }

    /// The sample rate in Hz.
    pub fn hz(self) -> u32 {
        match self {
            Self::Hz32000 => 32_000,
            Self::Hz44100 => 44_100,
            Self::Hz48000 => 48_000,
        }
    }

    fn bits(self) -> u32 {
        match self {
            Self::Hz44100 => 0b00,
            Self::Hz48000 => 0b01,
            Self::Hz32000 => 0b10,
        }
    }

    fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            0b00 => Some(Self::Hz44100),
            0b01 => Some(Self::Hz48000),
            0b10 => Some(Self::Hz32000),
            _ => None,
        }
    }
}

/// The channel mode, including the Layer III mode extension for joint stereo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelMode {
    Stereo,
    /// Joint stereo; the flags say which joint coding this frame uses.
    JointStereo {
        mid_side: bool,
        intensity: bool,
    },
    DualChannel,
    Mono,
}

impl ChannelMode {
    /// Number of coded channels.
    pub fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            _ => 2,
        }
    }

    fn mode_bits(self) -> u32 {
        match self {
            Self::Stereo => 0b00,
            Self::JointStereo { .. } => 0b01,
            Self::DualChannel => 0b10,
            Self::Mono => 0b11,
        }
    }

    fn extension_bits(self) -> u32 {
        match self {
            Self::JointStereo {
                mid_side,
                intensity,
            } => (u32::from(mid_side) << 1) | u32::from(intensity),
            _ => 0,
        }
    }

    /// The mode extension only has meaning in joint stereo and is ignored otherwise.
    fn from_bits(mode: u32, extension: u32) -> Self {
        match mode {
            0b00 => Self::Stereo,
            0b01 => Self::JointStereo {
                mid_side: extension & 0b10 != 0,
                intensity: extension & 0b01 != 0,
            },
            0b10 => Self::DualChannel,
            _ => Self::Mono,
        }
    }
}

/// De-emphasis the decoder should apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Emphasis {
    #[default]
    None,
    /// 50/15 µs.
    Us50_15,
    /// CCITT J.17.
    CcittJ17,
}

impl Emphasis {
    fn bits(self) -> u32 {
        match self {
            Self::None => 0b00,
            Self::Us50_15 => 0b01,
            Self::CcittJ17 => 0b11,
        }
    }

    fn from_bits(bits: u32) -> Option<Self> {
        match bits {
            0b00 => Some(Self::None),
            0b01 => Some(Self::Us50_15),
            0b11 => Some(Self::CcittJ17),
            _ => None,
        }
    }
}

/// Why four bytes are not a supported frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// The 11-bit sync word is not all ones.
    NoSync,
    /// Not MPEG-1 (MPEG-2, MPEG-2.5 or the reserved version).
    UnsupportedVersion,
    /// Not Layer III.
    UnsupportedLayer,
    /// Free-format (index 0) or forbidden (index 15) bitrate.
    UnsupportedBitrate,
    /// The reserved sampling frequency index.
    ReservedSampleRate,
    /// The reserved emphasis value.
    ReservedEmphasis,
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSync => "frame sync word not found",
            Self::UnsupportedVersion => "not an MPEG-1 frame",
            Self::UnsupportedLayer => "not a Layer III frame",
            Self::UnsupportedBitrate => "free-format or forbidden bitrate index",
            Self::ReservedSampleRate => "reserved sampling frequency index",
            Self::ReservedEmphasis => "reserved emphasis value",
        })
    }
}

impl std::error::Error for HeaderError {}

/// One MPEG-1 Layer III frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameHeader {
    pub bitrate: Bitrate,
    pub sample_rate: SampleRate,
    pub mode: ChannelMode,
    /// A CRC-16 follows the header.
    pub crc: bool,
    /// The frame carries one extra byte; see [`PaddingSchedule`].
    pub padding: bool,
    pub private: bool,
    pub copyright: bool,
    pub original: bool,
    pub emphasis: Emphasis,
}

impl FrameHeader {
    /// A header with no CRC, no padding and all flags cleared.
    pub fn new(bitrate: Bitrate, sample_rate: SampleRate, mode: ChannelMode) -> Self {
        Self {
            bitrate,
            sample_rate,
            mode,
            crc: false,
            padding: false,
            private: false,
            copyright: false,
            original: false,
            emphasis: Emphasis::None,
        }
    }

    /// Appends the 32 header bits to `w`.
    pub(crate) fn write(&self, w: &mut BitWriter) {
        w.write_bits(SYNC, 11);
        w.write_bits(VERSION_MPEG1, 2);
        w.write_bits(LAYER_III, 2);
        // The protection bit is inverted: 0 means a CRC follows.
        w.write_bit(!self.crc);
        w.write_bits(self.bitrate.index(), 4);
        w.write_bits(self.sample_rate.bits(), 2);
        w.write_bit(self.padding);
        w.write_bit(self.private);
        w.write_bits(self.mode.mode_bits(), 2);
        w.write_bits(self.mode.extension_bits(), 2);
        w.write_bit(self.copyright);
        w.write_bit(self.original);
        w.write_bits(self.emphasis.bits(), 2);
    }

    /// The header as four bytes.
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut w = BitWriter::with_capacity(HEADER_LEN);
        self.write(&mut w);
        w.as_bytes()
            .try_into()
            .expect("a header is exactly 32 bits")
    }

    /// Parses four header bytes.
    ///
    /// The mode extension is ignored unless the mode is joint stereo.
    pub fn parse(bytes: [u8; HEADER_LEN]) -> Result<Self, HeaderError> {
        let word = u32::from_be_bytes(bytes);
        let field = |shift: u32, width: u32| (word >> shift) & ((1 << width) - 1);

        if field(21, 11) != SYNC {
            return Err(HeaderError::NoSync);
        }
        if field(19, 2) != VERSION_MPEG1 {
            return Err(HeaderError::UnsupportedVersion);
        }
        if field(17, 2) != LAYER_III {
            return Err(HeaderError::UnsupportedLayer);
        }
        let bitrate = Bitrate::from_index(field(12, 4)).ok_or(HeaderError::UnsupportedBitrate)?;
        let sample_rate =
            SampleRate::from_bits(field(10, 2)).ok_or(HeaderError::ReservedSampleRate)?;
        let emphasis = Emphasis::from_bits(field(0, 2)).ok_or(HeaderError::ReservedEmphasis)?;

        Ok(Self {
            bitrate,
            sample_rate,
            mode: ChannelMode::from_bits(field(6, 2), field(4, 2)),
            crc: field(16, 1) == 0,
            padding: field(9, 1) == 1,
            private: field(8, 1) == 1,
            copyright: field(3, 1) == 1,
            original: field(2, 1) == 1,
            emphasis,
        })
    }

    /// Total frame length in bytes, header and padding included.
    pub fn frame_len(&self) -> usize {
        unpadded_frame_len(self.bitrate, self.sample_rate) + usize::from(self.padding)
    }

    /// Length of the Layer III side information in bytes.
    pub fn side_info_len(&self) -> usize {
        match self.mode.channels() {
            1 => 17,
            _ => 32,
        }
    }

    /// Bytes left for main data after the header, CRC and side information.
    pub fn main_data_len(&self) -> usize {
        let crc = if self.crc { CRC_LEN } else { 0 };
        self.frame_len() - HEADER_LEN - crc - self.side_info_len()
    }
}

/// The CRC-16 protecting a frame (2.4.3.1): polynomial 0x8005, initial
/// value 0xFFFF, over the last two header bytes and the side information.
pub fn frame_crc(header: &[u8; HEADER_LEN], side_info: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &byte in header[2..].iter().chain(side_info) {
        for bit in (0..8).rev() {
            let input = (byte >> bit) & 1 == 1;
            let top = crc & 0x8000 != 0;
            crc <<= 1;
            if input != top {
                crc ^= 0x8005;
            }
        }
    }
    crc
}

/// Frame length in bytes without the padding byte: `floor(144 * bitrate / sample_rate)`.
fn unpadded_frame_len(bitrate: Bitrate, sample_rate: SampleRate) -> usize {
    (bytes_per_frame_numerator(bitrate) / u64::from(sample_rate.hz())) as usize
}

/// `144 * bitrate`, i.e. 1152 samples / 8 bits per byte, times bit/s.
fn bytes_per_frame_numerator(bitrate: Bitrate) -> u64 {
    144 * u64::from(bitrate.bps())
}

/// Decides which frames carry the padding byte so the stream averages exactly
/// the nominal bitrate.
///
/// After `n` frames the total length is exactly `floor(n * 144 * bitrate / sample_rate)`
/// bytes. At 32 and 48 kHz every bitrate divides evenly and no frame is padded.
#[derive(Debug, Clone)]
pub struct PaddingSchedule {
    /// `144 * bitrate mod sample_rate`: the fractional byte each frame owes.
    remainder: u64,
    sample_rate: u64,
    /// Accumulated fractional bytes, always below `sample_rate`.
    owed: u64,
}

impl PaddingSchedule {
    pub fn new(bitrate: Bitrate, sample_rate: SampleRate) -> Self {
        let sample_rate = u64::from(sample_rate.hz());
        Self {
            remainder: bytes_per_frame_numerator(bitrate) % sample_rate,
            sample_rate,
            owed: 0,
        }
    }

    /// Whether the next frame should be padded.
    pub fn next_padding(&mut self) -> bool {
        self.owed += self.remainder;
        if self.owed >= self.sample_rate {
            self.owed -= self.sample_rate;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kbps(kbps: u32) -> Bitrate {
        Bitrate::from_kbps(kbps).unwrap()
    }

    #[test]
    fn bitrate_table() {
        assert_eq!(Bitrate::all().count(), 14);
        assert_eq!(Bitrate::from_kbps(128).unwrap().index(), 9);
        assert_eq!(Bitrate::from_kbps(32).unwrap().index(), 1);
        assert_eq!(Bitrate::from_kbps(320).unwrap().index(), 14);
        assert_eq!(Bitrate::from_kbps(0), None);
        assert_eq!(Bitrate::from_kbps(8), None);
        assert_eq!(Bitrate::from_kbps(144), None);
        assert_eq!(Bitrate::from_index(0), None);
        assert_eq!(Bitrate::from_index(15), None);
        for b in Bitrate::all() {
            assert_eq!(Bitrate::from_index(b.index()), Some(b));
        }
    }

    #[test]
    fn sample_rates() {
        assert_eq!(SampleRate::from_hz(44_100), Some(SampleRate::Hz44100));
        assert_eq!(SampleRate::from_hz(22_050), None);
        assert_eq!(SampleRate::Hz48000.hz(), 48_000);
    }

    #[test]
    fn known_header_bytes() {
        // The plain header most encoders emit for 128 kbit/s, 44.1 kHz stereo.
        let h = FrameHeader::new(kbps(128), SampleRate::Hz44100, ChannelMode::Stereo);
        assert_eq!(h.to_bytes(), [0xFF, 0xFB, 0x90, 0x00]);

        // LAME's typical joint-stereo header with M/S on and the original flag set.
        let h = FrameHeader {
            original: true,
            ..FrameHeader::new(
                kbps(128),
                SampleRate::Hz44100,
                ChannelMode::JointStereo {
                    mid_side: true,
                    intensity: false,
                },
            )
        };
        assert_eq!(h.to_bytes(), [0xFF, 0xFB, 0x90, 0x64]);

        // 320 kbit/s, 48 kHz, mono, CRC, padded, every flag and emphasis CCITT.
        let h = FrameHeader {
            crc: true,
            padding: true,
            private: true,
            copyright: true,
            original: true,
            emphasis: Emphasis::CcittJ17,
            ..FrameHeader::new(kbps(320), SampleRate::Hz48000, ChannelMode::Mono)
        };
        assert_eq!(h.to_bytes(), [0xFF, 0xFA, 0xE7, 0xCF]);
    }

    #[test]
    fn write_appends_to_existing_bits() {
        let h = FrameHeader::new(kbps(128), SampleRate::Hz44100, ChannelMode::Stereo);
        let mut w = BitWriter::new();
        w.write_bits(0xAB, 8);
        h.write(&mut w);
        assert_eq!(w.as_bytes(), [0xAB, 0xFF, 0xFB, 0x90, 0x00]);
    }

    #[test]
    fn parse_round_trips_every_field() {
        let modes = [
            ChannelMode::Stereo,
            ChannelMode::JointStereo {
                mid_side: false,
                intensity: false,
            },
            ChannelMode::JointStereo {
                mid_side: true,
                intensity: false,
            },
            ChannelMode::JointStereo {
                mid_side: false,
                intensity: true,
            },
            ChannelMode::JointStereo {
                mid_side: true,
                intensity: true,
            },
            ChannelMode::DualChannel,
            ChannelMode::Mono,
        ];
        let rates = [
            SampleRate::Hz32000,
            SampleRate::Hz44100,
            SampleRate::Hz48000,
        ];
        let emphases = [Emphasis::None, Emphasis::Us50_15, Emphasis::CcittJ17];
        for bitrate in Bitrate::all() {
            for sample_rate in rates {
                for mode in modes {
                    for emphasis in emphases {
                        for flags in 0..32u8 {
                            let h = FrameHeader {
                                bitrate,
                                sample_rate,
                                mode,
                                crc: flags & 1 != 0,
                                padding: flags & 2 != 0,
                                private: flags & 4 != 0,
                                copyright: flags & 8 != 0,
                                original: flags & 16 != 0,
                                emphasis,
                            };
                            assert_eq!(FrameHeader::parse(h.to_bytes()), Ok(h));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn parse_rejects_unsupported_headers() {
        use HeaderError::*;
        let good = [0xFF, 0xFB, 0x90, 0x00];
        assert!(FrameHeader::parse(good).is_ok());

        let cases: [([u8; 4], HeaderError); 9] = [
            ([0xFF, 0x1B, 0x90, 0x00], NoSync),
            ([0x00, 0x00, 0x00, 0x00], NoSync),
            ([0xFF, 0xF3, 0x90, 0x00], UnsupportedVersion), // MPEG-2
            ([0xFF, 0xE3, 0x90, 0x00], UnsupportedVersion), // MPEG-2.5
            ([0xFF, 0xFD, 0x90, 0x00], UnsupportedLayer),   // Layer II
            ([0xFF, 0xF9, 0x90, 0x00], UnsupportedLayer),   // reserved layer
            ([0xFF, 0xFB, 0x00, 0x00], UnsupportedBitrate), // free format
            ([0xFF, 0xFB, 0xF0, 0x00], UnsupportedBitrate), // forbidden
            ([0xFF, 0xFB, 0x9C, 0x00], ReservedSampleRate),
        ];
        for (bytes, err) in cases {
            assert_eq!(FrameHeader::parse(bytes), Err(err), "{bytes:02X?}");
        }
        assert_eq!(
            FrameHeader::parse([0xFF, 0xFB, 0x90, 0x02]),
            Err(ReservedEmphasis)
        );
    }

    #[test]
    fn parse_ignores_mode_extension_outside_joint_stereo() {
        let h = FrameHeader::parse([0xFF, 0xFB, 0x90, 0x30]).unwrap();
        assert_eq!(h.mode, ChannelMode::Stereo);
    }

    #[test]
    fn crc_matches_the_standard_algorithm() {
        // CRC-16/CMS (poly 0x8005, init 0xFFFF, unreflected) of "123456789" is 0xAEE7.
        let header = [0, 0, b'1', b'2'];
        assert_eq!(frame_crc(&header, b"3456789"), 0xAEE7);
        // The first two header bytes (sync, version, layer) are not covered.
        assert_eq!(frame_crc(&[0xFF, 0xFB, b'1', b'2'], b"3456789"), 0xAEE7);
    }

    #[test]
    fn frame_lengths() {
        let len = |kbps_: u32, rate, padding| {
            FrameHeader {
                padding,
                ..FrameHeader::new(kbps(kbps_), rate, ChannelMode::Stereo)
            }
            .frame_len()
        };
        assert_eq!(len(128, SampleRate::Hz44100, false), 417);
        assert_eq!(len(128, SampleRate::Hz44100, true), 418);
        assert_eq!(len(320, SampleRate::Hz44100, false), 1044);
        assert_eq!(len(32, SampleRate::Hz44100, false), 104);
        assert_eq!(len(320, SampleRate::Hz48000, false), 960);
        assert_eq!(len(128, SampleRate::Hz48000, false), 384);
        assert_eq!(len(32, SampleRate::Hz32000, false), 144);
        assert_eq!(len(320, SampleRate::Hz32000, false), 1440);
    }

    #[test]
    fn side_info_and_main_data_lengths() {
        let mono = FrameHeader::new(kbps(128), SampleRate::Hz48000, ChannelMode::Mono);
        assert_eq!(mono.side_info_len(), 17);
        assert_eq!(mono.main_data_len(), 384 - 4 - 17);

        let stereo = FrameHeader {
            crc: true,
            ..FrameHeader::new(kbps(128), SampleRate::Hz48000, ChannelMode::DualChannel)
        };
        assert_eq!(stereo.side_info_len(), 32);
        assert_eq!(stereo.main_data_len(), 384 - 4 - 2 - 32);

        // The smallest frame still has room for main data.
        let smallest = FrameHeader {
            crc: true,
            ..FrameHeader::new(kbps(32), SampleRate::Hz48000, ChannelMode::Stereo)
        };
        assert_eq!(smallest.main_data_len(), 96 - 4 - 2 - 32);
    }

    #[test]
    fn padding_keeps_the_exact_average_bitrate() {
        for bitrate in Bitrate::all() {
            for rate in [
                SampleRate::Hz32000,
                SampleRate::Hz44100,
                SampleRate::Hz48000,
            ] {
                let mut schedule = PaddingSchedule::new(bitrate, rate);
                let mut total = 0u64;
                for n in 1..=2_000u64 {
                    let padding = schedule.next_padding();
                    let h = FrameHeader {
                        padding,
                        ..FrameHeader::new(bitrate, rate, ChannelMode::Stereo)
                    };
                    total += h.frame_len() as u64;
                    let exact = n * 144 * u64::from(bitrate.bps()) / u64::from(rate.hz());
                    assert_eq!(total, exact, "{bitrate:?} at {rate:?}, frame {n}");
                }
            }
        }
    }

    #[test]
    fn no_padding_at_32_and_48_khz() {
        for bitrate in Bitrate::all() {
            for rate in [SampleRate::Hz32000, SampleRate::Hz48000] {
                let mut schedule = PaddingSchedule::new(bitrate, rate);
                assert!((0..100).all(|_| !schedule.next_padding()));
            }
        }
    }

    #[test]
    fn padding_pattern_at_128k_44k1() {
        // 144 * 128000 / 44100 = 417 + 42300/44100, so almost every frame pads.
        let mut schedule = PaddingSchedule::new(kbps(128), SampleRate::Hz44100);
        let pattern: Vec<bool> = (0..6).map(|_| schedule.next_padding()).collect();
        assert_eq!(pattern, [false, true, true, true, true, true]);
    }
}
