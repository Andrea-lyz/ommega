
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

// ── Ommega relay verdict (remote-health.json) ───────────────────────────────
// The keymint daemon records the last relay verdict, but this prebuilt bundle
// never read it: a server-side 401 or 429 stayed invisible and users retried the
// "enable remote" checkbox instead. Show the recorded cause in the remote config
// dialog, directly below that checkbox.
const OMMEGA_REMOTE_HEALTH = "/data/misc/keystore/ommega/remote-health.json";

function ommegaHealthAdvice(kind) {
  if (kind === "unauthorized") {
    return "服务器拒绝了这个 Token（HTTP 401/403）。请检查 Token 与卡片有效期；" +
      "重新勾选 enable remote 不会改变结果。 / The server rejected this token " +
      "(HTTP 401/403). Check the token and card expiry; toggling enable remote " +
      "will not change the outcome.";
  }
  if (kind === "throttled") {
    return "服务器正在限流或暂时不可用（HTTP 429/503）。它会自行恢复，不需要操作。 / " +
      "The server is throttling or temporarily unavailable (HTTP 429/503). " +
      "It recovers on its own; no action needed.";
  }
  if (kind === "transport") {
    return "连不上服务器。请检查 URL 和网络。 / Could not reach the server. " +
      "Check the URL and the network.";
  }
  return "服务器返回了错误。详见下方状态码与消息。 / The server returned an error. " +
    "See the status code and message below.";
}

function ommegaHealthStamp(unix) {
  const seconds = Number(unix);
  if (!seconds || seconds <= 0) return "";
  try {
    return new Date(seconds * 1000).toLocaleString();
  } catch (error) {
    return String(seconds);
  }
}

// Renders the recorded verdict, or "" when the daemon has recorded nothing yet.
// A failure is named as the last one seen, together with when the relay last
// worked: the daemon rewrites the snapshot only when the state flips, so the
// file must not be presented as a live probe result.
function ommegaHealthReport(health) {
  if (!health || typeof health !== "object") return "";
  const okUnix = Number(health.last_ok_unix) || 0;
  const errUnix = Number(health.last_error_unix) || 0;
  if (okUnix <= 0 && errUnix <= 0) return "";
  if (errUnix <= 0 || okUnix > errUnix) {
    const at = ommegaHealthStamp(okUnix);
    const lines = [
      "中继状态：正常（最近一次成功 " + at + "） / Relay status: OK (last success " + at + ")",
    ];
    if (errUnix > 0) {
      // Recovered, but the earlier failure is still worth naming: it is the
      // difference between "never had a problem" and "had one and recovered".
      const status = Number(health.last_status) || 0;
      const suffix = status > 0 ? "（HTTP " + status + "）" : "";
      lines.push("此前失败于 / Earlier failure at " + ommegaHealthStamp(errUnix) + suffix);
    }
    return lines.join("\n");
  }
  const lines = [
    "最近一次中继失败 / Last relay failure: " + ommegaHealthStamp(errUnix),
    ommegaHealthAdvice(health.last_kind),
  ];
  const status = Number(health.last_status) || 0;
  if (status > 0) lines.push("HTTP " + status);
  if (health.last_message) {
    lines.push("服务器消息 / Server message: " + String(health.last_message));
  }
  if (okUnix > 0) {
    lines.push("此前成功于 / Last success at " + ommegaHealthStamp(okUnix));
  }
  return lines.join("\n");
}

function ommegaHealthPanel() {
  let panel = document.getElementById("remote-health");
  if (panel) return panel;
  const content = document.querySelector(".remote-config-content");
  if (!content) return null;
  panel = document.createElement("div");
  panel.id = "remote-health";
  panel.style.cssText = "margin-top:4px;font-size:12px;line-height:1.5;" +
    "white-space:pre-wrap;word-break:break-word;opacity:.85";
  content.appendChild(panel);
  return panel;
}

async function ommegaRefreshHealth() {
  const panel = ommegaHealthPanel();
  if (!panel) return;
  let report = "";
  try {
    const {errno, stdout} = await _(`cat "${OMMEGA_REMOTE_HEALTH}" 2>/dev/null || true`);
    const raw = String(stdout === undefined || stdout === null ? "" : stdout).trim();
    if (errno === 0 && raw !== "") report = ommegaHealthReport(JSON.parse(raw));
  } catch (error) {
    // A half-written or malformed snapshot must not read as "nothing to report".
    report = "无法读取中继状态（remote-health.json 解析失败） / " +
      "Could not read the relay status (remote-health.json is unreadable).";
  }
  panel.textContent = report === ""
    ? "中继状态：暂无失败记录 / Relay status: no failures recorded yet."
    : report;
}

// Refresh whenever the remote config dialog opens. typeof keeps the glue safe
// in a context without the bundle's globals (the CI harness evaluates it that way).
if (typeof loadRemoteConfig === "function") {
  const ommegaBaseLoadRemoteConfigHealth = loadRemoteConfig;
  loadRemoteConfig = async function () {
    await ommegaBaseLoadRemoteConfigHealth();
    await ommegaRefreshHealth();
  };
}
