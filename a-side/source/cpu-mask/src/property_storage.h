// SPDX-License-Identifier: MIT
#pragma once

#include <stdint.h>
#include <stddef.h>

#include "property_rules.h"

namespace maskcpu {

struct AreaFile {
    uint64_t device;
    uint64_t inode;
    uint64_t size;
    char path[384];
};

struct Mapping {
    uintptr_t start;
    uintptr_t end;
    uint64_t offset;
    uint64_t inode;
    unsigned deviceMajor;
    unsigned deviceMinor;
    char permissions[5];
    char path[512];
};

bool parseMapping(const char* line, Mapping* mapping);
int copyPatchedSnapshot(int source, int destination, size_t size,
                        const PropertyPatch* properties, size_t propertyCount);
int bindReadOnlySnapshot(int snapshot, const char* source, const char* target);
int bindReadOnlySnapshotSet(const AreaFile* areas, const int* snapshots, size_t count,
                             size_t* failedArea = nullptr);
struct SharedViewStatus {
    uint32_t protocol;
    uint32_t appReady;
    uint32_t done;
    int32_t error;
};
bool waitForSharedFlag(const uint32_t* flag, unsigned timeoutMs);

struct SnapshotOpenStatus {
    size_t opened = 0;
    size_t skipped = 0;
    size_t failedArea = static_cast<size_t>(-1);
    const char* step = "none";
};

// Unmapped areas stay in the private filesystem view for future opens/exec.
// Only inherited mappings require a descriptor in the current security domain.
int openReadOnlySnapshots(const AreaFile* areas, size_t count, int* snapshots,
                          SnapshotOpenStatus* status = nullptr);
int remapSnapshots(const AreaFile* areas, const int* snapshots, size_t count);

}  // namespace maskcpu
