// SPDX-License-Identifier: MIT
// Pure contract checks. No Binder, namespace, service, network or device calls.
#include "../src/property_rules.h"
#include "../src/runtime_profile.h"
#include <assert.h>
#include <initializer_list>
#include <stdio.h>
#include <string>

static std::string profileText(const char* overrideKey = nullptr, const char* overrideValue = nullptr) {
    std::string text = "version=2\n";
    for (size_t i = 0; i < maskcpu::kProfileCount; ++i) {
        const char* value = (i >= 5 && i < 10) ? "" : "B-value";
        if (i == 21) value = "1765979644";
        if (i == 32) value = "36";
        if (i == 52) value = "36.1";
        if (i == 36 || i == 37 || i == 38) value = "0";
        if (overrideKey && !strcmp(overrideKey, maskcpu::kProfileKeys[i])) value = overrideValue;
        text += maskcpu::kProfileKeys[i]; text += '='; text += value; text += '\n';
    }
    return text;
}
static bool parse(std::string text, maskcpu::RuntimeProfile* result) {
    return maskcpu::parseRuntimeProfile(text.data(), result);
}
int main() {
    char scope[] = "version=1\nenabled=true\npackage=com.first.app\npackage=com.second.app\n";
    assert(maskcpu::parseConfiguredScope(scope, "com.second.app"));
    char disabled[] = "version=1\nenabled=false\npackage=com.second.app\n";
    assert(!maskcpu::parseConfiguredScope(disabled, "com.second.app"));
    char other[] = "version=1\nenabled=true\npackage=com.first.app\n";
    assert(!maskcpu::parseConfiguredScope(other, "com.second.app"));
    char malformed[] = "version=1\nenabled=true\npackage=../data\n";
    assert(!maskcpu::parseConfiguredScope(malformed, "com.second.app"));
    maskcpu::RuntimeProfile parsed{};
    assert(parse(profileText(), &parsed));
    assert(!strcmp(parsed.values[4], "B-value"));
    assert(!strcmp(parsed.values[32], "36"));
    assert(!strcmp(parsed.values[maskcpu::kProfileCount - 1], "B-value"));
    int sdkFull = 0;
    assert(maskcpu::parseSdkFull("36.1", 36, &sdkFull) && sdkFull == 3600001);
    assert(maskcpu::parseSdkFull("", 31, &sdkFull) && sdkFull == 3100000);
    assert(!maskcpu::parseSdkFull("37.0", 36, &sdkFull));
    assert(!maskcpu::parseSdkFull("36.100000", 36, &sdkFull));
    const auto saved = parsed;
    assert(!parse("version=2\nro.soc.model=SOC-B\n", &parsed));
    assert(!memcmp(&saved, &parsed, sizeof(saved))); // Failed parse is atomic.
    auto stale = profileText(); stale.replace(0, 9, "version=1");
    assert(!parse(stale, &parsed));
    assert(!parse(profileText() + "ro.product.model=duplicate\n", &parsed));
    assert(!parse(profileText("ro.build.version.sdk", "-1"), &parsed));
    assert(!parse(profileText("ro.build.version.sdk", "0"), &parsed));
    assert(!parse(profileText("ro.build.version.sdk_full", "37.0"), &parsed));
    assert(!parse(profileText("ro.product.cpu.abilist", "arm64-v8a,"), &parsed));
    assert(!parse(profileText("ro.build.version.sdk", "2147483648"), &parsed));
    assert(!parse(profileText("ro.build.version.preview_sdk", "bad"), &parsed));
    assert(!parse(profileText("ro.build.date.utc", "99999999999999999999"), &parsed));
    assert(!parse(profileText("ro.build.fingerprint", ""), &parsed));
    assert(!parse(profileText("ro.product.model_for_attestation", "nonempty"), &parsed));
    const std::string limit(91, 'b'), oversized(92, 'b');
    assert(parse(profileText("ro.build.fingerprint", limit.c_str()), &parsed));
    assert(!parse(profileText("ro.build.fingerprint", oversized.c_str()), &parsed));
    for (const char* replacement : {"", "B", "a-much-longer-B-device-model"}) {
        char bytes[512]{};
        maskcpu::PropertyPatch patch{};
        patch.recordOffset = 128; patch.serial = 5u << 24;
        strcpy(patch.name, "ro.product.model"); strcpy(patch.expected, "hostA");
        strcpy(patch.replacement, replacement);
        memcpy(bytes + 128, &patch.serial, 4);
        strcpy(bytes + 132, patch.expected);
        strcpy(bytes + 128 + maskcpu::kPropertyNameOffset, patch.name);
        assert(maskcpu::patchProperties(bytes, sizeof(bytes), &patch, 1));
        assert(!strcmp(bytes + 132, replacement));
        uint32_t serial = 0; memcpy(&serial, bytes + 128, 4);
        assert(serial >> 24 == strlen(replacement));
        assert(!strcmp(bytes + 128 + maskcpu::kPropertyNameOffset, patch.name));
        const char first = bytes[132];
        assert(!maskcpu::patchProperties(bytes, sizeof(bytes), &patch, 1));
        assert(bytes[132] == first);
    }
    puts("B identity v2 scope/profile and readonly-record contract checks passed");
}
