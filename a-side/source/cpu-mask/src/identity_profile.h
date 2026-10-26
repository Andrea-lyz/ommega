// SPDX-License-Identifier: MIT
#pragma once

#include <stddef.h>
#include <stdio.h>
#include <string.h>

namespace maskcpu {

struct IdentityField {
    const char* build;
    const char* attestationBuild;
    const char* property;
    const char* hostValue;
    const char* value;
};

struct RuntimeProfile { char values[13][92]; };
inline RuntimeProfile kRuntimeProfile{};
inline char kHostValues[8][92]{};
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

struct CpuAlias { const char* property; const char* hostValue; const char* value; };
inline const CpuAlias kCpuAliases[] = {
    {"ro.soc.model", kHostValues[5], kRuntimeProfile.values[10]},
    {"ro.product.oplus.cpuinfo", kHostValues[6], kRuntimeProfile.values[11]},
    {"ro.build.device_family", kHostValues[7], kRuntimeProfile.values[12]},
};

// Retain exp6's dedicated-property and partition guards, using the live A tuple.
inline const char* experimentHostError(const char* name, const char* value, bool* uncovered) {
    *uncovered = false;
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
    for (const auto& alias : kCpuAliases) {
        if (strcmp(name, alias.property) != 0) continue;
        if (strcmp(value, alias.hostValue) == 0) return nullptr;
        if (strcmp(name, "ro.soc.model") != 0 && strcmp(value, alias.value) == 0) return nullptr;
        return "unexpected host CPU";
    }
    return nullptr;
}

}  // namespace maskcpu
