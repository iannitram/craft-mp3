//! Locked tests for issue #8: the four public enums are #[non_exhaustive].
//!
//! Each match below names every variant AND ends in `_`. Inside the crate
//! that `_` would be unreachable; outside it, only a non_exhaustive enum
//! makes it reachable. `deny(unreachable_patterns)` turns the missing
//! attribute into a compile error, so this file is red until all four
//! enums carry it.

#![deny(unreachable_patterns)]

use craft_mp3::ConfigError;
use craft_mp3::header::{ChannelMode, HeaderError, SampleRate};

fn config_error(e: ConfigError) -> u8 {
    match e {
        ConfigError::UnsupportedSampleRate(_) => 1,
        ConfigError::UnsupportedChannels(_) => 2,
        ConfigError::UnsupportedBitrate(_) => 3,
        ConfigError::UnsupportedVbrQuality(_) => 4,
        _ => 99,
    }
}

fn sample_rate(r: SampleRate) -> u8 {
    match r {
        SampleRate::Hz16000 => 1,
        SampleRate::Hz22050 => 2,
        SampleRate::Hz24000 => 3,
        SampleRate::Hz32000 => 4,
        SampleRate::Hz44100 => 5,
        SampleRate::Hz48000 => 6,
        _ => 99,
    }
}

fn channel_mode(m: ChannelMode) -> u8 {
    match m {
        ChannelMode::Stereo => 1,
        ChannelMode::JointStereo { .. } => 2,
        ChannelMode::DualChannel => 3,
        ChannelMode::Mono => 4,
        _ => 99,
    }
}

fn header_error(e: HeaderError) -> u8 {
    match e {
        HeaderError::NoSync => 1,
        HeaderError::UnsupportedVersion => 2,
        HeaderError::UnsupportedLayer => 3,
        HeaderError::UnsupportedBitrate => 4,
        HeaderError::ReservedSampleRate => 5,
        HeaderError::ReservedEmphasis => 6,
        _ => 99,
    }
}

#[test]
fn config_error_is_non_exhaustive() {
    assert_eq!(config_error(ConfigError::UnsupportedSampleRate(8000)), 1);
    assert_eq!(config_error(ConfigError::UnsupportedChannels(3)), 2);
    assert_eq!(config_error(ConfigError::UnsupportedBitrate(7)), 3);
    assert_eq!(config_error(ConfigError::UnsupportedVbrQuality(10)), 4);
}

#[test]
fn sample_rate_is_non_exhaustive() {
    assert_eq!(sample_rate(SampleRate::Hz16000), 1);
    assert_eq!(sample_rate(SampleRate::Hz22050), 2);
    assert_eq!(sample_rate(SampleRate::Hz24000), 3);
    assert_eq!(sample_rate(SampleRate::Hz32000), 4);
    assert_eq!(sample_rate(SampleRate::Hz44100), 5);
    assert_eq!(sample_rate(SampleRate::Hz48000), 6);
}

#[test]
fn channel_mode_is_non_exhaustive() {
    assert_eq!(channel_mode(ChannelMode::Stereo), 1);
    assert_eq!(
        channel_mode(ChannelMode::JointStereo {
            mid_side: true,
            intensity: false
        }),
        2
    );
    assert_eq!(channel_mode(ChannelMode::DualChannel), 3);
    assert_eq!(channel_mode(ChannelMode::Mono), 4);
}

#[test]
fn header_error_is_non_exhaustive() {
    assert_eq!(header_error(HeaderError::NoSync), 1);
    assert_eq!(header_error(HeaderError::UnsupportedVersion), 2);
    assert_eq!(header_error(HeaderError::UnsupportedLayer), 3);
    assert_eq!(header_error(HeaderError::UnsupportedBitrate), 4);
    assert_eq!(header_error(HeaderError::ReservedSampleRate), 5);
    assert_eq!(header_error(HeaderError::ReservedEmphasis), 6);
}
