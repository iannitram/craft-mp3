//! Every frame, with the earlier bytes its main data starts in, fits the
//! decoder's 7680-bit input buffer that the standard specifies; a frame
//! that is itself 960 bytes or more borrows nothing.

use craft_mp3::header::FrameHeader;
use craft_mp3::{Encoder, EncoderConfig};

const DECODER_BUFFER_BYTES: usize = 7680 / 8;

/// A quiet tone for the first half (the reservoir fills), then loud noise
/// (the reservoir is spent).
fn easy_then_hard(sample_rate: u32, seconds: f64) -> Vec<f32> {
    let len = (f64::from(sample_rate) * seconds) as usize;
    let mut state: u32 = 0x1234_5678;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state as f32 / u32::MAX as f32) * 1.8 - 0.9
    };
    let mut out = Vec::with_capacity(2 * len);
    for i in 0..len {
        if i < len / 2 {
            let t = i as f64 / f64::from(sample_rate);
            let s = (0.05 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32;
            out.extend([s, s]);
        } else {
            out.extend([noise(), noise()]);
        }
    }
    out
}

/// `(main_data_begin, frame length)` of every frame after the Info tag.
fn frames(mp3: &[u8]) -> Vec<(usize, usize)> {
    let mut at = 0;
    let mut out = Vec::new();
    while at + 4 <= mp3.len() {
        let header = FrameHeader::parse(mp3[at..at + 4].try_into().unwrap())
            .expect("frames follow one another");
        let side = at + 4 + if header.crc { 2 } else { 0 };
        let begin = if header.sample_rate.is_lsf() {
            usize::from(mp3[side])
        } else {
            (usize::from(mp3[side]) << 1) | usize::from(mp3[side + 1] >> 7)
        };
        out.push((begin, header.frame_len()));
        at += header.frame_len();
    }
    assert_eq!(at, mp3.len(), "the stream ends on a frame boundary");
    out.remove(0); // the Info tag frame
    out
}

fn encode(config: EncoderConfig) -> Vec<u8> {
    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    let input = easy_then_hard(config.sample_rate, 3.0);
    encoder.encode(&input, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }
    mp3
}

#[test]
fn no_frame_overflows_the_decoder_buffer() {
    let configs = [
        EncoderConfig::new(32_000, 2, 320),
        EncoderConfig::new(32_000, 2, 256),
        EncoderConfig::new(44_100, 2, 320),
        EncoderConfig::new(44_100, 2, 256),
        EncoderConfig::new(44_100, 2, 192),
        EncoderConfig::new(48_000, 2, 320),
        EncoderConfig::new(16_000, 2, 160),
        EncoderConfig::new(24_000, 2, 160),
        EncoderConfig {
            crc: true,
            ..EncoderConfig::new(44_100, 2, 320)
        },
        EncoderConfig::vbr(44_100, 2, 0),
        EncoderConfig::vbr(32_000, 2, 0),
    ];
    for config in configs {
        for (i, (begin, len)) in frames(&encode(config)).into_iter().enumerate() {
            assert!(
                begin <= DECODER_BUFFER_BYTES.saturating_sub(len),
                "{config:?}: frame {i} of {len} bytes reaches {begin} bytes back"
            );
        }
    }
}

#[test]
fn smaller_frames_still_borrow_from_the_reservoir() {
    // At 128 kbit/s a frame is 418 bytes, so the full 511-byte reach fits.
    let frames = frames(&encode(EncoderConfig::new(44_100, 2, 128)));
    let reach = frames.iter().map(|&(begin, _)| begin).max().unwrap();
    assert!(reach > 400, "the reservoir still lends: {reach} bytes");
}
