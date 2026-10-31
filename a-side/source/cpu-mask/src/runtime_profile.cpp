// SPDX-License-Identifier: MIT
#include "runtime_profile.h"
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdlib.h>
#include <limits.h>
#include <initializer_list>
#include <sys/stat.h>
#include <unistd.h>

namespace maskcpu {
namespace {


bool validPackage(const char* package) {
    if (!package || !strchr(package, '.') || strlen(package) > 255) return false;
    bool first = true;
    for (const unsigned char* c = reinterpret_cast<const unsigned char*>(package); *c; ++c) {
        const bool letter = (*c >= 'a' && *c <= 'z') || (*c >= 'A' && *c <= 'Z');
        if (*c == '.') { if (first) return false; first = true; continue; }
        if (!letter && (first || !((*c >= '0' && *c <= '9') || *c == '_'))) return false;
        first = false;
    }
    return !first;
}

int readFile(const char* path, char* contents, size_t capacity) {
    const int fd = open(path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
    if (fd < 0) return errno;
    struct stat info{};
    if (fstat(fd, &info) != 0 || !S_ISREG(info.st_mode) || info.st_size < 0 ||
        static_cast<uint64_t>(info.st_size) >= capacity ||
        (info.st_uid != 0 && info.st_uid != 1017) || (info.st_mode & 0022)) {
        close(fd); return EINVAL;
    }
    size_t used = 0;
    while (used < capacity - 1) {
        const ssize_t count = read(fd, contents + used, capacity - used - 1);
        if (count < 0 && errno == EINTR) continue;
        if (count < 0) { const int error = errno; close(fd); return error; }
        if (!count) break;
        used += static_cast<size_t>(count);
    }
    close(fd);
    if (used != static_cast<size_t>(info.st_size) || memchr(contents, 0, used)) return EINVAL;
    contents[used] = 0;
    return 0;
}
}  // namespace

bool parseConfiguredScope(char* contents, const char* package) {
    if (!contents || !validPackage(package)) return false;
    bool version = false, enabled = false, enabledSeen = false, selected = false;
    size_t packages = 0;
    char* save = nullptr;
    for (char* line = strtok_r(contents, "\n", &save); line; line = strtok_r(nullptr, "\n", &save)) {
        char* value = strchr(line, '=');
        if (!value) return false;
        *value++ = 0;
        if (!strcmp(line, "version") && !version && !strcmp(value, "1")) version = true;
        else if (!strcmp(line, "enabled") && !enabledSeen &&
                 (!strcmp(value, "true") || !strcmp(value, "false"))) {
            enabledSeen = true; enabled = !strcmp(value, "true");
        } else if (!strcmp(line, "package") && ++packages <= 4096 && validPackage(value)) {
            if (!strcmp(value, package)) { if (selected) return false; selected = true; }
        } else return false;
    }
    return version && enabledSeen && enabled && selected;
}

bool parseRuntimeProfile(char* contents, RuntimeProfile* profile) {
    if (!contents || !profile) return false;
    RuntimeProfile result{};
    bool seen[kProfileCount]{};
    size_t count = 0;
    bool version = false;
    char* save = nullptr;
    for (char* line = strtok_r(contents, "\n", &save); line; line = strtok_r(nullptr, "\n", &save)) {
        char* value = strchr(line, '=');
        if (!value) return false;
        *value++ = 0;
        if (!strcmp(line, "version") && !version && !strcmp(value, "2")) { version = true; continue; }
        size_t index = 0;
        while (index < kProfileCount && strcmp(line, kProfileKeys[index])) ++index;
        if (index == kProfileCount || seen[index] || strlen(value) >= 92 || strchr(value, '\r')) return false;
        if (profileFieldRequired(index) && !value[0]) return false;
        if (index >= 5 && index < 10 && value[0]) return false;
        strcpy(result.values[index], value);
        seen[index] = true;
        ++count;
    }
    if (!version || count != kProfileCount) return false;
    for (size_t index : {size_t{32}, size_t{36}, size_t{37}, size_t{38}}) {
        const char* number = result.values[index];
        if (!number[0]) { if (index == 32) return false; else continue; }
        for (const char* p = number; *p; ++p) if (*p < '0' || *p > '9') return false;
        errno = 0; char* end = nullptr;
        const unsigned long value = strtoul(number, &end, 10);
        if (errno || !end || *end || value > INT_MAX || (index == 32 && value == 0)) return false;
    }
    const char* seconds = result.values[21];
    if (!seconds[0]) return false;
    for (const char* p = seconds; *p; ++p) if (*p < '0' || *p > '9') return false;
    errno = 0; char* end = nullptr;
    const unsigned long long timestamp = strtoull(seconds, &end, 10);
    if (errno || !end || *end || timestamp > static_cast<unsigned long long>(LLONG_MAX / 1000)) return false;
    int sdkFull = 0;
    if (!parseSdkFull(result.values[52], static_cast<unsigned>(strtoul(result.values[32], nullptr, 10)), &sdkFull)) return false;
    for (size_t index : {size_t{39}, size_t{40}, size_t{41}}) {
        const char* abi = result.values[index];
        if (!abi[0]) continue;
        bool separator = true;
        for (const char* p = abi; *p; ++p) {
            if (*p == ',') { if (separator) return false; separator = true; continue; }
            if (!((*p >= 'a' && *p <= 'z') || (*p >= 'A' && *p <= 'Z') ||
                  (*p >= '0' && *p <= '9') || *p == '-' || *p == '_')) return false;
            separator = false;
        }
        if (separator) return false;
    }
    *profile = result;
    return true;
}

int loadConfiguredProfile(const char* package, RuntimeProfile* profile) {
    // An enabled standalone copy would apply a second private view to the same app.
    struct stat standalone{};
    if (stat("/data/adb/modules/zygisk_maskcpu", &standalone) == 0 &&
        access("/data/adb/modules/zygisk_maskcpu/disable", F_OK) != 0 &&
        access("/data/adb/modules/zygisk_maskcpu/remove", F_OK) != 0) return EBUSY;
    char config[512 * 1024 + 1];
    int error = readFile("/data/misc/keystore/ommega/data/cpu-mask.conf", config, sizeof(config));
    if (error) return error;
    if (!parseConfiguredScope(config, package)) return ENOENT;
    char contents[kMaxProfileBytes + 1];
    error = readFile("/data/misc/keystore/ommega/data/cpu-mask-profile.conf", contents, sizeof(contents));
    if (error) return error;
    return parseRuntimeProfile(contents, profile) ? 0 : EINVAL;
}
}  // namespace maskcpu
