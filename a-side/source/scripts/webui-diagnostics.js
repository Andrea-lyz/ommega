
// ── Ommega diagnostics: level-2 overview and level-3 feature pages ──────────
// Applied by scripts/webui-patch.py. All probes are read-only; the only write
// is the debug-logging master switch, which rewrites the A-side config and
// asks the KeyMint daemon to reload.
const OMMEGA_DIAG_FLAT = "/data/adb/ommega/ommegadata/config";
const OMMEGA_DIAG_TOML = "/data/adb/ommega/ommegadata/config.toml";
const OMMEGA_DIAG_RESTART = "/data/adb/ommega/restart.keymint";
const OMMEGA_DIAG_PROBE_PKG = "aprobes.agap";
const OMMEGA_DIAG_PROBE_FILE = "/storage/emulated/0/Android/data/aprobes.agap/files/a-gap-probe.txt";
let ommegaDiagUnlocked = false, ommegaDiagBusy = false, ommegaDiagMaster = false;
const OMMEGA_DIAG_UNLOCK_KEY = "ommega.diag.unlocked";

function ommegaDiagStoredUnlock() {
  try { return localStorage.getItem(OMMEGA_DIAG_UNLOCK_KEY) === "1"; } catch { return false; }
}
function ommegaDiagStoreUnlock(value) {
  try { value ? localStorage.setItem(OMMEGA_DIAG_UNLOCK_KEY, "1") : localStorage.removeItem(OMMEGA_DIAG_UNLOCK_KEY); } catch {}
}

function ommegaDiagEsc(value) {
  return String(value == null ? "" : value).replace(/[&<>]/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;"}[c]));
}
function ommegaDiagT(zh, en) { return ommegaUiLocalize(zh + " / " + en); }

// One read-only command per page; the shell keeps every probe bounded.
const OMMEGA_DIAG_PAGES = [
  { id: "diag-keymint", zh: "KeyMint 密钥服务", en: "KeyMint keystore",
    zhNote: "本地 / 中继两种模式下的签名、证明与认证", enNote: "Signing, attestation and authentication in both modes",
    command: ["set +e",
      'C=/data/adb/ommega/ommegadata/config.toml',
      'L=/data/adb/ommega/ommegadata/logs/keymint.log',
      'echo "==mode"', 'grep -E "^(enabled|url|device_id) " "$C" 2>/dev/null',
      'T=/data/local/tmp/om-diag-tail',
      'tail -c 262144 "$L" > "$T" 2>/dev/null',
      'echo "==signals"',
      'grep -a -c "certificate_policy=local" "$T" 2>/dev/null',
      'grep -a -c "failing Device" "$T" 2>/dev/null',
      'echo "==recent"',
      'grep -a -E "certificate_policy|failing Device|ErrorCode|remote_attest_identity" "$T" 2>/dev/null | tail -n 30',
      'echo "==tail"', 'tail -n 40 "$L" 2>/dev/null'].join("\n") },
  { id: "diag-injector", zh: "注入器 Injector", en: "Injector",
    zhNote: "按应用的路由决策与改写失败", enNote: "Per-app routing decisions and rewrite failures",
    command: ["set +e",
      'L=/data/local/tmp/om-diag-tail-i',
      'tail -c 262144 /data/adb/ommega/ommegadata/logs/injector.log > "$L" 2>/dev/null',
      'echo "==counts"',
      'grep -a -c "event=route" "$L" 2>/dev/null',
      'grep -a -c "allowed=false" "$L" 2>/dev/null',
      'echo "==packages"',
      'grep -a -o "packages=\[[^]]*\]" "$L" 2>/dev/null | sort | uniq -c | sort -rn | head -n 8',
      'echo "==failures"',
      'grep -a "failed for uid=" "$L" 2>/dev/null | tail -n 20'].join("\n") },
  { id: "diag-relay", zh: "中继连接 Relay", en: "Relay connection",
    zhNote: "服务器、设备标识与最近一次往返结果", enNote: "Server, device id and the latest round trip",
    command: ["set +e",
      'D=/data/adb/ommega/ommegadata',
      'echo "==config"', 'grep -E "^(url|device_id|remote|tls_insecure):" "$D/config" 2>/dev/null',
      'echo "==health"', 'cat "$D/remote-health.json" 2>/dev/null; echo',
      'echo "==last"',
      'grep -a "relay_roundtrip" "$D/logs/keymint.log" 2>/dev/null | tail -n 6'].join("\n") },
  { id: "diag-cpu", zh: "设备属性视图", en: "Device property view",
    zhNote: "CPU 与设备属性伪装是否真正注入", enNote: "Whether the CPU/property mask reached the app",
    command: ["set +e",
      'D=/data/misc/keystore/ommega/data',
      'echo "==config"', 'cat "$D/cpu-mask.conf" 2>/dev/null; echo',
      'echo "==profile"', 'test -f "$D/cpu-mask-profile.conf" && echo present || echo missing',
      'echo "==log"', 'logcat -d -s MaskCPU 2>/dev/null | tail -n 30'].join("\n") },
  { id: "diag-soter", zh: "SOTER 软件 TA", en: "SOTER software TA",
    zhNote: "接管状态、设备标识与最近错误；详细叙述由调试总阀控制并写入 soterta.debug",
    enNote: "Takeover state, device id and recent errors; narration follows the master switch and lands in soterta.debug",
    command: ["set +e",
      'S=/data/adb/ommega/soterta',
      'echo "==enabled"', '[ -f "$S/enabled" ] && echo on || echo off',
      'echo "==status"', 'cat "$S/status.json" 2>/dev/null; echo',
      'echo "==errors"', 'tail -n 20 "$S/soterta.err" 2>/dev/null',
      'echo "==narration"', 'tail -n 20 "$S/soterta.debug" 2>/dev/null',
      'echo "==logs"',
      'grep -a -i "soter" /data/adb/ommega/ommegadata/logs/keymint.log 2>/dev/null | tail -n 10'].join("\n") }
];

// ---------------------------------------------------------------- helpers
function ommegaDiagB64(text) { return btoa(unescape(encodeURIComponent(text))); }
async function ommegaDiagRead(path) {
  const response = await ommegaReadOnly("cat '" + path + "' 2>/dev/null");
  return response && Number(response.errno) === 0 ? String(response.stdout || "") : "";
}
async function ommegaDiagWrite(path, text) {
  return _("printf '%s' '" + ommegaDiagB64(text) + "' | base64 -d > '" + path + "'");
}
function ommegaDiagSetFlat(text, on) {
  return String(text).replace(/^(\s*debug_logging:\s*)(true|false)/m, "$1" + (on ? "true" : "false"));
}
function ommegaDiagSetToml(text, on, currentLevel) {
  // Turning logging on must never weaken an existing trace configuration.
  const level = on ? (currentLevel === "trace" ? "trace" : "debug") : "info";
  return String(text)
    .replace(/^(\s*log_level\s*=\s*)"[^"]*"/m, "$1\"" + level + "\"")
    .replace(/^(\s*debug_logging\s*=\s*)(true|false)/m, "$1" + (on ? "true" : "false"));
}

function ommegaDiagItem(ok, zh, en, adviceZh, adviceEn) {
  return { ok, zh, en, adviceZh, adviceEn };
}

function ommegaDiagRenderItems(target, items) {
  target.innerHTML = "<ul class=\"om-diag-list\">" + items.map(item => {
    const hint = item.ok ? "" : "<span>" + ommegaDiagEsc(ommegaDiagT("建议：", "Advice: ") +
      ommegaDiagT(item.adviceZh, item.adviceEn)) + "</span>";
    return "<li class=\"om-diag-row\"><div class=\"om-diag-row-body\"><strong>" +
      (item.ok ? "\u2713 " : "\u26a0 ") + ommegaDiagEsc(ommegaDiagT(item.zh, item.en)) +
      "</strong>" + hint + "</div></li>";
  }).join("") + "</ul>";
}

// Each feature page answers the same three questions: what is wrong, what the
// device actually shows, and what to do next.
function ommegaDiagPageItems(page, output) {
  const lines = String(output || "").split("\n");
  const pick = prefix => {
    const index = lines.indexOf("==" + prefix);
    if (index < 0) return [];
    const out = [];
    for (let i = index + 1; i < lines.length && !lines[i].startsWith("=="); i++) if (lines[i].trim()) out.push(lines[i].trim());
    return out;
  };
  const items = [];
  if (page.id === "diag-keymint") {
    const mode = pick("mode");
    const relay = mode.some(line => /^enabled\s*=\s*true/.test(line));
    const signals = pick("signals").map(Number);
    const recent = pick("recent");
    const failure = signals[1] || 0;
    const cutoff = Date.now() - 15 * 60 * 1000;
    const inWindow = line => {
      const match = /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})/.exec(line);
      if (!match) return false;
      const stamp = Date.parse(match[1].replace(" ", "T") + "Z");
      return Number.isFinite(stamp) && stamp >= cutoff;
    };
    const recentLocal = recent.filter(line => line.indexOf("certificate_policy=local") >= 0 && inWindow(line)).length;
    const recentFailure = recent.filter(line => /failing Device|ErrorCode\(-/.test(line) && inWindow(line)).length;
    items.push(ommegaDiagItem(true, "当前模式：" + (relay ? "中继" : "本地"), "Mode: " + (relay ? "relay" : "local"), "", ""));
    items.push(ommegaDiagItem(true, "本地出证次数：" + (signals[0] || 0), "Local certificates: " + (signals[0] || 0), "", ""));
    items.push(ommegaDiagItem(failure === 0, "失败生成次数：" + failure, "Failed generations: " + failure,
      "打开「请求异常」查看具体应用与错误码", "Open Request failures for the app and error code"));
    items.push(ommegaDiagItem(recentFailure === 0,
      "最近 15 分钟：本地出证 " + recentLocal + " 次 · 失败 " + recentFailure + " 次",
      "Last 15 minutes: " + recentLocal + " local certificates, " + recentFailure + " failures",
      "最近有失败：打开「请求异常」查看具体应用", "Recent failures; open Request failures for the app"));
    items.push(ommegaDiagItem(true, "近期关键记录：" + recent.length + " 条",
      "Recent key events: " + recent.length, "", ""));
  } else if (page.id === "diag-injector") {
    const counts = pick("counts").map(Number);
    const failures = pick("failures");
    items.push(ommegaDiagItem((counts[0] || 0) > 0, "路由调用：" + (counts[0] || 0), "Routed calls: " + (counts[0] || 0),
      "注入器尚未记录路由，确认受管应用已启动且模块已生效", "No routed calls yet; confirm a managed app started with the module active"));
    items.push(ommegaDiagItem((counts[1] || 0) === 0, "被拒绝：" + (counts[1] || 0), "Denied: " + (counts[1] || 0),
      "有调用被拒绝：检查应用是否在「应用作用域」内以及策略模式", "Some calls were denied; check the app scope and policy mode"));
    items.push(ommegaDiagItem(failures.length === 0, "失败记录：" + failures.length + " 条", "Failures: " + failures.length,
      "查看下方原始日志，确认失败的具体方法与错误", "Read the raw log below for the failing method and error"));
  } else if (page.id === "diag-relay") {
    const config = pick("config").join(" ");
    const health = pick("health").join(" ");
    const hasUrl = /url:/.test(config);
    let verdict = "unknown";
    try {
      const parsed = JSON.parse(health.slice(health.indexOf("{"), health.lastIndexOf("}") + 1));
      const failed = Number(parsed.last_error_unix || 0), okAt = Number(parsed.last_ok_unix || 0);
      verdict = parsed.last_kind === "error" || failed > okAt ? "error" : okAt > 0 ? "ok" : "unknown";
    } catch {}
    items.push(ommegaDiagItem(hasUrl, "服务器与设备 ID：" + (hasUrl ? "已配置" : "缺失"),
      "Server and device id: " + (hasUrl ? "configured" : "missing"),
      "在「远程连接」中填写服务器地址、设备 ID 与 token", "Fill the server URL, device id and token under Remote connection"));
    items.push(ommegaDiagItem(verdict === "ok", "最近往返：" + verdict, "Last round trip: " + verdict,
      "检查 token 是否与 B 端一致、B 端是否在线，以及服务器端口是否可达",
      "Check that the token matches B, that B is online and the server port is reachable"));
  } else if (page.id === "diag-cpu") {
    const config = pick("config").join(" ");
    const profilePresent = (pick("profile")[0] || "") === "present";
    const logLines = pick("log");
    const enabled = /enabled=true/.test(config);
    const applied = logLines.some(line => /error=none/.test(line));
    items.push(ommegaDiagItem(enabled, "属性视图：" + (enabled ? "已启用" : "未启用"),
      "Property view: " + (enabled ? "enabled" : "disabled"),
      "在「CPU 与设备属性」中启用并选择目标应用", "Enable it under CPU and device properties and pick the target apps"));
    items.push(ommegaDiagItem(profilePresent, "设备快照：" + (profilePresent ? "已存在" : "缺失"),
      "Device snapshot: " + (profilePresent ? "present" : "missing"),
      "先在中继端拉取一次设备快照，再启用属性视图", "Pull a device snapshot from the relay before enabling the view"));
    items.push(ommegaDiagItem(applied, applied ? "注入结果：成功" : "注入结果：未见成功记录",
      "Injection: " + (applied ? "applied" : "no success yet"),
      "确认 Zygisk 已启用、应用已被强停重开，并检查 MaskCPU 日志",
      "Confirm Zygisk is enabled, restart the app, then check the MaskCPU log"));
  } else if (page.id === "diag-soter") {
    const enabled = (pick("enabled")[0] || "off") === "on";
    const status = pick("status").join(" ");
    const errors = pick("errors");
    const narration = pick("narration");
    const running = /"running"\s*:\s*(true|1)/.test(status);
    items.push(ommegaDiagItem(true, "软件 TA：" + (enabled ? "已请求接管" : "未启用"),
      "Software TA: " + (enabled ? "takeover requested" : "disabled"), "", ""));
    items.push(ommegaDiagItem(!enabled || running, "运行状态：" + (running ? "运行中" : enabled ? "未运行" : "原厂 HAL"),
      "Runtime: " + (running ? "running" : enabled ? "not running" : "stock HAL"),
      "接管未完成：查看状态文件与错误日志，或先关闭接管恢复原厂 HAL",
      "Takeover is incomplete; check the status file and error log, or disable takeover"));
    items.push(ommegaDiagItem(errors.length === 0, "错误记录：" + errors.length + " 条", "Error lines: " + errors.length,
      "查看下方原始日志中的 soterta.err", "Read soterta.err in the raw log below"));
    items.push(ommegaDiagItem(true, "详细叙述：" + narration.length + " 条（由调试总阀控制）",
      "Narration lines: " + narration.length + " (driven by the master switch)", "", ""));
  }
  return items;
}

async function ommegaDiagLoad(page) {
  const summary = document.getElementById("om-" + page.id + "-summary");
  const output = document.getElementById("om-" + page.id + "-out");
  if (!output) return;
  output.textContent = ommegaDiagT("读取中…", "Reading…");
  // Let the route change paint before the shell work starts.
  await new Promise(resolve => setTimeout(resolve, 0));
  const response = await ommegaDiagCached(page.id, page.command);
  const text = String((response && response.stdout) || "");
  output.textContent = text.trim() || ommegaDiagT("(无输出)", "(no output)");
  if (summary) ommegaDiagRenderItems(summary, ommegaDiagPageItems(page, text));
}

function ommegaDiagEnsurePages() {
  const anchor = document.getElementById("om-settings");
  for (const page of OMMEGA_DIAG_PAGES) {
    const id = "om-" + page.id;
    if (document.getElementById(id) || !anchor) continue;
    const section = document.createElement("section");
    section.id = id;
    section.className = "om-view";
    section.hidden = true;
    section.setAttribute("aria-label", ommegaDiagT(page.zh, page.en));
    section.innerHTML = "<div class=\"om-page-heading\"><h1>" + ommegaDiagEsc(ommegaDiagT(page.zh, page.en)) + "</h1><p>" +
      ommegaDiagEsc(ommegaDiagT(page.zhNote, page.enNote)) + "</p></div>" +
      "<section class=\"panel settings-group\"><ul class=\"om-diag-summary\" id=\"om-" + page.id + "-summary\"></ul>" +
      "<button type=\"button\" class=\"om-button secondary\" id=\"om-" + page.id + "-run\">" +
      ommegaDiagEsc(ommegaDiagT("重新抓取", "Collect again")) + "</button>" +
      "<pre class=\"om-diag-log\" id=\"om-" + page.id + "-out\"></pre></section>";
    anchor.after(section);
    document.getElementById("om-" + page.id + "-run").onclick = () => { void ommegaDiagLoad(page); };
  }
}

let ommegaDiagMasterRefreshAt = 0;

async function ommegaDiagRefreshMaster(force = false) {
  // Re-entering the page re-reads two small files; throttle it so navigation
  // stays snappy while the state still settles within a couple of seconds.
  if (!force && Date.now() - ommegaDiagMasterRefreshAt < 2000) return;
  ommegaDiagMasterRefreshAt = Date.now();
  const toml = await ommegaDiagRead(OMMEGA_DIAG_TOML);
  const level = (toml.match(/^\s*log_level\s*=\s*"([^"]+)"/m) || [])[1] || "info";
  ommegaDiagMaster = level === "debug" || level === "trace";
  ommegaDiagUnlocked = ommegaDiagMaster && ommegaDiagStoredUnlock();
  const toggle = document.getElementById("om-diag-master");
  if (toggle) toggle.checked = ommegaDiagMaster;
  const note = document.getElementById("om-diag-master-note");
  if (note) note.textContent = ommegaDiagMaster
    ? ommegaDiagT("调试日志已开启。请复现问题，然后解锁功能诊断。", "Debug logging is on. Reproduce the issue, then unlock feature diagnostics.")
    : ommegaDiagT("调试日志已关闭。开启后可抓取详细记录。", "Debug logging is off. Enable it to capture detailed records.");
  const unlock = document.getElementById("om-diag-unlock");
  if (unlock) unlock.hidden = !ommegaDiagMaster;
  let version = document.getElementById("om-diag-version");
  if (!version) {
    const anchor = document.getElementById("om-diag-unlock-note");
    if (anchor) {
      version = document.createElement("p");
      version.id = "om-diag-version";
      version.className = "note";
      anchor.after(version);
    }
  }
  const rotationSelect = document.getElementById("om-diag-rotation");
  if (rotationSelect) {
    const rotationFile = await ommegaDiagRead(OMMEGA_DIAG_ROTATION);
    const match = /max_size_mb\s*=\s*(\d+)/.exec(rotationFile);
    rotationSelect.value = match ? String(Number(match[1]) || 4) : "4";
    const note = document.getElementById("om-diag-rotation-note");
    if (note && !match) note.textContent = ommegaDiagT("当前使用默认 4 MB。", "Default 4 MB is in use.");
    else if (note) note.textContent = ommegaDiagT("当前：" + rotationSelect.value + " MB。",
      "Current: " + rotationSelect.value + " MB.");
  }
  if (version) {
    const prop = version.dataset.version
      ? {stdout: version.dataset.version}
      : await ommegaReadOnly("grep -E '^version=' /data/adb/modules/ommega/module.prop 2>/dev/null");
    const value = String((prop && prop.stdout) || "").replace("version=", "").trim();
    if (value) version.dataset.version = value;
    version.textContent = ommegaDiagT("模块版本：", "Module: ") + (value || "unknown");
  }
  ommegaDiagApplyUnlock();
}

function ommegaDiagApplyUnlock() {
  const host = document.getElementById("om-diag-features");
  if (host) host.hidden = !(ommegaDiagMaster && ommegaDiagUnlocked);
  const note = document.getElementById("om-diag-unlock-note");
  if (note) note.textContent = ommegaDiagUnlocked
    ? ommegaDiagT("诊断已解锁。", "Diagnostics unlocked.")
    : ommegaDiagT("完成复现后点击上面的按钮解锁。", "After reproducing, tap the button above to unlock.");
}

async function ommegaDiagSetMaster(on) {
  if (ommegaDiagBusy) return;
  ommegaDiagBusy = true;
  try {
    const [toml, flat] = await Promise.all([ommegaDiagRead(OMMEGA_DIAG_TOML), ommegaDiagRead(OMMEGA_DIAG_FLAT)]);
    await ommegaDiagWrite(OMMEGA_DIAG_TOML, ommegaDiagSetToml(toml, on, (toml.match(/^\s*log_level\s*=\s*"([^"]+)"/m) || [])[1] || "info"));
    await ommegaDiagWrite(OMMEGA_DIAG_FLAT, ommegaDiagSetFlat(flat, on));
    await _("mkdir -p /data/adb/ommega/soterta; " +
      (on ? "touch " + OMMEGA_DIAG_SOTER_DEBUG : "rm -f " + OMMEGA_DIAG_SOTER_DEBUG + " /data/adb/ommega/soterta/soterta.debug"));
    await _("touch /data/adb/ommega/restart.all");
    ommegaDiagUnlocked = false;
    ommegaDiagStoreUnlock(false);
  } finally {
    ommegaDiagBusy = false;
    await ommegaDiagRefreshMaster();
  }
}


// Clear every module log and let KeyMint reopen fresh files. The daemon watches
// restart.keymint, so no service is killed directly from the UI.
async function ommegaDiagClearLogs() {
  const note = document.getElementById("om-diag-clear-note");
  const show = text => { if (note) note.textContent = text; };
  show(ommegaDiagT("正在清除…", "Clearing…"));
  const command = ["set +e",
    'D=/data/adb/ommega/ommegadata/logs',
    'count=0; bytes=0',
    'for f in "$D"/*.log "$D"/*.log.1 "$D"/*.log.2 "$D"/*.log.3; do',
    '  [ -f "$f" ] || continue',
    '  s=$(stat -c %s "$f" 2>/dev/null || echo 0)',
    '  bytes=$((bytes+s)); count=$((count+1)); rm -f "$f"',
    'done',
    'if [ -f /data/adb/ommega/soterta/soterta.err ]; then',
    '  s=$(stat -c %s /data/adb/ommega/soterta/soterta.err 2>/dev/null || echo 0)',
    '  bytes=$((bytes+s)); count=$((count+1)); rm -f /data/adb/ommega/soterta/soterta.err',
    'fi',
    'touch /data/adb/ommega/restart.all',
    'echo "count=$count bytes=$bytes"'].join("\n");
  try {
    const response = await _ (command);
    const output = String((response && (response.stdout || response.stderr)) || "");
    const match = /count=(\d+) bytes=(\d+)/.exec(output);
    if (!match) { show(ommegaDiagT("清除失败：", "Clear failed: ") + output.slice(0, 120)); return; }
    const size = Number(match[2]) >= 1048576
      ? (Number(match[2]) / 1048576).toFixed(1) + " MB"
      : Math.round(Number(match[2]) / 1024) + " KB";
    show(ommegaDiagT("已清除 " + match[1] + " 个日志文件（" + size + "），KeyMint 正在重开新文件。",
      "Cleared " + match[1] + " log files (" + size + "); KeyMint is reopening fresh files."));
  } catch (error) {
    show(ommegaDiagT("清除失败：", "Clear failed: ") + String(error).slice(0, 120));
  }
}

function ommegaDiagInit() {
  const master = document.getElementById("om-diag-master");
  if (master && !master.dataset.bound) {
    master.dataset.bound = "1";
    master.addEventListener("change", () => { void ommegaDiagSetMaster(master.checked); });
  }
  const clear = document.getElementById("om-diag-clear");
  if (clear && !clear.dataset.bound) {
    clear.dataset.bound = "1";
    let armed = false;
    clear.onclick = () => {
      if (!armed) {
        armed = true;
        clear.textContent = ommegaDiagT("确认清除？", "Confirm clear?");
        setTimeout(() => { if (armed) { armed = false; clear.textContent = ommegaDiagT("清除", "Clear"); } }, 5000);
        return;
      }
      armed = false;
      clear.textContent = ommegaDiagT("清除", "Clear");
      void ommegaDiagClearLogs();
    };
  }
  const rotation = document.getElementById("om-diag-rotation");
  if (rotation && !rotation.dataset.bound) {
    rotation.dataset.bound = "1";
    rotation.onchange = async () => {
      const note = document.getElementById("om-diag-rotation-note");
      const megabytes = Number(rotation.value) || 4;
      await ommegaDiagWrite(OMMEGA_DIAG_ROTATION, "max_size_mb=" + megabytes + "\n");
      await _("touch /data/adb/ommega/restart.all");
      if (note) note.textContent = ommegaDiagT(
        "已设为 " + megabytes + " MB，服务正在重载以生效。",
        "Set to " + megabytes + " MB; services are reloading to apply it.");
    };
  }
  const unlock = document.getElementById("om-diag-unlock");
  if (unlock && !unlock.dataset.bound) {
    unlock.dataset.bound = "1";
    unlock.onclick = () => { ommegaDiagUnlocked = true; ommegaDiagStoreUnlock(true); ommegaDiagApplyUnlock(); };
  }
  const host = document.getElementById("om-diag-features");
  if (host) {
    // Bind every tile, including the pages that are created outside the generic
    // descriptor list (request failures, device probe).
    for (const tile of host.querySelectorAll("[data-om-diag]")) {
      if (tile.dataset.bound) continue;
      tile.dataset.bound = "1";
      tile.onclick = () => { location.hash = "om-" + tile.dataset.omDiag; };
    }
  }
  ommegaDiagEnsurePages();
  void ommegaDiagRefreshMaster();
}


function ommegaDiagEnsureErrorsPage() {
  const anchor = document.getElementById("om-settings");
  if (!anchor || document.getElementById("om-diag-errors")) return;
  const section = document.createElement("section");
  section.id = "om-diag-errors";
  section.className = "om-view";
  section.hidden = true;
  section.setAttribute("aria-label", ommegaDiagT("请求异常", "Request failures"));
  section.innerHTML = "<div class=\"om-page-heading\"><h1>" +
    ommegaDiagEsc(ommegaDiagT("请求异常", "Request failures")) + "</h1><p>" +
    ommegaDiagEsc(ommegaDiagT("按应用列出最近的链路级失败（仅应用层，隐藏 keystore/system 等系统身份）",
      "Link-level failures per app (app layer only; system identities such as keystore are hidden)")) + "</p></div>" +
    "<section class=\"panel settings-group\">" +
    "<button type=\"button\" class=\"om-button secondary\" id=\"om-diag-errors-run\">" +
    ommegaDiagEsc(ommegaDiagT("重新扫描", "Scan again")) + "</button>" +
    "<button type=\"button\" class=\"secondary-button\" id=\"om-diag-errors-copy\">" +
    ommegaDiagEsc(ommegaDiagT("复制摘要", "Copy summary")) + "</button>" +
    "<div id=\"om-diag-errors-out\"></div>" +
    "<p class=\"note\" id=\"om-diag-errors-copy-note\" role=\"status\"></p></section>";
  anchor.after(section);
  document.getElementById("om-diag-errors-run").onclick = () =>
    void ommegaDiagRenderErrors(document.getElementById("om-diag-errors-out"));
  document.getElementById("om-diag-errors-copy").onclick = () => {
    const target = document.getElementById("om-diag-errors-out");
    const text = (target && target.innerText ? target.innerText : "").trim();
    const report = document.getElementById("om-diag-errors-copy-note");
    const done = ok => { if (report) report.textContent = ok
      ? ommegaDiagT("已复制到剪贴板。", "Copied to the clipboard.")
      : ommegaDiagT("复制失败，请长按文本手动复制。", "Copy failed; long-press the text instead."); };
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(() => done(true), () => done(false));
        return;
      }
    } catch {}
    try {
      const area = document.createElement("textarea");
      area.value = text; area.style.position = "fixed"; area.style.opacity = "0";
      document.body.appendChild(area); area.select();
      const ok = document.execCommand("copy");
      area.remove(); done(ok);
    } catch { done(false); }
  };
}



// ------------------------------------------------------- native in-page probe
// Runs entirely inside the WebUI through the root bridge: no APK launch, no
// external app required. Each row is a problem, a result and an advice.
async function ommegaDiagNativeProbe() {
  const response = await ommegaDiagCached("errors", ["set +e",
    'M=/data/adb/modules/ommega', 'D=/data/adb/ommega/ommegadata',
    'echo "==module"', 'grep -E "^(version|versionCode)=" "$M/module.prop" 2>/dev/null',
    'echo "==services"', 'service check android.security.keymint 2>/dev/null', 'service check android.security.keystore 2>/dev/null',
    'echo "==keybox"', 'ls -l /data/misc/keystore/ommega/keybox.xml 2>/dev/null | wc -l',
    'grep -c "<Certificate" /data/misc/keystore/ommega/keybox.xml 2>/dev/null',
    'echo "==remote"', 'grep -E "^(url|device_id|remote|tls_insecure):" "$D/config" 2>/dev/null',
    'echo "==health"', 'cat "$D/remote-health.json" 2>/dev/null; echo',
    'echo "==spl"', 'getprop ro.build.version.security_patch', 'getprop ro.vendor.build.security_patch',
    'grep -E "^security_patch|^os_patchlevel|^vendor_patchlevel" /data/adb/ommega/ommegadata/config.toml 2>/dev/null',
    'echo "==mask"', 'cat /data/misc/keystore/ommega/data/cpu-mask.conf 2>/dev/null; echo',
    'echo "==zygisk"', 'ls "$M/zygisk" 2>/dev/null | head -n 3',
    'echo "==soter"', '[ -f /data/adb/ommega/soterta/enabled ] && echo on || echo off',
    'echo "==errors"',
    'grep -a "Error::Km(ErrorCode(-" "$D/logs/keymint.log" 2>/dev/null | sed -n "s/.*for AppUid(\\([0-9]*\\)).*/\\1/p" | awk \'$1>=10000\' | wc -l'].join("\n"));
  const lines = String((response && response.stdout) || "").split("\n");
  const pick = prefix => {
    const start = lines.indexOf("==" + prefix);
    if (start < 0) return [];
    const out = [];
    for (let i = start + 1; i < lines.length && !lines[i].startsWith("=="); i++) if (lines[i].trim()) out.push(lines[i].trim());
    return out;
  };
  const items = [];
  const push = (ok, zh, en, adviceZh, adviceEn) => items.push({ok, zh, en, adviceZh, adviceEn});

  const version = (pick("module")[0] || "unknown").replace("version=", "");
  push(version !== "unknown", "模块版本：" + version, "Module: " + version,
    "模块信息缺失，请重新安装完整模块", "Module metadata is missing; reinstall the full module");

  const services = pick("services");
  const keymintUp = services.some(line => line.includes("keymint") && line.includes("found"));
  push(keymintUp, keymintUp ? "KeyMint 服务在运行" : "KeyMint 服务未响应",
    keymintUp ? "KeyMint service is running" : "KeyMint service is not responding",
    "重启模块或设备后重试", "Restart the module or the device and retry");

  const keyboxCount = Number(pick("keybox")[1] || 0);
  push(keyboxCount > 0, "Keybox 证书数：" + keyboxCount,
    "Keybox certificates: " + keyboxCount,
    "未导入 keybox 或文件损坏；在「自动化 → Keybox 导入」中重新导入",
    "No keybox or a corrupt file; re-import under Automation → Keybox");

  const remote = pick("remote").join(" ");
  const relayOn = /remote: true/.test(remote);
  push(true, relayOn ? "中继模式已启用" : "本地模式", relayOn ? "Relay mode enabled" : "Local mode",
    relayOn ? "如需本地出证，请关闭中继" : "Enable the relay if remote certificate chains are expected",
    relayOn ? "Turn the relay off for purely local certificates" : "Enable the relay if remote chains are expected");

  const health = pick("health").join(" ");
  const verdict = /"last_verdict":"ok"/.test(health) ? "ok" : /"last_verdict":"error"/.test(health) ? "error" : "unknown";
  push(verdict !== "error", "中继最近往返：" + verdict, "Relay last verdict: " + verdict,
    "检查服务器地址、设备 ID 与 token，再确认 B 端在线", "Check the server URL, device id and token, then confirm B is online");

  const spl = pick("spl");
  const splProp = spl.find(line => /^\d{4}-\d{2}-\d{2}$/.test(line)) || "";
  const toml = await ommegaDiagRead(OMMEGA_DIAG_TOML);
  const splConfigured = /^\s*(security_patch|os_patchlevel|vendor_patchlevel)\s*=/m.test(toml);
  // "auto" is a valid trust setting: the module derives the value per request.
  const splKnown = !!splProp || splConfigured;
  push(splKnown, "系统补丁级别：" + (splProp || (splConfigured ? "使用信任参数" : "-")),
    "System patch level: " + (splProp || (splConfigured ? "trust parameters" : "-")),
    "属性为空时，请在「信任参数」中设置补丁级别，否则证明会被拒",
    "When the property is empty, set the patch level under trust parameters or attestation is rejected");

  const mask = pick("mask").join(" ");
  const maskOn = /enabled=true/.test(mask);
  push(true, maskOn ? "设备属性视图已启用" : "设备属性视图未启用",
    maskOn ? "Device property view enabled" : "Device property view disabled",
    maskOn ? "若应用仍读到本机身份，检查 Zygisk 是否注入" : "Enable it for apps that validate device identity",
    maskOn ? "If an app still sees the host identity, check Zygisk injection" : "Enable it for apps that validate device identity");

  // Read the directory directly: a section header can be swallowed when a
  // previous command prints without a trailing newline.
  const zygiskListing = await ommegaReadOnly("ls /data/adb/modules/ommega/zygisk 2>/dev/null | head -n 5");
  const zygisk = String((zygiskListing && zygiskListing.stdout) || "").trim().length > 0;
  push(zygisk, zygisk ? "Zygisk 库存在" : "缺少 Zygisk 库",
    zygisk ? "Zygisk library present" : "Zygisk library missing",
    "设备属性伪装依赖 Zygisk，请启用 Zygisk 后重启", "Property masking needs Zygisk; enable it and reboot");

  const soter = (pick("soter")[0] || "off") === "on";
  push(true, soter ? "SOTER 软件 TA 已接管" : "SOTER 使用原厂 HAL",
    soter ? "SOTER software TA active" : "SOTER uses the stock HAL",
    soter ? "如本地完整性检查失败，可关闭接管" : "Enable takeover if local integrity checks fail",
    soter ? "Disable takeover if local integrity checks fail" : "Enable takeover if local integrity checks fail");

  const errors = Number(pick("errors")[0] || 0);
  push(errors === 0, "近期密钥链路错误：" + errors, "Recent key-link errors: " + errors,
    "打开「请求异常」查看具体应用与原因", "Open Request failures to see the app and reason");

  return items;
}

function ommegaDiagRenderNativeProbe(target, items) {
  const bad = items.filter(item => !item.ok);
  const head = "<p class=\"note\">" + ommegaDiagEsc(ommegaDiagT(
    "通过 " + (items.length - bad.length) + " 项，需要关注 " + bad.length + " 项",
    "Passed " + (items.length - bad.length) + ", " + bad.length + " need attention")) + "</p>";
  const rows = items.map(item => {
    const title = ommegaDiagT(item.zh, item.en);
    const advice = ommegaDiagT(item.adviceZh, item.adviceEn);
    const hint = item.ok ? "" : "<span>" + ommegaDiagEsc(ommegaDiagT("建议：", "Advice: ") + advice) + "</span>";
    return "<li class=\"om-diag-row\"><div class=\"om-diag-row-body\"><strong>" +
      (item.ok ? "\u2713 " : "\u26a0 ") + ommegaDiagEsc(title) + "</strong>" + hint + "</div></li>";
  }).join("");
  target.innerHTML = head + "<ul class=\"om-diag-list\">" + rows + "</ul>";
}

async function ommegaDiagRunNativeProbe() {
  const target = document.getElementById("om-diag-probe-out");
  if (!target) return;
  target.textContent = ommegaDiagT("检查中…", "Checking…");
  ommegaDiagRenderNativeProbe(target, await ommegaDiagNativeProbe());
}


// Optional AProbe app: shipped inside the module, installed on demand, and
// configured for the diagnostics contract (auto auth-signing + property view).
const OMMEGA_DIAG_PROBE_APK = "/data/adb/modules/ommega/probe/a-probe.apk";
const OMMEGA_DIAG_MASK_CONF = "/data/misc/keystore/ommega/data/cpu-mask.conf";
const OMMEGA_DIAG_AUTH_CONF = "/data/adb/ommega/ommegadata/data/auth-signing.toml";
const OMMEGA_DIAG_ROTATION = "/data/adb/ommega/ommegadata/data/log-rotation.conf";
const OMMEGA_DIAG_SOTER_DEBUG = "/data/adb/ommega/soterta/debug";

async function ommegaDiagProbeInstalled() {
  const response = await ommegaReadOnly("pm list packages " + OMMEGA_DIAG_PROBE_PKG + " 2>/dev/null");
  return String((response && response.stdout) || "").indexOf("package:" + OMMEGA_DIAG_PROBE_PKG) >= 0;
}

async function ommegaDiagConfigureProbePolicy() {
  const mask = await ommegaDiagRead(OMMEGA_DIAG_MASK_CONF);
  let nextMask = (mask || "version=1\nenabled=false\n");
  if (!/^version=/m.test(nextMask)) nextMask = "version=1\n" + nextMask;
  nextMask = /^enabled=/m.test(nextMask)
    ? nextMask.replace(/^enabled=.*/m, "enabled=true")
    : nextMask + "enabled=true\n";
  if (!/^package=aprobes\.agap$/m.test(nextMask)) nextMask += "package=" + OMMEGA_DIAG_PROBE_PKG + "\n";
  await ommegaDiagWrite(OMMEGA_DIAG_MASK_CONF, nextMask);

  const auth = await ommegaDiagRead(OMMEGA_DIAG_AUTH_CONF);
  let nextAuth = (auth && auth.trim())
    ? auth
    : "# Ommega authentication-signing policy. New keys only.\nversion = 1\ndefault = \"auto\"\n\n[apps]\n";
  if (!/^version\s*=/m.test(nextAuth)) nextAuth = "version = 1\n" + nextAuth;
  if (!/^default\s*=/m.test(nextAuth)) nextAuth = nextAuth.replace(/^version[^\n]*\n/, "function ommegaDiagEnsureProbePage() {default = \"auto\"\n");
  if (!/^\[apps\]/m.test(nextAuth)) nextAuth += "\n[apps]\n";
  const entry = /^"aprobes\.agap"\s*=\s*"[a-z]+"/m;
  if (entry.test(nextAuth)) nextAuth = nextAuth.replace(entry, '"aprobes.agap" = "auto"');
  else nextAuth = nextAuth.replace(/^\[apps\]\s*$/m, '[apps]\n"aprobes.agap" = "auto"');
  await ommegaDiagWrite(OMMEGA_DIAG_AUTH_CONF, nextAuth);
}

async function ommegaDiagInstallProbe() {
  const target = document.getElementById("om-diag-probe-out");
  const show = text => { if (target) target.textContent = text; };
  show(ommegaDiagT("安装中…", "Installing…"));
  try {
    const present = await ommegaReadOnly("test -f '" + OMMEGA_DIAG_PROBE_APK + "' && echo yes || echo no");
    if (String((present && present.stdout) || "").indexOf("yes") < 0) {
      show(ommegaDiagT("模块内没有探针安装包（probe/a-probe.apk），请安装完整模块。",
        "The module does not contain probe/a-probe.apk; install the full module."));
      return;
    }
    let result = await _("pm install -r '" + OMMEGA_DIAG_PROBE_APK + "' 2>&1");
    let output = String((result && (result.stdout || result.stderr)) || "");
    if (/INCONSISTENT_CERTIFICATES|UPDATE_INCOMPATIBLE/i.test(output)) {
      // A probe signed with another key (for example the previous debug build)
      // cannot be upgraded in place; its data is only a result file.
      await _("pm uninstall " + OMMEGA_DIAG_PROBE_PKG);
      result = await _("pm install '" + OMMEGA_DIAG_PROBE_APK + "' 2>&1");
      output = String((result && (result.stdout || result.stderr)) || "");
    }
    await ommegaDiagConfigureProbePolicy();
    await ommegaDiagUpdateProbeButtons();
    if (await ommegaDiagProbeInstalled()) {
      show(ommegaDiagT("AProbe 已安装，并已加入「CPU 与设备属性」与自动授权策略。",
        "AProbe installed and added to CPU/device properties with the automatic policy."));
    } else {
      show(ommegaDiagT("安装失败：", "Install failed: ") + output.replace(/\s+/g, " ").slice(0, 160));
    }
  } catch (error) {
    show(ommegaDiagT("安装失败：", "Install failed: ") + String(error).slice(0, 160));
  }
}

async function ommegaDiagLaunchProbe() {
  await _("am force-stop " + OMMEGA_DIAG_PROBE_PKG);
  await _("am start -n " + OMMEGA_DIAG_PROBE_ACTIVITY);
}

async function ommegaDiagUpdateProbeButtons() {
  const installed = await ommegaDiagProbeInstalled();
  const install = document.getElementById("om-diag-probe-install");
  const launch = document.getElementById("om-diag-probe-launch");
  const read = document.getElementById("om-diag-probe-read");
  if (install) install.hidden = installed;
  if (launch) launch.hidden = !installed;
  if (read) read.hidden = !installed;
}

function ommegaDiagEnsureProbePage() {
  const anchor = document.getElementById("om-settings");
  if (!anchor || document.getElementById("om-diag-probe")) return;
  const section = document.createElement("section");
  section.id = "om-diag-probe";
  section.className = "om-view";
  section.hidden = true;
  section.setAttribute("aria-label", ommegaDiagT("设备体检", "Device probe"));
  section.innerHTML = "<div class=\"om-page-heading\"><h1>" +
    ommegaDiagEsc(ommegaDiagT("设备体检", "Device probe")) + "</h1><p>" +
    ommegaDiagEsc(ommegaDiagT("在 WebUI 内直接检查，不跳转应用；每条给出问题、结果与建议",
      "Checks run inside the WebUI without launching an app; each row shows problem, result and advice")) + "</p></div>" +
    "<section class=\"panel settings-group\">" +
    "<div class=\"om-diag-actions\">" +
    "<button type=\"button\" class=\"primary-button\" id=\"om-diag-probe-run\">" +
    ommegaDiagEsc(ommegaDiagT("重新检查", "Run checks")) + "</button>" +
    "<button type=\"button\" class=\"secondary-button\" id=\"om-diag-probe-install\">" +
    ommegaDiagEsc(ommegaDiagT("安装 AProbe（可选）", "Install AProbe (optional)")) + "</button>" +
    "<button type=\"button\" class=\"secondary-button\" id=\"om-diag-probe-launch\" hidden>" +
    ommegaDiagEsc(ommegaDiagT("执行 AProbe 检测", "Run AProbe checks")) + "</button>" +
    "<button type=\"button\" class=\"secondary-button\" id=\"om-diag-probe-read\" hidden>" +
    ommegaDiagEsc(ommegaDiagT("读取 AProbe 结果", "Read AProbe result")) + "</button>" +
    "</div><div id=\"om-diag-probe-out\"></div></section>";
  anchor.after(section);
  document.getElementById("om-diag-probe-run").onclick = () => void ommegaDiagRunNativeProbe();
  document.getElementById("om-diag-probe-install").onclick = () => void ommegaDiagInstallProbe();
  document.getElementById("om-diag-probe-launch").onclick = () => void ommegaDiagLaunchProbe();
  document.getElementById("om-diag-probe-read").onclick = () =>
    void ommegaDiagRenderProbe(document.getElementById("om-diag-probe-out"), {});
  void ommegaDiagUpdateProbeButtons();
}

function ommegaDiagRouteAllowed(view, unlocked) {
  return !String(view).startsWith("diag-") || !!unlocked;
}

function ommegaDiagRoute(view) {
  if (view === "overview") { void ommegaDiagRenderOverview(); return; }
  if (view === "diagnostics") { ommegaDiagInit(); return; }
  if (view === "diag-probe") {
    ommegaDiagInit();
    ommegaDiagEnsureProbePage();
    void ommegaDiagRunNativeProbe();
    void ommegaDiagUpdateProbeButtons();
    return;
  }
  if (view === "diag-errors") {
    ommegaDiagInit();
    ommegaDiagEnsureErrorsPage();
    void ommegaDiagRenderErrors(document.getElementById("om-diag-errors-out"));
    return;
  }
  if (!view.startsWith("diag-")) return;
  // The tiles are hidden until the user reproduces the issue; a hand-typed hash
  // must not bypass that gate.
  if (!ommegaDiagRouteAllowed(view, ommegaDiagStoredUnlock())) {
    location.replace("#om-diagnostics");
    return;
  }
  const page = OMMEGA_DIAG_PAGES.find(item => item.id === view);
  if (!page) return;
  ommegaDiagInit();
  void ommegaDiagLoad(page);
}
// The glue is also loaded by headless tests without a DOM.
if (typeof window !== "undefined" && typeof location !== "undefined") {
  window.addEventListener("hashchange", () => {
    const hash = location.hash || "";
    ommegaDiagRoute(hash.startsWith("#om-") ? hash.slice(4) : "");
  });
}
if (typeof document !== "undefined" && typeof location !== "undefined") {
  if (document.readyState !== "loading") ommegaDiagRoute((location.hash || "").replace("#om-", ""));
  else document.addEventListener("DOMContentLoaded", () => ommegaDiagRoute((location.hash || "").replace("#om-", "")));
}

if (typeof window !== "undefined") window.ommegaDiagRoute = ommegaDiagRoute;

// ------------------------------------------------- request error report
// Mirrors what a user sees as "it stopped working": link-level KeyMint
// failures such as -26/-33/-66, attributed to a package and explained in
// plain words instead of a numeric code.
const OMMEGA_DIAG_REASONS = {
  "-1":   ["未知错误", "Unknown error"],
  "-26":  ["用户未认证：指纹/密码没有绑定到这次操作", "Not authenticated: biometric did not authorize this operation"],
  "-28":  ["操作句柄已失效", "Operation handle is no longer valid"],
  "-30":  ["校验失败：签名或证书内容不一致", "Verification failed: signature or certificate mismatch"],
  "-33":  ["密钥数据无效：密钥被重置或不再属于该设备", "Invalid key blob: key was reset or does not belong to this device"],
  "-38":  ["参数无效", "Invalid argument"],
  "-66":  ["无法证明设备 ID：请求的身份与中继设备不匹配", "Cannot attest device ids: requested identity does not match the relay device"],
  "-68":  ["该硬件类型不可用（例如 StrongBox 被屏蔽）", "Hardware type unavailable (for example StrongBox is masked)"]
};

function ommegaDiagReason(code) {
  const entry = OMMEGA_DIAG_REASONS[String(code)];
  return entry ? ommegaDiagT(entry[0], entry[1]) : ommegaDiagT("错误码 " + code, "Error code " + code);
}

let ommegaDiagAppInfoCache = {at: 0, map: new Map()};

function ommegaDiagAppInfo() {
  if (Date.now() - ommegaDiagAppInfoCache.at < 10000) return ommegaDiagAppInfoCache.map;
  const info = new Map();
  for (const card of document.querySelectorAll("#apps-list .card[data-package]")) {
    const pkg = card.getAttribute("data-package");
    if (!pkg) continue;
    const nameNode = card.querySelector(".app-name");
    let label = "";
    if (nameNode) {
      const clone = nameNode.cloneNode(true);
      clone.querySelectorAll(".om-app-badges, .om-cert-count, .om-soter-count").forEach(node => node.remove());
      label = (clone.textContent || "").trim();
    }
    // The app list loads icons lazily, so reuse the KernelSU icon scheme instead
    // of whatever src the card happens to carry at this moment.
    const icon = card.querySelector("img.app-icon")?.getAttribute("src") || (pkg ? "ksu://icon/" + pkg : "");
    info.set(pkg, {label, icon});
  }
  return info;
}

let ommegaDiagHiddenSystem = 0;
const ommegaDiagCache = new Map();
const OMMEGA_DIAG_TTL = 8000;

// Shell reads are expensive (multi-MB logs, package manager). Serve the last
// answer instantly and refresh only when it is stale.
async function ommegaDiagCached(key, command, ttl = OMMEGA_DIAG_TTL) {
  const hit = ommegaDiagCache.get(key);
  if (hit && Date.now() - hit.at < ttl) return hit.value;
  const value = await ommegaReadOnly(command);
  ommegaDiagCache.set(key, {at: Date.now(), value});
  return value;
}

const OMMEGA_DIAG_UID_KEY = "ommega.diag.uidmap";
let ommegaDiagUidMap = (() => {
  try {
    const raw = localStorage.getItem(OMMEGA_DIAG_UID_KEY);
    if (raw) {
      const saved = JSON.parse(raw);
      if (saved && Array.isArray(saved.entries) && Date.now() - Number(saved.at || 0) < 86400000) {
        return {at: Number(saved.at), map: new Map(saved.entries)};
      }
    }
  } catch {}
  return {at: 0, map: new Map()};
})();

function ommegaDiagPersistUidMap() {
  try {
    localStorage.setItem(OMMEGA_DIAG_UID_KEY, JSON.stringify({
      at: ommegaDiagUidMap.at, entries: [...ommegaDiagUidMap.map.entries()]
    }));
  } catch {}
}

async function ommegaDiagUidPackages(uids) {
  const missing = uids.filter(uid => !ommegaDiagUidMap.map.has(uid));
  if (Date.now() - ommegaDiagUidMap.at > 300000) {
    ommegaDiagUidMap = {at: 0, map: new Map()};
    missing.length = 0;
    for (const uid of uids) missing.push(uid);
  }
  if (missing.length) {
    const filter = missing.join("|");
    const pattern = "uid:(" + filter + ")" + String.fromCharCode(36);
    const response = await ommegaReadOnly("pm list packages -U 2>/dev/null | grep -E '" + pattern + "'");
    for (const line of String((response && response.stdout) || "").split("\n")) {
      const match = /package:(\S+)\s+uid:(\d+)/.exec(line);
      if (match) ommegaDiagUidMap.map.set(match[2], match[1]);
    }
    ommegaDiagUidMap.at = Date.now();
    for (const uid of missing) if (!ommegaDiagUidMap.map.has(uid)) ommegaDiagUidMap.map.set(uid, "");
    ommegaDiagPersistUidMap();
  }
  return ommegaDiagUidMap.map;
}

async function ommegaDiagCollectErrors() {
  const response = await ommegaReadOnly(["set +e",
    'L=/data/adb/ommega/ommegadata/logs',
    'echo "==keymint"',
    'for f in "$L"/keymint.log "$L"/keymint.log.*; do [ -f "$f" ] && tail -c 131072 "$f"; done | grep -a -E "Error::Km\\(ErrorCode\\(-[0-9]+\\)\\) for AppUid\\([0-9]+\\)" | tail -n 80',
    'echo "==injector"',
    'for f in "$L"/injector.log "$L"/injector.log.*; do [ -f "$f" ] && tail -c 131072 "$f"; done | grep -a -E "failed for uid=[0-9]+" | tail -n 80'].join("\n"));
  const text = String((response && response.stdout) || "");
  const lines = text.split("\n");
  const entries = [];
  const uids = new Set();
  ommegaDiagHiddenSystem = 0;
  let section = "";
  let stamp = "";
  for (const line of lines) {
    if (line.startsWith("==")) { section = line.slice(2); continue; }
    const time = /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})/.exec(line);
    if (time) stamp = time[1];
    if (section === "keymint") {
      const match = /Error::Km\(ErrorCode\((-?\d+)\)\) for AppUid\((\d+)\)/.exec(line);
      if (match) {
        if (!ommegaDiagIsAppUid(match[2])) { ommegaDiagHiddenSystem++; continue; }
        entries.push({code: match[1], uid: match[2], ts: stamp, source: "keymint"}); uids.add(match[2]);
      }
    } else if (section === "injector") {
      const code = /ServiceSpecific\((-?\d+)\)/.exec(line);
      const uid = /uid=(\d+)/.exec(line);
      if (code && uid && Number(code[1]) < 0) {
        if (!ommegaDiagIsAppUid(uid[1])) { ommegaDiagHiddenSystem++; continue; }
        entries.push({code: code[1], uid: uid[1], ts: stamp, source: "injector"}); uids.add(uid[1]);
      }
    }
  }
  if (!uids.size) return [];
  const byUid = await ommegaDiagUidPackages([...uids]);
  const appInfo = ommegaDiagAppInfo();
  return ommegaDiagAggregateErrors(entries).slice(0, 12).map(entry => {
    const pkg = byUid.get(entry.uid) || "";
    // A package outside the enumerated app list still has a resolvable icon.
    const meta = appInfo.get(pkg) || {label: "", icon: pkg ? "ksu://icon/" + pkg : ""};
    return {
      code: entry.code, uid: entry.uid, ts: entry.ts, count: entry.count,
      pkg, label: meta.label, icon: meta.icon
    };
  });
}

function ommegaDiagErrorRows(list) {
  return list.map(item => {
    const system = Number(item.uid) < 10000;
    const title = item.label || item.pkg || (system ? ommegaDiagT("系统进程", "System process") : ommegaDiagT("未知应用", "Unknown app"));
    const tail = item.pkg && item.pkg !== title ? " · " + item.pkg : (item.pkg ? "" : " · uid " + item.uid);
    const icon = item.icon
      ? "<img class=\"om-diag-icon\" src=\"" + ommegaDiagEsc(item.icon) + "\" alt=\"\">"
      : "";
    const meta = [];
    if (item.count > 1) meta.push(ommegaDiagT("发生 " + item.count + " 次", "seen " + item.count + " times"));
    if (item.ts) meta.push(ommegaDiagT("最后 " + item.ts.slice(5), "last " + item.ts.slice(5)));
    const when = meta.length ? "<span class=\"om-diag-meta\">" + ommegaDiagEsc(meta.join(" · ")) + "</span>" : "";
    return "<li class=\"om-diag-row\">" + icon + "<div class=\"om-diag-row-body\"><strong>" +
      ommegaDiagEsc(ommegaDiagReason(item.code)) + "</strong><span>" +
      ommegaDiagEsc(title + tail) + "</span>" + when + "</div></li>";
  }).join("");
}

async function ommegaDiagRenderErrors(target) {
  if (!target) return;
  target.textContent = ommegaDiagT("读取中…", "Reading…");
  const list = await ommegaDiagCollectErrors();
  const hidden = ommegaDiagHiddenSystem > 0
    ? "<p class=\"note\">" + ommegaDiagEsc(ommegaDiagT(
        "已隐藏 " + ommegaDiagHiddenSystem + " 条系统身份（keystore/system 等）记录，只显示应用层异常。",
        "Hid " + ommegaDiagHiddenSystem + " system-identity records (keystore/system); only app-level failures are shown.")) + "</p>"
    : "";
  target.innerHTML = (list.length
    ? "<ul class=\"om-diag-list\">" + ommegaDiagErrorRows(list) + "</ul>"
    : "<p class=\"note\">" + ommegaDiagEsc(ommegaDiagT("最近没有链路级错误。", "No link-level errors recorded recently.")) + "</p>") + hidden;
}

async function ommegaDiagRenderOverview() {
  const host = document.getElementById("om-overview");
  if (!host) return;
  // The overview lays its panels out in a grid; joining that grid gives the card
  // the same gap as the others instead of sitting flush against it.
  const grid = host.querySelector(".dashboard-grid") || host;
  let card = document.getElementById("om-overview-errors");
  if (!card) {
    card = document.createElement("section");
    card.id = "om-overview-errors";
    card.className = "panel settings-group om-diag-overview-card";
    card.innerHTML = "<div class=\"group-header\"><h2 class=\"group-heading\">" +
      ommegaDiagEsc(ommegaDiagT("最近请求异常", "Recent request failures")) + "</h2></div>" +
      "<div id=\"om-overview-errors-body\"></div>";
  }
  if (card.parentElement !== grid) grid.appendChild(card);
  await ommegaDiagRenderErrors(document.getElementById("om-overview-errors-body"));
}

// ------------------------------------------------------- device probe (AProbe)
const OMMEGA_DIAG_PROBE_ACTIVITY = 'aprobes.agap/aprobes.MainActivity';
const OMMEGA_DIAG_PROBE_HINTS = {
  A07: [['设备属性证明不可用','Device property attestation unavailable'],['把该应用加入「CPU 与设备属性」后重试，或关闭中继再测','Add this app to CPU/device properties and retry, or test without the relay']],
  A29: [['认证绑定参数没有写进证书','Authentication parameters missing from the certificate'],['确认使用最新模块；该问题影响银行类应用的证书校验','Install the latest module build; banks validate this certificate']],
  A30: [['同一参数下不同别名的证书策略不一致','Certificate policy differs between aliases'],['属于实现差异，请把结果反馈给开发者','Report this to the maintainer']],
  A31: [['KeyInfo 与证书的认证标志不一致','KeyInfo and certificate disagree on authentication'],['安装最新模块后重新检测','Re-run after installing the latest build']],
  A34: [['认证有效期与证书不一致','Authentication timeout differs from the certificate'],['安装最新模块后重新检测','Re-run after installing the latest build']],
  T04: [['中继模式存在明显时延差','Relay mode shows a measurable latency gap'],['属于中继固有特征；如应用超时可改用本地模式','Inherent to relaying; use local mode if apps time out']],
  E01: [['短摘要无法签名','Short raw digests cannot be signed'],['升级模块（2.3.1+ 已修复）','Upgrade the module (fixed in 2.3.1+)']],
  E02: [['P-521 摘要截断不正确','P-521 digest truncation is wrong'],['升级模块（2.3.1+ 已修复）','Upgrade the module (fixed in 2.3.1+)']],
  E03: [['P-384 不支持 SHA-1','P-384 rejects SHA-1'],['升级模块（2.3.1+ 已修复）','Upgrade the module (fixed in 2.3.1+)']],
  E04: [['签名可重复，行为与原生不同','Signatures repeat, unlike a stock device'],['升级模块（2.3.1+ 已修复）','Upgrade the module (fixed in 2.3.1+)']],
  PT01: [['StrongBox 声明与实际不一致','StrongBox is declared but unusable'],['开启「屏蔽 StrongBox」后重启，或关闭原生 StrongBox','Enable the StrongBox mask and reboot, or keep native StrongBox off']],
  PT02: [['无法按 NPCI 参数创建指纹密钥','Cannot create the NPCI fingerprint key'],['查看下方原始日志中的异常类型','Check the exception type in the raw log below']],
  PT03: [['密钥不在安全硬件中','The key is not inside secure hardware'],['开启「CPU 与设备属性」或检查中继模式','Enable CPU/device properties or check relay mode']],
  PT04: [['证书没有如实描述认证要求','The certificate misstates the authentication policy'],['升级模块后重新检测','Re-run after upgrading the module']],
  PT05: [['签名对象无法初始化','The signature object cannot be initialized'],['检查钥匙串服务是否被中断','Check whether the keystore service is interrupted']],
  PT06: [['指纹认证后签名失败','Signing fails after biometric authentication'],['把原始日志发给开发者定位','Send the raw log to the maintainer']]
};

function ommegaDiagProbeHint(id, fallbackZh, fallbackEn) {
  const entry = OMMEGA_DIAG_PROBE_HINTS[id];
  if (!entry) return [ommegaDiagT('需要关注：' + fallbackZh, 'Needs attention: ' + fallbackEn),
    ommegaDiagT('把这条结果发给开发者', 'Send this result to the maintainer')];
  return [ommegaDiagT(entry[0][0], entry[0][1]), ommegaDiagT(entry[1][0], entry[1][1])];
}

function ommegaDiagParseProbe(text) {
  const rows = [];
  for (const line of String(text || '').split('\n')) {
    if (!line.startsWith('RESULT|')) continue;
    const parts = line.split('|');
    if (parts.length < 5) continue;
    rows.push({ id: parts[1], status: parts[2], title: parts[3], detail: parts[4] });
  }
  return rows;
}

async function ommegaDiagReadProbe() {
  const response = await ommegaReadOnly("cat '" + OMMEGA_DIAG_PROBE_FILE + "' 2>/dev/null");
  return String((response && response.stdout) || '');
}

async function ommegaDiagRenderProbe(target, options) {
  if (!target) return;
  target.textContent = ommegaDiagT('读取中…', 'Reading…');
  let text = await ommegaDiagReadProbe();
  if (options && options.run) {
    await _('am force-stop ' + OMMEGA_DIAG_PROBE_PKG);
    await _('am start -n ' + OMMEGA_DIAG_PROBE_ACTIVITY);
    for (let i = 0; i < 40; i++) {
      await new Promise(r => setTimeout(r, 2000));
      text = await ommegaDiagReadProbe();
      if (text.indexOf('RESULT|') >= 0) break;
    }
  }
  const rows = ommegaDiagParseProbe(text);
  if (!rows.length) {
    target.innerHTML = '<p class="note">' + ommegaDiagEsc(ommegaDiagT('还没有体检结果。点击“运行体检”开始。', 'No probe result yet. Tap “Run probe” to start.')) + '</p>';
    return;
  }
  const issues = rows.filter(row => row.status === 'HIT' || row.status === 'ERROR');
  const head = '<p class="note">' + ommegaDiagEsc(ommegaDiagT('通过 ' + (rows.length - issues.length) + ' 项，需要关注 ' + issues.length + ' 项', 'Passed ' + (rows.length - issues.length) + ', ' + issues.length + ' need attention')) + '</p>';
  const cards = issues.map(row => {
    const hint = ommegaDiagProbeHint(row.id, row.title, row.title);
    return '<li><strong>' + ommegaDiagEsc(row.id + ' · ' + hint[0]) + '</strong><br>' +
      ommegaDiagEsc(ommegaDiagT('结果：', 'Result: ') + row.title + ' — ' + String(row.detail || '').slice(0, 120)) + '<br>' +
      ommegaDiagEsc(ommegaDiagT('建议：', 'Advice: ') + hint[1]) + '</li>';
  }).join('');
  target.innerHTML = head + (issues.length ? '<ul class="om-diag-summary">' + cards + '</ul>'
    : '<p class="note">' + ommegaDiagEsc(ommegaDiagT('未发现需要关注的项目。', 'Nothing needs attention.')) + '</p>');
}

// Android app uids start at 10000; everything below is a system identity such
// as keystore (1017) or system (1000) and is not actionable for the user.
function ommegaDiagIsAppUid(uid) { return Number(uid) >= 10000; }

// One row per app+code, carrying how often it happened and when it last did.
function ommegaDiagAggregateErrors(entries) {
  const map = new Map();
  for (const entry of entries) {
    const key = entry.code + "|" + entry.uid;
    const current = map.get(key);
    if (current) {
      current.count += 1;
      if (entry.ts) current.ts = entry.ts;
    } else {
      map.set(key, {code: entry.code, uid: entry.uid, ts: entry.ts || "", count: 1});
    }
  }
  return [...map.values()].sort((left, right) => String(right.ts).localeCompare(String(left.ts)));
}

