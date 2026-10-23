#!/usr/bin/env bash
set -euo pipefail

if [[ "$CACHE_HIT" == 'true' ]]; then
  ndk_arch="${RUNNER_ARCH,,}"
  ndk_path="$RUNNER_TOOL_CACHE/ndk/$NDK_VERSION/$ndk_arch"
  if [[ ! -f "$ndk_path.complete" ]]; then
    echo "Restored NDK has no tool-cache completion marker: $ndk_path" >&2
    exit 1
  fi
  echo "Using restored NDK: $ndk_path"
else
  ndk_path="$INSTALLED_NDK_PATH"
fi

test -f "$ndk_path/source.properties"
printf 'ndk-path=%s\n' "$ndk_path" >> "$GITHUB_OUTPUT"
