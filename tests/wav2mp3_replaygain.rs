//! Locked tests: the wav2mp3 example's `--replaygain` flag sets
//! `EncoderConfig::replaygain`, so the Info/Xing tag records the input's
//! peak amplitude. Runs the example binary; only local temp files.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::OnceLock;

use craft_mp3::header::FrameHeader;

/// Builds the example once and returns the path cargo reports for it
/// (works whether or not cargo keeps an unhashed copy, see #47).
fn example() -> PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let out = Command::new(cargo)
            .args(["build", "--example", "wav2mp3", "--message-format=json"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("run cargo build --example wav2mp3");
        assert!(out.status.success(), "cargo build --example wav2mp3 failed");
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text
            .lines()
            .find(|l| l.contains("\"name\":\"wav2mp3\"") && l.contains("\"executable\":\""))
            .expect("cargo names the wav2mp3 executable");
        let start = line.find("\"executable\":\"").unwrap() + "\"executable\":\"".len();
        let end = start + line[start..].find('"').unwrap();
        PathBuf::from(line[start..end].replace("\\\\", "\\"))
    })
    .clone()
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-mp3-replaygain-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A 16-bit mono 44100 Hz WAV of 4410 samples: 16384 (0.5 of full scale)
/// everywhere except sample 1000, which is -24576 (0.75 of full scale, but
/// negative). The input's peak amplitude is exactly 0.75.
fn write_wav(path: &PathBuf) {
    let samples: Vec<i16> = (0..4410)
        .map(|i| if i == 1000 { -24576 } else { 16384 })
        .collect();
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut w = Vec::new();
    w.extend(b"RIFF");
    w.extend((36 + data.len() as u32).to_le_bytes());
    w.extend(b"WAVEfmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes()); // PCM
    w.extend(1u16.to_le_bytes()); // mono
    w.extend(44_100u32.to_le_bytes());
    w.extend((44_100u32 * 2).to_le_bytes());
    w.extend(2u16.to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend((data.len() as u32).to_le_bytes());
    w.extend(data);
    fs::write(path, w).unwrap();
}

fn run(args: &[&str]) -> Output {
    Command::new(example())
        .args(args)
        .output()
        .expect("run wav2mp3")
}

/// Encodes the test WAV with `flags` (placed after the two file names, or
/// before them when `flags_first`) and returns the output file's bytes.
fn encode(name: &str, flags: &[&str], flags_first: bool) -> Vec<u8> {
    let wav = scratch(&format!("{name}.wav"));
    let mp3 = scratch(&format!("{name}.mp3"));
    write_wav(&wav);
    let files = [wav.to_str().unwrap(), mp3.to_str().unwrap()];
    let args: Vec<&str> = if flags_first {
        flags.iter().copied().chain(files).collect()
    } else {
        files.into_iter().chain(flags.iter().copied()).collect()
    };
    let out = run(&args);
    assert!(
        out.status.success(),
        "wav2mp3 {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::read(&mp3).unwrap()
}

/// The first frame's tag id ("Info" or "Xing") and its raw 32-bit peak
/// field (offset 131 of the tag body; 1.0 is 1 << 23).
fn tag_peak(mp3: &[u8]) -> ([u8; 4], u32) {
    let header = FrameHeader::parse(mp3[..4].try_into().unwrap()).unwrap();
    let at = 4 + header.side_info_len();
    let id: [u8; 4] = mp3[at..at + 4].try_into().unwrap();
    let peak = u32::from_be_bytes(mp3[at + 131..at + 135].try_into().unwrap());
    (id, peak)
}

/// 0.75 in LAME's 9.23 fixed point: 0.75 * 8388608.
const PEAK_075: u32 = 6_291_456;

#[test]
fn replaygain_flag_records_the_input_peak() {
    // Catches: the flag parsed but never copied into EncoderConfig (peak
    // stays 0), and a peak taken without abs() (0.5, i.e. 4194304).
    let mp3 = encode("cbr", &["--replaygain"], false);
    assert_eq!(tag_peak(&mp3), (*b"Info", PEAK_075));
}

#[test]
fn replaygain_flag_works_before_the_file_names() {
    // Catches: the flag only recognised in one position, or left in the
    // argument list so it is taken as the input file name.
    let mp3 = encode("first", &["--replaygain"], true);
    assert_eq!(tag_peak(&mp3), (*b"Info", PEAK_075));
}

#[test]
fn replaygain_flag_works_with_a_bitrate_and_vbr() {
    // Catches: the flag left in the argument list, where it would be read
    // as the bitrate, and the flag lost on the VBR path (Xing tag).
    let mp3 = encode("cbr192", &["192", "--replaygain"], false);
    assert_eq!(tag_peak(&mp3), (*b"Info", PEAK_075));
    let mp3 = encode("vbr", &["--vbr", "4", "--replaygain"], false);
    assert_eq!(tag_peak(&mp3), (*b"Xing", PEAK_075));
}

#[test]
fn without_the_flag_the_peak_field_stays_zero() {
    // Catches: replaygain switched on for every run (default output must
    // stay byte-identical to 1.1.0).
    let mp3 = encode("plain", &[], false);
    assert_eq!(tag_peak(&mp3), (*b"Info", 0));
}

#[test]
fn usage_names_the_flag() {
    // Catches: the usage line not updated.
    let out = run(&[]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("usage: wav2mp3"), "stderr: {err}");
    assert!(err.contains("[--replaygain]"), "stderr: {err}");
}
