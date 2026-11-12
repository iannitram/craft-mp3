//! Locked tests for issue #10: the wav2mp3 example's three input checks.
//! Runs the built example (cargo test builds examples). Nothing but local
//! temp files.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::{Command, Output};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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

fn fmt_plain_24(rate: u32) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend(1u16.to_le_bytes());
    f.extend(1u16.to_le_bytes());
    f.extend(rate.to_le_bytes());
    f.extend((rate * 3).to_le_bytes());
    f.extend(3u16.to_le_bytes());
    f.extend(24u16.to_le_bytes());
    f
}

fn fmt_extensible_24(rate: u32, guid: [u8; 16]) -> Vec<u8> {
    let mut f = fmt_plain_24(rate);
    f[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    f.extend(22u16.to_le_bytes()); // cbSize
    f.extend(24u16.to_le_bytes()); // valid bits
    f.extend(4u32.to_le_bytes()); // channel mask
    f.extend(guid);
    f
}

fn fmt_float_plain(rate: u32) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend(3u16.to_le_bytes());
    f.extend(1u16.to_le_bytes());
    f.extend(rate.to_le_bytes());
    f.extend((rate * 4).to_le_bytes());
    f.extend(4u16.to_le_bytes());
    f.extend(32u16.to_le_bytes());
    f
}

fn fmt_float_extensible(rate: u32, guid: [u8; 16]) -> Vec<u8> {
    let mut f = fmt_float_plain(rate);
    f[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    f.extend(22u16.to_le_bytes()); // cbSize
    f.extend(32u16.to_le_bytes()); // valid bits
    f.extend(4u32.to_le_bytes()); // channel mask
    f.extend(guid);
    f
}

fn decode_mp3(bytes: &[u8]) -> Vec<f32> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
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
    let mut format = probed.format;
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
    samples
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
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
fn float_subformat_with_16_bit_still_rejected() {
    let data = pcm(4608);
    let bytes = wav(
        &fmt_extensible(44_100, FLOAT_GUID),
        data.len() as u32,
        &data,
    );
    let (out, _) = run("ext-float-16", &bytes, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out)
            .contains("error: only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported")
    );
}

#[test]
fn extensible_float_subformat_is_rejected_even_at_16_bits() {
    float_subformat_with_16_bit_still_rejected();
}

#[test]
fn bad_subformat_guid_is_rejected() {
    const BAD_GUID: [u8; 16] = [
        0xAA, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b,
        0x71,
    ];
    let data = pcm(4608);
    let bytes = wav(&fmt_extensible(44_100, BAD_GUID), data.len() as u32, &data);
    let (out, _) = run("ext-bad-guid", &bytes, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("error: unsupported WAV subformat (only integer PCM is supported)")
    );
}

#[test]
fn pcm8_is_rejected_with_a_clear_message() {
    let mut fmt = Vec::new();
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(44_100u32.to_le_bytes());
    fmt.extend(44_100u32.to_le_bytes());
    fmt.extend(1u16.to_le_bytes());
    fmt.extend(8u16.to_le_bytes());

    let data = vec![128u8; 4608];
    let bytes = wav(&fmt, data.len() as u32, &data);
    let (out, _) = run("pcm8", &bytes, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out)
            .contains("error: only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported")
    );
}

#[test]
fn pcm24_decodes_to_same_levels() {
    let n = 44_100;
    let tone: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / 44_100.0;
            0.5 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin()
        })
        .collect();

    let pcm16: Vec<u8> = tone
        .iter()
        .flat_map(|&s| ((s * 32767.0).round() as i16).to_le_bytes())
        .collect();
    let wav16 = wav(&fmt_plain(44_100), pcm16.len() as u32, &pcm16);
    let (out16, mp3_16) = run("pcm16_tone", &wav16, &[]);
    assert_eq!(out16.status.code(), Some(0), "stderr: {}", stderr(&out16));
    let decoded16 = decode_mp3(&fs::read(&mp3_16).unwrap());
    let rms16 = rms(&decoded16);

    let pcm24: Vec<u8> = tone
        .iter()
        .flat_map(|&s| {
            let v = (s * 8_388_607.0).round() as i32;
            let b = v.to_le_bytes();
            [b[0], b[1], b[2]]
        })
        .collect();

    // Plain format 1 (24-bit):
    let wav24 = wav(&fmt_plain_24(44_100), pcm24.len() as u32, &pcm24);
    let (out24, mp3_24) = run("pcm24_tone", &wav24, &[]);
    assert_eq!(out24.status.code(), Some(0), "stderr: {}", stderr(&out24));
    let decoded24 = decode_mp3(&fs::read(&mp3_24).unwrap());
    let rms24 = rms(&decoded24);

    assert!(
        (rms16 - rms24).abs() / rms16 < 0.01,
        "RMS mismatch: 16-bit={rms16}, 24-bit={rms24}"
    );

    // Extensible container with PCM_GUID (24-bit):
    let wav24_ext = wav(
        &fmt_extensible_24(44_100, PCM_GUID),
        pcm24.len() as u32,
        &pcm24,
    );
    let (out24_ext, mp3_24_ext) = run("pcm24_ext_tone", &wav24_ext, &[]);
    assert_eq!(
        out24_ext.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&out24_ext)
    );
    let decoded24_ext = decode_mp3(&fs::read(&mp3_24_ext).unwrap());
    let rms24_ext = rms(&decoded24_ext);

    assert!(
        (rms16 - rms24_ext).abs() / rms16 < 0.01,
        "RMS mismatch: 16-bit={rms16}, 24-bit extensible={rms24_ext}"
    );
}

#[test]
fn float32_wav_encodes() {
    let float_data: Vec<u8> = (0..4608)
        .flat_map(|i| {
            let s = (((i % 200) as f32 - 100.0) / 100.0) * 0.5;
            s.to_le_bytes()
        })
        .collect();

    // Plain format 3:
    let bytes_plain = wav(
        &fmt_float_plain(44_100),
        float_data.len() as u32,
        &float_data,
    );
    let (out_plain, mp3_plain) = run("float-plain", &bytes_plain, &[]);
    assert_eq!(
        out_plain.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&out_plain)
    );
    assert!(fs::metadata(&mp3_plain).unwrap().len() > 0);
    let decoded_plain = decode_mp3(&fs::read(&mp3_plain).unwrap());
    assert!(!decoded_plain.is_empty());

    // Extensible FLOAT_GUID:
    let bytes_ext = wav(
        &fmt_float_extensible(44_100, FLOAT_GUID),
        float_data.len() as u32,
        &float_data,
    );
    let (out_ext, mp3_ext) = run("float-ext", &bytes_ext, &[]);
    assert_eq!(
        out_ext.status.code(),
        Some(0),
        "stderr: {}",
        stderr(&out_ext)
    );
    assert!(fs::metadata(&mp3_ext).unwrap().len() > 0);
    let decoded_ext = decode_mp3(&fs::read(&mp3_ext).unwrap());
    assert!(!decoded_ext.is_empty());
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
