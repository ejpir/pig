#include <array>
#include <jni.h>
#include <stdint.h>
#include <unistd.h>
#include <vector>

extern "C" int pi_bun_init();
extern "C" int pi_bun_pump(uint32_t, uint32_t);
extern "C" int pi_bun_eval(const uint8_t *, size_t, uint8_t *, size_t,
                           size_t *);
extern "C" JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *, void *) {
  return JNI_VERSION_1_6;
}
extern "C" JNIEXPORT jint JNICALL
Java_dev_pi_bunembedding_ProbeActivity_nativePid(JNIEnv *, jclass) {
  return getpid();
}
extern "C" JNIEXPORT jint JNICALL
Java_dev_pi_bunembedding_ProbeActivity_nativeInit(JNIEnv *, jclass) {
  return pi_bun_init();
}
extern "C" JNIEXPORT jbyteArray JNICALL
Java_dev_pi_bunembedding_ProbeActivity_nativeEval(JNIEnv *env, jclass,
                                                  jbyteArray source) {
  const auto n = env->GetArrayLength(source);
  if (n < 0 || n > 65536) {
    env->ThrowNew(env->FindClass("java/lang/IllegalArgumentException"),
                  "Source limit");
    return nullptr;
  }
  std::vector<uint8_t> input(n);
  env->GetByteArrayRegion(source, 0, n,
                          reinterpret_cast<jbyte *>(input.data()));
  if (env->ExceptionCheck())
    return nullptr;
  std::array<uint8_t, 16384> output{};
  size_t written = 0;
  int result = pi_bun_eval(input.data(), input.size(), output.data(),
                           output.size(), &written);
  if (result != 0 || written > output.size()) {
    env->ThrowNew(env->FindClass("java/lang/IllegalStateException"),
                  "Bun evaluation failed");
    return nullptr;
  }
  auto bytes = env->NewByteArray(written);
  if (bytes)
    env->SetByteArrayRegion(bytes, 0, written,
                            reinterpret_cast<const jbyte *>(output.data()));
  return bytes;
}

extern "C" JNIEXPORT jint JNICALL
Java_dev_pi_bunembedding_ProbeActivity_nativePump(JNIEnv *, jclass,
                                                  jint milliseconds) {
  if (milliseconds < 1 || milliseconds > 20)
    return -1;
  const int turns = pi_bun_pump(4, static_cast<uint32_t>(milliseconds) * 1000);
  return turns < 0 ? turns : 0;
}
