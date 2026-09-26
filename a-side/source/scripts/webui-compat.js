
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

// ── Ommega software Soter TA switch (local Soter checks) ─────────────────────
// The vendor AIDL HAL vendor.qti.hardware.soter.ISoter/default is a thin proxy
// over a Qualcomm TA in the secure world. When that applet cannot run, every
// Soter call fails and applications that use Soter as a local integrity probe
// read the device as tampered with. The module's soterta.sh watchdog answers
// those calls from a software TA while the stock HAL is stopped.
//
// This panel owns the switch and nothing else: the enable flag and the published
// status are plain files, so the UI needs no module path, no shell quoting and no
// privileged command of its own. Local checks only; the server-side check path
// stays impossible, and turning the switch off rolls the stock HAL back.
const OMMEGA_SOTERTA_DIR = "/data/adb/ommega/soterta";
const OMMEGA_SOTERTA_FLAG = OMMEGA_SOTERTA_DIR + "/enabled";
const OMMEGA_SOTERTA_STATUS = OMMEGA_SOTERTA_DIR + "/status.json";
// The watchdog rewrites status.json on every change and otherwise every 30 s, so
// an old stamp means the watchdog is gone, not that nothing happened.
const OMMEGA_SOTERTA_STALE = 120;

function ommegaSotertaParseJson(text) {
  const raw = String(text === undefined || text === null ? "" : text).trim();
  if (raw === "") return null;
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" ? value : null;
  } catch (error) {
    return null;
  }
}

function ommegaSotertaStamp(unix) {
  const seconds = Number(unix) || 0;
  if (seconds <= 0) return "";
  try {
    return new Date(seconds * 1000).toLocaleString();
  } catch (error) {
    return String(seconds);
  }
}

function ommegaSotertaStale(status) {
  const updated = Number(status && status.updated) || 0;
  if (updated <= 0) return true;
  return Math.floor(Date.now() / 1000) - updated > OMMEGA_SOTERTA_STALE;
}

// The only command this panel has to run. Writing the flag is enough: the
// watchdog notices it within a couple of seconds and does the takeover itself.
function ommegaSotertaSwitchCommand(on) {
  if (!on) return 'rm -f "' + OMMEGA_SOTERTA_FLAG + '"';
  return [
    "set -e",
    'mkdir -p "' + OMMEGA_SOTERTA_DIR + '"',
    'date +%s > "' + OMMEGA_SOTERTA_FLAG + '.tmp"',
    'chmod 0600 "' + OMMEGA_SOTERTA_FLAG + '.tmp"',
    'mv "' + OMMEGA_SOTERTA_FLAG + '.tmp" "' + OMMEGA_SOTERTA_FLAG + '"',
  ].join("\n");
}

// What the panel can honestly say. `known` is false only when the flag could not
// be read at all, which is not the same as "off".
function ommegaSotertaReport(known, enabled, status) {
  if (!known) {
    return "无法读取开关状态（soterta 目录不可访问）/ " +
      "Could not read the switch state (the soterta directory is not accessible).";
  }
  const lines = [];
  lines.push(enabled
    ? "开关：已启用 / Switch: on"
    : "开关：已停用 / Switch: off");

  if (!enabled) {
    lines.push("软件 TA：未运行 / Software Soter TA: not running");
    if (status && status.owner === "us") {
      lines.push("原厂 HAL：仍被接管，看门狗还没回滚 / " +
        "Stock HAL: still taken over; the watchdog has not rolled back yet");
    } else {
      lines.push("原厂 HAL：在跑，Soter 维持现状（本地检查会失败）/ " +
        "Stock HAL: serving; Soter stays as it is (local checks fail)");
    }
    return lines.join("\n");
  }

  if (!status) {
    lines.push("软件 TA：已请求启用，等待看门狗上报 / " +
      "Software Soter TA: requested; waiting for the watchdog to report");
    lines.push("看门狗：还没有状态文件（模块装好后是否重启过？）/ " +
      "Watchdog: no status file yet (has the module been restarted since install?)");
    return lines.join("\n");
  }

  const mode = status.mode || "-";
  const pid = status.pid || "-";
  lines.push(status.running
    ? "软件 TA：运行中（mode=" + mode + "，pid=" + pid + "）/ Software Soter TA: running (mode=" +
      mode + ", pid=" + pid + ")"
    : "软件 TA：未运行 / Software Soter TA: not running");
  if (status.owner === "us") {
    lines.push("原厂 HAL：已停止（服务名已接管）/ Stock HAL: stopped (service name taken over)");
  } else if (status.hal === "running") {
    lines.push("原厂 HAL：在跑（接管尚未完成）/ Stock HAL: serving (takeover not finished)");
  } else {
    lines.push("原厂 HAL：" + (status.hal || "unknown") + " / Stock HAL: " + (status.hal || "unknown"));
  }
  if (status.device_id && status.device_id !== "-") {
    lines.push("设备 ID：" + status.device_id + "（账本：" +
      (status.ledger ? "已就绪" : "缺失，下次启动会重建") + "）/ Device id: " +
      status.device_id + " (ledger: " +
      (status.ledger ? "ready" : "missing; it is regenerated on the next start") + ")");
  }
  if (Number(status.id_changed) === 1) {
    lines.push("⚠ 设备 ID 变过：" + (status.id_note || "?") +
      " / the device id changed (" + (status.id_note || "?") +
      "); every client that remembers this device will see a new one");
  }
  const stamp = ommegaSotertaStamp(status.updated);
  if (stamp !== "") {
    lines.push(ommegaSotertaStale(status)
      ? "看门狗：自 " + stamp + " 起没有更新，可能已经停了 / Watchdog: nothing since " + stamp + "; it may be down"
      : "看门狗：" + stamp + " 更新 / Watchdog: updated " + stamp);
  }
  const failures = Number(status.failures) || 0;
  if (failures > 0) {
    lines.push("启动失败：" + failures + " 次（看门狗会继续重试）/ Start failures: " + failures +
      " (the watchdog keeps retrying)");
  }
  if (status.note) lines.push(String(status.note));
  return lines.join("\n");
}

// One round trip: the flag marker first, then the published status.
async function ommegaSotertaRead() {
  const {stdout} = await _(
    'if [ -f "' + OMMEGA_SOTERTA_FLAG + '" ]; then echo OMMEGA_FLAG=1; else echo OMMEGA_FLAG=0; fi\n' +
    'cat "' + OMMEGA_SOTERTA_STATUS + '" 2>/dev/null'
  );
  const text = String(stdout === undefined || stdout === null ? "" : stdout);
  const lines = text.split(/\r?\n/);
  const marker = (lines.shift() || "").trim();
  const match = marker.match(/^OMMEGA_FLAG=([01])$/);
  return {
    known: !!match,
    enabled: match ? match[1] === "1" : false,
    status: ommegaSotertaParseJson(lines.join("\n")),
  };
}

async function ommegaSotertaRefresh(pending) {
  const report = document.getElementById("ommega-soterta-report");
  const box = document.getElementById("ommega-soterta-enabled");
  if (!report) return;
  let state;
  try {
    state = await ommegaSotertaRead();
  } catch (error) {
    report.textContent = "读取 Soter 状态失败 / Could not read the Soter state: " + error;
    return;
  }
  if (box) box.checked = state.enabled;
  const lines = [ommegaSotertaReport(state.known, state.enabled, state.status)];
  if (pending) {
    lines.push("开关已写入，看门狗会在几秒内接管（要先停掉原厂 HAL）/ " +
      "Switch written; the watchdog applies it within a few seconds (it stops the stock HAL first).");
  }
  report.textContent = lines.join("\n\n");
}

// The flag is the request; the watchdog reacts within its poll interval and the
// takeover itself takes a few seconds (it stops a live HAL first), so the panel
// samples twice instead of pretending the switch is instant.
const OMMEGA_SOTERTA_REFRESH_MS = [3000, 9000];

async function ommegaSotertaApply() {
  const box = document.getElementById("ommega-soterta-enabled");
  const report = document.getElementById("ommega-soterta-report");
  const on = !!(box && box.checked);
  if (report) report.textContent = "正在写入开关… / Writing the switch…";
  let errno = 1;
  let stderr = "";
  try {
    const result = await _(ommegaSotertaSwitchCommand(on));
    errno = Number(result && result.errno);
    stderr = String((result && result.stderr) || "");
  } catch (error) {
    stderr = String(error);
  }
  if (errno !== 0) {
    if (report) {
      report.textContent = "写入开关失败（errno=" + errno + "）/ Failed to write the switch (errno=" + errno + ")\n" +
        stderr.trim();
    }
    return;
  }
  // The flag is only the request; the watchdog publishes the result a moment later.
  await new Promise(resolve => setTimeout(resolve, OMMEGA_SOTERTA_REFRESH_MS[0]));
  await ommegaSotertaRefresh(true);
  const second = OMMEGA_SOTERTA_REFRESH_MS[1] - OMMEGA_SOTERTA_REFRESH_MS[0];
  await new Promise(resolve => setTimeout(resolve, second));
  await ommegaSotertaRefresh(false);
}

function ommegaSotertaDialog() {
  const existing = document.getElementById("ommega-soterta-dialog");
  if (existing) return existing;
  const wrapper = document.querySelector(".dialog-wrapper");
  if (!wrapper || typeof document.createElement !== "function") return null;
  const dialog = document.createElement("md-dialog");
  dialog.id = "ommega-soterta-dialog";
  dialog.className = "text-field-dialog";
  dialog.innerHTML =
    '<div slot="headline">Soter 本地检查 / Soter local check</div>' +
    '<div slot="content" style="display:flex;flex-direction:column;gap:12px">' +
      '<div id="ommega-soterta-report" style="font-size:12px;line-height:1.5;white-space:pre-wrap;word-break:break-word;opacity:.85"></div>' +
      '<label class="config-option" style="display:flex;align-items:center;gap:8px">' +
        '<md-checkbox id="ommega-soterta-enabled" touch-target="wrapper"></md-checkbox>' +
        '<span>启用软件 TA / Enable software TA</span>' +
      '</label>' +
      '<div style="font-size:11px;line-height:1.4;opacity:.7">' +
        '只覆盖本地检查；服务器校验路径做不到。开关打开后原厂 Soter HAL 会被停止、由软件 TA 接管服务名，关闭即回滚。 / ' +
        'Local checks only; the server-side check path stays impossible. While the switch is on, the stock Soter HAL is ' +
        'stopped and the software TA takes its service name over; turning it off rolls back.' +
      '</div>' +
    '</div>' +
    '<div slot="actions">' +
      '<md-text-button id="ommega-soterta-refresh">刷新 / Refresh</md-text-button>' +
      '<md-text-button id="ommega-soterta-close">关闭 / Close</md-text-button>' +
    '</div>';
  wrapper.appendChild(dialog);
  const box = dialog.querySelector("#ommega-soterta-enabled");
  if (box) box.addEventListener("click", () => { ommegaSotertaApply(); });
  const refresh = dialog.querySelector("#ommega-soterta-refresh");
  if (refresh) refresh.addEventListener("click", () => { ommegaSotertaRefresh(false); });
  const close = dialog.querySelector("#ommega-soterta-close");
  if (close) close.addEventListener("click", () => { dialog.close(); });
  return dialog;
}

function ommegaSotertaOpen() {
  const dialog = ommegaSotertaDialog();
  if (!dialog) return Promise.resolve();
  if (typeof dialog.show === "function") dialog.show();
  return ommegaSotertaRefresh(false);
}

function ommegaSotertaInstallMenu() {
  if (document.getElementById("ommega-soterta")) return;
  const anchor = document.getElementById("remote-config");
  if (!anchor || typeof anchor.insertAdjacentElement !== "function") return;
  const item = document.createElement("md-menu-item");
  item.id = "ommega-soterta";
  item.className = "automation-menu-item";
  item.innerHTML = '<div slot="headline">Soter 本地检查 / Soter local check</div>' +
    '<md-icon slot="end">fingerprint</md-icon>';
  item.addEventListener("click", () => {
    // md-menu closes itself when an item is activated; close() is only the nudge
    // for elements that need one.
    const menu = document.getElementById("menu-options");
    if (menu && typeof menu.close === "function") menu.close();
    ommegaSotertaOpen();
  });
  anchor.insertAdjacentElement("afterend", item);
}

ommegaSotertaInstallMenu();
