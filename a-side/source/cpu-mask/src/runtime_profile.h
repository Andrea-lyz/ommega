// SPDX-License-Identifier: MIT
#pragma once
#include "identity_profile.h"

namespace maskcpu {
// Buffers are consumed while parsing. No network or device service calls.
bool parseConfiguredScope(char* contents, const char* package);
bool parseRuntimeProfile(char* contents, RuntimeProfile* profile);
int loadConfiguredProfile(const char* package, RuntimeProfile* profile);
}  // namespace maskcpu
