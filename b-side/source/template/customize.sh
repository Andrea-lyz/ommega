# shellcheck disable=SC2034
# KernelSU/Magisk auto-unpack the whole module zip, then run this script. All
# module files are already present under $MODPATH (relay binary at
# $MODPATH/libs/<abi>/relay, etc.), so we only set permissions and perform
# light setup here — we never re-unzip.

if [ "$BOOTMODE" != "true" ]; then
  abort "! Please install via Magisk/KernelSU app"
fi

ui_print "- Installing ommegaclient-b B-side relay agent"

# Ensure the relay binary and the uninstall hook are executable.
chmod 0755 "$MODPATH/libs/arm64-v8a/relay" "$MODPATH/libs/x86_64/relay" \
  "$MODPATH/spl-control.sh" "$MODPATH/service.sh" "$MODPATH/uninstall.sh" 2>/dev/null || true

# Verify the expected binary exists for this ABI.
case "$ARCH" in
  arm64|arm64-v8a)
    [ -f "$MODPATH/libs/arm64-v8a/relay" ] || abort "! Missing libs/arm64-v8a/relay"
    ;;
  x64|x86_64)
    [ -f "$MODPATH/libs/x86_64/relay" ] || abort "! Missing libs/x86_64/relay"
    ;;
  *)
    abort "! Unsupported platform: $ARCH"
    ;;
esac

ui_print "- Done"
