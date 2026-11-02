//! Byte identity of `encode_i16` and `encode_planar` against `encode` for
//! input lengths around the chunk size, including ones that do not divide
//! evenly into chunks.

use std::f64::consts::PI;

use craft_mp3::{Encoder, EncoderConfig};

/// Lengths in samples: empty, tiny, either side of one and two chunks
/// (4608), and a few that leave a ragged last chunk.
const LENGTHS: [usize; 11] = [0, 1, 2, 575, 4607, 4608, 4609, 9215, 9216, 9217, 13_001];

fn tone(len: usize) -> Vec<i16> {
    (0..len)
        .map(|i| ((2.0 * PI * 440.0 * i as f64 / 44_100.0).sin() * 12_000.0) as i16)
        .collect()
}

fn run(config: EncoderConfig, f: impl FnOnce(&mut Encoder, &mut Vec<u8>)) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(config).unwrap();
    f(&mut encoder, &mut out);
    encoder.finish(&mut out);
    out
}

#[test]
fn encode_i16_is_byte_identical_for_ragged_lengths() {
    for channels in [1u8, 2] {
        for &len in &LENGTHS {
            let samples = tone(len);
            let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
            let config = || EncoderConfig::new(44_100, channels.into(), 128);
            let expected = run(config(), |e, o| e.encode(&scaled, o));
            let actual = run(config(), |e, o| e.encode_i16(&samples, o));
            assert_eq!(actual, expected, "channels {channels}, {len} samples");
        }
    }
}

#[test]
fn encode_planar_is_byte_identical_for_ragged_lengths() {
    for channels in [1usize, 2] {
        for &frames in &LENGTHS {
            let planes: Vec<Vec<f32>> = (0..channels)
                .map(|c| {
                    tone(frames)
                        .iter()
                        .map(|&s| f32::from(s) / 32768.0 / (c + 1) as f32)
                        .collect()
                })
                .collect();
            let slices: Vec<&[f32]> = planes.iter().map(|p| p.as_slice()).collect();
            let mut interleaved = Vec::new();
            for i in 0..frames {
                interleaved.extend(planes.iter().map(|p| p[i]));
            }
            let config = || EncoderConfig::new(44_100, channels as _, 128);
            let expected = run(config(), |e, o| e.encode(&interleaved, o));
            let actual = run(config(), |e, o| e.encode_planar(&slices, o));
            assert_eq!(actual, expected, "channels {channels}, {frames} frames");
        }
    }
}
