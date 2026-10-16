//! The public encoder: PCM in, MPEG-1 Layer III frames out.

use std::f32::consts::FRAC_1_SQRT_2;
use std::fmt;

use crate::attack::AttackDetector;
use crate::bitstream::BitWriter;
use crate::filterbank::{AnalysisFilterbank, SUBBANDS};
use crate::header::{
    Bitrate, ChannelMode, FrameHeader, PaddingSchedule, SAMPLES_PER_FRAME, SampleRate, frame_crc,
};
use crate::info::{self, InfoTag};
use crate::layout::Layout;
use crate::mdct::{BlockType, GRANULE_LINES, Mdct, SLOTS};
use crate::psycho::Psycho;
use crate::quantize::{self, Granule};
use crate::reservoir::{MAX_MAIN_DATA_BEGIN, Reservoir};

const GRANULES: usize = 2;

/// Encoder settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// 32000, 44100 or 48000.
    pub sample_rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// One of 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256 or 320:
    /// the bitrate of every frame, or with [`vbr_quality`](Self::vbr_quality)
    /// the most any frame may use.
    pub bitrate_kbps: u32,
    /// Encode at a variable bitrate: each frame gets the bits its content
    /// needs at this quality, from 0 (best, about 245 kbit/s for stereo
    /// music) to 9 (smallest, about 65 kbit/s).
    pub vbr_quality: Option<u8>,
    /// Start the stream with a Xing "Info" tag frame giving players the
    /// exact duration and a seek table; see [`Encoder::finish`].
    pub info_tag: bool,
    /// Protect each frame's header and side information with a CRC-16, so
    /// decoders can detect corruption. Costs 2 bytes per frame.
    pub crc: bool,
}

impl EncoderConfig {
    /// Constant-bitrate settings with the Info tag enabled.
    pub fn new(sample_rate: u32, channels: u16, bitrate_kbps: u32) -> Self {
        Self {
            sample_rate,
            channels,
            bitrate_kbps,
            vbr_quality: None,
            info_tag: true,
            crc: false,
        }
    }

    /// Variable-bitrate settings at `quality` (0 best, 9 smallest) with the
    /// Info tag enabled.
    pub fn vbr(sample_rate: u32, channels: u16, quality: u8) -> Self {
        Self {
            vbr_quality: Some(quality),
            ..Self::new(sample_rate, channels, 320)
        }
    }
}

/// Why an [`EncoderConfig`] was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    UnsupportedSampleRate(u32),
    UnsupportedChannels(u16),
    UnsupportedBitrate(u32),
    UnsupportedVbrQuality(u8),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSampleRate(hz) => {
                write!(
                    f,
                    "unsupported sample rate {hz} Hz (expected 32000, 44100 or 48000)"
                )
            }
            Self::UnsupportedChannels(n) => {
                write!(f, "unsupported channel count {n} (expected 1 or 2)")
            }
            Self::UnsupportedBitrate(kbps) => write!(f, "unsupported bitrate {kbps} kbit/s"),
            Self::UnsupportedVbrQuality(q) => {
                write!(f, "unsupported VBR quality {q} (expected 0 to 9)")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Input samples beyond the current frame needed to decide its block types:
/// the attack detector must see the granule after the frame.
const LOOKAHEAD: usize = 576;

/// Largest input magnitude accepted; larger values are clipped.
const MAX_SAMPLE: f32 = 1024.0;

/// Typical stereo bitrate for each VBR quality, which sets its lowpass.
const VBR_TYPICAL_KBPS: [u32; 10] = [245, 225, 190, 175, 165, 130, 115, 100, 85, 65];

/// Noise targets for VBR quality 0, relative to the psychoacoustic model's
/// thresholds, and how far each quality step relaxes them, in dB. Tuned so
/// the qualities land near LAME's V0..V9 bitrates on typical material.
const VBR_BEST_DB: f32 = -13.0;
const VBR_STEP_DB: f32 = 1.8;

/// How much the bit pressure moves per doubling of demand over supply, and
/// its ceiling, in dB.
const PRESSURE_GAIN_DB: f32 = 6.0;
const MAX_PRESSURE_DB: f32 = 40.0;

/// Per-channel analysis state.
#[derive(Clone, Default)]
struct ChannelState {
    filterbank: AnalysisFilterbank,
    mdct: Mdct,
    attacks: AttackDetector,
    /// Input samples waiting for a full frame plus lookahead.
    pending: Vec<f32>,
}

/// A streaming MP3 encoder.
///
/// Feed interleaved PCM with [`Encoder::encode`] (or
/// [`Encoder::encode_i16`]) and call [`Encoder::finish`] at the end; each
/// call appends whole MP3 frames to the output.
pub struct Encoder {
    template: FrameHeader,
    padding: PaddingSchedule,
    layout: Layout,
    channels: Vec<ChannelState>,
    reservoir: Reservoir,
    psycho: Psycho,
    /// Per-line gain applied before quantization, for long and short
    /// blocks: 1 in the passband, 0 above the bitrate's cutoff.
    lowpass_long: Box<[f32; GRANULE_LINES]>,
    lowpass_short: Box<[f32; GRANULE_LINES]>,
    /// Block type of the last granule encoded.
    last_block: BlockType,
    /// For VBR, how far the noise targets are relaxed, in dB.
    vbr_relax_db: Option<f32>,
    info_tag: bool,
    /// Length of the tag frame, once written.
    tag_len: usize,
    /// Length of every audio frame so far, for the tag.
    frame_lengths: Vec<u32>,
    /// How far, in dB, recent frames have had to relax their noise targets
    /// to fit the bitrate; see [`Psycho::thresholds`].
    pressure_db: f32,
    /// Channel the next interleaved input sample belongs to.
    next_channel: usize,
}

impl Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, ConfigError> {
        let sample_rate = SampleRate::from_hz(config.sample_rate)
            .ok_or(ConfigError::UnsupportedSampleRate(config.sample_rate))?;
        let bitrate = Bitrate::from_kbps(config.bitrate_kbps)
            .ok_or(ConfigError::UnsupportedBitrate(config.bitrate_kbps))?;
        if let Some(q) = config.vbr_quality.filter(|&q| q > 9) {
            return Err(ConfigError::UnsupportedVbrQuality(q));
        }
        let mode = match config.channels {
            1 => ChannelMode::Mono,
            2 => ChannelMode::JointStereo {
                mid_side: false,
                intensity: false,
            },
            n => return Err(ConfigError::UnsupportedChannels(n)),
        };
        let layout = Layout::new(sample_rate.hz());
        let kbps_per_channel = match config.vbr_quality {
            // The VBR table is for stereo; mono gets half.
            Some(q) => VBR_TYPICAL_KBPS[usize::from(q)] / 2,
            None => config.bitrate_kbps / u32::from(config.channels),
        };

        Ok(Self {
            template: FrameHeader {
                crc: config.crc,
                ..FrameHeader::new(bitrate, sample_rate, mode)
            },
            padding: PaddingSchedule::new(bitrate, sample_rate),
            psycho: Psycho::new(&layout),
            lowpass_long: lowpass(&layout, BlockType::Normal, kbps_per_channel),
            lowpass_short: lowpass(&layout, BlockType::Short, kbps_per_channel),
            layout,
            channels: vec![ChannelState::default(); usize::from(config.channels)],
            reservoir: Reservoir::default(),
            last_block: BlockType::Normal,
            vbr_relax_db: config
                .vbr_quality
                .map(|q| VBR_BEST_DB + f32::from(q) * VBR_STEP_DB),
            info_tag: config.info_tag,
            tag_len: 0,
            frame_lengths: Vec::new(),
            pressure_db: 0.0,
            next_channel: 0,
        })
    }

    /// Encodes interleaved samples, nominally in `[-1, 1]`, appending any
    /// completed frames to `out`. Calls may split the input anywhere, even
    /// between the channels of one sample.
    ///
    /// NaN is treated as silence and magnitudes beyond 1024 (infinities
    /// included) are clipped, so no input can break the stream.
    pub fn encode(&mut self, samples: &[f32], out: &mut Vec<u8>) {
        let n = self.channels.len();
        for &sample in samples {
            let sample = if sample.is_nan() {
                0.0
            } else {
                sample.clamp(-MAX_SAMPLE, MAX_SAMPLE)
            };
            let ch = self.next_channel;
            let state = &mut self.channels[ch];
            state.pending.push(sample);
            state.attacks.push(sample);
            self.next_channel = (ch + 1) % n;
            if ch == n - 1 && state.pending.len() == SAMPLES_PER_FRAME + LOOKAHEAD {
                self.encode_frame(out);
            }
        }
    }

    /// Encodes interleaved 16-bit samples; see [`Encoder::encode`].
    pub fn encode_i16(&mut self, samples: &[i16], out: &mut Vec<u8>) {
        let scaled: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
        self.encode(&scaled, out);
    }

    /// Pads the input with silence so every sample reaches a frame, and
    /// writes the remaining frames.
    ///
    /// With [`EncoderConfig::info_tag`], the stream began with a placeholder
    /// tag frame and this returns the finished one: write it over the start
    /// of the stream ([`InfoTag::write_to`]) so players get the exact
    /// duration. The stream plays correctly either way.
    pub fn finish(mut self, out: &mut Vec<u8>) -> Option<InfoTag> {
        // Complete a partial set of channels with silence.
        while self.next_channel != 0 {
            self.encode(&[0.0], out);
        }
        // Buffered samples plus the filterbank delay must all reach a frame,
        // and the last frame needs its lookahead too.
        let buffered = self.channels[0].pending.len();
        let frames = (buffered + ENCODER_DELAY).div_ceil(SAMPLES_PER_FRAME);
        let silence = frames * SAMPLES_PER_FRAME + LOOKAHEAD - buffered;
        let zeros = vec![0.0; silence * self.channels.len()];
        self.encode(&zeros, out);
        self.reservoir.drain_all(out);
        self.info_tag.then(|| {
            let counts = info::Counts::from_frame_lengths(self.tag_len, &self.frame_lengths);
            info::tag_frame(
                &self.tag_template(),
                self.vbr_relax_db.is_some(),
                Some(&counts),
            )
        })
    }

    /// The header the tag frame is sized from: for VBR the smallest frame,
    /// so the tag costs as little as possible.
    fn tag_template(&self) -> FrameHeader {
        match self.vbr_relax_db {
            Some(_) => FrameHeader {
                bitrate: Bitrate::all().next().unwrap(),
                ..self.template
            },
            None => self.template,
        }
    }

    /// Block types for this frame's two granules, the same for every
    /// channel (mid/side coding needs matching windows).
    fn block_types(&mut self) -> [BlockType; GRANULES] {
        let attack = |g: usize| {
            self.channels.iter().any(|c| {
                c.attacks
                    .attack(g)
                    .expect("lookahead covers the next granule")
            })
        };
        let (a0, a1, a2) = (attack(0), attack(1), attack(2));
        let first = BlockType::next(self.last_block, a0, a1);
        let second = BlockType::next(first, a1, a2);
        self.last_block = second;
        for state in &mut self.channels {
            state.attacks.pop();
            state.attacks.pop();
        }
        [first, second]
    }

    fn encode_frame(&mut self, out: &mut Vec<u8>) {
        if self.info_tag && self.tag_len == 0 {
            let placeholder =
                info::tag_frame(&self.tag_template(), self.vbr_relax_db.is_some(), None);
            out.extend_from_slice(placeholder.as_bytes());
            self.tag_len = placeholder.as_bytes().len();
        }
        let channels = self.channels.len();
        let blocks = self.block_types();

        let mut xr = vec![[[0.0f32; GRANULE_LINES]; GRANULES]; channels];
        for (ch, state) in self.channels.iter_mut().enumerate() {
            for (gr, xr) in xr[ch].iter_mut().enumerate() {
                let mut slots = [[0.0f32; SUBBANDS]; SLOTS];
                for (t, slot) in slots.iter_mut().enumerate() {
                    let start = (gr * SLOTS + t) * SUBBANDS;
                    let input = state.pending[start..start + SUBBANDS].try_into().unwrap();
                    state.filterbank.process(input, slot);
                }
                state.mdct.process(&slots, blocks[gr], xr);
                let weights = if blocks[gr] == BlockType::Short {
                    *xr = self.layout.short_to_bitstream(xr);
                    &self.lowpass_short
                } else {
                    &self.lowpass_long
                };
                for (x, w) in xr.iter_mut().zip(weights.iter()) {
                    *x *= w;
                }
            }
            state.pending.drain(..SAMPLES_PER_FRAME);
        }

        // Allowed noise per partition, from the left/right signals: that is
        // where a listener hears the noise, whichever way it was coded.
        let mut allowed: Vec<[Vec<f32>; GRANULES]> = xr
            .iter()
            .map(|granules| {
                std::array::from_fn(|gr| {
                    let mut thr = self.psycho.thresholds(
                        &self.layout,
                        &granules[gr],
                        blocks[gr],
                        self.pressure_db,
                    );
                    if let Some(db) = self.vbr_relax_db {
                        let factor = 10f32.powf(db / 10.0);
                        thr.iter_mut().for_each(|t| *t *= factor);
                    }
                    thr
                })
            })
            .collect();

        let mid_side = channels == 2 && prefers_mid_side(&xr[0], &xr[1]);
        if mid_side {
            // Noise in M and S reaches each output channel as (nM + nS) / 2,
            // so holding both under the quieter channel's threshold keeps
            // both outputs clean.
            let (left, right) = allowed.split_at_mut(1);
            for (l, r) in left[0].iter_mut().zip(right[0].iter_mut()) {
                for (a, b) in l.iter_mut().zip(r.iter_mut()) {
                    let quieter = a.min(*b);
                    (*a, *b) = (quieter, quieter);
                }
            }
            let (left, right) = xr.split_at_mut(1);
            for (l, r) in left[0]
                .iter_mut()
                .flatten()
                .zip(right[0].iter_mut().flatten())
            {
                (*l, *r) = ((*l + *r) * FRAC_1_SQRT_2, (*l - *r) * FRAC_1_SQRT_2);
            }
        }

        // Bitstream order: granule 0's channels, then granule 1's.
        let order: Vec<(usize, usize)> = (0..GRANULES)
            .flat_map(|gr| (0..channels).map(move |ch| (gr, ch)))
            .collect();

        // What each granule needs to keep its noise inaudible.
        let demands: Vec<Granule> = order
            .iter()
            .map(|&(gr, ch)| {
                quantize::perceptual(&xr[ch][gr], blocks[gr], &allowed[ch][gr], &self.layout)
            })
            .collect();
        let demand: u32 = demands.iter().map(Granule::part2_3_length).sum();

        let mode = match self.template.mode {
            ChannelMode::Mono => ChannelMode::Mono,
            _ => ChannelMode::JointStereo {
                mid_side,
                intensity: false,
            },
        };
        let main_data_begin = self.reservoir.main_data_begin();
        let header = match self.vbr_relax_db {
            None => FrameHeader {
                padding: self.padding.next_padding(),
                mode,
                ..self.template
            },
            // The smallest frame that holds the demand, up to the maximum.
            Some(_) => Bitrate::all()
                .filter(|&b| b <= self.template.bitrate)
                .map(|bitrate| FrameHeader {
                    bitrate,
                    mode,
                    ..self.template
                })
                .find(|h| 8 * (main_data_begin + h.main_data_len()) as u32 >= demand)
                .unwrap_or(FrameHeader {
                    mode,
                    ..self.template
                }),
        };
        let slot_len = header.main_data_len();
        self.frame_lengths.push(header.frame_len() as u32);

        // Bits this frame can use, and how many it must use: whatever the
        // next frame can't borrow is lost.
        let available = 8 * (main_data_begin + slot_len) as u32;
        let must_use = available.saturating_sub(8 * MAX_MAIN_DATA_BEGIN as u32);

        // Steer the next frame's relaxation towards demand matching supply.
        let ratio = demand.max(1) as f32 / available.max(1) as f32;
        self.pressure_db =
            (self.pressure_db + PRESSURE_GAIN_DB * ratio.log2()).clamp(0.0, MAX_PRESSURE_DB);

        let granules: Vec<Granule> = if demand > available || demand < must_use {
            // Share `target` bits in proportion to demand, letting each
            // granule's leftovers flow to the ones after it.
            let target = if demand > available {
                available
            } else {
                must_use
            };
            let finer_allowed = demand < must_use;
            let mut remaining = target;
            let mut remaining_weight: u32 = demands.iter().map(|d| d.part2_3_length().max(1)).sum();
            order
                .iter()
                .zip(&demands)
                .map(|(&(gr, ch), d)| {
                    let weight = d.part2_3_length().max(1);
                    let budget = (u64::from(remaining) * u64::from(weight)
                        / u64::from(remaining_weight)) as u32;
                    let min_gain = if finer_allowed { 0 } else { d.global_gain };
                    let g = quantize::rate_loop(
                        &xr[ch][gr],
                        blocks[gr],
                        d.scalefac,
                        &self.layout,
                        budget,
                        min_gain,
                    );
                    remaining = remaining.saturating_sub(g.part2_3_length());
                    remaining_weight -= weight;
                    g
                })
                .collect()
        } else {
            demands
        };

        let mut main_data = BitWriter::with_capacity(main_data_begin + slot_len);
        for granule in &granules {
            granule.write_main_data(&mut main_data, &self.layout);
        }
        main_data.align_to_byte();

        let mut side_info = BitWriter::with_capacity(header.side_info_len());
        write_side_info(&mut side_info, main_data_begin, channels, &granules);
        let header_bytes = header.to_bytes();
        let mut head = Vec::with_capacity(header.frame_len() - slot_len);
        head.extend_from_slice(&header_bytes);
        if header.crc {
            head.extend_from_slice(&frame_crc(&header_bytes, side_info.as_bytes()).to_be_bytes());
        }
        head.extend_from_slice(side_info.as_bytes());
        debug_assert_eq!(head.len(), header.frame_len() - slot_len);

        self.reservoir.push(&head, slot_len, main_data.as_bytes());
        self.reservoir.drain_finished(out);
    }
}

/// Lowpass cutoff in Hz for a per-channel bitrate, following LAME's choices
/// for joint stereo (which spends about half the bitrate on each channel).
fn cutoff_hz(kbps_per_channel: u32) -> f32 {
    const TABLE: [(u32, f32); 16] = [
        (16, 5_500.0),
        (20, 7_000.0),
        (24, 7_500.0),
        (28, 10_000.0),
        (32, 11_000.0),
        (40, 13_500.0),
        (48, 15_100.0),
        (56, 15_600.0),
        (64, 17_000.0),
        (80, 17_500.0),
        (96, 18_600.0),
        (112, 19_400.0),
        (128, 19_700.0),
        (160, 20_500.0),
        (192, 21_000.0),
        (u32::MAX, 22_000.0),
    ];
    let upper = TABLE
        .iter()
        .position(|&(k, _)| kbps_per_channel <= k)
        .unwrap();
    if upper == 0 {
        return TABLE[0].1;
    }
    let ((k0, f0), (k1, f1)) = (TABLE[upper - 1], TABLE[upper]);
    if k1 == u32::MAX {
        return f1;
    }
    f0 + (f1 - f0) * (kbps_per_channel - k0) as f32 / (k1 - k0) as f32
}

/// Per-line weights, in bitstream order for `block`: a short linear taper
/// ending at the cutoff.
fn lowpass(layout: &Layout, block: BlockType, kbps_per_channel: u32) -> Box<[f32; GRANULE_LINES]> {
    let stop = cutoff_hz(kbps_per_channel) as f64;
    let start = stop * 0.94;
    let mut weights = Box::new([0.0; GRANULE_LINES]);
    for p in layout.partitions(block) {
        let lines = p.lines.len();
        for (i, line) in p.lines.clone().enumerate() {
            let f =
                p.lowest_hz + (p.highest_hz - p.lowest_hz) * i as f64 / (lines - 1).max(1) as f64;
            weights[line] = ((stop - f) / (stop - start)).clamp(0.0, 1.0) as f32;
        }
    }
    weights
}

/// Whether coding a frame as mid/side should take fewer bits than
/// left/right: for roughly Gaussian spectra the bits grow with the log of
/// each channel's energy, so compare the energy products.
fn prefers_mid_side(
    left: &[[f32; GRANULE_LINES]; GRANULES],
    right: &[[f32; GRANULE_LINES]; GRANULES],
) -> bool {
    let (mut ll, mut rr, mut mm, mut ss) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (&l, &r) in left.iter().flatten().zip(right.iter().flatten()) {
        let (l, r) = (f64::from(l), f64::from(r));
        ll += l * l;
        rr += r * r;
        mm += (l + r) * (l + r) / 2.0;
        ss += (l - r) * (l - r) / 2.0;
    }
    ll * rr > 0.0 && mm * ss < 0.5 * ll * rr
}

/// Samples by which decoded output lags the input: the analysis and
/// synthesis filterbanks plus one granule of MDCT overlap.
pub const ENCODER_DELAY: usize = 481 + GRANULE_LINES;

/// Writes MPEG-1 side information (2.4.1.7). `granules` is in bitstream
/// order: granule 0's channels, then granule 1's.
fn write_side_info(
    w: &mut BitWriter,
    main_data_begin: usize,
    channels: usize,
    granules: &[Granule],
) {
    w.write_bits(main_data_begin as u32, 9);
    w.write_bits(0, if channels == 1 { 5 } else { 3 }); // private bits
    for _ in 0..channels {
        w.write_bits(0, 4); // scfsi: scale factors are always sent
    }
    for g in granules {
        w.write_bits(g.part2_3_length(), 12);
        w.write_bits(g.big_values as u32, 9);
        w.write_bits(g.global_gain, 8);
        w.write_bits(g.scalefac.compress as u32, 4);
        let switched = g.block != BlockType::Normal;
        w.write_bit(switched); // window_switching_flag
        if switched {
            w.write_bits(g.block as u32, 2);
            w.write_bit(false); // mixed_block_flag
            for &table in &g.table_select[..2] {
                w.write_bits(table as u32, 5);
            }
            for &gain in &g.scalefac.subblock_gain {
                w.write_bits(u32::from(gain), 3);
            }
        } else {
            for &table in &g.table_select {
                w.write_bits(table as u32, 5);
            }
            w.write_bits(g.region0_count as u32, 4);
            w.write_bits(g.region1_count as u32, 3);
        }
        w.write_bit(g.scalefac.preflag);
        w.write_bit(g.scalefac.scale);
        w.write_bit(g.count1_table_b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(sample_rate: u32, channels: u16, bitrate_kbps: u32) -> EncoderConfig {
        EncoderConfig {
            info_tag: false,
            ..EncoderConfig::new(sample_rate, channels, bitrate_kbps)
        }
    }

    #[test]
    fn rejects_unsupported_configs() {
        assert_eq!(
            Encoder::new(config(22_050, 2, 128)).err(),
            Some(ConfigError::UnsupportedSampleRate(22_050))
        );
        assert_eq!(
            Encoder::new(config(44_100, 3, 128)).err(),
            Some(ConfigError::UnsupportedChannels(3))
        );
        assert_eq!(
            Encoder::new(config(44_100, 2, 100)).err(),
            Some(ConfigError::UnsupportedBitrate(100))
        );
    }

    #[test]
    fn output_is_a_sequence_of_valid_frames_at_the_exact_bitrate() {
        let mut enc = Encoder::new(config(44_100, 2, 128)).unwrap();
        let samples: Vec<f32> = (0..44_100 * 2)
            .map(|i| ((i / 2) as f32 * 0.05).sin() * 0.5)
            .collect();
        let mut out = Vec::new();
        enc.encode(&samples, &mut out);
        assert_eq!(enc.finish(&mut out), None);

        let mut pos = 0;
        let mut frames = 0;
        while pos < out.len() {
            let header = FrameHeader::parse(out[pos..pos + 4].try_into().unwrap()).unwrap();
            assert_eq!(header.bitrate.kbps(), 128);
            pos += header.frame_len();
            frames += 1;
        }
        assert_eq!(pos, out.len());
        // One second of audio plus the delay, rounded up to whole frames.
        assert_eq!(frames, (44_100 + ENCODER_DELAY).div_ceil(SAMPLES_PER_FRAME));
        let exact = frames as u64 * 144 * 128_000 / 44_100;
        assert_eq!(out.len() as u64, exact);
    }

    #[test]
    fn info_tag_counts_the_frames_and_bytes_that_follow_it() {
        let mut enc = Encoder::new(EncoderConfig::new(48_000, 1, 64)).unwrap();
        let mut out = Vec::new();
        enc.encode(&vec![0.1; 48_000], &mut out);
        let tag = enc.finish(&mut out).expect("tag enabled");
        let len = tag.as_bytes().len();
        assert!(
            out[..len].windows(4).any(|w| w == b"Info"),
            "placeholder is at the start"
        );
        tag.write_to(&mut out);

        let at = 4 + 17 + 4;
        let be = |i: usize| u32::from_be_bytes(out[at + i..at + i + 4].try_into().unwrap());
        let frames = (48_000 + ENCODER_DELAY).div_ceil(SAMPLES_PER_FRAME) as u32;
        assert_eq!(be(4), frames);
        assert_eq!(be(8) as usize, out.len());
        // 48 kHz frames are all the same length.
        assert_eq!(out.len() - len, frames as usize * 192);
    }

    #[test]
    fn mid_side_is_chosen_for_correlated_channels_only() {
        let mut seed = 5u32;
        let mut noise = || {
            let mut g = [[0.0f32; GRANULE_LINES]; GRANULES];
            for x in g.iter_mut().flatten() {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *x = (seed >> 8) as f32 / (1 << 24) as f32 - 0.5;
            }
            g
        };
        let (a, b) = (noise(), noise());
        let mut near_a = a;
        for (x, y) in near_a.iter_mut().flatten().zip(b.iter().flatten()) {
            *x += 0.1 * y;
        }
        assert!(prefers_mid_side(&a, &near_a));
        assert!(!prefers_mid_side(&a, &b));
        assert!(!prefers_mid_side(&a, &[[0.0; GRANULE_LINES]; GRANULES]));
    }

    #[test]
    fn lowpass_cutoff_rises_with_bitrate_and_zeroes_the_top() {
        let mut last = 0.0;
        for kbps in [16, 24, 32, 48, 64, 96, 128, 160, 320] {
            let f = cutoff_hz(kbps);
            assert!(f >= last, "{kbps}: {f} < {last}");
            last = f;
        }
        assert_eq!(cutoff_hz(64), 17_000.0);
        let layout = Layout::new(44_100);
        let w = lowpass(&layout, BlockType::Normal, 64);
        let line = |hz: f32| (hz / (44_100.0 / 2.0 / GRANULE_LINES as f32)) as usize;
        assert_eq!(w[line(10_000.0)], 1.0);
        assert_eq!(w[line(17_100.0)], 0.0);
        assert_eq!(w[GRANULE_LINES - 1], 0.0);
        // Short blocks cut at the same frequency, in each window.
        let short = lowpass(&layout, BlockType::Short, 64);
        for p in layout.partitions(BlockType::Short) {
            let expected = if p.highest_hz < 15_000.0 {
                1.0
            } else if p.lowest_hz > 17_000.0 {
                0.0
            } else {
                continue;
            };
            assert!(
                short[p.lines.clone()].iter().all(|&x| x == expected),
                "{p:?}"
            );
        }
    }

    #[test]
    fn output_does_not_depend_on_how_input_is_split() {
        let samples: Vec<f32> = (0..20_000)
            .map(|i| (i as f32 * 0.013).sin() * 0.4)
            .collect();
        let encode = |chunk: usize| {
            let mut enc = Encoder::new(config(48_000, 2, 192)).unwrap();
            let mut out = Vec::new();
            for part in samples.chunks(chunk) {
                enc.encode(part, &mut out);
            }
            enc.finish(&mut out);
            out
        };
        let whole = encode(samples.len());
        for chunk in [1, 3, 1151, 2304, 4097] {
            assert!(encode(chunk) == whole, "chunks of {chunk}");
        }
    }
}
