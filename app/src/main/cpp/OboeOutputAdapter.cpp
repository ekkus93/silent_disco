#include "OboeOutputAdapter.h"

#include "silent_disco_audio.h"

#include <android/log.h>
#include <dlfcn.h>

namespace silentdisco {

namespace {

constexpr const char *kRustAbiLibraryName = "libsilent_disco_ffi.so";
constexpr const char *kLogTag = "SilentDiscoOboe";

using ReadInterleavedF32Fn = SilentDiscoAudioStatus (*)(
        SilentDiscoAudioEngine *,
        float *,
        uint32_t,
        uint32_t,
        uint32_t *);
using U64QueryFn = uint64_t (*)(const SilentDiscoAudioEngine *);

// Resolved once per process. `libsilent_disco_ffi.so` is packaged in the
// same APK's native library directory as this adapter, so dlopen-by-name
// finds and loads it directly regardless of whether Kotlin has already
// called System.loadLibrary for it -- no build-time or load-order
// coordination between the two native build systems is required.
void *resolveAbiSymbol(const char *symbolName) {
    void *libraryHandle = dlopen(kRustAbiLibraryName, RTLD_NOW);
    if (libraryHandle == nullptr) {
        __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                             "dlopen(%s) failed: %s", kRustAbiLibraryName, dlerror());
        return nullptr;
    }
    void *symbol = dlsym(libraryHandle, symbolName);
    if (symbol == nullptr) {
        __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                             "dlsym(%s) failed: %s", symbolName, dlerror());
    }
    return symbol;
}

ReadInterleavedF32Fn resolveReadFn() {
    static ReadInterleavedF32Fn cached = reinterpret_cast<ReadInterleavedF32Fn>(
            resolveAbiSymbol("silent_disco_audio_read_interleaved_f32"));
    return cached;
}

U64QueryFn resolveUnderrunCountFn() {
    static U64QueryFn cached = reinterpret_cast<U64QueryFn>(
            resolveAbiSymbol("silent_disco_audio_underrun_count"));
    return cached;
}

U64QueryFn resolveSilenceFilledFramesFn() {
    static U64QueryFn cached = reinterpret_cast<U64QueryFn>(
            resolveAbiSymbol("silent_disco_audio_silence_filled_frames"));
    return cached;
}

U64QueryFn resolveFramesRenderedFn() {
    static U64QueryFn cached = reinterpret_cast<U64QueryFn>(
            resolveAbiSymbol("silent_disco_audio_frames_rendered"));
    return cached;
}

}  // namespace

OboeOutputAdapter::~OboeOutputAdapter() {
    close();
}

OboeAdapterStatus OboeOutputAdapter::open(int64_t engineToken) {
    if (stream_ != nullptr) {
        return OboeAdapterStatus::AlreadyOpen;
    }
    if (resolveReadFn() == nullptr) {
        return OboeAdapterStatus::AbiSymbolsUnavailable;
    }

    engineToken_ = engineToken;
    fatalStatus_.store(0, std::memory_order_relaxed);
    disconnected_.store(false, std::memory_order_relaxed);

    oboe::AudioStreamBuilder builder;
    builder.setDirection(oboe::Direction::Output)
            ->setPerformanceMode(oboe::PerformanceMode::LowLatency)
            ->setSharingMode(oboe::SharingMode::Exclusive)
            ->setFormat(oboe::AudioFormat::Float)
            ->setChannelCount(2)
            ->setSampleRate(48000)
            // Every value above is a *request*. Without these two, a device
            // that grants a different rate or format hands the callback a
            // stream clocked differently from the render ring, and the ring's
            // 48 kHz float content is then rendered through it verbatim --
            // continuous corruption, not silence. Letting Oboe own the
            // conversion keeps the callback's contract (48 kHz float) true
            // whatever the device actually granted. This matters most on the
            // *second* open of a process: closing an Exclusive/MMAP stream
            // and immediately reopening it is exactly when a device is most
            // likely to grant something other than what was asked for.
            ->setSampleRateConversionQuality(oboe::SampleRateConversionQuality::Medium)
            ->setFormatConversionAllowed(true)
            ->setCallback(this);

    std::shared_ptr<oboe::AudioStream> stream;
    if (builder.openStream(stream) != oboe::Result::OK) {
        engineToken_ = 0;
        return OboeAdapterStatus::OpenFailed;
    }

    // Recorded before any rejection below, so a refused configuration is
    // still diagnosable afterwards rather than vanishing with the stream.
    lastOpenSampleRate_ = stream->getSampleRate();
    lastOpenChannelCount_ = stream->getChannelCount();
    lastOpenSharingMode_ = static_cast<int32_t>(stream->getSharingMode());
    lastOpenPerformanceMode_ = static_cast<int32_t>(stream->getPerformanceMode());
    openCount_ += 1;
    __android_log_print(ANDROID_LOG_INFO, kLogTag,
                         "open #%d granted: sampleRate=%d channels=%d sharing=%d perf=%d",
                         openCount_, lastOpenSampleRate_, lastOpenChannelCount_,
                         lastOpenSharingMode_, lastOpenPerformanceMode_);

    if (stream->getFormat() != oboe::AudioFormat::Float) {
        // Writing float samples into a non-float stream would produce
        // garbage audio rather than silence; fail explicitly instead.
        stream->close();
        engineToken_ = 0;
        return OboeAdapterStatus::UnexpectedFormat;
    }

    // Backstop for the conversion requests above: nothing downstream can
    // adapt to a rate the ring does not produce, so a surviving mismatch is
    // reported instead of rendered.
    if (stream->getSampleRate() != 48000) {
        stream->close();
        engineToken_ = 0;
        return OboeAdapterStatus::UnexpectedSampleRate;
    }
    if (stream->getChannelCount() != 2) {
        stream->close();
        engineToken_ = 0;
        return OboeAdapterStatus::UnexpectedChannelCount;
    }

    if (stream->requestStart() != oboe::Result::OK) {
        stream->close();
        engineToken_ = 0;
        return OboeAdapterStatus::StartFailed;
    }

    stream_ = stream;
    return OboeAdapterStatus::Ok;
}

void OboeOutputAdapter::close() {
    if (stream_ == nullptr) {
        return;
    }
    stream_->stop();
    stream_->close();
    stream_ = nullptr;
    engineToken_ = 0;
}

bool OboeOutputAdapter::isOpen() const {
    return stream_ != nullptr;
}

int32_t OboeOutputAdapter::actualSampleRate() const {
    return stream_ != nullptr ? stream_->getSampleRate() : 0;
}

int32_t OboeOutputAdapter::actualChannelCount() const {
    return stream_ != nullptr ? stream_->getChannelCount() : 0;
}

int32_t OboeOutputAdapter::takeFatalStatus() {
    return fatalStatus_.exchange(0, std::memory_order_relaxed);
}

bool OboeOutputAdapter::takeDisconnected() {
    return disconnected_.exchange(false, std::memory_order_relaxed);
}

uint64_t OboeOutputAdapter::underrunCount() const {
    if (stream_ == nullptr) {
        return 0;
    }
    U64QueryFn queryFn = resolveUnderrunCountFn();
    if (queryFn == nullptr) {
        return 0;
    }
    auto *engine = reinterpret_cast<const SilentDiscoAudioEngine *>(
            static_cast<uintptr_t>(engineToken_));
    return queryFn(engine);
}

uint64_t OboeOutputAdapter::silenceFilledFrames() const {
    if (stream_ == nullptr) {
        return 0;
    }
    U64QueryFn queryFn = resolveSilenceFilledFramesFn();
    if (queryFn == nullptr) {
        return 0;
    }
    auto *engine = reinterpret_cast<const SilentDiscoAudioEngine *>(
            static_cast<uintptr_t>(engineToken_));
    return queryFn(engine);
}

uint64_t OboeOutputAdapter::framesRendered() const {
    if (stream_ == nullptr) {
        return 0;
    }
    U64QueryFn queryFn = resolveFramesRenderedFn();
    if (queryFn == nullptr) {
        return 0;
    }
    auto *engine = reinterpret_cast<const SilentDiscoAudioEngine *>(
            static_cast<uintptr_t>(engineToken_));
    return queryFn(engine);
}

oboe::DataCallbackResult OboeOutputAdapter::onAudioReady(
        oboe::AudioStream *audioStream, void *audioData, int32_t numFrames) {
    ReadInterleavedF32Fn readFn = resolveReadFn();
    if (readFn == nullptr) {
        return oboe::DataCallbackResult::Stop;
    }

    auto *engine = reinterpret_cast<SilentDiscoAudioEngine *>(
            static_cast<uintptr_t>(engineToken_));
    uint32_t framesFromRing = 0;
    SilentDiscoAudioStatus status = readFn(
            engine,
            static_cast<float *>(audioData),
            static_cast<uint32_t>(numFrames),
            static_cast<uint32_t>(audioStream->getChannelCount()),
            &framesFromRing);

    // PARTIAL (silence-filled underrun) is already fully handled by Rust --
    // the output buffer was filled correctly -- so only genuinely fatal
    // statuses are recorded here for the non-real-time layer to observe.
    if (status == SILENT_DISCO_AUDIO_PANIC_CONTAINED ||
        status == SILENT_DISCO_AUDIO_INVALID_STATE) {
        fatalStatus_.store(static_cast<int32_t>(status), std::memory_order_relaxed);
    }
    return oboe::DataCallbackResult::Continue;
}

void OboeOutputAdapter::onErrorAfterClose(oboe::AudioStream * /* audioStream */, oboe::Result /* error */) {
    disconnected_.store(true, std::memory_order_relaxed);
}

}  // namespace silentdisco
