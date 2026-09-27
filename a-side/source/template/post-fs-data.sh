MODDIR=${0%/*}
TARGET_DIR=/data/misc/keystore/ommega
LOG_DIR=$TARGET_DIR/logs
TARGET_KEYBOX=$TARGET_DIR/keybox.xml
TARGET_INJECTOR_CONFIG=$TARGET_DIR/injector.toml
TARGET_CONF=$TARGET_DIR/config
TARGET_TARGET_LIST=$TARGET_DIR/target.txt
TARGET_SECURITY_POLICY=$TARGET_DIR/target-security.toml
STATE_DIR=/data/adb/ommega
# A-side config directory (webroot UI writes here; `ommegadata` is a symlink to
# $TARGET_DIR, so the UI and the keystore process (uid 1017) share one copy).
CLIENTA_DIR=/data/adb/ommega
# --- Boot-state property normalization ---------------------------------------
# Values that only an unlocked / tampered device reports are rewritten here, in
# the post-fs-data stage, before the framework or any detector app can read
# them. Only the property area is reachable from here: the bootloader parameters
# the kernel exposes separately (/proc/bootconfig, /proc/cmdline) keep their
# original values and cannot be changed through resetprop.
RESETPROP_BIN="$(command -v resetprop 2>/dev/null)"
if [ -z "$RESETPROP_BIN" ]; then
  for candidate in \
    /data/adb/ksu/bin/resetprop \
    /data/adb/magisk/resetprop \
    /data/adb/ap/bin/resetprop \
    /system_ext/bin/resetprop \
    /system/bin/resetprop
  do
    if [ -x "$candidate" ]; then
      RESETPROP_BIN=$candidate
      break
    fi
  done
fi
# Fall back to the plain name so PATH lookup keeps working as before.
[ -n "$RESETPROP_BIN" ] || RESETPROP_BIN=resetprop

prop_get() {
  local name=$1
  "$RESETPROP_BIN" "$name" 2>/dev/null | head -n 1
}

# Write and read back; a value that did not stick is reported instead of being
# silently assumed, because every one of these properties is a detection signal.
prop_set_verified() {
  local name=$1 want=$2 got
  "$RESETPROP_BIN" "$name" "$want" 2>/dev/null || true
  got=$(prop_get "$name")
  [ "$got" = "$want" ] || echo "ommega: failed to set $name='$want' (got '$got')" >&2
}

prop_delete_verified() {
  local name=$1 got
  "$RESETPROP_BIN" --delete "$name" 2>/dev/null || true
  got=$(prop_get "$name")
  [ -z "$got" ] || echo "ommega: failed to delete $name (still '$got')" >&2
}

# Rewrite a property the bootloader published, or create it when it is missing.
force_prop_or_create() {
  local name=$1 want=$2 cur
  cur=$(prop_get "$name")
  [ "$cur" = "$want" ] || prop_set_verified "$name" "$want"
}

# Rewrite a property only when the bootloader published it: a property that is
# normally absent must stay absent, because its presence is itself an anomaly.
force_prop() {
  local name=$1 want=$2 cur
  cur=$(prop_get "$name")
  [ -n "$cur" ] || return 0
  [ "$cur" = "$want" ] || prop_set_verified "$name" "$want"
}

# Rewrite only when the current value contains a substring (stale boot mode).
replace_prop_if_contains() {
  local name=$1 match=$2 want=$3 cur
  cur=$(prop_get "$name")
  case "$cur" in
    *"$match"*) prop_set_verified "$name" "$want" ;;
  esac
}

# Remove an emulator / mod tell entirely instead of publishing a "safe" value.
delete_prop_if_present() {
  local name=$1
  [ -n "$(prop_get "$name")" ] && prop_delete_verified "$name"
}

# The OPLUS/Goodix fingerprint HAL reads ro.boot.vbmeta.device_state once, while
# it starts, and picks the plain or the wrapped calibration object from it. A
# device that really is unlocked can only read the plain one: once the property
# below claims "locked", the TA tries to unwrap an object the secure world
# refuses, and the factory calibration check (engineering mode -> 器件校准状态)
# reports "cali hash get failed" with status 1036. Starting the HAL here, before
# the properties are rewritten, keeps that check passing while the rest of the
# boot still hides the unlock.
#
# This does not widen the window in which a detector could read the real value:
# it is the same window that already exists between the bootloader and this
# script, and nothing user facing runs this early.
start_fingerprint_hal_early() {
  local binary=/odm/bin/hw/vendor.oplus.hardware.biometrics.fingerprint@2.1-service_uff i=0
  # The HAL's own rc file creates these from an `on boot` trigger that has not
  # run yet this early, so mirror that part here.
  mkdir -p /data/vendor/fingerprint /data/vendor/fingerprint/dump 2>/dev/null
  mkdir -p /data/vendor/fingerprint_ori /data/vendor/fingerprint_ori/dump 2>/dev/null
  chmod 0770 /data/vendor/fingerprint /data/vendor/fingerprint/dump 2>/dev/null
  chmod 0770 /data/vendor/fingerprint_ori /data/vendor/fingerprint_ori/dump 2>/dev/null
  chown system system /data/vendor/fingerprint /data/vendor/fingerprint/dump 2>/dev/null
  chown system system /data/vendor/fingerprint_ori /data/vendor/fingerprint_ori/dump 2>/dev/null
  chown system system /dev/fingerprint_dev 2>/dev/null
  chmod 0666 /dev/fingerprint_dev 2>/dev/null
  # Without the sensor node the HAL cannot come up; init would start it later
  # instead, i.e. after the rewrite. Keep the previous behaviour then (the check
  # stays red, exactly as it was before this change).
  [ -e /dev/fingerprint_dev ] || return 0
  setprop ctl.start fps_hal 2>/dev/null || true
  while [ "$i" -lt 8 ]; do
    pidof "$binary" >/dev/null 2>&1 && break
    sleep 1
    i=$((i + 1))
  done
  [ "$i" -lt 8 ] || return 0
  # It reads the property while opening its storage; let that init finish before
  # the rewrite below.
  sleep 3
}

apply_boot_state_props() {
  # Verified boot / lock state. A locked retail device reports green + locked +
  # enforcing; an unlocked bootloader reports orange / 0 / permissive.
  force_prop_or_create ro.boot.vbmeta.device_state locked
  force_prop_or_create ro.boot.verifiedbootstate   green
  force_prop_or_create ro.boot.flash.locked        1
  force_prop_or_create ro.boot.veritymode          enforcing
  # Vendors that keep a second copy under the vendor namespace.
  force_prop_or_create vendor.boot.vbmeta.device_state locked
  force_prop_or_create vendor.boot.verifiedbootstate   green
  force_prop            vendor.boot.flash.locked        1
  force_prop            vendor.boot.veritymode          enforcing
  # Anti-debug build surface.
  force_prop_or_create ro.secure           1
  force_prop_or_create ro.adb.secure       1
  force_prop_or_create ro.debuggable       0
  force_prop           ro.force.debuggable 0
  force_prop           ro.build.type       user
  force_prop           ro.build.tags       release-keys
  # User-data encryption state.
  force_prop ro.crypto.state encrypted
  # A stale recovery boot mode or an emulator marker is a mod-detection tell.
  replace_prop_if_contains ro.bootmode          recovery unknown
  replace_prop_if_contains ro.boot.bootmode     recovery unknown
  replace_prop_if_contains vendor.boot.bootmode recovery unknown
  delete_prop_if_present   ro.kernel.qemu
  # Android 16+ Duck Detector treats any observed sys.oem_unlock_allowed as a
  # tell (dangerousValues="*"). Forcing 0 still leaves the property visible.
  # Delete it instead of publishing a "safe" value.
  delete_prop_if_present sys.oem_unlock_allowed
}

prop_set_verified persist.logd.size ""
prop_set_verified persist.logd.size.crash ""
prop_set_verified persist.logd.size.system ""
prop_set_verified persist.logd.size.main ""
# The fingerprint HAL has to be up before the lock state is rewritten.
start_fingerprint_hal_early
apply_boot_state_props
mkdir -p "$TARGET_DIR"
chmod 0770 "$TARGET_DIR"
chown 1017:1017 "$TARGET_DIR"
mkdir -p "$LOG_DIR"
chmod 0770 "$LOG_DIR"
chown 1017:1017 "$LOG_DIR"
mkdir -p "$STATE_DIR"
rm -f "$STATE_DIR/keymint-daemon.pid" "$STATE_DIR/injector-daemon.pid"
rm -f "$STATE_DIR/restart.keymint" "$STATE_DIR/restart.injector" "$STATE_DIR/restart.all"

# Make the shared A-side config directory traversable and expose the data dir.
mkdir -p "$CLIENTA_DIR"
chmod 0755 "$CLIENTA_DIR"
if [ ! -e "$CLIENTA_DIR/ommegadata" ]; then
  ln -s "$TARGET_DIR" "$CLIENTA_DIR/ommegadata" 2>/dev/null
fi

# Single data location for the flat A-side config and per-app target list.
# The webroot UI writes these through the `ommegadata` symlink, so no copy is
# needed.  Seed them (empty defaults) so the keystore process always has files.
if [ ! -f "$TARGET_CONF" ]; then
  : > "$TARGET_CONF"
fi
if [ ! -f "$TARGET_TARGET_LIST" ]; then
  : > "$TARGET_TARGET_LIST"
fi
if grep -Eq '[!?][[:space:]]*$' "$TARGET_TARGET_LIST" 2>/dev/null; then
  tmp=$TARGET_TARGET_LIST.tmp.$$
  awk '
    /^[[:space:]]*($|#|\[)/ { print; next }
    { sub(/[!?][[:space:]]*$/, ""); print }
  ' "$TARGET_TARGET_LIST" | sort -u > "$tmp" && mv "$tmp" "$TARGET_TARGET_LIST"
fi
if [ ! -f "$TARGET_SECURITY_POLICY" ]; then
  printf 'version = 1\n\n[packages]\n' > "$TARGET_SECURITY_POLICY"
fi
chmod 0644 "$TARGET_CONF" "$TARGET_TARGET_LIST" "$TARGET_SECURITY_POLICY" 2>/dev/null || true
chown 1017:1017 "$TARGET_CONF" "$TARGET_TARGET_LIST" "$TARGET_SECURITY_POLICY" 2>/dev/null || true

# Keybox: the webroot UI writes `/data/adb/ommega/ommegadata/keybox.xml` which IS
# $TARGET_KEYBOX (via symlink).  Seed from the module keybox if absent.
if [ ! -f "$TARGET_KEYBOX" ] && [ -f "$MODDIR/keybox.xml" ]; then
  cp "$MODDIR/keybox.xml" "$TARGET_KEYBOX"
fi

if [ ! -f "$TARGET_INJECTOR_CONFIG" ] && [ -f "$MODDIR/injector.toml" ]; then
  cp "$MODDIR/injector.toml" "$TARGET_INJECTOR_CONFIG"
fi

if [ -f "$TARGET_KEYBOX" ]; then
  chmod 0600 "$TARGET_KEYBOX"
  chown 1017:1017 "$TARGET_KEYBOX"
fi

if [ -f "$TARGET_INJECTOR_CONFIG" ]; then
  chmod 0600 "$TARGET_INJECTOR_CONFIG"
  chown 1017:1017 "$TARGET_INJECTOR_CONFIG"
fi

# WebUI overlay props (security patch, vbmeta digest, vbmeta public key).
# Stored in the data dir so Magisk/KSU overlay installs do not wipe them.
WEBUI_PROPS=$TARGET_DIR/webui-props.sh
if [ -f "$WEBUI_PROPS" ]; then
  chmod 0644 "$WEBUI_PROPS" 2>/dev/null || true
  chown 1017:1017 "$WEBUI_PROPS" 2>/dev/null || true
  . "$WEBUI_PROPS"
fi

# Mirror overlay props into config.toml [trust] so attestation tags match
# the system properties the WebUI set. Skip when keymint has not created
# the file yet (service.sh retries after the daemon starts).
if [ -f "$MODDIR/webui-trust.sh" ] && [ -f "$TARGET_DIR/config.toml" ]; then
  sh "$MODDIR/webui-trust.sh" sync
fi
