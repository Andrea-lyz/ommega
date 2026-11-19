// SPDX-License-Identifier: MIT
#pragma once

#include <android/log.h>
#include <sys/types.h>

#include "zygisk.hpp"

// Module-side code runs in the target app's process, and logd lets an app read
// its own entries. Those lines exist only in debug builds; release builds
// compile them out. The root companion logs normally.
#ifdef MASKCPU_DEBUG_LOG
#define MASKCPU_APP_LOG(...) __android_log_print(__VA_ARGS__)
#else
#define MASKCPU_APP_LOG(...) do { if (false) __android_log_print(__VA_ARGS__); } while (false)
#endif

namespace maskcpu {

bool preparePropertyView(zygisk::Api* api, int targetUid, const char* package, void** pendingState);
bool finalizePropertyView(void* state);
bool verifyPropertyView();
void propertyCompanion(int socket);

}  // namespace maskcpu
