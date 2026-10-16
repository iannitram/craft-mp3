//! Encodes test signals, decodes them with an independent decoder
//! (symphonia) and compares the result with the input.

use std::f64::consts::PI;
use std::io::Cursor;

use craft_mp3::{ENCODER_DELAY, Encoder, EncoderConfig};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decodes an MP3 stream to interleaved samples, returning them with the
/// channel count and sample rate the decoder reports.
fn decode(mp3: Vec<u8>) -> (Vec<f32>, usize, u32) {
    let source = MediaSourceStream::new(Box::new(Cursor::new(mp3)), Default::default());
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
    let mut spec = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => panic!("demuxing failed: {e}"),
        };
        let decoded = decoder.decode(&packet).expect("every frame decodes");
        spec = Some(*decoded.spec());
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    let spec = spec.expect("at least one frame");
    (samples, spec.channels.count(), spec.rate)
}

fn encode(config: EncoderConfig, input: &[f32]) -> Vec<u8> {
    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    // Uneven chunks exercise the input buffering.
    for chunk in input.chunks(1000 * usize::from(config.channels) + 1) {
        encoder.encode(chunk, &mut mp3);
    }
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }
    mp3
}

/// A linear chirp on the left channel and a decaying tone burst on the right.
fn test_signal(sample_rate: u32, channels: u16, seconds: f64) -> Vec<f32> {
    let rate = f64::from(sample_rate);
    let len = (rate * seconds) as usize;
    let mut out = Vec::with_capacity(len * usize::from(channels));
    for n in 0..len {
        let t = n as f64 / rate;
        let top = rate / 2.0 * 0.6;
        let chirp = 0.5 * (2.0 * PI * (50.0 * t + (top - 50.0) / (2.0 * seconds) * t * t)).sin();
        out.push(chirp as f32);
        if channels == 2 {
            let burst = 0.5 * (2.0 * PI * 300.0 * t).sin() * (-(t % 0.5) * 8.0).exp();
            out.push(burst as f32);
        }
    }
    out
}

/// Signal-to-noise ratio of `output` against `input` for one channel, skipping
/// the codec delay and the edges.
fn snr_db(input: &[f32], output: &[f32], channels: usize, channel: usize) -> f64 {
    let frames = input.len() / channels;
    let (mut signal, mut noise) = (0.0f64, 0.0f64);
    for n in 2000..frames - 2000 {
        let x = f64::from(input[n * channels + channel]);
        let y = f64::from(output[(n + ENCODER_DELAY) * channels + channel]);
        signal += x * x;
        noise += (x - y) * (x - y);
    }
    10.0 * (signal / noise).log10()
}

#[test]
fn stereo_44100_at_128_kbps_reconstructs_the_input() {
    let config = EncoderConfig::new(44_100, 2, 128);
    let input = test_signal(44_100, 2, 2.0);
    let (output, channels, rate) = decode(encode(config, &input));

    assert_eq!((channels, rate), (2, 44_100));
    assert!(
        output.len() >= input.len() + ENCODER_DELAY * 2,
        "every input sample is decoded"
    );
    for ch in 0..2 {
        let snr = snr_db(&input, &output, 2, ch);
        assert!(snr > 40.0, "channel {ch}: SNR {snr:.1} dB");
    }
}

#[test]
fn every_sample_rate_and_channel_count_decodes() {
    for sample_rate in [32_000, 44_100, 48_000] {
        for channels in [1, 2] {
            let config = EncoderConfig::new(sample_rate, channels, 160);
            let input = test_signal(sample_rate, channels, 0.5);
            let (output, got_channels, got_rate) = decode(encode(config, &input));
            assert_eq!(
                (got_channels, got_rate),
                (usize::from(channels), sample_rate)
            );
            let snr = snr_db(&input, &output, usize::from(channels), 0);
            assert!(
                snr > 30.0,
                "{sample_rate} Hz, {channels} ch: SNR {snr:.1} dB"
            );
        }
    }
}

#[test]
fn every_bitrate_decodes_and_quality_rises_with_bitrate() {
    let input = test_signal(48_000, 1, 0.5);
    let mut last = f64::NEG_INFINITY;
    for kbps in [
        32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ] {
        let config = EncoderConfig::new(48_000, 1, kbps);
        let (output, _, _) = decode(encode(config, &input));
        let snr = snr_db(&input, &output, 1, 0);
        // Allow small dips between neighbouring rates.
        assert!(
            snr > last - 1.0,
            "{kbps} kbit/s: {snr:.1} dB after {last:.1} dB"
        );
        last = last.max(snr);
    }
    assert!(last > 50.0, "best SNR only {last:.1} dB");
}

#[test]
fn silence_and_full_scale_both_decode() {
    let config = EncoderConfig::new(44_100, 2, 128);

    let (output, _, _) = decode(encode(config, &vec![0.0; 44_100]));
    assert!(output.iter().all(|&s| s.abs() < 1e-6));

    // A full-scale square wave is the hardest case for the quantizer range.
    let square: Vec<f32> = (0..44_100)
        .map(|n| if (n / 2 / 50) % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let (output, _, _) = decode(encode(config, &square));
    assert!(output.iter().any(|&s| s.abs() > 0.5));
}

#[test]
fn transients_do_not_smear_ahead_in_time() {
    // Isolated clicks in silence: with long blocks only, quantization noise
    // spreads over the whole 1152-sample window, well before each click.
    let rate = 44_100usize;
    let mut input = vec![0.0f32; rate * 2];
    let clicks = [20_000usize, 45_000, 70_000];
    for &at in &clicks {
        for n in 0..1500 {
            let t = n as f64 / rate as f64;
            input[at + n] = (0.6 * (-t * 300.0).exp() * (2.0 * PI * 2500.0 * t).sin()) as f32;
        }
    }
    let config = EncoderConfig::new(44_100, 1, 64);
    let (output, _, _) = decode(encode(config, &input));

    for &at in &clicks {
        let energy = |range: std::ops::Range<usize>| -> f64 {
            range
                .map(|n| f64::from(output[n + ENCODER_DELAY]).powi(2))
                .sum()
        };
        let click = energy(at..at + 1500);
        // 300 to 1000 samples before the click: past the filterbank's own
        // spread, but inside a long window. Long blocks only give about
        // -25 dB here; short blocks better than -50 dB.
        let before = energy(at - 1000..at - 300);
        let db = 10.0 * (before / click).log10();
        assert!(db < -45.0, "pre-echo {db:.1} dB before the click at {at}");
    }
}

#[test]
fn crc_protected_streams_decode() {
    let config = EncoderConfig {
        crc: true,
        ..EncoderConfig::new(48_000, 2, 128)
    };
    let input = test_signal(48_000, 2, 0.5);
    let mp3 = encode(config, &input);
    // The first audio frame, after the Info tag frame, has its protection
    // bit cleared (0 means a CRC follows).
    let tag_len = 144 * 128_000 / 48_000;
    assert_eq!(&mp3[tag_len..tag_len + 2], [0xFF, 0xFA]);
    let (output, _, _) = decode(mp3);
    let snr = snr_db(&input, &output, 2, 0);
    assert!(snr > 30.0, "SNR {snr:.1} dB");
}

#[test]
fn hostile_input_still_gives_a_decodable_stream() {
    let len = 48_000;
    let cases: [(&str, Vec<f32>); 6] = [
        (
            "NaN",
            (0..len)
                .map(|n| if n % 7 == 0 { f32::NAN } else { 0.3 })
                .collect(),
        ),
        (
            "infinity",
            (0..len)
                .map(|n| if n % 1000 == 0 { f32::INFINITY } else { -0.2 })
                .collect(),
        ),
        (
            "huge",
            (0..len)
                .map(|n| if n % 2 == 0 { 1e6 } else { -1e6 })
                .collect(),
        ),
        ("DC", vec![1.0; len]),
        ("full-scale noise", {
            let mut seed = 1u32;
            (0..len)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    if seed >> 31 == 1 { 1.0 } else { -1.0 }
                })
                .collect()
        }),
        ("subnormal", vec![1e-40; len]),
    ];
    for (name, input) in cases {
        for (rate, channels, kbps) in [(48_000, 2, 32), (44_100, 1, 320), (32_000, 2, 128)] {
            let config = EncoderConfig::new(rate, channels, kbps);
            let mp3 = encode(config, &input);
            let (output, _, _) = decode(mp3);
            assert!(
                output.iter().all(|s| s.is_finite()),
                "{name} at {rate} Hz {kbps} kbit/s"
            );
            if name == "NaN" {
                // NaN samples become silence; the rest of the signal survives.
                let mid = &output[output.len() / 3..output.len() / 2];
                let mean = mid.iter().sum::<f32>() / mid.len() as f32;
                assert!(
                    (mean - 0.3 * 6.0 / 7.0).abs() < 0.05,
                    "NaN case mean {mean}"
                );
            }
        }
    }
}
