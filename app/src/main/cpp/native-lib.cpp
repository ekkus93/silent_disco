#include <jni.h>
#include <oboe/Oboe.h>
#include <string>

extern "C" JNIEXPORT jstring JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeGetAudioBackend(
        JNIEnv *env,
        jobject /* this */) {
    std::string backend = "Oboe";
    return env->NewStringUTF(backend.c_str());
}

extern "C" JNIEXPORT jstring JNICALL
Java_com_ekkus_silentdisco_core_audio_OboeBridge_nativeGetAudioStatus(
        JNIEnv *env,
        jobject /* this */) {
    std::string status =
            "Sample rate conversion and playback stream management should be built on this native boundary.";
    return env->NewStringUTF(status.c_str());
}
