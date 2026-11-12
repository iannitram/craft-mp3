//! Encodes a WAV file (16-bit or 24-bit integer PCM, or 32-bit IEEE float) to MP3.
//!
//! ```text
//! cargo run --release --example wav2mp3 -- input.wav output.mp3 [kbps | --vbr Q] [--crc]
//! ```

use std::error::Error;
use std::fs;

use craft_mp3::{Encoder, EncoderConfig};

fn run() -> Result<(), Box<dyn Error>> {
    let mut args: Vec<String> = std::env::args().collect();
    let crc = args.iter().any(|a| a == "--crc");
    args.retain(|a| a != "--crc");
    let vbr = match args.iter().position(|a| a == "--vbr") {
        Some(i) if i + 1 < args.len() => {
            let quality: u8 = args[i + 1].parse()?;
            args.drain(i..i + 2);
            Some(quality)
        }
        Some(_) => return Err("--vbr needs a quality from 0 to 9".into()),
        None => None,
    };
    let [_, input, output, rest @ ..] = args.as_slice() else {
        return Err("usage: wav2mp3 <input.wav> <output.mp3> [kbps | --vbr Q] [--crc]".into());
    };
    let bitrate_kbps = rest.first().map_or(Ok(128), |s| s.parse())?;

    let wav = fs::read(input)?;
    let (sample_rate, channels, samples) = parse_wav(&wav)?;

    if ![16_000, 22_050, 24_000, 32_000, 44_100, 48_000].contains(&sample_rate) {
        return Err(format!("{sample_rate} Hz is not supported; MP3 Layer III supports 16000, 22050, 24000, 32000, 44100 and 48000 Hz").into());
    }
    if vbr.is_some() && !rest.is_empty() {
        eprintln!("warning: --vbr given, so the bitrate is used as the per-frame maximum");
    }

    let base = match vbr {
        Some(quality) => EncoderConfig::vbr(sample_rate, channels, quality),
        None => EncoderConfig::new(sample_rate, channels, bitrate_kbps),
    };
    let config = EncoderConfig { crc, ..base };
    let mut encoder = Encoder::new(config)?;
    let mut mp3 = Vec::new();
    match samples {
        Samples::I16(s) => encoder.encode_i16(&s, &mut mp3),
        Samples::F32(s) => encoder.encode(&s, &mut mp3),
    }
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }
    fs::write(output, &mp3)?;
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WavFormat {
    Pcm16,
    Pcm24,
    Float32,
}

enum Samples {
    I16(Vec<i16>),
    F32(Vec<f32>),
}

/// Minimal RIFF/WAVE reader: 16-bit or 24-bit integer PCM, or 32-bit float.
fn parse_wav(bytes: &[u8]) -> Result<(u32, u16, Samples), Box<dyn Error>> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());

    let mut format = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let len = u32_at(pos + 4) as usize;
        let body = pos + 8;
        let end = if len == 0 && id == b"data" {
            bytes.len()
        } else {
            (body + len).min(bytes.len())
        };
        match id {
            b"fmt " => {
                if len < 16 || body + 16 > bytes.len() {
                    return Err("truncated fmt chunk".into());
                }
                let (tag, channels, rate, bits) = (
                    u16_at(body),
                    u16_at(body + 2),
                    u32_at(body + 4),
                    u16_at(body + 14),
                );
                const PCM_GUID: [u8; 16] = [
                    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00,
                    0x38, 0x9b, 0x71,
                ];
                const FLOAT_GUID: [u8; 16] = [
                    0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xaa, 0x00,
                    0x38, 0x9b, 0x71,
                ];
                let wav_format = if tag == 0xFFFE {
                    if len < 40
                        || body + 40 > bytes.len()
                        || (bytes[body + 24..body + 40] != PCM_GUID
                            && bytes[body + 24..body + 40] != FLOAT_GUID)
                    {
                        return Err(
                            "unsupported WAV subformat (only integer PCM is supported)".into()
                        );
                    }
                    if bytes[body + 24..body + 40] == PCM_GUID {
                        match bits {
                            16 => WavFormat::Pcm16,
                            24 => WavFormat::Pcm24,
                            _ => {
                                return Err(
                                    "only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported"
                                        .into(),
                                );
                            }
                        }
                    } else {
                        match bits {
                            32 => WavFormat::Float32,
                            _ => {
                                return Err(
                                    "only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported"
                                        .into(),
                                );
                            }
                        }
                    }
                } else if tag == 1 {
                    match bits {
                        16 => WavFormat::Pcm16,
                        24 => WavFormat::Pcm24,
                        _ => {
                            return Err(
                                "only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported"
                                    .into(),
                            );
                        }
                    }
                } else if tag == 3 {
                    match bits {
                        32 => WavFormat::Float32,
                        _ => {
                            return Err(
                                "only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported"
                                    .into(),
                            );
                        }
                    }
                } else {
                    return Err(
                        "only 16-bit PCM, 24-bit PCM or 32-bit float WAV is supported".into(),
                    );
                };
                format = Some((rate, channels, wav_format));
            }
            b"data" => {
                let (rate, channels, wav_format) = format.ok_or("data chunk before fmt chunk")?;
                if len == 0 {
                    if end <= body {
                        return Err("no audio data".into());
                    }
                    eprintln!(
                        "warning: data chunk size is 0; using the rest of the file ({} bytes)",
                        end - body
                    );
                }
                let data = &bytes[body..end];
                let samples = match wav_format {
                    WavFormat::Pcm16 => Samples::I16(
                        data.as_chunks::<2>()
                            .0
                            .iter()
                            .map(|&b| i16::from_le_bytes(b))
                            .collect(),
                    ),
                    WavFormat::Pcm24 => Samples::F32(
                        data.as_chunks::<3>()
                            .0
                            .iter()
                            .map(|&b| {
                                let v = i32::from_le_bytes([
                                    b[0],
                                    b[1],
                                    b[2],
                                    if b[2] >= 0x80 { 0xff } else { 0x00 },
                                ]);
                                v as f32 / 8_388_608.0
                            })
                            .collect(),
                    ),
                    WavFormat::Float32 => Samples::F32(
                        data.as_chunks::<4>()
                            .0
                            .iter()
                            .map(|&b| f32::from_le_bytes(b))
                            .collect(),
                    ),
                };
                return Ok((rate, channels, samples));
            }
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    Err("no data chunk".into())
}
