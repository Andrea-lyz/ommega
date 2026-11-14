#!/system/bin/sh
# Only Verified Boot hash/key overlays belong to A. SPL is never applied here.
# Historical overlay files are data, not shell programs: never source them.

TARGET_DIR=/data/misc/keystore/ommega
WEBUI_PROPS=$TARGET_DIR/webui-props.sh
CONFIG_TOML=$TARGET_DIR/config.toml
RESTART_FLAG=/data/adb/ommega/restart.keymint

VBMETA_HASH=
VBMETA_KEY=

hex64() {
  [ "${#1}" -eq 64 ] || return 1
  case "$1" in *[!0-9a-f]*) return 1 ;; esac
}

load_props() {
  local file lines name value
  file=${1:-$WEBUI_PROPS}
  VBMETA_HASH=
  VBMETA_KEY=
  [ -f "$file" ] || return 0
  # Accept only literal 32-byte hex values, optionally quoted, with no shell
  # suffix, expansion, redirection or command. Old SPL/other lines are ignored.
  lines=$(awk '
    { sub(/\r$/, "") }
    $1 == "resetprop" {
      i=2
      if ($i == "-n") i++
      if (NF != i+1) next
      name=$i
      if (name != "ro.boot.vbmeta.digest" &&
          name != "ro.boot.vbmeta.public_key_digest") next
      value=$(i+1)
      quote=substr(value,1,1)
      if ((quote == "\"" || quote == sprintf("%c",39)) &&
          substr(value,length(value),1) == quote)
        value=substr(value,2,length(value)-2)
      if (length(value) == 64 && value !~ /[^0-9a-fA-F]/)
        print name " " tolower(value)
    }
  ' "$file") || return 1
  while IFS=' ' read -r name value; do
    case "$name" in
      ro.boot.vbmeta.digest) VBMETA_HASH=$value ;;
      ro.boot.vbmeta.public_key_digest) VBMETA_KEY=$value ;;
    esac
  done <<EOF
$lines
EOF
}

write_props() {
  local tmp
  mkdir -p "$TARGET_DIR" || return 1
  tmp=$WEBUI_PROPS.tmp.$$
  if ! (
    printf '%s\n' '# ommega WebUI Verified Boot overrides (data only)' || exit 1
    if [ -n "$VBMETA_HASH" ]; then
      printf 'resetprop ro.boot.vbmeta.digest %s\n' "$VBMETA_HASH" || exit 1
    fi
    if [ -n "$VBMETA_KEY" ]; then
      printf 'resetprop ro.boot.vbmeta.public_key_digest %s\n' "$VBMETA_KEY" || exit 1
    fi
  ) > "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  if ! chmod 0644 "$tmp" || ! chown 1017:1017 "$tmp" ||
     ! mv "$tmp" "$WEBUI_PROPS"; then
    rm -f "$tmp"
    return 1
  fi
}

migrate_props() {
  local file
  # An existing file is authoritative, including a file cleared by the user.
  # Never resurrect cleared values from an older module or layout.
  [ -f "$WEBUI_PROPS" ] && return 0
  for file in "$@"; do
    load_props "$file" || return 1
    [ -n "$VBMETA_HASH$VBMETA_KEY" ] || continue
    write_props || return 1
    return 0
  done
  return 0
}

sync_toml() {
  local tmp
  [ -f "$CONFIG_TOML" ] || return 0
  tmp=$CONFIG_TOML.tmp.$$
  # Update both VB fields atomically. All other trust fields (especially old
  # SPL settings) and all other sections are preserved, not reset to auto.
  if ! awk -v hash="${VBMETA_HASH:-auto}" -v pubkey="${VBMETA_KEY:-auto}" '
    function missing() {
      if (!hash_done) print "vb_hash = \"" hash "\""
      if (!key_done) print "vb_key = \"" pubkey "\""
      hash_done=key_done=1
    }
    /^[[:space:]]*\[trust\][[:space:]]*(#.*)?$/ {
      seen=1; in_trust=1; print; next
    }
    /^[[:space:]]*\[/ {
      if (in_trust) missing()
      in_trust=0
    }
    in_trust && /^[[:space:]]*vb_hash[[:space:]]*=/ {
      if (!hash_done) print "vb_hash = \"" hash "\""
      hash_done=1; next
    }
    in_trust && /^[[:space:]]*vb_key[[:space:]]*=/ {
      if (!key_done) print "vb_key = \"" pubkey "\""
      key_done=1; next
    }
    { print }
    END {
      if (!seen) print "\n[trust]"
      if (in_trust || !seen) missing()
    }
  ' "$CONFIG_TOML" > "$tmp"; then
    rm -f "$tmp"
    return 1
  fi
  if cmp -s "$tmp" "$CONFIG_TOML"; then
    rm -f "$tmp"
    return 0
  fi
  if ! chmod 0644 "$tmp" || ! chown 1017:1017 "$tmp" ||
     ! mv "$tmp" "$CONFIG_TOML"; then
    rm -f "$tmp"
    return 1
  fi
}

apply_live() {
  local name value
  case "$1" in
    hash) name=ro.boot.vbmeta.digest; value=$VBMETA_HASH ;;
    key) name=ro.boot.vbmeta.public_key_digest; value=$VBMETA_KEY ;;
    *) return 2 ;;
  esac
  if [ -n "$value" ]; then
    resetprop -n "$name" "$value"
  else
    resetprop --delete "$name"
  fi
}

request_keymint_restart() {
  local old_pid new_pid tries
  mkdir -p "${RESTART_FLAG%/*}" || return 1
  old_pid=$(pidof keymint 2>/dev/null | awk '{print $1}')
  touch "$RESTART_FLAG" || return 1
  tries=0
  while [ "$tries" -lt 120 ]; do
    new_pid=$(pidof keymint 2>/dev/null | awk '{print $1}')
    if [ -n "$new_pid" ] && [ "$new_pid" != "$old_pid" ] && [ -S "$TARGET_DIR/rpc.sock" ]; then
      return 0
    fi
    sleep 0.5
    tries=$((tries + 1))
  done
  echo "keymint did not become ready after config reload" >&2
  return 1
}

trust_fingerprint() {
  [ -f "$CONFIG_TOML" ] || { echo none; return 0; }
  awk '
    /^[[:space:]]*\[trust\][[:space:]]*(#.*)?$/ { in_trust=1; next }
    /^[[:space:]]*\[/ { in_trust=0 }
    in_trust && /^[[:space:]]*vb_(hash|key)[[:space:]]*=/ { print }
  ' "$CONFIG_TOML"
}

save_overlay() {
  write_props || return 1
  sync_toml || return 1
  apply_live "$1" || return 1
  request_keymint_restart
}

restore_boot() {
  [ -f "$WEBUI_PROPS" ] || return 0
  load_props || return 1
  # Absence/clear means keep bootloader defaults on boot, not delete stock VB.
  if [ -n "$VBMETA_HASH" ]; then apply_live hash || return 1; fi
  if [ -n "$VBMETA_KEY" ]; then apply_live key || return 1; fi
  sync_toml
}

sync_overlay() {
  local before after
  [ -f "$WEBUI_PROPS" ] || return 0
  load_props || return 1
  before=$(trust_fingerprint) || return 1
  sync_toml || return 1
  after=$(trust_fingerprint) || return 1
  if [ "$before" != "$after" ]; then request_keymint_restart; fi
}

cmd=$1
[ -n "$cmd" ] || exit 2
shift
case "$cmd" in
  hash|key)
    [ "$#" -le 1 ] || exit 2
    val=$1
    if [ "$val" = clear ] || [ -z "$val" ]; then
      val=
    else
      val=$(printf '%s' "$val" | tr 'A-F' 'a-f')
      hex64 "$val" || exit 3
    fi
    load_props || exit 1
    case "$cmd" in
      hash) VBMETA_HASH=$val ;;
      key) VBMETA_KEY=$val ;;
    esac
    save_overlay "$cmd"
    ;;
  patch)
    echo "A-side SPL changes are disabled; System/Boot/Vendor SPL was not changed" >&2
    exit 2
    ;;
  migrate)
    migrate_props "$@"
    ;;
  boot)
    [ "$#" -eq 0 ] || exit 2
    restore_boot
    ;;
  sync)
    [ "$#" -eq 0 ] || exit 2
    sync_overlay
    ;;
  *) exit 2 ;;
esac
