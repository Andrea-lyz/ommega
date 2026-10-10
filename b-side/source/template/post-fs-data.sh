#!/system/bin/sh
# Fill new supported configuration keys without replacing existing values.
# No service restart, SPL apply, network binding change or HAL operation.
MODDIR=${0%/*}
STATE_DIR=/data/adb/ommega
CONF_FILE=$STATE_DIR/relay.conf
mkdir -p "$STATE_DIR"
chmod 0700 "$STATE_DIR" 2>/dev/null || true
if [ ! -f "$CONF_FILE" ] && [ -f "$MODDIR/relay.conf" ]; then
  cp "$MODDIR/relay.conf" "$CONF_FILE"
fi
if [ -f "$CONF_FILE" ] && [ -f "$MODDIR/relay.conf" ]; then
  while IFS= read -r line; do
    case "$line" in ''|'#'*) continue ;; esac
    key=${line%%=*}
    value=${line#*=}
    case "$key" in OMMEGA_*) ;; *) continue ;; esac
    case "$key" in OMMEGA_RELAY_BIND_IFACE) continue ;; esac
    case "$value" in *'<'*) continue ;; esac
    if ! grep -q "^[[:space:]]*${key}[[:space:]]*=" "$CONF_FILE" 2>/dev/null; then
      printf '\n%s\n' "$line" >> "$CONF_FILE"
    fi
  done < "$MODDIR/relay.conf"
fi
chmod 0600 "$CONF_FILE" 2>/dev/null || true
