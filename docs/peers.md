# Peer scan (2026-11-12)

Part of [issue #31](https://github.com/iannitram/craft-mp3/issues/31) (After 1.0, item A3). The peer list and the licences are in [SPEC.md `## Prior art`](../SPEC.md). The feature list comes from issue #31; I read the issue through a page summary, not the raw body, so check the list against the issue.

Key: **yes**, **no**, **partial** (some of it, or only by a workaround), **?** (not confirmed on a page I read). craft-mp3's column is read from this repo's code. Each cell cites a source number; the sources are listed at the end, and each one was fetched and returned HTTP 200 on 2026-11-12.

Peers in the table: LAME, Shine, ffmpeg (its LAME and Shine wrappers; its `mp3_mf` encoder is not covered), shine-rs, rusty_mp3 and audiobox-rs. mp3lame-encoder is a binding over LAME, so it has LAME's features and is not a separate column. Helix (one copy at maikmerten/hmp3) was not checked for features, so it has no column. minimp3 and symphonia are decoders and are not in the table.

## Feature table

| Feature | craft-mp3 | LAME | Shine | ffmpeg (libmp3lame / libshine) | shine-rs | rusty_mp3 | audiobox-rs |
|---|---|---|---|---|---|---|---|
| ABR mode | no [C1](../src/encoder.rs) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) | ? [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | yes [S9](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c) | ? [S12](https://github.com/wshon/shine-rs) | ? [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | no [S14](https://github.com/jmsansan/audiobox-rs) |
| Forced mid/side or dual channel | partial: mid/side chosen per frame [C1](../src/encoder.rs) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) [S2](https://github.com/lameproject/lame/blob/master/include/lame.h) | partial: mode enum has DUAL_CHANNEL [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | ? [S9](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c) | partial: Dual Channel listed [S12](https://github.com/wshon/shine-rs) | partial: mono, stereo, joint stereo listed [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | ? |
| Intensity stereo | no [C1](../src/encoder.rs) | no [S1](https://github.com/lameproject/lame/blob/master/USAGE) | ? [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | no [S9](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c) | ? [S12](https://github.com/wshon/shine-rs) | no [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | ? |
| Presets | no [C6](../examples/wav2mp3.rs) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) | ? | ? | ? | ? | ? |
| Free-format bitrates | no [C2](../src/header.rs) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) | ? [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | ? | ? | no: listed under "Not supported" [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | ? |
| MPEG-2.5 rates | no [C2](../src/header.rs) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) | yes [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | ? | yes [S12](https://github.com/wshon/shine-rs) | yes [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | no [S14](https://github.com/jmsansan/audiobox-rs) |
| 24-bit or float input | partial: f32 and i16 in the API, 16-bit WAV only in wav2mp3 [C1](../src/encoder.rs)[C6](../examples/wav2mp3.rs) | yes [S2](https://github.com/lameproject/lame/blob/master/include/lame.h) | no [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | yes [S9](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c) | ? | ? | ? |
| ReplayGain in tag | no [C3](../src/info.rs) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) [S2](https://github.com/lameproject/lame/blob/master/include/lame.h) | ? | no [S9](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c) | ? | ? | ? |
| ID3v1 tag | no (ID3v2 planned, #29 [S15](https://github.com/iannitram/craft-mp3/issues/29)) | yes [S1](https://github.com/lameproject/lame/blob/master/USAGE) | ? | ? | ? | ? | ? |
| Streaming writer / io::Write | partial: appends to a Vec, repeatable [C1](../src/encoder.rs) | partial: buffer-returning calls, no stream callback [S2](https://github.com/lameproject/lame/blob/master/include/lame.h) | partial: buffer per call [S5](https://github.com/toots/shine/blob/master/src/lib/layer3.h) | ? | partial: per-buffer calls then finish [S12](https://github.com/wshon/shine-rs) | ? | ? |
| no_std | no [C5](../src/lib.rs) | ? | ? | ? | ? | ? | ? |
| Multithreading | no [C5](../src/lib.rs) | ? [S2](https://github.com/lameproject/lame/blob/master/include/lame.h) | ? | ? | ? | ? | ? |
| SIMD | no [C5](../src/lib.rs) | yes: SSE file in the vector directory [S3](https://github.com/lameproject/lame/tree/master/libmp3lame/vector) | ? | ? | ? | yes: explicit SIMD kernels [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | ? |
| WASM | ? (not built in CI) [C7](../.github/workflows/ci.yml) | ? | ? | ? | ? | yes: README lists wasm32 targets [S13](https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md) | yes: compiles to WebAssembly [S14](https://github.com/jmsansan/audiobox-rs) |
| Encoder delay and lookahead | yes: ENCODER_DELAY 1057 and attack lookahead [C1](../src/encoder.rs)[C4](../src/attack.rs) | partial: delay yes [S2](https://github.com/lameproject/lame/blob/master/include/lame.h); lookahead not checked | ? | partial: reads LAME's delay [S9](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c) | ? | ? | partial: writes LAME-layout delay fields [S14](https://github.com/jmsansan/audiobox-rs) |
| Downmix / upmix | no: 1 or 2 channels only [C1](../src/encoder.rs) | ? | ? | ? | ? | ? | ? |
| Resampling of refused rates | no: 8000, 11025, 12000 and 96000 Hz refused [C6](../examples/wav2mp3.rs) | yes: --resample, lame_set_out_samplerate [S1](https://github.com/lameproject/lame/blob/master/USAGE) [S2](https://github.com/lameproject/lame/blob/master/include/lame.h) | ? | ? | ? | ? | ? |

Rows left as `?` are not wrong; I did not check them. The cells with the most evidence are LAME's, because I read its USAGE file and header.

Notes on specific cells:

- **Shine's rate list** comes from a comment in its header, not from a support statement. Shine's header also has no VBR or ABR setting and no mono/stereo list beyond the mode enum.
- **ffmpeg's cells** are its LAME or Shine wrapper's cells. ffmpeg also has a third MP3 encoder, `mp3_mf` (`ff_mp3_mf_encoder`, Windows Media Foundation, listed in [allcodecs.c](https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/allcodecs.c)); I did not check its features, so no column covers it. For the wrappers: ffmpeg's LAME wrapper calls only the functions listed in [S9]. I did not confirm ffmpeg's own forced-stereo or ABR options beyond `abr`.
- **rusty_mp3** cells come from its README (crate in the remade_ffmpeg_rs repo). The README's "Not supported" paragraph lists free-format bitrate streams (the decoder rejects them) and says the encoder does not produce intensity stereo or mixed blocks. Its README also lists wasm32 targets. It has explicit SIMD kernels, each with a safe scalar path; the crate therefore uses `unsafe`, which this crate forbids (M2).
- **audiobox-rs** cells come from its README: MPEG-1 CBR encoding, VBR and psychoacoustic features not ported, and a WebAssembly build. The LAME-layout gapless fields are its own tag, not LAME's.

## Gaps worth doing

Six gaps. Each has a section in `_proposed-issues.md` (untracked, not committed) with a verdict and acceptance criteria.

1. **ABR mode.** A target average bitrate between CBR and VBR. LAME (`--abr`) and ffmpeg's wrapper have it [S1][S9].
2. **Forced stereo mode.** A caller can force mid/side on or off, and choose dual channel. LAME has `-m f`, `-m d` and `lame_set_force_ms` [S1][S2]. craft-mp3 decides per frame only [C1].
3. **24-bit and float WAV input in `wav2mp3`.** The reader takes 16-bit PCM only [C6]; the library already takes f32 [C1].
4. **ReplayGain in the LAME-layout tag.** LAME writes the peak and gain fields and has `lame_set_findReplayGain` [S1][S2]; craft-mp3 does not record them [C3].
5. **A `std::io::Write` adapter.** Output is a caller-owned `Vec<u8>` [C1]; no peer's core API is a stream either [S2][S5][S12], so this is a small convenience, not a port.
6. **A wasm32 build check in CI.** audiobox-rs and rusty_mp3 claim WebAssembly builds [S14][S13]; craft-mp3's CI has no wasm job [C7].

## Not worth doing

- **Free-format bitrates** (LAME `--freeformat`) [S1]: non-standard rates that many players reject; the header refuses them on purpose [C2].
- **Presets** (LAME `--preset`) [S1]: a naming layer over the kbps and `--vbr` settings that wav2mp3 already has [C6].
- **Intensity stereo** [S1]: LAME does not do it either, rusty_mp3 says its encoder does not [S13], and it costs quality.
- **MPEG-2.5 rates** [S1][S5][S12][S13]: keep the SPEC Won't ("No MPEG-2.5 ... rates", owner decision 2026-11-12); 16 kHz and higher speech already works through MPEG-2.
- **Resampling inside the encoder** [S1][S2]: SPEC Won't covers rates above 48 kHz, and wav2mp3 already refuses 8000, 11025, 12000 and 96000 Hz with a message (M8) [C6].
- **Downmix and upmix** [C1]: the caller's job (owner decision 2026-11-12); WavCraft and SoundCraft already choose the channels before encoding.
- **ID3v1** [S1]: a fixed 128-byte tag superseded by ID3v2, which #29 covers [S15].
- **no_std** [C5]: the crate allocates Vecs and its users (WavCraft, SoundCraft) run on desktop hosts, so it would be a rework with no user.
- **Fixed-point arithmetic, as Shine does** [S5][S6]: aimed at CPUs without an FPU, which none of our targets is; #30 will measure the float path.

## Covered by #29 / #30

- **ID3v2.4 text frames and CHAP/CTOC chapters** (the ID3v2 row): issue #29 [S15]. Not re-filed.
- **Encode-time benchmark** against LAME and ffmpeg (reference binaries, run only when installed): issue #30 [S16].
- **Objective quality measures** (segmental SNR, noise-to-mask proxy, log-spectral distance, bandwidth) against LAME and ffmpeg: issue #30 [S16].
- **Multithreading and SIMD**: no gap filed here, because they depend on speed numbers #30 will produce first.

## Peer sources (weekly diff scan)

- LAME (a GitHub mirror, which its README calls "unmonitored"; the official site was not fetched): https://github.com/lameproject/lame
- Shine: https://github.com/toots/shine
- ffmpeg: https://github.com/FFmpeg/FFmpeg
- shine-rs: https://github.com/wshon/shine-rs
- remade_ffmpeg_rs (contains rusty_mp3): https://github.com/Remade-With-Rust/remade_ffmpeg_rs
- audiobox-rs: https://github.com/jmsansan/audiobox-rs
- mp3lame-encoder: https://github.com/DoumanAsh/mp3lame-encoder
- mp3lame-sys: https://github.com/DoumanAsh/mp3lame-sys
- Helix MP3 encoder (copy): https://github.com/maikmerten/hmp3
- minimp3: https://github.com/lieff/minimp3
- symphonia: https://github.com/pdeljanov/Symphonia

## Sources

Craft-mp3 (repo files, relative to this file):

- [C1] [src/encoder.rs](../src/encoder.rs)
- [C2] [src/header.rs](../src/header.rs)
- [C3] [src/info.rs](../src/info.rs)
- [C4] [src/attack.rs](../src/attack.rs)
- [C5] [src/lib.rs](../src/lib.rs)
- [C6] [examples/wav2mp3.rs](../examples/wav2mp3.rs)
- [C7] [.github/workflows/ci.yml](../.github/workflows/ci.yml)

Peers:

- [S1] LAME USAGE: https://github.com/lameproject/lame/blob/master/USAGE
- [S2] LAME header lame.h: https://github.com/lameproject/lame/blob/master/include/lame.h (read to line 1000 of 1345)
- [S3] LAME vector directory: https://github.com/lameproject/lame/tree/master/libmp3lame/vector
- [S5] Shine header layer3.h: https://github.com/toots/shine/blob/master/src/lib/layer3.h
- [S6] Shine README (repo front page): https://github.com/toots/shine
- Shine COPYING: https://github.com/toots/shine/blob/master/COPYING
- [S8] ffmpeg codec list: https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/allcodecs.c
- [S9] ffmpeg libmp3lame wrapper: https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libmp3lame.c
- ffmpeg libshine wrapper: https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/libshine.c
- ffmpeg mpegaudioenc.c (an MP2 encoder): https://github.com/FFmpeg/FFmpeg/blob/master/libavcodec/mpegaudioenc.c
- [S12] shine-rs: https://github.com/wshon/shine-rs
- [S13] rusty_mp3 README: https://github.com/Remade-With-Rust/remade_ffmpeg_rs/blob/main/crates/rusty_mp3/README.md
- rusty_mp3 directory: https://github.com/Remade-With-Rust/remade_ffmpeg_rs/tree/main/crates/rusty_mp3
- rusty_mp3 on docs.rs: https://docs.rs/rusty_mp3
- [S14] audiobox-rs: https://github.com/jmsansan/audiobox-rs
- mp3lame-encoder: https://github.com/DoumanAsh/mp3lame-encoder
- mp3lame-sys: https://github.com/DoumanAsh/mp3lame-sys
- Helix copy (hmp3): https://github.com/maikmerten/hmp3 and its LICENSE.txt: https://github.com/maikmerten/hmp3/blob/master/hmp3/LICENSE.txt
- minimp3: https://github.com/lieff/minimp3
- symphonia: https://github.com/pdeljanov/Symphonia

Issues:

- [S15] #29: https://github.com/iannitram/craft-mp3/issues/29
- [S16] #30: https://github.com/iannitram/craft-mp3/issues/30
- #31: https://github.com/iannitram/craft-mp3/issues/31

## Not confirmed

- Shine's VBR, mono, and float input: the header gives only a bitrate and int16 input, so those cells rest on the header alone.
- Helix's feature set: I read its licence file only.
- ffmpeg's own options beyond its LAME and Shine calls.
- Threads, SIMD, no_std and WASM for Shine, ffmpeg, LAME (beyond the vector directory), and shine-rs.
- rusty_mp3's ABR and resampling status: its README did not mention them in the text I read.
- ffmpeg's `mp3_mf` encoder features: only its listing in allcodecs.c was read.
- Shine's WebAssembly support: its README does not mention it, so no claim is made.
- LAME's lookahead.
