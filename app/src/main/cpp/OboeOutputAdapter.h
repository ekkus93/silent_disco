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
    // mistake for one that appears at random.
    int32_t openCount() const { return openCount_; }

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
    int64_t engineToken_ = 0;
    std::atomic<int32_t> fatalStatus_{0};
    std::atomic<bool> disconnected_{false};
    int32_t lastOpenSampleRate_ = 0;
    int32_t lastOpenChannelCount_ = 0;
    int32_t lastOpenSharingMode_ = -1;
    int32_t lastOpenPerformanceMode_ = -1;
    int32_t openCount_ = 0;
};

}  // namespace silentdisco

#endif
