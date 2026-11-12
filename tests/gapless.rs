//! The LAME-layout gapless fields in the Info/Xing tag: each stream is read
//! back field by field, and an opt-in check asks ffmpeg for the exact length.

use std::process::Command;

use craft_mp3::header::FrameHeader;
use craft_mp3::{ENCODER_DELAY, Encoder, EncoderConfig};

/// The encoder delay the tag records; decoders add their own 529 samples.
const DELAY_FIELD: usize = ENCODER_DELAY - 529;

/// Input lengths per channel: empty, short, one sample past a frame, and long.
const LENGTHS: [usize; 4] = [0, 100, 1153, 12_000];

/// Lengths for the ffmpeg check. ffmpeg refuses to open a stream of only two
/// frames ("Failed to find two consecutive MPEG audio frames"), which is what
/// empty and one-sample inputs give; that happens on main too, so the check
/// starts at three frames. The read-back test covers the short lengths.
const FFMPEG_LENGTHS: [usize; 3] = [100, 1153, 12_000];

/// Every sample rate with a bitrate it allows: MPEG-2 rates take 64 kbit/s.
const RATES: [(u32, u32); 6] = [
    (16_000, 64),
    (22_050, 64),
    (24_000, 64),
    (32_000, 128),
    (44_100, 128),
    (48_000, 128),
];

fn config(rate: u32, kbps: u32, channels: u16, vbr: bool) -> EncoderConfig {
    if vbr {
        EncoderConfig::vbr(rate, channels, 4)
    } else {
        EncoderConfig::new(rate, channels, kbps)
    }
}

/// Encodes `per_channel` samples of a tone per channel, with the tag written.
fn encode(config: EncoderConfig, per_channel: usize) -> Vec<u8> {
    let channels = usize::from(config.channels);
    let pcm: Vec<f32> = (0..per_channel * channels)
        .map(|i| (i as f32 * 0.05).sin() * 0.5)
        .collect();
    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    encoder.encode(&pcm, &mut mp3);
    let tag = encoder.finish(&mut mp3).expect("the Info tag is on");
    tag.write_to(&mut mp3);
    mp3
}

/// CRC-16 with polynomial 0xA001, initial value 0, bit by bit: independent
/// of the crate's table.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

/// The tag's fields, read from its frame at the offsets LAME's layout gives.
struct Tag {
    id: [u8; 4],
    frame_len: usize,
    frames: u32,
    quality: u32,
    version: [u8; 9],
    delay: u32,
    padding: u32,
    music_len: u32,
    music_crc: u16,
    tag_crc: u16,
    samples_per_frame: usize,
}

fn read_tag(mp3: &[u8]) -> Tag {
    let header = FrameHeader::parse(mp3[..4].try_into().unwrap()).unwrap();
    let frame_len = header.frame_len();
    let at = 4 + header.side_info_len();
    let frame = &mp3[..frame_len];
    let u32_at = |i: usize| u32::from_be_bytes(frame[at + i..at + i + 4].try_into().unwrap());
    let u16_at = |i: usize| u16::from_be_bytes(frame[at + i..at + i + 2].try_into().unwrap());
    let delay_padding = u32::from_be_bytes([0, frame[at + 141], frame[at + 142], frame[at + 143]]);
    Tag {
        id: frame[at..at + 4].try_into().unwrap(),
        frame_len,
        frames: u32_at(8),
        quality: u32_at(116),
        version: frame[at + 120..at + 129].try_into().unwrap(),
        delay: delay_padding >> 12,
        padding: delay_padding & 0xFFF,
        music_len: u32_at(148),
        music_crc: u16_at(152),
        tag_crc: u16_at(154),
        samples_per_frame: header.samples(),
    }
}

/// Checks every gapless field of one stream against its input length.
fn check_stream(rate: u32, channels: u16, vbr: bool, per_channel: usize) {
    let kbps = RATES.iter().find(|r| r.0 == rate).unwrap().1;
    let label = format!("{rate} Hz, {channels} ch, vbr {vbr}, {per_channel} samples");
    let mp3 = encode(config(rate, kbps, channels, vbr), per_channel);
    let tag = read_tag(&mp3);

    assert_eq!(&tag.id, if vbr { b"Xing" } else { b"Info" }, "{label}");
    assert_eq!(&tag.version, b"LAMEcraft", "{label}");
    assert_eq!(tag.delay as usize, DELAY_FIELD, "{label}");

    // The frames after the tag add up to the frame count and the music length.
    let mut pos = tag.frame_len;
    let mut frames = 0u32;
    while pos < mp3.len() {
        let header = FrameHeader::parse(mp3[pos..pos + 4].try_into().unwrap()).unwrap();
        pos += header.frame_len();
        frames += 1;
    }
    assert_eq!(pos, mp3.len(), "{label}");
    assert_eq!(frames, tag.frames, "{label}");
    assert_eq!(tag.music_len as usize, mp3.len() - tag.frame_len, "{label}");

    // The CRCs: music over the audio bytes, tag over the body before it.
    assert_eq!(tag.music_crc, crc16(&mp3[tag.frame_len..]), "{label}");
    let side_info_len = FrameHeader::parse(mp3[..4].try_into().unwrap())
        .unwrap()
        .side_info_len();
    assert_eq!(tag.tag_crc, crc16(&mp3[..4 + side_info_len + 154]), "{label}");

    // Decoded length, less the delay the decoder skips and the padding, is
    // the input: the field values are the ones ffmpeg uses.
    let decoded = frames as usize * tag.samples_per_frame;
    assert_eq!(
        decoded,
        per_channel + DELAY_FIELD + tag.padding as usize,
        "{label}"
    );
    assert!(tag.padding < tag.samples_per_frame as u32 + 529, "{label}");
    assert!(tag.quality <= 100, "{label}");
    assert!(decoded >= per_channel + ENCODER_DELAY, "{label}");
}

#[test]
fn gapless_fields_read_back_at_every_rate_length_and_mode() {
    for (rate, _) in RATES {
        for channels in [1, 2] {
            for vbr in [false, true] {
                for per_channel in LENGTHS {
                    check_stream(rate, channels, vbr, per_channel);
                }
            }
        }
    }
}

#[test]
fn vbr_quality_is_recorded_on_the_scale_lame_uses() {
    let mp3 = encode(EncoderConfig::vbr(44_100, 2, 0), 1000);
    assert_eq!(read_tag(&mp3).quality, 100);
    let mp3 = encode(EncoderConfig::vbr(44_100, 2, 4), 1000);
    assert_eq!(read_tag(&mp3).quality, 60);
    let mp3 = encode(EncoderConfig::new(44_100, 2, 128), 1000);
    assert_eq!(read_tag(&mp3).quality, 100);
}

fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Opt-in: decodes each stream with ffmpeg and checks it has exactly the
/// input's sample count. Run with `cargo test --test gapless -- --ignored`;
/// skipped (passes) when ffmpeg is not installed.
#[test]
#[ignore = "runs ffmpeg; cargo test --test gapless -- --ignored"]
fn ffmpeg_decodes_exactly_the_input_length() {
    if !ffmpeg_available() {
        eprintln!("skipped: ffmpeg is not installed");
        return;
    }
    let dir = std::env::temp_dir();
    for (rate, kbps) in RATES {
        for channels in [1u16, 2] {
            for vbr in [false, true] {
                for per_channel in FFMPEG_LENGTHS {
                    let mp3 = encode(config(rate, kbps, channels, vbr), per_channel);
                    let path = dir.join(format!(
                        "craft-mp3-gapless-{}-{rate}-{channels}-{vbr}-{per_channel}.mp3",
                        std::process::id()
                    ));
                    std::fs::write(&path, &mp3).unwrap();
                    let rate_arg = rate.to_string();
                    let channels_arg = channels.to_string();
                    let output = Command::new("ffmpeg")
                        .args(["-v", "error", "-i"])
                        .arg(&path)
                        .args(["-f", "f32le", "-ar", &rate_arg, "-ac", &channels_arg, "-"])
                        .output()
                        .unwrap();
                    let _ = std::fs::remove_file(&path);
                    let label = format!("{rate} Hz, {channels} ch, vbr {vbr}, {per_channel}");
                    assert!(
                        output.status.success(),
                        "{label}: ffmpeg failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let bytes_per_frame = 4 * usize::from(channels);
                    assert_eq!(output.stdout.len() % bytes_per_frame, 0, "{label}");
                    assert_eq!(
                        output.stdout.len() / bytes_per_frame,
                        per_channel,
                        "{label}: decoded length"
                    );
                }
            }
        }
    }
}
