//! Pure-Rust MP3 (MPEG-1 Layer III) encoder shared by WavCraft and SoundCraft.
//!
//! The encoder is built up module by module, each with its own tests.

mod attack;
pub mod bitstream;
mod encoder;
pub mod filterbank;
pub mod header;
mod huffman;
mod huffman_tables;
mod layout;
pub mod mdct;
mod psycho;
mod quantize;
mod reservoir;
mod tables;

pub use encoder::{ConfigError, ENCODER_DELAY, Encoder, EncoderConfig};
