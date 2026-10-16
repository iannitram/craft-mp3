//! A small psychoacoustic model: how much quantization noise each
//! scale-factor band can hide.
//!
//! The allowed noise in a band is the larger of
//! - the absolute threshold of hearing (Terhardt's approximation), with a
//!   full-scale sine taken as 96 dB SPL, and
//! - the masking threshold: band energies spread across neighbouring bands
//!   on the Bark scale (Schroeder's spreading function), lowered by an
//!   offset that depends on how tonal the band is (Johnston): noise masks
//!   noise 5.5 dB below it, but noise must stay 14.5 + z dB below a tone
//!   at z Bark. Tonality comes from the band's spectral flatness.
//!
//! Energies are in the MDCT domain, where a full-scale sine carries an
//! energy of about 1.0 per granule.

use crate::mdct::GRANULE_LINES;
use crate::quantize::SfbBounds;

/// Long-block scale-factor bands, including the top band that has no scale factor.
pub(crate) const BANDS: usize = 22;

/// Masking offset for a noise-like masker, in dB.
const NOISE_OFFSET_DB: f32 = 5.5;

/// Spectral flatness, in dB, at which a band counts as fully tonal.
const TONAL_FLATNESS_DB: f32 = -40.0;

/// Level of a full-scale sine in dB SPL.
const FULL_SCALE_DB_SPL: f64 = 96.0;

/// Absolute threshold of hearing in dB SPL (Terhardt).
fn ath_db(freq_hz: f64) -> f64 {
    let khz = (freq_hz / 1000.0).max(0.01);
    3.64 * khz.powf(-0.8) - 6.5 * (-0.6 * (khz - 3.3).powi(2)).exp() + 1e-3 * khz.powi(4)
}

/// Critical-band rate in Bark (Zwicker).
fn bark(freq_hz: f64) -> f64 {
    13.0 * (0.00076 * freq_hz).atan() + 3.5 * (freq_hz / 7500.0).powi(2).atan()
}

/// Schroeder's spreading function, in linear power, for a masker `dz`
/// Bark below (positive) or above (negative) the masked band.
fn spreading(dz: f64) -> f64 {
    let x = dz + 0.474;
    let db = 15.81 + 7.5 * x - 17.5 * (1.0 + x * x).sqrt();
    10f64.powf(db / 10.0)
}

pub(crate) struct Psycho {
    sfb: &'static SfbBounds,
    /// Threshold of hearing per band, as an energy.
    ath: [f32; BANDS],
    /// `spread[masked][masker]`.
    spread: [[f32; BANDS]; BANDS],
    /// Masking offset for a tonal band, in dB.
    tonal_offset_db: [f32; BANDS],
}

impl Psycho {
    pub(crate) fn new(sample_rate: u32, sfb: &'static SfbBounds) -> Self {
        let line_hz =
            |line: usize| (line as f64 + 0.5) * f64::from(sample_rate) / (2 * GRANULE_LINES) as f64;

        let mut ath = [0.0; BANDS];
        let mut centre = [0.0; BANDS];
        for band in 0..BANDS {
            let lines = sfb[band]..sfb[band + 1];
            // The quietest line in the band sets the band's threshold.
            let db = lines
                .clone()
                .map(|l| ath_db(line_hz(l)))
                .fold(f64::INFINITY, f64::min);
            ath[band] = 10f64.powf((db - FULL_SCALE_DB_SPL) / 10.0) as f32;
            centre[band] = bark(line_hz((lines.start + lines.end) / 2));
        }

        let tonal_offset_db = centre.map(|z| (14.5 + z) as f32);
        let mut spread = [[0.0; BANDS]; BANDS];
        for (masked, row) in spread.iter_mut().enumerate() {
            for (masker, s) in row.iter_mut().enumerate() {
                *s = spreading(centre[masked] - centre[masker]) as f32;
            }
        }

        Self {
            sfb,
            ath,
            spread,
            tonal_offset_db,
        }
    }

    /// Energy of each band.
    pub(crate) fn band_energies(&self, xr: &[f32; GRANULE_LINES]) -> [f32; BANDS] {
        let mut energy = [0.0; BANDS];
        for (band, e) in energy.iter_mut().enumerate() {
            *e = xr[self.sfb[band]..self.sfb[band + 1]]
                .iter()
                .map(|x| x * x)
                .sum();
        }
        energy
    }

    /// How tonal each band is, from 0 (noise) to 1 (a pure tone), judged by
    /// the spectral flatness of the band and its neighbours.
    pub(crate) fn tonality(&self, xr: &[f32; GRANULE_LINES]) -> [f32; BANDS] {
        let mut tonality = [0.0; BANDS];
        for (band, t) in tonality.iter_mut().enumerate() {
            // At least 8 lines, so that flatness means something.
            let (mut lo, mut hi) = (band, band + 1);
            while self.sfb[hi] - self.sfb[lo] < 8 {
                lo = lo.saturating_sub(1);
                hi = (hi + 1).min(BANDS);
            }
            let lines = &xr[self.sfb[lo]..self.sfb[hi]];
            let n = lines.len() as f32;
            let arithmetic = lines.iter().map(|x| x * x).sum::<f32>() / n;
            if arithmetic <= 0.0 {
                continue;
            }
            let floor = arithmetic * 1e-6;
            let log_geometric = lines.iter().map(|x| (x * x + floor).ln()).sum::<f32>() / n;
            let flatness_db = 10.0 * (log_geometric.exp() / arithmetic).log10();
            *t = (flatness_db / TONAL_FLATNESS_DB).clamp(0.0, 1.0);
        }
        tonality
    }

    /// Allowed noise energy for each band of a granule.
    pub(crate) fn thresholds(&self, xr: &[f32; GRANULE_LINES]) -> [f32; BANDS] {
        let energy = self.band_energies(xr);
        let tonality = self.tonality(xr);
        let mut thr = [0.0; BANDS];
        for (band, t) in thr.iter_mut().enumerate() {
            let spread: f32 = self.spread[band]
                .iter()
                .zip(&energy)
                .map(|(s, e)| s * e)
                .sum();
            let a = tonality[band];
            let offset_db = a * self.tonal_offset_db[band] + (1.0 - a) * NOISE_OFFSET_DB;
            *t = (spread * 10f32.powf(-offset_db / 10.0)).max(self.ath[band]);
        }
        thr
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::SFB_LONG_44100;

    #[test]
    fn hearing_is_most_sensitive_around_3_to_4_khz() {
        let best = (100..20_000)
            .step_by(50)
            .min_by(|&a, &b| ath_db(f64::from(a)).total_cmp(&ath_db(f64::from(b))));
        assert!((3000..4500).contains(&best.unwrap()));
        assert!(ath_db(50.0) > 30.0);
        assert!(ath_db(18_000.0) > 60.0);
    }

    #[test]
    fn spreading_is_0_db_in_band_and_falls_off_asymmetrically() {
        assert!((spreading(0.0) - 1.0).abs() < 0.01);
        // Masking reaches further up in frequency than down.
        assert!(spreading(2.0) > spreading(-2.0));
        assert!(spreading(5.0) < 1e-2);
    }

    #[test]
    fn silence_is_held_to_the_threshold_of_hearing() {
        let p = Psycho::new(44_100, &SFB_LONG_44100);
        assert_eq!(p.thresholds(&[0.0; GRANULE_LINES]), p.ath);
    }

    #[test]
    fn a_loud_band_raises_its_own_and_neighbouring_thresholds() {
        let p = Psycho::new(44_100, &SFB_LONG_44100);
        let mut xr = [0.0; GRANULE_LINES];
        let band = 10;
        for x in &mut xr[SFB_LONG_44100[band]..SFB_LONG_44100[band + 1]] {
            *x = 0.1;
        }
        let energy = p.band_energies(&xr)[band];
        let thr = p.thresholds(&xr);
        // A flat band is noise-like: masked 5.5 dB down.
        let expected = energy * 10f32.powf(-NOISE_OFFSET_DB / 10.0);
        assert!(
            (thr[band] / expected - 1.0).abs() < 0.05,
            "{} vs {expected}",
            thr[band]
        );
        assert!(thr[band + 1] > p.ath[band + 1] && thr[band + 1] < thr[band]);
        assert!(thr[band - 1] > p.ath[band - 1] && thr[band - 1] < thr[band]);
        // Far away, only the threshold of hearing is left.
        assert_eq!(thr[0], p.ath[0]);
    }

    #[test]
    fn tones_get_a_lower_threshold_than_noise_of_the_same_energy() {
        let p = Psycho::new(44_100, &SFB_LONG_44100);
        let band = 12;
        let range = SFB_LONG_44100[band]..SFB_LONG_44100[band + 1];

        let mut noise = [0.0f32; GRANULE_LINES];
        let mut seed = 1u32;
        for x in &mut noise[range.clone()] {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *x = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
        }
        let mut tone = [0.0f32; GRANULE_LINES];
        tone[range.start + 5] = p.band_energies(&noise)[band].sqrt();

        assert!(p.tonality(&noise)[band] < 0.3);
        assert!(p.tonality(&tone)[band] > 0.9);
        let (t_noise, t_tone) = (p.thresholds(&noise)[band], p.thresholds(&tone)[band]);
        assert!(t_tone < t_noise / 10.0, "tone {t_tone} vs noise {t_noise}");
    }
}
