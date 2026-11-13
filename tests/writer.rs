//! Integration tests for [`craft_mp3::Writer`].

use std::f64::consts::PI;
use std::fs::File;
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};

use craft_mp3::{ENCODER_DELAY, Encoder, EncoderConfig, Writer};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

fn target_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target")
}

/// Decodes an MP3 stream using symphonia, returning (samples, channels, sample_rate).
fn decode(file: File) -> (Vec<f32>, usize, u32) {
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("mp3");
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .expect("recognised as MP3");
    let mut format = probed.format;
    let track = format.default_track().expect("one audio track");
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .expect("decoder for track");

    let mut samples = Vec::new();
    let mut spec = None;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => panic!("demuxing failed: {e}"),
        };
        let decoded = decoder.decode(&packet).expect("frame decodes");
        spec = Some(*decoded.spec());
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    let spec = spec.expect("at least one frame decoded");
    (samples, spec.channels.count(), spec.rate)
}

#[test]
fn writer_to_file_round_trips() {
    let rate = 44_100;
    let channels = 2;
    let frames = 44_100; // 1 second
    let config = EncoderConfig::new(rate, channels, 128);

    // Generate stereo sine wave.
    let mut pcm_i16 = Vec::with_capacity(frames * channels as usize);
    for i in 0..frames {
        let t = i as f64 / rate as f64;
        let left = (2.0 * PI * 440.0 * t).sin() * 16_000.0;
        let right = (2.0 * PI * 880.0 * t).sin() * 12_000.0;
        pcm_i16.push(left as i16);
        pcm_i16.push(right as i16);
    }
    let pcm_bytes: Vec<u8> = pcm_i16.iter().flat_map(|&s| s.to_le_bytes()).collect();

    let path = target_dir().join("writer_roundtrip_test.mp3");
    let file = File::create(&path).expect("create test file");
    let encoder = Encoder::new(config).expect("create encoder");
    let mut writer = Writer::new(encoder, file);

    for chunk in pcm_bytes.chunks(2048) {
        writer.write_all(chunk).expect("write chunk");
    }
    writer.finish().expect("finish writer");

    // Decode with symphonia and verify sample count, channels, and sample rate.
    let file = File::open(&path).expect("open test file");
    let (decoded, got_channels, got_rate) = decode(file);
    let _ = std::fs::remove_file(&path);

    assert_eq!(got_channels, channels as usize);
    assert_eq!(got_rate, rate);
    // Decoded samples include ENCODER_DELAY (1057) and padding to whole frames.
    assert!(
        decoded.len() >= pcm_i16.len() + ENCODER_DELAY * channels as usize,
        "decoded {} samples, expected at least {}",
        decoded.len(),
        pcm_i16.len() + ENCODER_DELAY * channels as usize
    );
}

struct FailingWriter {
    writes: usize,
}

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.writes >= 3 {
            return Err(io::Error::other("disk error on third write"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn write_errors_propagate() {
    let config = EncoderConfig::new(44_100, 2, 128);
    let encoder = Encoder::new(config).expect("create encoder");
    let sink = FailingWriter { writes: 0 };
    let mut writer = Writer::new(encoder, sink);

    // Each MPEG-1 frame has 1152 samples per channel. For stereo i16, that is
    // 1152 * 2 * 2 = 4608 bytes per frame. Write chunks that trigger frame emissions.
    let frame_bytes = vec![0u8; 4608];
    let mut err = None;
    for _ in 0..10 {
        if let Err(e) = writer.write_all(&frame_bytes) {
            err = Some(e);
            break;
        }
    }

    let err = err.expect("expected error on third write");
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(err.to_string(), "disk error on third write");
}

#[test]
fn ragged_byte_writes_match_single_write() {
    let config = EncoderConfig::new(44_100, 1, 64);
    let samples: Vec<i16> = (0..5000)
        .map(|i| ((i as f64 * 0.1).sin() * 10000.0) as i16)
        .collect();
    let bytes: Vec<u8> = samples.iter().flat_map(|&s| s.to_le_bytes()).collect();

    // Reference: write all in one write.
    let enc1 = Encoder::new(config).unwrap();
    let cur1 = Cursor::new(Vec::new());
    let mut w1 = Writer::new(enc1, cur1);
    w1.write_all(&bytes).unwrap();
    let expected = w1.finish().unwrap().into_inner();

    // Ragged: write with primes and odd chunk sizes (1, 3, 7 bytes).
    let enc2 = Encoder::new(config).unwrap();
    let cur2 = Cursor::new(Vec::new());
    let mut w2 = Writer::new(enc2, cur2);
    let chunk_sizes = [1, 3, 7, 2, 5, 11];
    let mut offset = 0;
    let mut i = 0;
    while offset < bytes.len() {
        let size = chunk_sizes[i % chunk_sizes.len()].min(bytes.len() - offset);
        w2.write_all(&bytes[offset..offset + size]).unwrap();
        offset += size;
        i += 1;
    }
    let actual = w2.finish().unwrap().into_inner();

    assert_eq!(actual, expected);
}

fn reference(config: EncoderConfig, samples: &[f32]) -> Vec<u8> {
    let mut encoder = Encoder::new(config).unwrap();
    let mut expected = Vec::new();
    encoder.encode(samples, &mut expected);
    if let Some(tag) = encoder.finish(&mut expected) {
        tag.write_to(&mut expected);
    }
    expected
}

fn stereo_samples() -> Vec<i16> {
    (0..6000)
        .flat_map(|i| {
            let t = f64::from(i) * 0.05;
            [(t.sin() * 9000.0) as i16, ((t * 1.7).sin() * 7000.0) as i16]
        })
        .collect()
}

/// Feeds `bytes` in the given chunk sizes, cycling through them.
fn feed<W: Write>(writer: &mut Writer<W>, bytes: &[u8], sizes: &[usize]) {
    let mut offset = 0;
    let mut i = 0;
    while offset < bytes.len() {
        let end = (offset + sizes[i % sizes.len()]).min(bytes.len());
        writer.write_all(&bytes[offset..end]).unwrap();
        offset = end;
        i += 1;
    }
}

#[test]
fn ragged_stereo_writes_match_encode() {
    let config = EncoderConfig::new(44_100, 2, 128);
    let samples = stereo_samples();
    let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
    let expected = reference(config, &scaled);
    let i16_bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let f32_bytes: Vec<u8> = scaled.iter().flat_map(|s| s.to_le_bytes()).collect();

    for sizes in [&[1, 3, 7, 5, 11][..], &[4093], &[1]] {
        let mut w = Writer::new_i16(Encoder::new(config).unwrap(), Cursor::new(Vec::new()));
        feed(&mut w, &i16_bytes, sizes);
        assert_eq!(w.finish().unwrap().into_inner(), expected, "i16 {sizes:?}");

        let mut w = Writer::new_f32(Encoder::new(config).unwrap(), Cursor::new(Vec::new()));
        feed(&mut w, &f32_bytes, sizes);
        assert_eq!(w.finish().unwrap().into_inner(), expected, "f32 {sizes:?}");
    }
}

#[test]
fn finish_keeps_data_written_before_the_stream() {
    let config = EncoderConfig::new(44_100, 2, 128);
    let samples = stereo_samples();
    let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
    let expected = reference(config, &scaled);
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

    let prefix = b"ID3-stand-in-bytes";
    let mut cursor = Cursor::new(Vec::new());
    cursor.write_all(prefix).unwrap();
    let mut w = Writer::new(Encoder::new(config).unwrap(), cursor);
    feed(&mut w, &bytes, &[1000, 3]);
    let cursor = w.finish().unwrap();

    // The position is back at the end of the stream.
    let len = cursor.get_ref().len() as u64;
    assert_eq!(cursor.position(), len);
    let out = cursor.into_inner();
    assert_eq!(&out[..prefix.len()], prefix);
    assert_eq!(&out[prefix.len()..], &expected[..]);
}
