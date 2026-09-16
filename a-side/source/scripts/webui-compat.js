
// ── Ommega detector compatibility policy (target-compat.toml) ────────────────
// Packages listed there get positive key ids, for clients that treat a
// non-positive key id as "unspecified". The list is empty by default, so the
// stock id distribution is kept until a package is opted in from the mode
// dialog below. Applied by scripts/webui-patch.py to the prebuilt WebUI bundle.
const OMMEGA_TARGET_COMPAT = "/data/adb/ommega/ommegadata/target-compat.toml";
let ommegaCompatPackages = new Set();

function ommegaParseTargetCompat(contents) {
  const packages = new Set();
  let inSection = false;
  let buffer = null;
  const collect = text => {
    for (const item of text.matchAll(/"([^"]*)"/g)) {
      const pkg = item[1].trim();
      if (pkg) packages.add(pkg);
    }
  };
  for (const rawLine of String(contents || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (buffer !== null) {
      buffer += " " + line;
      if (line.includes("]")) {
        collect(buffer);
        buffer = null;
      }
      continue;
    }
    if (line.startsWith("[")) {
      inSection = line === "[positive_key_id]";
      continue;
    }
    if (!inSection || line === "" || line.startsWith("#")) continue;
    const assignment = line.match(/^packages\s*=\s*\[(.*)$/);
    if (!assignment) continue;
    if (assignment[1].includes("]")) collect(assignment[1]);
    else buffer = assignment[1];
  }
  return packages;
}

function ommegaTargetCompatToml(packages) {
  const lines = [
    "# Ommega detector compatibility policy, written by the WebUI.",
    "# Packages under [positive_key_id] get positive key ids, which keeps",
    "# clients that treat a non-positive key id as unspecified working.",
    "version = 1",
    "",
    "[positive_key_id]",
  ];
  const list = [...packages].sort().map(pkg => '"' + pkg + '"').join(", ");
  lines.push("packages = [" + list + "]");
  return lines.join("\n") + "\n";
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
  const pkg = ommegaCompatDialogPackage();
  if (!box || !pkg) return;
  box.checked = ommegaCompatPackages.has(pkg);
}

function ommegaApplyCompatToggle() {
  const box = document.getElementById("mode-compat");
  const pkg = ommegaCompatDialogPackage();
  if (!box || !pkg) return;
  if (box.checked) ommegaCompatPackages.add(pkg);
  else ommegaCompatPackages.delete(pkg);
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
  hint.textContent = "为该包分配正数 key id / Allocate positive key ids for this package";
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
  const selected = new Set(
    Array.from(se.querySelectorAll("md-checkbox"))
      .filter(box => box.checked)
      .map(box => box.closest(".card").getAttribute("data-package"))
      .filter(pkg => /^[A-Za-z0-9_.]+$/.test(pkg))
  );
  Dl.forEach(pkg => selected.add(pkg));
  for (const pkg of [...ommegaCompatPackages]) {
    if (!selected.has(pkg)) ommegaCompatPackages.delete(pkg);
  }
  const body = ommegaTargetCompatToml(ommegaCompatPackages);
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
  ommegaCompatPackages = ommegaParseTargetCompat(stdout);
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
