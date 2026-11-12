//! The Xing "Info" tag: a silent first frame telling players how many
//! frames and bytes the stream has, with a seek table, so they can show the
//! duration and seek without scanning the file.
//!
//! The tag also carries LAME's layout extension after the seek table: a
//! 9-byte encoder string, the encoder delay and padding, the music length and
//! two CRC-16s. The string is [`LAYOUT_MARKER`], which says the fields follow
//! LAME's layout; it does not claim that LAME encoded the stream. ffmpeg
//! applies the delay and padding only when the string starts with `LAME`,
//! `Lavf` or `Lavc`, so the marker starts with `LAME`. Players that apply
//! them trim the encoder delay and padding, and the stream plays back at the
//! input's exact length.
//!
//! The encoder can't know the counts until the end, so it writes a
//! placeholder tag first and hands back the finished one from
//! [`crate::Encoder::finish`] for the caller to write over it.

use crate::bitstream::BitWriter;
use crate::encoder::ENCODER_DELAY;
use crate::header::{Bitrate, FrameHeader};

/// "Info" marks a constant-bitrate stream, "Xing" a variable one.
const CBR_ID: &[u8; 4] = b"Info";
const VBR_ID: &[u8; 4] = b"Xing";

/// Flags: frame count, byte count, table of contents and quality present.
const FLAGS: u32 = 0x1 | 0x2 | 0x4 | 0x8;

/// The 9-byte encoder string: LAME's layout marker, see the module docs.
pub(crate) const LAYOUT_MARKER: &[u8; 9] = b"LAMEcraft";

/// LAME's rate-control codes for the revision/method byte: 1 for CBR, 4 for
/// VBR (its "mtrh" method).
const CBR_METHOD: u8 = 1;
const VBR_METHOD: u8 = 4;

/// The encoder delay the tag records: decoders skip this many samples plus
/// their own 529, so the total is [`ENCODER_DELAY`].
pub(crate) const DELAY_FIELD: usize = ENCODER_DELAY - 529;

/// Largest padding the 12-bit field holds.
pub(crate) const MAX_PADDING: u32 = 0xFFF;

/// Length of the tag body: id, flags, frames, bytes, a 100-entry table, the
/// quality and LAME's 36-byte extension (encoder string, delay and padding,
/// music length and CRC, tag CRC).
const TAG_LEN: usize = 4 + 4 + 4 + 4 + 100 + 4 + 36;

/// Body bytes covered by the tag CRC: everything before it.
const TAG_CRC_COVERS: usize = TAG_LEN - 2;

/// The finished Info tag frame for a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoTag {
    frame: Vec<u8>,
}

impl InfoTag {
    /// The tag frame's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.frame
    }

    /// Overwrites the placeholder at the start of the complete stream.
    ///
    /// # Panics
    ///
    /// Panics if `stream` is shorter than the tag frame.
    pub fn write_to(&self, stream: &mut [u8]) {
        stream[..self.frame.len()].copy_from_slice(&self.frame);
    }
}

/// The header for the tag frame: the stream's own, unpadded, at the
/// smallest bitrate (no lower than the stream's) whose frame has room for
/// the tag.
fn tag_header(template: &FrameHeader) -> FrameHeader {
    Bitrate::all(template.sample_rate)
        .filter(|&b| b >= template.bitrate)
        .map(|bitrate| FrameHeader {
            bitrate,
            padding: false,
            crc: false,
            ..*template
        })
        .find(|h| h.main_data_len() >= TAG_LEN)
        .expect("the largest frame always has room")
}

/// What the finished tag records about the stream.
pub(crate) struct Counts {
    /// Audio frames after the tag frame.
    pub(crate) frames: u32,
    /// Bytes in the whole stream, tag frame included.
    pub(crate) bytes: u32,
    /// Entry `i` is the byte offset of `i`% of the playing time, as a
    /// fraction of `bytes` scaled to 0..256.
    pub(crate) toc: [u8; 100],
    /// Silence after the input, in samples per channel, once the encoder
    /// delay is removed; at most [`MAX_PADDING`].
    pub(crate) padding: u32,
    /// Bytes of audio frames, tag frame excluded.
    pub(crate) music_len: u32,
    /// CRC-16 of those audio bytes.
    pub(crate) music_crc: u16,
    /// LAME's quality indicator, 0 (worst) to 100 (best).
    pub(crate) quality: u8,
}

impl Counts {
    /// Builds the counts from each audio frame's length; `tag_len` is the
    /// tag frame's. The gapless fields are left at zero for the caller to set.
    pub(crate) fn from_frame_lengths(tag_len: usize, frame_lengths: &[u32]) -> Self {
        let frames = frame_lengths.len();
        let mut offsets = Vec::with_capacity(frames);
        let mut offset = tag_len as u64;
        for &len in frame_lengths {
            offsets.push(offset);
            offset += u64::from(len);
        }
        let bytes = offset;
        let toc = std::array::from_fn(|i| {
            let frame = i * frames / 100;
            let at = offsets.get(frame).copied().unwrap_or(bytes);
            (at * 256 / bytes.max(1)).min(255) as u8
        });
        Self {
            frames: frames as u32,
            bytes: u32::try_from(bytes).unwrap_or(u32::MAX),
            toc,
            padding: 0,
            music_len: 0,
            music_crc: 0,
            quality: 100,
        }
    }
}

/// CRC-16 with polynomial 0xA001 (0x8005 bit-reversed), initial value 0 and
/// no final XOR, as LAME's tag uses.
pub(crate) fn crc16(bytes: &[u8]) -> u16 {
    bytes.iter().fold(0, |crc, &b| {
        (crc >> 8) ^ CRC16_TABLE[usize::from((crc ^ u16::from(b)) & 0xFF)]
    })
}

const CRC16_TABLE: [u16; 256] = crc16_table();

const fn crc16_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u16;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// The tag body, from the id on. With counts, the LAME extension follows the
/// seek table and the tag CRC closes the frame prefix.
fn tag_body(vbr: bool, counts: Option<&Counts>) -> Vec<u8> {
    let id = if vbr { VBR_ID } else { CBR_ID };
    let Some(counts) = counts else {
        let mut body = id.to_vec();
        body.extend_from_slice(&0u32.to_be_bytes());
        return body;
    };
    let mut body = Vec::with_capacity(TAG_LEN);
    body.extend_from_slice(id);
    body.extend_from_slice(&FLAGS.to_be_bytes());
    body.extend_from_slice(&counts.frames.to_be_bytes());
    body.extend_from_slice(&counts.bytes.to_be_bytes());
    body.extend_from_slice(&counts.toc);
    body.extend_from_slice(&u32::from(counts.quality).to_be_bytes());
    // LAME's extension: the encoder string, then the fields in order.
    body.extend_from_slice(LAYOUT_MARKER);
    body.push(if vbr { VBR_METHOD } else { CBR_METHOD });
    // Lowpass, peak, replay gains, encoding flags and ABR minimum: not recorded.
    body.extend_from_slice(&[0; 11]);
    // Delay and padding, 12 bits each, packed into three bytes.
    let delay_padding = ((DELAY_FIELD as u32) << 12) | (counts.padding & MAX_PADDING);
    body.extend_from_slice(&delay_padding.to_be_bytes()[1..]);
    // Misc, MP3 gain and preset: not recorded.
    body.extend_from_slice(&[0; 4]);
    body.extend_from_slice(&counts.music_len.to_be_bytes());
    body.extend_from_slice(&counts.music_crc.to_be_bytes());
    debug_assert_eq!(body.len(), TAG_CRC_COVERS);
    body
}

/// Builds the tag frame; `None` gives a placeholder of the same length
/// with no counts.
pub(crate) fn tag_frame(template: &FrameHeader, vbr: bool, counts: Option<&Counts>) -> InfoTag {
    let header = tag_header(template);
    let mut w = BitWriter::with_capacity(header.frame_len());
    header.write(&mut w);
    // All-zero side information: every granule is empty, so a decoder that
    // doesn't know the tag plays one frame of silence.
    for _ in 0..header.side_info_len() {
        w.write_bits(0, 8);
    }
    let mut frame = w.into_bytes();
    frame.extend_from_slice(&tag_body(vbr, counts));
    if counts.is_some() {
        debug_assert_eq!(frame.len(), 4 + header.side_info_len() + TAG_CRC_COVERS);
        let tag_crc = crc16(&frame);
        frame.extend_from_slice(&tag_crc.to_be_bytes());
    }
    frame.resize(header.frame_len(), 0);
    InfoTag { frame }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::{ChannelMode, SampleRate};

    fn template(kbps: u32, rate: SampleRate, mode: ChannelMode) -> FrameHeader {
        FrameHeader::new(Bitrate::from_kbps(kbps).unwrap(), rate, mode)
    }

    #[test]
    fn tag_frame_is_a_valid_frame_with_the_tag_after_the_side_info() {
        let stereo = ChannelMode::JointStereo {
            mid_side: false,
            intensity: false,
        };
        let t = template(128, SampleRate::Hz44100, stereo);
        let counts = Counts::from_frame_lengths(418, &[418; 999]);
        let tag = tag_frame(&t, false, Some(&counts));
        let bytes = tag.as_bytes();
        let header = FrameHeader::parse(bytes[..4].try_into().unwrap()).unwrap();
        assert_eq!(header.frame_len(), bytes.len());

        let at = 4 + header.side_info_len();
        assert!(bytes[4..at].iter().all(|&b| b == 0));
        assert_eq!(&bytes[at..at + 4], b"Info");
        let be = |i: usize| u32::from_be_bytes(bytes[at + i..at + i + 4].try_into().unwrap());
        assert_eq!((be(4), be(8), be(12)), (FLAGS, 999, 418_000));
        assert_eq!(bytes[at + 16], 0);
        // Half the frames sit about half way through the bytes.
        assert!((127..=129).contains(&bytes[at + 16 + 50]));
    }

    #[test]
    fn placeholder_has_the_same_length_and_no_counts() {
        let t = template(128, SampleRate::Hz48000, ChannelMode::Mono);
        let placeholder = tag_frame(&t, false, None);
        let counts = Counts::from_frame_lengths(100, &[384]);
        let finished = tag_frame(&t, false, Some(&counts));
        assert_eq!(placeholder.as_bytes().len(), finished.as_bytes().len());
        let at = 4 + 17;
        assert_eq!(&placeholder.as_bytes()[at..at + 8], b"Info\0\0\0\0");
    }

    #[test]
    fn low_bitrates_use_a_bigger_frame_for_the_tag() {
        let stereo = ChannelMode::JointStereo {
            mid_side: false,
            intensity: false,
        };
        let t = template(32, SampleRate::Hz48000, stereo);
        let header = FrameHeader::parse(
            tag_frame(&t, false, None).as_bytes()[..4]
                .try_into()
                .unwrap(),
        )
        .unwrap();
        assert!(header.bitrate.kbps() > 32);
        assert!(header.main_data_len() >= TAG_LEN);
        // Everything else matches the stream.
        assert_eq!((header.sample_rate, header.mode), (t.sample_rate, t.mode));
    }

    #[test]
    fn write_to_overwrites_the_start_of_the_stream() {
        let t = template(128, SampleRate::Hz44100, ChannelMode::Mono);
        let tag = tag_frame(&t, false, Some(&Counts::from_frame_lengths(10, &[5, 6])));
        let mut stream = vec![0xAA; tag.as_bytes().len() + 10];
        tag.write_to(&mut stream);
        assert_eq!(&stream[..tag.as_bytes().len()], tag.as_bytes());
        assert!(stream[tag.as_bytes().len()..].iter().all(|&b| b == 0xAA));
    }

    #[test]
    fn crc16_matches_the_crc16_arc_check_value() {
        // CRC-16/ARC (this polynomial, initial value 0, no final XOR) of "123456789".
        assert_eq!(crc16(b"123456789"), 0xBB3D);
        assert_eq!(crc16(&[]), 0);
    }

    #[test]
    fn lame_layout_fields_follow_the_seek_table() {
        let stereo = ChannelMode::JointStereo {
            mid_side: false,
            intensity: false,
        };
        let t = template(128, SampleRate::Hz44100, stereo);
        let counts = Counts {
            padding: 700,
            music_len: 418,
            music_crc: 0x1234,
            quality: 60,
            ..Counts::from_frame_lengths(418, &[418])
        };
        let tag = tag_frame(&t, true, Some(&counts));
        let bytes = tag.as_bytes();
        let at = 4 + 32;
        let at_body = |i: usize| &bytes[at + i..];
        assert_eq!(&at_body(0)[..4], b"Xing");
        assert_eq!(&at_body(116)[..4], &[0, 0, 0, 60]);
        assert_eq!(&at_body(120)[..9], LAYOUT_MARKER);
        assert_eq!(at_body(129)[0], VBR_METHOD);
        // 528 in the top 12 bits, 700 in the bottom 12.
        let delay_padding = (528u32 << 12) | 700;
        assert_eq!(&at_body(141)[..3], &delay_padding.to_be_bytes()[1..]);
        assert_eq!(&at_body(148)[..4], &418u32.to_be_bytes());
        assert_eq!(&at_body(152)[..2], &0x1234u16.to_be_bytes());
        let tag_crc = u16::from_be_bytes([at_body(154)[0], at_body(154)[1]]);
        assert_eq!(tag_crc, crc16(&bytes[..at + 154]));
    }

    #[test]
    fn vbr_table_of_contents_follows_the_frame_sizes() {
        // 50 small frames then 50 large ones: half the time is a fifth of the bytes.
        let lengths: Vec<u32> = (0..100).map(|i| if i < 50 { 100 } else { 400 }).collect();
        let counts = Counts::from_frame_lengths(0, &lengths);
        assert_eq!(counts.bytes, 25_000);
        assert_eq!(counts.toc[0], 0);
        assert_eq!(counts.toc[50], (5_000 * 256 / 25_000) as u8);
        assert!(counts.toc.windows(2).all(|w| w[0] <= w[1]));

        let stereo = ChannelMode::JointStereo {
            mid_side: false,
            intensity: false,
        };
        let t = template(32, SampleRate::Hz44100, stereo);
        let tag = tag_frame(&t, true, Some(&counts));
        let at = 4 + 32;
        assert_eq!(&tag.as_bytes()[at..at + 4], b"Xing");
    }
}
