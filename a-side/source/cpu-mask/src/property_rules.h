// SPDX-License-Identifier: MIT
#pragma once

#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "scope.h"

namespace maskcpu {

inline constexpr size_t kMaxAreas = 16;
inline constexpr size_t kMaxPatches = 64;
inline constexpr size_t kMaxAreaBytes = 4 * 1024 * 1024;
inline constexpr size_t kPropertyValueBytes = 92;
inline constexpr size_t kPropertyNameOffset = 96;
inline constexpr uint32_t kLongPropertyFlag = 1u << 16;

// The fixed short prop_info layout is checked against callback pointers at runtime.
// Names/trie offsets never move; only the 92-byte value union and serial change.
struct PropertyPatch {
    uint64_t recordOffset;
    uint32_t serial;
    char name[128];
    char expected[kPropertyValueBytes];
    char replacement[kPropertyValueBytes];
};

inline bool patchProperties(char* bytes, size_t size, const PropertyPatch* patches, size_t count) {
    if (!bytes || !patches || count == 0 || count > kMaxPatches) return false;
    for (size_t i = 0; i < count; ++i) {
        const auto& patch = patches[i];
        const size_t nameLength = strnlen(patch.name, sizeof(patch.name));
        const size_t oldLength = strnlen(patch.expected, sizeof(patch.expected));
        const size_t newLength = strnlen(patch.replacement, sizeof(patch.replacement));
        if (nameLength == sizeof(patch.name) || strncmp(patch.name, "ro.", 3) != 0 ||
            oldLength >= kPropertyValueBytes || newLength >= kPropertyValueBytes ||
            (patch.serial & (kLongPropertyFlag | 1u)) != 0 ||
            (patch.serial >> 24) != oldLength || patch.recordOffset % 4 != 0 ||
            patch.recordOffset > size || size - static_cast<size_t>(patch.recordOffset) <
                kPropertyNameOffset + nameLength + 1) return false;
        uint32_t actualSerial;
        memcpy(&actualSerial, bytes + patch.recordOffset, sizeof(actualSerial));
        if (actualSerial != patch.serial ||
            memcmp(bytes + patch.recordOffset + 4, patch.expected, oldLength + 1) != 0 ||
            memcmp(bytes + patch.recordOffset + kPropertyNameOffset, patch.name, nameLength + 1) != 0)
            return false;
        for (size_t j = 0; j < i; ++j) {
            const uint64_t start = patch.recordOffset;
            const uint64_t end = start + kPropertyNameOffset + nameLength + 1;
            const uint64_t otherStart = patches[j].recordOffset;
            const uint64_t otherEnd = otherStart + kPropertyNameOffset + strlen(patches[j].name) + 1;
            if (start < otherEnd && otherStart < end) return false;
        }
    }
    for (size_t i = 0; i < count; ++i) {
        const auto& patch = patches[i];
        const size_t length = strlen(patch.replacement);
        memset(bytes + patch.recordOffset + 4, 0, kPropertyValueBytes);
        memcpy(bytes + patch.recordOffset + 4, patch.replacement, length);
        const uint32_t serial = (patch.serial & 0x00ffffffu) | (static_cast<uint32_t>(length) << 24);
        memcpy(bytes + patch.recordOffset, &serial, sizeof(serial));
    }
    return true;
}

inline bool isPropertyAreaPath(const char* path) {
    constexpr char prefix[] = "/dev/__properties__/";
    if (path == nullptr || strncmp(path, prefix, sizeof(prefix) - 1) != 0) {
        return false;
    }
    const char* leaf = path + sizeof(prefix) - 1;
    return strncmp(leaf, "u:object_r:", 11) == 0 &&
           strchr(leaf, '/') == nullptr && strchr(leaf, ' ') == nullptr &&
           strchr(leaf, '\n') == nullptr && strchr(leaf, '\r') == nullptr;
}

}  // namespace maskcpu
