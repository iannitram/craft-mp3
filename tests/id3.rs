//! ID3v2.4 tags in front of a stream: symphonia reads the text fields back,
//! the tagged stream decodes to the same samples as the untagged one, and an
//! opt-in ffprobe/ffmpeg check reads the chapters and the gapless length.

use std::io::Cursor;
use std::process::Command;

use craft_mp3::{Encoder, EncoderConfig, Id3Tag, with_id3};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;

const RATE: u32 = 44_100;
/// Three seconds of input per channel.
const LENGTH: usize = 132_300;

/// A stereo test tone, interleaved, `frames` frames long.
fn tone(frames: usize) -> Vec<f32> {
    (0..frames)
        .flat_map(|n| {
            let t = n as f32 / RATE as f32;
            let left = (t * 440.0 * std::f32::consts::TAU).sin() * 0.4;
            let right = (t * 660.0 * std::f32::consts::TAU).sin() * 0.3;
            [left, right]
        })
        .collect()
}

/// The stream as the wav2mp3 example writes it: the Info frame at the start.
fn encode(frames: usize) -> Vec<u8> {
    let mut encoder = Encoder::new(EncoderConfig::new(RATE, 2, 128)).unwrap();
    let mut mp3 = Vec::new();
    encoder.encode(&tone(frames), &mut mp3);
    if let Some(info) = encoder.finish(&mut mp3) {
        info.write_to(&mut mp3);
    }
    mp3
}

/// What symphonia returns for a stream: its interleaved samples, the title
/// and artist it read from the tag, if any.
struct Decoded {
    samples: Vec<f32>,
    title: Option<String>,
    artist: Option<String>,
}

fn decode(mp3: Vec<u8>) -> Decoded {
    let source = MediaSourceStream::new(Box::new(Cursor::new(mp3)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("stream is recognised as MP3");
    // An ID3v2 tag at the start is read during the probe, so it is in the
    // probe's metadata log, not in the format reader's.
    let mut probed_metadata = probed.metadata;
    let mut format = probed.format;
    let mut tags = Vec::new();
    if let Some(log) = probed_metadata.get() {
        collect_tags(log.current(), &mut tags);
    }
    collect_tags(format.metadata().current(), &mut tags);
    let find = |key: &str| {
        tags.iter()
            .rev()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    };
    let (title, artist) = (find("title"), find("artist"));

    let track = format.default_track().expect("one audio track");
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("decoder for the track");
    let mut samples = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => panic!("demuxing failed: {e}"),
        };
        let decoded = decoder.decode(&packet).expect("every frame decodes");
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    Decoded {
        samples,
        title,
        artist,
    }
}

/// The title and artist of a metadata revision, as symphonia names them.
fn collect_tags(revision: Option<&MetadataRevision>, out: &mut Vec<(&'static str, String)>) {
    let Some(revision) = revision else { return };
    for tag in revision.tags() {
        let key = match tag.std_key {
            Some(StandardTagKey::TrackTitle) => "title",
            Some(StandardTagKey::Artist) => "artist",
            _ => continue,
        };
        out.push((key, tag.value.to_string()));
    }
}

fn tagged(mp3: &[u8], tag: &Id3Tag, frames: usize) -> Vec<u8> {
    with_id3(tag, mp3, RATE, frames as u64).unwrap()
}

#[test]
fn title_and_artist_read_back_through_symphonia() {
    let mp3 = encode(LENGTH);
    let tag = Id3Tag::new().title("Morning Tone").artist("Ian Martin");
    let decoded = decode(tagged(&mp3, &tag, LENGTH));
    assert_eq!(decoded.title.as_deref(), Some("Morning Tone"));
    assert_eq!(decoded.artist.as_deref(), Some("Ian Martin"));
}

#[test]
fn non_ascii_title_round_trips() {
    let mp3 = encode(LENGTH);
    let title = "Café – 東京";
    let tag = Id3Tag::new().title(title);
    let decoded = decode(tagged(&mp3, &tag, LENGTH));
    assert_eq!(decoded.title.as_deref(), Some(title));
}

#[test]
fn tagged_stream_decodes_to_the_same_samples_as_the_untagged_one() {
    let mp3 = encode(LENGTH);
    let tag = Id3Tag::new()
        .title("T")
        .artist("A")
        .album("L")
        .year(2026)
        .comment("c")
        .chapter("Intro", 0, Some(44_100))
        .chapter("Verse", 44_100, None)
        .chapter("Outro", 88_200, None);
    let plain = decode(mp3.clone());
    let with_tag = decode(tagged(&mp3, &tag, LENGTH));
    assert_eq!(with_tag.samples.len(), plain.samples.len());
    assert_eq!(with_tag.samples, plain.samples);
    assert!(
        !plain.samples.is_empty(),
        "the stream decodes to some audio"
    );
}

/// Opt-in: ffprobe reads the title, artist and chapters back, and ffmpeg
/// decodes the tagged file to exactly the input length. Run with
/// `cargo test --test id3 -- --ignored`; skipped (passes) without ffmpeg.
#[test]
#[ignore = "runs ffprobe and ffmpeg; cargo test --test id3 -- --ignored"]
fn ffprobe_and_ffmpeg_read_the_tagged_file() {
    if !tool_available("ffprobe") || !tool_available("ffmpeg") {
        eprintln!("skipped: ffprobe or ffmpeg is not installed");
        return;
    }
    let tag = Id3Tag::new()
        .title("Café – 東京")
        .artist("Ian Martin")
        .chapter("Intro", 0, Some(44_100))
        .chapter("Verse", 44_100, None)
        .chapter("Outro", 88_200, None);
    let mp3 = tagged(&encode(LENGTH), &tag, LENGTH);
    let path = std::env::temp_dir().join(format!("craft-mp3-id3-{}.mp3", std::process::id()));
    std::fs::write(&path, &mp3).unwrap();

    let probe = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_chapters",
            "-show_format",
            "-of",
            "default",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(probe.status.success(), "ffprobe failed");
    let text = String::from_utf8_lossy(&probe.stdout);
    let (format, chapters) = parse_probe(&text);
    assert_eq!(
        format.get("TAG:title").map(String::as_str),
        Some("Café – 東京")
    );
    assert_eq!(
        format.get("TAG:artist").map(String::as_str),
        Some("Ian Martin")
    );

    let expected = [
        ("Intro", 0.0, 1.0),
        ("Verse", 1.0, 2.0),
        ("Outro", 2.0, 3.0),
    ];
    assert_eq!(chapters.len(), expected.len(), "{text}");
    for (chapter, (title, start, end)) in chapters.iter().zip(expected) {
        assert_eq!(chapter.get("TAG:title").map(String::as_str), Some(title));
        let start_time: f64 = chapter["start_time"].parse().unwrap();
        let end_time: f64 = chapter["end_time"].parse().unwrap();
        assert!(
            (start_time - start).abs() < 0.001,
            "{title} starts at {start_time}"
        );
        assert!((end_time - end).abs() < 0.001, "{title} ends at {end_time}");
    }

    let decoded = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args(["-f", "f32le", "-ar", "44100", "-ac", "2", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success(), "ffmpeg failed");
    assert_eq!(
        decoded.stdout.len() / 8,
        LENGTH,
        "decoded length is the input's"
    );
    std::fs::remove_file(&path).ok();
}

/// The built wav2mp3 example, next to the test binary's directory.
fn example() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let debug_dir = exe.parent().unwrap().parent().unwrap();
    debug_dir
        .join("examples")
        .join(format!("wav2mp3{}", std::env::consts::EXE_SUFFIX))
}

/// A mono 16-bit 44.1 kHz WAV of `samples` samples of a ramp.
fn wav(samples: usize) -> Vec<u8> {
    let data_len = (samples * 2) as u32;
    let mut out = Vec::new();
    out.extend(b"RIFF");
    out.extend((36 + data_len).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend(RATE.to_le_bytes());
    out.extend((RATE * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data_len.to_le_bytes());
    for i in 0..samples {
        out.extend((((i % 200) as i16 - 100) * 100).to_le_bytes());
    }
    out
}

/// Runs the example on a one-second WAV with the given extra arguments and
/// returns the MP3 it wrote.
fn run_wav2mp3(name: &str, extra: &[&str]) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("craft-mp3-id3-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join(format!("{name}.wav"));
    let output = dir.join(format!("{name}.mp3"));
    std::fs::write(&input, wav(RATE as usize)).unwrap();
    let status = Command::new(example())
        .arg(&input)
        .arg(&output)
        .args(extra)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "wav2mp3 failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    std::fs::read(&output).unwrap()
}

#[test]
fn wav2mp3_tag_flags_write_the_tag_in_front_of_the_info_frame() {
    let mp3 = run_wav2mp3(
        "tagged",
        &[
            "--title",
            "Café – 東京",
            "--artist",
            "Ian Martin",
            "--chapter",
            "Intro@0",
            "--chapter",
            "Outro@0.5",
        ],
    );
    assert_eq!(&mp3[..3], b"ID3");
    assert_eq!(&mp3[3..6], [4, 0, 0], "ID3v2.4.0, no flags");
    let size = mp3[6..10]
        .iter()
        .fold(0usize, |n, &b| (n << 7) | usize::from(b));
    // The Info frame comes straight after the tag, and the audio after it.
    assert_eq!(
        &mp3[10 + size..10 + size + 2],
        [0xFF, 0xFB],
        "Info frame first"
    );
    let decoded = decode(mp3.clone());
    assert_eq!(decoded.title.as_deref(), Some("Café – 東京"));
    assert_eq!(decoded.artist.as_deref(), Some("Ian Martin"));
    assert_eq!(mp3.windows(4).filter(|w| *w == b"CHAP").count(), 2);
    assert_eq!(mp3.windows(4).filter(|w| *w == b"CTOC").count(), 1);
}

#[test]
fn wav2mp3_without_tag_flags_writes_no_tag() {
    let mp3 = run_wav2mp3("plain", &[]);
    assert_eq!(&mp3[..2], [0xFF, 0xFB], "the Info frame is the first thing");
}

fn tool_available(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Splits `ffprobe -of default` output into the [FORMAT] section and the
/// [CHAPTER] sections, each a map of key to value.
fn parse_probe(
    text: &str,
) -> (
    std::collections::HashMap<String, String>,
    Vec<std::collections::HashMap<String, String>>,
) {
    use std::collections::HashMap;
    let mut format = HashMap::new();
    let mut chapters: Vec<HashMap<String, String>> = Vec::new();
    let mut section: Option<&str> = None;
    for line in text.lines() {
        match line {
            "[FORMAT]" => section = Some("format"),
            "[CHAPTER]" => {
                section = Some("chapter");
                chapters.push(HashMap::new());
            }
            l if l.starts_with('[') => section = None,
            l => {
                if let Some((k, v)) = l.split_once('=') {
                    match section {
                        Some("format") => {
                            format.insert(k.to_owned(), v.to_owned());
                        }
                        Some("chapter") => {
                            chapters
                                .last_mut()
                                .unwrap()
                                .insert(k.to_owned(), v.to_owned());
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    (format, chapters)
}
