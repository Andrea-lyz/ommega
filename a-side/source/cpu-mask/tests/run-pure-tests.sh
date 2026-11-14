#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
cd "$(dirname "$0")/.."
: "${JNI_INCLUDE_DIR:?Set JNI_INCLUDE_DIR to an Android NDK sysroot/usr/include directory}"
taskCpuTests=$(mktemp -d /tmp/ommega-cpu-pure.XXXXXX)
case "$taskCpuTests" in /tmp/ommega-cpu-pure.*) ;; *) exit 1 ;; esac
taskCpuProfileTest="$taskCpuTests/profile-test"
taskCpuSplTest="$taskCpuTests/host-spl-test"
trap 'rm -f "$taskCpuProfileTest" "$taskCpuSplTest"; rmdir "$taskCpuTests"' EXIT
g++ -std=c++17 -Wall -Wextra -Werror tests/profile_test.cpp src/runtime_profile.cpp -o "$taskCpuProfileTest"
"$taskCpuProfileTest"
g++ -std=c++17 -Wall -Wextra -Werror -Itests/mocks -Ithird_party -idirafter "$JNI_INCLUDE_DIR" \
    tests/host_spl_test.cpp src/property_storage.cpp src/runtime_profile.cpp -o "$taskCpuSplTest"
"$taskCpuSplTest"
