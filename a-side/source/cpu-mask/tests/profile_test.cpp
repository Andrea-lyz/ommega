// SPDX-License-Identifier: MIT
// Pure memory/config checks. No namespace, process, service, or device calls.
#include "../src/property_rules.h"
#include "../src/runtime_profile.h"
#include <assert.h>
#include <initializer_list>
#include <stdio.h>

int main() {
    char scope[] = "version=1\nenabled=true\npackage=com.first.app\npackage=com.second.app\n";
    assert(maskcpu::parseConfiguredScope(scope, "com.second.app"));
    char disabled[] = "version=1\nenabled=false\npackage=com.second.app\n";
    assert(!maskcpu::parseConfiguredScope(disabled, "com.second.app"));
    char other[] = "version=1\nenabled=true\npackage=com.first.app\n";
    assert(!maskcpu::parseConfiguredScope(other, "com.second.app"));
    char malformed[] = "version=1\nenabled=true\npackage=../data\n";
    assert(!maskcpu::parseConfiguredScope(malformed, "com.second.app"));
    char profile[] = "version=1\nro.product.brand=Brand B\nro.product.device=deviceB\n"
        "ro.product.name=productB\nro.product.manufacturer=Manufacturer B\nro.product.model=B\n"
        "ro.product.brand_for_attestation=\nro.product.device_for_attestation=\n"
        "ro.product.name_for_attestation=\nro.product.manufacturer_for_attestation=\n"
        "ro.product.model_for_attestation=\nro.soc.model=SOC-B\nro.product.oplus.cpuinfo=\n"
        "ro.build.device_family=family-B\n";
    maskcpu::RuntimeProfile parsed{};
    assert(maskcpu::parseRuntimeProfile(profile, &parsed));
    assert(!strcmp(parsed.values[4], "B"));
    assert(!strcmp(parsed.values[10], "SOC-B"));
    char incomplete[] = "version=1\nro.soc.model=SOC-B\n";
    assert(!maskcpu::parseRuntimeProfile(incomplete, &parsed));
    for (const char* replacement : {"B", "a-much-longer-B-device-model"}) {
        char bytes[512]{};
        maskcpu::PropertyPatch patch{};
        patch.recordOffset = 128;
        patch.serial = 5u << 24;
        strcpy(patch.name, "ro.product.model");
        strcpy(patch.expected, "hostA");
        strcpy(patch.replacement, replacement);
        memcpy(bytes + 128, &patch.serial, 4);
        strcpy(bytes + 132, patch.expected);
        strcpy(bytes + 128 + maskcpu::kPropertyNameOffset, patch.name);
        assert(maskcpu::patchProperties(bytes, sizeof(bytes), &patch, 1));
        assert(!strcmp(bytes + 132, replacement));
        uint32_t serial = 0; memcpy(&serial, bytes + 128, 4);
        assert(serial >> 24 == strlen(replacement));
        assert(!strcmp(bytes + 128 + maskcpu::kPropertyNameOffset, patch.name));
        const char saved = bytes[132];
        assert(!maskcpu::patchProperties(bytes, sizeof(bytes), &patch, 1));
        assert(bytes[132] == saved);
    }
    puts("exp6 scope/profile and variable-length readonly-record checks passed");
}
