MODDIR=${0%/*}
STATE_DIR=/data/adb/ommega

mkdir -p "$STATE_DIR" "$STATE_DIR/logs"
rm -f "$STATE_DIR/restart.all"

# First-install only: drop the relay config template so the relay daemon
# (B-side, remote TEE attestation) actually has settings to load.  Edit
# /data/adb/ommega/relay.conf after install to point at the real relay_server.
TARGET_RELAY_CONFIG=$STATE_DIR/relay.conf
if [ ! -f "$TARGET_RELAY_CONFIG" ] && [ -f "$MODDIR/relay.conf" ]; then
  cp "$MODDIR/relay.conf" "$TARGET_RELAY_CONFIG"
  chmod 0600 "$TARGET_RELAY_CONFIG"
fi
