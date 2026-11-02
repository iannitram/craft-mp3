//! Locked tests for issue #10: the wav2mp3 example's three input checks.
//! Runs the built example (cargo test builds examples). Nothing but local
//! temp files.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const PCM_GUID: [u8; 16] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];
const FLOAT_GUID: [u8; 16] = [
    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
];

fn example() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let debug_dir = exe.parent().unwrap().parent().unwrap();
    debug_dir
        .join("examples")
        .join(format!("wav2mp3{}", std::env::consts::EXE_SUFFIX))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-mp3-wav2mp3-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// 16-bit mono PCM bytes: a ramp that is not silence.
fn pcm(samples: usize) -> Vec<u8> {
    (0..samples)
        .flat_map(|i| (((i % 200) as i16 - 100) * 100).to_le_bytes())
        .collect()
}

fn fmt_plain(rate: u32) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend(1u16.to_le_bytes());
    f.extend(1u16.to_le_bytes());
    f.extend(rate.to_le_bytes());
    f.extend((rate * 2).to_le_bytes());
    f.extend(2u16.to_le_bytes());
    f.extend(16u16.to_le_bytes());
    f
}

fn fmt_extensible(rate: u32, guid: [u8; 16]) -> Vec<u8> {
    let mut f = fmt_plain(rate);
    f[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    f.extend(22u16.to_le_bytes()); // cbSize
    f.extend(16u16.to_le_bytes()); // valid bits
    f.extend(4u32.to_le_bytes()); // channel mask
    f.extend(guid);
    f
}

fn wav(fmt: &[u8], data_size_field: u32, data: &[u8]) -> Vec<u8> {
    let mut w = Vec::new();
    w.extend(b"RIFF");
    w.extend((4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    w.extend(b"WAVE");
    w.extend(b"fmt ");
    w.extend((fmt.len() as u32).to_le_bytes());
    w.extend(fmt);
    w.extend(b"data");
    w.extend(data_size_field.to_le_bytes());
    w.extend(data);
    w
}

fn run(name: &str, wav_bytes: &[u8], extra: &[&str]) -> (Output, PathBuf) {
    let input = scratch(&format!("{name}.wav"));
    let output = scratch(&format!("{name}.mp3"));
    let _ = fs::remove_file(&output);
    fs::write(&input, wav_bytes).unwrap();
    let out = Command::new(example())
        .arg(&input)
        .arg(&output)
        .args(extra)
        .output()
        .expect("the wav2mp3 example is built (run plain `cargo test`)");
    (out, output)
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn zero_size_data_chunk_uses_the_rest_of_the_file() {
    let data = pcm(4608);
    let (out, mp3) = run("zero-size", &wav(&fmt_plain(44_100), 0, &data), &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("warning: data chunk size is 0; using the rest of the file (9216 bytes)")
    );
    assert!(fs::metadata(&mp3).unwrap().len() > 0);
}

#[test]
fn zero_size_data_chunk_with_nothing_after_it_is_an_error() {
    let (out, _) = run("zero-empty", &wav(&fmt_plain(44_100), 0, &[]), &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("error: no audio data"));
}

#[test]
fn extensible_integer_pcm_is_accepted() {
    let data = pcm(4608);
    let bytes = wav(&fmt_extensible(44_100, PCM_GUID), data.len() as u32, &data);
    let (out, mp3) = run("ext-pcm", &bytes, &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(fs::metadata(&mp3).unwrap().len() > 0);
}

#[test]
fn extensible_float_subformat_is_rejected_even_at_16_bits() {
    let data = pcm(4608);
    let bytes = wav(
        &fmt_extensible(44_100, FLOAT_GUID),
        data.len() as u32,
        &data,
    );
    let (out, _) = run("ext-float", &bytes, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("error: unsupported WAV subformat (only integer PCM is supported)")
    );
}

fn rate_case(rate: u32) {
    let data = pcm(4608);
    let bytes = wav(&fmt_plain(rate), data.len() as u32, &data);
    let (out, _) = run(&format!("rate-{rate}"), &bytes, &[]);
    assert_eq!(out.status.code(), Some(1));
    let expected = format!(
        "error: {rate} Hz is not supported; MP3 Layer III supports 16000, 22050, 24000, 32000, 44100 and 48000 Hz"
    );
    assert!(stderr(&out).contains(&expected), "stderr: {}", stderr(&out));
    assert!(!stderr(&out).contains("UnsupportedSampleRate"));
}

#[test]
fn rate_8000_is_rejected_with_a_clear_message() {
    rate_case(8000);
}

#[test]
fn rate_11025_is_rejected_with_a_clear_message() {
    rate_case(11_025);
}

#[test]
fn rate_12000_is_rejected_with_a_clear_message() {
    rate_case(12_000);
}

#[test]
fn rate_96000_is_rejected_with_a_clear_message() {
    rate_case(96_000);
}

#[test]
fn bitrate_together_with_vbr_warns() {
    let data = pcm(4608);
    let bytes = wav(&fmt_plain(44_100), data.len() as u32, &data);
    let (out, _) = run("vbr-and-rate", &bytes, &["128", "--vbr", "4"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("warning: --vbr given, so the bitrate is used as the per-frame maximum")
    );
}
