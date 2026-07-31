# Desktop Block 19 — Bounded Streaming Decode

**Status:** Complete

## Decision carried forward

Desktop Block 18 selected the shared Rust decoder path, Symphonia `0.6.0`, and canonical 48 kHz stereo signed PCM16 output. Block 19 implements that decision in `silent-disco-core`; it does not add an HTML/Web Audio, TypeScript, FFmpeg, or platform-decoder fallback.

## Shared boundary

`silent_disco_core::audio` now owns:

- `AudioFormat::CANONICAL` for 48 kHz stereo PCM16;
- `DecodedPcmChunk` with a checked `SampleIndex`, bounded interleaved `Vec<i16>`, and explicit end-of-stream marker;
- `StreamingDecodeConfig` with hard limits for chunk frames, queue chunks, and source bytes;
- `StreamingDecodeHandle` with bounded receive operations, observable queue pressure, cooperative cancellation, deterministic join, and cancel-on-drop;
- stable error categories that distinguish unsupported, corrupt, invalid-metadata, empty, resource-limit, format-change, cancellation, I/O, and worker-panic failures.

The decoder reads one encoded packet at a time. It retains only the current decoded packet, one prior resampler frame, one current output chunk, one pending full chunk, and the bounded channel. It never concatenates the complete track.

## Conversion policy

The initial shared conversion policy is explicit:

- source rates from 8 kHz through 192 kHz;
- mono duplicated to left/right;
- stereo preserved in canonical channel order;
- more than two channels rejected as unsupported;
- incremental linear resampling to 48 kHz;
- source format changes rejected rather than silently reconfigured;
- floating-point decoder output clamped and converted to signed PCM16.

Duration remains `None` unless a future reviewed container path can supply a reliable value. Chunk position is reported only as the checked first canonical sample index.

## Desktop staging integration

The desktop `prepare_staged_audio_source` adapter resolves only the currently registered opaque source ID, verifies the complete descriptor still matches, keeps the canonical native path inside the desktop boundary, and starts the shared worker. This establishes the staged-source-to-shared-PCM handoff needed by the later packetizer and playback blocks without prematurely starting an unconsumed decoder from the platform-effect runner.

## Executable coverage

The guarded validation covers:

- WAV, FLAC, and MP3 success fixtures;
- an intentionally unenabled OGG container as unsupported;
- truncated FLAC as corrupt input;
- malformed ID3v2 metadata;
- empty input;
- a non-empty short final chunk;
- cancellation while a one-slot queue is full;
- observable bounded backpressure;
- restart with a new source at sample index zero;
- repeated open/decode/join cycles with no surviving worker;
- exact desktop staged-source resolution and stale-descriptor rejection;
- the complete shared Rust, desktop frontend/backend, Linux bundle, Android build/unit/lint/ABI, and managed-device regression matrix.

## Validation provenance

- Guarded GitHub Actions run: `30599085238`
- Guarded input commit: `4c05e5763b1771fc2c7a04690d46b8c76665aa43`
- Completion commit: written by the guarded workflow after all gates pass

Block 25/shared migration Block 14 still owns streaming packetization and playback consumption. Block 19 supplies its bounded ingestion boundary; it does not claim packetizer completion.
