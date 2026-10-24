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
`complete` property must be true before trusting the WAV: it requires a
nonempty capture, at least one callback, no tap loss, and agreement between
written and captured frame counts. A tap overflow or truncated/empty recording
invalidates that evidence. `noUnexpectedOutputReopen` additionally requires a
previously opened native stream and unchanged native open **and close** counts
through the capture interval; rebinding the same output stream is permitted.
Unknown native lifecycle state is never treated as a pass. These checks do not
establish acoustic/DAC continuity or rule out underruns; compare the PCM against
canonical source data and inspect native underrun/silence counters separately.
The helper is disabled until explicitly enabled through developer diagnostics.

This tap captures the Rust callback-returned PCM, still upstream of Android's
mixer, speaker, DAC and any Bluetooth device. Do not misreport this as an
independently measured physical output recording; that hardware-level
acceptance remains a separate device test.

## Android debug-screen capture controls

Commit `a8d5d41d` adds a developer-only control under
**Advanced diagnostics → Playback output**. On a DEBUG build and when the
device is a listener, tap **Start PCM capture** after the listener runtime
has opened, play a representative multi-track sequence, then tap
**Stop PCM capture** (or leave the session). The helper writes an app-specific
diagnostics WAV and displays its filename, callback count and captured
frame count, or explicitly warns if the native tap lost data.
Release builds do not expose these controls and recordings are never uploaded
to GitHub Actions by default. Retrieve a user-authorized debug capture
from the device for comparison against canonical reference WAV files.

Hardware acceptance must still check playback under a real output route
(speaker, headphone or Bluetooth) and independently measure or otherwise
verify OS/mixer/DAC discontinuity, as in the authoritative TODO.

## Exact source identity in machine-readable comparison evidence

Pass `--commit-sha "$GITHUB_SHA"` when running the comparator in GitHub Actions,
using the exact 40-character checkout SHA rather than a moving branch name.
The JSON report records `commit_sha` (lowercase), or `null` when no commit
identity was supplied. Invalid or abbreviated identities produce ERROR/exit 2;
they cannot silently label a passing comparison. This is **caller-supplied
provenance**, not cryptographic attestation: the caller must bind the checkout,
reference WAV generation, capture, and comparator invocation to the same SHA.
The comparison still requires explicit real audio capture and does not imply
physical-device acceptance.

The `track_boundaries` report also records `first_mismatch_frame` for each
boundary window (or `null` if that window is clean). This is independent of
the globally capped `first_mismatches` examples, so heavy corruption around
one transition cannot hide the location of a later transition's first error.

## Non-track-aligned native callback regression

The real decoded mixed-rate playlist -> authorized UDP socket -> listener scheduler/pump
-> render ring -> native C-ABI callback test reads the 1,440 canonical output
frames in callback chunks of 241, 337, 517 and 345 frames. Two callbacks
cross the source boundaries at sample indices 480 and 960; no callback is
track-aligned. Hosted PCM metadata records these chunk sizes and asserts four
callbacks, 1,440 native frames, zero callback underruns, zero synthesized
silence, zero capture-tap loss, and bit-exact PCM (apart from the modeled
initial startup fade). This qualifies a deterministic software callback
continuity regression, not physical output continuity or all pause/Stop/edit
permutations.

The hosted mixed-rate socket/callback regression writes its comparison JSON
**before** asserting on a PCM mismatch, so a failing Rust test can retain the
first mismatching frame/channel, total mismatching samples, actual native
callback/underrun/silence counters and tap-loss counts. The report includes
only deterministic fixture metadata, never raw PCM. A failure does not become
PASS merely because a diagnostic artifact was produced.

The mixed-rate native-callback regression also exercises **production TCP
StreamStart and TrackBoundary control delivery** for both boundaries over the
same authenticated host/listener socket pair that transports UDP audio. It
checks that the listener receives each exact canonical sample index and entry
metadata before evaluating native callback-returned PCM. This does not yet
cover the Android UI boundary presentation or reconnect under live hardware.

## Android output-lifecycle acceptance guard

CallbackAudioCaptureReport.noUnexpectedOutputReopen requires a live output
at both capture start and end (opens > closes), unchanged open/close counts,
and successfully parsed nonnegative lifecycle counters. Historical
opens=1/closes=1 is an already closed output, not successful no-reopen
capture. Rebinding the same live output does not imply reopening. JVM
regressions cover inactive output, close, replacement open, malformed summary,
and complete versus truncated/dropped WAV evidence. Unknown counters remain
unknown, not inferred as zero.

## Encoded-source callbacks crossing codec boundaries

The WAV/FLAC/MP3 production-playlist regression now retains all decoded
packets in a single native output ring and requests three non-packet-aligned
callbacks. The first callback spans the WAV-to-FLAC decoded-sample boundary,
and the second spans the FLAC-to-MP3 boundary. The test compares the returned
float PCM with independently decoded canonical samples, permits only explicit
final-packet zero padding, and checks native callback underruns, silence and
capture-tap loss. This is software callback evidence, not physical DAC evidence.

The first version at `68fd7235` exposed a **test harness** limitation: its
4,800-frame capture tap was smaller than the deliberately long first callback,
so the tap correctly reported dropped diagnostic frames. Commit `bfcc983e`
sizes the tap to the bounded encoded playlist's output ring and applies pinned
rustfmt. The source-line policy passed; exact-head Rust/Android and desktop
workflows were pending at this checkpoint. Do not mark the comprehensive
production-path regression complete until those tests pass and the remaining
scheduler, edit, pause, Stop/replay and recovery permutations are covered.

## Encoded fixtures through both direct callback and listener scheduler (pending CI)

At exact source `dc69129a`, the encoded WAV/FLAC/MP3 test sends the real
playlist producer's original sample-indexed presentation timestamps over
authenticated TCP/UDP sockets. In addition to the direct C-ABI ring/capture
comparison, received UDP datagrams are submitted to the production Rust
`PlaybackScheduler` and `PlaybackPump`, then captured using a second native
callback. Virtual deadlines advance the bounded listener jitter buffer; the
expected waveform independently decodes each file using the canonical Rust
decoder and explicitly models only the initial listener startup fade and
final-packet padding. Codec changes must not introduce another fade, silence,
or output-ring reopen. The production Rust CI job writes only diagnostic JSON
(`encoded-socket-callback.json`) with the exact `GITHUB_SHA`, boundary
indices, first failing frame/channel and native scheduler/capture counters.
No raw PCM or music is attached to a CI artifact.

This additional coverage was **not yet qualified by exact-head CI** when
recorded. It does not exercise real Oboe/desktop output-device lifecycle,
actual acoustic output, late-join/reconnect, queue edits, or Stop/replay.
Do not interpret the software callback test as physical gapless acceptance.

## Authoritative queue edit, pause and replay through native callback (pending CI)

Commit `705118ba` connects the actor's previously qualified edit-before-reservation
backpressure test to two separate real native output rings and callback capture
taps. Before Stop and after the fresh replay stream, it checks that callbacks
which span both queued boundaries return only the current track, accepted
replacement, and appended tail, with zero native underrun, inserted silence,
or tap loss. The removed source must never appear, and the actor must refuse
a delayed reservation from the retired stream. The two stream generations
must not share the same render-ring token.

This new output comparison remains pending exact-head hosted test qualification.
It is separate from a live Android Oboe device output or mixer/DAC measurement.

## Dynamic gain and rejected-reservation failure callback (pending exact-head qualification)

The gain test added at source `b0200694` drives real encoded WAV/FLAC/MP3 fixtures over the
host TCP/UDP sockets and native Rust listener scheduler. It deliberately changes listener
volume from unity to one-half at the first source boundary and to one-quarter at the
second. Expected samples are independently decoded, adjusted for the initial native
scheduler fade, and scaled by each explicit gain only **after** its sample boundary.
The nonaligned native callback, capture-tap and machine-readable JSON must agree exactly;
a gain change must not insert silence, corrupt a sample or reset the initial fade.
Its hosted Rust test, formatter and strict Clippy gate passed, but exact-head Android and
desktop qualification was still outstanding when recorded.

A separate new reserved-boundary failure test at source `50b58ef0` checks that the
production FFI playlist refuses a stale successor and publishes neither its track
metadata nor its audio. The still-valid 480-frame current song is rendered through the
real C-ABI callback and compared sample-by-sample. A subsequent *intentional* read of
17 missing frames must produce explicit callback underrun/silence counters; this is
**not** accepted as a gapless track transition. When hosted, the test saves only
`rejected-boundary-callback.json` with the SHA, first mismatch sample/channel and
native/tap counts. The new failure-path test remains pending exact-head CI; it is
not evidence of any physical Android mixer/DAC output continuity.

## Exact-SHA JSON-evidence gate in GitHub Actions (pending qualification)

Source commit `043d815b` adds `scripts/check_pcm_evidence.py` and standard-library
regressions that require `mixed-rate-socket-callback.json`,
`encoded-socket-callback.json`, and `rejected-boundary-callback.json` on the
exact hosted `GITHUB_SHA`. The validator checks report success and measurable
callback/boundary counts, including exact scheduler samples and native tap loss,
while treating 17 deliberately missing post-failure frames as an **expected
reported underrun**, not as gapless playback. It rejects stale SHA labels,
incomplete or malformed JSON, missing report files, and any unrecognized extra
file (including copyrighted/raw audio) in the evidence directory. The GitHub
Actions Rust quality job runs the validator regression suite and then the
validator after actual workspace tests, before always-uploaded diagnostics.

This is a report integrity gate only: the Rust comparison regressions still
must run successfully, and neither an emulator nor Rust callback proves a
physical mixer, DAC, Bluetooth or speaker output was continuous. The new CI
gate's exact-source qualification remained pending when these notes were written.

## Exact code-head Rust qualification of PCM evidence gate

At source `043d815b228f2d16db4085af406ca6c1a4ae9ae8`, the hosted Rust
format, strict Clippy and workspace-test job passed. The regression logs
explicitly show both the encoded source/scheduler callback and rejected
successor native callback tests passing, plus 10 report-validator tests and
`PCM_EVIDENCE_PASS`. The three JSON-only metadata reports were uploaded in
GitHub Actions artifact `12537977844`. The uploaded reports prove software
callback-level comparisons only, and the intentional post-rejection output
underrun is reported rather than hidden as zero-gap playback.

The separate Android build/emulator/release and desktop CI workflows were
still running at the time this checkpoint was written; they must be checked
before claiming a full exact-source platform pass. Hardware output and broad
remaining software checklist items remain open.

## Truncated RIFF metadata cannot certify complete callback PCM

The PCM comparator now scans RIFF chunks even when the physical WAV file is
shorter than its declared RIFF size. A missing trailing metadata chunk or a
partially written non-audio chunk fails closed instead of returning PASS merely
because the first audio data chunk matches the reference. Only an actually
truncated audio data payload proceeds to frame-level accounting, preserving
explicit missing-frame diagnostics. Synthetic regressions cover both the
captured WAV and canonical source WAV. This is evidence-integrity hardening,
not new physical output or complete lifecycle qualification.

## Reject incomplete interleaved PCM frames in declared WAV chunks

The WAV comparator now rejects a RIFF `data` chunk whose declared byte count
is not a multiple of the PCM16 channel-frame width (2 bytes for mono, 4 for
stereo). Python's `wave.getnframes()` rounds down such a malformed header,
which could otherwise obscure a partial callback frame in the evidence.
Synthetic regressions exercise malformed callback and canonical-source WAVs,
while verifying that a valid odd-numbered mono frame count still passes.
This improves software evidence integrity; it does not establish device output
continuity or complete the production-path regression.

## Reject out-of-range encoded callback boundary evidence

The exact-SHA JSON gate also requires both encoded WAV/FLAC/MP3 source
boundaries to fall strictly inside the captured PCM output. A report whose
observed and expected boundaries agree but point past the final captured
frame cannot certify any transition. Negative tests cover boundaries at
and beyond the output end; real production callback comparisons remain the
source of the reports. This is a metadata consistency check, not physical
output acceptance.

## Hosted authoritative pause/resume callback evidence

The production UniFFI actor-reserved playlist pause test now retains the
prebuffered successor while the host is Paused, rejects removal of that
reserved successor, resumes without replacing the native render-ring token,
and compares non-track-aligned C-ABI callback-returned PCM against both
synthetic source tracks. Its new `paused-resume-callback.json` CI report
records the actual source boundary, callback frame chunks, tap counts,
underruns, synthesized silence, actor reservation state and output-token
continuity. The exact-SHA validator requires this fifth JSON-only report
and fails if any pause, removal, callback or tap condition is not met.
The test exercises Rust producer/actor/callback software behavior; it does
not yet prove Android Oboe, desktop audio-device, TCP/UDP scheduler, or
physical mixer/DAC continuity under pause/resume.

## Paused actor-reserved producer: packet-order evidence

The deterministic authoritative UniFFI pause/resume regression now checks every
producer audio packet's sequence and first sample index before pushing decoded
PCM to the same native output ring. It requires sequences `[0, 1, 2, 3]`,
starts `[0, 240, 480, 720]`, and a reserved successor boundary at exactly
sample 480, while the actor remains paused. The callback comparison then
verifies 960 exact frames across non-track-aligned callback reads after resume.
The public metadata report includes packet sequences and first sample indices;
its fail-closed validator rejects missing, duplicate, reordered, or gapped
packet evidence. This verifies producer ordering and native callback continuity,
not physical speaker/DAC output or real-device pause behavior.

## Reject inconsistent or ambiguous WAV format metadata

The standalone PCM comparator now checks each RIFF `fmt ` chunk's PCM16
channel/frame alignment and declared byte rate against its sample rate.
Python's `wave` reader accepts malformed `nBlockAlign`/`nAvgBytesPerSec`
values, which could otherwise let corrupted canonical or callback evidence
pass a byte-for-byte comparison. Multiple `fmt ` chunks also fail closed.
Synthetic regressions cover both the canonical references and captured WAVs,
while a restored valid header still passes. This is evidence integrity, not
proof of the physical DAC or comprehensive playback lifecycle qualification.

## Retired native token cannot drain preloaded replay PCM

The actor/FFI Stop/replay regression now loads the first real 240-frame replay packet into the new native ring before invoking a delayed callback with the old output token. It verifies the retired callback returns Stopping with silence and zero supplied frames, that the fresh output still has all 240 frames, and that the fresh callback tap has zero callbacks and zero captured/lost frames. The test then renders the complete 10,560-frame replay (including that preserved packet) and requires exact canonical PCM, zero underruns, zero tap loss, and an empty ring. The exact-SHA JSON gate rejects missing or incorrect `retired_native_token_preserved_replay_frames` metadata. This is deterministic native callback generation isolation, not a physical Oboe/mixer/DAC measurement.

## Reject capture/reference file aliasing (2026-10-24)

The standalone comparator now rejects a captured WAV that is the same file as
any canonical source reference, including hard-linked or symlinked aliases.
Otherwise a self-comparison can report PASS even for a capture containing an
undetected gap or duplicated frame. This is a provenance guard, not proof that
two distinct files were independently generated; CI must still decode its
canonical reference from source fixtures and capture native callback output.
Intentionally repeated source tracks remain supported in the expected playlist.
CLI and direct comparator regressions exercise the rejection and duplicate case.
