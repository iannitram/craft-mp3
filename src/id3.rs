//! ID3v2.4 tags for the front of an MP3 stream: title, artist, album,
//! recording year, comment, and chapters (the ID3v2 Chapter Frame Addendum's
//! CTOC and CHAP frames).
//!
//! The tag goes in front of the stream, before the Info frame. The Info
//! frame stays the first audio frame, so gapless playback is unchanged, and
//! the Info tag's counts still cover the audio only.
//!
//! Only what this crate needs is written: UTF-8 text, frames with no flags,
//! no unsynchronisation, no extended header, no compression, no padding and
//! no footer. Chapter times are given in input samples and written in
//! milliseconds, so the encoder delay is not added to them.

use std::fmt;

/// The largest value a synchsafe integer holds: 28 bits, seven per byte.
const MAX_SYNCHSAFE: u64 = (1 << 28) - 1;

/// The text encoding byte for UTF-8 (ID3v2.4).
const UTF8: u8 = 3;

/// CHAP's start and end byte offsets when they are not used.
const OFFSET_UNUSED: [u8; 4] = [0xFF; 4];

/// CTOC flags: top-level and ordered, both bits set.
const CTOC_FLAGS: u8 = 0b11;

/// The most chapters a CTOC lists: its entry count is a single byte.
const MAX_CHAPTERS: usize = 255;

/// The ID3v2 tag written in front of an MP3 stream.
///
/// Build it with the setters, then call [`Id3Tag::to_bytes`] or
/// [`with_id3`]. Fields that are not set are left out of the tag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Id3Tag {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    year: Option<u16>,
    comment: Option<String>,
    chapters: Vec<Chapter>,
}

/// One chapter, with times in input samples.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Chapter {
    title: String,
    start: u64,
    end: Option<u64>,
}

impl Id3Tag {
    /// An empty tag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the title (TIT2).
    pub fn title(mut self, s: &str) -> Self {
        self.title = Some(s.to_owned());
        self
    }

    /// Sets the artist (TPE1).
    pub fn artist(mut self, s: &str) -> Self {
        self.artist = Some(s.to_owned());
        self
    }

    /// Sets the album (TALB).
    pub fn album(mut self, s: &str) -> Self {
        self.album = Some(s.to_owned());
        self
    }

    /// Sets the recording year (TDRC, written as `YYYY`). Years above 9999
    /// are refused by [`Id3Tag::to_bytes`].
    pub fn year(mut self, y: u16) -> Self {
        self.year = Some(y);
        self
    }

    /// Sets the comment (COMM), with language `und` and an empty description.
    pub fn comment(mut self, s: &str) -> Self {
        self.comment = Some(s.to_owned());
        self
    }

    /// Adds one chapter. Times are in samples of the input (before the
    /// encoder delay); the tag converts them to milliseconds for CHAP. A
    /// chapter with no end ends where the next chapter starts, or at the
    /// end of the stream for the last one. Chapters may be added in any
    /// order; they are sorted by start.
    pub fn chapter(mut self, title: &str, start_sample: u64, end_sample: Option<u64>) -> Self {
        self.chapters.push(Chapter {
            title: title.to_owned(),
            start: start_sample,
            end: end_sample,
        });
        self
    }

    /// The bytes of the whole tag: the 10-byte header and the frames, with
    /// no padding and no footer.
    ///
    /// `sample_rate` and `total_samples` (the input length per channel) turn
    /// the chapter times into milliseconds, rounding to the nearest
    /// millisecond with halves going up, and bound the chapters.
    ///
    /// # Errors
    ///
    /// Fails if the sample rate is zero, a text field holds a NUL byte, the
    /// year is above 9999, there are more than 255 chapters, a chapter lies
    /// past `total_samples` or ends before it starts, a chapter time does not
    /// fit in CHAP's 32-bit milliseconds, or the tag is larger than 2^28 - 1
    /// bytes (the most a synchsafe size holds). A tag is never truncated.
    pub fn to_bytes(&self, sample_rate: u32, total_samples: u64) -> Result<Vec<u8>, Id3Error> {
        if sample_rate == 0 {
            return Err(Id3Error::ZeroSampleRate);
        }
        let mut frames = Vec::new();
        if let Some(s) = &self.title {
            frames.extend(text_frame(*b"TIT2", s, "title")?);
        }
        if let Some(s) = &self.artist {
            frames.extend(text_frame(*b"TPE1", s, "artist")?);
        }
        if let Some(s) = &self.album {
            frames.extend(text_frame(*b"TALB", s, "album")?);
        }
        if let Some(y) = self.year {
            if y > 9999 {
                return Err(Id3Error::YearOutOfRange(y));
            }
            frames.extend(text_frame(*b"TDRC", &format!("{y:04}"), "year")?);
        }
        if let Some(s) = &self.comment {
            frames.extend(comment_frame(s)?);
        }
        if !self.chapters.is_empty() {
            frames.extend(chapter_frames(&self.chapters, sample_rate, total_samples)?);
        }

        let size = synchsafe(frames.len() as u64)?;
        let mut tag = Vec::with_capacity(10 + frames.len());
        tag.extend_from_slice(b"ID3");
        tag.extend_from_slice(&[4, 0, 0]);
        tag.extend_from_slice(&size);
        tag.extend_from_slice(&frames);
        Ok(tag)
    }
}

/// The ID3v2 tag in front of the finished stream: the tag's bytes, then the
/// stream (the Info frame first, then the audio) unchanged.
///
/// `sample_rate` and `total_samples` are as for [`Id3Tag::to_bytes`]. Write
/// the Info frame into `stream` before calling this, as the crate's own
/// examples do.
///
/// ```
/// use craft_mp3::{Encoder, EncoderConfig, Id3Tag, with_id3};
///
/// let mut encoder = Encoder::new(EncoderConfig::new(44_100, 1, 128))?;
/// let mut mp3 = Vec::new();
/// encoder.encode(&[0.0; 44_100], &mut mp3);
/// if let Some(info) = encoder.finish(&mut mp3) {
///     info.write_to(&mut mp3);
/// }
/// let tag = Id3Tag::new().title("Tone").chapter("Start", 0, None);
/// let file = with_id3(&tag, &mp3, 44_100, 44_100)?;
/// assert_eq!(&file[..3], b"ID3");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// Fails as [`Id3Tag::to_bytes`] does.
pub fn with_id3(
    tag: &Id3Tag,
    stream: &[u8],
    sample_rate: u32,
    total_samples: u64,
) -> Result<Vec<u8>, Id3Error> {
    let header = tag.to_bytes(sample_rate, total_samples)?;
    let mut out = Vec::with_capacity(header.len() + stream.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(stream);
    Ok(out)
}

/// Why a tag could not be written.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Id3Error {
    /// The tag, or one of its frames, is larger than 2^28 - 1 bytes.
    TooLarge { bytes: u64 },
    /// The sample rate is zero, so chapter times cannot be converted.
    ZeroSampleRate,
    /// A text field holds a NUL byte, which ends text in ID3v2.
    NulInText { field: &'static str },
    /// The recording year is above 9999, which TDRC's four digits cannot hold.
    YearOutOfRange(u16),
    /// More chapters than a CTOC frame lists (255).
    TooManyChapters { count: usize },
    /// A chapter starts or ends after the end of the stream.
    SampleBeyondEnd { sample: u64, total_samples: u64 },
    /// A chapter ends before it starts.
    ChapterEndsBeforeStart { start: u64, end: u64 },
    /// A chapter time in milliseconds does not fit CHAP's 32-bit field.
    ChapterTooLong { sample: u64 },
}

impl fmt::Display for Id3Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "ID3 tag data of {bytes} bytes is larger than the 2^28 - 1 a synchsafe size holds"
            ),
            Self::ZeroSampleRate => write!(f, "sample rate must not be zero"),
            Self::NulInText { field } => write!(f, "the {field} contains a NUL byte"),
            Self::YearOutOfRange(y) => write!(f, "year {y} does not fit four digits"),
            Self::TooManyChapters { count } => {
                write!(f, "{count} chapters; an ID3 CTOC frame lists at most 255")
            }
            Self::SampleBeyondEnd {
                sample,
                total_samples,
            } => write!(
                f,
                "chapter at sample {sample} lies past the end of the stream ({total_samples} samples)"
            ),
            Self::ChapterEndsBeforeStart { start, end } => {
                write!(
                    f,
                    "chapter ends at sample {end}, before it starts at {start}"
                )
            }
            Self::ChapterTooLong { sample } => write!(
                f,
                "chapter time at sample {sample} does not fit in 32-bit milliseconds"
            ),
        }
    }
}

impl std::error::Error for Id3Error {}

/// Four synchsafe bytes: 7 bits each, most significant first, so bit 7 of
/// every byte is zero.
fn synchsafe(n: u64) -> Result<[u8; 4], Id3Error> {
    if n > MAX_SYNCHSAFE {
        return Err(Id3Error::TooLarge { bytes: n });
    }
    Ok([
        ((n >> 21) & 0x7F) as u8,
        ((n >> 14) & 0x7F) as u8,
        ((n >> 7) & 0x7F) as u8,
        (n & 0x7F) as u8,
    ])
}

/// A frame: id, synchsafe size, two zero flag bytes, then the body.
fn frame(id: [u8; 4], body: &[u8]) -> Result<Vec<u8>, Id3Error> {
    let mut out = Vec::with_capacity(10 + body.len());
    out.extend_from_slice(&id);
    out.extend_from_slice(&synchsafe(body.len() as u64)?);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(body);
    Ok(out)
}

/// The encoding byte and the text, UTF-8, with no terminator.
fn text_body(s: &str, field: &'static str) -> Result<Vec<u8>, Id3Error> {
    if s.contains('\0') {
        return Err(Id3Error::NulInText { field });
    }
    let mut body = Vec::with_capacity(1 + s.len());
    body.push(UTF8);
    body.extend_from_slice(s.as_bytes());
    Ok(body)
}

fn text_frame(id: [u8; 4], s: &str, field: &'static str) -> Result<Vec<u8>, Id3Error> {
    frame(id, &text_body(s, field)?)
}

/// COMM: UTF-8, language `und`, an empty (NUL-terminated) description, then
/// the text.
fn comment_frame(s: &str) -> Result<Vec<u8>, Id3Error> {
    let text = text_body(s, "comment")?;
    let mut body = Vec::with_capacity(text.len() + 4);
    body.push(UTF8);
    body.extend_from_slice(b"und");
    body.push(0);
    body.extend_from_slice(&text[1..]);
    frame(*b"COMM", &body)
}

/// An element id as the Chapter Frame Addendum writes it: ASCII, NUL-ended.
fn element_id(name: &str) -> Vec<u8> {
    let mut id = name.as_bytes().to_vec();
    id.push(0);
    id
}

/// Rounds a sample position to the nearest millisecond, halves going up:
/// `floor(sample * 1000 / rate + 1/2)`, computed exactly in integers.
fn millis(sample: u64, rate: u32) -> Result<u32, Id3Error> {
    let rate = u128::from(rate);
    let ms = (2 * u128::from(sample) * 1000 + rate) / (2 * rate);
    u32::try_from(ms).map_err(|_| Id3Error::ChapterTooLong { sample })
}

/// The CTOC frame and one CHAP frame per chapter, sorted by start.
fn chapter_frames(
    chapters: &[Chapter],
    sample_rate: u32,
    total_samples: u64,
) -> Result<Vec<u8>, Id3Error> {
    if chapters.len() > MAX_CHAPTERS {
        return Err(Id3Error::TooManyChapters {
            count: chapters.len(),
        });
    }
    let mut sorted: Vec<&Chapter> = chapters.iter().collect();
    sorted.sort_by_key(|c| c.start);

    // Each chapter's end: its own, else the next chapter's start, else the
    // end of the stream. Checked against the stream before anything is built.
    let mut spans = Vec::with_capacity(sorted.len());
    for (i, chapter) in sorted.iter().enumerate() {
        let end = chapter
            .end
            .unwrap_or_else(|| sorted.get(i + 1).map_or(total_samples, |next| next.start));
        for sample in [chapter.start, end] {
            if sample > total_samples {
                return Err(Id3Error::SampleBeyondEnd {
                    sample,
                    total_samples,
                });
            }
        }
        if end < chapter.start {
            return Err(Id3Error::ChapterEndsBeforeStart {
                start: chapter.start,
                end,
            });
        }
        spans.push((chapter.title.as_str(), chapter.start, end));
    }

    // CTOC: top-level, ordered, listing every CHAP in order.
    let mut toc = element_id("toc");
    toc.push(CTOC_FLAGS);
    toc.push(sorted.len() as u8);
    for i in 0..sorted.len() {
        toc.extend(element_id(&format!("chp{i}")));
    }
    let mut out = frame(*b"CTOC", &toc)?;

    for (i, (title, start, end)) in spans.into_iter().enumerate() {
        let mut body = element_id(&format!("chp{i}"));
        body.extend_from_slice(&millis(start, sample_rate)?.to_be_bytes());
        body.extend_from_slice(&millis(end, sample_rate)?.to_be_bytes());
        body.extend_from_slice(&OFFSET_UNUSED);
        body.extend_from_slice(&OFFSET_UNUSED);
        body.extend(text_frame(*b"TIT2", title, "chapter title")?);
        out.extend(frame(*b"CHAP", &body)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synchsafe_encodes_the_boundaries() {
        let cases: [(u64, [u8; 4]); 5] = [
            (0, [0, 0, 0, 0]),
            (127, [0, 0, 0, 0x7F]),
            (128, [0, 0, 1, 0]),
            (1 << 21, [1, 0, 0, 0]),
            ((1 << 28) - 1, [0x7F; 4]),
        ];
        for (n, bytes) in cases {
            assert_eq!(synchsafe(n).unwrap(), bytes, "synchsafe({n})");
        }
    }

    #[test]
    fn synchsafe_refuses_what_it_cannot_hold() {
        assert_eq!(
            synchsafe(1 << 28),
            Err(Id3Error::TooLarge { bytes: 1 << 28 })
        );
    }

    #[test]
    fn title_only_tag_matches_the_spec_bytes() {
        // Header: "ID3", v2.4.0, no flags, size 13. Frame: "TIT2", size 3,
        // no flags, encoding 03 (UTF-8), "Hi".
        let expected = [
            0x49, 0x44, 0x33, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0D, // header
            0x54, 0x49, 0x54, 0x32, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, // TIT2 header
            0x03, 0x48, 0x69, // encoding, "Hi"
        ];
        let bytes = Id3Tag::new().title("Hi").to_bytes(44_100, 0).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn empty_tag_is_a_bare_header() {
        let bytes = Id3Tag::new().to_bytes(44_100, 0).unwrap();
        assert_eq!(bytes, [0x49, 0x44, 0x33, 0x04, 0x00, 0x00, 0, 0, 0, 0]);
    }

    #[test]
    fn artist_and_album_are_written_as_text_frames() {
        let bytes = Id3Tag::new()
            .artist("A")
            .album("B")
            .to_bytes(44_100, 0)
            .unwrap();
        // Two frames of 12 bytes each: size 24 (0x18).
        let mut expected = vec![0x49, 0x44, 0x33, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x18];
        expected.extend([0x54, 0x50, 0x45, 0x31, 0, 0, 0, 0x02, 0, 0, 0x03, b'A']); // TPE1
        expected.extend([0x54, 0x41, 0x4C, 0x42, 0, 0, 0, 0x02, 0, 0, 0x03, b'B']); // TALB
        assert_eq!(bytes, expected);
    }

    #[test]
    fn year_is_written_as_four_digits() {
        let year = Id3Tag::new().year(1999).to_bytes(44_100, 0).unwrap();
        assert_eq!(
            &year[10..],
            [
                0x54, 0x44, 0x52, 0x43, 0, 0, 0, 5, 0, 0, 3, b'1', b'9', b'9', b'9'
            ]
        );
    }

    #[test]
    fn comment_has_und_and_an_empty_description() {
        let bytes = Id3Tag::new().comment("x").to_bytes(44_100, 0).unwrap();
        assert_eq!(
            &bytes[10..],
            [
                0x43, 0x4F, 0x4D, 0x4D, 0, 0, 0, 0x06, 0, 0, 0x03, b'u', b'n', b'd', 0, b'x'
            ]
        );
    }

    #[test]
    fn year_above_9999_is_refused() {
        assert_eq!(
            Id3Tag::new().year(10_000).to_bytes(44_100, 0),
            Err(Id3Error::YearOutOfRange(10_000))
        );
    }

    #[test]
    fn nul_in_text_is_refused() {
        assert_eq!(
            Id3Tag::new().title("a\0b").to_bytes(44_100, 0),
            Err(Id3Error::NulInText { field: "title" })
        );
    }

    #[test]
    fn zero_sample_rate_is_refused() {
        assert_eq!(
            Id3Tag::new().title("x").to_bytes(0, 0),
            Err(Id3Error::ZeroSampleRate)
        );
    }

    #[test]
    fn chapter_times_round_to_the_nearest_millisecond_at_44_1_khz() {
        assert_eq!(millis(0, 44_100), Ok(0));
        assert_eq!(millis(1, 44_100), Ok(0)); // 0.023 ms
        assert_eq!(millis(22, 44_100), Ok(0)); // 0.499 ms
        assert_eq!(millis(23, 44_100), Ok(1)); // 0.522 ms
        assert_eq!(millis(441_000, 44_100), Ok(10_000));
        assert_eq!(millis(44_100, 44_100), Ok(1_000));
    }

    #[test]
    fn chapter_times_round_half_up_at_48_khz() {
        assert_eq!(millis(23, 48_000), Ok(0)); // 0.479 ms
        assert_eq!(millis(24, 48_000), Ok(1)); // exactly 0.5 ms goes up
        assert_eq!(millis(72, 48_000), Ok(2)); // exactly 1.5 ms goes up
        assert_eq!(millis(480_000, 48_000), Ok(10_000));
    }

    #[test]
    fn chapter_millis_must_fit_in_32_bits() {
        assert_eq!(
            millis(u64::MAX, 1),
            Err(Id3Error::ChapterTooLong { sample: u64::MAX })
        );
    }

    /// The CHAP frames of a tag, as (element id, start ms, end ms, title).
    fn chaps(tag: &[u8]) -> Vec<(String, u32, u32, String)> {
        let mut out = Vec::new();
        let mut pos = 10;
        while pos + 10 <= tag.len() {
            let id = &tag[pos..pos + 4];
            let size = tag[pos + 4..pos + 8]
                .iter()
                .fold(0usize, |n, &b| (n << 7) | usize::from(b));
            let body = &tag[pos + 10..pos + 10 + size];
            if id == b"CHAP" {
                let nul = body.iter().position(|&b| b == 0).unwrap();
                let element = String::from_utf8(body[..nul].to_vec()).unwrap();
                let at = nul + 1;
                let start = u32::from_be_bytes(body[at..at + 4].try_into().unwrap());
                let end = u32::from_be_bytes(body[at + 4..at + 8].try_into().unwrap());
                assert_eq!(&body[at + 8..at + 16], [0xFF; 8], "offsets unused");
                let sub = &body[at + 16..];
                assert_eq!(&sub[..4], b"TIT2");
                let title = String::from_utf8(sub[11..].to_vec()).unwrap();
                out.push((element, start, end, title));
            }
            pos += 10 + size;
        }
        out
    }

    #[test]
    fn chapters_have_ms_times_and_implied_ends() {
        let bytes = Id3Tag::new()
            .chapter("Verse", 441_000, None)
            .chapter("Intro", 0, Some(441_000))
            .to_bytes(44_100, 882_000)
            .unwrap();
        assert_eq!(
            chaps(&bytes),
            [
                ("chp0".to_owned(), 0, 10_000, "Intro".to_owned()),
                ("chp1".to_owned(), 10_000, 20_000, "Verse".to_owned()),
            ]
        );
    }

    #[test]
    fn last_chapter_without_an_end_ends_at_the_stream_end() {
        let bytes = Id3Tag::new()
            .chapter("Only", 48_000, None)
            .to_bytes(48_000, 96_000)
            .unwrap();
        assert_eq!(
            chaps(&bytes),
            [("chp0".to_owned(), 1_000, 2_000, "Only".to_owned())]
        );
    }

    #[test]
    fn ctoc_lists_the_chapters_in_order_with_flags() {
        let bytes = Id3Tag::new()
            .chapter("b", 10, None)
            .chapter("a", 0, Some(10))
            .to_bytes(1_000, 20)
            .unwrap();
        let ctoc = bytes.windows(4).position(|w| w == b"CTOC").unwrap();
        let body = &bytes[ctoc + 10..];
        assert_eq!(&body[..4], b"toc\0");
        assert_eq!(body[4], 0b11, "top-level and ordered");
        assert_eq!(body[5], 2, "two entries");
        assert_eq!(&body[6..11], b"chp0\0");
        assert_eq!(&body[11..16], b"chp1\0");
    }

    #[test]
    fn chapter_past_the_stream_is_refused() {
        assert_eq!(
            Id3Tag::new().chapter("x", 101, None).to_bytes(44_100, 100),
            Err(Id3Error::SampleBeyondEnd {
                sample: 101,
                total_samples: 100
            })
        );
        assert_eq!(
            Id3Tag::new()
                .chapter("x", 0, Some(101))
                .to_bytes(44_100, 100),
            Err(Id3Error::SampleBeyondEnd {
                sample: 101,
                total_samples: 100
            })
        );
    }

    #[test]
    fn chapter_ending_before_its_start_is_refused() {
        assert_eq!(
            Id3Tag::new()
                .chapter("x", 50, Some(10))
                .to_bytes(44_100, 100),
            Err(Id3Error::ChapterEndsBeforeStart { start: 50, end: 10 })
        );
    }

    #[test]
    fn more_than_255_chapters_are_refused() {
        let mut tag = Id3Tag::new();
        for i in 0..256 {
            tag = tag.chapter("c", i, None);
        }
        assert_eq!(
            tag.to_bytes(44_100, 1_000),
            Err(Id3Error::TooManyChapters { count: 256 })
        );
    }

    #[test]
    fn with_id3_puts_the_tag_before_the_stream_untouched() {
        let stream = [0xFF, 0xFB, 0x90, 0x00, 1, 2, 3];
        let tag = Id3Tag::new().title("x");
        let out = with_id3(&tag, &stream, 44_100, 0).unwrap();
        let header = tag.to_bytes(44_100, 0).unwrap();
        assert_eq!(&out[..header.len()], header);
        assert_eq!(&out[header.len()..], stream);
    }
}
