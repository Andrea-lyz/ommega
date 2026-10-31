// SPDX-License-Identifier: MIT
#pragma once
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include <errno.h>

namespace maskcpu {
// Wire order must match common/src/cpu_mask.rs on A and B.
inline constexpr const char* kProfileKeys[] = {
    "ro.product.brand",
    "ro.product.device",
    "ro.product.name",
    "ro.product.manufacturer",
    "ro.product.model",
    "ro.product.brand_for_attestation",
    "ro.product.device_for_attestation",
    "ro.product.name_for_attestation",
    "ro.product.manufacturer_for_attestation",
    "ro.product.model_for_attestation",
    "ro.soc.model",
    "ro.product.oplus.cpuinfo",
    "ro.build.device_family",
    "ro.build.fingerprint",
    "ro.build.id",
    "ro.build.display.id",
    "ro.build.version.incremental",
    "ro.build.type",
    "ro.build.tags",
    "ro.build.user",
    "ro.build.host",
    "ro.build.date.utc",
    "ro.build.flavor",
    "ro.build.product",
    "ro.build.description",
    "ro.product.board",
    "ro.board.platform",
    "ro.hardware",
    "ro.bootloader",
    "ro.soc.manufacturer",
    "ro.build.version.release",
    "ro.build.version.release_or_codename",
    "ro.build.version.sdk",
    "ro.build.version.security_patch",
    "ro.build.version.base_os",
    "ro.build.version.codename",
    "ro.build.version.preview_sdk",
    "ro.product.first_api_level",
    "ro.build.version.min_supported_target_sdk",
    "ro.product.cpu.abilist",
    "ro.product.cpu.abilist32",
    "ro.product.cpu.abilist64",
    "ro.system.build.fingerprint",
    "ro.vendor.build.fingerprint",
    "ro.odm.build.fingerprint",
    "ro.product.build.fingerprint",
    "ro.system_ext.build.fingerprint",
    "ro.bootimage.build.fingerprint",
    "ro.build.version.release_or_preview_display",
    "ro.product.cpu.abi",
    "ro.product.cpu.abi2",
    "ro.build.characteristics",
    "ro.build.version.sdk_full",
    "ro.build.version.all_codenames",
    "ro.build.version.preview_sdk_fingerprint",
    "ro.product.bootimage.brand",
    "ro.product.bootimage.device",
    "ro.product.bootimage.name",
    "ro.product.bootimage.manufacturer",
    "ro.product.bootimage.model",
    "ro.product.odm.brand",
    "ro.product.odm.device",
    "ro.product.odm.name",
    "ro.product.odm.manufacturer",
    "ro.product.odm.model",
    "ro.product.product.brand",
    "ro.product.product.device",
    "ro.product.product.name",
    "ro.product.product.manufacturer",
    "ro.product.product.model",
    "ro.product.system.brand",
    "ro.product.system.device",
    "ro.product.system.name",
    "ro.product.system.manufacturer",
    "ro.product.system.model",
    "ro.product.system_ext.brand",
    "ro.product.system_ext.device",
    "ro.product.system_ext.name",
    "ro.product.system_ext.manufacturer",
    "ro.product.system_ext.model",
    "ro.product.vendor.brand",
    "ro.product.vendor.device",
    "ro.product.vendor.name",
    "ro.product.vendor.manufacturer",
    "ro.product.vendor.model",
    "ro.product.odm_dlkm.brand",
    "ro.product.odm_dlkm.device",
    "ro.product.odm_dlkm.name",
    "ro.product.odm_dlkm.manufacturer",
    "ro.product.odm_dlkm.model",
    "ro.product.vendor_dlkm.brand",
    "ro.product.vendor_dlkm.device",
    "ro.product.vendor_dlkm.name",
    "ro.product.vendor_dlkm.manufacturer",
    "ro.product.vendor_dlkm.model",
};
inline constexpr size_t kMaxProfileBytes = 16 * 1024;
inline constexpr size_t kProfileCount = sizeof(kProfileKeys) / sizeof(kProfileKeys[0]);
inline constexpr size_t kDirectCount = kProfileCount - 10;
inline bool profileFieldRequired(size_t index) {
    return index < 5 || index == 10 || index == 13 || index == 14 ||
           index == 30 || index == 32 || index == 39;
}
// Same major.minor range as Android Build.parseFullVersion().
inline bool parseSdkFull(const char* text, unsigned sdk, int* result) {
    if (!text || !result || sdk >= 21474) return false;
    if (!text[0]) { *result = static_cast<int>(sdk * 100000); return true; }
    for (const char* p = text; *p; ++p)
        if ((*p < '0' || *p > '9') && *p != '.') return false;
    errno = 0; char* end = nullptr;
    const unsigned long major = strtoul(text, &end, 10);
    if (errno || end == text || major != sdk) return false;
    unsigned long minor = 0;
    if (*end == '.') {
        const char* start = end + 1;
        minor = strtoul(start, &end, 10);
        if (errno || end == start) return false;
    }
    if (*end || minor >= 100000) return false;
    *result = static_cast<int>(major * 100000 + minor);
    return true;
}
}  // namespace maskcpu
