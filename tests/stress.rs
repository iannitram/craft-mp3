//! Randomized configurations and signals: every stream must be a clean
//! sequence of frames that an independent decoder plays.

use std::io::Cursor;

use craft_mp3::header::{Bitrate, FrameHeader, SampleRate};
use craft_mp3::{ENCODER_DELAY, Encoder, EncoderConfig};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// A small deterministic generator, so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn unit(&mut self) -> f32 {
        self.below(1 << 24) as f32 / (1 << 24) as f32 * 2.0 - 1.0
    }
}

/// Stretches of silence, tones, noise and clicks at random levels.
fn signal(rng: &mut Rng, rate: u32, channels: usize, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    while out.len() < frames * channels {
        let len = 200 + rng.below(rate as u64 / 4) as usize;
        let level = rng.unit().abs() * 1.2;
        let kind = rng.below(4);
        let freq = 20.0 + rng.below(u64::from(rate) / 2 - 20) as f32;
        for n in 0..len {
            for _ in 0..channels {
                let t = n as f32 / rate as f32;
                let x = match kind {
                    0 => 0.0,
                    1 => (std::f32::consts::TAU * freq * t).sin(),
                    2 => rng.unit(),
                    _ => rng.unit() * (-t * 400.0).exp(),
                };
                out.push(x * level);
            }
        }
    }
    out.truncate(frames * channels);
    out
}

fn decode(mp3: Vec<u8>) -> Vec<f32> {
    let source = MediaSourceStream::new(Box::new(Cursor::new(mp3)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("recognised as MP3")
        .format;
    let track = format.default_track().expect("a track");
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("a decoder");
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

#[test]
fn random_configurations_produce_clean_streams() {
    // STRESS_CASES and STRESS_SEED run longer or different sweeps locally.
    let env = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
    let cases = env("STRESS_CASES").unwrap_or(if cfg!(debug_assertions) { 24 } else { 150 });
    let mut rng = Rng(env("STRESS_SEED").unwrap_or(0x5EED));
    let rates = [16_000, 22_050, 24_000, 32_000, 44_100, 48_000];
    for case in 0..cases {
        let rate = rates[rng.below(rates.len() as u64) as usize];
        let channels = 1 + rng.below(2) as u16;
        let sample_rate = SampleRate::from_hz(rate).unwrap();
        let bitrates: Vec<Bitrate> = Bitrate::all(sample_rate).collect();
        let mut config = if rng.below(3) == 0 {
            EncoderConfig::vbr(rate, channels, rng.below(10) as u8)
        } else {
            let kbps = bitrates[rng.below(bitrates.len() as u64) as usize].kbps();
            EncoderConfig::new(rate, channels, kbps)
        };
        config.crc = rng.below(2) == 0;
        config.info_tag = rng.below(4) != 0;
        let what = format!("case {case}: {config:?}");

        let frames = 1 + rng.below(u64::from(rate) / 2) as usize;
        let input = signal(&mut rng, rate, usize::from(channels), frames);
        let chunk = 1 + rng.below(5000) as usize;

        let mut encoder = Encoder::new(config).unwrap_or_else(|e| panic!("{what}: {e}"));
        let mut mp3 = Vec::new();
        for part in input.chunks(chunk) {
            encoder.encode(part, &mut mp3);
        }
        if let Some(tag) = encoder.finish(&mut mp3) {
            tag.write_to(&mut mp3);
        }

        // The stream is whole frames of the configured format, back to back.
        let mut pos = 0;
        while pos < mp3.len() {
            let header = FrameHeader::parse(mp3[pos..pos + 4].try_into().unwrap())
                .unwrap_or_else(|e| panic!("{what}: bad header at {pos}: {e}"));
            assert_eq!(header.sample_rate, sample_rate, "{what}");
            assert_eq!(header.mode.channels(), usize::from(channels), "{what}");
            if config.vbr_quality.is_none() && pos > 0 {
                assert_eq!(header.bitrate.kbps(), config.bitrate_kbps, "{what}");
            }
            pos += header.frame_len();
        }
        assert_eq!(pos, mp3.len(), "{what}: trailing partial frame");

        let output = decode(mp3);
        assert!(output.iter().all(|s| s.is_finite()), "{what}");
        let expected = (frames + ENCODER_DELAY) * usize::from(channels);
        assert!(
            output.len() >= expected,
            "{what}: {} samples, expected {expected}",
            output.len()
        );
    }
}
