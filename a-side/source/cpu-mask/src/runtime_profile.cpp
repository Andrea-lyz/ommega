// SPDX-License-Identifier: MIT
#include "runtime_profile.h"
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <sys/stat.h>
#include <unistd.h>

namespace maskcpu {
namespace {
constexpr const char* kKeys[] = {
    "ro.product.brand", "ro.product.device", "ro.product.name",
    "ro.product.manufacturer", "ro.product.model",
    "ro.product.brand_for_attestation", "ro.product.device_for_attestation",
    "ro.product.name_for_attestation", "ro.product.manufacturer_for_attestation",
    "ro.product.model_for_attestation", "ro.soc.model",
    "ro.product.oplus.cpuinfo", "ro.build.device_family"
};

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
    unsigned seen = 0;
    bool version = false;
    char* save = nullptr;
    for (char* line = strtok_r(contents, "\n", &save); line; line = strtok_r(nullptr, "\n", &save)) {
        char* value = strchr(line, '=');
        if (!value) return false;
        *value++ = 0;
        if (!strcmp(line, "version") && !version && !strcmp(value, "1")) { version = true; continue; }
        size_t index = 0;
        while (index < 13 && strcmp(line, kKeys[index])) ++index;
        if (index == 13 || (seen & (1u << index)) || strlen(value) >= 92 || strchr(value, '\r')) return false;
        if ((index < 5 || index == 10) && !value[0]) return false;
        if (index >= 5 && index < 10 && value[0]) return false;
        strcpy(result.values[index], value);
        seen |= 1u << index;
    }
    if (!version || seen != ((1u << 13) - 1)) return false;
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
    char contents[8193];
    error = readFile("/data/misc/keystore/ommega/data/cpu-mask-profile.conf", contents, sizeof(contents));
    if (error) return error;
    return parseRuntimeProfile(contents, profile) ? 0 : EINVAL;
}
}  // namespace maskcpu
