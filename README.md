# craft-mp3

Pure-Rust MP3 encoder shared by WavCraft and SoundCraft. No C dependencies,
no `unsafe`.

```rust
use craft_mp3::{Encoder, EncoderConfig};

let mut encoder = Encoder::new(EncoderConfig::new(44_100, 2, 128))?;
let mut mp3 = Vec::new();
encoder.encode(&interleaved_f32, &mut mp3); // or encode_i16; call as often as you like
if let Some(tag) = encoder.finish(&mut mp3) {
    tag.write_to(&mut mp3); // exact duration and seek table for players
}
```

To convert a WAV file:

```text
cargo run --release --example wav2mp3 -- input.wav output.mp3 [kbps] [--crc]
```

## What it does

- MPEG-1 Layer III at 32, 44.1 and 48 kHz; mono or joint stereo; every
  constant bitrate from 32 to 320 kbit/s.
- The standard's polyphase filterbank and MDCT, with short blocks for
  transients (attack detection with lookahead, so no extra delay).
- A psychoacoustic model: threshold of hearing, Bark-scale masking, and a
  masking offset that depends on how tonal each band is.
- Noise shaping with scale factors and subblock gains, a bit reservoir that
  saves bits on easy frames for hard ones, per-frame mid/side stereo, and a
  bitrate-dependent lowpass.
- A Xing "Info" tag with frame count and seek table, and optional CRC-16
  frame protection.

About 29× realtime for stereo on one core.

## Delay and gapless playback

Decoded audio lags the input by `ENCODER_DELAY` (1057) samples, and the end
is padded with silence to a whole frame. The Info tag carries no gapless
fields: decoders only honour LAME's delay/padding extension when the tag
names LAME or libavcodec as the encoder. Players that need sample-exact
gapless playback can trim `ENCODER_DELAY` samples from the start and keep
the input length.

## Quality

Measured on synthetic test material at 128 kbit/s, by the share of
critical bands whose quantization noise exceeds an estimated masking
threshold (lower is better):

| material              | craft-mp3 | LAME  | shine |
|-----------------------|----------:|------:|------:|
| band (notes + drums)  |      5.2% |  7.1% | 41.8% |
| tonal solo            |      0.1% |  0.1% | 34.4% |
| stereo ambience       |      8.2% |  4.4% | 43.0% |
| clicks in silence     |     24.3% | 18.6% | 93.6% |

That is a rough objective measure, not a listening test.

## Testing

`cargo test` runs unit tests for every stage and round-trip tests that
decode the output with an independent decoder (symphonia) and check it
against the input: every sample rate, channel count and bitrate, pre-echo
around transients, CRC streams and the Info tag. CI runs fmt, clippy and the
tests on every pull request.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
