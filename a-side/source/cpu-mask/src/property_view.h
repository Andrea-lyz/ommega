// SPDX-License-Identifier: MIT
#pragma once

#include <sys/types.h>

#include "zygisk.hpp"

namespace maskcpu {

bool preparePropertyView(zygisk::Api* api, int targetUid, const char* package, void** pendingState);
bool finalizePropertyView(void* state);
bool verifyPropertyView();
void propertyCompanion(int socket);

}  // namespace maskcpu
