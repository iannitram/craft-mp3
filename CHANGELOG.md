# Changelog

Newest first. One line per change of stage, release or decision that the next session needs.

- 2026-11-12 Forced stereo modes (#35): StereoMode option on EncoderConfig (Auto, MidSide, LeftRight, DualChannel) with default output byte-identical to 1.1.0; wav2mp3 --stereo flag; mono input with stereo option rejected with ConfigError.
- 2026-11-12 ID3v2.4 tags (#29): `Id3Tag` (title, artist, album, year, comment, chapters from WavCraft markers as CTOC/CHAP) and `with_id3`, which puts the tag in front of the stream so the Info frame stays first; `wav2mp3` gets `--title`, `--artist`, `--album`, `--year`, `--comment` and `--chapter name@seconds`. Streams without a tag are byte-identical. The version bump waits for the release.
- 2026-11-12 24-bit PCM and IEEE float WAV input in wav2mp3 (#36): wav2mp3 reads 16-bit or 24-bit PCM and 32-bit float WAV (plain and EXTENSIBLE), scaled to +/-1.0 for Encoder::encode; 16-bit output stays byte-identical.
- 2026-11-12 1.1.0: gapless MP3 (#25: LAME-layout delay, padding and CRCs in the Info tag, encoder string `LAMEcraft`), each frame and its borrowed bytes kept within the decoder buffer (#24), the duplicate 2.038 s Info-tag test removed (#16). The API is unchanged. Tagged `v1.1.0` on the owner's ruling "Tag v1.1.0 + bump WavCraft (Recommended)" (2026-11-12), so WavCraft can pin it.
- 2026-11-12 Gapless (#25): the Info/Xing tag carries LAME's layout fields (delay 528, padding, music length and CRCs) with the encoder string `LAMEcraft`, so ffmpeg-based players play the exact input length; the owner chose the LAME-prefixed marker. Opt-in ffmpeg check in tests/gapless.rs. Streams of two frames (empty or one-sample input) still don't open in ffmpeg, as on main.
- 2026-11-11 Roadmap map: the roadmap now opens with a features block (encoder, provenance, docs, api, wav2mp3, ci, release) and its status says 1.0.0 is released, with #16 the one open issue.
- 2026-11-03 1.0.0: first release. Huffman table source recorded (#4); docs fixed for MPEG-2 and the Info-tag duration (#5, #6, #7); public enums non_exhaustive (#8); chunked encode_i16/encode_planar (#9); wav2mp3 input checks (#10); CI on Mac, PC and Linux (#3); dead bitwriter removed (#2).
- 2026-10-26 End-of-night sweep: code map links and counts corrected, ROADMAP lists #2, #3 and #11 with a status line; locked tests added to Easy issues #2, #8, #9, #10, #11; #3, #5, #6, #7 state their CI check.
- 2026-10-26 Spec approved by the owner ("ROADMAP's first release (Recommended)"): SPEC.md, docs/code-map.md, issues #3-#11; ROADMAP item 2 found done; stage Dev Loop.
- 2026-10-26 Normalized to pipeline v2 at Spec (board had it at Prototype / Beta; no spec yet).
- 2026-10-15 Crate created (MIT OR Apache-2.0); MP3 encoder merged since.
