# Roadmap

Updated: 2026-11-11

features:
- encoder: The Layer III encoder: MPEG-1 and MPEG-2 rates, mono and joint stereo, CBR and VBR, the Info tag, CRC-16; pure Rust, no unsafe (SPEC M1, M2)
- provenance: Recorded, licence-clean sources for the Huffman tables and the borrowed constants (items 1 to 3; SPEC M3, M4)
- docs: Accurate docs: MPEG-1 and MPEG-2 wording, what the Info-tag duration counts, the to_bytes panic (items 4 to 6; SPEC M5)
- api: The public API: non_exhaustive enums, chunked encode_i16 and encode_planar (items 7 and 8; SPEC M6, M7)
- wav2mp3: The wav2mp3 example and its input checks (items 9 to 11; SPEC M8)
- ci: CI on Mac, PC and Linux, the test suite, and no dead code (items 12 and 13; SPEC M9, M10)
- release: Releases: version, tag, and WavCraft and SoundCraft on the tag (item 14; SPEC M11)

The list for craft-mp3's first release, from the independent review of the encoder PR
(2026-10-16). The spec for 1.0 is [SPEC.md](SPEC.md); each item below has an issue.
The encoder itself passed the review: no bitstream bugs, and ffmpeg and symphonia
agree on every stream checked.

Where it stands (2026-11-11): 1.0.0 is released. Every item below is built and merged
(PRs #12 to #22, 2026-11-02 to 2026-11-04), issues #2 to #11 are closed, the version is
1.0.0 and the tag `v1.0.0` exists. One issue is open: #16, remove the duplicate test of
the 2.038 s Info-tag duration (PRs #14 and #15 each added one). Nothing is planned for
After 1.0 yet.

## Before the first release (all done)

1. *(#4)* **Huffman table provenance.** `src/huffman_tables.rs` matches symphonia's
   `codebooks.rs` value for value (symphonia is MPL-2.0; the crate is MIT OR Apache-2.0
   and carries no notice). Either regenerate the tables from the standard (ISO 11172-3
   Table B.7) or from minimp3 (CC0) and record the source in the file header, or keep
   the file and add the MPL-2.0 header and notice, saying so in the README.
2. *(done: no `Claude-Session:` trailer remains on this repo's main, checked 2026-10-26)* **Purge the internal session links from public history.** All 22 commits on the PR
   branch carry a `Claude-Session:` trailer that links to an internal session. Rewrite
   history to drop the trailer (`git filter-repo --message-callback`), force-push, and
   re-open or retarget the PR. Do this once, before the merge, and tell anyone with a
   clone to re-clone. The same trailers are in the history of soundcraft (31 of our
   commits), filmcraft (6 of ours) and AnyPS5 (94 of ours); the other forks only inherit
   them from upstream.
3. *(LAME credits done; window source: #7)* **Source comments for borrowed constants.** `SUBDIVISION` (`src/quantize.rs`) is
   LAME's `subdv_table` and `cutoff_hz` (`src/encoder.rs`) is LAME's lowpass table.
   Say where they come from. The 512-entry window in `src/tables.rs` is only described
   as "ISO Table B.3"; confirm its source.

## Docs and API (all done)

4. *(#6, #7)* Say what the Info tag's duration really is: it counts the 1057-sample delay plus the
   padding up to whole frames (a 2.000 s input reports 2.0376 s), so "exact duration" is
   misleading in `lib.rs` and `encoder.rs`.
5. *(#5, #6)* Update the stale MPEG-1-only text: `lib.rs`, `encoder.rs`, the `header.rs` module doc
   (version row), the `PaddingSchedule` doc and the `tables.rs` header.
6. *(#5)* Document the panic in `FrameHeader::to_bytes` for a bitrate that is not valid for the
   sample rate (for example 320 kbit/s at 22.05 kHz).
7. *(#8)* Mark the public enums (`ConfigError`, `HeaderError`, `ChannelMode`, `SampleRate`)
   `#[non_exhaustive]` before 1.0.
8. *(#9)* `encode_i16` and `encode_planar` copy the whole input; chunk them to halve peak memory
   on huge buffers.

## The `wav2mp3` example (all done)

9. *(#10)* A `data` chunk of size 0 followed by real data (a recorder that crashed) gives a
   one-frame silent file and exit code 0. Warn or fail.
10. *(#10)* Check the WAVE_FORMAT_EXTENSIBLE subformat GUID, so a float file with 16-bit width is
    not accepted as PCM.
11. *(#10)* Give 8000, 11025, 12000 and 96000 Hz input a clear message instead of
    `UnsupportedSampleRate(8000)`, and warn when both a bitrate and `--vbr` are given.

## Also for 1.0 (from the spec) (all done)

12. *(#2)* Delete the orphaned `src/bitwriter.rs` (never compiled; the encoder uses the
    crate-private `BitWriter` in `src/bitstream.rs`).
13. *(#3)* Run CI on macOS and Windows as well as Linux (SPEC M9).
14. *(#11)* Release 1.0.0: version, tag `v1.0.0`, WavCraft and SoundCraft on the tag.

## Done in this review

- Denormal-range input no longer makes encoding 60 to 100 times slower (flushed to
  silence below 1e-18).
- `wav2mp3` no longer panics on a WAV whose `fmt ` chunk is shorter than 16 bytes.
