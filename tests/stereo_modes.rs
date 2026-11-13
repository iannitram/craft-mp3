//! Tests for forced stereo modes (issue #35).

use std::f64::consts::PI;
use std::io::Cursor;

use craft_mp3::header::{ChannelMode, FrameHeader};
use craft_mp3::{ENCODER_DELAY, Encoder, EncoderConfig, StereoMode};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn test_signal(sample_rate: u32, channels: u16, seconds: f64) -> Vec<f32> {
    let rate = f64::from(sample_rate);
    let len = (rate * seconds) as usize;
    let mut out = Vec::with_capacity(len * usize::from(channels));
    for i in 0..len {
        let t = i as f64 / rate;
        let chirp = (2.0 * PI * (200.0 + 800.0 * t * t) * t).sin() as f32 * 0.4;
        let burst = ((2.0 * PI * 1000.0 * t).sin() * (-3.0 * t).exp()) as f32 * 0.4;
        match channels {
            1 => out.push(chirp),
            2 => {
                out.push(chirp);
                out.push(burst);
            }
            _ => unreachable!(),
        }
    }
    out
}

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

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let k = [
        0x428a2f98u32,
        0x71374491,
        0xb5c0fbcf,
        0xe9b5dba5,
        0x3956c25b,
        0x59f111f1,
        0x923f82a4,
        0xab1c5ed5,
        0xd807aa98,
        0x12835b01,
        0x243185be,
        0x550c7dc3,
        0x72be5d74,
        0x80deb1fe,
        0x9bdc06a7,
        0xc19bf174,
        0xe49b69c1,
        0xefbe4786,
        0x0fc19dc6,
        0x240ca1cc,
        0x2de92c6f,
        0x4a7484aa,
        0x5cb0a9dc,
        0x76f988da,
        0x983e5152,
        0xa831c66d,
        0xb00327c8,
        0xbf597fc7,
        0xc6e00bf3,
        0xd5a79147,
        0x06ca6351,
        0x14292967,
        0x27b70a85,
        0x2e1b2138,
        0x4d2c6dfc,
        0x53380d13,
        0x650a7354,
        0x766a0abb,
        0x81c2c92e,
        0x92722c85,
        0xa2bfe8a1,
        0xa81a664b,
        0xc24b8b70,
        0xc76c51a3,
        0xd192e819,
        0xd6990624,
        0xf40e3585,
        0x106aa070,
        0x19a4c116,
        0x1e376c08,
        0x2748774c,
        0x34b0bcb5,
        0x391c0cb3,
        0x4ed8aa4a,
        0x5b9cca4f,
        0x682e6ff3,
        0x748f82ee,
        0x78a5636f,
        0x84c87814,
        0x8cc70208,
        0x90befffa,
        0xa4506ceb,
        0xbef9a3f7,
        0xc67178f2,
    ];

    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, &b) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(b);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 32];
    for (i, val) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&val.to_be_bytes());
    }
    out
}

#[test]
fn default_output_unchanged() {
    let input = test_signal(44_100, 2, 1.0);

    // Default config (stereo_mode defaults to Auto).
    let config = EncoderConfig::new(44_100, 2, 128);
    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    encoder.encode(&input, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }

    assert_eq!(mp3.len(), 17135);
    assert_eq!(fnv1a(&mp3), 0x25fbc5716c481454);
    assert_eq!(
        sha256(&mp3),
        [
            0x16, 0x1c, 0x9f, 0x5f, 0xb7, 0x23, 0x5c, 0x82, 0xfd, 0x19, 0xd7, 0x05, 0xf2, 0xb5,
            0x53, 0x26, 0x62, 0x4e, 0x86, 0xdf, 0x10, 0xe1, 0x27, 0x8c, 0xe3, 0x74, 0xfc, 0x4c,
            0x52, 0xee, 0x34, 0x83,
        ]
    );

    // Explicit StereoMode::Auto must produce byte-identical output.
    let mut config_auto = EncoderConfig::new(44_100, 2, 128);
    config_auto.stereo_mode = StereoMode::Auto;
    let mut encoder_auto = Encoder::new(config_auto).unwrap();
    let mut mp3_auto = Vec::new();
    encoder_auto.encode(&input, &mut mp3_auto);
    if let Some(tag) = encoder_auto.finish(&mut mp3_auto) {
        tag.write_to(&mut mp3_auto);
    }
    assert_eq!(mp3_auto, mp3);
}

#[test]
fn forced_ms_sets_mode_bit_in_every_frame() {
    let mut config = EncoderConfig::new(44_100, 2, 128);
    config.stereo_mode = StereoMode::MidSide;

    // Use an anti-correlated / independent signal where Auto would not pick mid/side.
    let rate = 44_100.0;
    let len = 44_100;
    let mut input = Vec::with_capacity(len * 2);
    for i in 0..len {
        let t = i as f64 / rate;
        let l = (2.0 * PI * 300.0 * t).sin() as f32 * 0.5;
        let r = -(2.0 * PI * 1200.0 * t).sin() as f32 * 0.5;
        input.push(l);
        input.push(r);
    }

    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    encoder.encode(&input, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }

    let mut pos = 0;
    let mut frame_count = 0;
    while pos < mp3.len() {
        let header = FrameHeader::parse(mp3[pos..pos + 4].try_into().unwrap())
            .unwrap_or_else(|e| panic!("bad header at pos {pos}: {e}"));
        match header.mode {
            ChannelMode::JointStereo { mid_side, .. } => {
                assert!(mid_side, "frame at {pos} should have mid_side enabled");
            }
            other => panic!("expected JointStereo, found {other:?} at pos {pos}"),
        }

        // Mode extension is bits 4..5 of byte 3. Bit 5 is ms_stereo.
        let mode_extension = (mp3[pos + 3] >> 4) & 0b11;
        assert_eq!(
            mode_extension & 0b10,
            0b10,
            "mode extension mid_side bit must be 1 at pos {pos}"
        );

        pos += header.frame_len();
        frame_count += 1;
    }
    assert!(frame_count > 10);
    assert_eq!(pos, mp3.len());
}

#[test]
fn forced_lr_round_trips_through_symphonia() {
    let mut config = EncoderConfig::new(44_100, 2, 128);
    config.stereo_mode = StereoMode::LeftRight;

    // Correlated signal where Auto would normally choose mid/side.
    let input = test_signal(44_100, 2, 1.0);
    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    encoder.encode(&input, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }

    // Check that every frame has mid/side disabled.
    let mut pos = 0;
    let mut frame_count = 0;
    while pos < mp3.len() {
        let header = FrameHeader::parse(mp3[pos..pos + 4].try_into().unwrap())
            .unwrap_or_else(|e| panic!("bad header at pos {pos}: {e}"));
        match header.mode {
            ChannelMode::JointStereo { mid_side, .. } => {
                assert!(!mid_side, "frame at {pos} should have mid_side disabled");
            }
            other => panic!("expected JointStereo, found {other:?} at pos {pos}"),
        }

        let mode_extension = (mp3[pos + 3] >> 4) & 0b11;
        assert_eq!(
            mode_extension & 0b10,
            0,
            "mode extension mid_side bit must be 0 at pos {pos}"
        );

        pos += header.frame_len();
        frame_count += 1;
    }
    assert!(frame_count > 10);
    assert_eq!(pos, mp3.len());

    let (decoded, channels, rate) = decode(mp3);
    assert_eq!(channels, 2);
    assert_eq!(rate, 44_100);
    assert!(decoded.iter().all(|s| s.is_finite()));
    assert!(decoded.iter().any(|&s| s.abs() > 0.05));
    let expected_samples = (44_100 + ENCODER_DELAY) * 2;
    assert!(
        decoded.len() >= expected_samples,
        "got {} samples, expected at least {expected_samples}",
        decoded.len()
    );
}

#[test]
fn dual_channel_round_trips_through_symphonia() {
    let mut config = EncoderConfig::new(44_100, 2, 128);
    config.stereo_mode = StereoMode::DualChannel;

    let input = test_signal(44_100, 2, 1.0);
    let mut encoder = Encoder::new(config).unwrap();
    let mut mp3 = Vec::new();
    encoder.encode(&input, &mut mp3);
    if let Some(tag) = encoder.finish(&mut mp3) {
        tag.write_to(&mut mp3);
    }

    let mut pos = 0;
    let mut frame_count = 0;
    while pos < mp3.len() {
        let header = FrameHeader::parse(mp3[pos..pos + 4].try_into().unwrap())
            .unwrap_or_else(|e| panic!("bad header at pos {pos}: {e}"));
        assert_eq!(header.mode, ChannelMode::DualChannel);

        // Mode is bits 6..7 of byte 3. Dual channel is 0b10.
        let mode_bits = (mp3[pos + 3] >> 6) & 0b11;
        assert_eq!(mode_bits, 0b10);

        pos += header.frame_len();
        frame_count += 1;
    }
    assert!(frame_count > 10);
    assert_eq!(pos, mp3.len());

    let (decoded, channels, rate) = decode(mp3);
    assert_eq!(channels, 2);
    assert_eq!(rate, 44_100);
    assert!(decoded.iter().all(|s| s.is_finite()));
    assert!(decoded.iter().any(|&s| s.abs() > 0.05));
}

fn example() -> std::path::PathBuf {
    let exe = std::env::current_exe().unwrap();
    let debug_dir = exe.parent().unwrap().parent().unwrap();
    debug_dir
        .join("examples")
        .join(format!("wav2mp3{}", std::env::consts::EXE_SUFFIX))
}

fn write_wav(path: &std::path::Path, sample_rate: u32, channels: u16, num_samples: usize) {
    let mut w = Vec::new();
    let data_bytes = num_samples * usize::from(channels) * 2;
    w.extend(b"RIFF");
    w.extend(((36 + data_bytes) as u32).to_le_bytes());
    w.extend(b"WAVE");
    w.extend(b"fmt ");
    w.extend(16u32.to_le_bytes());
    w.extend(1u16.to_le_bytes()); // PCM
    w.extend(channels.to_le_bytes());
    w.extend(sample_rate.to_le_bytes());
    w.extend((sample_rate * u32::from(channels) * 2).to_le_bytes());
    w.extend((channels * 2).to_le_bytes());
    w.extend(16u16.to_le_bytes());
    w.extend(b"data");
    w.extend((data_bytes as u32).to_le_bytes());
    for i in 0..num_samples {
        for ch in 0..channels {
            let s = (((i + usize::from(ch) * 100) % 200) as i16 - 100) * 100;
            w.extend(s.to_le_bytes());
        }
    }
    std::fs::write(path, &w).unwrap();
}

#[test]
fn wav2mp3_stereo_flags() {
    let dir = std::env::temp_dir().join(format!("craft-mp3-stereo-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let stereo_wav = dir.join("stereo.wav");
    let mono_wav = dir.join("mono.wav");
    write_wav(&stereo_wav, 44_100, 2, 44_100);
    write_wav(&mono_wav, 44_100, 1, 44_100);

    // 1. Stereo input with --stereo ms succeeds and sets mid_side
    let out_ms = dir.join("out_ms.mp3");
    let res = std::process::Command::new(example())
        .arg(&stereo_wav)
        .arg(&out_ms)
        .arg("--stereo")
        .arg("ms")
        .output()
        .expect("runs wav2mp3");
    assert_eq!(
        res.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let mp3 = std::fs::read(&out_ms).unwrap();
    let header = FrameHeader::parse(mp3[..4].try_into().unwrap()).unwrap();
    match header.mode {
        ChannelMode::JointStereo { mid_side, .. } => assert!(mid_side),
        _ => panic!("expected JointStereo"),
    }

    // 2. Stereo input with --stereo lr succeeds and clears mid_side
    let out_lr = dir.join("out_lr.mp3");
    let res = std::process::Command::new(example())
        .arg(&stereo_wav)
        .arg(&out_lr)
        .arg("--stereo")
        .arg("lr")
        .output()
        .expect("runs wav2mp3");
    assert_eq!(
        res.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let mp3_lr = std::fs::read(&out_lr).unwrap();
    let header_lr = FrameHeader::parse(mp3_lr[..4].try_into().unwrap()).unwrap();
    match header_lr.mode {
        ChannelMode::JointStereo { mid_side, .. } => assert!(!mid_side),
        _ => panic!("expected JointStereo"),
    }

    // 3. Stereo input with --stereo dual succeeds
    let out_dual = dir.join("out_dual.mp3");
    let res = std::process::Command::new(example())
        .arg(&stereo_wav)
        .arg(&out_dual)
        .arg("--stereo")
        .arg("dual")
        .output()
        .expect("runs wav2mp3");
    assert_eq!(
        res.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&res.stderr)
    );
    let mp3_dual = std::fs::read(&out_dual).unwrap();
    let header_dual = FrameHeader::parse(mp3_dual[..4].try_into().unwrap()).unwrap();
    assert_eq!(header_dual.mode, ChannelMode::DualChannel);

    // 4. Mono input with --stereo gives a clear error and exits 1
    let out_mono = dir.join("out_mono.mp3");
    let res = std::process::Command::new(example())
        .arg(&mono_wav)
        .arg(&out_mono)
        .arg("--stereo")
        .arg("ms")
        .output()
        .expect("runs wav2mp3");
    assert_eq!(res.status.code(), Some(1));
    let err = String::from_utf8_lossy(&res.stderr);
    assert!(
        err.contains("stereo option is not supported for mono input"),
        "unexpected stderr: {err}"
    );

    // Clean up
    let _ = std::fs::remove_dir_all(&dir);
}
