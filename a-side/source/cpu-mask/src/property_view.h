// SPDX-License-Identifier: MIT
#pragma once

#include <android/log.h>
#include <sys/types.h>

#include "zygisk.hpp"

// In-app details follow the runtime diagnostics switch; companion logs are
// written independently by the root process.
#define MASKCPU_APP_LOG(...) do { if (maskcpu::appDebugLogEnabled()) __android_log_print(__VA_ARGS__); } while (false)

namespace maskcpu {

bool appDebugLogEnabled();
bool preparePropertyView(zygisk::Api* api, int targetUid, const char* package, void** pendingState);
bool finalizePropertyView(void* state);
bool verifyPropertyView();
void propertyCompanion(int socket);

}  // namespace maskcpu
