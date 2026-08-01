#ifndef SILENT_DISCO_AUDIO_H
#define SILENT_DISCO_AUDIO_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Narrow, real-time-safe C ABI for reading from a Rust-owned render ring.
 *
 * This header is the ONLY contract the real-time audio callback (e.g. an
 * Oboe callback on Android) should use. It must never call into UniFFI,
 * JNI, SQLite, or any blocking/allocating Rust path — every function
 * declared here is safe to call from the audio thread at callback cadence.
 *
 * `SilentDiscoAudioEngine*` is an opaque handle, not a real object pointer:
 * Rust never dereferences it. It is obtained from a separate, non-real-time
 * control-plane path (UniFFI or an internal Rust API), passed down to
 * native audio setup code once per stream, and then used only with the
 * functions below for the lifetime of that stream.
 */

typedef struct SilentDiscoAudioEngine SilentDiscoAudioEngine;

typedef enum SilentDiscoAudioStatus {
    /* Every requested frame was supplied from real ring contents. */
    SILENT_DISCO_AUDIO_OK = 0,
    /* At least one requested frame was missing and filled with silence;
     * `frames_from_ring` reports how many frames were real. Not an error:
     * the caller should treat this as an audible underrun to surface in
     * diagnostics, not a fatal condition. */
    SILENT_DISCO_AUDIO_PARTIAL = 1,
    /* The engine has been released (stream stopping/stopped). Output is
     * entirely silence. This is a normal terminal state, not an error. */
    SILENT_DISCO_AUDIO_STOPPING = 2,
    /* A call-shape problem: a required pointer was null, `requested_frames`
     * was zero, or `output_channels` did not match the fixed render format.
     * Output is untouched (may be null) or, where a valid output pointer
     * was provided, zeroed. */
    SILENT_DISCO_AUDIO_INVALID_ARGUMENT = -1,
    /* `engine` does not refer to any known engine (never issued, or a
     * malformed/guessed value). Output is zeroed. */
    SILENT_DISCO_AUDIO_INVALID_STATE = -2,
    /* A Rust panic occurred and was caught at this boundary; it did not
     * unwind into native code. Output is zeroed and a diagnostic counter
     * was incremented. */
    SILENT_DISCO_AUDIO_PANIC_CONTAINED = -3
} SilentDiscoAudioStatus;

/* Stable ABI contract version implemented by the linked native library.
 * Callers should check this once at startup, not per callback. */
uint32_t silent_disco_audio_abi_version(void);

/*
 * Reads up to `requested_frames` interleaved frames of `output_channels`
 * channels into `output`, filling any missing frames with silence, and
 * reports the real (non-silence) frame count via `frames_from_ring`.
 *
 * `output_channels` must match the engine's fixed internal channel count
 * (2, stereo) or this call returns SILENT_DISCO_AUDIO_INVALID_ARGUMENT
 * without touching `output`.
 *
 * Never blocks, never allocates, and never lets a Rust panic propagate to
 * the caller — a caught panic zeroes `output` and returns
 * SILENT_DISCO_AUDIO_PANIC_CONTAINED instead of unwinding.
 *
 * `output` and `frames_from_ring`, if non-null, must each point to memory
 * valid for the entire duration of this call: `output` for at least
 * `requested_frames * output_channels` `float`s, `frames_from_ring` for one
 * `uint32_t`. Passing null for either, or zero for `requested_frames`,
 * returns SILENT_DISCO_AUDIO_INVALID_ARGUMENT and leaves both untouched.
 *
 * A null, unknown, or never-issued `engine` returns
 * SILENT_DISCO_AUDIO_INVALID_STATE and zeroes `output` (when non-null) and
 * `frames_from_ring` (when non-null). A released `engine` returns
 * SILENT_DISCO_AUDIO_STOPPING with the same zeroing behavior.
 */
SilentDiscoAudioStatus silent_disco_audio_read_interleaved_f32(
    SilentDiscoAudioEngine *engine,
    float *output,
    uint32_t requested_frames,
    uint32_t output_channels,
    uint32_t *frames_from_ring
);

/* Frames currently available to read without silence-filling. Returns 0
 * for a null, unknown, or released `engine` — indistinguishable from a
 * legitimately empty ring; only the read function's status is
 * authoritative for detecting an invalid or stopping engine. */
uint32_t silent_disco_audio_available_frames(const SilentDiscoAudioEngine *engine);

/* Cumulative count of read calls that had to silence-fill at least one
 * frame. See the null/unknown/released-engine note above. */
uint64_t silent_disco_audio_underrun_count(const SilentDiscoAudioEngine *engine);

/* Cumulative count of frames actually supplied from real ring contents
 * (never silence). See the null/unknown/released-engine note above. */
uint64_t silent_disco_audio_frames_rendered(const SilentDiscoAudioEngine *engine);

/* Cumulative count of requested frames that were filled with silence
 * because the ring did not hold enough data. See the null/unknown/
 * released-engine note above. */
uint64_t silent_disco_audio_silence_filled_frames(const SilentDiscoAudioEngine *engine);

/* Cumulative count of producer writes that could not fit every frame they
 * attempted to write. See the null/unknown/released-engine note above. */
uint64_t silent_disco_audio_ring_full_events(const SilentDiscoAudioEngine *engine);

/* Cumulative count of silent_disco_audio_read_interleaved_f32 invocations
 * for this engine. See the null/unknown/released-engine note above. */
uint64_t silent_disco_audio_callback_count(const SilentDiscoAudioEngine *engine);

/* Cumulative count of panics caught and contained inside
 * silent_disco_audio_read_interleaved_f32 for this engine. See the
 * null/unknown/released-engine note above. */
uint64_t silent_disco_audio_contained_panic_count(const SilentDiscoAudioEngine *engine);

#ifdef __cplusplus
}
#endif

#endif
