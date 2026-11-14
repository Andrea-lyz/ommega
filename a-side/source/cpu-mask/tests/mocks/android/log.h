// SPDX-License-Identifier: MIT
#pragma once
// Test-only Android logging boundary; no log daemon or device access.
#define ANDROID_LOG_INFO 4
#define ANDROID_LOG_ERROR 6
inline int __android_log_print(int, const char*, const char*, ...) { return 0; }
