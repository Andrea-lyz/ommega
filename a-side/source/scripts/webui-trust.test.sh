#!/bin/sh
# Host-only contract tests. Never execute complete installer/boot/service scripts.
# All helper paths and lifecycle snippets are rewritten into one disposable /tmp tree.
set -eu
SOURCE=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TEMPLATE=$SOURCE/template
for file in webui-trust.sh customize.sh post-fs-data.sh service.sh; do
  sh -n "$TEMPLATE/$file"
done
sh -n "$0"
TEST_ROOT=$(mktemp -d /tmp/ommega-webui-trust.XXXXXX)
case "$TEST_ROOT" in /tmp/ommega-webui-trust.*) ;; *) exit 1 ;; esac
cleanup() {
  case "$TEST_ROOT" in
    /tmp/ommega-webui-trust.*)
      rm -rf -- "$TEST_ROOT"
      [ ! -e "$TEST_ROOT" ] || exit 1
      printf 'temporary mock tree cleaned\n'
      ;;
    *) exit 1 ;;
  esac
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
REAL_SH=$(command -v sh)
REAL_AWK=$(command -v awk)
REAL_MV=$(command -v mv)
REAL_MKDIR=$(command -v mkdir)
REAL_CHMOD=$(command -v chmod)
REAL_TOUCH=$(command -v touch)
export TEST_ROOT REAL_SH REAL_AWK REAL_MV REAL_MKDIR REAL_CHMOD REAL_TOUCH
mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/data" "$TEST_ROOT/module" "$TEST_ROOT/legacy"
# Mock every device-facing command. These mocks cannot reach a device.
printf '%s\n' '#!/bin/sh' '
name=${0##*/}
case "$name" in
  resetprop)
    if [ "$1" = -n ]; then prop=$2; else [ "$1" = --delete ] || exit 90; prop=$2; fi
    case "$prop" in ro.boot.vbmeta.digest|ro.boot.vbmeta.public_key_digest) ;; *) exit 91 ;; esac
    printf "%s\n" "$*" >> "$TEST_ROOT/live.log"
    [ "${MOCK_FAIL:-}" != live ]
    ;;
  chown) [ "${MOCK_FAIL:-}" != chown ] ;;
  sleep) printf "wait\n" >> "$TEST_ROOT/waits.log" ;;
  pidof) printf "111\n" ;;
  awk)
    last=
    for last do :; done
    [ "${MOCK_FAIL:-}" != parse ] || exit 92
    if [ "${MOCK_FAIL:-}" = toml ] && [ "$last" = "$TEST_ROOT/data/config.toml" ]; then exit 92; fi
    exec "$REAL_AWK" "$@"
    ;;
  mv)
    [ "${MOCK_FAIL:-}" != mv ] || exit 92
    if [ "${MOCK_FAIL:-}" = toml-mv ] && [ "$2" = "$TEST_ROOT/data/config.toml" ]; then exit 92; fi
    exec "$REAL_MV" "$@"
    ;;
  mkdir) [ "${MOCK_FAIL:-}" != mkdir ] || exit 92; exec "$REAL_MKDIR" "$@" ;;
  chmod) [ "${MOCK_FAIL:-}" != chmod ] || exit 92; exec "$REAL_CHMOD" "$@" ;;
  touch) [ "${MOCK_FAIL:-}" != touch ] || exit 92; exec "$REAL_TOUCH" "$@" ;;
  *) exit 93 ;;
esac' > "$TEST_ROOT/bin/mock"
chmod 755 "$TEST_ROOT/bin/mock"
for name in resetprop chown sleep pidof awk mv mkdir chmod touch; do
  ln -s mock "$TEST_ROOT/bin/$name"
done
# Test copies only: production does not gain environment-controlled paths or hooks.
"$REAL_AWK" -v root="$TEST_ROOT" '
  /^TARGET_DIR=/ { print "TARGET_DIR=\"" root "/data\""; next }
  /^RESTART_FLAG=/ { print "RESTART_FLAG=\"" root "/state/restart.keymint\""; next }
  { print }
' "$TEMPLATE/webui-trust.sh" > "$TEST_ROOT/real-helper.sh"
"$REAL_AWK" '/^cmd=/ { exit } { print }' "$TEST_ROOT/real-helper.sh" > "$TEST_ROOT/functions.sh"
"$REAL_AWK" '
  /^cmd=/ {
    print "printf() {"
    print "  if [ \"${MOCK_FAIL:-}\" = write ] && [ \"$2\" = \"# ommega WebUI Verified Boot overrides (data only)\" ]; then return 94; fi"
    print "  command printf \"$@\""
    print "}"
    print "request_keymint_restart() {"
    print "  printf \"restart\\n\" >> \"$TEST_ROOT/restart.log\""
    print "  [ \"${MOCK_FAIL:-}\" != restart ]"
    print "}"
  }
  { print }
' "$TEST_ROOT/real-helper.sh" > "$TEST_ROOT/helper.sh"
# Fail closed if any executable device data path survived rewriting.
"$REAL_AWK" '/^[^#]*\/data\/(adb|misc)\// { bad=1 } END { exit bad }' "$TEST_ROOT/helper.sh"
PATH=$TEST_ROOT/bin:$PATH
export PATH
MOCK_FAIL=
export MOCK_FAIL
HASH=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
HASH2=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb
KEY=cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc
KEY2=dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd
PROPS=$TEST_ROOT/data/webui-props.sh
CONFIG=$TEST_ROOT/data/config.toml
count=0
ok() { count=$((count + 1)); printf 'ok %s - %s\n' "$count" "$1"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
run() { "$REAL_SH" "$TEST_ROOT/helper.sh" "$@"; }
has() { "$REAL_AWK" -v want="$2" '$0 == want { found=1 } END { exit !found }' "$1"; }
no_line() { if has "$1" "$2"; then fail "unexpected line: $2"; fi; }
expect_status() {
  want=$1; shift
  actual=0
  "$@" > "$TEST_ROOT/stdout" 2> "$TEST_ROOT/stderr" || actual=$?
  [ "$actual" -eq "$want" ] || fail "wanted status $want, got $actual: $*"
}
reset_logs() { : > "$TEST_ROOT/live.log"; : > "$TEST_ROOT/restart.log"; }
seed_config() {
  printf '%s\n' '# keep comments and all non-VB fields' '[trust] # retained header' \
    '  security_patch = "2027-03-05"' 'os_patchlevel = "2027-03-05"' \
    'vendor_patchlevel = "2027-02-01"' 'boot_patchlevel = "2027-01-05"' \
    'vb_hash = "auto"' '  vb_key = "random"' 'device_locked = true' \
    '[other]' 'vb_key = "unrelated"' 'note = "untouched"' > "$CONFIG"
}
non_vb() {
  "$REAL_AWK" '
    /^[[:space:]]*\[trust\][[:space:]]*(#.*)?$/ { inside=1; print; next }
    /^[[:space:]]*\[/ { inside=0 }
    inside && /^[[:space:]]*vb_(hash|key)[[:space:]]*=/ { next }
    { print }
  ' "$CONFIG"
}
no_temps() {
  for file in "$TEST_ROOT/data/"*.tmp.*; do
    [ ! -e "$file" ] || fail "leaked temporary file: $file"
  done
}
seed_config
non_vb > "$TEST_ROOT/non-vb.before"
printf 'resetprop ro.boot.vbmeta.digest %s\nresetprop ro.boot.vbmeta.public_key_digest %s\n' "$HASH" "$KEY" > "$PROPS"
reset_logs
run hash "$(printf '%s' "$HASH2" | tr a-f A-F)"
has "$PROPS" "resetprop ro.boot.vbmeta.digest $HASH2"
has "$PROPS" "resetprop ro.boot.vbmeta.public_key_digest $KEY"
has "$CONFIG" "vb_hash = \"$HASH2\""
has "$CONFIG" "vb_key = \"$KEY\""
has "$TEST_ROOT/live.log" "-n ro.boot.vbmeta.digest $HASH2"
[ "$(wc -l < "$TEST_ROOT/live.log")" -eq 1 ]
[ -s "$TEST_ROOT/restart.log" ]
non_vb > "$TEST_ROOT/non-vb.after"
cmp "$TEST_ROOT/non-vb.before" "$TEST_ROOT/non-vb.after"
ok 'hash set normalizes hex and preserves key, SPL, comments and other sections'
run key "$KEY2"
has "$PROPS" "resetprop ro.boot.vbmeta.digest $HASH2"
has "$CONFIG" "vb_key = \"$KEY2\""
ok 'key set preserves independent hash'
reset_logs
run hash clear
no_line "$PROPS" "resetprop ro.boot.vbmeta.digest $HASH2"
has "$PROPS" "resetprop ro.boot.vbmeta.public_key_digest $KEY2"
has "$CONFIG" 'vb_hash = "auto"'
has "$CONFIG" "vb_key = \"$KEY2\""
has "$TEST_ROOT/live.log" '--delete ro.boot.vbmeta.digest'
[ "$(wc -l < "$TEST_ROOT/live.log")" -eq 1 ]
ok 'hash clear resets only hash to auto and deletes only live hash'
run key ''
has "$CONFIG" 'vb_key = "auto"'
reset_logs
run boot
[ ! -s "$TEST_ROOT/live.log" ] && [ ! -s "$TEST_ROOT/restart.log" ]
ok 'empty key clear persists and boot retains stock values without restart'
cp "$PROPS" "$TEST_ROOT/props.before"
cp "$CONFIG" "$TEST_ROOT/config.before"
for value in short "$HASH;touch $TEST_ROOT/injected" "${HASH}a" "$(printf '%064d' 0)g"; do
  expect_status 3 run hash "$value"
done
expect_status 2 run key "$KEY" extra
expect_status 2 run unknown
expect_status 2 run boot extra
expect_status 2 run sync extra
expect_status 2 run
for args in auto 'all 2027-01-05' 'set system=2027-01-05'; do
  # Deliberate fixed-word splitting for these literal subcommand cases only.
  expect_status 2 run patch $args
  [ -s "$TEST_ROOT/stderr" ]
done
cmp "$PROPS" "$TEST_ROOT/props.before"
cmp "$CONFIG" "$TEST_ROOT/config.before"
[ ! -s "$TEST_ROOT/live.log" ] && [ ! -s "$TEST_ROOT/restart.log" ]
ok 'invalid values and every patch subcommand fail with no side effects'
# Historical file contains valid values plus commands, SPL, quoted expansions,
# redirections, inline suffixes, malformed values and no final newline.
printf '%s\n' "touch $TEST_ROOT/injected" \
  'resetprop ro.build.version.security_patch 2099-01-01' \
  'resetprop ro.system.build.security_patch 2099-01-01' \
  'resetprop ro.boot.image.build.security_patch 2099-01-01' \
  'resetprop ro.vendor.build.security_patch 2099-01-01' \
  " resetprop -n ro.boot.vbmeta.digest \"$(printf '%s' "$HASH" | tr a-f A-F)\"" \
  "resetprop ro.boot.vbmeta.digest $HASH2;" \
  "resetprop ro.boot.vbmeta.digest $HASH2 > $TEST_ROOT/injected" \
  "resetprop ro.boot.vbmeta.public_key_digest '$KEY'" > "$PROPS"
printf 'resetprop ro.boot.vbmeta.digest "$(touch %s/injected)"\n' "$TEST_ROOT" >> "$PROPS"
printf '%s' 'resetprop ro.boot.vbmeta.public_key_digest nothex' >> "$PROPS"
cp "$PROPS" "$TEST_ROOT/legacy.before"
reset_logs
run boot
has "$TEST_ROOT/live.log" "-n ro.boot.vbmeta.digest $HASH"
has "$TEST_ROOT/live.log" "-n ro.boot.vbmeta.public_key_digest $KEY"
[ "$(wc -l < "$TEST_ROOT/live.log")" -eq 2 ]
[ ! -s "$TEST_ROOT/restart.log" ] && [ ! -e "$TEST_ROOT/injected" ]
cmp "$PROPS" "$TEST_ROOT/legacy.before"
non_vb > "$TEST_ROOT/non-vb.after"
cmp "$TEST_ROOT/non-vb.before" "$TEST_ROOT/non-vb.after"
ok 'boot whitelist safely parses old files without sourcing, SPL writes or deletion'
reset_logs
run sync
[ ! -s "$TEST_ROOT/restart.log" ] && [ ! -s "$TEST_ROOT/live.log" ]
ok 'idempotent sync neither reloads nor applies live properties'
seed_config
run sync
[ -s "$TEST_ROOT/restart.log" ] && [ ! -s "$TEST_ROOT/live.log" ]
ok 'changed VB sync reloads and never applies SPL or live properties'
rm "$PROPS"
cp "$CONFIG" "$TEST_ROOT/config.before"
reset_logs
run sync
cmp "$CONFIG" "$TEST_ROOT/config.before"
[ ! -s "$TEST_ROOT/restart.log" ]
ok 'absent overlay does not erase manually configured trust values'
cp "$TEST_ROOT/legacy.before" "$TEST_ROOT/legacy/webui-props.sh"
printf '%s\n' 'private sentinel, never migrated or modified' > "$TEST_ROOT/legacy/secret.sentinel"
cp "$TEST_ROOT/legacy/secret.sentinel" "$TEST_ROOT/secret.before"
run migrate "$TEST_ROOT/legacy/webui-props.sh"
has "$PROPS" "resetprop ro.boot.vbmeta.digest $HASH"
has "$PROPS" "resetprop ro.boot.vbmeta.public_key_digest $KEY"
[ "$(wc -l < "$PROPS")" -eq 3 ]
cmp "$TEST_ROOT/legacy/secret.sentinel" "$TEST_ROOT/secret.before"
cmp "$TEST_ROOT/legacy/webui-props.sh" "$TEST_ROOT/legacy.before"
[ ! -s "$TEST_ROOT/live.log" ] && [ ! -s "$TEST_ROOT/restart.log" ]
ok 'old directory migration copies VB only, not commands, SPL or private data'
printf '# explicitly cleared\n' > "$PROPS"
run migrate "$TEST_ROOT/legacy/webui-props.sh"
[ "$(wc -l < "$PROPS")" -eq 1 ]
ok 'migration never resurrects cleared overrides'
rm "$PROPS"
printf 'resetprop ro.vendor.build.security_patch 2099-01-01\n' > "$TEST_ROOT/module/post-fs-data.sh"
run migrate "$TEST_ROOT/module/post-fs-data.sh" "$TEST_ROOT/legacy/webui-props.sh"
has "$PROPS" "resetprop ro.boot.vbmeta.digest $HASH"
ok 'legacy script candidates are parsed as data with safe fallback'
# Config created by the daemon later: saves must not invent a runtime config.
rm "$CONFIG"
run hash "$HASH2"
[ ! -e "$CONFIG" ]
printf '[other]\nnote = "retained"\n' > "$CONFIG"
run boot
has "$CONFIG" '[trust]'
has "$CONFIG" "vb_hash = \"$HASH2\""
has "$CONFIG" 'note = "retained"'
ok 'first-install saves defer missing TOML; subsequent boot creates missing trust table'
# I/O and command failures must stop later phases and leave no temporary files.
for fault in parse write mkdir chmod chown mv; do
  cp "$PROPS" "$TEST_ROOT/props.before"
  cp "$CONFIG" "$TEST_ROOT/config.before"
  reset_logs
  MOCK_FAIL=$fault
  expect_status 1 run hash "$HASH"
  MOCK_FAIL=
  cmp "$PROPS" "$TEST_ROOT/props.before"
  cmp "$CONFIG" "$TEST_ROOT/config.before"
  [ ! -s "$TEST_ROOT/live.log" ] && [ ! -s "$TEST_ROOT/restart.log" ]
  no_temps
done
ok 'parse, output write, mkdir, metadata and persistence rename failures propagate and clean up'
for fault in toml toml-mv; do
  cp "$CONFIG" "$TEST_ROOT/config.before"
  reset_logs
  MOCK_FAIL=$fault
  expect_status 1 run hash "$HASH"
  MOCK_FAIL=
  cmp "$CONFIG" "$TEST_ROOT/config.before"
  [ ! -s "$TEST_ROOT/live.log" ] && [ ! -s "$TEST_ROOT/restart.log" ]
  no_temps
done
ok 'TOML failures are nonzero, preserve old config and stop live/reload'
reset_logs
MOCK_FAIL=live
expect_status 1 run key clear
expect_status 1 run hash "$HASH2"
MOCK_FAIL=
[ ! -s "$TEST_ROOT/restart.log" ]
ok 'failed live set/clear is not masked and cannot request a restart'
MOCK_FAIL=restart
expect_status 1 run hash "$HASH"
MOCK_FAIL=
ok 'failed reload reaches command exit status'
# Exercise the real reload timeout and flag failures, not a device or process.
reset_logs
expect_status 1 "$REAL_SH" -c '. "$1"; request_keymint_restart' test "$TEST_ROOT/functions.sh"
[ "$(wc -l < "$TEST_ROOT/waits.log")" -eq 120 ]
for fault in mkdir touch; do
  MOCK_FAIL=$fault
  expect_status 1 "$REAL_SH" -c '. "$1"; request_keymint_restart' test "$TEST_ROOT/functions.sh"
done
MOCK_FAIL=
ok 'real reload helper propagates timeout and restart flag failures with mocked commands'
# Execute ONLY the three small VB integration regions; all paths are mocked.
region() {
  "$REAL_AWK" -v region="$2" -v root="$TEST_ROOT" '
    $0 == "# BEGIN " region { inside=1; next }
    $0 == "# END " region { exit }
    inside {
      gsub("/data/adb/ommega/ommegadata",root "/legacy")
      gsub("/data/adb/modules/ommega",root "/old-module")
      gsub("/data/adb/modules/.ommega",root "/hidden-old-module")
      print
    }
  ' "$1"
}
printf 'MODDIR="%s/module"\n' "$TEST_ROOT" > "$TEST_ROOT/boot-region.sh"
region "$TEMPLATE/post-fs-data.sh" VB_BOOT >> "$TEST_ROOT/boot-region.sh"
printf 'exit "$boot_status"\n' >> "$TEST_ROOT/boot-region.sh"
printf 'MODDIR="%s/module"\nservice_status=${INITIAL_STATUS:-0}\n' "$TEST_ROOT" > "$TEST_ROOT/sync-region.sh"
region "$TEMPLATE/service.sh" VB_SYNC >> "$TEST_ROOT/sync-region.sh"
printf 'exit "$service_status"\n' >> "$TEST_ROOT/sync-region.sh"
for snippet in "$TEST_ROOT/boot-region.sh" "$TEST_ROOT/sync-region.sh"; do
  "$REAL_AWK" '/^[^#]*\/data\/(adb|misc)\// { bad=1 } END { exit bad }' "$snippet"
done
cp "$TEST_ROOT/helper.sh" "$TEST_ROOT/module/webui-trust.sh"
reset_logs
"$REAL_SH" "$TEST_ROOT/boot-region.sh"
[ -s "$TEST_ROOT/live.log" ] && [ ! -s "$TEST_ROOT/restart.log" ]
MOCK_FAIL=live
expect_status 1 "$REAL_SH" "$TEST_ROOT/boot-region.sh"
MOCK_FAIL=toml
expect_status 1 "$REAL_SH" "$TEST_ROOT/sync-region.sh"
MOCK_FAIL=
INITIAL_STATUS=1 expect_status 1 "$REAL_SH" "$TEST_ROOT/sync-region.sh"
ok 'boot/service snippets invoke the safe helper and preserve failure statuses'
printf 'MODPATH="%s/module"\nZIPFILE=mock\n' "$TEST_ROOT" > "$TEST_ROOT/install-region.sh"
printf '%s\n' 'extract() { cp "$TEST_ROOT/helper.sh" "$3/webui-trust.sh"; }' \
  'abort() { printf "%s\n" "$*" >&2; exit 1; }' >> "$TEST_ROOT/install-region.sh"
region "$TEMPLATE/customize.sh" VB_MIGRATION >> "$TEST_ROOT/install-region.sh"
"$REAL_AWK" '/^[^#]*\/data\/(adb|misc)\// { bad=1 } END { exit bad }' "$TEST_ROOT/install-region.sh"
rm "$PROPS"
"$REAL_SH" "$TEST_ROOT/install-region.sh"
has "$PROPS" "resetprop ro.boot.vbmeta.digest $HASH"
rm "$PROPS"
MOCK_FAIL=mkdir
expect_status 1 "$REAL_SH" "$TEST_ROOT/install-region.sh"
MOCK_FAIL=
[ ! -e "$TEST_ROOT/injected" ]
ok 'installer snippet migrates before replacement, and aborts on migration failure'
printf 'resetprop ro.boot.vbmeta.digest %s\r\n' "$HASH" > "$PROPS"
printf 'resetprop ro.boot.vbmeta.digest %s' "$HASH2" >> "$PROPS"
reset_logs
run boot
has "$TEST_ROOT/live.log" "-n ro.boot.vbmeta.digest $HASH2"
[ "$(wc -l < "$TEST_ROOT/live.log")" -eq 1 ]
ok 'legacy CRLF and final unterminated records use last valid literal only'
printf '%s\n' '[before]' 'vb_key = "unrelated"' '  [trust] # sparse' \
  'security_patch = "2028-04-05"' 'boot_patchlevel = "2028-04-05"' \
  '[after]' 'vb_hash = "unrelated"' > "$CONFIG"
non_vb > "$TEST_ROOT/non-vb.before"
run boot
has "$CONFIG" "vb_hash = \"$HASH2\""
has "$CONFIG" 'vb_key = "auto"'
non_vb > "$TEST_ROOT/non-vb.after"
cmp "$TEST_ROOT/non-vb.before" "$TEST_ROOT/non-vb.after"
ok 'sparse indented trust gets VB fields without editing adjacent sections or old SPL'
no_temps
printf 'PASS: %s pure-host contracts; 5 sh -n syntax checks\n' "$count"
