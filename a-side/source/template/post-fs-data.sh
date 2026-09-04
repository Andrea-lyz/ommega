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
resetprop persist.logd.size ""
resetprop persist.logd.size.crash ""
resetprop persist.logd.size.system ""
resetprop persist.logd.size.main ""
resetprop ro.boot.flash.locked 1
resetprop ro.boot.verifiedbootstate green
resetprop ro.boot.veritymode enforcing
resetprop ro.boot.vbmeta.device_state locked
resetprop ro.secure 1
resetprop ro.adb.secure 1
resetprop ro.debuggable 0
# Android 16+ Duck Detector treats any observed sys.oem_unlock_allowed as a
# tell (dangerousValues="*"). Forcing 0 still leaves the property visible.
# Delete it instead of publishing a "safe" value.
resetprop --delete sys.oem_unlock_allowed 2>/dev/null || true
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
