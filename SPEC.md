# craft-mp3: spec

**Approval:** 2026-10-26, the owner, in the Project Manager session's question tool: "ROADMAP's first release (Recommended)". The spec is the README's "What it does" plus [ROADMAP.md](ROADMAP.md)'s "Before the first release" and "Docs and API" (and the `wav2mp3` items that go with them). A changed Must or Won't sets this back to "not yet".

**1.0 means:** everything on that list is done, version 1.0.0 is tagged `v1.0.0`, and WavCraft and SoundCraft use that tag as a git dependency for MP3 export. It is not published on crates.io.

## Musts

| # | Must | Done looks like | Check | Issue |
|---|---|---|---|---|
| M1 | THE ENCODER SHALL do what README "What it does" lists: Layer III at 32/44.1/48 kHz (MPEG-1) and 16/22.05/24 kHz (MPEG-2), mono or joint stereo, CBR and VBR quality 0-9, a Xing/Info tag with seek table, optional CRC-16. | Every combination decodes cleanly in an independent decoder. | `cargo test` (round-trip tests decode with symphonia). | already built |
| M2 | THE CRATE SHALL stay pure Rust with no C dependencies and no `unsafe` (README). | `#![forbid(unsafe_code)]` in `lib.rs`; no build scripts or `-sys` crates. | `cargo tree` shows no runtime dependencies; the build passes with the forbid. | already true |
| M3 | THE HUFFMAN TABLES SHALL have a recorded, licence-clean source (ROADMAP 1). | `huffman_tables.rs` names its source; a generator or test rebuilds the same values. | The table test; no MPL-2.0 text unless its notice is added. | #4 |
| M4 | BORROWED CONSTANTS SHALL say where they come from (ROADMAP 3). | `SUBDIVISION` and the lowpass table credit LAME (done); the window table names its published source. | Read the doc comments. | #7 |
| M5 | THE DOCS SHALL be accurate: MPEG-1 and MPEG-2 everywhere, the Info-tag duration explained (it includes the 1057-sample delay and frame padding), and the `FrameHeader::to_bytes` panic documented (ROADMAP 4-6). | No doc says "MPEG-1 only" or "exact duration". | `cargo doc --no-deps` with no warnings; grep. | #5, #6, #7 |
| M6 | THE PUBLIC ENUMS (`ConfigError`, `HeaderError`, `ChannelMode`, `SampleRate`) SHALL be `#[non_exhaustive]` (ROADMAP 7). | Adding a variant later is not a breaking change. | `cargo test`, clippy. | #8 |
| M7 | `encode_i16` AND `encode_planar` SHALL encode in chunks instead of copying the whole input (ROADMAP 8). | Byte-identical output, no allocation proportional to the input. | Equality tests against `encode`. | #9 |
| M8 | THE `wav2mp3` EXAMPLE SHALL warn on a zero-size `data` chunk, check the EXTENSIBLE subformat GUID, and explain unsupported sample rates (ROADMAP 9-11). | The three bad inputs give the messages in the issue. | Run the example on each case. | #10 |
| M9 | CI SHALL run fmt, clippy and tests on macOS, Windows and Linux (pipeline R04). | Three green `check` runs per PR, plus the MSRV run. | GitHub Actions on a PR. | #3 |
| M10 | THE CRATE SHALL carry no dead code from merges. | The orphaned `src/bitwriter.rs` is gone. | `cargo build`; the file is absent. | #2 |
| M11 | THE 1.0 RELEASE SHALL be version 1.0.0, tagged `v1.0.0`, and used by WavCraft and SoundCraft as a git dependency on that tag. | Both apps build their MP3 export against the tag. | `Cargo.toml` and the tag here; each app's dependency line. | #11 |

Done before this spec: ROADMAP item 2 (purge the internal session trailers from this repo's history): no `Claude-Session:` trailer remains on `main` (checked 2026-10-26).

## Won'ts

- No crates.io release (`publish = false`).
- No C dependencies and no `unsafe`.
- No decoder: decoding in tests uses symphonia as a dev-dependency only.
- No MPEG-2.5 (8, 11.025, 12 kHz) or rates above 48 kHz: `wav2mp3` explains them instead (M8).
- No Layer I or Layer II.

## Platforms (R04)

A pure Rust library with no platform code: it must build and pass its tests on macOS, Windows and Linux (M9). Its users run on all three.

## Testing decisions

Round-trip tests decode every stream with an independent decoder (symphonia) and compare against the input; equality tests prove refactors give byte-identical output; stress tests cover long and edge-case input (`tests/stress.rs`). The README's quality table is an objective estimate, not a listening test.

## Prior art

Peers checked on 2026-11-12. Licences were read from each project's own repository (see [docs/peers.md](docs/peers.md) for the feature table and the sources). "Learn" means no code is copied from it, only ideas; the rule is that code is ported only from a peer under MIT OR Apache-2.0 compatible terms.

- **LAME**, LGPL-2.0 (GitHub mirror): the reference MP3 encoder; the feature bar and the source of the Info-tag layout and the borrowed constants.
- **Shine**, LGPL-2.0 per its COPYING file (some package metadata says GPL-2.0): a small fixed-point encoder with a bitrate-only API (no VBR in its header), 16-bit input.
- **ffmpeg**, LGPL-2.1 or later, with optional GPL parts: a decoder and muxer; it encodes MP3 only through wrappers and platform APIs: libmp3lame (LAME), libshine (Shine) and `mp3_mf` (Windows Media Foundation). It has no native MP3 encoder.
- **Helix MP3 encoder** (RealNetworks), RPSL/RCSL terms, one copy at maikmerten/hmp3: RealNetworks' encoder, licensed under RPSL/RCSL; learn only. Its features were not checked (see docs/peers.md).
- **mp3lame-encoder**, LGPL-3.0: a Rust wrapper over LAME.
- **mp3lame-sys**, LGPL-3.0: the low-level Rust binding that builds LAME.
- **shine-rs**, LGPL-2.0: a pure-Rust port of Shine's encoder.
- **rusty_mp3**, Apache-2.0: a pure-Rust MP3 decoder and encoder (MPEG-1, 2 and 2.5, CBR and VBR) in the remade_ffmpeg_rs repository; uses SIMD with scalar fallbacks.
- **audiobox-rs**, MIT: a zero-dependency Rust audio library with its own MPEG-1 CBR encoder and a LAME-layout gapless tag, which the gapless work took its marker approach from.
- **minimp3**, CC0: a single-header MP3 decoder; the source of the Huffman tables' regeneration path (#4).
- **symphonia**, MPL-2.0: a pure-Rust decoder; the independent decoder used by the tests (dev-dependency only).

Dropped: the "ffmpeg native encoder" entry in the issue's list. ffmpeg has no native MP3 Layer III encoder; its `mpegaudioenc.c` is an MP2 encoder. The ffmpeg wrappers stay as the peer.
