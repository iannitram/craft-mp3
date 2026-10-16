//! Encodes a 16-bit PCM WAV file to MP3.
//!
//! ```text
//! cargo run --release --example wav2mp3 -- input.wav output.mp3 [kbps | --vbr Q] [--crc]
//! ```

use std::error::Error;
use std::fs;

use craft_mp3::{Encoder, EncoderConfig};

fn main() -> Result<(), Box<dyn Error>> {
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

    let base = match vbr {
        Some(quality) => EncoderConfig::vbr(sample_rate, channels, quality),
        None => EncoderConfig::new(sample_rate, channels, bitrate_kbps),
    };
    let config = EncoderConfig { crc, ..base };
    let mut encoder = Encoder::new(config)?;
    let mut mp3 = Vec::new();
    encoder.encode_i16(&samples, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }
    fs::write(output, &mp3)?;
    Ok(())
}

/// Minimal RIFF/WAVE reader: 16-bit integer PCM only.
fn parse_wav(bytes: &[u8]) -> Result<(u32, u16, Vec<i16>), Box<dyn Error>> {
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
        let end = (body + len).min(bytes.len());
        match id {
            b"fmt " => {
                let (tag, channels, rate, bits) = (
                    u16_at(body),
                    u16_at(body + 2),
                    u32_at(body + 4),
                    u16_at(body + 14),
                );
                if !(tag == 1 || tag == 0xFFFE) || bits != 16 {
                    return Err("only 16-bit PCM WAV is supported".into());
                }
                format = Some((rate, channels));
            }
            b"data" => {
                let (rate, channels) = format.ok_or("data chunk before fmt chunk")?;
                let samples = bytes[body..end]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&b| i16::from_le_bytes(b))
                    .collect();
                return Ok((rate, channels, samples));
            }
            _ => {}
        }
        pos = body + len + (len & 1);
    }
    Err("no data chunk".into())
}
