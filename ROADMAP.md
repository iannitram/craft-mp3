# Roadmap

Open items for craft-mp3, from the independent review of the encoder PR (2026-10-16).
The encoder itself passed the review: no bitstream bugs, and ffmpeg and symphonia
agree on every stream checked.

## Before the first release

1. **Huffman table provenance.** `src/huffman_tables.rs` matches symphonia's
   `codebooks.rs` value for value (symphonia is MPL-2.0; the crate is MIT OR Apache-2.0
   and carries no notice). Either regenerate the tables from the standard (ISO 11172-3
   Table B.7) or from minimp3 (CC0) and record the source in the file header, or keep
   the file and add the MPL-2.0 header and notice, saying so in the README.
2. **Purge the internal session links from public history.** All 22 commits on the PR
   branch carry a `Claude-Session:` trailer that links to an internal session. Rewrite
   history to drop the trailer (`git filter-repo --message-callback`), force-push, and
   re-open or retarget the PR. Do this once, before the merge, and tell anyone with a
   clone to re-clone. The same trailers are in the history of soundcraft (31 of our
   commits), filmcraft (6 of ours) and AnyPS5 (94 of ours); the other forks only inherit
   them from upstream.
3. **Source comments for borrowed constants.** `SUBDIVISION` (`src/quantize.rs`) is
   LAME's `subdv_table` and `cutoff_hz` (`src/encoder.rs`) is LAME's lowpass table.
   Say where they come from. The 512-entry window in `src/tables.rs` is only described
   as "ISO Table B.3"; confirm its source.

## Docs and API

4. Say what the Info tag's duration really is: it counts the 1057-sample delay plus the
   padding up to whole frames (a 2.000 s input reports 2.0376 s), so "exact duration" is
   misleading in `lib.rs` and `encoder.rs`.
5. Update the stale MPEG-1-only text: `lib.rs`, `encoder.rs`, the `header.rs` module doc
   (version row), the `PaddingSchedule` doc and the `tables.rs` header.
6. Document the panic in `FrameHeader::to_bytes` for a bitrate that is not valid for the
   sample rate (for example 320 kbit/s at 22.05 kHz).
7. Mark the public enums (`ConfigError`, `HeaderError`, `ChannelMode`, `SampleRate`)
   `#[non_exhaustive]` before 1.0.
8. `encode_i16` and `encode_planar` copy the whole input; chunk them to halve peak memory
   on huge buffers.

## The `wav2mp3` example

9. A `data` chunk of size 0 followed by real data (a recorder that crashed) gives a
   one-frame silent file and exit code 0. Warn or fail.
10. Check the WAVE_FORMAT_EXTENSIBLE subformat GUID, so a float file with 16-bit width is
    not accepted as PCM.
11. Give 8000, 11025, 12000 and 96000 Hz input a clear message instead of
    `UnsupportedSampleRate(8000)`, and warn when both a bitrate and `--vbr` are given.

## Done in this review

- Denormal-range input no longer makes encoding 60 to 100 times slower (flushed to
  silence below 1e-18).
- `wav2mp3` no longer panics on a WAV whose `fmt ` chunk is shorter than 16 bytes.
