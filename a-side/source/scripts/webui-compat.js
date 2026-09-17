
// ── Ommega detector compatibility policy (target-compat.toml) ────────────────
// One global switch: every target app gets positive key ids, for clients that
// treat a non-positive key id as "unspecified". It is off by default, so the
// stock id distribution is kept until it is turned on from the mode dialog
// below. Applied by scripts/webui-patch.py to the prebuilt WebUI bundle.
const OMMEGA_TARGET_COMPAT = "/data/adb/ommega/ommegadata/target-compat.toml";
let ommegaCompatEnabled = false;

function ommegaParseTargetCompat(contents) {
  let enabled = false;
  let inSection = false;
  for (const rawLine of String(contents || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[")) {
      inSection = line === "[positive_key_id]";
      continue;
    }
    if (!inSection || line === "" || line.startsWith("#")) continue;
    if (/^enabled\s*=\s*true\b/.test(line)) enabled = true;
    // A legacy per-package list means the switch was on before it went global.
    if (/^packages\s*=\s*\[.*"[^"]+"/.test(line)) enabled = true;
  }
  return enabled;
}

function ommegaTargetCompatToml(enabled) {
  return [
    "# Ommega detector compatibility policy, written by the WebUI.",
    "# Global switch: every target app gets positive key ids, which keeps",
    "# clients that treat a non-positive key id as unspecified working.",
    "version = 1",
    "",
    "[positive_key_id]",
    "enabled = " + (enabled ? "true" : "false"),
    "",
  ].join("\n");
}

function ommegaCompatDialogPackage() {
  const element = document.getElementById("mode-dialog-appname");
  if (!element) return null;
  const parts = element.innerHTML
    .split(/<br\s*\/?>/i)
    .map(part => part.replace(/<[^>]*>/g, "").trim())
    .filter(Boolean);
  return parts.length > 1 ? parts[parts.length - 1] : null;
}

function ommegaCompatSyncOption() {
  const box = document.getElementById("mode-compat");
  if (!box) return;
  box.checked = ommegaCompatEnabled;
}

function ommegaApplyCompatToggle() {
  const box = document.getElementById("mode-compat");
  if (!box) return;
  ommegaCompatEnabled = box.checked;
}

function ommegaInstallCompatOption() {
  const container = document.querySelector("#mode-dialog .mode-options");
  if (!container || document.getElementById("mode-compat-row")) return;
  const row = document.createElement("label");
  row.className = "mode-option";
  row.id = "mode-compat-row";
  row.innerHTML = '<md-checkbox id="mode-compat" touch-target="wrapper"></md-checkbox>' +
    '<span data-i18n="mode_detector_compat">检测器兼容 / Detector compat</span>';
  container.appendChild(row);
  const hint = document.createElement("div");
  hint.id = "mode-compat-hint";
  hint.setAttribute("data-i18n", "mode_detector_compat_hint");
  hint.textContent = "对所有目标应用生效 / Applies to every target application";
  hint.style.cssText = "font-size:11px;line-height:1.4;opacity:.7;margin:-4px 0 8px 4px";
  container.parentElement.appendChild(hint);
  const box = row.querySelector("md-checkbox");
  box.addEventListener("click", ommegaApplyCompatToggle);
  row.querySelector("span").addEventListener("click", event => {
    event.preventDefault();
    box.checked = !box.checked;
    ommegaApplyCompatToggle();
  });
  // The mode dialog opens 500 ms after a long press on a card; mirror that delay
  // to show the state of the package that is about to be displayed.
  document.addEventListener("pointerdown", event => {
    if (!event.target || !event.target.closest || !event.target.closest(".card")) return;
    setTimeout(ommegaCompatSyncOption, 520);
  }, true);
}

async function ommegaWriteTargetCompat() {
  const body = ommegaTargetCompatToml(ommegaCompatEnabled);
  const command = [
    "set -e",
    "mkdir -p /data/adb/ommega/ommegadata",
    "cat > /data/adb/ommega/ommegadata/target-compat.toml.tmp << 'OMMEGA_COMPAT_EOF'",
    body + "OMMEGA_COMPAT_EOF",
    "chmod 0644 /data/adb/ommega/ommegadata/target-compat.toml.tmp",
    "chown 1017:1017 /data/adb/ommega/ommegadata/target-compat.toml.tmp 2>/dev/null || true",
    "mv /data/adb/ommega/ommegadata/target-compat.toml.tmp /data/adb/ommega/ommegadata/target-compat.toml",
  ].join("\n");
  return _(command);
}

const ommegaBaseAppLoadCompat = Mi;
Mi = async function () {
  const {stdout} = await _('cat "' + OMMEGA_TARGET_COMPAT + '" 2>/dev/null || true');
  ommegaCompatEnabled = ommegaParseTargetCompat(stdout);
  return ommegaBaseAppLoadCompat();
};

const ommegaBaseSaveCompat = document.getElementById("save").onclick;
document.getElementById("save").onclick = async event => {
  try {
    await ommegaWriteTargetCompat();
  } catch (error) {
    console.error("Ommega compat policy write failed:", error);
  }
  return ommegaBaseSaveCompat ? ommegaBaseSaveCompat.call(event.currentTarget, event) : undefined;
};

ommegaInstallCompatOption();
