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
};

}  // namespace silentdisco

#endif
