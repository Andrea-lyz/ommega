#!/system/bin/sh

# Overridable only so the host-side test can sandbox this script; nothing on
# device sets it, so the deployed path is always the default.
STATE_DIR=${OMMEGA_STATE_DIR:-/data/adb/ommega}
CONF_FILE=$STATE_DIR/spl.conf
BASELINE_FILE=$STATE_DIR/spl-baseline.conf

SYSTEM_SPL=
BOOT_SPL=
VENDOR_SPL=
OS_VERSION=
BASE_SYSTEM_SPL=
BASE_BOOT_VENDOR_SPL=
BASE_BOOT_IMAGE_SPL=
BASE_VENDOR_SPL=
BASE_OS_VERSION=
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

SPL_DOWNGRADE_MARKER=$STATE_DIR/allow-spl-downgrade

# Security patch levels and the OS release are one-way ratchets for the
# hardware KeyMint: a key blob minted - or upgraded - while the device reported
# a newer value refuses to be upgraded once the device reports an older one, so
# every later sign and attestation for those keys fails with
# INVALID_ARGUMENT (-38) and nothing on the module side can undo it. A stored
# baseline is not a safe value either: it may have been recorded while an
# override was already active.
is_older_value() {
  # $1 = current value, $2 = desired value; both are non-empty.
  case "$1$2" in
    *[!0-9]*) [ "$2" \< "$1" ] ;;
    *) [ "$2" -lt "$1" ] ;;
  esac
}

refuses_downgrade() {
  # $1 = property name, $2 = desired value.
  [ -f "$SPL_DOWNGRADE_MARKER" ] && return 1
  current=$(getprop "$1")
  [ -n "$current" ] && [ -n "$2" ] || return 1
  is_older_value "$current" "$2" || return 1
  return 0
}

write_property() {
  name=$1
  desired=$2
  if refuses_downgrade "$name" "$desired"; then
    echo "refused to lower $name from $(getprop "$name") to $desired: patch levels and the OS version are one-way for the hardware KeyMint, and lowering them permanently breaks existing key blobs on this device (later signs and attestations fail with INVALID_ARGUMENT/-38). Keep the newer value, or create $SPL_DOWNGRADE_MARKER to force this one change." >&2
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
  load_baseline || return 1
  DOWNGRADE_REFUSED=
  RESETPROP=$(resetprop_bin) || {
    echo "resetprop is unavailable" >&2
    return 1
  }

  desired_system=${SYSTEM_SPL:-$BASE_SYSTEM_SPL}
  desired_vendor=${VENDOR_SPL:-$BASE_VENDOR_SPL}
  desired_boot_vendor=${BOOT_SPL:-$BASE_BOOT_VENDOR_SPL}
  desired_boot_image=${BOOT_SPL:-$BASE_BOOT_IMAGE_SPL}
  desired_os_version=${OS_VERSION:-$BASE_OS_VERSION}
  changed=0

  # A baseline recorded before OS version support existed has no OS_VERSION
  # line, so an upgrade has nothing to fall back to. The first time an
  # override is applied, record the live release as the baseline first: it is
  # still the device's own value at this point because nothing has replaced
  # it yet. Without this, clearing the field later would have no value to
  # restore.
  if [ -n "$OS_VERSION" ] && [ -z "$BASE_OS_VERSION" ]; then
    current_release=$(getprop ro.build.version.release)
    if [ -n "$current_release" ]; then
      printf 'OS_VERSION=%s\n' "$current_release" >> "$BASELINE_FILE" || return 1
      BASE_OS_VERSION=$current_release
    fi
  fi

  # Release first: the KeyMint HAL reads ro.build.version.release when it
  # starts, so the value has to be in place before the restart at the end of
  # this function, and a release change alone must trigger that restart.
  # Only touch it when a value is known. Unlike the SPL properties, an empty
  # release here means "no baseline recorded and nothing configured", and
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
  echo "SYSTEM_SPL=$SYSTEM_SPL"
  echo "BOOT_SPL=$BOOT_SPL"
  echo "VENDOR_SPL=$VENDOR_SPL"
  echo "OS_VERSION=$OS_VERSION"
  echo "CURRENT_SYSTEM_SPL=$(getprop ro.build.version.security_patch)"
  echo "CURRENT_BOOT_VENDOR_SPL=$(getprop ro.vendor.boot_security_patch)"
  echo "CURRENT_BOOT_IMAGE_SPL=$(getprop ro.boot.image.build.security_patch)"
  echo "CURRENT_VENDOR_SPL=$(getprop ro.vendor.build.security_patch)"
  echo "CURRENT_OS_VERSION=$(getprop ro.build.version.release)"
}

case "$1" in
  apply) apply_config ;;
  save) shift; save_config "$1" "$2" "$3" "$4" ;;
  status) show_status ;;
  *) echo "usage: $0 {apply|save <system> <boot> <vendor> <os_version>|status}" >&2; exit 2 ;;
esac
