# Desktop Block 18 decoder decision

## Decision

Select **Path B: one shared Rust streaming decoder**. Pin `symphonia` exactly at `0.6.0`, disable default features, and enable only `wav`, `pcm`, `flac`, `mp3`, `id3v1`, and `id3v2` for the initial release. There is no automatic fallback to Android `MediaCodec`, HTML audio, Web Audio, or a TypeScript decoder.

## Initial production scope

- Containers/codecs: WAV/PCM, native FLAC, and MP3.
- Input: stable app-private files produced by the platform staging boundary.
- Decoder output: source-native planar buffers consumed incrementally.
- Canonical host boundary: bounded 48 kHz stereo PCM16 little-endian chunks. Block 19 owns incremental sample conversion, channel mapping, resampling, backpressure, cancellation, and worker join.
- Metadata: retain Symphonia's defensive reader limits and regression-test oversized metadata; never load cover art or entire tracks into the host pipeline.

## Evidence

The executable spike builds on the repository's pinned Rust `1.97.1` toolchain, decodes representative WAV, FLAC, and MP3 fixtures, rejects corrupt/truncated input visibly, observes cancellation at a packet boundary, measures startup, throughput, and peak RSS, and exercises a 2 MiB metadata tag. Exact results are in `docs/measurements/DESKTOP_BLOCK18_DECODER_SPIKE_RESULTS.md` and its JSON companion.

## Rejected alternatives

- **Android/iOS-only platform decoding:** rejected as the long-term ownership model because desktop requires an independent decoder and dual production paths would create format and failure-policy drift. Existing Android decoding remains temporary until shared Block 23 migrates mobile and receives device evidence.
- **Desktop-only Rust adapter:** unnecessary because the selected library is portable Rust and can live at the shared boundary.
- **FFmpeg bindings:** rejected for the initial path because they add native-library packaging, ABI, licensing, and cross-platform deployment complexity not required for WAV/FLAC/MP3.
- **Rodio:** rejected as an ownership layer because it combines playback concerns with decoding and does not replace the project's shared bounded packetization boundary.
- **TypeScript, HTML audio, or Web Audio:** prohibited by the desktop architecture.

## Coordination with shared Block 23

This decision selects shared Block 23 Path B, but does **not** claim Block 23 complete. Android PCM-copy overhead, current platform memory/startup measurements, iOS file-access constraints, and physical-device format parity remain required before the temporary mobile decoder path is removed. No hidden fallback is introduced during that migration.
