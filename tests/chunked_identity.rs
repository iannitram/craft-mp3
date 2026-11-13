//! `encode_i16` and `encode_planar` work in fixed-size chunks; their output
//! must stay byte-identical to a single `encode` call over the same samples.

use std::f64::consts::PI;

use craft_mp3::{Encoder, EncoderConfig};

/// Longer than one chunk (4608 samples) by a wide margin.
const FRAMES: usize = 100_000;

fn sine(frequency: f64, amplitude: f64) -> Vec<f64> {
    (0..FRAMES)
        .map(|i| amplitude * (2.0 * PI * frequency * i as f64 / 44_100.0).sin())
        .collect()
}

/// Encodes `samples` in one `encode` call and finishes the stream.
fn reference(config: EncoderConfig, samples: &[f32]) -> Vec<u8> {
    let mut encoder = Encoder::new(config).unwrap();
    let mut out = Vec::new();
    encoder.encode(samples, &mut out);
    encoder.finish(&mut out);
    out
}

#[test]
fn encode_i16_stereo_matches_encode_on_scaled_samples() {
    let config = EncoderConfig::new(44_100, 2, 128);
    let left = sine(440.0, 16_000.0);
    let right = sine(440.0, 8_000.0);
    let samples: Vec<i16> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l as i16, r as i16])
        .collect();
    let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();

    let expected = reference(config, &scaled);
    let mut actual = Vec::new();
    let mut encoder = Encoder::new(config).unwrap();
    encoder.encode_i16(&samples, &mut actual);
    encoder.finish(&mut actual);

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);
}

#[test]
fn encode_planar_stereo_matches_encode_on_interleaved_samples() {
    let config = EncoderConfig::new(44_100, 2, 128);
    let left: Vec<f32> = sine(440.0, 0.5).iter().map(|&s| s as f32).collect();
    let right: Vec<f32> = sine(440.0, 0.25).iter().map(|&s| s as f32).collect();
    let interleaved: Vec<f32> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l, r])
        .collect();

    let expected = reference(config, &interleaved);
    let mut actual = Vec::new();
    let mut encoder = Encoder::new(config).unwrap();
    encoder.encode_planar(&[&left, &right], &mut actual);
    encoder.finish(&mut actual);

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);
}

#[test]
fn encode_i16_mono_matches_encode_on_scaled_samples() {
    let config = EncoderConfig::new(44_100, 1, 64);
    let samples: Vec<i16> = sine(440.0, 16_000.0).iter().map(|&s| s as i16).collect();
    let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();

    let expected = reference(config, &scaled);
    let mut actual = Vec::new();
    let mut encoder = Encoder::new(config).unwrap();
    encoder.encode_i16(&samples, &mut actual);
    encoder.finish(&mut actual);

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);
}

#[test]
fn encode_planar_mono_matches_encode_on_samples() {
    let config = EncoderConfig::new(44_100, 1, 64);
    let mono: Vec<f32> = sine(440.0, 0.5).iter().map(|&s| s as f32).collect();

    let expected = reference(config, &mono);
    let mut actual = Vec::new();
    let mut encoder = Encoder::new(config).unwrap();
    encoder.encode_planar(&[&mono], &mut actual);
    encoder.finish(&mut actual);

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);
}

#[test]
fn writer_matches_encode() {
    use std::io::Write;

    let config = EncoderConfig::new(44_100, 2, 128);
    let left = sine(440.0, 16_000.0);
    let right = sine(440.0, 8_000.0);
    let samples: Vec<i16> = left
        .iter()
        .zip(&right)
        .flat_map(|(&l, &r)| [l as i16, r as i16])
        .collect();
    let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();

    let mut encoder = Encoder::new(config).unwrap();
    let mut expected = Vec::new();
    encoder.encode(&scaled, &mut expected);
    if let Some(tag) = encoder.finish(&mut expected) {
        tag.write_to(&mut expected);
    }

    // Interleaved i16 bytes through Writer.
    let encoder = Encoder::new(config).unwrap();
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = craft_mp3::Writer::new(encoder, cursor);
    let bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();

    for chunk in bytes.chunks(1024) {
        writer.write_all(chunk).unwrap();
    }
    let cursor = writer.finish().unwrap();
    let actual = cursor.into_inner();

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);

    // Interleaved f32 bytes through Writer::new_f32.
    let encoder = Encoder::new(config).unwrap();
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = craft_mp3::Writer::new_f32(encoder, cursor);
    let float_bytes: Vec<u8> = scaled.iter().flat_map(|&s| s.to_le_bytes()).collect();

    for chunk in float_bytes.chunks(1024) {
        writer.write_all(chunk).unwrap();
    }
    let cursor = writer.finish().unwrap();
    let actual_f32 = cursor.into_inner();

    assert_eq!(actual_f32, expected);
}
