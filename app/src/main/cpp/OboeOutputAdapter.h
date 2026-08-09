#ifndef SILENT_DISCO_OBOE_OUTPUT_ADAPTER_H
#define SILENT_DISCO_OBOE_OUTPUT_ADAPTER_H

#include <oboe/Oboe.h>

#include <atomic>
#include <cstdint>
#include <memory>

namespace silentdisco {

// Non-real-time result of an open() attempt.
enum class OboeAdapterStatus : int32_t {
    Ok = 0,
    AlreadyOpen = 1,
    OpenFailed = -1,
    StartFailed = -2,
    UnexpectedFormat = -3,
    AbiSymbolsUnavailable = -4,
    // The device granted a stream whose sample rate or channel count does not
    // match the render ring's fixed geometry. The read ABI is handed the
    // stream's own channel count, so channels can be adapted -- the sample
    // rate cannot: nothing communicates it to Rust, so a mismatched rate
    // renders 48 kHz ring content through a stream clocked at some other
    // rate. That is continuous audible corruption, not a degraded-but-usable
    // stream, so it is reported rather than played.
    UnexpectedSampleRate = -5,
    UnexpectedChannelCount = -6,
    // rebind() was called with no stream open. The caller opens instead;
    // this is a state report, not a failure of the rebind itself.
    NotOpen = -7,
};

// Owns one Oboe output stream bound to one Rust render-ring engine token.
//
// Every method except onAudioReady() (and the read-only accessors) is
// non-real-time: open()/close() must be called off the audio thread, exactly
// as the shared architecture requires. onAudioReady() calls only
// silent_disco_audio_read_interleaved_f32() -- resolved once via dlopen/dlsym
// against the already-packaged Rust cdylib rather than linked at build time,
// so this adapter has no build-order dependency on cargo-ndk's output -- and
// never touches JNI, UniFFI, or any allocating/blocking path.
class OboeOutputAdapter : public oboe::AudioStreamCallback {
public:
    OboeOutputAdapter() = default;
    ~OboeOutputAdapter() override;

    OboeOutputAdapter(const OboeOutputAdapter &) = delete;
    OboeOutputAdapter &operator=(const OboeOutputAdapter &) = delete;

    // Opens a low-latency float stereo 48 kHz output stream bound to
    // `engineToken` (opaque; see include/silent_disco_audio.h) and starts
    // it. Returns AlreadyOpen without changing anything if already open.
    OboeAdapterStatus open(int64_t engineToken);

    // Points the already-running stream at a different render-ring engine
    // token, without stopping, closing, or reopening it.
    //
    // This is what a track change uses. Closing an Exclusive/MMAP stream and
    // immediately reopening it does not reliably get the exclusive path back
    // -- a real Android 8.0 device granted `Shared` on the second open
    // (2026-08-09), so a second track rendered through a different output
    // path, with different burst and callback timing, than the first.
    // Keeping one stream for the session's lifetime removes that entire
    // class of difference: the output device is owned by the connection, not
    // by an individual stream, so only the *content* changes between tracks.
    //
    // Safe against the live callback: the token is swapped atomically, and
    // both the outgoing and incoming tokens are always valid to read through
    // (a released token reads as silence via the ABI, never freed memory),
    // so a callback landing mid-swap sees one or the other and never a torn
    // value. Call only after the outgoing stream has finished draining.
    OboeAdapterStatus rebind(int64_t engineToken);

    // Stops and closes the stream. Safe to call even if not open.
    void close();

    bool isOpen() const;
    int32_t actualSampleRate() const;
    int32_t actualChannelCount() const;

    // Configuration the device granted on the most recent open(), retained
    // after close() so a run's output path stays diagnosable afterwards.
    // The live accessors above read through `stream_` and therefore report
    // zero once closed, which is useless on a device whose logcat is
    // unavailable (confirmed on a real Android 8.0 phone) -- there, a
    // stream's granted configuration could otherwise only be observed while
    // it was still open, which is exactly when the UI cannot be navigated to
    // without tearing the session down.
    int32_t lastOpenSampleRate() const { return lastOpenSampleRate_; }
    int32_t lastOpenChannelCount() const { return lastOpenChannelCount_; }
    // oboe::SharingMode / oboe::PerformanceMode as their underlying ints,
    // so a silent Exclusive->Shared or LowLatency->None downgrade on reopen
    // is visible rather than inferred.
    int32_t lastOpenSharingMode() const { return lastOpenSharingMode_; }
    int32_t lastOpenPerformanceMode() const { return lastOpenPerformanceMode_; }
    // Times open() has been called successfully this process; a defect that
    // only appears on the second and later streams is otherwise easy to
    // mistake for one that appears at random. With rebinding working, a
    // multi-track session should show openCount 1 and rebindCount climbing;
    // an openCount that tracks the track count means something fell back to
    // reopening and the Shared-downgrade risk is back.
    int32_t openCount() const { return openCount_; }
    int32_t rebindCount() const { return rebindCount_; }

    // Non-real-time: returns and clears the fatal status last observed by
    // the real-time callback (SILENT_DISCO_AUDIO_PANIC_CONTAINED or
    // SILENT_DISCO_AUDIO_INVALID_STATE), or 0 if none. Poll this outside the
    // callback to decide whether to surface a failure.
    int32_t takeFatalStatus();

    // Non-real-time: true once the stream has disconnected (e.g. route
    // change) and closed itself; poll to decide whether to reopen.
    bool takeDisconnected();

    // Non-real-time telemetry queries, forwarded to the render ring's own
    // counters (see include/silent_disco_audio.h). Each returns 0 if this
    // adapter is not open.
    uint64_t underrunCount() const;
    uint64_t silenceFilledFrames() const;
    uint64_t framesRendered() const;

    oboe::DataCallbackResult onAudioReady(
        oboe::AudioStream *audioStream,
        void *audioData,
        int32_t numFrames) override;

    void onErrorAfterClose(oboe::AudioStream *audioStream, oboe::Result error) override;

private:
    std::shared_ptr<oboe::AudioStream> stream_;
    // Atomic because the real-time callback loads it on every wake-up while
    // a control-plane rebind() may be storing a new one; 64-bit atomics are
    // lock-free on every ABI this app ships, so the callback stays real-time
    // safe.
    static_assert(std::atomic<int64_t>::is_always_lock_free,
                  "engine token must be lock-free for the real-time callback");
    std::atomic<int64_t> engineToken_{0};
    std::atomic<int32_t> fatalStatus_{0};
    std::atomic<bool> disconnected_{false};
    int32_t lastOpenSampleRate_ = 0;
    int32_t lastOpenChannelCount_ = 0;
    int32_t lastOpenSharingMode_ = -1;
    int32_t lastOpenPerformanceMode_ = -1;
    int32_t openCount_ = 0;
    int32_t rebindCount_ = 0;
};

}  // namespace silentdisco

#endif
