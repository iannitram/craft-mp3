//! Pure-Rust MP3 (MPEG-1 Layer III) encoder shared by WavCraft and SoundCraft.
//!
//! ```
//! use craft_mp3::{Encoder, EncoderConfig};
//!
//! // One second of a 440 Hz tone, interleaved stereo.
//! let pcm: Vec<f32> = (0..44_100)
//!     .flat_map(|n| {
//!         let s = (n as f32 * 440.0 * std::f32::consts::TAU / 44_100.0).sin() * 0.5;
//!         [s, s]
//!     })
//!     .collect();
//!
//! let mut encoder = Encoder::new(EncoderConfig::new(44_100, 2, 128))?;
//! let mut mp3 = Vec::new();
//! encoder.encode(&pcm, &mut mp3);
//! if let Some(tag) = encoder.finish(&mut mp3) {
//!     // Give players the exact duration.
//!     tag.write_to(&mut mp3);
//! }
//! assert_eq!(&mp3[..2], [0xFF, 0xFB]);
//! # Ok::<(), craft_mp3::ConfigError>(())
//! ```
//!
//! Supports 32, 44.1 and 48 kHz, mono or joint stereo, every MPEG-1 Layer
//! III bitrate, and variable bitrate by quality ([`EncoderConfig::vbr`]).
//! Inside: the standard's polyphase
//! filterbank and MDCT with window switching for transients, a
//! psychoacoustic model with tonality-dependent masking, scale-factor noise
//! shaping, a bit reservoir and per-frame mid/side stereo.

#![forbid(unsafe_code)]

mod attack;
mod bitstream;
mod encoder;
mod filterbank;
pub mod header;
mod huffman;
mod huffman_tables;
mod info;
mod layout;
mod mdct;
mod psycho;
mod quantize;
mod reservoir;
mod tables;

pub use encoder::{ConfigError, ENCODER_DELAY, Encoder, EncoderConfig};
pub use info::InfoTag;
