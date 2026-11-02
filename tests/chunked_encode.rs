//! Locked tests for issue #9: encode_i16 and encode_planar work in
//! fixed-size chunks and give byte-identical output to `encode`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::f64::consts::PI;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use craft_mp3::{Encoder, EncoderConfig};

/// Records the largest single allocation made while `TRACK` is set.
struct Tracking;
static TRACK: AtomicBool = AtomicBool::new(false);
static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if TRACK.load(Ordering::Relaxed) {
            LARGEST.fetch_max(new_size, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

/// Tests run one at a time so the allocation tracker sees only one encoder.
static SERIAL: Mutex<()> = Mutex::new(());

const FRAMES: usize = 100_000;
/// The old code copied the whole input (800 000 bytes of f32); the chunked
/// code needs about 18 000. Anything under this passes.
const MAX_SINGLE_ALLOCATION: usize = 400_000;

fn config() -> EncoderConfig {
    EncoderConfig::new(44_100, 2, 128)
}

/// 440 Hz sine, stereo, the right channel at half the level.
fn sine_i16() -> Vec<i16> {
    let mut v = Vec::with_capacity(FRAMES * 2);
    for i in 0..FRAMES {
        let s = (2.0 * PI * 440.0 * i as f64 / 44_100.0).sin();
        v.push((s * 16_000.0) as i16);
        v.push((s * 8_000.0) as i16);
    }
    v
}

fn finish(encoder: Encoder, out: &mut Vec<u8>) {
    encoder.finish(out);
}

#[test]
fn encode_i16_matches_encode_on_scaled_samples() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let samples = sine_i16();
    let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();

    let mut expected = Vec::new();
    let mut reference = Encoder::new(config()).unwrap();
    reference.encode(&scaled, &mut expected);
    finish(reference, &mut expected);

    let mut actual = Vec::new();
    let mut encoder = Encoder::new(config()).unwrap();
    encoder.encode_i16(&samples, &mut actual);
    finish(encoder, &mut actual);

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);
}

#[test]
fn encode_planar_matches_encode_on_interleaved_samples() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let left: Vec<f32> = (0..FRAMES)
        .map(|i| (2.0 * PI * 440.0 * i as f64 / 44_100.0).sin() as f32 * 0.5)
        .collect();
    let right: Vec<f32> = left.iter().map(|&s| s * 0.5).collect();
    let mut interleaved = Vec::with_capacity(FRAMES * 2);
    for i in 0..FRAMES {
        interleaved.push(left[i]);
        interleaved.push(right[i]);
    }

    let mut expected = Vec::new();
    let mut reference = Encoder::new(config()).unwrap();
    reference.encode(&interleaved, &mut expected);
    finish(reference, &mut expected);

    let mut actual = Vec::new();
    let mut encoder = Encoder::new(config()).unwrap();
    encoder.encode_planar(&[&left, &right], &mut actual);
    finish(encoder, &mut actual);

    assert!(actual.len() > 10_000);
    assert_eq!(actual, expected);
}

#[test]
fn encode_i16_makes_no_allocation_proportional_to_the_input() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let samples = sine_i16();
    let mut encoder = Encoder::new(config()).unwrap();
    let mut out = Vec::with_capacity(4_000_000);

    LARGEST.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    encoder.encode_i16(&samples, &mut out);
    TRACK.store(false, Ordering::Relaxed);

    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(
        largest < MAX_SINGLE_ALLOCATION,
        "largest single allocation was {largest} bytes"
    );
}

#[test]
fn encode_planar_makes_no_allocation_proportional_to_the_input() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let left: Vec<f32> = (0..FRAMES)
        .map(|i| (2.0 * PI * 440.0 * i as f64 / 44_100.0).sin() as f32 * 0.5)
        .collect();
    let right = left.clone();
    let mut encoder = Encoder::new(config()).unwrap();
    let mut out = Vec::with_capacity(4_000_000);

    LARGEST.store(0, Ordering::Relaxed);
    TRACK.store(true, Ordering::Relaxed);
    encoder.encode_planar(&[&left, &right], &mut out);
    TRACK.store(false, Ordering::Relaxed);

    let largest = LARGEST.load(Ordering::Relaxed);
    assert!(
        largest < MAX_SINGLE_ALLOCATION,
        "largest single allocation was {largest} bytes"
    );
}
