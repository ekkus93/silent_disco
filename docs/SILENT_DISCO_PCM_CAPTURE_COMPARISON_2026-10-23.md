# Objective gapless PCM capture and comparison — 2026-10-23

## Existing capture tap and its limit

ListenerPlaybackRuntime.start_debug_capture(path) already provides opt-in WAV output
from the post-scheduler PCM released toward the Rust render ring. It records
concealed and drained frames, but not any silence the final native callback
must fill because the ring is empty. It also cannot prove actual DAC output.
See rust/silent-disco-core/src/audio/debug_recorder.rs and
rust/silent-disco-ffi/src/listener_playback/runtime.rs.

The native callback is silent_disco_audio_read_interleaved_f32 in
rust/silent-disco-ffi/src/audio_abi.rs. Its status and from-ring frame count
expose underruns. The native callback must never write a file, allocate, or
wait for file I/O. The Rust callback-capture regression records the returned
PCM *after the callback returns*, and asserts exact agreement with three
consecutive canonical source tracks. A second regression intentionally
requests a callback at the track boundary before the next track is queued
and verifies a one-frame silence insertion is observable.

## Automated exact PCM comparison

The standalone standard-library command is:

    python3 scripts/compare_pcm_capture.py \
      --expected /tmp/canonical-first.wav /tmp/canonical-second.wav \
      --captured /tmp/listener-capture.wav \
      --boundary-window-frames 480 \
      --report /tmp/gapless-report.json

Exit 0 = every frame/sample matched (or stayed within an *explicitly*
configured tolerance) and total length matched. Exit 1 = mismatch. Exit 2 =
unsupported format or failed read. Report includes expected/captured counts,
first mismatch positions, sample errors, and mismatches around each boundary.
Length and index alignment are strict; silently aligning the stream after
every new track could hide an inserted or dropped frame.

All WAV inputs must be uncompressed PCM16 with *the same* sample rate and
channel count. For heterogeneous source formats, decode and normalize with
the app's canonical Rust decoder first. Do not pretend that comparing the
bytes of a compressed or 44.1kHz source to 48kHz stereo output is meaningful.
Account for explicitly specified final packet padding in the reference.
Do not loosen tolerance for lossy/compressed source content without first
reproducing its intended decoded PCM.

## CI and future device acceptance

GitHub Actions source-file policy runs

    python3 scripts/test_compare_pcm_capture.py

The Python tests synthesize three tracks and assert failure for a missing
boundary frame, inserted zero, duplicated frame, corrupted sample, and
truncated capture. Regular Rust workspace tests also execute the real native
callback path through its C ABI, with an intentionally injected underrun.
This is stronger evidence than a source-only or pre-ring simulation, but
it still does not measure a physical loudspeaker.

Physical output capture requires an explicit opt-in nonblocking diagnostic
tap (with bounded preallocated storage, a non-real-time writer and overflow
telemetry) or an independent external audio loopback recorder. A sound-card,
Android system mixer, Bluetooth device, or DAC can still produce discontinuities
outside Rust. No physical audio evidence or real-device acceptance can be
claimed solely on the basis of these new tests.

Do not upload real copyrighted songs or listener audio into public CI
artifacts automatically. Use generated fixtures and retain the exact source
SHA, boundary samples, callback underrun counters and capture provenance.


## Produce canonical references from real source tracks

Use the repository's decoder, rather than Python/FFmpeg or manual
resampling, when the original sources are MP3, 44.1kHz, mono, etc.:

    cargo run --manifest-path rust/silent-disco-core/Cargo.toml \
      --example gapless_reference_wav -- \
      /tmp/silent-disco-references song-a.mp3 song-b.wav

This writes 000.wav and 001.wav in source order, using the same
StreamingDecodeHandle implementation as host preparation. Then compare
those per-track files to the listener capture with the command above.
These reference WAVs contain **decoded source frames**: any intentional
final-packet padding or other output DSP must be represented in the expected
data explicitly, not silently ignored by the comparator.

## Opt-in Android callback WAV capture (implementation checkpoint)

The native callback tap and its UniFFI methods are now implemented in
`rust/silent-disco-ffi/src/callback_capture.rs`, `audio_abi.rs`,
and `listener_playback/ffi_handle.rs`. A live `FfiListenerPlaybackHandle`
can call `startCallbackCapture(48_000u)` to attach a one-second bounded
float32 stereo tap; `readCallbackCapture(4096u)` drains samples on a normal
thread and reports cumulative captured, lost and callback counts;
`endCallbackCapture()` detaches while preserving the buffered tail, and
`clearCallbackCapture()` releases the diagnostic state. The audio thread
does not perform file I/O or allocate diagnostic buffers.

The Android helper `NativeCallbackWavCapture.start(playback, privateDirectory, scope)`
in `core/audio/NativeCallbackWavCapture.kt` starts that capture explicitly,
polls/dumps samples to an app-private WAV from `Dispatchers.IO`, and
`stopAndReport()` detaches, drains, finalizes the WAV and returns counters.
The separate `CallbackPcmWavWriter` limits recording to five minutes by
default and rejects malformed or non-finite frames. The report's
`complete` property must be true before trusting the WAV; a tap overflow
invalidates that evidence. The helper is not automatically enabled during
normal listening and is not a finished debug UI toggle.

This tap captures the Rust callback-returned PCM, still upstream of Android's
mixer, speaker, DAC and any Bluetooth device. Do not misreport this as an
independently measured physical output recording; that hardware-level
acceptance remains a separate device test.
