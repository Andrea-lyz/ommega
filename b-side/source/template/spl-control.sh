#!/system/bin/sh

STATE_DIR=/data/adb/ommega
CONF_FILE=$STATE_DIR/spl.conf
BASELINE_FILE=$STATE_DIR/spl-baseline.conf

SYSTEM_SPL=
BOOT_SPL=
VENDOR_SPL=
BASE_SYSTEM_SPL=
BASE_BOOT_VENDOR_SPL=
BASE_BOOT_IMAGE_SPL=
BASE_VENDOR_SPL=

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
}

write_property() {
  name=$1
  desired=$2
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
  RESETPROP=$(resetprop_bin) || {
    echo "resetprop is unavailable" >&2
    return 1
  }

  desired_system=${SYSTEM_SPL:-$BASE_SYSTEM_SPL}
  desired_vendor=${VENDOR_SPL:-$BASE_VENDOR_SPL}
  desired_boot_vendor=${BOOT_SPL:-$BASE_BOOT_VENDOR_SPL}
  desired_boot_image=${BOOT_SPL:-$BASE_BOOT_IMAGE_SPL}
  changed=0

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
}

save_config() {
  SYSTEM_SPL=$1
  BOOT_SPL=$2
  VENDOR_SPL=$3
  valid_spl "$SYSTEM_SPL" && valid_spl "$BOOT_SPL" && valid_spl "$VENDOR_SPL" || {
    echo "invalid SPL date" >&2
    return 2
  }
  mkdir -p "$STATE_DIR"
  capture_baseline || return 1
  tmp=$CONF_FILE.tmp.$$
  {
    echo "SYSTEM_SPL=$SYSTEM_SPL"
    echo "BOOT_SPL=$BOOT_SPL"
    echo "VENDOR_SPL=$VENDOR_SPL"
  } > "$tmp" || return 1
  chmod 0600 "$tmp" 2>/dev/null || true
  mv "$tmp" "$CONF_FILE" || return 1
  apply_config
}

show_status() {
  load_config
  echo "SYSTEM_SPL=$SYSTEM_SPL"
  echo "BOOT_SPL=$BOOT_SPL"
  echo "VENDOR_SPL=$VENDOR_SPL"
  echo "CURRENT_SYSTEM_SPL=$(getprop ro.build.version.security_patch)"
  echo "CURRENT_BOOT_VENDOR_SPL=$(getprop ro.vendor.boot_security_patch)"
  echo "CURRENT_BOOT_IMAGE_SPL=$(getprop ro.boot.image.build.security_patch)"
  echo "CURRENT_VENDOR_SPL=$(getprop ro.vendor.build.security_patch)"
}

case "$1" in
  apply) apply_config ;;
  save) shift; save_config "$1" "$2" "$3" ;;
  status) show_status ;;
  *) echo "usage: $0 {apply|save <system> <boot> <vendor>|status}" >&2; exit 2 ;;
esac
