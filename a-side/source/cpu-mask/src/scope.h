// SPDX-License-Identifier: MIT
#pragma once

#include <string.h>

namespace maskcpu {

inline char kTargetPackage[256]{};

// app_data_dir comes from Android specialization, not the app's process name.
inline bool packageFromDataDir(const char* appDataDir) noexcept {
    if (appDataDir == nullptr || appDataDir[0] != '/') {
        return false;
    }
    const char* package = strrchr(appDataDir, '/');
    if (!package || strlen(package + 1) >= sizeof(kTargetPackage)) return false;
    strcpy(kTargetPackage, package + 1);
    return true;
}

}  // namespace maskcpu
