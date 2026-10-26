// SPDX-License-Identifier: MIT
#include <android/log.h>
#include <unistd.h>

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
        const jint frameResult = env_->PushLocalFrame(32);
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

    bool readBuildField(jclass build, jfieldID field, char* output, size_t capacity) {
        auto value = static_cast<jstring>(env_->GetStaticObjectField(build, field));
        if (clearException("read Build field")) return false;
        if (!value) { output[0] = '\0'; return true; }
        const char* text = env_->GetStringUTFChars(value, nullptr);
        if (clearException("decode Build field") || !text) return false;
        const bool fits = strlen(text) < capacity;
        if (fits) strcpy(output, text);
        env_->ReleaseStringUTFChars(value, text);
        env_->DeleteLocalRef(value);
        return fits;
    }

    bool replaceBuildIdentity() {
        jclass build = env_->FindClass("android/os/Build");
        if (clearException("find Build") || !build) return false;
        jfieldID fields[maskcpu::kIdentityCount]{};
        jfieldID attestFields[maskcpu::kIdentityCount]{};
        jstring replacements[maskcpu::kIdentityCount]{};
        for (size_t i = 0; i < maskcpu::kIdentityCount; ++i) {
            fields[i] = env_->GetStaticFieldID(build, maskcpu::kIdentity[i].build, "Ljava/lang/String;");
            if (clearException(maskcpu::kIdentity[i].build) || !fields[i]) return false;
            attestFields[i] = env_->GetStaticFieldID(build, maskcpu::kIdentity[i].attestationBuild, "Ljava/lang/String;");
            if (clearException(maskcpu::kIdentity[i].attestationBuild) || !attestFields[i]) return false;
            replacements[i] = env_->NewStringUTF(maskcpu::kIdentity[i].value);
            if (clearException("allocate identity string") || !replacements[i]) return false;
        }
        jfieldID socField = env_->GetStaticFieldID(build, "SOC_MODEL", "Ljava/lang/String;");
        if (clearException("find SOC_MODEL") || !socField) return false;
        jstring empty = env_->NewStringUTF("");
        if (clearException("allocate empty attestation value") || !empty) return false;
        jstring soc = env_->NewStringUTF(maskcpu::kSocModel);
        if (clearException("allocate SOC_MODEL") || !soc) return false;

        // All fields/strings are resolved before any cache write. Empty dedicated
        // fields force the Android 17 KeyStore fallback to the ordinary B tuple.
        for (size_t i = 0; i < maskcpu::kIdentityCount; ++i) {
            env_->SetStaticObjectField(build, fields[i], replacements[i]);
            if (clearException(maskcpu::kIdentity[i].build)) return false;
            env_->SetStaticObjectField(build, attestFields[i], empty);
            if (clearException(maskcpu::kIdentity[i].attestationBuild)) return false;
        }
        env_->SetStaticObjectField(build, socField, soc);
        if (clearException("set SOC_MODEL")) return false;

        char ordinary[maskcpu::kIdentityCount][92]{};
        char attest[maskcpu::kIdentityCount][92]{};
        const char* effective[maskcpu::kIdentityCount]{};
        bool verified = true;
        for (size_t i = 0; i < maskcpu::kIdentityCount; ++i) {
            if (!readBuildField(build, fields[i], ordinary[i], sizeof(ordinary[i])) ||
                !readBuildField(build, attestFields[i], attest[i], sizeof(attest[i]))) return false;
            effective[i] = maskcpu::effectiveRequestValue(attest[i], ordinary[i]);
            verified &= strcmp(ordinary[i], maskcpu::kIdentity[i].value) == 0 &&
                attest[i][0] == '\0' && strcmp(effective[i], maskcpu::kIdentity[i].value) == 0;
        }
        char actualSoc[92]{};
        if (!readBuildField(build, socField, actualSoc, sizeof(actualSoc))) return false;
        verified &= strcmp(actualSoc, maskcpu::kSocModel) == 0;
        __android_log_print(ANDROID_LOG_INFO, kLogTag, "Build tuple=%s/%s/%s/%s/%s SOC_MODEL=%s",
                            ordinary[0], ordinary[1], ordinary[2], ordinary[3], ordinary[4], actualSoc);
        __android_log_print(ANDROID_LOG_INFO, kLogTag, "FOR_ATTESTATION tuple=%s/%s/%s/%s/%s",
                            attest[0][0] ? attest[0] : "<empty>", attest[1][0] ? attest[1] : "<empty>",
                            attest[2][0] ? attest[2] : "<empty>", attest[3][0] ? attest[3] : "<empty>",
                            attest[4][0] ? attest[4] : "<empty>");
        __android_log_print(verified ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kLogTag,
                            "effective request VIEW=%s/%s/%s/%s/%s verified=%s (calculated; no keygen)",
                            effective[0], effective[1], effective[2], effective[3], effective[4],
                            verified ? "yes" : "no");
        return verified;
    }

    zygisk::Api* api_ = nullptr;
    JNIEnv* env_ = nullptr;
    bool isTarget_ = false;
    bool viewReady_ = false;
    void* pendingState_ = nullptr;
};

}  // namespace

REGISTER_ZYGISK_MODULE(MaskCpuModule)
