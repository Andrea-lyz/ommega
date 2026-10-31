// SPDX-License-Identifier: MIT
#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif

#include "property_view.h"
#include "property_storage.h"
#include "identity_profile.h"
#include "runtime_profile.h"

#include <android/log.h>
#include <ctype.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/mount.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <sys/system_properties.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <sys/xattr.h>
#include <time.h>
#include <unistd.h>

namespace maskcpu {
namespace {

constexpr char kTag[] = "MaskCPU";
constexpr uint32_t kProtocol = 0x4d435037;  // MCP6: pre IPC, post shared-memory coordination

struct ScopeRequest { uint32_t protocol; int32_t pid; uint32_t uid; char package[256]; };
struct ScopeReply { uint32_t protocol; int32_t error; RuntimeProfile profile; };

void captureHostIdentity() {
    for (size_t i = 0; i < kIdentityCount; ++i) {
        char key[80];
        snprintf(key, sizeof(key), "ro.product.%s", kIdentity[i].property);
        __system_property_get(key, kHostValues[i]);
    }
    for (size_t i = 0; i < kDirectCount; ++i) __system_property_get(kDirectProperties[i].property, kHostValues[5 + i]);
}

struct NamespaceId { uint64_t device; uint64_t inode; };
struct Request {
    uint32_t protocol;
    int32_t pid;
    uint32_t uid;
    NamespaceId parentNamespace;
};
struct Reply {
    uint32_t protocol;
    int32_t error;
    uint32_t count;
    uint32_t patches;
    AreaFile areas[kMaxAreas];
};
struct AreaPlan {
    AreaFile file;
    PropertyPatch properties[kMaxPatches];
    size_t count;
};
struct Plan {
    AreaPlan areas[kMaxAreas];
    size_t count;
    size_t patches;
    int error;
    bool modelFound;
    uint32_t identityFound;
    unsigned skippedAliases;
    bool directFound[kDirectCount];
};

struct PropertyContext { Plan* plan; const prop_info* property; };

class Fd {
public:
    explicit Fd(int value = -1) : value_(value) {}
    ~Fd() { if (value_ >= 0) close(value_); }
    Fd(const Fd&) = delete;
    Fd& operator=(const Fd&) = delete;
    int get() const { return value_; }
    int release() { const int value = value_; value_ = -1; return value; }
private:
    int value_;
};

class StatusPage {
public:
    explicit StatusPage(int fd) {
        struct stat info{};
        if (fstat(fd, &info) != 0) return;
        if (!S_ISREG(info.st_mode) || info.st_size != sysconf(_SC_PAGESIZE)) { errno = EINVAL; return; }
        size_ = static_cast<size_t>(info.st_size);
        void* page = mmap(nullptr, size_, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
        if (page != MAP_FAILED) page_ = static_cast<SharedViewStatus*>(page);
    }
    ~StatusPage() { if (page_) munmap(page_, size_); }
    SharedViewStatus* get() const { return page_; }
    void* release() { void* page = page_; page_ = nullptr; return page; }
private:
    SharedViewStatus* page_ = nullptr;
    size_t size_ = 0;
};

bool transferBytes(int socket, void* data, size_t size, bool sending) {
    char* bytes = static_cast<char*>(data);
    size_t done = 0;
    while (done < size) {
        const ssize_t result = sending
            ? send(socket, bytes + done, size - done, MSG_NOSIGNAL)
            : recv(socket, bytes + done, size - done, 0);
        if (result < 0 && errno == EINTR) continue;
        if (result <= 0) { if (result == 0) errno = EPIPE; return false; }
        done += static_cast<size_t>(result);
    }
    return true;
}

void socketTimeout(int socket) {
    const timeval timeout{10, 0};
    setsockopt(socket, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout));
    setsockopt(socket, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout));
}

bool sendPacket(int socket, void* packet, size_t size, const int* fds,
                 size_t count, bool includeCredentials = false) {
    if (count > kMaxAreas) { errno = EINVAL; return false; }
    alignas(cmsghdr) char control[CMSG_SPACE(sizeof(int) * kMaxAreas) +
                                   CMSG_SPACE(sizeof(ucred))]{};
    iovec io{packet, size};
    msghdr message{};
    message.msg_iov = &io;
    message.msg_iovlen = 1;
    if (count != 0 || includeCredentials) {
        message.msg_control = control;
        message.msg_controllen = (count ? CMSG_SPACE(count * sizeof(int)) : 0) +
            (includeCredentials ? CMSG_SPACE(sizeof(ucred)) : 0);
        cmsghdr* header = CMSG_FIRSTHDR(&message);
        if (count != 0) {
            header->cmsg_level = SOL_SOCKET;
            header->cmsg_type = SCM_RIGHTS;
            header->cmsg_len = CMSG_LEN(count * sizeof(int));
            memcpy(CMSG_DATA(header), fds, count * sizeof(int));
            if (includeCredentials) header = CMSG_NXTHDR(&message, header);
        }
        if (includeCredentials) {
            header->cmsg_level = SOL_SOCKET;
            header->cmsg_type = SCM_CREDENTIALS;
            header->cmsg_len = CMSG_LEN(sizeof(ucred));
            const ucred credentials{getpid(), getuid(), getgid()};
            memcpy(CMSG_DATA(header), &credentials, sizeof(credentials));
        }
    }
    ssize_t sent;
    do { sent = sendmsg(socket, &message, MSG_NOSIGNAL); } while (sent < 0 && errno == EINTR);
    if (sent <= 0) { if (sent == 0) errno = EPIPE; return false; }
    return transferBytes(socket, static_cast<char*>(packet) + sent, size - sent, true);
}

bool receivePacket(int socket, void* packet, size_t size, int* fds,
                    size_t* count, ucred* credentials = nullptr) {
    *count = 0;
    if (credentials) *credentials = {};
    alignas(cmsghdr) char control[CMSG_SPACE(sizeof(int) * kMaxAreas) +
                                   CMSG_SPACE(sizeof(ucred))]{};
    iovec io{packet, size};
    msghdr message{};
    message.msg_iov = &io;
    message.msg_iovlen = 1;
    message.msg_control = control;
    message.msg_controllen = sizeof(control);
    ssize_t received;
    do { received = recvmsg(socket, &message, MSG_WAITALL | MSG_CMSG_CLOEXEC); }
    while (received < 0 && errno == EINTR);
    if (received <= 0) { if (received == 0) errno = EPIPE; return false; }
    bool valid = !(message.msg_flags & (MSG_CTRUNC | MSG_TRUNC));
    for (cmsghdr* header = CMSG_FIRSTHDR(&message); header;
         header = CMSG_NXTHDR(&message, header)) {
        if (header->cmsg_level != SOL_SOCKET) continue;
        if (header->cmsg_len < CMSG_LEN(0)) { valid = false; continue; }
        if (header->cmsg_type == SCM_RIGHTS) {
            const size_t n = (header->cmsg_len - CMSG_LEN(0)) / sizeof(int);
            const int* values = reinterpret_cast<const int*>(CMSG_DATA(header));
            for (size_t i = 0; i < n; ++i) {
                if (*count < kMaxAreas) fds[(*count)++] = values[i];
                else { close(values[i]); valid = false; }
            }
        } else if (header->cmsg_type == SCM_CREDENTIALS && credentials &&
                   header->cmsg_len == CMSG_LEN(sizeof(ucred))) {
            memcpy(credentials, CMSG_DATA(header), sizeof(ucred));
        }
    }
    if (!valid) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "companion IPC control truncated: flags=0x%x fds=%zu",
                            message.msg_flags, *count);
        errno = EPROTO;
        return false;
    }
    return transferBytes(socket, static_cast<char*>(packet) + received,
                         size - received, false);
}

bool namespaceId(int descriptor, NamespaceId* id) {
    struct stat info{};
    if (fstat(descriptor, &info) != 0) return false;
    *id = {static_cast<uint64_t>(info.st_dev), static_cast<uint64_t>(info.st_ino)};
    return true;
}

bool sameNamespace(const NamespaceId& a, const NamespaceId& b) {
    return a.device == b.device && a.inode == b.inode;
}

bool targetUidInstalled(uint32_t uid) {
    const uint32_t appId = uid % 100000;
    if (appId < 10000 || appId > 19999) return false;
    FILE* packages = fopen("/data/system/packages.list", "re");
    if (!packages) return false;
    char* line = nullptr;
    size_t capacity = 0;
    bool found = false;
    while (getline(&line, &capacity, packages) >= 0) {
        char package[256];
        unsigned packageUid;
        if (sscanf(line, "%255s %u", package, &packageUid) == 2 &&
            strcmp(package, kTargetPackage) == 0) {
            found = packageUid % 100000 == appId;
            break;
        }
    }
    free(line);
    fclose(packages);
    return found;
}

// Check before entering the namespace: no other process may be affected.
bool namespaceExclusiveTo(pid_t target, const NamespaceId& targetNamespace) {
    DIR* processes = opendir("/proc");
    if (!processes) return false;
    bool exclusive = true;
    while (dirent* entry = readdir(processes)) {
        char* end = nullptr;
        const long pid = strtol(entry->d_name, &end, 10);
        if (end == entry->d_name || *end != '\0' || pid <= 0 || pid == target) continue;
        char path[80];
        snprintf(path, sizeof(path), "/proc/%ld/ns/mnt", pid);
        struct stat info{};
        if (stat(path, &info) == 0) {
            const NamespaceId other{static_cast<uint64_t>(info.st_dev),
                                    static_cast<uint64_t>(info.st_ino)};
            if (sameNamespace(other, targetNamespace)) { exclusive = false; break; }
        } else if (errno != ENOENT && errno != ESRCH) {
            exclusive = false;
            break;
        }
    }
    closedir(processes);
    if (!exclusive) errno = EBUSY;
    return exclusive;
}

bool mappingForValue(const char* value, Mapping* found) {
    FILE* maps = fopen("/proc/self/maps", "re");
    if (!maps) return false;
    const uintptr_t address = reinterpret_cast<uintptr_t>(value);
    char* line = nullptr;
    size_t capacity = 0;
    bool matched = false;
    while (getline(&line, &capacity, maps) >= 0) {
        Mapping mapping{};
        if (parseMapping(line, &mapping) && address >= mapping.start && address < mapping.end &&
            mapping.permissions[0] == 'r' && mapping.permissions[1] == '-' &&
            mapping.permissions[2] == '-' && isPropertyAreaPath(mapping.path)) {
            *found = mapping;
            matched = true;
            break;
        }
    }
    free(line);
    fclose(maps);
    if (!matched) errno = EINVAL;
    return matched;
}

void planProperty(void* cookie, const char* name, const char* value, uint32_t serial) {
    const auto* context = static_cast<PropertyContext*>(cookie);
    Plan* plan = context->plan;
    if (strcmp(name, "ro.soc.model") == 0) plan->modelFound = true;
    for (size_t i = 0; i < kDirectCount; ++i) {
        if (strcmp(name, kDirectProperties[i].property) == 0) plan->directFound[i] = true;
    }
    for (size_t i = 0; i < kIdentityCount; ++i) {
        char key[80];
        snprintf(key, sizeof(key), "ro.product.%s", kIdentity[i].property);
        if (strcmp(name, key) == 0) plan->identityFound |= 1u << i;
    }
    if (plan->error || strncmp(name, "ro.", 3) != 0) return;
    const auto* direct = directProperty(name);
    const char* replacement = direct ? direct->value : identityPropertyOverride(name);
    bool uncovered = false;
    if (const char* error = experimentHostError(name, value, &uncovered)) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "experiment refused: %s %s=%s", error, name, value);
        plan->error = EPROTO;
        return;
    }
    if (uncovered) {
        ++plan->skippedAliases;
        __android_log_print(ANDROID_LOG_INFO, kTag, "uncovered partition alias retained: %s=%s", name, value);
        return;
    }
    if (!direct && replacement && replacement[0] == '\0') {
        return; // Dedicated attestation properties remain empty; no insertion.
    }
    if (!replacement || strcmp(value, replacement) == 0) return;
    // exp6 already patches the complete short prop_info union and length serial.
    // B's tuple may have a different byte length without moving any trie offsets.
    const uintptr_t record = reinterpret_cast<uintptr_t>(context->property);
    if ((serial & (kLongPropertyFlag | 1u)) != 0 || strlen(value) >= kPropertyValueBytes ||
        (serial >> 24) != strlen(value) || strlen(name) >= sizeof(PropertyPatch::name) ||
        reinterpret_cast<uintptr_t>(value) != record + 4 ||
        reinterpret_cast<uintptr_t>(name) != record + kPropertyNameOffset) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "unsupported readonly property layout: %s", name);
        plan->error = EOPNOTSUPP;
        return;
    }
    Mapping mapping{};
    if (!mappingForValue(reinterpret_cast<const char*>(record), &mapping)) {
        plan->error = errno; return;
    }
    struct stat info{};
    if (stat(mapping.path, &info) != 0) { plan->error = errno; return; }
    if (info.st_ino != mapping.inode || info.st_size <= 0 ||
        static_cast<uint64_t>(info.st_size) > kMaxAreaBytes ||
        major(info.st_dev) != mapping.deviceMajor || minor(info.st_dev) != mapping.deviceMinor) {
        plan->error = EINVAL;
        return;
    }
    size_t index = 0;
    for (; index < plan->count; ++index) {
        if (plan->areas[index].file.device == static_cast<uint64_t>(info.st_dev) &&
            plan->areas[index].file.inode == static_cast<uint64_t>(info.st_ino)) break;
    }
    if (index == plan->count) {
        if (plan->count == kMaxAreas) { plan->error = E2BIG; return; }
        AreaFile& file = plan->areas[plan->count++].file;
        file.device = info.st_dev;
        file.inode = info.st_ino;
        file.size = info.st_size;
        if (strlen(mapping.path) >= sizeof(file.path)) { plan->error = ENAMETOOLONG; return; }
        strcpy(file.path, mapping.path);
    }
    AreaPlan& area = plan->areas[index];
    const uint64_t offset = mapping.offset + record - mapping.start;
    if (area.count == kMaxPatches || offset > area.file.size ||
        area.file.size - offset < kPropertyNameOffset + strlen(name) + 1) {
        plan->error = E2BIG;
        return;
    }
    PropertyPatch& patch = area.properties[area.count++];
    patch.recordOffset = offset;
    patch.serial = serial;
    strcpy(patch.name, name);
    strcpy(patch.expected, value);
    strcpy(patch.replacement, replacement);
    ++plan->patches;
}

void enumerateForPlan(const prop_info* property, void* cookie) {
    PropertyContext context{static_cast<Plan*>(cookie), property};
    __system_property_read_callback(property, planProperty, &context);
}

struct OriginalCheck { const PropertyPatch* patch; bool matches; };
void checkOriginal(void* cookie, const char* name, const char* value, uint32_t serial) {
    auto* check = static_cast<OriginalCheck*>(cookie);
    check->matches = strcmp(name, check->patch->name) == 0 &&
        strcmp(value, check->patch->expected) == 0 && serial == check->patch->serial;
}

bool verifyOriginalAreas(const Plan& plan) {
    for (size_t i = 0; i < plan.count; ++i) {
        for (size_t j = 0; j < plan.areas[i].count; ++j) {
            const auto& patch = plan.areas[i].properties[j];
            const prop_info* property = __system_property_find(patch.name);
            if (!property) return false;
            OriginalCheck check{&patch, false};
            __system_property_read_callback(property, checkOriginal, &check);
            if (!check.matches) return false;
        }
    }
    return true;
}

int createSnapshot(const AreaPlan& area, int* snapshot, char* temporary, size_t capacity) {
    Fd source(open(area.file.path, O_RDONLY | O_CLOEXEC | O_NOFOLLOW));
    if (source.get() < 0) return -1;
    struct stat info{};
    if (fstat(source.get(), &info) != 0) return -1;
    if (!S_ISREG(info.st_mode) || static_cast<uint64_t>(info.st_dev) != area.file.device ||
        static_cast<uint64_t>(info.st_ino) != area.file.inode ||
        static_cast<uint64_t>(info.st_size) != area.file.size) {
        errno = ESTALE;
        return -1;
    }
    snprintf(temporary, capacity, "/dev/.maskcpu-%d-XXXXXX", getpid());
    *snapshot = mkostemp(temporary, O_CLOEXEC);
    if (*snapshot < 0) return -1;
    if (copyPatchedSnapshot(source.get(), *snapshot, area.file.size,
                            area.properties, area.count) != 0) return -1;
    char context[256];
    const ssize_t length = fgetxattr(source.get(), "security.selinux", context, sizeof(context));
    if (length <= 0) { if (length == 0) errno = EINVAL; return -1; }
    if (fsetxattr(*snapshot, "security.selinux", context, length, 0) != 0 ||
        fchown(*snapshot, 0, 0) != 0 || fchmod(*snapshot, 0444) != 0) return -1;
    return 0;
}

bool processHasUid(pid_t pid, uid_t expected) {
    char path[80], line[256];
    snprintf(path, sizeof(path), "/proc/%d/status", pid);
    FILE* status = fopen(path, "re");
    if (!status) return false;
    bool matched = false;
    while (fgets(line, sizeof(line), status)) {
        unsigned real = 0, effective = 0, saved = 0, filesystem = 0;
        if (sscanf(line, "Uid: %u %u %u %u", &real, &effective, &saved, &filesystem) == 4) {
            matched = real == expected && effective == expected && saved == expected && filesystem == expected;
            break;
        }
    }
    fclose(status);
    if (!matched) errno = EPERM;
    return matched;
}

void companionWork(int socket) {
    socketTimeout(socket);
    int enabled = 1;
    if (setsockopt(socket, SOL_SOCKET, SO_PASSCRED, &enabled, sizeof(enabled)) != 0) return;
    uint32_t ready = kProtocol;
    if (!transferBytes(socket, &ready, sizeof(ready), true)) return;
    // Check the package and locally cached B profile before unsharing or mounting.
    ScopeRequest scope{};
    ScopeReply scopeReply{};
    scopeReply.protocol = kProtocol;
    ucred scopeCredentials{};
    int scopeFds[kMaxAreas];
    size_t scopeFdCount = 0;
    if (!receivePacket(socket, &scope, sizeof(scope), scopeFds, &scopeFdCount, &scopeCredentials)) return;
    for (size_t i = 0; i < scopeFdCount; ++i) close(scopeFds[i]);
    if (getuid() != 0 || scope.protocol != kProtocol || scope.pid <= 1 || scopeCredentials.uid != 0 ||
        scopeCredentials.pid != scope.pid || scopeFdCount ||
        strnlen(scope.package, sizeof(scope.package)) == sizeof(scope.package)) scopeReply.error = EPERM;
    else {
        strcpy(kTargetPackage, scope.package);
        scopeReply.error = loadConfiguredProfile(scope.package, &scopeReply.profile);
        if (!scopeReply.error && !targetUidInstalled(scope.uid)) scopeReply.error = EPERM;
    }
    if (!sendPacket(socket, &scopeReply, sizeof(scopeReply), nullptr, 0) || scopeReply.error) return;
    kRuntimeProfile = scopeReply.profile;
    captureHostIdentity();
    Request request{};
    ucred credentials{};
    int incoming[kMaxAreas];
    size_t incomingCount = 0;
    Reply reply{};
    reply.protocol = kProtocol;
    if (!receivePacket(socket, &request, sizeof(request), incoming, &incomingCount, &credentials)) {
        for (size_t i = 0; i < incomingCount; ++i) close(incoming[i]);
        return;
    }
    if (incomingCount != 2) {
        for (size_t i = 0; i < incomingCount; ++i) close(incoming[i]);
        reply.error = EPROTO;
        sendPacket(socket, &reply, sizeof(reply), nullptr, 0);
        return;
    }
    Fd targetNamespace(incoming[0]);
    Fd stateFd(incoming[1]);
    StatusPage shared(stateFd.get());
    if (!shared.get() || shared.get()->protocol != kProtocol) {
        reply.error = EPROTO;
        sendPacket(socket, &reply, sizeof(reply), nullptr, 0);
        return;
    }
    Fd targetProcess(static_cast<int>(syscall(__NR_pidfd_open, request.pid, 0)));
    int snapshots[kMaxAreas];
    char temporary[kMaxAreas][128]{};
    for (int& descriptor : snapshots) descriptor = -1;
    auto* allocatedPlan = static_cast<Plan*>(calloc(1, sizeof(Plan)));
    if (!allocatedPlan) {
        reply.error = ENOMEM;
        sendPacket(socket, &reply, sizeof(reply), nullptr, 0);
        return;
    }
    Plan& plan = *allocatedPlan;
    size_t mounted = 0;
    bool entered = false;
    Fd ownNamespace(open("/proc/thread-self/ns/mnt", O_RDONLY | O_CLOEXEC));
    do {
        if (getuid() != 0 || request.protocol != kProtocol || request.pid <= 1 ||
            credentials.pid != request.pid || credentials.uid != 0 ||
            request.pid != scope.pid || request.uid != scope.uid ||
            !targetUidInstalled(request.uid)) { reply.error = EPERM; break; }
        if (targetProcess.get() < 0) { reply.error = errno ? errno : ESRCH; break; }
        NamespaceId targetId{}, ownId{}, actualId{};
        char namespacePath[80];
        snprintf(namespacePath, sizeof(namespacePath), "/proc/%d/ns/mnt", request.pid);
        Fd actualNamespace(open(namespacePath, O_RDONLY | O_CLOEXEC));
        if (!namespaceId(targetNamespace.get(), &targetId) ||
            !namespaceId(ownNamespace.get(), &ownId) ||
            !namespaceId(actualNamespace.get(), &actualId)) { reply.error = errno; break; }
        if (sameNamespace(targetId, request.parentNamespace) || sameNamespace(targetId, ownId) ||
            !sameNamespace(targetId, actualId)) { reply.error = EPERM; break; }
        if (!namespaceExclusiveTo(request.pid, targetId)) { reply.error = errno; break; }
        if (__system_property_foreach(enumerateForPlan, &plan) != 0) {
            reply.error = EIO; break;
        }
        if (plan.error || !plan.modelFound || plan.identityFound != ((1u << kIdentityCount) - 1)) {
            reply.error = plan.error ? plan.error : ENODEV; break;
        }
        for (size_t i = 0; i < kDirectCount; ++i) {
            if (!plan.directFound[i] && kDirectProperties[i].value[0]) {
                __android_log_print(ANDROID_LOG_ERROR, kTag, "B view requires absent property: %s", kDirectProperties[i].property);
                reply.error = ENODEV;
                break;
            }
        }
        if (reply.error) break;
        for (size_t i = 0; i < plan.count; ++i) {
            if (createSnapshot(plan.areas[i], &snapshots[i], temporary[i], sizeof(temporary[i])) != 0) {
                reply.error = errno; break;
            }
            reply.areas[i] = plan.areas[i].file;
        }
        if (reply.error) break;
        if (setns(targetNamespace.get(), CLONE_NEWNS) != 0) { reply.error = errno; break; }
        entered = true;
        // /dev is shared on stock ColorOS. Detach propagation before any bind.
        if (mount(nullptr, "/dev", nullptr, MS_REC | MS_PRIVATE, nullptr) != 0) {
            reply.error = errno; break;
        }
        for (; mounted < plan.count; ++mounted) {
            const int result = bindReadOnlySnapshot(snapshots[mounted], temporary[mounted],
                                                    plan.areas[mounted].file.path);
            if (result != 0) {
                reply.error = errno;
                __android_log_print(ANDROID_LOG_ERROR, kTag, "property mount failed: step=%s path=%s error=%s",
                                    result == -2 ? "readonly" : "bind", plan.areas[mounted].file.path,
                                    strerror(reply.error));
                break;
            }
        }
        if (reply.error) break;
        const bool hostUnchanged = verifyOriginalAreas(plan);
        __android_log_print(hostUnchanged ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kTag,
                            "host original readonly values unchanged=%s checked=%zu uncovered_partition_aliases=%u",
                            hostUnchanged ? "yes" : "no", plan.patches, plan.skippedAliases);
        if (!hostUnchanged) { reply.error = ESTALE; break; }
        reply.count = static_cast<uint32_t>(plan.count);
        reply.patches = static_cast<uint32_t>(plan.patches);
    } while (false);

    // This worker is a separate process; it exits in the target namespace.
    // Neither the persistent companion nor zygote ever enters it.
    if (reply.error && entered) {
        while (mounted > 0) umount2(plan.areas[--mounted].file.path, MNT_DETACH);
    }
    // The binds are already installed in the caller's private namespace. Return
    // metadata only; let zygote open its own O_RDONLY descriptors after receiving it.
    const bool delivered = sendPacket(socket, &reply, sizeof(reply), nullptr, 0);
    __android_log_print(delivered ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kTag,
                        "companion metadata reply: sent=%s areas=%u patches=%u reply_fds=0 status=%d",
                        delivered ? "yes" : "no", reply.count, reply.patches, reply.error);
    if (!delivered && !reply.error && entered) {
        while (mounted > 0) umount2(plan.areas[--mounted].file.path, MNT_DETACH);
    }
    if (delivered && !reply.error) {
        // No socket operations are required in the app after its SELinux
        // transition. The pre-authenticated shared page signals post entry;
        // kernel proc data and pidfd still determine identity and lifetime.
        do {
            if (setns(ownNamespace.get(), CLONE_NEWNS) != 0) { reply.error = errno; break; }
            entered = false;
            mounted = 0;
            if (!waitForSharedFlag(&shared.get()->appReady, 10000)) { reply.error = errno; break; }
            pollfd lifetime{targetProcess.get(), POLLIN, 0};
            const int alive = poll(&lifetime, 1, 0);
            if (alive < 0) { reply.error = errno; break; }
            if (alive != 0) { reply.error = ESRCH; break; }
            if (!processHasUid(request.pid, static_cast<uid_t>(request.uid))) { reply.error = errno; break; }
            char path[80];
            snprintf(path, sizeof(path), "/proc/%d/ns/mnt", request.pid);
            Fd finalNamespace(open(path, O_RDONLY | O_CLOEXEC));
            NamespaceId finalId{}, ownId{};
            if (!namespaceId(finalNamespace.get(), &finalId) ||
                !namespaceId(ownNamespace.get(), &ownId)) { reply.error = errno; break; }
            if (sameNamespace(finalId, ownId) || sameNamespace(finalId, request.parentNamespace)) {
                reply.error = EPERM; break;
            }
            if (!namespaceExclusiveTo(request.pid, finalId)) { reply.error = errno; break; }
            if (poll(&lifetime, 1, 0) != 0) { reply.error = ESRCH; break; }
            if (setns(finalNamespace.get(), CLONE_NEWNS) != 0) { reply.error = errno; break; }
            entered = true;
            if (mount(nullptr, "/dev", nullptr, MS_REC | MS_PRIVATE, nullptr) != 0) {
                reply.error = errno; break;
            }
            size_t failed = 0;
            if (bindReadOnlySnapshotSet(reply.areas, snapshots, reply.count, &failed) != 0) {
                reply.error = errno;
                __android_log_print(ANDROID_LOG_ERROR, kTag, "post-specialize mount failed: path=%s error=%s",
                                    failed < reply.count ? reply.areas[failed].path : "<none>", strerror(reply.error));
                break;
            }
            mounted = reply.count;
            if (!verifyOriginalAreas(plan)) { reply.error = ESTALE; break; }
        } while (false);
        if (reply.error && entered) {
            while (mounted > 0) umount2(plan.areas[--mounted].file.path, MNT_DETACH);
        }
        __atomic_store_n(&shared.get()->error, reply.error, __ATOMIC_RELAXED);
        __atomic_store_n(&shared.get()->done, 1u, __ATOMIC_RELEASE);
        __android_log_print(reply.error ? ANDROID_LOG_ERROR : ANDROID_LOG_INFO, kTag,
                            "companion post-specialize shared status: areas=%u status=%d uid=%d pidfd_bound=yes",
                            reply.count, reply.error, request.uid);
    }
    // Temporary paths belong to the worker's original /dev, even if the app
    // switched to a different filesystem view during specialization.
    if (ownNamespace.get() >= 0) setns(ownNamespace.get(), CLONE_NEWNS);
    for (size_t i = 0; i < kMaxAreas; ++i) {
        if (temporary[i][0]) unlink(temporary[i]);
        if (snapshots[i] >= 0) close(snapshots[i]);
    }
    free(allocatedPlan);
}

bool childGetprop(char* output, size_t capacity) {
    int descriptors[2];
    if (pipe2(descriptors, O_CLOEXEC) != 0) return false;
    const pid_t child = fork();
    if (child == 0) {
        close(descriptors[0]);
        if (dup2(descriptors[1], STDOUT_FILENO) < 0) _exit(126);
        close(descriptors[1]);
        execl("/system/bin/getprop", "getprop", static_cast<char*>(nullptr));
        _exit(127);
    }
    close(descriptors[1]);
    if (child < 0) { close(descriptors[0]); return false; }
    timespec now{};
    clock_gettime(CLOCK_MONOTONIC, &now);
    const int64_t deadline = now.tv_sec * 1000LL + now.tv_nsec / 1000000 + 3000;
    size_t used = 0;
    bool eof = false;
    while (used + 1 < capacity) {
        clock_gettime(CLOCK_MONOTONIC, &now);
        const int remaining = static_cast<int>(deadline - (now.tv_sec * 1000LL + now.tv_nsec / 1000000));
        if (remaining <= 0) break;
        pollfd descriptor{descriptors[0], POLLIN | POLLHUP, 0};
        const int ready = poll(&descriptor, 1, remaining);
        if (ready < 0 && errno == EINTR) continue;
        if (ready <= 0) break;
        const ssize_t length = read(descriptors[0], output + used, capacity - used - 1);
        if (length < 0 && errno == EINTR) continue;
        if (length < 0) break;
        if (length == 0) { eof = true; break; }
        used += static_cast<size_t>(length);
    }
    close(descriptors[0]);
    output[used] = '\0';
    int status = 0;
    pid_t waited = 0;
    for (int attempt = 0; attempt < 100 && eof; ++attempt) {
        waited = waitpid(child, &status, WNOHANG);
        if (waited == child || (waited < 0 && errno != EINTR)) break;
        usleep(10000);
    }
    if (waited != child) {
        kill(child, SIGKILL);
        while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
        return false;
    }
    return eof && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

bool listingValue(const char* listing, const char* key, char* output, size_t capacity, bool allowMissing) {
    char prefix[160];
    snprintf(prefix, sizeof(prefix), "[%s]: [", key);
    const size_t prefixLength = strlen(prefix);
    const char* line = listing;
    output[0] = '\0';
    while (*line) {
        const char* end = strchr(line, '\n');
        if (!end) end = line + strlen(line);
        if (strncmp(line, prefix, prefixLength) == 0) {
            const char* value = line + prefixLength;
            if (end <= value || end[-1] != ']') return false;
            const size_t length = static_cast<size_t>(end - value - 1);
            if (length >= capacity) return false;
            memcpy(output, value, length);
            output[length] = '\0';
            return true;
        }
        line = *end ? end + 1 : end;
    }
    return allowMissing;
}

struct ExpectedValue { const char* value; bool matches; };
void verifyCallback(void* cookie, const char*, const char* value, uint32_t serial) {
    auto* expected = static_cast<ExpectedValue*>(cookie);
    expected->matches = strcmp(value, expected->value) == 0 &&
        (serial & (kLongPropertyFlag | 1u)) == 0 && (serial >> 24) == strlen(value);
}

bool verifyNativeKey(const char* key, const char* expected) {
    char direct[PROP_VALUE_MAX]{};
    const int length = __system_property_get(key, direct);
    if (length != static_cast<int>(strlen(expected)) || strcmp(direct, expected) != 0) return false;
    const prop_info* property = __system_property_find(key);
    if (!property) return expected[0] == '\0';
    char name[128]{}, value[PROP_VALUE_MAX]{};
    const int read = __system_property_read(property, name, value);
    ExpectedValue callback{expected, false};
    __system_property_read_callback(property, verifyCallback, &callback);
    return read == length && strcmp(name, key) == 0 && strcmp(value, expected) == 0 && callback.matches;
}

struct AliasCheck { unsigned mismatches; unsigned uncovered; };
void checkAlias(void* cookie, const char* name, const char* value, uint32_t) {
    auto* check = static_cast<AliasCheck*>(cookie);
    if (const auto* direct = directProperty(name)) {
        if (strcmp(value, direct->value) != 0) ++check->mismatches;
        return;
    }
    const char* expected = identityPropertyOverride(name);
    if (expected) {
        if (strcmp(value, expected) == 0) return;
        if (expected[0] == '\0') { ++check->mismatches; return; }
        const auto* field = identityFieldForProperty(name);
        char root[80];
        snprintf(root, sizeof(root), "ro.product.%s", field->property);
        if (strcmp(name, root) == 0 || strcmp(value, field->hostValue) == 0 ||
            (strncmp(name, "ro.product.vendor.", 18) == 0 && !emptyOrUnknown(value))) {
            ++check->mismatches;
        } else if (!emptyOrUnknown(value)) {
            ++check->uncovered;
        }
        return;
    }
    for (const auto& alias : kDirectProperties) {
        if (strcmp(name, alias.property) == 0 && strcmp(value, alias.value) != 0) ++check->mismatches;
    }
}
void enumerateForAliases(const prop_info* property, void* cookie) {
    __system_property_read_callback(property, checkAlias, cookie);
}

}  // namespace

void propertyCompanion(int socket) {
    // Never change the persistent root companion's own mount namespace.
    const pid_t worker = fork();
    if (worker == 0) {
        companionWork(socket);
        close(socket);
        _exit(0);
    }
    close(socket);
    if (worker > 0) {
        while (waitpid(worker, nullptr, 0) < 0 && errno == EINTR) {}
    }
}

bool preparePropertyView(zygisk::Api* api, int targetUid, const char* package, void** pendingState) {
    if (!pendingState) { errno = EINVAL; return false; }
    *pendingState = nullptr;
    Fd socket(api->connectCompanion());
    if (socket.get() < 0) return false;
    socketTimeout(socket.get());
    uint32_t ready = 0;
    ScopeRequest scope{};
    scope.protocol = kProtocol; scope.pid = getpid(); scope.uid = targetUid;
    if (!package || strlen(package) >= sizeof(scope.package)) return false;
    strcpy(scope.package, package);
    if (!transferBytes(socket.get(), &ready, sizeof(ready), false) || ready != kProtocol ||
        !sendPacket(socket.get(), &scope, sizeof(scope), nullptr, 0, true)) return false;
    ScopeReply scopeReply{};
    int unused[kMaxAreas];
    size_t unusedCount = 0;
    const bool scopeReceived = receivePacket(socket.get(), &scopeReply, sizeof(scopeReply), unused, &unusedCount);
    for (size_t i = 0; i < unusedCount; ++i) close(unused[i]);
    if (!scopeReceived || unusedCount || scopeReply.protocol != kProtocol || scopeReply.error) return false;
    kRuntimeProfile = scopeReply.profile;
    captureHostIdentity();
    Fd stateFd(memfd_create("maskcpu-view-status", MFD_CLOEXEC));
    if (stateFd.get() < 0 || ftruncate(stateFd.get(), sysconf(_SC_PAGESIZE)) != 0) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "shared status create failed: %s", strerror(errno));
        return false;
    }
    StatusPage status(stateFd.get());
    if (!status.get()) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "shared status map failed: %s", strerror(errno));
        return false;
    }
    status.get()->protocol = kProtocol;
    Fd parentNamespace(open("/proc/thread-self/ns/mnt", O_RDONLY | O_CLOEXEC));
    Request request{};
    request.protocol = kProtocol;
    request.pid = getpid();
    request.uid = targetUid;
    if (!namespaceId(parentNamespace.get(), &request.parentNamespace) || unshare(CLONE_NEWNS) != 0) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "private namespace failed: %s", strerror(errno));
        return false;
    }
    Fd targetNamespace(open("/proc/thread-self/ns/mnt", O_RDONLY | O_CLOEXEC));
    if (targetNamespace.get() < 0) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "target namespace open failed: %s", strerror(errno));
        return false;
    }
    const int requestFds[] = {targetNamespace.get(), stateFd.get()};
    if (!sendPacket(socket.get(), &request, sizeof(request), requestFds, 2, true)) {
        __android_log_print(ANDROID_LOG_ERROR, kTag, "companion request failed: %s", strerror(errno));
        return false;
    }
    Reply reply{};
    int snapshots[kMaxAreas];
    size_t count = 0;
    const bool received = receivePacket(socket.get(), &reply, sizeof(reply), snapshots, &count);
    for (size_t i = 0; i < count; ++i) close(snapshots[i]);
    for (int& snapshot : snapshots) snapshot = -1;
    bool success = false;
    if (received && reply.protocol == kProtocol && reply.error == 0 && count == 0 && reply.count <= kMaxAreas) {
        bool pathsValid = true;
        for (size_t i = 0; i < reply.count; ++i) {
            pathsValid &= strnlen(reply.areas[i].path, sizeof(reply.areas[i].path)) < sizeof(reply.areas[i].path) &&
                isPropertyAreaPath(reply.areas[i].path);
        }
        if (!pathsValid) errno = EPROTO;
        SnapshotOpenStatus status{};
        const bool opened = pathsValid && openReadOnlySnapshots(reply.areas, reply.count, snapshots, &status) == 0;
        const int remapped = opened ? remapSnapshots(reply.areas, snapshots, reply.count) : -1;
        success = remapped >= 0;
        const int error = success ? 0 : errno;
        __android_log_print(success ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kTag,
                            "private property view: areas=%u patches=%u reply_fds=%zu local_readonly=%s opened=%zu unmapped=%zu remapped=%d error=%s",
                            reply.count, reply.patches, count, opened ? "yes" : "no", status.opened, status.skipped,
                            remapped, error ? strerror(error) : "none");
        if (!opened) {
            const char* path = status.failedArea < reply.count ? reply.areas[status.failedArea].path : "<none>";
            __android_log_print(ANDROID_LOG_ERROR, kTag, "snapshot reader failed: step=%s path=%s errno=%d",
                                pathsValid ? status.step : "reply-path", path, error);
        }
    } else {
        __android_log_print(ANDROID_LOG_ERROR, kTag,
                            "property view reply failed: received=%s protocol=0x%x expected=0x%x status=%d fds=%zu areas=%u errno=%d",
                            received ? "yes" : "no", reply.protocol, kProtocol, reply.error, count, reply.count, errno);
    }
    for (int snapshot : snapshots) if (snapshot >= 0) close(snapshot);
    if (success) *pendingState = status.release();
    return success;
}

bool finalizePropertyView(void* opaque) {
    auto* status = static_cast<SharedViewStatus*>(opaque);
    if (!status) return false;
    bool completed = false;
    int error = EPROTO;
    if (status->protocol == kProtocol) {
        __atomic_store_n(&status->appReady, 1u, __ATOMIC_RELEASE);
        completed = waitForSharedFlag(&status->done, 10000);
        error = completed ? status->error : errno;
    }
    const bool success = completed && error == 0;
    __android_log_print(success ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kTag,
                        "post-specialize shared status: completed=%s status=%d ready=%s (no post socket IPC)",
                        completed ? "yes" : "no", error, success ? "yes" : "no");
    munmap(opaque, static_cast<size_t>(sysconf(_SC_PAGESIZE)));
    return success;
}

bool verifyPropertyView() {
    char native[kIdentityCount][PROP_VALUE_MAX]{}, attest[kIdentityCount][PROP_VALUE_MAX]{};
    char vendor[kIdentityCount][PROP_VALUE_MAX]{};
    bool nativeOk = true;
    for (size_t i = 0; i < kIdentityCount; ++i) {
        char key[128];
        snprintf(key, sizeof(key), "ro.product.%s", kIdentity[i].property);
        __system_property_get(key, native[i]);
        nativeOk &= verifyNativeKey(key, kIdentity[i].value);
        snprintf(key, sizeof(key), "ro.product.%s_for_attestation", kIdentity[i].property);
        __system_property_get(key, attest[i]);
        nativeOk &= verifyNativeKey(key, "");
        snprintf(key, sizeof(key), "ro.product.vendor.%s", kIdentity[i].property);
        __system_property_get(key, vendor[i]);
        const auto* expectedVendor = directProperty(key);
        nativeOk &= expectedVendor ? strcmp(vendor[i], expectedVendor->value) == 0 :
            (emptyOrUnknown(vendor[i]) || strcmp(vendor[i], kIdentity[i].value) == 0);
    }
    char nativeSoc[PROP_VALUE_MAX]{};
    __system_property_get("ro.soc.model", nativeSoc);
    nativeOk &= verifyNativeKey("ro.soc.model", kSocModel);
    for (const auto& field : kDirectProperties) nativeOk &= verifyNativeKey(field.property, field.value);
    AliasCheck aliases{};
    if (__system_property_foreach(enumerateForAliases, &aliases) != 0) ++aliases.mismatches;
    __android_log_print(ANDROID_LOG_INFO, kTag, "native tuple=%s/%s/%s/%s/%s SOC_MODEL=%s APIs_verified=%s",
                        native[0], native[1], native[2], native[3], native[4], nativeSoc, nativeOk ? "yes" : "no");
    __android_log_print(ANDROID_LOG_INFO, kTag, "native FOR props=%s/%s/%s/%s/%s",
                        attest[0][0] ? attest[0] : "<empty>", attest[1][0] ? attest[1] : "<empty>",
                        attest[2][0] ? attest[2] : "<empty>", attest[3][0] ? attest[3] : "<empty>",
                        attest[4][0] ? attest[4] : "<empty>");
    __android_log_print(ANDROID_LOG_INFO, kTag, "native vendor fallback=%s/%s/%s/%s/%s",
                        vendor[0][0] ? vendor[0] : "<empty>", vendor[1][0] ? vendor[1] : "<empty>",
                        vendor[2][0] ? vendor[2] : "<empty>", vendor[3][0] ? vendor[3] : "<empty>",
                        vendor[4][0] ? vendor[4] : "<empty>");

    constexpr size_t listingCapacity = 1024 * 1024;
    char* listing = static_cast<char*>(malloc(listingCapacity));
    bool childOk = listing && childGetprop(listing, listingCapacity);
    char child[kIdentityCount][PROP_VALUE_MAX]{}, childAttest[kIdentityCount][PROP_VALUE_MAX]{};
    char childSoc[PROP_VALUE_MAX]{};
    if (childOk) {
        for (size_t i = 0; i < kIdentityCount; ++i) {
            char key[128];
            snprintf(key, sizeof(key), "ro.product.%s", kIdentity[i].property);
            childOk &= listingValue(listing, key, child[i], sizeof(child[i]), false) &&
                strcmp(child[i], kIdentity[i].value) == 0;
            snprintf(key, sizeof(key), "ro.product.%s_for_attestation", kIdentity[i].property);
            childOk &= listingValue(listing, key, childAttest[i], sizeof(childAttest[i]), true) &&
                childAttest[i][0] == '\0';
        }
        childOk &= listingValue(listing, "ro.soc.model", childSoc, sizeof(childSoc), false) &&
            strcmp(childSoc, kSocModel) == 0;
        for (const auto& alias : kDirectProperties) {
            char value[PROP_VALUE_MAX]{};
            childOk &= listingValue(listing, alias.property, value, sizeof(value), alias.value[0] == '\0') &&
                strcmp(value, alias.value) == 0;
        }
    }
    free(listing);
    __android_log_print(childOk ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kTag,
                        "child getprop tuple=%s/%s/%s/%s/%s SOC_MODEL=%s FOR_props_empty=%s verified=%s",
                        child[0], child[1], child[2], child[3], child[4], childSoc,
                        childOk ? "yes" : "not verified", childOk ? "yes" : "no");
    const bool complete = nativeOk && childOk && aliases.mismatches == 0;
    __android_log_print(complete ? ANDROID_LOG_INFO : ANDROID_LOG_ERROR, kTag,
                        "native/child identity complete=%s alias_mismatches=%u uncovered_partition_aliases=%u",
                        complete ? "yes" : "no", aliases.mismatches, aliases.uncovered);
    return complete;
}

}  // namespace maskcpu

REGISTER_ZYGISK_COMPANION(maskcpu::propertyCompanion)
