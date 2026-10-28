
// ── Ommega detector compatibility policy (target-compat.toml) ────────────────
// One global switch: every target app gets positive key ids, for clients that
// treat a non-positive key id as "unspecified". It is off by default, so the
// stock id distribution is kept until it is turned on in Settings.
// Applied by scripts/webui-patch.py to the prebuilt WebUI bundle.
const OMMEGA_TARGET_COMPAT = "/data/adb/ommega/ommegadata/target-compat.toml";
let ommegaCompatEnabled = false, ommegaCompatSaving = false, ommegaCompatRevision = 0;
let ommegaAppsBaseline = null;

// The modern shell supplies the shared bounded read bridge. Mutations keep the
// original bridge: a UI timeout must never interrupt or retry a device write.
function ommegaReadOnly(command) {
  const detail = {command, promise:null};
  if (typeof CustomEvent === "function") document.dispatchEvent(new CustomEvent("ommega-readonly", {detail}));
  if (detail.promise) return detail.promise;
  // Startup may precede the modern module. Bound only the wait for this read;
  // a late original callback settles its own promise and cannot repaint state.
  let timer;
  return Promise.race([_(command), new Promise(resolve => {
    timer = setTimeout(() => resolve({errno:1,stdout:"",stderr:ommegaUiLocalize("读取超时，请刷新重试。 / Read timed out. Refresh to retry."),timedOut:true}), 12000);
  })]).finally(() => clearTimeout(timer));
}

function ommegaAppSettingsSnapshot(packages) {
  const selected = [...new Set(packages)].sort();
  return JSON.stringify(selected.map(pkg => [pkg, ommegaTargetModes.get(pkg) || "global_default"]));
}

function ommegaAppsHaveChanges() {
  if (ommegaAppsBaseline === null) return false;
  // Keep loaded selections until their cards exist. Loading, filtering and
  // sorting the list must not look like an edit to the scope.
  const selected = new Set(Qe);
  for (const box of se.querySelectorAll("md-checkbox")) {
    const pkg = box.closest(".card")?.getAttribute("data-package");
    if (!pkg) continue;
    if (box.checked) selected.add(pkg);
    else selected.delete(pkg);
  }
  Dl.forEach(pkg => selected.add(pkg));
  return ommegaAppSettingsSnapshot(selected) !== ommegaAppsBaseline;
}

// modern.js is a separate module, so expose the result through a synchronous
// event rather than the visibility of the legacy Save button.
document.addEventListener("ommega-app-changes", event => {
  event.detail.dirty = ommegaAppsHaveChanges();
});

// Existing diagnostics keep their bilingual representation for tooling; the UI
// presents the matching half using the same resolved preference as the shell.
function ommegaUiLocalize(value) {
  let chinese = false;
  try {
    const preference = localStorage.getItem("trickyAddonLanguage");
    chinese = /^zh/i.test(preference && preference !== "default" ? preference : navigator.language);
  } catch (_) {}
  return String(value).split("\n").map(line => {
    const halves = line.split(/\s*\/ (?=[A-Za-z])/);
    return halves.length === 2 && /[\u3400-\u9fff]/.test(halves[0]) ? halves[chinese ? 0 : 1] : line;
  }).join("\n");
}
function ommegaUiLocalizeHtml(html) {
  return html.replace(/>([^<>]+)</g, (_, value) => ">" + ommegaUiLocalize(value) + "<");
}

function ommegaParseTargetCompat(contents) {
  let enabled = false;
  let inSection = false;
  for (const rawLine of String(contents || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (line.startsWith("[")) {
      inSection = /^\[\s*(?:positive_key_id|"positive_key_id"|'positive_key_id')\s*\]\s*(?:#.*)?$/.test(line);
      continue;
    }
    if (!inSection || line === "" || line.startsWith("#")) continue;
    if (/^(?:enabled|"enabled"|'enabled')\s*=\s*true\b/.test(line)) enabled = true;
    // A legacy per-package list means the switch was on before it went global.
    if (/^(?:packages|"packages"|'packages')\s*=\s*\[.*["'][^"']+["']/.test(line)) enabled = true;
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

function ommegaTargetCompatUpdate(original, enabled) {
  if (original === null) return ommegaTargetCompatToml(enabled);
  const lines = String(original).split("\n");
  let section = -1, inSection = false, enabledLine = -1, packagesLine = -1;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line || line.startsWith("#")) continue;
    if (line.startsWith("[")) {
      inSection = /^\[\s*(?:positive_key_id|"positive_key_id"|'positive_key_id')\s*\]\s*(?:#.*)?$/.test(line);
      if (inSection) {
        if (section !== -1) throw new Error("Duplicate positive_key_id section");
        section = i;
      }
      continue;
    }
    if (!inSection) continue;
    if (/^(?:enabled|"enabled"|'enabled')\s*=/.test(line)) {
      if (enabledLine !== -1 || !/^(?:enabled|"enabled"|'enabled')\s*=\s*(?:true|false)\s*(?:#.*)?$/.test(line)) throw new Error("Unsupported positive_key_id enabled value");
      enabledLine = i;
    } else if (/^(?:packages|"packages"|'packages')\s*=/.test(line)) {
      if (packagesLine !== -1 || !/^(?:packages|"packages"|'packages')\s*=\s*\[[^\]]*\]\s*(?:#.*)?$/.test(line)) throw new Error("Unsupported legacy package list");
      packagesLine = i;
    }
  }
  const newline = String(original).includes("\r\n") ? "\r" : "";
  if (packagesLine !== -1 && !enabled) lines[packagesLine] = lines[packagesLine].replace(/^(\s*(?:packages|"packages"|'packages')\s*=\s*)\[[^\]]*\]/, "$1[]");
  if (enabledLine !== -1) lines[enabledLine] = lines[enabledLine].replace(/^(\s*(?:enabled|"enabled"|'enabled')\s*=\s*)(true|false)/, "$1" + enabled);
  else if (section !== -1) lines.splice(section + 1, 0, "enabled = " + enabled + newline);
  else return String(original) + (String(original).endsWith("\n") ? "" : "\n") + "\n[positive_key_id]\nenabled = " + enabled + "\n";
  return lines.join("\n");
}

function ommegaCompatWriteCommand(original, enabled) {
  const before = original === null ? "M:" : "F:" + ommegaSoterRemoteBase64(original);
  const after = ommegaSoterRemoteBase64(ommegaTargetCompatUpdate(original, enabled));
  return ["set -e", 'ommega_compat_file="' + OMMEGA_TARGET_COMPAT + '"',
    'ommega_compat_tmp="$ommega_compat_file.$$"', 'trap \'rm -f "$ommega_compat_tmp"\' EXIT',
    'if [ ! -e "$ommega_compat_file" ]; then ommega_compat_current="M:"; elif [ -f "$ommega_compat_file" ]; then ommega_compat_current="F:$(base64 < "$ommega_compat_file" | tr -d \'\\r\\n\')"; else exit 1; fi',
    `[ "$ommega_compat_current" = '${before}' ] || { echo "Config changed; retry" >&2; exit 75; }`,
    "printf '%s' '" + after + "' | base64 -d > \"$ommega_compat_tmp\"",
    'chmod 0644 "$ommega_compat_tmp"', 'chown 1017:1017 "$ommega_compat_tmp"',
    'mv -f "$ommega_compat_tmp" "$ommega_compat_file"', 'trap - EXIT'].join("\n");
}

const ommegaBaseAppLoadCompat = Mi;
Mi = async function () {
  const {stdout} = await _('cat "' + OMMEGA_TARGET_COMPAT + '" 2>/dev/null || true');
  ommegaCompatEnabled = ommegaParseTargetCompat(stdout);
  const setting = document.getElementById("ommega-global-compat");
  if (setting && !setting.disabled) setting.checked = ommegaCompatEnabled;
  const result = await ommegaBaseAppLoadCompat();
  // A successful scope save reloads Mi; a failed save leaves this baseline.
  ommegaAppsBaseline = ommegaAppSettingsSnapshot(Qe);
  return result;
};

async function ommegaReadTargetCompat() {
  const result = await ommegaReadOnly('if [ ! -e "' + OMMEGA_TARGET_COMPAT + '" ]; then printf "M:"; elif [ -f "' + OMMEGA_TARGET_COMPAT + '" ]; then printf "F:"; base64 < "' + OMMEGA_TARGET_COMPAT + '"; else exit 1; fi');
  if (Number(result.errno) !== 0) throw new Error(result.stderr || "Could not read compatibility policy");
  const encoded = String(result.stdout).replace(/\s/g, "");
  if (encoded === "M:") return {original:null, enabled:false};
  if (!encoded.startsWith("F:")) throw new Error("Invalid compatibility policy response");
  const bytes = Uint8Array.from(atob(encoded.slice(2)), c => c.charCodeAt(0));
  const original = new TextDecoder("utf-8", {fatal:true, ignoreBOM:true}).decode(bytes);
  ommegaTargetCompatUpdate(original, true); // Reject ambiguous policy syntax before displaying an editable state.
  return {original, enabled:ommegaParseTargetCompat(original)};
}

async function ommegaGlobalCompatChange() {
  const box = document.getElementById("ommega-global-compat");
  const status = document.getElementById("ommega-global-compat-status");
  if (!box || !status || box.disabled) return;
  const requested = box.checked;
  ommegaCompatSaving = true; ++ommegaCompatRevision;
  box.disabled = true;
  status.textContent = ommegaUiLocalize("正在保存… / Saving…");
  try {
    const before = await ommegaReadTargetCompat();
    const written = await _(ommegaCompatWriteCommand(before.original, requested));
    if (Number(written.errno) !== 0) throw new Error(written.stderr || "Write failed");
    const {enabled:actual} = await ommegaReadTargetCompat();
    if (actual !== requested) throw new Error("Saved policy did not match on readback");
    ommegaCompatEnabled = actual;
    status.textContent = ommegaUiLocalize("已保存，对所有调用者生效。 / Saved; applies to all callers.");
  } catch (error) {
    try { ({enabled:ommegaCompatEnabled} = await ommegaReadTargetCompat()); }
    catch (_) { ommegaCompatSaving = false; box.checked = ommegaCompatEnabled; box.disabled = true; status.textContent = ommegaUiLocalize("读取失败，请刷新后重试。 / Read failed; refresh and try again."); return; }
    status.textContent = ommegaUiLocalize("保存失败 / Save failed") + ": " + error.message;
  }
  box.checked = ommegaCompatEnabled;
  box.disabled = false;
  ommegaCompatSaving = false;
}

async function ommegaInstallGlobalCompat() {
  const slot = document.getElementById("om-compat-settings-slot");
  if (!slot || typeof slot.appendChild !== "function" || document.getElementById("ommega-global-compat")) return;
  const panel = document.createElement("section");
  panel.className = "om-action om-compat-settings";
  panel.innerHTML = ommegaUiLocalizeHtml('<label for="ommega-global-compat"><span><strong>正数密钥 ID 兼容 / Positive key ID compatibility</strong><small>对所有调用者生效 / Applies to all callers</small></span><input id="ommega-global-compat" type="checkbox" role="switch" disabled></label><div id="ommega-global-compat-status" role="status" aria-live="polite">正在读取… / Loading…</div>');
  slot.appendChild(panel);
  const box = document.getElementById("ommega-global-compat");
  const status = document.getElementById("ommega-global-compat-status");
  box.addEventListener("change", ommegaGlobalCompatChange);
  try {
    ({enabled:ommegaCompatEnabled} = await ommegaReadTargetCompat());
    box.checked = ommegaCompatEnabled;
    box.disabled = false;
    status.textContent = "";
  } catch (error) { status.textContent = ommegaUiLocalize("读取失败 / Could not read settings") + ": " + error.message; }
}
if (typeof window !== "undefined") window.addEventListener("ommega-settings-ready", ommegaInstallGlobalCompat);

// ── Ommega relay verdict (remote-health.json) ───────────────────────────────
// The keymint daemon records the last relay verdict, but this prebuilt bundle
// never read it: a server-side 401 or 429 stayed invisible and users retried the
// "enable remote" checkbox instead. Show the recorded cause in the remote config
// dialog, directly below that checkbox.
const OMMEGA_REMOTE_HEALTH = "/data/misc/keystore/ommega/remote-health.json";

function ommegaHealthAdvice(kind) {
  if (kind === "attestation") {
    return "证书链验证或密钥保存失败。 / Certificate validation or key storage failed.";
  }
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
    const {errno, stdout, stderr} = await ommegaReadOnly(`cat "${OMMEGA_REMOTE_HEALTH}" 2>/dev/null || true`);
    if (errno !== 0) throw new Error(stderr || "Could not read relay status");
    const raw = String(stdout === undefined || stdout === null ? "" : stdout).trim();
    if (errno === 0 && raw !== "") report = ommegaHealthReport(JSON.parse(raw));
  } catch (error) {
    // A half-written or malformed snapshot must not read as "nothing to report".
    report = "无法读取中继状态（remote-health.json 解析失败） / " +
      "Could not read the relay status (remote-health.json is unreadable).";
  }
  panel.textContent = ommegaUiLocalize("KeyMint 远程中继 / KeyMint remote relay") + "\n" + ommegaUiLocalize(report === ""
    ? "中继状态：暂无失败记录 / Relay status: no failures recorded yet."
    : report);
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
// The software TA owns the native HAL name in both modes. Its own remote.conf
// selects local material or a separate SOTER relay without changing the
// Integrity relay configuration managed by the other dialog.
const OMMEGA_SOTERTA_DIR = "/data/adb/ommega/soterta";
const OMMEGA_SOTERTA_FLAG = OMMEGA_SOTERTA_DIR + "/enabled";
const OMMEGA_SOTERTA_STATUS = OMMEGA_SOTERTA_DIR + "/status.json";
const OMMEGA_SOTER_REMOTE_CONFIG = OMMEGA_SOTERTA_DIR + "/remote.conf";
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
    lines.push("本地账本设备 ID：" + status.device_id + "（账本：" +
      (status.ledger ? "已就绪" : "缺失，下次启动会重建") + "）/ Local ledger device id: " +
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
  const {errno, stdout, stderr} = await ommegaReadOnly(
    'if [ -f "' + OMMEGA_SOTERTA_FLAG + '" ]; then echo OMMEGA_FLAG=1; else echo OMMEGA_FLAG=0; fi\n' +
    'cat "' + OMMEGA_SOTERTA_STATUS + '" 2>/dev/null'
  );
  if (errno && !String(stdout).startsWith("OMMEGA_FLAG=")) throw new Error(stderr || "Could not read Soter state");
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
  const refresh = document.getElementById("ommega-soterta-refresh");
  if (!report) return;
  if (refresh) { refresh.disabled = true; refresh.setAttribute?.("aria-busy", "true"); }
  try {
    const state = await ommegaSotertaRead();
    if (box) box.checked = state.enabled;
    const lines = [ommegaSotertaReport(state.known, state.enabled, state.status)];
    if (pending) lines.push("开关已写入，看门狗会在几秒内接管（要先停掉原厂 HAL）/ " +
      "Switch written; the watchdog applies it within a few seconds (it stops the stock HAL first).");
    report.textContent = ommegaUiLocalize(lines.join("\n\n"));
  } catch (error) {
    report.textContent = ommegaUiLocalize("读取 Soter 状态失败 / Could not read the Soter state") + ": " + error;
  } finally {
    if (refresh) { refresh.disabled = false; refresh.setAttribute?.("aria-busy", "false"); }
  }
}

// The flag is the request; the watchdog reacts within its poll interval and the
// takeover itself takes a few seconds (it stops a live HAL first), so the panel
// samples twice instead of pretending the switch is instant.
const OMMEGA_SOTERTA_REFRESH_MS = [3000, 9000];

async function ommegaSotertaApply() {
  const box = document.getElementById("ommega-soterta-enabled");
  const report = document.getElementById("ommega-soterta-report");
  const on = !!(box && box.checked);
  if (report) report.textContent = ommegaUiLocalize("正在写入开关… / Writing the switch…");
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
  dialog.style.cssText = "max-width:min(560px,calc(100vw - 24px));max-height:calc(100dvh - 24px)";
  dialog.innerHTML =
    '<div slot="headline">Soter HAL</div>' +
    '<div slot="content" style="display:flex;flex-direction:column;gap:12px;max-height:min(60dvh,520px);min-height:0;overflow-y:auto;overscroll-behavior:contain;padding-bottom:20px">' +
      '<div id="ommega-soterta-report" style="font-size:12px;line-height:1.5;white-space:pre-wrap;word-break:break-word;opacity:.85"></div>' +
      '<button type="button" id="ommega-soterta-refresh" class="text-button" style="align-self:flex-end">刷新 / Refresh</button>' +
      '<label class="config-option sheet-switch-row"><span class="toggle-copy"><strong>启用软件 TA / Enable software TA</strong></span><span class="switch"><input id="ommega-soterta-enabled" type="checkbox" role="switch"><span></span></span></label>' +
      '<div style="font-size:11px;line-height:1.4;opacity:.7">' +
        '软件 TA 接管原生 Soter HAL。远程关闭时使用本地自洽账本；远程开启后使用下方独立服务器，远程失败不会混用本地密钥。 / ' +
        'The software TA owns the native HAL. Remote off uses its local ledger; remote on uses the separate server below and fails closed if that relay fails.' +
      '</div>' +
      '<label class="config-option sheet-switch-row"><span class="toggle-copy"><strong>Soter 远程中继 / Remote Soter relay</strong></span><span class="switch"><input id="ommega-soter-remote-enabled" type="checkbox" role="switch"><span></span></span></label>' +
      '<md-outlined-text-field id="ommega-soter-remote-url" label="Soter 服务器 URL" type="url"></md-outlined-text-field>' +
      '<md-outlined-text-field id="ommega-soter-remote-device" label="Soter B 设备 ID"></md-outlined-text-field>' +
      '<md-outlined-text-field id="ommega-soter-remote-token" label="Soter Token" type="password"></md-outlined-text-field>' +
      '<md-outlined-text-field id="ommega-soter-remote-uid-map" label="UID 映射（A=B，可留空）"></md-outlined-text-field>' +
      '<label class="config-option sheet-switch-row"><span class="toggle-copy"><strong>允许自签 TLS 证书 / Accept self-signed TLS</strong></span><span class="switch"><input id="ommega-soter-remote-tls-insecure" type="checkbox" role="switch"><span></span></span></label>' +
      '<div id="ommega-soter-remote-result" style="font-size:12px;white-space:pre-wrap;word-break:break-word"></div>' +
    '</div>' +
    '<div slot="actions">' +
      '<button type="button" id="ommega-soter-remote-save" class="primary-button">保存 / Save</button>' +
      '<button type="button" id="ommega-soterta-close" class="secondary-button">关闭 / Close</button>' +
    '</div>';
  dialog.innerHTML = ommegaUiLocalizeHtml(dialog.innerHTML);
  const englishLabels = {url:"Soter server URL",device:"Soter B device ID","uid-map":"UID mapping (A=B, optional)"};
  if (ommegaUiLocalize("中文 / English") === "English") {
    for (const [key, label] of Object.entries(englishLabels)) dialog.querySelector("#ommega-soter-remote-" + key)?.setAttribute("label", label);
  }
  wrapper.appendChild(dialog);
  const box = dialog.querySelector("#ommega-soterta-enabled");
  if (box) box.addEventListener("click", () => { ommegaSotertaApply(); });
  const refresh = dialog.querySelector("#ommega-soterta-refresh");
  if (refresh) refresh.addEventListener("click", () => { ommegaSotertaRefresh(false); });
  const saveRemote = dialog.querySelector("#ommega-soter-remote-save");
  if (saveRemote) saveRemote.addEventListener("click", () => { ommegaSoterRemoteSave(); });
  const close = dialog.querySelector("#ommega-soterta-close");
  if (close) close.addEventListener("click", () => { dialog.close(); });
  return dialog;
}

function ommegaSotertaOpen() {
  const dialog = ommegaSotertaDialog();
  if (!dialog) return Promise.resolve();
  if (typeof dialog.show === "function") dialog.show();
  return Promise.all([ommegaSotertaRefresh(false), ommegaSoterRemoteLoad()]);
}

function ommegaSotertaInstallMenu() {
  if (document.getElementById("ommega-soterta")) return;
  const anchor = document.getElementById("remote-config");
  if (!anchor || typeof anchor.insertAdjacentElement !== "function") return;
  const item = document.createElement("md-menu-item");
  item.id = "ommega-soterta";
  item.className = "automation-menu-item";
  item.innerHTML = '<div slot="headline">Soter 原生 HAL 与远程中继 / Native Soter HAL and relay</div>' +
    '<md-icon slot="end">fingerprint</md-icon>';
  item.innerHTML = ommegaUiLocalizeHtml(item.innerHTML);
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

function ommegaSoterRemoteParse(raw) {
  const out = {enabled: false, url: "", token: "", device_id: "", uid_map: "", tls_insecure: false};
  for (const line of String(raw || "").split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const at = trimmed.indexOf("=");
    if (at < 0) continue;
    const key = trimmed.slice(0, at).trim();
    const value = trimmed.slice(at + 1).trim();
    if (!(key in out)) continue;
    if (key === "enabled" || key === "tls_insecure") out[key] = /^(1|true|yes|on)$/i.test(value);
    else out[key] = value;
  }
  return out;
}

function ommegaSoterRemoteField(id) {
  const element = document.getElementById("ommega-soter-remote-" + id);
  return element ? element.value.trim() : "";
}

async function ommegaSoterRemoteLoad() {
  const result = document.getElementById("ommega-soter-remote-result");
  try {
    const {errno, stdout, stderr} = await ommegaReadOnly('cat "' + OMMEGA_SOTER_REMOTE_CONFIG + '" 2>/dev/null || true');
    if (errno) throw new Error(stderr || "Could not read Soter relay config");
    const cfg = ommegaSoterRemoteParse(stdout);
    for (const key of ["url", "token", "uid_map"]) {
      const element = document.getElementById("ommega-soter-remote-" + key.replaceAll("_", "-"));
      if (element) element.value = cfg[key];
    }
    const device = document.getElementById("ommega-soter-remote-device");
    if (device) device.value = cfg.device_id;
    for (const key of ["enabled", "tls_insecure"]) {
      const element = document.getElementById("ommega-soter-remote-" + key.replaceAll("_", "-"));
      if (element) element.checked = cfg[key];
    }
    if (result) result.textContent = ommegaUiLocalize(cfg.enabled
      ? "Soter 使用独立远程服务器；原生 HAL 仍由软件 TA 接管。 / Soter uses a separate relay server; the software TA still owns the HAL."
      : "Soter 远程已关闭；软件 TA 使用本地账本。 / Remote Soter is off; the software TA uses its local ledger.");
  } catch (error) {
    if (result) result.textContent = "读取 Soter 远程配置失败 / Could not read Soter relay config: " + error;
  }
}

function ommegaSoterRemoteBase64(text) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

async function ommegaSoterRemoteSave() {
  const result = document.getElementById("ommega-soter-remote-result");
  const enabled = !!document.getElementById("ommega-soter-remote-enabled")?.checked;
  const tls = !!document.getElementById("ommega-soter-remote-tls-insecure")?.checked;
  const url = ommegaSoterRemoteField("url");
  const token = ommegaSoterRemoteField("token");
  const device = ommegaSoterRemoteField("device");
  const uidMap = ommegaSoterRemoteField("uid-map");
  const values = [url, token, device, uidMap];
  if (values.some(value => /[\r\n]/.test(value))) {
    if (result) result.textContent = ommegaUiLocalize("配置中不能有换行 / Newlines are not allowed.");
    return;
  }
  if (enabled) {
    let validUrl = false;
    try {
      const parsed = new URL(url);
      validUrl = ["http:", "https:"].includes(parsed.protocol) && !!parsed.hostname && !parsed.username && !parsed.password;
    } catch (error) { /* invalid URL */ }
    if (!validUrl || !token || !device) {
      if (result) result.textContent = ommegaUiLocalize("启用前请填写 HTTP(S) URL、Token 和 B 设备 ID。 / Enter an HTTP(S) URL, token and B device ID before enabling.");
      return;
    }
  }
  const contents = [
    "enabled=" + enabled,
    "url=" + url,
    "token=" + token,
    "device_id=" + device,
    "tls_insecure=" + tls,
    "uid_map=" + uidMap,
    "",
  ].join("\n");
  // Only base64 characters enter the shell command; no token or URL is ever
  // interpolated as shell syntax. Same-directory rename makes reads atomic.
  const encoded = ommegaSoterRemoteBase64(contents);
  const command = [
    "set -e",
    "umask 077",
    'mkdir -p "' + OMMEGA_SOTERTA_DIR + '"',
    "printf '%s' '" + encoded + "' | base64 -d > '" + OMMEGA_SOTER_REMOTE_CONFIG + ".tmp'",
    'chmod 0600 "' + OMMEGA_SOTER_REMOTE_CONFIG + '.tmp"',
    'mv -f "' + OMMEGA_SOTER_REMOTE_CONFIG + '.tmp" "' + OMMEGA_SOTER_REMOTE_CONFIG + '"',
  ].join("\n");
  if (result) result.textContent = ommegaUiLocalize("正在保存 Soter 远程配置… / Saving Soter relay config…");
  try {
    const response = await _(command);
    if (Number(response.errno) !== 0) throw new Error(String(response.stderr || "write failed"));
    if (result) result.textContent = enabled
      ? "Soter 远程配置已保存；软件 TA 下一笔调用起生效。Integrity 服务器设置未改动。"
      : "Soter 远程已关闭；软件 TA 下一笔调用起使用本地账本。";
  } catch (error) {
    if (result) result.textContent = "保存失败 / Save failed: " + error;
  }
}


// Global scope reuses [filter].enabled in injector.toml: false means all regular
// applications and exactly bound UID-1000 apps, excluding core system services.
// The saved target lists and per-app policies are never rewritten by this switch.
const OMMEGA_SCOPE_CONFIG = "/data/adb/ommega/ommegadata/injector.toml";
let ommegaGlobalScopeEnabled = false;
let ommegaGlobalScopeKnown = false;
let ommegaGlobalScopePending = false;
let ommegaGlobalScopeMessage = "";

function ommegaScopeSetting(contents) {
  const lines = String(contents).split("\n");
  let section = -1;
  let index = -1;
  let inFilter = false;
  let match = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line || line.startsWith("#")) continue;
    // Do not mistake a table-looking line inside a multiline value for a table.
    if (line.includes('"""') || line.includes("'''")) {
      throw new Error("Multiline TOML values require editing injector.toml directly");
    }
    if (line.startsWith("[")) {
      inFilter = /^\[\s*(?:filter|"filter"|'filter')\s*\]\s*(?:#.*)?$/.test(line);
      if (inFilter) {
        if (section !== -1) throw new Error("Duplicate filter table");
        section = i;
      }
      continue;
    }
    if (inFilter && /^(?:enabled|"enabled"|'enabled')\s*=/.test(line)) {
      if (index !== -1) throw new Error("Duplicate filter.enabled");
      match = lines[i].match(/^(\s*(?:enabled|"enabled"|'enabled')\s*=\s*)(true|false)([ \t]*(?:#[^\r]*)?\r?)$/);
      if (!match) throw new Error("Invalid filter.enabled");
      index = i;
    }
  }
  return {lines, section, index, match, global: !!match && match[2] === "false"};
}

function ommegaScopeConfig(contents, global) {
  const parsed = ommegaScopeSetting(contents);
  const value = global ? "false" : "true";
  if (parsed.index !== -1) {
    parsed.lines[parsed.index] = parsed.match[1] + value + parsed.match[3];
  } else if (parsed.section !== -1) {
    parsed.lines.splice(parsed.section + 1, 0, "enabled = " + value +
      (parsed.lines[parsed.section].endsWith("\r") ? "\r" : ""));
  } else {
    return String(contents) + (String(contents).endsWith("\n") ? "" : "\n") +
      "\n[filter]\nenabled = " + value + "\n";
  }
  return parsed.lines.join("\n");
}

function ommegaScopeWriteCommand(contents, global) {
  const before = ommegaSoterRemoteBase64(contents);
  const after = ommegaSoterRemoteBase64(ommegaScopeConfig(contents, global));
  return [
    "set -e",
    'ommega_scope_file="' + OMMEGA_SCOPE_CONFIG + '"',
    'ommega_scope_tmp="$ommega_scope_file.scope.$$"',
    'trap \'rm -f "$ommega_scope_tmp"\' EXIT',
    'ommega_scope_current=$(base64 < "$ommega_scope_file" | tr -d \'\\r\\n\')',
    `[ "$ommega_scope_current" = '${before}' ] || { echo "Config changed; retry" >&2; exit 75; }`,
    "printf '%s' '" + after + "' | base64 -d > \"$ommega_scope_tmp\"",
    'chmod 0644 "$ommega_scope_tmp"',
    'chown 1017:1017 "$ommega_scope_tmp"',
    'mv -f "$ommega_scope_tmp" "$ommega_scope_file"',
    "trap - EXIT",
  ].join("\n");
}

function ommegaScopeSync() {
  const box = document.getElementById("ommega-global-scope-enabled");
  const report = document.getElementById("ommega-global-scope-status");
  const list = document.getElementById("apps-list");
  if (!box || !report || !list) return;
  if (!ommegaGlobalScopePending) box.checked = ommegaGlobalScopeEnabled;
  box.disabled = !ommegaGlobalScopeKnown || ommegaGlobalScopePending;
  const unavailable = !ommegaGlobalScopeKnown || ommegaGlobalScopePending;
  list.inert = unavailable;
  list.setAttribute("aria-disabled", String(unavailable));
  list.style.opacity = unavailable ? "0.45" : "";
  list.style.pointerEvents = unavailable ? "none" : "";
  for (const id of ["select-all", "deselect-all", "select-denylist"]) {
    const control = document.getElementById(id);
    if (control) control.disabled = unavailable || ommegaGlobalScopeEnabled;
  }
  for (const id of ["deselect-unnecessary", "add-system-app", "save"]) {
    const control = document.getElementById(id);
    if (control) control.disabled = unavailable;
  }
  report.textContent = ommegaGlobalScopeMessage || ommegaUiLocalize(ommegaGlobalScopeEnabled
    ? "已开启：覆盖用户应用和普通系统应用，排除核心系统服务；target.txt 不参与作用域判断，单独作用域设置暂不生效。应用策略仍可使用。 / On: user apps and regular system apps are in scope; core system services are excluded. target.txt is ignored and individual scope is paused; app policies remain available."
    : "已关闭：按各应用的作用域开关生效。 / Off: each application's scope switch applies.");
}

async function ommegaScopeRead() {
  // The manager's exec bridge joins stdout lines and drops their terminators.
  // Transport the file as base64 so CRLF and the final newline survive exactly;
  // the atomic writer compares these original bytes before replacing the file.
  const result = await ommegaReadOnly('base64 < "' + OMMEGA_SCOPE_CONFIG + '"');
  if (Number(result.errno) !== 0) throw new Error(result.stderr || "Could not read injector.toml");
  const bytes = Uint8Array.from(atob(String(result.stdout).replace(/\s/g, "")), byte => byte.charCodeAt(0));
  const contents = new TextDecoder("utf-8", {fatal: true, ignoreBOM: true}).decode(bytes);
  const setting = ommegaScopeSetting(contents);
  ommegaGlobalScopeEnabled = setting.global;
  ommegaGlobalScopeKnown = true;
  return contents;
}

async function ommegaRefreshGlobalScope() {
  if (ommegaGlobalScopePending) return;
  try {
    await ommegaScopeRead();
    ommegaGlobalScopeMessage = "";
  } catch (error) {
    ommegaGlobalScopeKnown = false;
    ommegaGlobalScopeMessage = ommegaUiLocalize("无法读取作用域状态 / Could not read scope") + ": " + error.message;
  }
  ommegaScopeSync();
}

async function ommegaApplyGlobalScope() {
  if (ommegaGlobalScopePending || !ommegaGlobalScopeKnown) return;
  const requested = document.getElementById("ommega-global-scope-enabled").checked;
  ++ommegaTargetSnapshotGeneration;
  ommegaGlobalScopePending = true;
  ommegaGlobalScopeMessage = ommegaUiLocalize("正在保存… / Saving…");
  ommegaScopeSync();
  try {
    const contents = await ommegaScopeRead();
    const response = await _(ommegaScopeWriteCommand(contents, requested));
    if (Number(response.errno) !== 0) throw new Error(response.stderr || "Write failed");
    await ommegaScopeRead();
    if (ommegaGlobalScopeEnabled !== requested) throw new Error("Scope readback did not match");
    ommegaGlobalScopeMessage = "";
  } catch (error) {
    try { await ommegaScopeRead(); } catch { ommegaGlobalScopeKnown = false; }
    ommegaGlobalScopeMessage = ommegaUiLocalize("保存失败 / Could not save scope") + ": " + error.message;
  } finally {
    ommegaGlobalScopePending = false;
    ommegaScopeSync();
  }
}

function ommegaInstallGlobalScope() {
  const list = document.getElementById("apps-list");
  const slot = document.getElementById("om-global-scope-slot");
  if (!list || !slot || typeof slot.appendChild !== "function" || document.getElementById("ommega-global-scope")) return false;
  const panel = document.createElement("section");
  panel.id = "ommega-global-scope";
  panel.style.cssText = "margin:10px 0;padding:14px 18px;border:1px solid var(--om-line,#bbb);border-radius:18px;" +
    "background:var(--om-surface,#eee);color:var(--om-ink,#222)";
  panel.innerHTML = ommegaUiLocalizeHtml('<style>#ommega-global-scope label{display:flex;align-items:center;gap:12px;min-height:48px;cursor:pointer}' +
    '#ommega-global-scope input{appearance:none;flex:none;width:48px;height:28px;border:2px solid currentColor;' +
    'border-radius:18px;position:relative;cursor:pointer;background:transparent;color:inherit}' +
    '#ommega-global-scope input:before{content:"";position:absolute;width:18px;height:18px;left:3px;top:3px;' +
    'border-radius:50%;background:currentColor;transition:transform .15s}' +
    '#ommega-global-scope input:checked{background:var(--md-sys-color-primary,#465b91);color:var(--md-sys-color-on-primary,#fff);border-color:var(--md-sys-color-primary,#465b91)}' +
    '#ommega-global-scope input:checked:before{transform:translateX(20px)}' +
    '#ommega-global-scope input:focus-visible{outline:3px solid var(--md-sys-color-primary,#465b91);outline-offset:4px}' +
    '#ommega-global-scope input:disabled{opacity:.5;cursor:wait}' +
    '@media(prefers-reduced-motion:reduce){#ommega-global-scope input:before{transition:none}}</style>' +
    '<label for="ommega-global-scope-enabled"><input type="checkbox" role="switch" id="ommega-global-scope-enabled" ' +
    'aria-describedby="ommega-global-scope-status" disabled><strong>全局作用域 / Global scope</strong></label>' +
    '<div id="ommega-global-scope-status" role="status" aria-live="polite" style="font-size:13px;line-height:1.5;margin-top:4px">正在读取… / Loading…</div>');
  slot.appendChild(panel);
  document.getElementById("ommega-global-scope-enabled").addEventListener("change", ommegaApplyGlobalScope);
  const save = document.getElementById("save");
  const baseSave = save.onclick;
  save.onclick = function (event) {
    if (!ommegaGlobalScopeKnown || ommegaGlobalScopePending) return;
    return baseSave ? baseSave.call(this, event) : undefined;
  };
  ommegaScopeSync();
  return true;
}

ommegaInstallGlobalScope();
if (typeof window !== "undefined") window.addEventListener("ommega-settings-ready", ommegaInstallGlobalScope);
