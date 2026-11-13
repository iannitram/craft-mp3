# Benchmarks

What this measures, how to run it, and what the numbers can and cannot say.
The harness is `examples/bench.rs` (metrics in `examples/bench/metrics.rs`).
Issue #30.

## How to run

```text
cargo run --release --example bench                    # Markdown table, 60 s clips, median of 5
cargo run --release --example bench -- --json          # the same as JSON
cargo run --release --example bench -- --seconds 2 --runs 1 --no-reference   # quick smoke run
cargo run --release --example bench -- --corpus DIR    # also encode the 16-bit PCM WAVs in DIR
```

`--corpus` files are only read, never copied or committed; each gets CBR 128
(CBR 64 below 32 kHz) and VBR 2 at its own rate and channel count.
Run one benchmark at a time and nothing else heavy on the machine: the encode
is one thread, so the time is wall time on one core.

## What is measured

**Corpus** (generated in code, so no downloads and no licence): 60 s each of
pink noise, a logarithmic sine sweep from 20 Hz to 20 kHz (capped at 95 % of
Nyquist at 22.05 kHz), a sum of seven tones with 0.5 s of silence in every
2 s, and an impulse train (a decaying click every 150 ms over faint noise;
transient-heavy, so it exercises short blocks). Stereo channels differ.

**Settings:** at 44.1 kHz stereo CBR 128, CBR 320, VBR 0, VBR 2, VBR 5; also
48 kHz stereo CBR 192 and 22.05 kHz mono CBR 64.

**Speed:** seconds of wall time to encode 60 s of input (`s per min`; scaled
if `--seconds` differs), median of the runs, and the realtime factor
(`60 / s per min`). craft-mp3 is timed around `encode_i16` plus `finish`
inside the process (no file I/O). `lame` and `ffmpeg` are timed as whole
processes (start-up, reading the WAV and writing the MP3 included), so they
carry a little overhead craft-mp3 does not; ffmpeg runs with `-threads 1`.
They run as external programs found on `PATH` (`lame -b N` or `-V N`;
`ffmpeg -c:a libmp3lame -b:a Nk` or `-q:a N`). If one is not installed its rows
say "not found"; the harness never fails for that. No code from either is
used, and CI does not install them.

**Quality**, every output decoded in-process by symphonia (the same decoder
for every encoder), aligned and trimmed to the input length. The alignment
uses the stated delay (craft-mp3: 1057; LAME and ffmpeg: the LAME-tag delay
plus the decoder's 529) and then picks the best cross-correlation lag within
±16 samples of it (the lag found is in the JSON as `delay`). The measures are
simple proxies, not PEAQ (ITU-R BS.1387), and not a listening test:

| Column | Meaning |
|---|---|
| kbit/s | The file's actual average bitrate. Compare VBR rows only at similar bitrates. |
| seg SNR dB | Mean over 20 ms frames of signal-to-error ratio in dB, each frame capped at 100 dB; frames of the input below -60 dBFS are skipped. Higher is better. Identical signals give 100. |
| NMR dB | Noise-to-mask proxy: error energy over craft-mp3's own psychoacoustic masking threshold, per scale-factor band (long blocks) and granule, averaged in dB. Lower is better; below 0 means the error is under the threshold. **This is craft-mp3's model judging**: it is the model the encoder aims at, so it favours craft-mp3 by construction. It compares encoders with one yardstick; it does not say what a listener hears. |
| LSD dB | Log-spectral distance: per 2048-point frame the RMS difference in dB between the two power spectra below craft-mp3's lowpass for that setting (the same band for every encoder), averaged. Lower is better; 0 when identical. |
| bandwidth kHz | Effective bandwidth: upper edge of the highest of 40 equal bands in which the decoded power is within 60 dB of the input's. |

Stereo files are scored per channel and averaged.

**Limits.** Segmental SNR rewards waveform match, which MP3 deliberately does
not aim for (it spends bits only where noise is audible), so a lower SNR can
sound as good or better. Noise-like and sweep signals say little about real
music; the corpus is synthetic on purpose. A lowpass that LAME chooses
differently from craft-mp3 shows up in LSD and bandwidth rather than as a
fault. Treat differences of a few tenths of a dB as noise. Timings are for
the machine described below only.

## Results

First measurement, 2026-11-12, release build, one thread, 60 s clips, median
of 5 runs. Machine: Apple M1 (Apple Silicon Mac), macOS aarch64, rustc 1.99.1.
Reference binaries: LAME 4.0 (`lame --version`), ffmpeg 9.0.2 with libmp3lame.
Windows and Linux runs are still to do (they need a run on those machines;
paste each table below with its CPU model and `rustc --version`).

Command: `cargo run --release --example bench`

| file | setting | encoder | s per min | x realtime | kbit/s | seg SNR dB | NMR dB | LSD dB | bandwidth kHz |
|---|---|---|---|---|---|---|---|---|---|
| pink noise | CBR 128 44.1 kHz stereo | craft-mp3 | 0.878 | 68.3 | 128.1 | 7.9 | -1.3 | 14.8 | 17.1 |
| pink noise | CBR 128 44.1 kHz stereo | lame | 0.563 | 106.5 | 128.1 | 10.1 | -6.6 | 8.7 | 17.6 |
| pink noise | CBR 128 44.1 kHz stereo | ffmpeg | 0.577 | 103.9 | 128.1 | 10.1 | -6.6 | 8.7 | 17.6 |
| sine sweep | CBR 128 44.1 kHz stereo | craft-mp3 | 0.454 | 132.2 | 128.1 | 63.6 | -34.4 | 3.1 | 17.6 |
| sine sweep | CBR 128 44.1 kHz stereo | lame | 0.360 | 166.8 | 128.1 | 25.3 | -31.5 | 1.2 | 17.6 |
| sine sweep | CBR 128 44.1 kHz stereo | ffmpeg | 0.370 | 162.0 | 128.1 | 25.3 | -31.5 | 1.2 | 17.6 |
| tones and gaps | CBR 128 44.1 kHz stereo | craft-mp3 | 0.802 | 74.8 | 128.1 | 32.6 | -50.7 | 6.5 | 22.1 |
| tones and gaps | CBR 128 44.1 kHz stereo | lame | 0.591 | 101.5 | 128.1 | 25.8 | -49.2 | 6.9 | 22.1 |
| tones and gaps | CBR 128 44.1 kHz stereo | ffmpeg | 0.599 | 100.2 | 128.1 | 25.8 | -49.2 | 6.9 | 22.1 |
| impulse train | CBR 128 44.1 kHz stereo | craft-mp3 | 0.610 | 98.3 | 128.1 | 2.1 | -3.1 | 7.3 | 17.6 |
| impulse train | CBR 128 44.1 kHz stereo | lame | 0.748 | 80.2 | 128.1 | 0.9 | -8.6 | 10.0 | 17.6 |
| impulse train | CBR 128 44.1 kHz stereo | ffmpeg | 0.764 | 78.5 | 128.1 | 0.9 | -8.6 | 10.0 | 17.6 |
| pink noise | CBR 320 44.1 kHz stereo | craft-mp3 | 0.907 | 66.2 | 320.3 | 17.6 | -14.8 | 4.8 | 21.5 |
| pink noise | CBR 320 44.1 kHz stereo | lame | 0.723 | 82.9 | 320.3 | 17.7 | -18.6 | 7.5 | 20.9 |
| pink noise | CBR 320 44.1 kHz stereo | ffmpeg | 0.737 | 81.4 | 320.3 | 17.7 | -18.6 | 7.5 | 20.9 |
| sine sweep | CBR 320 44.1 kHz stereo | craft-mp3 | 0.461 | 130.1 | 320.3 | 65.7 | -34.2 | 2.8 | 20.9 |
| sine sweep | CBR 320 44.1 kHz stereo | lame | 0.437 | 137.2 | 320.3 | 80.1 | -39.8 | 1.0 | 20.9 |
| sine sweep | CBR 320 44.1 kHz stereo | ffmpeg | 0.446 | 134.6 | 320.3 | 80.1 | -39.8 | 1.0 | 20.9 |
| tones and gaps | CBR 320 44.1 kHz stereo | craft-mp3 | 0.793 | 75.6 | 320.3 | 56.6 | -62.3 | 2.7 | 22.1 |
| tones and gaps | CBR 320 44.1 kHz stereo | lame | 0.518 | 115.8 | 320.3 | 66.5 | -67.5 | 1.8 | 22.1 |
| tones and gaps | CBR 320 44.1 kHz stereo | ffmpeg | 0.528 | 113.6 | 320.3 | 66.5 | -67.5 | 1.8 | 22.1 |
| impulse train | CBR 320 44.1 kHz stereo | craft-mp3 | 0.881 | 68.1 | 320.3 | 6.1 | -13.8 | 3.9 | 21.5 |
| impulse train | CBR 320 44.1 kHz stereo | lame | 0.839 | 71.5 | 320.3 | 6.0 | -20.7 | 6.1 | 20.9 |
| impulse train | CBR 320 44.1 kHz stereo | ffmpeg | 0.855 | 70.1 | 320.3 | 6.0 | -20.7 | 6.1 | 20.9 |
| pink noise | VBR 0 44.1 kHz stereo | craft-mp3 | 0.937 | 64.0 | 319.9 | 16.5 | -14.8 | 4.4 | 20.4 |
| pink noise | VBR 0 44.1 kHz stereo | lame | 0.600 | 99.9 | 242.0 | 15.2 | -15.2 | 4.1 | 22.1 |
| pink noise | VBR 0 44.1 kHz stereo | ffmpeg | 0.612 | 98.1 | 242.0 | 15.2 | -15.2 | 4.1 | 22.1 |
| sine sweep | VBR 0 44.1 kHz stereo | craft-mp3 | 0.423 | 141.7 | 37.8 | 54.6 | -31.5 | 3.8 | 20.4 |
| sine sweep | VBR 0 44.1 kHz stereo | lame | 0.352 | 170.4 | 97.1 | 79.2 | -41.5 | 1.0 | 20.9 |
| sine sweep | VBR 0 44.1 kHz stereo | ffmpeg | 0.362 | 166.0 | 97.1 | 79.2 | -41.5 | 1.0 | 20.9 |
| tones and gaps | VBR 0 44.1 kHz stereo | craft-mp3 | 0.618 | 97.1 | 100.1 | 32.5 | -51.1 | 6.1 | 22.1 |
| tones and gaps | VBR 0 44.1 kHz stereo | lame | 0.417 | 143.8 | 211.1 | 62.2 | -64.6 | 2.0 | 22.1 |
| tones and gaps | VBR 0 44.1 kHz stereo | ffmpeg | 0.434 | 138.4 | 211.1 | 62.2 | -64.6 | 2.0 | 22.1 |
| impulse train | VBR 0 44.1 kHz stereo | craft-mp3 | 0.862 | 69.6 | 319.6 | 5.5 | -14.6 | 3.5 | 20.4 |
| impulse train | VBR 0 44.1 kHz stereo | lame | 0.606 | 99.0 | 286.2 | 13.2 | -18.3 | 2.6 | 22.1 |
| impulse train | VBR 0 44.1 kHz stereo | ffmpeg | 0.621 | 96.6 | 286.2 | 13.2 | -18.3 | 2.6 | 22.1 |
| pink noise | VBR 2 44.1 kHz stereo | craft-mp3 | 0.932 | 64.4 | 319.9 | 15.5 | -14.9 | 4.8 | 19.3 |
| pink noise | VBR 2 44.1 kHz stereo | lame | 0.536 | 111.9 | 179.3 | 12.5 | -11.2 | 4.4 | 19.8 |
| pink noise | VBR 2 44.1 kHz stereo | ffmpeg | 0.547 | 109.6 | 179.2 | 12.5 | -11.2 | 4.4 | 19.8 |
| sine sweep | VBR 2 44.1 kHz stereo | craft-mp3 | 0.460 | 130.4 | 32.3 | 52.1 | -31.0 | 4.2 | 19.3 |
| sine sweep | VBR 2 44.1 kHz stereo | lame | 0.301 | 199.7 | 52.1 | 70.4 | -35.8 | 1.5 | 19.8 |
| sine sweep | VBR 2 44.1 kHz stereo | ffmpeg | 0.313 | 191.6 | 52.1 | 70.4 | -35.8 | 1.5 | 19.8 |
| tones and gaps | VBR 2 44.1 kHz stereo | craft-mp3 | 0.612 | 98.0 | 88.1 | 28.6 | -49.5 | 6.9 | 22.1 |
| tones and gaps | VBR 2 44.1 kHz stereo | lame | 0.414 | 144.9 | 141.9 | 48.0 | -57.6 | 4.8 | 22.1 |
| tones and gaps | VBR 2 44.1 kHz stereo | ffmpeg | 0.425 | 141.3 | 141.9 | 48.0 | -57.6 | 4.8 | 22.1 |
| impulse train | VBR 2 44.1 kHz stereo | craft-mp3 | 0.830 | 72.3 | 311.6 | 4.6 | -14.4 | 3.6 | 19.3 |
| impulse train | VBR 2 44.1 kHz stereo | lame | 0.534 | 112.4 | 210.0 | 3.0 | -14.5 | 3.3 | 19.8 |
| impulse train | VBR 2 44.1 kHz stereo | ffmpeg | 0.548 | 109.5 | 210.0 | 3.0 | -14.5 | 3.3 | 19.8 |
| pink noise | VBR 5 44.1 kHz stereo | craft-mp3 | 0.863 | 69.5 | 177.7 | 11.0 | -5.9 | 14.6 | 17.1 |
| pink noise | VBR 5 44.1 kHz stereo | lame | 0.467 | 128.6 | 118.8 | 9.2 | -6.8 | 12.7 | 17.6 |
| pink noise | VBR 5 44.1 kHz stereo | ffmpeg | 0.481 | 124.8 | 118.8 | 9.2 | -6.8 | 12.7 | 17.6 |
| sine sweep | VBR 5 44.1 kHz stereo | craft-mp3 | 0.457 | 131.2 | 31.9 | 48.8 | -31.3 | 4.9 | 17.6 |
| sine sweep | VBR 5 44.1 kHz stereo | lame | 0.278 | 215.9 | 32.0 | 39.0 | -30.1 | 4.1 | 17.6 |
| sine sweep | VBR 5 44.1 kHz stereo | ffmpeg | 0.291 | 206.1 | 32.0 | 39.0 | -30.1 | 4.1 | 17.6 |
| tones and gaps | VBR 5 44.1 kHz stereo | craft-mp3 | 0.614 | 97.7 | 73.5 | 23.6 | -47.5 | 8.2 | 22.1 |
| tones and gaps | VBR 5 44.1 kHz stereo | lame | 0.381 | 157.6 | 84.1 | 31.8 | -49.8 | 8.2 | 22.1 |
| tones and gaps | VBR 5 44.1 kHz stereo | ffmpeg | 0.395 | 151.9 | 84.1 | 31.8 | -49.8 | 8.2 | 22.1 |
| impulse train | VBR 5 44.1 kHz stereo | craft-mp3 | 0.592 | 101.4 | 178.4 | 3.0 | -7.7 | 5.4 | 18.2 |
| impulse train | VBR 5 44.1 kHz stereo | lame | 0.468 | 128.1 | 139.2 | 0.8 | -9.9 | 9.3 | 17.6 |
| impulse train | VBR 5 44.1 kHz stereo | ffmpeg | 0.485 | 123.7 | 139.2 | 0.8 | -9.9 | 9.3 | 17.6 |
| pink noise | CBR 192 48 kHz stereo | craft-mp3 | 1.061 | 56.6 | 192.2 | 10.8 | -5.9 | 14.1 | 19.2 |
| pink noise | CBR 192 48 kHz stereo | lame | 0.718 | 83.5 | 192.2 | 12.5 | -10.6 | 3.8 | 19.8 |
| pink noise | CBR 192 48 kHz stereo | ffmpeg | 0.720 | 83.3 | 192.2 | 12.5 | -10.6 | 3.8 | 19.8 |
| sine sweep | CBR 192 48 kHz stereo | craft-mp3 | 0.492 | 121.9 | 192.2 | 65.8 | -35.1 | 2.9 | 19.2 |
| sine sweep | CBR 192 48 kHz stereo | lame | 0.412 | 145.8 | 192.2 | 30.1 | -32.9 | 1.1 | 19.8 |
| sine sweep | CBR 192 48 kHz stereo | ffmpeg | 0.422 | 142.0 | 192.2 | 30.1 | -32.9 | 1.1 | 19.8 |
| tones and gaps | CBR 192 48 kHz stereo | craft-mp3 | 0.842 | 71.2 | 192.2 | 48.6 | -57.1 | 6.3 | 23.1 |
| tones and gaps | CBR 192 48 kHz stereo | lame | 0.485 | 123.8 | 192.2 | 30.4 | -50.9 | 4.4 | 23.1 |
| tones and gaps | CBR 192 48 kHz stereo | ffmpeg | 0.494 | 121.4 | 192.2 | 30.4 | -50.9 | 4.4 | 23.1 |
| impulse train | CBR 192 48 kHz stereo | craft-mp3 | 1.027 | 58.4 | 192.2 | 2.0 | -7.8 | 9.2 | 19.8 |
| impulse train | CBR 192 48 kHz stereo | lame | 0.761 | 78.8 | 192.2 | 2.3 | -12.5 | 3.5 | 19.8 |
| impulse train | CBR 192 48 kHz stereo | ffmpeg | 0.777 | 77.2 | 192.2 | 2.3 | -12.5 | 3.5 | 19.8 |
| pink noise | CBR 64 22.05 kHz mono | craft-mp3 | 0.257 | 233.3 | 64.1 | 17.3 | -9.5 | 3.0 | 11.0 |
| pink noise | CBR 64 22.05 kHz mono | lame | 0.143 | 418.6 | 64.1 | 15.7 | -10.0 | 19.8 | 10.5 |
| pink noise | CBR 64 22.05 kHz mono | ffmpeg | 0.165 | 363.8 | 64.1 | 15.7 | -10.0 | 19.8 | 10.5 |
| sine sweep | CBR 64 22.05 kHz mono | craft-mp3 | 0.139 | 431.8 | 64.1 | 62.0 | -31.4 | 3.5 | 11.0 |
| sine sweep | CBR 64 22.05 kHz mono | lame | 0.124 | 482.2 | 64.1 | 25.8 | -30.5 | 1.2 | 11.0 |
| sine sweep | CBR 64 22.05 kHz mono | ffmpeg | 0.145 | 413.7 | 64.1 | 25.8 | -30.5 | 1.2 | 11.0 |
| tones and gaps | CBR 64 22.05 kHz mono | craft-mp3 | 0.214 | 280.7 | 64.1 | 56.1 | -59.3 | 4.4 | 11.0 |
| tones and gaps | CBR 64 22.05 kHz mono | lame | 0.152 | 394.7 | 64.1 | 26.0 | -52.8 | 3.4 | 11.0 |
| tones and gaps | CBR 64 22.05 kHz mono | ffmpeg | 0.173 | 347.5 | 64.1 | 26.0 | -52.8 | 3.4 | 11.0 |
| impulse train | CBR 64 22.05 kHz mono | craft-mp3 | 0.226 | 265.6 | 64.1 | 8.0 | -5.2 | 4.6 | 11.0 |
| impulse train | CBR 64 22.05 kHz mono | lame | 0.166 | 361.1 | 64.1 | 5.9 | -9.6 | 17.2 | 10.5 |
| impulse train | CBR 64 22.05 kHz mono | ffmpeg | 0.190 | 315.3 | 64.1 | 5.9 | -9.6 | 17.2 | 10.5 |

### Reading the first numbers

- **Speed.** craft-mp3 runs at about 57 to 430 times realtime depending on
  the signal and setting (about 65 to 70 times for stereo 44.1 kHz noise at
  CBR 128 to 320), well above the README's old "about 25×". LAME is
  1.2 to 1.8 times faster on most rows (the LAME rows include process
  start-up, so the true gap is slightly larger). The impulse train at CBR 128 is the
  one row where craft-mp3 is faster.
- **lame and ffmpeg rows match** in every quality column because ffmpeg calls
  the same libmp3lame; only their times differ.
- **VBR is not comparable yet.** craft-mp3's VBR bitrates differ widely from
  LAME's at the same quality number (pink noise at VBR 0 to 2 hits the 320
  kbit/s ceiling; the sweep sits at 32 to 38 kbit/s against LAME's 52 to 97).
  Compare quality only where the kbit/s columns are close: all CBR rows.
- **CBR quality is mixed.** On the sweep and tones, craft-mp3 has a much
  higher segmental SNR at CBR 128, 192 and 64 (a pure tone is cheap to code
  exactly), but at CBR 320 LAME is ahead. On pink noise and the impulse train
  LAME has a lower NMR by 4 to 8 dB at CBR 128 and 192, and a much lower LSD
  on noise at CBR 128 and 192 (8.7 against 14.8 dB at CBR 128): craft-mp3
  zeroes more of the upper spectrum there. NMR is our own model, so it
  should be read with the caveat above.
- LSD on the 22.05 kHz mono rows for LAME (17 to 20 dB) probably looks high because
  LAME's lowpass there sits below the common band; treat it as a limit of
  the single-cutoff definition, not a fault.
