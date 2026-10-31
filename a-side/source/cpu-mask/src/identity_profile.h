// SPDX-License-Identifier: MIT
#pragma once

#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include "profile_fields.h"

namespace maskcpu {

struct IdentityField {
    const char* build;
    const char* attestationBuild;
    const char* property;
    const char* hostValue;
    const char* value;
};

struct RuntimeProfile { char values[kProfileCount][92]; };
inline RuntimeProfile kRuntimeProfile{};
inline char kHostValues[5 + kDirectCount][92]{};
inline const char* const kSocModel = kRuntimeProfile.values[10];
inline const IdentityField kIdentity[] = {
    {"BRAND", "BRAND_FOR_ATTESTATION", "brand", kHostValues[0], kRuntimeProfile.values[0]},
    {"DEVICE", "DEVICE_FOR_ATTESTATION", "device", kHostValues[1], kRuntimeProfile.values[1]},
    {"PRODUCT", "PRODUCT_FOR_ATTESTATION", "name", kHostValues[2], kRuntimeProfile.values[2]},
    {"MANUFACTURER", "MANUFACTURER_FOR_ATTESTATION", "manufacturer", kHostValues[3], kRuntimeProfile.values[3]},
    {"MODEL", "MODEL_FOR_ATTESTATION", "model", kHostValues[4], kRuntimeProfile.values[4]},
};
inline constexpr size_t kIdentityCount = sizeof(kIdentity) / sizeof(kIdentity[0]);
inline constexpr char kUnknown[] = "unknown";

inline bool emptyOrUnknown(const char* value) {
    return value == nullptr || value[0] == '\0' || strcmp(value, kUnknown) == 0;
}

// Mirrors AndroidKeyStoreKeyPairGeneratorSpi.isPropertyEmptyOrUnknown().
// This computes a view only; it neither generates a key nor calls KeyMint.
inline const char* effectiveRequestValue(const char* attestationValue, const char* buildValue) {
    return emptyOrUnknown(attestationValue) ? buildValue : attestationValue;
}

inline const char* identityPropertyOverride(const char* name) {
    constexpr char prefix[] = "ro.product.";
    if (!name || strncmp(name, prefix, sizeof(prefix) - 1) != 0) return nullptr;
    const char* suffix = name + sizeof(prefix) - 1;
    constexpr const char* partitions[] = {
        "bootimage", "odm", "product", "system", "system_ext", "vendor", "odm_dlkm", "vendor_dlkm"
    };
    for (const auto& field : kIdentity) {
        if (strcmp(suffix, field.property) == 0) return field.value;
        char special[80];
        snprintf(special, sizeof(special), "%s_for_attestation", field.property);
        // B's dedicated properties are empty. The guard refuses nonempty ones;
        // empty/absent records stay untouched, and no properties are inserted.
        if (strcmp(suffix, special) == 0) return "";
        for (const char* partition : partitions) {
            const size_t length = strlen(partition);
            if (strncmp(suffix, partition, length) == 0 && suffix[length] == '.' &&
                strcmp(suffix + length + 1, field.property) == 0) return field.value;
        }
    }
    return nullptr;
}

inline const IdentityField* identityFieldForProperty(const char* name) {
    if (!identityPropertyOverride(name)) return nullptr;
    const char* leaf = strrchr(name, '.') + 1;
    for (const auto& field : kIdentity) {
        char special[80];
        snprintf(special, sizeof(special), "%s_for_attestation", field.property);
        if (strcmp(leaf, field.property) == 0 || strcmp(leaf, special) == 0) return &field;
    }
    return nullptr;
}

struct PropertyField { const char* property; const char* hostValue; const char* value; };
// Exact B values, including empty optional fields; no synthetic defaults here.
inline const PropertyField kDirectProperties[] = {
    {"ro.soc.model", kHostValues[5], kRuntimeProfile.values[10]},
    {"ro.product.oplus.cpuinfo", kHostValues[6], kRuntimeProfile.values[11]},
    {"ro.build.device_family", kHostValues[7], kRuntimeProfile.values[12]},
    {"ro.build.fingerprint", kHostValues[8], kRuntimeProfile.values[13]},
    {"ro.build.id", kHostValues[9], kRuntimeProfile.values[14]},
    {"ro.build.display.id", kHostValues[10], kRuntimeProfile.values[15]},
    {"ro.build.version.incremental", kHostValues[11], kRuntimeProfile.values[16]},
    {"ro.build.type", kHostValues[12], kRuntimeProfile.values[17]},
    {"ro.build.tags", kHostValues[13], kRuntimeProfile.values[18]},
    {"ro.build.user", kHostValues[14], kRuntimeProfile.values[19]},
    {"ro.build.host", kHostValues[15], kRuntimeProfile.values[20]},
    {"ro.build.date.utc", kHostValues[16], kRuntimeProfile.values[21]},
    {"ro.build.flavor", kHostValues[17], kRuntimeProfile.values[22]},
    {"ro.build.product", kHostValues[18], kRuntimeProfile.values[23]},
    {"ro.build.description", kHostValues[19], kRuntimeProfile.values[24]},
    {"ro.product.board", kHostValues[20], kRuntimeProfile.values[25]},
    {"ro.board.platform", kHostValues[21], kRuntimeProfile.values[26]},
    {"ro.hardware", kHostValues[22], kRuntimeProfile.values[27]},
    {"ro.bootloader", kHostValues[23], kRuntimeProfile.values[28]},
    {"ro.soc.manufacturer", kHostValues[24], kRuntimeProfile.values[29]},
    {"ro.build.version.release", kHostValues[25], kRuntimeProfile.values[30]},
    {"ro.build.version.release_or_codename", kHostValues[26], kRuntimeProfile.values[31]},
    {"ro.build.version.sdk", kHostValues[27], kRuntimeProfile.values[32]},
    {"ro.build.version.security_patch", kHostValues[28], kRuntimeProfile.values[33]},
    {"ro.build.version.base_os", kHostValues[29], kRuntimeProfile.values[34]},
    {"ro.build.version.codename", kHostValues[30], kRuntimeProfile.values[35]},
    {"ro.build.version.preview_sdk", kHostValues[31], kRuntimeProfile.values[36]},
    {"ro.product.first_api_level", kHostValues[32], kRuntimeProfile.values[37]},
    {"ro.build.version.min_supported_target_sdk", kHostValues[33], kRuntimeProfile.values[38]},
    {"ro.product.cpu.abilist", kHostValues[34], kRuntimeProfile.values[39]},
    {"ro.product.cpu.abilist32", kHostValues[35], kRuntimeProfile.values[40]},
    {"ro.product.cpu.abilist64", kHostValues[36], kRuntimeProfile.values[41]},
    {"ro.system.build.fingerprint", kHostValues[37], kRuntimeProfile.values[42]},
    {"ro.vendor.build.fingerprint", kHostValues[38], kRuntimeProfile.values[43]},
    {"ro.odm.build.fingerprint", kHostValues[39], kRuntimeProfile.values[44]},
    {"ro.product.build.fingerprint", kHostValues[40], kRuntimeProfile.values[45]},
    {"ro.system_ext.build.fingerprint", kHostValues[41], kRuntimeProfile.values[46]},
    {"ro.bootimage.build.fingerprint", kHostValues[42], kRuntimeProfile.values[47]},
    {"ro.build.version.release_or_preview_display", kHostValues[43], kRuntimeProfile.values[48]},
    {"ro.product.cpu.abi", kHostValues[44], kRuntimeProfile.values[49]},
    {"ro.product.cpu.abi2", kHostValues[45], kRuntimeProfile.values[50]},
    {"ro.build.characteristics", kHostValues[46], kRuntimeProfile.values[51]},
    {"ro.build.version.sdk_full", kHostValues[47], kRuntimeProfile.values[52]},
    {"ro.build.version.all_codenames", kHostValues[48], kRuntimeProfile.values[53]},
    {"ro.build.version.preview_sdk_fingerprint", kHostValues[49], kRuntimeProfile.values[54]},
    {"ro.product.bootimage.brand", kHostValues[50], kRuntimeProfile.values[55]},
    {"ro.product.bootimage.device", kHostValues[51], kRuntimeProfile.values[56]},
    {"ro.product.bootimage.name", kHostValues[52], kRuntimeProfile.values[57]},
    {"ro.product.bootimage.manufacturer", kHostValues[53], kRuntimeProfile.values[58]},
    {"ro.product.bootimage.model", kHostValues[54], kRuntimeProfile.values[59]},
    {"ro.product.odm.brand", kHostValues[55], kRuntimeProfile.values[60]},
    {"ro.product.odm.device", kHostValues[56], kRuntimeProfile.values[61]},
    {"ro.product.odm.name", kHostValues[57], kRuntimeProfile.values[62]},
    {"ro.product.odm.manufacturer", kHostValues[58], kRuntimeProfile.values[63]},
    {"ro.product.odm.model", kHostValues[59], kRuntimeProfile.values[64]},
    {"ro.product.product.brand", kHostValues[60], kRuntimeProfile.values[65]},
    {"ro.product.product.device", kHostValues[61], kRuntimeProfile.values[66]},
    {"ro.product.product.name", kHostValues[62], kRuntimeProfile.values[67]},
    {"ro.product.product.manufacturer", kHostValues[63], kRuntimeProfile.values[68]},
    {"ro.product.product.model", kHostValues[64], kRuntimeProfile.values[69]},
    {"ro.product.system.brand", kHostValues[65], kRuntimeProfile.values[70]},
    {"ro.product.system.device", kHostValues[66], kRuntimeProfile.values[71]},
    {"ro.product.system.name", kHostValues[67], kRuntimeProfile.values[72]},
    {"ro.product.system.manufacturer", kHostValues[68], kRuntimeProfile.values[73]},
    {"ro.product.system.model", kHostValues[69], kRuntimeProfile.values[74]},
    {"ro.product.system_ext.brand", kHostValues[70], kRuntimeProfile.values[75]},
    {"ro.product.system_ext.device", kHostValues[71], kRuntimeProfile.values[76]},
    {"ro.product.system_ext.name", kHostValues[72], kRuntimeProfile.values[77]},
    {"ro.product.system_ext.manufacturer", kHostValues[73], kRuntimeProfile.values[78]},
    {"ro.product.system_ext.model", kHostValues[74], kRuntimeProfile.values[79]},
    {"ro.product.vendor.brand", kHostValues[75], kRuntimeProfile.values[80]},
    {"ro.product.vendor.device", kHostValues[76], kRuntimeProfile.values[81]},
    {"ro.product.vendor.name", kHostValues[77], kRuntimeProfile.values[82]},
    {"ro.product.vendor.manufacturer", kHostValues[78], kRuntimeProfile.values[83]},
    {"ro.product.vendor.model", kHostValues[79], kRuntimeProfile.values[84]},
    {"ro.product.odm_dlkm.brand", kHostValues[80], kRuntimeProfile.values[85]},
    {"ro.product.odm_dlkm.device", kHostValues[81], kRuntimeProfile.values[86]},
    {"ro.product.odm_dlkm.name", kHostValues[82], kRuntimeProfile.values[87]},
    {"ro.product.odm_dlkm.manufacturer", kHostValues[83], kRuntimeProfile.values[88]},
    {"ro.product.odm_dlkm.model", kHostValues[84], kRuntimeProfile.values[89]},
    {"ro.product.vendor_dlkm.brand", kHostValues[85], kRuntimeProfile.values[90]},
    {"ro.product.vendor_dlkm.device", kHostValues[86], kRuntimeProfile.values[91]},
    {"ro.product.vendor_dlkm.name", kHostValues[87], kRuntimeProfile.values[92]},
    {"ro.product.vendor_dlkm.manufacturer", kHostValues[88], kRuntimeProfile.values[93]},
    {"ro.product.vendor_dlkm.model", kHostValues[89], kRuntimeProfile.values[94]},
};
static_assert(sizeof(kDirectProperties) / sizeof(kDirectProperties[0]) == kDirectCount);
inline const PropertyField* directProperty(const char* name) {
    for (const auto& field : kDirectProperties)
        if (strcmp(name, field.property) == 0) return &field;
    return nullptr;
}

// Retain exp6's dedicated-property and partition guards, using the live A tuple.
inline const char* experimentHostError(const char* name, const char* value, bool* uncovered) {
    *uncovered = false;
    if (const auto* direct = directProperty(name)) {
        return strcmp(value, direct->hostValue) == 0 ? nullptr : "unexpected host property";
    }
    const char* replacement = identityPropertyOverride(name);
    if (replacement) {
        if (replacement[0] == '\0') return value[0] ? "nonempty FOR_ATTESTATION" : nullptr;
        const IdentityField* field = identityFieldForProperty(name);
        char root[80];
        snprintf(root, sizeof(root), "ro.product.%s", field->property);
        if (strcmp(name, root) == 0) {
            return strcmp(value, field->hostValue) == 0 ? nullptr : "unexpected host identity";
        }
        if (emptyOrUnknown(value) || strcmp(value, field->hostValue) == 0 || strcmp(value, field->value) == 0)
            return nullptr;
        if (strncmp(name, "ro.product.vendor.", 18) == 0) return "unexpected vendor identity";
        *uncovered = true;
        return nullptr;
    }
    return nullptr;
}

}  // namespace maskcpu
