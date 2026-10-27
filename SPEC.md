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
