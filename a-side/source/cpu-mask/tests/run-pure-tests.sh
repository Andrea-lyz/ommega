#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu
cd "$(dirname "$0")/.."
taskCpuTest=$(mktemp /tmp/ommega-cpu-pure.XXXXXX)
trap 'rm -f "$taskCpuTest"' EXIT
g++ -std=c++17 -Wall -Wextra -Werror tests/profile_test.cpp src/runtime_profile.cpp -o "$taskCpuTest"
"$taskCpuTest"
