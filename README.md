# craft-mp3

Pure-Rust MP3 encoder shared by WavCraft and SoundCraft. No C dependencies,
no `unsafe`.

```rust
use craft_mp3::{Encoder, EncoderConfig};

let mut encoder = Encoder::new(EncoderConfig::new(44_100, 2, 128))?; // or ::vbr(44_100, 2, 4)
let mut mp3 = Vec::new();
encoder.encode(&interleaved_f32, &mut mp3); // or encode_i16; call as often as you like
if let Some(tag) = encoder.finish(&mut mp3) {
    tag.write_to(&mut mp3); // exact duration and seek table for players
}
```

To convert a WAV file:

```text
cargo run --release --example wav2mp3 -- input.wav output.mp3 [kbps | --vbr Q] [--crc]
```

## What it does

- Layer III at 32, 44.1 and 48 kHz (MPEG-1) and 16, 22.05 and 24 kHz
  (MPEG-2); mono or joint stereo.
- Constant bitrate (32 to 320 kbit/s, or 8 to 160 at the MPEG-2 rates) or
  variable bitrate at quality 0 (best) to 9 (smallest), on a scale close to
  LAME's V0..V9.
- The standard's polyphase filterbank and MDCT, with short blocks for
  transients (attack detection with lookahead, so no extra delay).
- A psychoacoustic model: threshold of hearing, Bark-scale masking, and a
  masking offset that depends on how tonal each band is.
- Noise shaping with scale factors and subblock gains, a bit reservoir that
  saves bits on easy frames for hard ones, per-frame mid/side stereo, and a
  bitrate-dependent lowpass.
- A Xing/Info tag with frame count and seek table (built from the real
  frame offsets for VBR), and optional CRC-16 frame protection.

About 25× realtime for stereo on one core.

## Delay and gapless playback

Decoded audio lags the input by `ENCODER_DELAY` (1057) samples, and the end
is padded with silence to a whole frame. The Info tag carries no gapless
fields: decoders only honour LAME's delay/padding extension when the tag
names LAME or libavcodec as the encoder. Players that need sample-exact
gapless playback can trim `ENCODER_DELAY` samples from the start and keep
the input length.

## Quality

Measured on synthetic test material by the share of critical bands whose
quantization noise exceeds an estimated masking threshold (lower is
better):

| material              | kbit/s | craft-mp3 |  LAME | shine |
|-----------------------|-------:|----------:|------:|------:|
| band (notes + drums)  |    128 |      2.8% |  7.1% | 41.8% |
| stereo ambience       |    128 |      4.3% |  4.4% | 43.0% |
| tonal solo            |    128 |      0.0% |  0.1% | 34.4% |
| clicks in silence     |    128 |     14.8% | 18.6% | 93.6% |
| band (notes + drums)  |     64 |     25.9% | 24.7% | 50.6% |
| stereo ambience       |     64 |     30.6% | 30.3% | 75.6% |
| tonal solo            |     64 |      6.8% | 10.9% | 37.5% |
| clicks in silence     |     64 |     23.7% | 28.1% | 96.1% |

Variable bitrate against LAME's matching V setting, on the band mix:

| quality | craft-mp3 kbit/s | audible | LAME kbit/s | audible |
|--------:|-----------------:|--------:|------------:|--------:|
|       0 |              192 |    1.2% |         231 |    0.1% |
|       2 |              160 |    1.5% |         166 |    2.2% |
|       5 |              116 |    3.8% |         118 |   12.9% |
|       9 |               60 |   25.9% |          71 |   23.7% |

Noise-like material costs craft-mp3 more than LAME at the high-quality end
(stereo ambience at quality 0: 312 against 232 kbit/s).

That is a rough objective measure, not a listening test, and its masking
estimate resembles the encoder's own model, so it likely flatters
craft-mp3. Read it as "in LAME's league on this material", not "better than
LAME".

## Testing

`cargo test` runs unit tests for every stage and round-trip tests that
decode the output with an independent decoder (symphonia) and check it
against the input: every sample rate, channel count and bitrate, pre-echo
around transients, CRC streams and the Info tag. CI runs fmt, clippy and the
tests on every pull request.

## License

Licensed under either of Apache License 2.0 or MIT, at your option.
