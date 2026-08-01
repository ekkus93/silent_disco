#include "OboeOutputAdapter.h"

#include <jni.h>

namespace {

// One process-wide output stream, matching the PoC's single-host/single
// audio-source scope. Every function here is non-real-time control-plane
// glue called by Kotlin outside the audio callback.
silentdisco::OboeOutputAdapter &adapter() {
    static silentdisco::OboeOutputAdapter instance;
    return instance;
}

}  // namespace

extern "C" JNIEXPORT jint JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeOpen(
        JNIEnv * /* env */, jobject /* this */, jlong engineToken) {
    return static_cast<jint>(adapter().open(static_cast<int64_t>(engineToken)));
}

extern "C" JNIEXPORT void JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeClose(
        JNIEnv * /* env */, jobject /* this */) {
    adapter().close();
}

extern "C" JNIEXPORT jboolean JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeIsOpen(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jboolean>(adapter().isOpen());
}

extern "C" JNIEXPORT jint JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeActualSampleRate(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jint>(adapter().actualSampleRate());
}

extern "C" JNIEXPORT jint JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeActualChannelCount(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jint>(adapter().actualChannelCount());
}

extern "C" JNIEXPORT jint JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeTakeFatalStatus(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jint>(adapter().takeFatalStatus());
}

extern "C" JNIEXPORT jboolean JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeTakeDisconnected(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jboolean>(adapter().takeDisconnected());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeUnderrunCount(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jlong>(adapter().underrunCount());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeSilenceFilledFrames(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jlong>(adapter().silenceFilledFrames());
}

extern "C" JNIEXPORT jlong JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeOboeFramesRendered(
        JNIEnv * /* env */, jobject /* this */) {
    return static_cast<jlong>(adapter().framesRendered());
}
