// SPDX-License-Identifier: MIT
#include <android/log.h>
#include <unistd.h>
#include <errno.h>
#include <stdlib.h>
#include <limits.h>

#include "scope.h"
#include "identity_profile.h"
#include "property_view.h"
#include "zygisk.hpp"

namespace {

constexpr char kLogTag[] = "MaskCPU";

class MaskCpuModule final : public zygisk::ModuleBase {
public:
    void onLoad(zygisk::Api* api, JNIEnv* env) override {
        api_ = api;
        env_ = env;
    }

    void preAppSpecialize(zygisk::AppSpecializeArgs* args) override {
        // The private file mounts and property mappings survive library unload.
        api_->setOption(zygisk::Option::DLCLOSE_MODULE_LIBRARY);
        isTarget_ = false;
        viewReady_ = false;
        if (args->app_data_dir == nullptr || env_->ExceptionCheck()) {
            return;
        }
        const char* directory = env_->GetStringUTFChars(args->app_data_dir, nullptr);
        if (clearException("read app_data_dir") || directory == nullptr) {
            return;
        }
        isTarget_ = args->uid >= 10000 && maskcpu::packageFromDataDir(directory);
        env_->ReleaseStringUTFChars(args->app_data_dir, directory);
        if (isTarget_) {
            viewReady_ = maskcpu::preparePropertyView(api_, args->uid, maskcpu::kTargetPackage, &pendingState_);
            isTarget_ = viewReady_;
        }
    }

    void postAppSpecialize(const zygisk::AppSpecializeArgs*) override {
        if (viewReady_) viewReady_ = maskcpu::finalizePropertyView(pendingState_);
        pendingState_ = nullptr;
        if (!isTarget_ || env_->ExceptionCheck()) {
            return;
        }
        if (!viewReady_) {
            __android_log_print(ANDROID_LOG_ERROR, kLogTag,
                                "B identity experiment not applied: property view/host validation failed");
            return;
        }
        const jint frameResult = env_->PushLocalFrame(256);
        if (clearException("PushLocalFrame") || frameResult != JNI_OK) {
            return;
        }
        const bool javaVerified = replaceBuildIdentity();
        env_->PopLocalFrame(nullptr);
        const bool propertiesVerified = maskcpu::verifyPropertyView();
        const bool complete = viewReady_ && javaVerified && propertiesVerified;
        __android_log_print(complete ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kLogTag,
                            "package=%s B identity view complete=%s (no keygen/KeyMint self-test)",
                            maskcpu::kTargetPackage, complete ? "yes" : "no");
    }

    void preServerSpecialize(zygisk::ServerSpecializeArgs*) override {
        api_->setOption(zygisk::Option::DLCLOSE_MODULE_LIBRARY);
    }

private:
    bool clearException(const char* operation) {
        if (!env_->ExceptionCheck()) {
            return false;
        }
        env_->ExceptionClear();
        __android_log_print(ANDROID_LOG_ERROR, kLogTag, "JNI failed: %s", operation);
        return true;
    }

    struct PendingField {
        jclass owner;
        jfieldID id;
        char kind;
        jvalue before;
        jvalue after;
    };

    bool stringEqual(jstring a, jstring b) {
        if (!a || !b) return a == b;
        const char* x = env_->GetStringUTFChars(a, nullptr);
        if (clearException("read string") || !x) return false;
        const char* y = env_->GetStringUTFChars(b, nullptr);
        if (clearException("read expected string") || !y) {
            env_->ReleaseStringUTFChars(a, x); return false;
        }
        const bool same = strcmp(x, y) == 0;
        env_->ReleaseStringUTFChars(a, x);
        env_->ReleaseStringUTFChars(b, y);
        return same;
    }

    jobjectArray stringArray(const char* text) {
        jclass strings = env_->FindClass("java/lang/String");
        if (clearException("find String") || !strings) return nullptr;
        size_t count = text[0] ? 1 : 0;
        for (const char* p = text; *p; ++p) if (*p == ',') ++count;
        auto result = env_->NewObjectArray(static_cast<jsize>(count), strings, nullptr);
        if (clearException("allocate ABI array") || !result) return nullptr;
        const char* start = text;
        for (size_t i = 0; i < count; ++i) {
            const char* end = strchr(start, ',');
            const size_t length = end ? static_cast<size_t>(end - start) : strlen(start);
            if (length == 0 || length >= 92) return nullptr;
            char item[92]{}; memcpy(item, start, length);
            jstring value = env_->NewStringUTF(item);
            if (clearException("allocate ABI") || !value) return nullptr;
            env_->SetObjectArrayElement(result, static_cast<jsize>(i), value);
            env_->DeleteLocalRef(value);
            if (clearException("set ABI")) return nullptr;
            if (end) start = end + 1;
        }
        return result;
    }

    bool arrayEqual(jobjectArray a, jobjectArray b) {
        if (!a || !b) return a == b;
        const jsize length = env_->GetArrayLength(a);
        if (length != env_->GetArrayLength(b) || clearException("array size")) return false;
        for (jsize i = 0; i < length; ++i) {
            auto x = static_cast<jstring>(env_->GetObjectArrayElement(a, i));
            auto y = static_cast<jstring>(env_->GetObjectArrayElement(b, i));
            const bool same = !clearException("read ABI") && stringEqual(x, y);
            if (x) env_->DeleteLocalRef(x);
            if (y) env_->DeleteLocalRef(y);
            if (!same) return false;
        }
        return true;
    }

    bool queueField(PendingField* fields, size_t* count, jclass owner, const char* name,
                    char kind, const char* value, bool optional = false) {
        const char* signature = kind == 'S' ? "Ljava/lang/String;" :
            kind == 'A' ? "[Ljava/lang/String;" : kind == 'I' ? "I" : "J";
        jfieldID id = env_->GetStaticFieldID(owner, name, signature);
        if (env_->ExceptionCheck() || !id) {
            jthrowable exception = env_->ExceptionOccurred();
            env_->ExceptionClear();
            bool absent = false;
            if (optional && exception) {
                jclass missing = env_->FindClass("java/lang/NoSuchFieldError");
                if (!clearException("find NoSuchFieldError") && missing)
                    absent = env_->IsInstanceOf(exception, missing);
                if (missing) env_->DeleteLocalRef(missing);
            }
            if (exception) env_->DeleteLocalRef(exception);
            if (!absent) __android_log_print(ANDROID_LOG_ERROR, kLogTag, "B view field unavailable: %s", name);
            return absent;
        }
        if (*count >= 80) return false;
        PendingField field{}; field.owner = owner; field.id = id; field.kind = kind;
        if (kind == 'S' || kind == 'A') {
            field.before.l = env_->GetStaticObjectField(owner, id);
            if (clearException(name)) return false;
            field.after.l = kind == 'S' ? static_cast<jobject>(env_->NewStringUTF(value)) :
                static_cast<jobject>(stringArray(value));
            if (clearException(name) || !field.after.l) return false;
        } else {
            errno = 0; char* end = nullptr;
            const long long number = value[0] ? strtoll(value, &end, 10) : 0;
            if (errno || (value[0] && (!end || *end)) || number < 0 ||
                (kind == 'I' && number > INT_MAX)) return false;
            if (kind == 'I') {
                field.before.i = env_->GetStaticIntField(owner, id);
                field.after.i = static_cast<jint>(number);
            } else {
                if (number > LLONG_MAX / 1000) return false;
                field.before.j = env_->GetStaticLongField(owner, id);
                field.after.j = number * 1000;
            }
            if (clearException(name)) return false;
        }
        fields[(*count)++] = field;
        return true;
    }

    void writeField(const PendingField& field, bool restore) {
        const jvalue value = restore ? field.before : field.after;
        if (field.kind == 'I') env_->SetStaticIntField(field.owner, field.id, value.i);
        else if (field.kind == 'J') env_->SetStaticLongField(field.owner, field.id, value.j);
        else env_->SetStaticObjectField(field.owner, field.id, value.l);
    }

    bool fieldMatches(const PendingField& field) {
        if (field.kind == 'I') return env_->GetStaticIntField(field.owner, field.id) == field.after.i;
        if (field.kind == 'J') return env_->GetStaticLongField(field.owner, field.id) == field.after.j;
        jobject current = env_->GetStaticObjectField(field.owner, field.id);
        if (clearException("verify field")) return false;
        const bool same = field.kind == 'S' ?
            stringEqual(static_cast<jstring>(current), static_cast<jstring>(field.after.l)) :
            arrayEqual(static_cast<jobjectArray>(current), static_cast<jobjectArray>(field.after.l));
        if (current) env_->DeleteLocalRef(current);
        return same;
    }

    bool replaceBuildIdentity() {
        jclass build = env_->FindClass("android/os/Build");
        if (clearException("find Build") || !build) return false;
        jclass version = env_->FindClass("android/os/Build$VERSION");
        if (clearException("find VERSION") || !version) return false;
        PendingField fields[80]{};
        size_t count = 0;
        auto property = [](size_t index) { return maskcpu::kRuntimeProfile.values[index]; };
        auto stringValue = [&](size_t index) { return property(index)[0] ? property(index) : "unknown"; };
        for (size_t i = 0; i < maskcpu::kIdentityCount; ++i) {
            if (!queueField(fields, &count, build, maskcpu::kIdentity[i].build, 'S', property(i)) ||
                !queueField(fields, &count, build, maskcpu::kIdentity[i].attestationBuild, 'S', "", true)) return false;
        }
        struct StringField { const char* name; size_t index; };
        constexpr StringField buildStrings[] = {
            {"SOC_MODEL", 10}, {"SOC_MANUFACTURER", 29}, {"FINGERPRINT", 13},
            {"ID", 14}, {"DISPLAY", 15}, {"TYPE", 17}, {"TAGS", 18},
            {"USER", 19}, {"HOST", 20}, {"BOARD", 25}, {"HARDWARE", 27}, {"BOOTLOADER", 28},
        };
        for (const auto& field : buildStrings)
            if (!queueField(fields, &count, build, field.name, 'S', stringValue(field.index))) return false;
        if (!queueField(fields, &count, build, "TIME", 'J', property(21))) return false;
        constexpr StringField versionStrings[] = {
            {"INCREMENTAL", 16}, {"RELEASE", 30}, {"SDK", 32}, {"CODENAME", 35},
        };
        for (const auto& field : versionStrings)
            if (!queueField(fields, &count, version, field.name, 'S', stringValue(field.index))) return false;
        if (!queueField(fields, &count, version, "BASE_OS", 'S', property(34)) ||
            !queueField(fields, &count, version, "SECURITY_PATCH", 'S', property(33)) ||
            !queueField(fields, &count, version, "RELEASE_OR_CODENAME", 'S', stringValue(31), true) ||
            !queueField(fields, &count, version, "RELEASE_OR_PREVIEW_DISPLAY", 'S', stringValue(48), true)) return false;
        constexpr StringField abiFields[] = {
            {"SUPPORTED_ABIS", 39}, {"SUPPORTED_32_BIT_ABIS", 40}, {"SUPPORTED_64_BIT_ABIS", 41},
        };
        for (const auto& field : abiFields)
            if (!queueField(fields, &count, build, field.name, 'A', property(field.index))) return false;
#if defined(__LP64__)
        const char* abis = property(41);
#else
        const char* abis = property(40);
#endif
        if (!abis[0]) return false; // B must support this process's native ABI.
        char first[92]{}, second[92]{};
        const char* comma = strchr(abis, ',');
        const size_t firstLength = comma ? static_cast<size_t>(comma - abis) : strlen(abis);
        memcpy(first, abis, firstLength);
        if (comma) {
            const char* next = comma + 1; const char* end = strchr(next, ',');
            memcpy(second, next, end ? static_cast<size_t>(end - next) : strlen(next));
        }
        if (!queueField(fields, &count, build, "CPU_ABI", 'S', first) ||
            !queueField(fields, &count, build, "CPU_ABI2", 'S', second)) return false;
        const unsigned sdk = static_cast<unsigned>(strtoul(property(32), nullptr, 10));
        int sdkFull = 0;
        if (!maskcpu::parseSdkFull(property(52), sdk, &sdkFull)) return false;
        const char* allCodenames = property(53)[0] ? property(53) : "REL";
        const char* activeCodenames = strncmp(allCodenames, "REL", 3) == 0 &&
            (allCodenames[3] == '\0' || allCodenames[3] == ',') ? "" : allCodenames;
        unsigned activeCount = activeCodenames[0] ? 1 : 0;
        for (const char* q = activeCodenames; *q; ++q) if (*q == ',') ++activeCount;
        if (sdk + activeCount >= 21474) return false;
        char resources[24]{}, full[24]{}, resourceFull[24]{};
        snprintf(resources, sizeof(resources), "%u", sdk + activeCount);
        snprintf(full, sizeof(full), "%d", sdkFull);
        snprintf(resourceFull, sizeof(resourceFull), "%d", sdkFull + static_cast<int>(activeCount * 100000));
        if (!queueField(fields, &count, version, "ALL_CODENAMES", 'A', allCodenames, true) ||
            !queueField(fields, &count, version, "ACTIVE_CODENAMES", 'A', activeCodenames, true) ||
            !queueField(fields, &count, version, "PREVIEW_SDK_FINGERPRINT", 'S', property(54)[0] ? property(54) : "REL", true) ||
            !queueField(fields, &count, version, "SDK_INT_FULL", 'I', full, true) ||
            !queueField(fields, &count, version, "RESOURCES_SDK_INT_FULL", 'I', resourceFull, true)) return false;
        if (!queueField(fields, &count, version, "PREVIEW_SDK_INT", 'I', property(36)) ||
            !queueField(fields, &count, version, "DEVICE_INITIAL_SDK_INT", 'I', property(37), true) ||
            !queueField(fields, &count, version, "MIN_SUPPORTED_TARGET_SDK_INT", 'I', property(38), true) ||
            !queueField(fields, &count, version, "RESOURCES_SDK_INT", 'I', resources, true) ||
            !queueField(fields, &count, version, "SDK_INT", 'I', property(32))) return false;
        // All field IDs, originals and replacements exist before the first write.
        for (size_t i = 0; i < count; ++i) {
            writeField(fields[i], false);
            if (clearException("write B view") || !fieldMatches(fields[i]) || clearException("verify B view")) {
                for (size_t j = i + 1; j > 0; --j) {
                    writeField(fields[j - 1], true); clearException("restore Build");
                }
                return false;
            }
        }
        __android_log_print(ANDROID_LOG_INFO, kLogTag,
                            "B Java view verified fields=%zu fingerprint=%s sdk=%s (framework unchanged)",
                            count, property(13), property(32));
        return true;
    }

    zygisk::Api* api_ = nullptr;
    JNIEnv* env_ = nullptr;
    bool isTarget_ = false;
    bool viewReady_ = false;
    void* pendingState_ = nullptr;
};

}  // namespace

REGISTER_ZYGISK_MODULE(MaskCpuModule)
