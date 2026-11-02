//! Extra test for issue #10: a zero-size `data` chunk must encode exactly the
//! same bytes as a correctly sized one, so the samples really run to the end
//! of the file. Runs the built example (plain `cargo test` builds examples).

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn example() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let debug_dir = exe.parent().unwrap().parent().unwrap();
    debug_dir
        .join("examples")
        .join(format!("wav2mp3{}", std::env::consts::EXE_SUFFIX))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("craft-mp3-zero-size-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A 44.1 kHz mono 16-bit WAV whose data chunk carries `size_field`.
fn wav(samples: &[i16], size_field: u32) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut fmt = Vec::new();
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(44_100u32.to_le_bytes());
    fmt.extend(88_200u32.to_le_bytes());
    fmt.extend(2u16.to_le_bytes());
    fmt.extend(16u16.to_le_bytes());

    let mut w = Vec::new();
    w.extend(b"RIFF");
    w.extend((4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    w.extend(b"WAVE");
    w.extend(b"fmt ");
    w.extend((fmt.len() as u32).to_le_bytes());
    w.extend(fmt);
    w.extend(b"data");
    w.extend(size_field.to_le_bytes());
    w.extend(data);
    w
}

fn encode(name: &str, bytes: &[u8]) -> Vec<u8> {
    let input = scratch(&format!("{name}.wav"));
    let output = scratch(&format!("{name}.mp3"));
    fs::write(&input, bytes).unwrap();
    let out = Command::new(example())
        .arg(&input)
        .arg(&output)
        .output()
        .expect("the wav2mp3 example is built (run plain `cargo test`)");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::read(&output).unwrap()
}

#[test]
fn zero_size_data_chunk_encodes_the_same_bytes_as_a_sized_one() {
    let samples: Vec<i16> = (0..4608).map(|i| ((i % 200) as i16 - 100) * 100).collect();
    let sized = encode("sized", &wav(&samples, (samples.len() * 2) as u32));
    let zero = encode("zero", &wav(&samples, 0));
    assert!(!sized.is_empty());
    assert_eq!(zero, sized);
}
