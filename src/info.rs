//! The Xing "Info" tag: a silent first frame telling players how many
//! frames and bytes the stream has, with a seek table, so they can show the
//! exact duration and seek without scanning the file.
//!
//! The encoder can't know the counts until the end, so it writes a
//! placeholder tag first and hands back the finished one from
//! [`crate::Encoder::finish`] for the caller to write over it.

use crate::bitstream::BitWriter;
use crate::header::{Bitrate, FrameHeader};

/// "Info" marks a constant-bitrate stream ("Xing" is used for VBR).
const TAG_ID: &[u8; 4] = b"Info";

/// Flags: frame count, byte count and table of contents present.
const FLAGS: u32 = 0x1 | 0x2 | 0x4;

/// Length of the tag body: id, flags, frames, bytes and a 100-entry table.
const TAG_LEN: usize = 4 + 4 + 4 + 4 + 100;

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
    Bitrate::all()
        .filter(|&b| b >= template.bitrate)
        .map(|bitrate| FrameHeader {
            bitrate,
            padding: false,
            crc: false,
            ..*template
        })
        .find(|h| h.main_data_len() >= TAG_LEN)
        .expect("a 320 kbit/s frame always has room")
}

/// Builds the tag frame. `frames` counts the audio frames after it and
/// `bytes` the whole stream including the tag frame; `None` gives a
/// placeholder of the same length with no counts.
pub(crate) fn tag_frame(template: &FrameHeader, counts: Option<(u32, u32)>) -> InfoTag {
    let header = tag_header(template);
    let mut w = BitWriter::with_capacity(header.frame_len());
    header.write(&mut w);
    // All-zero side information: every granule is empty, so a decoder that
    // doesn't know the tag plays one frame of silence.
    for _ in 0..header.side_info_len() {
        w.write_bits(0, 8);
    }
    let mut frame = w.into_bytes();

    frame.extend_from_slice(TAG_ID);
    match counts {
        Some((frames, bytes)) => {
            frame.extend_from_slice(&FLAGS.to_be_bytes());
            frame.extend_from_slice(&frames.to_be_bytes());
            frame.extend_from_slice(&bytes.to_be_bytes());
            // Table of contents: entry i is the byte position of i% of the
            // stream, scaled to 0..256. A constant bitrate makes it linear.
            frame.extend((0..100u32).map(|i| (i * 256 / 100) as u8));
        }
        None => frame.extend_from_slice(&0u32.to_be_bytes()),
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
        let tag = tag_frame(&t, Some((1000, 418_000)));
        let bytes = tag.as_bytes();
        let header = FrameHeader::parse(bytes[..4].try_into().unwrap()).unwrap();
        assert_eq!(header.frame_len(), bytes.len());

        let at = 4 + header.side_info_len();
        assert!(bytes[4..at].iter().all(|&b| b == 0));
        assert_eq!(&bytes[at..at + 4], b"Info");
        let be = |i: usize| u32::from_be_bytes(bytes[at + i..at + i + 4].try_into().unwrap());
        assert_eq!((be(4), be(8), be(12)), (FLAGS, 1000, 418_000));
        assert_eq!(bytes[at + 16], 0);
        assert_eq!(bytes[at + 16 + 50], 128);
    }

    #[test]
    fn placeholder_has_the_same_length_and_no_counts() {
        let t = template(128, SampleRate::Hz48000, ChannelMode::Mono);
        let placeholder = tag_frame(&t, None);
        assert_eq!(
            placeholder.as_bytes().len(),
            tag_frame(&t, Some((1, 2))).as_bytes().len()
        );
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
        let header =
            FrameHeader::parse(tag_frame(&t, None).as_bytes()[..4].try_into().unwrap()).unwrap();
        assert!(header.bitrate.kbps() > 32);
        assert!(header.main_data_len() >= TAG_LEN);
        // Everything else matches the stream.
        assert_eq!((header.sample_rate, header.mode), (t.sample_rate, t.mode));
    }

    #[test]
    fn write_to_overwrites_the_start_of_the_stream() {
        let t = template(128, SampleRate::Hz44100, ChannelMode::Mono);
        let tag = tag_frame(&t, Some((5, 6)));
        let mut stream = vec![0xAA; tag.as_bytes().len() + 10];
        tag.write_to(&mut stream);
        assert_eq!(&stream[..tag.as_bytes().len()], tag.as_bytes());
        assert!(stream[tag.as_bytes().len()..].iter().all(|&b| b == 0xAA));
    }
}
