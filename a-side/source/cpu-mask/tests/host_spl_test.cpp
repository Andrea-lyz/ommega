// SPDX-License-Identifier: MIT
// Production planning/native policy and JNI cache paths against memory-only mocks.
// Never call prepare/finalizePropertyView, companionWork, childGetprop or mount APIs.
#include <assert.h>
#include <map>
#include <memory>
#include <string>
#include <vector>
#include "zygisk.hpp"
#include "../src/property_view.cpp"
// Expose only the existing Java transaction to the test, not a runtime test API.
#define private public
#include "../src/main.cpp"
#undef private

namespace mock {
constexpr char kSpl[] = "ro.build.version.security_patch";
std::map<std::string, prop_info> properties;
unsigned gets = 0, finds = 0, reads = 0, callbacks = 0;
bool badRead = false, badCallback = false;

void put(const char* name, const char* value) {
    auto& record = properties[name];
    record = {};
    strcpy(record.name, name);
    strcpy(record.value, value);
    record.serial = static_cast<uint32_t>(strlen(value)) << 24;
}

void profile(const char* host, const char* target) {
    properties.clear();
    for (size_t i = 0; i < maskcpu::kProfileCount; ++i) {
        strcpy(maskcpu::kRuntimeProfile.values[i], i >= 5 && i < 10 ? "" : "B-value");
        put(maskcpu::kProfileKeys[i], "A-value");
    }
    for (const auto& item : std::map<size_t, const char*>{
        {21, "1765979644"}, {32, "36"}, {33, target}, {36, "0"}, {37, "31"}, {38, "23"},
        {39, "arm64-v8a"}, {40, ""}, {41, "arm64-v8a"}, {52, "36.1"}, {53, "REL"}, {54, ""}})
        strcpy(maskcpu::kRuntimeProfile.values[item.first], item.second);
    put(kSpl, host);
    maskcpu::captureHostIdentity();
}

struct String : _jstring { std::string text; };
struct Array : _jobjectArray { std::vector<jobject> values; };
struct Class : _jclass { std::string name; };
struct Field { std::string name; std::string signature; jvalue value{}; };
std::map<std::string, Class> classes;
std::map<std::string, Field> fields;
std::vector<std::unique_ptr<String>> strings;
std::vector<std::unique_ptr<Array>> arrays;
std::string javaSpl, failWrite;
unsigned splWrites = 0, writes = 0;
bool pendingException = false, oldAndroid = false, missingSpl = false, failSplReadAfterWrites = false;
_jthrowable exception;

jstring newString(JNIEnv*, const char* text) {
    auto value = std::make_unique<String>();
    value->text = text;
    auto result = value.get();
    strings.push_back(std::move(value));
    return result;
}
jclass findClass(JNIEnv*, const char* name) {
    auto& owner = classes[name]; owner.name = name; return &owner;
}
jfieldID fieldId(JNIEnv* env, jclass owner, const char* name, const char* signature) {
    const bool optionalAbsent = oldAndroid &&
        (strstr(name, "FOR_ATTESTATION") || !strcmp(name, "RELEASE_OR_CODENAME") ||
         !strcmp(name, "RELEASE_OR_PREVIEW_DISPLAY") || !strcmp(name, "SDK_INT_FULL") ||
         !strcmp(name, "RESOURCES_SDK_INT_FULL"));
    if (optionalAbsent || (missingSpl && !strcmp(name, "SECURITY_PATCH"))) {
        pendingException = true; return nullptr;
    }
    const std::string key = static_cast<Class*>(owner)->name + "." + name;
    auto found = fields.find(key);
    if (found == fields.end()) {
        Field field; field.name = name; field.signature = signature;
        if (!strcmp(signature, "Ljava/lang/String;"))
            field.value.l = newString(env, !strcmp(name, "SECURITY_PATCH") ? javaSpl.c_str() : "A-cache");
        else if (!strcmp(signature, "[Ljava/lang/String;")) {
            auto array = std::make_unique<Array>();
            field.value.l = array.get(); arrays.push_back(std::move(array));
        } else if (!strcmp(signature, "I")) field.value.i = 31;
        else field.value.j = 1000;
        found = fields.emplace(key, std::move(field)).first;
    }
    return reinterpret_cast<jfieldID>(&found->second);
}
Field& field(jfieldID id) { return *reinterpret_cast<Field*>(id); }
bool fail(Field& value) {
    ++writes;
    if (value.name == "SECURITY_PATCH") ++splWrites;
    if (value.name != failWrite) return false;
    failWrite.clear(); pendingException = true; return true;
}
JNINativeInterface table{};
JNIEnv env{&table};
void java(const char* patch, bool old = false) {
    fields.clear(); classes.clear(); strings.clear(); arrays.clear();
    javaSpl = patch; oldAndroid = old; missingSpl = false; failWrite.clear();
    splWrites = writes = 0; pendingException = failSplReadAfterWrites = false;
    table = {};
    table.FindClass = findClass;
    table.GetStaticFieldID = fieldId;
    table.NewStringUTF = newString;
    table.GetStringUTFChars = [](JNIEnv*, jstring value, jboolean*) { return static_cast<String*>(value)->text.c_str(); };
    table.ReleaseStringUTFChars = [](JNIEnv*, jstring, const char*) {};
    table.DeleteLocalRef = [](JNIEnv*, jobject) {};
    table.ExceptionCheck = [](JNIEnv*) -> jboolean { return pendingException; };
    table.ExceptionOccurred = [](JNIEnv*) -> jthrowable { return pendingException ? &exception : nullptr; };
    table.ExceptionClear = [](JNIEnv*) { pendingException = false; };
    table.IsInstanceOf = [](JNIEnv*, jobject value, jclass owner) -> jboolean {
        return value == &exception && static_cast<Class*>(owner)->name == "java/lang/NoSuchFieldError";
    };
    table.GetStaticObjectField = [](JNIEnv*, jclass, jfieldID id) -> jobject {
        if (field(id).name == "SECURITY_PATCH" && failSplReadAfterWrites && writes) return nullptr;
        return field(id).value.l;
    };
    table.GetStaticIntField = [](JNIEnv*, jclass, jfieldID id) { return field(id).value.i; };
    table.GetStaticLongField = [](JNIEnv*, jclass, jfieldID id) { return field(id).value.j; };
    table.SetStaticObjectField = [](JNIEnv*, jclass, jfieldID id, jobject value) {
        if (!fail(field(id))) field(id).value.l = value;
    };
    table.SetStaticIntField = [](JNIEnv*, jclass, jfieldID id, jint value) {
        if (!fail(field(id))) field(id).value.i = value;
    };
    table.SetStaticLongField = [](JNIEnv*, jclass, jfieldID id, jlong value) {
        if (!fail(field(id))) field(id).value.j = value;
    };
    table.NewObjectArray = [](JNIEnv*, jsize count, jclass, jobject initial) -> jobjectArray {
        auto value = std::make_unique<Array>(); value->values.resize(count, initial);
        auto result = value.get(); arrays.push_back(std::move(value)); return result;
    };
    table.GetArrayLength = [](JNIEnv*, jarray value) { return static_cast<jsize>(static_cast<Array*>(value)->values.size()); };
    table.GetObjectArrayElement = [](JNIEnv*, jobjectArray value, jsize index) { return static_cast<Array*>(value)->values.at(index); };
    table.SetObjectArrayElement = [](JNIEnv*, jobjectArray value, jsize index, jobject element) {
        static_cast<Array*>(value)->values.at(index) = element;
    };
}
std::string text(const char* owner, const char* name) {
    const auto id = fieldId(&env, findClass(&env, owner), name, "Ljava/lang/String;");
    return static_cast<String*>(field(id).value.l)->text;
}
} // namespace mock

extern "C" int __system_property_get(const char* name, char* value) {
    if (!strcmp(name, mock::kSpl)) ++mock::gets;
    const auto found = mock::properties.find(name);
    strcpy(value, found == mock::properties.end() ? "" : found->second.value);
    return static_cast<int>(strlen(value));
}
extern "C" const prop_info* __system_property_find(const char* name) {
    if (!strcmp(name, mock::kSpl)) ++mock::finds;
    const auto found = mock::properties.find(name);
    return found == mock::properties.end() ? nullptr : &found->second;
}
extern "C" int __system_property_read(const prop_info* record, char* name, char* value) {
    if (!strcmp(record->name, mock::kSpl)) ++mock::reads;
    strcpy(name, record->name); strcpy(value, mock::badRead ? "B-SPL" : record->value);
    return static_cast<int>(strlen(value));
}
extern "C" void __system_property_read_callback(const prop_info* record,
        void (*callback)(void*, const char*, const char*, uint32_t), void* cookie) {
    if (!strcmp(record->name, mock::kSpl)) ++mock::callbacks;
    callback(cookie, record->name, mock::badCallback ? "B-SPL" : record->value, record->serial);
}
extern "C" int __system_property_foreach(void (*callback)(const prop_info*, void*), void* cookie) {
    for (const auto& entry : mock::properties) callback(&entry.second, cookie);
    return 0;
}

static void nativePolicy(const char* host, const char* target) {
    using namespace maskcpu;
    mock::profile(host, target);
    const auto* patch = directProperty(mock::kSpl);
    assert(patch && patch->value == patch->hostValue && !strcmp(patch->value, host));
    assert(!strcmp(kRuntimeProfile.values[33], target));
    auto plan = std::make_unique<Plan>();
    const auto* record = __system_property_find(mock::kSpl);
    PropertyContext context{plan.get(), record};
    planProperty(&context, record->name, record->value, record->serial);
    assert(!plan->error && plan->count == 0 && plan->patches == 0);
    assert(plan->directFound[23]);
    for (size_t i = 0; i < kDirectCount; ++i) assert(plan->directFound[i] == (i == 23));
    bool uncovered = true;
    assert(!experimentHostError(mock::kSpl, host, &uncovered) && !uncovered);
    assert(experimentHostError(mock::kSpl, target, &uncovered));
    assert(verifyPolicy(patch->value, host, true));
    assert(!verifyPolicy(patch->value, target, true));
    assert(!verifyPolicy(patch->value, nullptr, true));
    if (host[0]) assert(!verifyPolicy(patch->value, "", true));
    mock::gets = mock::finds = mock::reads = mock::callbacks = 0;
    assert(verifyNativeKey(mock::kSpl, patch->value));
    assert(mock::gets && mock::finds && mock::reads && mock::callbacks);
    mock::badRead = true; assert(!verifyNativeKey(mock::kSpl, patch->value)); mock::badRead = false;
    mock::badCallback = true; assert(!verifyNativeKey(mock::kSpl, patch->value)); mock::badCallback = false;
    AliasCheck aliases{};
    checkAlias(&aliases, mock::kSpl, host, record->serial);
    assert(aliases.mismatches == 0);
    checkAlias(&aliases, mock::kSpl, target, record->serial);
    assert(aliases.mismatches == 1);
    char listing[256], value[PROP_VALUE_MAX];
    snprintf(listing, sizeof(listing), "[%s]: [%s]\n", mock::kSpl, host);
    assert(listingValue(listing, mock::kSpl, value, sizeof(value), true));
    assert(verifyPolicy(patch->value, value, patch->value == patch->hostValue));
    snprintf(listing, sizeof(listing), "[%s]: [%s]\n", mock::kSpl, target);
    assert(listingValue(listing, mock::kSpl, value, sizeof(value), true));
    assert(!verifyPolicy(patch->value, value, patch->value == patch->hostValue));
    assert(listingValue("", mock::kSpl, value, sizeof(value), true));
    assert(verifyPolicy(patch->value, value, true) == !host[0]);
    mock::properties.erase(mock::kSpl);
    assert(verifyNativeKey(mock::kSpl, patch->value) == !host[0]);
    // The existing optional-B absence policy is unchanged for other fields.
    assert(verifyPolicy("B-optional", ""));
    assert(verifyPolicy("", "A-optional"));
}

static void javaPolicy(const char* host, const char* target, bool old) {
    mock::profile(host, target);
    mock::java(host, old);
    MaskCpuModule module; module.onLoad(nullptr, &mock::env);
    assert(module.replaceBuildIdentity());
    assert(mock::splWrites == 0 && mock::writes > 30);
    assert(mock::text("android/os/Build$VERSION", "SECURITY_PATCH") == host);
    char native[PROP_VALUE_MAX]{};
    __system_property_get(mock::kSpl, native);
    assert(mock::text("android/os/Build$VERSION", "SECURITY_PATCH") == native);
    assert(!strcmp(maskcpu::kRuntimeProfile.values[33], target));
    assert(mock::text("android/os/Build", "MODEL") == "B-value");
    assert(mock::text("android/os/Build", "SOC_MODEL") == "B-value");
    assert(mock::text("android/os/Build$VERSION", "BASE_OS") == "B-value");
    assert(mock::fields.at("android/os/Build$VERSION.SDK_INT").value.i == 36);
    mock::java(target);
    assert(!module.replaceBuildIdentity());
    assert(mock::splWrites == 0 && mock::writes == 0); // Cached/native disagreement is not rewritten.
    mock::java(host); mock::missingSpl = true;
    assert(!module.replaceBuildIdentity());
    assert(mock::splWrites == 0 && mock::writes == 0);
    mock::java(host); mock::failWrite = "SDK_INT";
    assert(!module.replaceBuildIdentity());
    assert(mock::splWrites == 0 && mock::writes > 30);
    assert(mock::text("android/os/Build$VERSION", "SECURITY_PATCH") == host);
    assert(mock::text("android/os/Build", "MODEL") == "A-cache");
    mock::java(host); mock::failSplReadAfterWrites = true;
    assert(!module.replaceBuildIdentity());
    assert(mock::splWrites == 0 && mock::writes > 30);
    assert(mock::text("android/os/Build$VERSION", "SECURITY_PATCH") == host);
    assert(mock::text("android/os/Build", "MODEL") == "A-cache");
}

static void sharedRecordArea() {
    using namespace maskcpu;
    // A snapshot also containing a patched B model retains the entire A SPL record.
    prop_info area[2]{};
    strcpy(area[0].name, "ro.product.model"); strcpy(area[0].value, "A-model");
    strcpy(area[1].name, mock::kSpl); strcpy(area[1].value, "2024-02-05");
    for (auto& record : area) record.serial = static_cast<uint32_t>(strlen(record.value)) << 24;
    const auto saved = area[1];
    PropertyPatch model{};
    model.serial = area[0].serial;
    strcpy(model.name, area[0].name); strcpy(model.expected, area[0].value);
    strcpy(model.replacement, "B-different-model");
    assert(patchProperties(reinterpret_cast<char*>(area), sizeof(area), &model, 1));
    assert(!strcmp(area[0].value, model.replacement));
    assert(!memcmp(&saved, &area[1], sizeof(saved)));
}

int main() {
    nativePolicy("2024-02-05", "2026-10-05");
    nativePolicy("2026-10-05", "2024-02-05");
    nativePolicy("", "2026-10-05");
    nativePolicy("2024-02-05", "");
    sharedRecordArea();
    javaPolicy("2024-02-05", "2026-10-05", false);
    javaPolicy("2026-10-05", "2024-02-05", true);
    javaPolicy("", "2026-10-05", false);
    javaPolicy("2024-02-05", "", false);
    puts("host SPL planning/directFound/native APIs/child-listing policy/Java transaction mocks passed");
}
