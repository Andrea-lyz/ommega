// SPDX-License-Identifier: MIT
#include "property_storage.h"

#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/stat.h>
#include <sys/statvfs.h>
#include <sys/sysmacros.h>
#include <unistd.h>
#include <time.h>

namespace maskcpu {

bool parseMapping(const char* line, Mapping* mapping) {
    unsigned long start = 0, end = 0;
    unsigned long long offset = 0, inode = 0;
    int pathIndex = 0;
    *mapping = {};
    if (sscanf(line, "%lx-%lx %4s %llx %x:%x %llu %n", &start, &end,
               mapping->permissions, &offset, &mapping->deviceMajor,
               &mapping->deviceMinor, &inode, &pathIndex) != 7 || start >= end) {
        return false;
    }
    mapping->start = start;
    mapping->end = end;
    mapping->offset = offset;
    mapping->inode = inode;
    const size_t length = strcspn(line + pathIndex, "\r\n");
    if (length >= sizeof(mapping->path)) {
        return false;
    }
    memcpy(mapping->path, line + pathIndex, length);
    return true;
}

int copyPatchedSnapshot(int source, int destination, size_t size,
                        const PropertyPatch* properties, size_t propertyCount) {
    if (size == 0 || size > kMaxAreaBytes) {
        errno = EFBIG;
        return -1;
    }
    char* bytes = static_cast<char*>(malloc(size));
    char* comparison = static_cast<char*>(malloc(size));
    if (bytes == nullptr || comparison == nullptr) {
        free(bytes);
        free(comparison);
        errno = ENOMEM;
        return -1;
    }
    const auto readSource = [&](char* destinationBytes) {
        size_t read = 0;
        while (read < size) {
            const ssize_t result = pread(source, destinationBytes + read, size - read, read);
            if (result < 0 && errno == EINTR) continue;
            if (result <= 0) { if (result == 0) errno = EIO; return false; }
            read += static_cast<size_t>(result);
        }
        return true;
    };
    // Some CPU aliases share an area with mutable properties. Do not capture
    // bytes while property_service is updating that area's trie or values.
    bool stable = false;
    for (unsigned attempt = 0; attempt < 8; ++attempt) {
        if (!readSource(bytes)) break;
        usleep(1000);
        if (!readSource(comparison)) break;
        if (memcmp(bytes, comparison, size) == 0) { stable = true; break; }
        errno = EAGAIN;
    }
    const int readError = errno;
    free(comparison);
    if (!stable) {
        free(bytes);
        errno = readError;
        return -1;
    }
    if (!patchProperties(bytes, size, properties, propertyCount)) {
        free(bytes);
        errno = EINVAL;
        return -1;
    }
    if (ftruncate(destination, static_cast<off_t>(size)) != 0) {
        const int error = errno;
        free(bytes);
        errno = error;
        return -1;
    }
    size_t done = 0;
    while (done < size) {
        const ssize_t result = pwrite(destination, bytes + done, size - done, done);
        if (result < 0 && errno == EINTR) {
            continue;
        }
        if (result <= 0) {
            const int error = result == 0 ? EIO : errno;
            free(bytes);
            errno = error;
            return -1;
        }
        done += static_cast<size_t>(result);
    }
    free(bytes);
    return 0;
}

int bindReadOnlySnapshot(int snapshot, const char* source, const char* target) {
    struct stat descriptor{}, named{};
    if (fstat(snapshot, &descriptor) != 0 || stat(source, &named) != 0) {
        return -1;
    }
    if (!S_ISREG(descriptor.st_mode) || descriptor.st_dev != named.st_dev ||
        descriptor.st_ino != named.st_ino) {
        errno = ESTALE;
        return -1;
    }
    if (mount(source, target, nullptr, MS_BIND, nullptr) != 0) {
        return -1;
    }
    if (mount(nullptr, target, nullptr,
              MS_BIND | MS_REMOUNT | MS_RDONLY | MS_NOSUID | MS_NODEV | MS_NOEXEC,
              nullptr) != 0) {
        const int error = errno;
        umount2(target, MNT_DETACH);
        errno = error;
        return -2;
    }
    return 0;
}

bool waitForSharedFlag(const uint32_t* flag, unsigned timeoutMs) {
    timespec start{};
    if (clock_gettime(CLOCK_MONOTONIC, &start) != 0) return false;
    while (__atomic_load_n(flag, __ATOMIC_ACQUIRE) == 0) {
        timespec now{};
        if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) return false;
        const int64_t elapsed = static_cast<int64_t>(now.tv_sec - start.tv_sec) * 1000000000LL +
            now.tv_nsec - start.tv_nsec;
        if (elapsed >= static_cast<int64_t>(timeoutMs) * 1000000LL) { errno = ETIMEDOUT; return false; }
        const timespec pause{0, 1000000};
        nanosleep(&pause, nullptr);
    }
    return true;
}

int bindReadOnlySnapshotSet(const AreaFile* areas, const int* snapshots, size_t count,
                             size_t* failedArea) {
    if (failedArea) *failedArea = static_cast<size_t>(-1);
    if (!areas || !snapshots || count > kMaxAreas) { errno = EINVAL; return -1; }
    size_t mounted = 0;
    for (; mounted < count; ++mounted) {
        // Keep the named snapshot until both specialization phases finish.
        // Resolve its descriptor to a regular path: mount(2) does not accept
        // every procfs magic-link reference across namespace transitions.
        char descriptor[64], source[512];
        snprintf(descriptor, sizeof(descriptor), "/proc/self/fd/%d", snapshots[mounted]);
        const ssize_t length = readlink(descriptor, source, sizeof(source) - 1);
        if (length < 0) break;
        if (static_cast<size_t>(length) == sizeof(source) - 1) { errno = ENAMETOOLONG; break; }
        source[length] = '\0';
        if (bindReadOnlySnapshot(snapshots[mounted], source, areas[mounted].path) != 0) break;
    }
    if (mounted == count) return 0;
    const int error = errno;
    if (failedArea) *failedArea = mounted;
    while (mounted > 0) umount2(areas[--mounted].path, MNT_DETACH);
    errno = error;
    return -1;
}

int openReadOnlySnapshots(const AreaFile* areas, size_t count, int* snapshots,
                          SnapshotOpenStatus* status) {
    SnapshotOpenStatus local{};
    if (!status) status = &local;
    *status = SnapshotOpenStatus{};
    if (!areas || !snapshots || count > kMaxAreas) {
        status->step = "arguments";
        errno = EINVAL;
        return -1;
    }
    for (size_t i = 0; i < count; ++i) snapshots[i] = -1;
    for (size_t i = 0; i < count; ++i) {
        if (areas[i].path[0] != '/' || strnlen(areas[i].path, sizeof(areas[i].path)) == sizeof(areas[i].path) ||
            areas[i].size == 0 || areas[i].size > kMaxAreaBytes) {
            status->step = "metadata";
            status->failedArea = i;
            errno = EINVAL;
            return -1;
        }
    }

    // Root can enumerate areas which zygote is forbidden to read (for example,
    // build_bootimage_prop on ColorOS). Do not open an area unless this process
    // actually inherited a mapping that must be replaced. The root-installed
    // readonly bind remains available to later opens under their normal policy.
    bool needed[kMaxAreas]{};
    FILE* maps = fopen("/proc/self/maps", "re");
    if (!maps) { status->step = "maps-open"; return -1; }
    char* line = nullptr;
    size_t capacity = 0;
    while (getline(&line, &capacity, maps) >= 0) {
        Mapping mapping{};
        if (!parseMapping(line, &mapping)) continue;
        for (size_t i = 0; i < count; ++i) {
            if (mapping.inode == areas[i].inode &&
                mapping.deviceMajor == major(static_cast<dev_t>(areas[i].device)) &&
                mapping.deviceMinor == minor(static_cast<dev_t>(areas[i].device))) needed[i] = true;
        }
    }
    const bool readFailed = ferror(maps) != 0;
    free(line);
    fclose(maps);
    if (readFailed) { status->step = "maps-read"; errno = EIO; return -1; }

    int error = 0;
    for (size_t i = 0; i < count; ++i) {
        if (!needed[i]) { ++status->skipped; continue; }
        status->failedArea = i;
        status->step = "open";
        snapshots[i] = open(areas[i].path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
        if (snapshots[i] < 0) { error = errno; break; }
        struct stat info{};
        struct statvfs filesystem{};
        status->step = "fstat";
        if (fstat(snapshots[i], &info) != 0) { error = errno; break; }
        status->step = "file-layout";
        if (!S_ISREG(info.st_mode) || static_cast<uint64_t>(info.st_size) != areas[i].size) {
            error = EINVAL; break;
        }
        status->step = "snapshot-inode";
        if (static_cast<uint64_t>(info.st_dev) == areas[i].device &&
            static_cast<uint64_t>(info.st_ino) == areas[i].inode) { error = ESTALE; break; }
        status->step = "fstatvfs";
        if (fstatvfs(snapshots[i], &filesystem) != 0) { error = errno; break; }
        status->step = "readonly-mount";
        if (!(filesystem.f_flag & ST_RDONLY)) { error = EACCES; break; }
        ++status->opened;
    }
    if (error) {
        for (size_t i = 0; i < count; ++i) {
            if (snapshots[i] >= 0) close(snapshots[i]);
            snapshots[i] = -1;
        }
        errno = error;
        return -1;
    }
    status->failedArea = static_cast<size_t>(-1);
    status->step = "none";
    return 0;
}

int remapSnapshots(const AreaFile* areas, const int* snapshots, size_t count) {
    FILE* maps = fopen("/proc/self/maps", "re");
    if (maps == nullptr) {
        return -1;
    }
    char* line = nullptr;
    size_t capacity = 0;
    int remapped = 0;
    int error = 0;
    while (getline(&line, &capacity, maps) >= 0) {
        Mapping mapping{};
        if (!parseMapping(line, &mapping)) {
            continue;
        }
        for (size_t i = 0; i < count; ++i) {
            if (mapping.inode != areas[i].inode ||
                mapping.deviceMajor != major(static_cast<dev_t>(areas[i].device)) ||
                mapping.deviceMinor != minor(static_cast<dev_t>(areas[i].device))) {
                continue;
            }
            // An inaccessible area may be skipped only when it has no cached
            // mapping. Never silently leave an inherited original mapping live.
            if (snapshots[i] < 0) { error = EBADF; break; }
            const size_t length = mapping.end - mapping.start;
            if (mapping.permissions[0] != 'r' || mapping.permissions[1] != '-' ||
                mapping.permissions[2] != '-' || mapping.offset > areas[i].size ||
                length > areas[i].size - mapping.offset) {
                error = EINVAL;
                break;
            }
            void* address = reinterpret_cast<void*>(mapping.start);
            if (mmap(address, length, PROT_READ, MAP_SHARED | MAP_FIXED,
                     snapshots[i], static_cast<off_t>(mapping.offset)) == MAP_FAILED) {
                error = errno;
                break;
            }
            ++remapped;
        }
        if (error != 0) {
            break;
        }
    }
    if (ferror(maps) && error == 0) {
        error = EIO;
    }
    free(line);
    fclose(maps);
    if (error != 0) {
        errno = error;
        return -1;
    }
    return remapped;
}

}  // namespace maskcpu
