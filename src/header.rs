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

/// PCM samples per channel in one MPEG-1 Layer III frame (MPEG-2 frames,
/// at the low sample rates, hold half as many; see
/// [`FrameHeader::samples`]).
pub const SAMPLES_PER_FRAME: usize = 1152;

/// Length of the frame header in bytes.
pub const HEADER_LEN: usize = 4;

/// Length of the optional CRC-16 that follows the header, in bytes.
pub const CRC_LEN: usize = 2;

const SYNC: u32 = 0x7FF;
const VERSION_MPEG1: u32 = 0b11;
const VERSION_MPEG2: u32 = 0b10;
const LAYER_III: u32 = 0b01;

/// Bitrates in kbit/s for bitrate indices 1 through 14, for MPEG-1 and for
/// MPEG-2 at the low sample rates. Index 0 (free format) and 15
/// (forbidden) are not supported.
const MPEG1_KBPS: [u16; 14] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
const MPEG2_KBPS: [u16; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

fn bitrate_table(lsf: bool) -> &'static [u16; 14] {
    if lsf { &MPEG2_KBPS } else { &MPEG1_KBPS }
}

/// A Layer III bitrate. Which ones a stream may use depends on its sample
/// rate; see [`Bitrate::all`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bitrate(u16);

impl Bitrate {
    /// Every bitrate a stream at `sample_rate` can use, lowest first.
    pub fn all(sample_rate: SampleRate) -> impl Iterator<Item = Bitrate> {
        bitrate_table(sample_rate.is_lsf())
            .iter()
            .map(|&kbps| Bitrate(kbps))
    }

    /// Returns the bitrate for `kbps` if Layer III allows it at some
    /// sample rate.
    pub fn from_kbps(kbps: u32) -> Option<Self> {
        MPEG1_KBPS
            .iter()
            .chain(&MPEG2_KBPS)
            .find(|&&allowed| u32::from(allowed) == kbps)
            .map(|&allowed| Bitrate(allowed))
    }

    /// Whether a stream at `sample_rate` can use this bitrate.
    pub fn is_allowed_at(self, sample_rate: SampleRate) -> bool {
        bitrate_table(sample_rate.is_lsf()).contains(&self.0)
    }

    /// The bitrate in kbit/s.
    pub fn kbps(self) -> u32 {
        u32::from(self.0)
    }

    /// The bitrate in bit/s.
    pub fn bps(self) -> u32 {
        self.kbps() * 1000
    }

    /// # Panics
    ///
    /// Panics if the bitrate isn't allowed for this MPEG version.
    fn index(self, lsf: bool) -> u32 {
        let pos = bitrate_table(lsf).iter().position(|&kbps| kbps == self.0);
        pos.expect("bitrate is not allowed at this sample rate") as u32 + 1
    }

    fn from_index(index: u32, lsf: bool) -> Option<Self> {
        let i = usize::try_from(index).ok()?.checked_sub(1)?;
        bitrate_table(lsf).get(i).map(|&kbps| Bitrate(kbps))
    }
}

/// A sampling frequency: MPEG-1's three, or MPEG-2's low sampling
/// frequencies (LSF), which halve the frame length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SampleRate {
    Hz16000,
    Hz22050,
    Hz24000,
    Hz32000,
    Hz44100,
    Hz48000,
}

impl SampleRate {
    /// Returns the sample rate for `hz` if MPEG-1 or MPEG-2 allows it.
    pub fn from_hz(hz: u32) -> Option<Self> {
        match hz {
            16_000 => Some(Self::Hz16000),
            22_050 => Some(Self::Hz22050),
            24_000 => Some(Self::Hz24000),
            32_000 => Some(Self::Hz32000),
            44_100 => Some(Self::Hz44100),
            48_000 => Some(Self::Hz48000),
            _ => None,
        }
    }

    /// The sample rate in Hz.
    pub fn hz(self) -> u32 {
        match self {
            Self::Hz16000 => 16_000,
            Self::Hz22050 => 22_050,
            Self::Hz24000 => 24_000,
            Self::Hz32000 => 32_000,
            Self::Hz44100 => 44_100,
            Self::Hz48000 => 48_000,
        }
    }

    /// Whether this is an MPEG-2 low sampling frequency.
    pub fn is_lsf(self) -> bool {
        matches!(self, Self::Hz16000 | Self::Hz22050 | Self::Hz24000)
    }

    fn bits(self) -> u32 {
        match self {
            Self::Hz44100 | Self::Hz22050 => 0b00,
            Self::Hz48000 | Self::Hz24000 => 0b01,
            Self::Hz32000 | Self::Hz16000 => 0b10,
        }
    }

    fn from_bits(bits: u32, lsf: bool) -> Option<Self> {
        match (bits, lsf) {
            (0b00, false) => Some(Self::Hz44100),
            (0b01, false) => Some(Self::Hz48000),
            (0b10, false) => Some(Self::Hz32000),
            (0b00, true) => Some(Self::Hz22050),
            (0b01, true) => Some(Self::Hz24000),
            (0b10, true) => Some(Self::Hz16000),
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
    /// Neither MPEG-1 nor MPEG-2 (MPEG-2.5 or the reserved version).
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
            Self::UnsupportedVersion => "not an MPEG-1 or MPEG-2 frame",
            Self::UnsupportedLayer => "not a Layer III frame",
            Self::UnsupportedBitrate => "free-format or forbidden bitrate index",
            Self::ReservedSampleRate => "reserved sampling frequency index",
            Self::ReservedEmphasis => "reserved emphasis value",
        })
    }
}

impl std::error::Error for HeaderError {}

/// One Layer III frame header (MPEG-1, or MPEG-2 at the low sample rates).
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
        let lsf = self.sample_rate.is_lsf();
        w.write_bits(if lsf { VERSION_MPEG2 } else { VERSION_MPEG1 }, 2);
        w.write_bits(LAYER_III, 2);
        // The protection bit is inverted: 0 means a CRC follows.
        w.write_bit(!self.crc);
        w.write_bits(self.bitrate.index(lsf), 4);
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
        let lsf = match field(19, 2) {
            VERSION_MPEG1 => false,
            VERSION_MPEG2 => true,
            _ => return Err(HeaderError::UnsupportedVersion),
        };
        if field(17, 2) != LAYER_III {
            return Err(HeaderError::UnsupportedLayer);
        }
        let bitrate =
            Bitrate::from_index(field(12, 4), lsf).ok_or(HeaderError::UnsupportedBitrate)?;
        let sample_rate =
            SampleRate::from_bits(field(10, 2), lsf).ok_or(HeaderError::ReservedSampleRate)?;
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

    /// PCM samples per channel in the frame: two granules of 576 for
    /// MPEG-1, one for MPEG-2.
    pub fn samples(&self) -> usize {
        if self.sample_rate.is_lsf() {
            SAMPLES_PER_FRAME / 2
        } else {
            SAMPLES_PER_FRAME
        }
    }

    /// Length of the Layer III side information in bytes.
    pub fn side_info_len(&self) -> usize {
        match (self.sample_rate.is_lsf(), self.mode.channels()) {
            (false, 1) => 17,
            (false, _) => 32,
            (true, 1) => 9,
            (true, _) => 17,
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

/// Frame length in bytes without the padding byte:
/// `floor(144 * bitrate / sample_rate)`, or 72 for MPEG-2.
fn unpadded_frame_len(bitrate: Bitrate, sample_rate: SampleRate) -> usize {
    (bytes_per_frame_numerator(bitrate, sample_rate) / u64::from(sample_rate.hz())) as usize
}

/// Samples per frame / 8 bits per byte, times bit/s.
fn bytes_per_frame_numerator(bitrate: Bitrate, sample_rate: SampleRate) -> u64 {
    let factor = if sample_rate.is_lsf() { 72 } else { 144 };
    factor * u64::from(bitrate.bps())
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
    pub fn new(bitrate: Bitrate, rate: SampleRate) -> Self {
        let sample_rate = u64::from(rate.hz());
        Self {
            remainder: bytes_per_frame_numerator(bitrate, rate) % sample_rate,
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
        assert_eq!(Bitrate::all(SampleRate::Hz44100).count(), 14);
        assert_eq!(Bitrate::from_kbps(128).unwrap().index(false), 9);
        assert_eq!(Bitrate::from_kbps(32).unwrap().index(false), 1);
        assert_eq!(Bitrate::from_kbps(320).unwrap().index(false), 14);
        assert_eq!(Bitrate::from_kbps(0), None);
        assert_eq!(Bitrate::from_kbps(100), None);
        assert_eq!(Bitrate::from_index(0, false), None);
        assert_eq!(Bitrate::from_index(15, false), None);
        for lsf_rate in [SampleRate::Hz44100, SampleRate::Hz22050] {
            let lsf = lsf_rate.is_lsf();
            for b in Bitrate::all(lsf_rate) {
                assert!(b.is_allowed_at(lsf_rate));
                assert_eq!(Bitrate::from_index(b.index(lsf), lsf), Some(b));
            }
        }
        // MPEG-2 has its own table: 8..160, with 144.
        assert_eq!(Bitrate::from_kbps(144).unwrap().index(true), 13);
        assert_eq!(Bitrate::from_kbps(8).unwrap().index(true), 1);
        assert!(
            !Bitrate::from_kbps(8)
                .unwrap()
                .is_allowed_at(SampleRate::Hz44100)
        );
        assert!(
            !Bitrate::from_kbps(320)
                .unwrap()
                .is_allowed_at(SampleRate::Hz24000)
        );
    }

    #[test]
    fn sample_rates() {
        assert_eq!(SampleRate::from_hz(44_100), Some(SampleRate::Hz44100));
        assert_eq!(SampleRate::from_hz(22_050), Some(SampleRate::Hz22050));
        assert_eq!(SampleRate::from_hz(11_025), None);
        assert!(SampleRate::Hz16000.is_lsf() && !SampleRate::Hz32000.is_lsf());
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
    fn mpeg2_low_sample_rate_headers() {
        // 64 kbit/s, 22.05 kHz, mono: version bits 10, bitrate index 8 in
        // the MPEG-2 table.
        let h = FrameHeader::new(kbps(64), SampleRate::Hz22050, ChannelMode::Mono);
        assert_eq!(h.to_bytes(), [0xFF, 0xF3, 0x80, 0xC0]);
        assert_eq!(FrameHeader::parse(h.to_bytes()), Ok(h));
        assert_eq!(h.samples(), 576);
        assert_eq!(h.frame_len(), 72 * 64_000 / 22_050);
        assert_eq!(h.side_info_len(), 9);
        let stereo = FrameHeader {
            mode: ChannelMode::Stereo,
            ..h
        };
        assert_eq!(stereo.side_info_len(), 17);
        assert_eq!(
            FrameHeader::new(kbps(160), SampleRate::Hz16000, ChannelMode::Mono).frame_len(),
            720
        );
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
            SampleRate::Hz16000,
            SampleRate::Hz22050,
            SampleRate::Hz24000,
            SampleRate::Hz32000,
            SampleRate::Hz44100,
            SampleRate::Hz48000,
        ];
        let emphases = [Emphasis::None, Emphasis::Us50_15, Emphasis::CcittJ17];
        for sample_rate in rates {
            for bitrate in Bitrate::all(sample_rate) {
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
            ([0xFF, 0xEB, 0x90, 0x00], UnsupportedVersion), // reserved version
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
        for rate in [
            SampleRate::Hz22050,
            SampleRate::Hz32000,
            SampleRate::Hz44100,
            SampleRate::Hz48000,
        ] {
            for bitrate in Bitrate::all(rate) {
                let mut schedule = PaddingSchedule::new(bitrate, rate);
                let mut total = 0u64;
                for n in 1..=2_000u64 {
                    let padding = schedule.next_padding();
                    let h = FrameHeader {
                        padding,
                        ..FrameHeader::new(bitrate, rate, ChannelMode::Stereo)
                    };
                    total += h.frame_len() as u64;
                    let factor = if rate.is_lsf() { 72 } else { 144 };
                    let exact = n * factor * u64::from(bitrate.bps()) / u64::from(rate.hz());
                    assert_eq!(total, exact, "{bitrate:?} at {rate:?}, frame {n}");
                }
            }
        }
    }

    #[test]
    fn no_padding_at_32_and_48_khz() {
        for rate in [SampleRate::Hz32000, SampleRate::Hz48000] {
            for bitrate in Bitrate::all(rate) {
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
