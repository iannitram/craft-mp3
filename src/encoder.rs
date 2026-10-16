//! The public encoder: PCM in, MPEG-1 Layer III frames out.

use std::fmt;

use crate::bitstream::BitWriter;
use crate::filterbank::{AnalysisFilterbank, SUBBANDS};
use crate::header::{
    Bitrate, ChannelMode, FrameHeader, PaddingSchedule, SAMPLES_PER_FRAME, SampleRate,
};
use crate::mdct::{GRANULE_LINES, Mdct, SLOTS};
use crate::quantize::{self, Granule, ScaleFactors, SfbBounds};
use crate::reservoir::Reservoir;
use crate::tables::{SFB_LONG_32000, SFB_LONG_44100, SFB_LONG_48000};

const GRANULES: usize = 2;

/// Encoder settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    /// 32000, 44100 or 48000.
    pub sample_rate: u32,
    /// 1 or 2.
    pub channels: u16,
    /// One of 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256 or 320.
    pub bitrate_kbps: u32,
}

/// Why an [`EncoderConfig`] was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    UnsupportedSampleRate(u32),
    UnsupportedChannels(u16),
    UnsupportedBitrate(u32),
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
        }
    }
}

impl std::error::Error for ConfigError {}

/// Per-channel analysis state.
#[derive(Clone, Default)]
struct ChannelState {
    filterbank: AnalysisFilterbank,
    mdct: Mdct,
    /// Input samples waiting for a full frame.
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
    sfb: &'static SfbBounds,
    channels: Vec<ChannelState>,
    reservoir: Reservoir,
    /// Channel the next interleaved input sample belongs to.
    next_channel: usize,
}

impl Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, ConfigError> {
        let sample_rate = SampleRate::from_hz(config.sample_rate)
            .ok_or(ConfigError::UnsupportedSampleRate(config.sample_rate))?;
        let bitrate = Bitrate::from_kbps(config.bitrate_kbps)
            .ok_or(ConfigError::UnsupportedBitrate(config.bitrate_kbps))?;
        let mode = match config.channels {
            1 => ChannelMode::Mono,
            2 => ChannelMode::Stereo,
            n => return Err(ConfigError::UnsupportedChannels(n)),
        };
        let sfb = match sample_rate {
            SampleRate::Hz32000 => &SFB_LONG_32000,
            SampleRate::Hz44100 => &SFB_LONG_44100,
            SampleRate::Hz48000 => &SFB_LONG_48000,
        };

        Ok(Self {
            template: FrameHeader::new(bitrate, sample_rate, mode),
            padding: PaddingSchedule::new(bitrate, sample_rate),
            sfb,
            channels: vec![ChannelState::default(); usize::from(config.channels)],
            reservoir: Reservoir::default(),
            next_channel: 0,
        })
    }

    /// Encodes interleaved samples, nominally in `[-1, 1]`, appending any
    /// completed frames to `out`. Calls may split the input anywhere, even
    /// between the channels of one sample.
    pub fn encode(&mut self, samples: &[f32], out: &mut Vec<u8>) {
        let n = self.channels.len();
        for &sample in samples {
            let ch = self.next_channel;
            self.channels[ch].pending.push(sample);
            self.next_channel = (ch + 1) % n;
            if ch == n - 1 && self.channels[ch].pending.len() == SAMPLES_PER_FRAME {
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
    pub fn finish(mut self, out: &mut Vec<u8>) {
        // Complete a partial set of channels with silence.
        while self.next_channel != 0 {
            self.encode(&[0.0], out);
        }
        // Buffered samples plus the filterbank delay must all reach a frame.
        let frames = (self.channels[0].pending.len() + ENCODER_DELAY).div_ceil(SAMPLES_PER_FRAME);
        for _ in 0..frames {
            for ch in &mut self.channels {
                ch.pending.resize(SAMPLES_PER_FRAME, 0.0);
            }
            self.encode_frame(out);
        }
        self.reservoir.drain_all(out);
    }

    fn encode_frame(&mut self, out: &mut Vec<u8>) {
        let channels = self.channels.len();

        let mut xr = vec![[[0.0f32; GRANULE_LINES]; GRANULES]; channels];
        for (ch, state) in self.channels.iter_mut().enumerate() {
            for (gr, xr) in xr[ch].iter_mut().enumerate() {
                let mut slots = [[0.0f32; SUBBANDS]; SLOTS];
                for (t, slot) in slots.iter_mut().enumerate() {
                    let start = (gr * SLOTS + t) * SUBBANDS;
                    let input = state.pending[start..start + SUBBANDS].try_into().unwrap();
                    state.filterbank.process(input, slot);
                }
                state.mdct.process(&slots, xr);
            }
            state.pending.clear();
        }

        let header = FrameHeader {
            padding: self.padding.next_padding(),
            ..self.template
        };
        let slot_len = header.main_data_len();
        let main_data_begin = self.reservoir.main_data_begin();

        // Share the frame's bits, reservoir included, evenly over what is
        // left; savings in one granule carry to the next.
        let units = GRANULES * channels;
        let mut remaining = 8 * (main_data_begin + slot_len) as u32;
        let mut granules = Vec::with_capacity(units);
        for (unit, (gr, ch)) in (0..GRANULES)
            .flat_map(|gr| (0..channels).map(move |ch| (gr, ch)))
            .enumerate()
        {
            let budget = remaining / (units - unit) as u32;
            let granule =
                quantize::rate_loop(&xr[ch][gr], ScaleFactors::default(), self.sfb, budget);
            remaining -= granule.part2_3_length();
            granules.push(granule);
        }

        let mut main_data = BitWriter::with_capacity(main_data_begin + slot_len);
        for granule in &granules {
            granule.write_main_data(&mut main_data, self.sfb);
        }
        main_data.align_to_byte();

        let mut head = BitWriter::with_capacity(header.frame_len() - slot_len);
        header.write(&mut head);
        write_side_info(&mut head, main_data_begin, channels, &granules);
        debug_assert_eq!(head.as_bytes().len(), header.frame_len() - slot_len);

        self.reservoir
            .push(head.as_bytes(), slot_len, main_data.as_bytes());
        self.reservoir.drain_finished(out);
    }
}

/// Samples by which decoded output lags the input: the analysis and
/// synthesis filterbanks plus one granule of MDCT overlap.
pub const ENCODER_DELAY: usize = 481 + GRANULE_LINES;

/// Writes MPEG-1 side information for long blocks (2.4.1.7).
/// `granules` is in bitstream order: granule 0's channels, then granule 1's.
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
        w.write_bit(false); // window_switching_flag: long blocks only
        for &table in &g.table_select {
            w.write_bits(table as u32, 5);
        }
        w.write_bits(g.region0_count as u32, 4);
        w.write_bits(g.region1_count as u32, 3);
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
            sample_rate,
            channels,
            bitrate_kbps,
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
        enc.finish(&mut out);

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
