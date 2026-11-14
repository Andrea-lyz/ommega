// SPDX-License-Identifier: MIT
#pragma once
#include <stdint.h>
// In-memory short prop_info records for production callback/planning tests.
#define PROP_VALUE_MAX 92
struct prop_info {
    uint32_t serial;
    char value[PROP_VALUE_MAX];
    char name[128];
};
extern "C" {
int __system_property_get(const char*, char*);
const prop_info* __system_property_find(const char*);
int __system_property_read(const prop_info*, char*, char*);
void __system_property_read_callback(const prop_info*, void (*)(void*, const char*, const char*, uint32_t), void*);
int __system_property_foreach(void (*)(const prop_info*, void*), void*);
}
