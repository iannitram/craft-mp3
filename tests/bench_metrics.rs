//! The benchmark's quality metrics on known signals.

#[path = "../examples/bench/metrics.rs"]
mod metrics;

use metrics::*;

fn noise(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((s >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0) as f32
        })
        .collect()
}

#[test]
fn identical_signals_hit_the_cap_and_zero_distance() {
    let x: Vec<f32> = noise(44_100, 1).iter().map(|v| v * 0.3).collect();
    assert_eq!(segmental_snr_db(&x, &x, 44_100), Some(SNR_CAP_DB));
    assert_eq!(log_spectral_distance_db(&x, &x, 44_100, 16_000.0), 0.0);
}

#[test]
fn white_noise_at_minus_40_db_gives_40_db_snr() {
    let x: Vec<f32> = noise(88_200, 2).iter().map(|v| v * 0.3).collect();
    let n = noise(88_200, 3);
    let power = |v: &[f32]| v.iter().map(|&a| f64::from(a).powi(2)).sum::<f64>() / v.len() as f64;
    let gain = (power(&x) / power(&n) * 1e-4).sqrt() as f32;
    let y: Vec<f32> = x.iter().zip(&n).map(|(a, b)| a + b * gain).collect();
    let snr = segmental_snr_db(&x, &y, 44_100).unwrap();
    assert!((snr - 40.0).abs() < 0.5, "{snr}");
}

#[test]
fn quiet_frames_are_skipped() {
    let x = vec![1e-5f32; 44_100];
    assert_eq!(segmental_snr_db(&x, &x, 44_100), None);
}

#[test]
fn delay_alignment_finds_1057_exactly() {
    let x = noise(50_000, 4);
    let mut y = vec![0.0f32; 1057];
    y.extend_from_slice(&x);
    assert_eq!(find_delay(&x, &y, 1057, 16), 1057);
    // Found even when the stated delay is a few samples off.
    assert_eq!(find_delay(&x, &y, 1050, 16), 1057);
    assert_eq!(align(&y, 1057, x.len()), x);
}

#[test]
fn bandwidth_sees_a_lowpass() {
    let x = noise(44_100, 5);
    assert_eq!(effective_bandwidth_hz(&x, &x, 44_100), 22_050.0);
    // Four cascaded one-pole filters take well over 60 dB off the top.
    let mut z = x.clone();
    for _ in 0..4 {
        let mut state = 0.0f32;
        for v in z.iter_mut() {
            state = 0.9 * state + 0.1 * *v;
            *v = state;
        }
    }
    assert!(effective_bandwidth_hz(&x, &z, 44_100) < 22_050.0);
}
