#!/system/bin/sh
# Keep WebUI overlay props and keymint config.toml [trust] in sync.
# Used by the WebUI (save hash/key/patch) and by post-fs-data.sh (boot).

TARGET_DIR=/data/misc/keystore/ommega
WEBUI_PROPS=$TARGET_DIR/webui-props.sh
CONFIG_TOML=$TARGET_DIR/config.toml
RESTART_FLAG=/data/adb/ommega/restart.keymint

SYS_PATCH=
BOOT_PATCH=
VENDOR_PATCH=
VBMETA_HASH=
VBMETA_KEY=

hex64() {
  echo "$1" | grep -Eq '^[0-9a-f]{64}$'
}

date8() {
  echo "$1" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'
}

load_props() {
  SYS_PATCH=
  BOOT_PATCH=
  VENDOR_PATCH=
  VBMETA_HASH=
  VBMETA_KEY=
  [ -f "$WEBUI_PROPS" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      "resetprop ro.build.version.security_patch "*)
        SYS_PATCH=${line#resetprop ro.build.version.security_patch }
        ;;
      "resetprop ro.boot.image.build.security_patch "*)
        BOOT_PATCH=${line#resetprop ro.boot.image.build.security_patch }
        ;;
      "resetprop ro.vendor.build.security_patch "*)
        VENDOR_PATCH=${line#resetprop ro.vendor.build.security_patch }
        ;;
      "resetprop ro.boot.vbmeta.digest "*)
        VBMETA_HASH=${line#resetprop ro.boot.vbmeta.digest }
        ;;
      "resetprop ro.boot.vbmeta.public_key_digest "*)
        VBMETA_KEY=${line#resetprop ro.boot.vbmeta.public_key_digest }
        ;;
    esac
  done < "$WEBUI_PROPS"
}

write_props() {
  mkdir -p "$TARGET_DIR" /data/adb/ommega
  {
    echo "# ommega WebUI overlay props (survives module overlay installs)"
    [ -n "$SYS_PATCH" ] && echo "resetprop ro.build.version.security_patch $SYS_PATCH"
    [ -n "$BOOT_PATCH" ] && echo "resetprop ro.boot.image.build.security_patch $BOOT_PATCH"
    [ -n "$VENDOR_PATCH" ] && echo "resetprop ro.vendor.build.security_patch $VENDOR_PATCH"
    [ -n "$VBMETA_HASH" ] && echo "resetprop ro.boot.vbmeta.digest $VBMETA_HASH"
    [ -n "$VBMETA_KEY" ] && echo "resetprop ro.boot.vbmeta.public_key_digest $VBMETA_KEY"
  } > "$WEBUI_PROPS"
  chmod 0644 "$WEBUI_PROPS" 2>/dev/null || true
  chown 1017:1017 "$WEBUI_PROPS" 2>/dev/null || true
}

toml_quote() {
  echo "\"$1\""
}

toml_set_trust() {
  key=$1
  val=$2
  [ -f "$CONFIG_TOML" ] || return 0
  tmp=$CONFIG_TOML.tmp.$$
  awk -v key="$key" -v val="$val" '
    BEGIN { in_trust=0; done=0 }
    /^\[trust\][[:space:]]*$/ {
      print
      in_trust=1
      next
    }
    /^\[/ {
      if (in_trust && !done) {
        print key " = " val
        done=1
      }
      in_trust=0
      print
      next
    }
    in_trust && $0 ~ ("^" key "[[:space:]]*=") {
      if (!done) {
        print key " = " val
        done=1
      }
      next
    }
    { print }
    END {
      if (in_trust && !done) print key " = " val
    }
  ' "$CONFIG_TOML" > "$tmp" || {
    rm -f "$tmp"
    return 1
  }
  mv "$tmp" "$CONFIG_TOML"
  chmod 0644 "$CONFIG_TOML" 2>/dev/null || true
  chown 1017:1017 "$CONFIG_TOML" 2>/dev/null || true
}

sync_toml() {
  [ -f "$CONFIG_TOML" ] || return 0
  if [ -n "$SYS_PATCH" ]; then
    toml_set_trust security_patch "$(toml_quote "$SYS_PATCH")"
    toml_set_trust os_patchlevel "$(toml_quote "$SYS_PATCH")"
  else
    toml_set_trust security_patch "$(toml_quote auto)"
    toml_set_trust os_patchlevel "$(toml_quote auto)"
  fi
  if [ -n "$VENDOR_PATCH" ]; then
    toml_set_trust vendor_patchlevel "$(toml_quote "$VENDOR_PATCH")"
  else
    toml_set_trust vendor_patchlevel "$(toml_quote auto)"
  fi
  if [ -n "$BOOT_PATCH" ]; then
    toml_set_trust boot_patchlevel "$(toml_quote "$BOOT_PATCH")"
  else
    toml_set_trust boot_patchlevel "$(toml_quote auto)"
  fi
  if [ -n "$VBMETA_KEY" ]; then
    toml_set_trust vb_key "$(toml_quote "$VBMETA_KEY")"
  else
    toml_set_trust vb_key "$(toml_quote auto)"
  fi
  if [ -n "$VBMETA_HASH" ]; then
    toml_set_trust vb_hash "$(toml_quote "$VBMETA_HASH")"
  else
    toml_set_trust vb_hash "$(toml_quote auto)"
  fi
}

apply_live() {
  [ -n "$SYS_PATCH" ] && resetprop -n ro.build.version.security_patch "$SYS_PATCH" 2>/dev/null
  [ -n "$BOOT_PATCH" ] && resetprop -n ro.boot.image.build.security_patch "$BOOT_PATCH" 2>/dev/null
  [ -n "$VENDOR_PATCH" ] && resetprop -n ro.vendor.build.security_patch "$VENDOR_PATCH" 2>/dev/null
  [ -n "$VBMETA_HASH" ] && resetprop -n ro.boot.vbmeta.digest "$VBMETA_HASH" 2>/dev/null
  [ -n "$VBMETA_KEY" ] && resetprop -n ro.boot.vbmeta.public_key_digest "$VBMETA_KEY" 2>/dev/null
}

request_keymint_restart() {
  mkdir -p /data/adb/ommega
  touch "$RESTART_FLAG"
}

trust_fingerprint() {
  [ -f "$CONFIG_TOML" ] || { echo none; return 0; }
  grep -E '^(security_patch|os_patchlevel|vendor_patchlevel|boot_patchlevel|vb_key|vb_hash) *=' "$CONFIG_TOML" 2>/dev/null
}

save_overlay() {
  live=$1
  restart=$2
  write_props
  before=$(trust_fingerprint)
  sync_toml
  after=$(trust_fingerprint)
  [ "$live" = 1 ] && apply_live
  if [ "$restart" = 1 ]; then
    request_keymint_restart
  elif [ "$before" != "$after" ]; then
    request_keymint_restart
  fi
}

cmd=$1
[ -n "$cmd" ] || exit 2
shift

case "$cmd" in
  hash)
    val=$1
    load_props
    if [ "$val" = "clear" ] || [ -z "$val" ]; then
      VBMETA_HASH=
    else
      val=$(echo "$val" | tr 'A-F' 'a-f')
      hex64 "$val" || exit 3
      VBMETA_HASH=$val
    fi
    save_overlay 1 1
    ;;
  key)
    val=$1
    load_props
    if [ "$val" = "clear" ] || [ -z "$val" ]; then
      VBMETA_KEY=
    else
      val=$(echo "$val" | tr 'A-F' 'a-f')
      hex64 "$val" || exit 3
      VBMETA_KEY=$val
    fi
    save_overlay 1 1
    ;;
  patch)
    sub=$1
    shift
    load_props
    case "$sub" in
      auto)
        SYS_PATCH=
        BOOT_PATCH=
        VENDOR_PATCH=
        ;;
      all)
        date=$1
        date8 "$date" || exit 3
        SYS_PATCH=$date
        BOOT_PATCH=$date
        VENDOR_PATCH=$date
        ;;
      set)
        SYS_PATCH=
        BOOT_PATCH=
        VENDOR_PATCH=
        for spec in "$@"; do
          key=${spec%%=*}
          val=${spec#*=}
          [ "$val" = "prop" ] && val=
          [ -n "$val" ] && ! date8 "$val" && exit 3
          case "$key" in
            system) SYS_PATCH=$val ;;
            boot) BOOT_PATCH=$val ;;
            vendor) VENDOR_PATCH=$val ;;
          esac
        done
        ;;
      *)
        exit 2
        ;;
    esac
    save_overlay 1 1
    ;;
  sync)
    load_props
    before=$(trust_fingerprint)
    sync_toml
    after=$(trust_fingerprint)
    [ "$before" != "$after" ] && request_keymint_restart
    ;;
  *)
    exit 2
    ;;
esac

exit 0
