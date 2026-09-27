#!/system/bin/sh

# Overridable only so the host-side test can sandbox this script; nothing on
# device sets it, so the deployed path is always the default.
STATE_DIR=${OMMEGA_STATE_DIR:-/data/adb/ommega}
CONF_FILE=$STATE_DIR/spl.conf
BASELINE_FILE=$STATE_DIR/spl-baseline.conf
ORIGIN_FILE=$STATE_DIR/spl-origin.conf

# Read-only property files that still carry the values the installed firmware
# shipped with. resetprop only rewrites the in-memory property copy, so these
# files are the device's real original patch levels and release - the values a
# configured override must never go below. Overridable only so the host-side
# test can sandbox them.
PROP_FILES=${OMMEGA_PROP_FILES:-"/system/build.prop /system/system/build.prop /system_ext/build.prop /product/build.prop /vendor/build.prop /odm/build.prop /odm/etc/build.prop /my_product/build.prop /my_heytap/build.prop"}

SYSTEM_SPL=
BOOT_SPL=
VENDOR_SPL=
OS_VERSION=
BASE_SYSTEM_SPL=
BASE_BOOT_VENDOR_SPL=
BASE_BOOT_IMAGE_SPL=
BASE_VENDOR_SPL=
BASE_OS_VERSION=
ORIGIN_SYSTEM_SPL=
ORIGIN_BOOT_VENDOR_SPL=
ORIGIN_BOOT_IMAGE_SPL=
ORIGIN_VENDOR_SPL=
ORIGIN_OS_VERSION=
DOWNGRADE_REFUSED=
SAVE_IN_PROGRESS=

resetprop_bin() {
  if command -v resetprop >/dev/null 2>&1; then
    command -v resetprop
  elif [ -x /data/adb/ksu/bin/resetprop ]; then
    echo /data/adb/ksu/bin/resetprop
  elif [ -x /data/adb/ksud ]; then
    echo "/data/adb/ksud resetprop"
  else
    return 1
  fi
}

valid_spl() {
  [ -z "$1" ] || echo "$1" | grep -Eq '^[0-9]{4}-(0[1-9]|1[0-2])-([0-2][0-9]|3[01])$'
}

# Android major version, 1-2 digits. Empty keeps the device value. Android
# majors are single digits today; two are accepted so a future release does
# not need a code change.
valid_os_version() {
  [ -z "$1" ] || echo "$1" | grep -Eq '^[0-9]{1,2}$'
}

read_key_file() {
  file=$1
  key=$2
  [ -r "$file" ] || return 0
  sed -n "s/^${key}=//p" "$file" | tail -n 1
}

load_config() {
  SYSTEM_SPL=$(read_key_file "$CONF_FILE" SYSTEM_SPL)
  BOOT_SPL=$(read_key_file "$CONF_FILE" BOOT_SPL)
  VENDOR_SPL=$(read_key_file "$CONF_FILE" VENDOR_SPL)
  OS_VERSION=$(read_key_file "$CONF_FILE" OS_VERSION)
}

capture_baseline() {
  [ -f "$BASELINE_FILE" ] && return 0
  mkdir -p "$STATE_DIR"
  tmp=$BASELINE_FILE.tmp.$$
  {
    echo "SYSTEM_SPL=$(getprop ro.build.version.security_patch)"
    echo "BOOT_VENDOR_SPL=$(getprop ro.vendor.boot_security_patch)"
    echo "BOOT_IMAGE_SPL=$(getprop ro.boot.image.build.security_patch)"
    echo "VENDOR_SPL=$(getprop ro.vendor.build.security_patch)"
    echo "OS_VERSION=$(getprop ro.build.version.release)"
  } > "$tmp" || return 1
  chmod 0600 "$tmp" 2>/dev/null || true
  mv "$tmp" "$BASELINE_FILE"
}

load_baseline() {
  capture_baseline || return 1
  BASE_SYSTEM_SPL=$(read_key_file "$BASELINE_FILE" SYSTEM_SPL)
  BASE_BOOT_VENDOR_SPL=$(read_key_file "$BASELINE_FILE" BOOT_VENDOR_SPL)
  BASE_BOOT_IMAGE_SPL=$(read_key_file "$BASELINE_FILE" BOOT_IMAGE_SPL)
  BASE_VENDOR_SPL=$(read_key_file "$BASELINE_FILE" VENDOR_SPL)
  BASE_OS_VERSION=$(read_key_file "$BASELINE_FILE" OS_VERSION)
}

# Security patch levels and the OS release are one-way ratchets for the
# hardware KeyMint: a key blob minted - or upgraded - while the device reported
# a newer value refuses to be upgraded once the device reports an older one, so
# every later sign and attestation for those keys fails with
# INVALID_ARGUMENT (-38) and nothing on the module side can undo it.
#
# The module therefore lets a value move in both directions, but never below
# the device's own original value. That original value is read from the
# read-only firmware property files, because the live property may already
# carry an override and a recorded baseline may have been captured while one
# was active.
is_older_value() {
  # $1 = current value, $2 = desired value; both are non-empty.
  case "$1$2" in
    *[!0-9]*) [ "$2" \< "$1" ] ;;
    *) [ "$2" -lt "$1" ] ;;
  esac
}

# $1 = recorded value, $2 = freshly read value; true when $2 is strictly newer.
is_newer_value() {
  case "$1$2" in
    *[!0-9]*) [ "$2" \> "$1" ] ;;
    *) [ "$2" -gt "$1" ] ;;
  esac
}

# Echoes the newer of the two values; either may be empty.
keep_newest() {
  if [ -z "$1" ] || [ -z "$2" ]; then
    echo "${1:-$2}"
  elif is_newer_value "$1" "$2"; then
    echo "$2"
  else
    echo "$1"
  fi
}

# Echoes the newest value the firmware files carry for the property named in
# $1, or nothing when none of them can be read. The newest reading wins, so a
# stale copy in a lower-priority file cannot weaken the floor.
prop_file_value() {
  name=$1
  best=
  for file in $PROP_FILES; do
    [ -r "$file" ] || continue
    found=$(sed -n "s/^${name}=//p" "$file" 2>/dev/null | tail -n 1)
    [ -n "$found" ] || continue
    best=$(keep_newest "$best" "$found")
  done
  [ -n "$best" ] || return 1
  echo "$best"
}

load_origin() {
  ORIGIN_SYSTEM_SPL=$(read_key_file "$ORIGIN_FILE" SYSTEM_SPL)
  ORIGIN_BOOT_VENDOR_SPL=$(read_key_file "$ORIGIN_FILE" BOOT_VENDOR_SPL)
  ORIGIN_BOOT_IMAGE_SPL=$(read_key_file "$ORIGIN_FILE" BOOT_IMAGE_SPL)
  ORIGIN_VENDOR_SPL=$(read_key_file "$ORIGIN_FILE" VENDOR_SPL)
  ORIGIN_OS_VERSION=$(read_key_file "$ORIGIN_FILE" OS_VERSION)
}

save_origin() {
  mkdir -p "$STATE_DIR" 2>/dev/null || return 0
  tmp=$ORIGIN_FILE.tmp.$$
  {
    echo "SYSTEM_SPL=$ORIGIN_SYSTEM_SPL"
    echo "BOOT_VENDOR_SPL=$ORIGIN_BOOT_VENDOR_SPL"
    echo "BOOT_IMAGE_SPL=$ORIGIN_BOOT_IMAGE_SPL"
    echo "VENDOR_SPL=$ORIGIN_VENDOR_SPL"
    echo "OS_VERSION=$ORIGIN_OS_VERSION"
  } > "$tmp" 2>/dev/null || return 0
  chmod 0600 "$tmp" 2>/dev/null || true
  mv "$tmp" "$ORIGIN_FILE" 2>/dev/null || rm -f "$tmp"
}

# Records the device's own values the first time they can be read and never
# lets a later read lower them: a firmware update raises the floor, while a
# file that cannot be read right now cannot drop one that is already known.
refresh_origin() {
  load_origin
  changed=0
  settled=$(keep_newest "$ORIGIN_SYSTEM_SPL" "$(prop_file_value ro.build.version.security_patch)")
  [ "$settled" = "$ORIGIN_SYSTEM_SPL" ] || { ORIGIN_SYSTEM_SPL=$settled; changed=1; }
  settled=$(keep_newest "$ORIGIN_BOOT_VENDOR_SPL" "$(prop_file_value ro.vendor.boot_security_patch)")
  [ "$settled" = "$ORIGIN_BOOT_VENDOR_SPL" ] || { ORIGIN_BOOT_VENDOR_SPL=$settled; changed=1; }
  settled=$(keep_newest "$ORIGIN_BOOT_IMAGE_SPL" "$(prop_file_value ro.boot.image.build.security_patch)")
  [ "$settled" = "$ORIGIN_BOOT_IMAGE_SPL" ] || { ORIGIN_BOOT_IMAGE_SPL=$settled; changed=1; }
  settled=$(keep_newest "$ORIGIN_VENDOR_SPL" "$(prop_file_value ro.vendor.build.security_patch)")
  [ "$settled" = "$ORIGIN_VENDOR_SPL" ] || { ORIGIN_VENDOR_SPL=$settled; changed=1; }
  settled=$(keep_newest "$ORIGIN_OS_VERSION" "$(prop_file_value ro.build.version.release)")
  [ "$settled" = "$ORIGIN_OS_VERSION" ] || { ORIGIN_OS_VERSION=$settled; changed=1; }
  [ "$changed" -eq 0 ] || save_origin
  return 0
}

# The device's own value for one property, used to restore an emptied field.
device_value() {
  case "$1" in
    ro.build.version.release) echo "$ORIGIN_OS_VERSION" ;;
    ro.vendor.build.security_patch) echo "$ORIGIN_VENDOR_SPL" ;;
    ro.vendor.boot_security_patch) echo "$ORIGIN_BOOT_VENDOR_SPL" ;;
    ro.boot.image.build.security_patch) echo "$ORIGIN_BOOT_IMAGE_SPL" ;;
    *) echo "$ORIGIN_SYSTEM_SPL" ;;
  esac
}

baseline_value() {
  case "$1" in
    ro.build.version.release) echo "$BASE_OS_VERSION" ;;
    ro.vendor.build.security_patch) echo "$BASE_VENDOR_SPL" ;;
    ro.vendor.boot_security_patch) echo "$BASE_BOOT_VENDOR_SPL" ;;
    ro.boot.image.build.security_patch) echo "$BASE_BOOT_IMAGE_SPL" ;;
    *) echo "$BASE_SYSTEM_SPL" ;;
  esac
}

# What an emptied configuration field restores: the device's own value, or the
# baseline when the firmware files could not be read at all.
restore_value() {
  value=$(device_value "$1")
  [ -n "$value" ] || value=$(baseline_value "$1")
  echo "$value"
}

# The lowest value a write may use for the property named in $1. A patch level
# without a readable original is held above the device's own system patch
# level, so a boot or vendor override can never pull the reported date below
# what the firmware shipped with.
floor_for() {
  floor=$(device_value "$1")
  if [ -z "$floor" ]; then
    case "$1" in
      ro.build.version.release) floor=$BASE_OS_VERSION ;;
      *) floor=${ORIGIN_SYSTEM_SPL:-$BASE_SYSTEM_SPL} ;;
    esac
  fi
  echo "$floor"
}

refuses_downgrade() {
  # $1 = property name, $2 = desired value.
  [ -n "$2" ] || return 1
  floor=$(floor_for "$1")
  [ -n "$floor" ] || return 1
  is_older_value "$floor" "$2" || return 1
  return 0
}

write_property() {
  name=$1
  desired=$2
  if refuses_downgrade "$name" "$desired"; then
    echo "refused to set $name to $desired: this device's original value is $floor, and an override may not go below it. Patch levels and the OS version are one-way ratchets for the hardware KeyMint, and a key blob minted or upgraded while the device reported a newer value stays unusable once the device reports an older one (later signs and attestations fail with INVALID_ARGUMENT/-38). Set $floor or newer, or leave the field empty to restore $floor." >&2
    DOWNGRADE_REFUSED=1
    return 1
  fi
  current=$(getprop "$name")
  [ "$current" = "$desired" ] && return 1
  if [ -n "$desired" ]; then
    $RESETPROP -n "$name" "$desired" || return 2
  else
    $RESETPROP --delete "$name" 2>/dev/null || true
  fi
  [ "$(getprop "$name")" = "$desired" ] || return 2
  return 0
}

restart_keymint_stack() {
  services=$(getprop | awk -F'[][]' '
    $2 ~ /^init\.svc\./ && $2 ~ /(keymint|keymaster)/ && $4 == "running" {
      sub(/^init\.svc\./, "", $2); print $2
    }
  ')
  [ -n "$services" ] || {
    echo "no running KeyMint/Keymaster init service was discovered" >&2
    return 1
  }
  for service_name in $services; do
    setprop ctl.restart "$service_name" || return 1
  done
  setprop ctl.restart keystore2 || return 1

  tries=0
  while [ "$tries" -lt 60 ]; do
    if service check android.hardware.security.keymint.IKeyMintDevice/default 2>/dev/null \
      | grep -q 'found'; then
      return 0
    fi
    sleep 0.5
    tries=$((tries + 1))
  done
  echo "KeyMint Binder did not recover" >&2
  return 1
}

apply_config() {
  load_config
  refresh_origin
  load_baseline || return 1
  DOWNGRADE_REFUSED=
  RESETPROP=$(resetprop_bin) || {
    echo "resetprop is unavailable" >&2
    return 1
  }

  # A baseline recorded before OS version support existed has no OS_VERSION
  # line, so an emptied release field would have nothing to restore when the
  # firmware files cannot be read either. The first time an override is
  # applied, record the live release as the baseline: nothing has replaced it
  # yet at this point.
  if [ -n "$OS_VERSION" ] && [ -z "$BASE_OS_VERSION" ]; then
    current_release=$(getprop ro.build.version.release)
    if [ -n "$current_release" ]; then
      printf 'OS_VERSION=%s\n' "$current_release" >> "$BASELINE_FILE" || return 1
      BASE_OS_VERSION=$current_release
    fi
  fi

  # An emptied field restores the device's own value, never the live property:
  # the live value may be this module's own previous override.
  desired_system=${SYSTEM_SPL:-$(restore_value ro.build.version.security_patch)}
  desired_vendor=${VENDOR_SPL:-$(restore_value ro.vendor.build.security_patch)}
  desired_boot_vendor=${BOOT_SPL:-$(restore_value ro.vendor.boot_security_patch)}
  desired_boot_image=${BOOT_SPL:-$(restore_value ro.boot.image.build.security_patch)}
  desired_os_version=${OS_VERSION:-$(restore_value ro.build.version.release)}
  changed=0

  # Release first: the KeyMint HAL reads ro.build.version.release when it
  # starts, so the value has to be in place before the restart at the end of
  # this function, and a release change alone must trigger that restart.
  # Only touch it when a value is known: unlike the SPL properties, an empty
  # release here means "nothing configured and nothing recorded", and
  # write_property would delete the property in that case, which no device
  # should have happen.
  if [ -n "$desired_os_version" ]; then
    write_property ro.build.version.release "$desired_os_version"
    rc=$?
    [ "$rc" -eq 2 ] && return 1
    [ "$rc" -eq 0 ] && changed=1
  fi
  write_property ro.build.version.security_patch "$desired_system"
  rc=$?
  [ "$rc" -eq 2 ] && return 1
  [ "$rc" -eq 0 ] && changed=1
  write_property ro.vendor.build.security_patch "$desired_vendor"
  rc=$?
  [ "$rc" -eq 2 ] && return 1
  [ "$rc" -eq 0 ] && changed=1
  write_property ro.vendor.boot_security_patch "$desired_boot_vendor"
  rc=$?
  [ "$rc" -eq 2 ] && return 1
  [ "$rc" -eq 0 ] && changed=1
  write_property ro.boot.image.build.security_patch "$desired_boot_image"
  rc=$?
  [ "$rc" -eq 2 ] && return 1
  [ "$rc" -eq 0 ] && changed=1

  [ "$changed" -eq 0 ] || restart_keymint_stack

  # A refusal during an interactive save must reach the WebUI; the boot-time
  # apply only reports it on stderr so a persisted downgrade cannot fail the
  # module startup path.
  if [ -n "$DOWNGRADE_REFUSED" ] && [ -n "$SAVE_IN_PROGRESS" ]; then
    return 3
  fi
}

save_config() {
  SYSTEM_SPL=$1
  BOOT_SPL=$2
  VENDOR_SPL=$3
  OS_VERSION=$4
  valid_spl "$SYSTEM_SPL" && valid_spl "$BOOT_SPL" && valid_spl "$VENDOR_SPL" \
    && valid_os_version "$OS_VERSION" || {
    echo "invalid SPL date or OS version" >&2
    return 2
  }
  mkdir -p "$STATE_DIR"
  capture_baseline || return 1
  tmp=$CONF_FILE.tmp.$$
  {
    echo "SYSTEM_SPL=$SYSTEM_SPL"
    echo "BOOT_SPL=$BOOT_SPL"
    echo "VENDOR_SPL=$VENDOR_SPL"
    echo "OS_VERSION=$OS_VERSION"
  } > "$tmp" || return 1
  chmod 0600 "$tmp" 2>/dev/null || true
  mv "$tmp" "$CONF_FILE" || return 1
  SAVE_IN_PROGRESS=1
  apply_config
}

show_status() {
  load_config
  # Record the device's own values here as well, so the WebUI can show the
  # floor before the first save. Reading the recorded baseline directly keeps
  # status from capturing a live baseline as a side effect.
  refresh_origin
  BASE_SYSTEM_SPL=$(read_key_file "$BASELINE_FILE" SYSTEM_SPL)
  BASE_OS_VERSION=$(read_key_file "$BASELINE_FILE" OS_VERSION)
  echo "SYSTEM_SPL=$SYSTEM_SPL"
  echo "BOOT_SPL=$BOOT_SPL"
  echo "VENDOR_SPL=$VENDOR_SPL"
  echo "OS_VERSION=$OS_VERSION"
  echo "CURRENT_SYSTEM_SPL=$(getprop ro.build.version.security_patch)"
  echo "CURRENT_BOOT_VENDOR_SPL=$(getprop ro.vendor.boot_security_patch)"
  echo "CURRENT_BOOT_IMAGE_SPL=$(getprop ro.boot.image.build.security_patch)"
  echo "CURRENT_VENDOR_SPL=$(getprop ro.vendor.build.security_patch)"
  echo "CURRENT_OS_VERSION=$(getprop ro.build.version.release)"
  echo "ORIGIN_SYSTEM_SPL=$(device_value ro.build.version.security_patch)"
  echo "ORIGIN_VENDOR_SPL=$(device_value ro.vendor.build.security_patch)"
  echo "ORIGIN_OS_VERSION=$(device_value ro.build.version.release)"
  echo "FLOOR_SYSTEM_SPL=$(floor_for ro.build.version.security_patch)"
  echo "FLOOR_VENDOR_SPL=$(floor_for ro.vendor.build.security_patch)"
  echo "FLOOR_OS_VERSION=$(floor_for ro.build.version.release)"
}

case "$1" in
  apply) apply_config ;;
  save) shift; save_config "$1" "$2" "$3" "$4" ;;
  status) show_status ;;
  *) echo "usage: $0 {apply|save <system> <boot> <vendor> <os_version>|status}" >&2; exit 2 ;;
esac
