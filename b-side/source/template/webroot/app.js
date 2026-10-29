import {KEYS, LEVELS, parseConfig, validateConfig, serializeConfig, readCommand, parseSnapshot, saveCommand} from './relay-config.js?v=material-ui-6';
import {applyAppearance, appearanceControls, bindAppearance, icon, installDialogGuard, parseProperties, pollWhenVisible, setText, text} from './ui-kit.js?v=material-ui-6';
import {t, translatePage} from './i18n.js?v=material-ui-6';

applyAppearance();
translatePage();
document.querySelectorAll('[data-icon]').forEach(el => { el.innerHTML = icon(el.dataset.icon); });
document.getElementById('appearance').innerHTML = appearanceControls();
const themeToggle = document.getElementById('theme-toggle');
const themePreference = () => { try { return localStorage.getItem('ommega.ui.theme') || 'auto'; } catch { return 'auto'; } };
const effectiveTheme = () => themePreference() === 'auto'
  ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : themePreference();
const paintTheme = () => {
  const dark = effectiveTheme() === 'dark';
  themeToggle.innerHTML = icon(dark ? 'sun' : 'moon');
  const label = dark ? text('切换浅色主题','Switch to light theme') : text('切换深色主题','Switch to dark theme');
  themeToggle.setAttribute('aria-label', label); themeToggle.title = label;
};
themeToggle.onclick = () => {
  try { localStorage.setItem('ommega.ui.theme', effectiveTheme() === 'dark' ? 'light' : 'dark'); } catch { /* ephemeral WebView */ }
  applyAppearance(); paintTheme();
};
matchMedia('(prefers-color-scheme: dark)').addEventListener?.('change', paintTheme);
paintTheme();


const $ = id => document.getElementById(id);
let seq = 0;
function exec(command, {read = false} = {}) {
  return new Promise(resolve => {
    const callback = `ommega_b_${Date.now()}_${seq++}`;
    let settled = false, timer;
    const finish = result => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      delete window[callback];
      resolve(result);
    };
    window[callback] = (errno, stdout, stderr) => finish({errno:Number(errno), stdout:stdout || '', stderr:stderr || ''});
    // Only stop waiting for READ callbacks. Never time out, cancel or retry a
    // save/apply operation: it may still be running on the device.
    if (read) timer = setTimeout(() => finish({errno:1, stdout:'', stderr:t('读取超时，请刷新重试。'), timedOut:true}), 12000);
    try { window.ksu.exec(command, '{}', callback); }
    catch { finish({errno:1, stdout:'', stderr:text('请从支持 WebUI 的模块管理器打开此页面。','Open from a module manager with WebUI support.')}); }
  });
}
// All refreshes and mutations share one queue. A click during a read waits for
// its own follow-up transaction, and an older response cannot undo a newer save.
let actions = Promise.resolve();
function queueAction(action) {
  const task = actions.then(action);
  actions = task.catch(() => {});
  return task;
}
const moduleDir = '/data/adb/modules/ommegaclient_b';
const relayIds = ['relay-server', 'relay-device', 'relay-machine', 'relay-token', 'file-log-enabled', 'file-log-level', 'logcat-enabled', 'logcat-level'];
const logKeys = KEYS.slice(4);
let snapshot = null;
let savedValues = null;
let summaryValues = null;
let relaySaving = false;

for (const id of ['file-log-level', 'logcat-level']) {
  for (const level of LEVELS) $(id).add(new Option(level, level));
}
function relayMessage(text, error = false) {
  $('relay-message').textContent = t(text);
  $('relay-message').className = error ? 'om-message error' : 'om-message';
}
function relayBusy(busy) {
  $('relay-fields').disabled = busy || !snapshot;
  $('relay-save').setAttribute('aria-busy', String(busy));
  $('relay-reload').disabled = busy;
}
function fillRelay(values) {
  relayIds.forEach((id, i) => {
    if ($(id).type === 'checkbox') $(id).checked = values[KEYS[i]] === 'true';
    else $(id).value = values[KEYS[i]];
  });
  $('relay-token').type = 'password';
  $('token-toggle').textContent = t('显示');
  $('token-toggle').setAttribute('aria-pressed', 'false');
}
function relayValues() {
  return Object.fromEntries(relayIds.map((id, i) => [KEYS[i], $(id).type === 'checkbox' ? String($(id).checked) : $(id).value]));
}
const initialRelayValues = relayValues();
function relayDirty() {
  const values = relayValues(), baseline = savedValues || initialRelayValues;
  return KEYS.some(key => values[key] !== baseline[key]);
}
async function readRelay({replaceDraft = false, announce = false} = {}) {
  if (announce || !snapshot) relayMessage('正在读取配置…');
  try {
    const result = await exec(readCommand(), {read:true});
    if (result.errno !== 0) throw new Error(result.stderr || '读取失败，请检查管理器的 root 权限。');
    const loaded = parseSnapshot(result.stdout);
    summaryValues = parseConfig(loaded.raw);
    // An external config change must not advance the editor's CAS digest while
    // its draft is dirty. Saving that old draft must still detect the conflict.
    const preserve = relayDirty() && replaceDraft !== JSON.stringify(relayValues());
    if (!preserve || !savedValues) {
      snapshot = loaded;
      savedValues = summaryValues;
      if (!preserve) fillRelay(savedValues);
    }
    updateSummary();
    if (!relaySaving) relayMessage(preserve ? '已刷新保存的配置，未保存的修改已保留。'
      : loaded.digest === 'missing' ? '尚无配置，请填写连接参数后保存。' : '已读取保存的配置。');
    return {ok:true};
  } catch (error) {
    summaryValues = null;
    updateSummary();
    relayMessage(error.message, true);
    return {ok:false, error:t(error.message)};
  } finally { if (!relaySaving) relayBusy(false); }
}
$('relay-form').onsubmit = async event => {
  event.preventDefault();
  if (!snapshot || $('relay-fields').disabled || relaySaving) return;
  let values;
  try { values = validateConfig(relayValues()); }
  catch (error) { relayMessage(error.message, true); return; }
  const connectionChanged = KEYS.slice(0, 4).some(key => values[key] !== savedValues[key]);
  const logsChanged = logKeys.some(key => values[key] !== savedValues[key]);
  if (!connectionChanged && !logsChanged && snapshot.digest !== 'missing') { relayMessage('配置未改变，无需保存。'); return; }
  const raw = serializeConfig(snapshot.raw, values), expectedDigest = snapshot.digest;
  relaySaving = true;
  relayBusy(true);
  relayMessage('正在保存…');
  return queueAction(async () => {
    try {
      const result = await exec(saveCommand(raw, expectedDigest, connectionChanged));
      if (result.errno !== 0) {
        if ((result.stdout + result.stderr).includes('CONFIG_CHANGED')) {
          snapshot = null;
          throw new Error('配置已被其他操作修改，请重新读取后再保存。');
        }
        throw new Error('保存失败，请重新读取配置后检查。');
      }
      // Only this verification read is bounded; the write callback above is not.
      const loaded = await exec(readCommand(), {read:true});
      if (loaded.errno !== 0) { snapshot = null; throw new Error('配置已写入，但复读失败，请重新读取确认。'); }
      snapshot = parseSnapshot(loaded.stdout);
      if (snapshot.raw !== raw) { snapshot = null; throw new Error('保存后配置发生变化，请重新读取确认。'); }
      savedValues = summaryValues = values;
      fillRelay(values);
      updateSummary();
      await refreshServerStatus(true);
      let notice = t('配置已保存。');
      if (connectionChanged) notice += t('连接参数将由运行中的 relay 自动加载。');
      if (logsChanged) notice += t('日志选项在 relay 下次启动时生效。');
      if (result.stdout.includes('SAVED_RELOAD_MARKER_FAILED')) notice += t('加载通知未写入，将由文件监测触发加载。');
      relayMessage(notice);
    } catch (error) { relayMessage(error.message, true); }
    finally { relaySaving = false; relayBusy(false); }
  });
};
$('relay-reload').onclick = () => {
  if (relayDirty() && !window.confirm(t('重新读取会丢弃未保存的连接修改，继续？'))) return;
  return requestRefresh({replaceRelay:JSON.stringify(relayValues())});
};
$('token-toggle').onclick = () => {
  const visible = $('relay-token').type === 'password';
  $('relay-token').type = visible ? 'text' : 'password';
  $('token-toggle').textContent = t(visible ? '隐藏' : '显示');
  $('token-toggle').setAttribute('aria-pressed', String(visible));
};

const fields = ['system', 'boot', 'vendor'];
// Field id -> the key spl-control.sh reports in `status` and expects in `save`.
// Order matters: it is both the save argument order and the status order.
const propertyFields = [
  ...fields.map(name => ({id: name, status: `${name.toUpperCase()}_SPL`, label: name[0].toUpperCase() + name.slice(1)})),
  {id: 'osversion', status: 'OS_VERSION', label: 'OS'},
];
let savedSplValues = null;
let splReadable = false, splSaving = false, splReadFailed = false;
const message = $('message');
const save = $('save');
const valid = value => value === '' || /^\d{4}-(0[1-9]|1[0-2])-([0-2]\d|3[01])$/.test(value);
// Android major version, matching valid_os_version in spl-control.sh.
const validOsVersion = value => value === '' || /^\d{1,2}$/.test(value);
const splValues = () => propertyFields.map(field => $(field.id).value);
const initialSplValues = splValues();
const splDirty = () => splValues().some((value, index) => value !== (savedSplValues || initialSplValues)[index]);
function splBusy(busy) {
  save.disabled = busy || !splReadable;
  save.setAttribute('aria-busy', String(busy));
  for (const id of [...propertyFields.map(field => field.id), 'same', 'auto', 'spl-reload']) $(id).disabled = busy;
}
async function refreshSpl({replaceDraft = false, announce = false} = {}) {
  if (announce && !splSaving) { message.className = 'om-message'; message.textContent = t('正在读取设备属性…'); }
  try {
    const result = await exec(`sh "${moduleDir}/spl-control.sh" status --readonly`, {read:true});
    if (result.errno !== 0) throw new Error(result.stderr || result.stdout || '状态不可用');
    const values = parseProperties(result.stdout);
    if (!propertyFields.every(field => Object.hasOwn(values, field.status))) throw new Error('设备属性响应无效，请刷新重试。');
    $('status').textContent = result.stdout;
    const preserve = splDirty() && replaceDraft !== JSON.stringify(splValues());
    if (!preserve) propertyFields.forEach(field => $(field.id).value = values[field.status]);
    if (!preserve || !savedSplValues) savedSplValues = propertyFields.map(field => values[field.status]);
    // Original values and ratcheted minimums are separate facts.
    const original = key => values['ORIGIN_' + key] || t('未知');
    $('origin').textContent = t('设备原始值：System ') + original('SYSTEM_SPL')
      + ' · Vendor ' + original('VENDOR_SPL') + ' · OS ' + original('OS_VERSION');
    const summary = prefix => propertyFields.filter(field => prefix !== 'FLOOR_' || field.id !== 'boot')
      .map(field => `${field.label} ${values[prefix + field.status] || '—'}`).join(' · ');
    $('spl-floor').textContent = t('下限：') + summary('FLOOR_') + t('。低于下限的保存会被拒绝。');
    $('spl-applied').textContent = t('已生效目标：') + summary('APPLIED_');
    splReadable = true;
    if ((announce || splReadFailed) && !splSaving) {
      message.className = 'om-message';
      message.textContent = t(preserve ? '设备属性已刷新，未保存的修改已保留。' : '设备属性已刷新。');
    }
    splReadFailed = false;
    return {ok:true};
  } catch (error) {
    splReadable = false;
    splReadFailed = true;
    $('status').textContent = t(error.message);
    $('origin').textContent = t('设备原始值：不可用');
    $('spl-floor').textContent = t('下限：不可用');
    $('spl-applied').textContent = t('已生效目标：不可用');
    message.className = 'om-message error';
    message.textContent = t(error.message);
    return {ok:false, error:t(error.message)};
  } finally { save.disabled = splSaving || !splReadable; }
}
save.onclick = () => {
  if (save.disabled || splSaving) return;
  const values = propertyFields.map(field => $(field.id).value.trim());
  const datesValid = fields.every(name => valid($(name).value.trim()));
  if (!datesValid) { message.className = 'om-message error'; message.textContent = t('日期格式必须为 YYYY-MM-DD，或留空保持当前生效值。'); return; }
  if (!validOsVersion($('osversion').value.trim())) { message.className = 'om-message error'; message.textContent = t('OS 版本必须是 1-2 位数字，或留空保持当前生效值。'); return; }
  splSaving = true;
  splBusy(true);
  message.className = 'om-message';
  message.textContent = t('正在保存并应用…');
  const args = values.map(value => `'${value}'`).join(' ');
  return queueAction(async () => {
    try {
      const result = await exec(`sh "${moduleDir}/spl-control.sh" save ${args}`);
      message.className = result.errno === 0 ? 'om-message' : 'om-message error';
      message.textContent = result.errno === 0
        ? t('已应用。新生成的证明证书才会体现 OS 版本；已存在的密钥不变。')
        : t('应用失败：') + (result.stderr || result.stdout);
      // Re-read only after the mutation callback has confirmed completion.
      if (result.errno === 0) savedSplValues = splValues();
      await refreshData(true);
    } finally { splSaving = false; splBusy(false); }
  });
};
$('same').onclick = () => { $('boot').value = $('vendor').value = $('system').value.trim(); };
$('auto').onclick = () => propertyFields.forEach(field => $(field.id).value = '');
$('spl-reload').onclick = () => {
  if (splDirty() && !window.confirm(t('重新读取会丢弃未保存的设备属性修改，继续？'))) return;
  return requestRefresh({replaceSpl:JSON.stringify(splValues()), announceSpl:true});
};
function updateSummary() {
  setText('summary-device', summaryValues?.[KEYS[1]] || '—');
  setText('runtime-device', summaryValues?.[KEYS[1]] || '—');
  // Never display embedded URL credentials, fragments or queries on the overview.
  try { const url = new URL(summaryValues?.[KEYS[0]]); setText('summary-server',url.origin + url.pathname); }
  catch { setText('summary-server','—'); }
}

// The public status endpoint is the only server-side signal this WebUI can
// trust: a running local relay does not prove that the server has seen it.
let serverCheckedAt = 0, serverCheckedTarget = '';
let serverResult = {ok:false, error:t('无法读取 Server 状态；本机进程状态不代表已连接。')};
function serverOnlineState(raw, deviceId) {
  let status;
  try { status = JSON.parse(raw); } catch { return 'unknown'; }
  if (!status || status.status !== 'ok' || !Array.isArray(status.online_b_devices) || !deviceId) return 'unknown';
  if (!status.online_b_devices.every(device => device && typeof device.device_id === 'string')) return 'unknown';
  return status.online_b_devices.some(device => device.device_id === deviceId) ? 'online' : 'offline';
}

// The relay already talks to the server on every poll. Reading its last poll
// result is cheaper and more reliable than asking the WebView to fetch HTTPS:
// stock Android ships no curl, and the manager's busybox wget cannot always
// complete this server's TLS handshake. The log line is a plain local read.
function parseRelayPoll(raw) {
  const line = String(raw || '').trim().split(/\r?\n/).filter(Boolean).pop();
  if (!line) return null;
  const match = /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}) UTC .*b\/poll status=(\d+)/.exec(line);
  if (!match) return null;
  const at = Date.parse(match[1].replace(' ', 'T') + 'Z');
  if (!Number.isFinite(at)) return null;
  return { status: Number(match[2]), at };
}
function renderRelayPollStatus(poll) {
  const heading = $('server-state');
  const time = new Intl.DateTimeFormat(document.documentElement.lang, {timeStyle:'medium'}).format(new Date(poll.at));
  const age = Date.now() - poll.at;
  let title, note, tone;
  if ((poll.status === 200 || poll.status === 204) && age <= 30000) {
    title = text('服务器已连接', 'Server connected');
    note = text(`Relay 最近一次 poll 成功（HTTP ${poll.status}），时间 ${time}。`, `Relay last poll succeeded (HTTP ${poll.status}) at ${time}.`);
    tone = 'online';
  } else if (poll.status === 200 || poll.status === 204) {
    title = text('连接状态未知', 'Connection unknown');
    note = text(`Relay 最近一次成功 poll 是 ${time}，已超过 30 秒。屏幕关闭或进程被冻结时可能出现。`, `The last successful relay poll was at ${time}, more than 30 seconds ago. This can happen while the screen is off or the process is frozen.`);
    tone = 'unknown';
  } else if (poll.status === 429) {
    title = text('服务器限流（429）', 'Server throttled (429)');
    note = text(`Relay 最近一次 poll 于 ${time} 被服务器限流；请检查 Server 的 relay 配额。`, `The relay was rate-limited at ${time}; check the server relay budget.`);
    tone = 'offline';
  } else if (poll.status === 401 || poll.status === 403) {
    title = text(`Relay Token 被拒绝（${poll.status}）`, `Relay token rejected (${poll.status})`);
    note = text(`Relay 最近一次 poll 于 ${time} 被拒绝；请核对 B 端与 Server 的 token。`, `The relay poll was rejected at ${time}; check the token on B and the server.`);
    tone = 'offline';
  } else if (poll.status >= 500) {
    title = text(`Server 错误（${poll.status}）`, `Server error (${poll.status})`);
    note = text(`Relay 最近一次 poll 于 ${time} 返回 HTTP ${poll.status}。`, `The last relay poll returned HTTP ${poll.status} at ${time}.`);
    tone = 'offline';
  } else {
    title = text(`Server 返回 ${poll.status}`, `Server returned ${poll.status}`);
    note = text(`Relay 最近一次 poll 于 ${time} 返回 HTTP ${poll.status}。`, `The last relay poll returned HTTP ${poll.status} at ${time}.`);
    tone = 'unknown';
  }
  setText('server-state', title);
  setText('server-note', note);
  if (heading) heading.dataset.tone = tone;
}
function renderServerStatus(state, reason = '') {
  const title = state === 'online' ? '服务器已连接' : state === 'offline' ? 'B 端未连接' : reason === 'config' ? '尚未配置服务器连接' : '连接状态未知';
  const note = state === 'online'
    ? 'Server 最近收到此设备的轮询。'
    : state === 'offline'
      ? 'Server 尚未收到此 B 端的轮询。'
      : reason === 'config'
        ? '请先在连接页配置服务器地址和设备 ID。'
        : '无法读取 Server 状态；本机进程状态不代表已连接。';
  const heading = $('server-state');
  setText('server-state', t(title));
  setText('server-note', t(note));
  if (heading) heading.dataset.tone = state;
}
async function refreshServerStatus(force = false, configReadable = true) {
  $('server-panel').setAttribute('aria-busy', 'false');
  if (!configReadable) {
    serverCheckedTarget = '';
    renderServerStatus('unknown');
    return {ok:false, error:t('无法读取保存的连接配置，请刷新重试。')};
  }
  const url = summaryValues?.[KEYS[0]], deviceId = summaryValues?.[KEYS[1]];
  if (!url || !deviceId) {
    serverCheckedTarget = '';
    renderServerStatus('unknown', 'config');
    return {ok:true};
  }
  const target = JSON.stringify([url, deviceId]);
  // The queue ensures no status read can overlap a newer config read/save.
  // Forced reads and a changed saved endpoint always bypass the 20s poll limit.
  if (!force && target === serverCheckedTarget && Date.now() - serverCheckedAt < 20000) return serverResult;
  serverCheckedTarget = target;
  serverCheckedAt = Date.now();
  $('server-panel').setAttribute('aria-busy', 'true');
  renderServerStatus('unknown');
  setText('server-note', t('正在读取服务器状态…'));
  try {
    const base = new URL(url);
    if (!['http:', 'https:'].includes(base.protocol) || !base.hostname || base.username || base.password || base.search || base.hash) {
      throw new Error('服务器地址无效，请重新读取或修改连接配置。');
    }
    // Prefer the relay's own last poll: it proves the server accepted the
    // connection without depending on a WebView-side HTTPS client.
    const relayRead = await exec(`log=/data/adb/ommega/logs/relay.log
if [ -r "$log" ]; then
  tail -n 500 "$log" 2>/dev/null | grep -aE 'b/poll status=[0-9]+' | tail -n 1
fi`, {read:true});
    const poll = relayRead.errno === 0 ? parseRelayPoll(relayRead.stdout) : null;
    if (poll) {
      renderRelayPollStatus(poll);
      serverResult = {ok:true};
      return serverResult;
    }
    const statusUrl = base.href.replace(/\/+$/, '') + '/api/status/';
    const bytes = new TextEncoder().encode(statusUrl);
    let binary = '';
    for (const byte of bytes) binary += String.fromCharCode(byte);
    const encoded = btoa(binary);
    const result = await exec(`ommega_status_url=$(printf '%s' '${encoded}' | base64 -d)
if command -v curl >/dev/null 2>&1; then
  curl --silent --show-error --fail --insecure --connect-timeout 3 --max-time 6 -- "$ommega_status_url" | head -c 262145
elif command -v busybox >/dev/null 2>&1; then
  busybox wget -q --no-check-certificate -O - -T 6 "$ommega_status_url" | head -c 262145
else
  exit 1
fi`, {read:true});
    if (result.errno !== 0) throw new Error(result.stderr || '无法读取 Server 状态；本机进程状态不代表已连接。');
    const state = result.stdout.length <= 262144 ? serverOnlineState(result.stdout, deviceId) : 'unknown';
    if (state === 'unknown') throw new Error('服务器响应无效，请刷新重试。');
    renderServerStatus(state);
    serverResult = {ok:true};
  } catch (error) {
    renderServerStatus('unknown');
    setText('server-note', t(error.message));
    serverResult = {ok:false, error:t(error.message)};
  } finally { $('server-panel').setAttribute('aria-busy', 'false'); }
  return serverResult;
}

async function readRuntime() {
  let values = {}, error = '';
  try {
    const result = await exec(`pids=$(pidof relay 2>/dev/null)
printf 'PIDS=%s\\n' "$pids"
printf 'SPL=%s\\n' "$(getprop ro.build.version.security_patch)"
printf 'OS=%s\\n' "$(getprop ro.build.version.release)"
printf 'VERSION=%s\\n' "$(grep '^version=' /data/adb/modules/ommegaclient_b/module.prop 2>/dev/null | cut -d= -f2-)"`, {read:true});
    if (result.errno !== 0) throw new Error(result.stderr || '状态不可用');
    values = parseProperties(result.stdout);
    if (!['PIDS','SPL','OS','VERSION'].every(key => Object.hasOwn(values, key))) throw new Error('本机状态响应无效，请刷新重试。');
  } catch (cause) { values = {}; error = t(cause.message); }
  const running = !error && /^\d+(\s+\d+)*$/.test(values.PIDS || '');
  setText('relay-status',running ? t('进程运行中') : t('未确认 relay 运行状态'));
  const relayStatus = $('relay-status');
  relayStatus.className = 'badge' + (running ? '' : ' unknown');
  relayStatus.dataset.tone = running ? 'ok' : 'unknown';
  const emblem = $('runtime-emblem');
  emblem.className = 'status-emblem' + (running ? '' : ' unknown');
  emblem.dataset.tone = running ? 'ok' : 'unknown';
  $('runtime-panel').dataset.tone = running ? 'ok' : 'unknown';
  setText('runtime-pid',running ? `PID ${values.PIDS}` : '—');
  setText('summary-spl', values.SPL || '—'); setText('summary-os',values.OS || '—');
  setText('summary-version',values.VERSION || '—');
  setText('runtime-time', error || `${t('状态读取于')} ${new Intl.DateTimeFormat(document.documentElement.lang,{timeStyle:'medium'}).format(new Date())}`);
  return {ok:!error, error};
}

let refreshFailed = false;
function refreshFeedback(value, error = false) {
  const output = $('refresh-message');
  output.hidden = false;
  output.className = error ? 'om-message error' : 'om-updated';
  output.textContent = value;
}
async function refreshData(forced = false, {replaceRelay = false, replaceSpl = false, announceSpl = false} = {}) {
  if (forced) refreshFeedback(t('正在刷新已保存的配置、运行状态与服务器状态…'));
  for (const id of ['relay-reload','spl-reload']) $(id).setAttribute('aria-busy', 'true');
  try {
    const config = readRelay({replaceDraft:replaceRelay, announce:replaceRelay});
    const results = await Promise.all([
      config, readRuntime(), refreshSpl({replaceDraft:replaceSpl, announce:announceSpl}),
      config.then(result => refreshServerStatus(forced, result.ok)),
    ]);
    const labels = ['Relay 配置', '本机状态', '设备属性', '服务器连接'];
    const failures = results.flatMap((result, index) => result.ok ? [] : [`${t(labels[index])} · ${result.error}`]);
    if (failures.length) refreshFeedback(t('部分状态读取失败：') + failures.join('；'), true);
    else if (forced || refreshFailed) refreshFeedback(t(relayDirty() || splDirty() ? '状态已刷新，未保存的修改已保留。' : '状态已刷新。'));
    refreshFailed = failures.length > 0;
    return {ok:!refreshFailed};
  } catch (error) {
    refreshFailed = true;
    refreshFeedback(t('刷新失败，请重试。') + ' ' + t(error.message), true);
    return {ok:false};
  } finally {
    for (const id of ['relay-reload','spl-reload']) $(id).setAttribute('aria-busy', 'false');
  }
}
function requestRefresh(options = {}) {
  refreshFeedback(t('正在刷新已保存的配置、运行状态与服务器状态…'));
  return queueAction(() => refreshData(true, options));
}
const refresh = pollWhenVisible(forced => queueAction(() => refreshData(forced)), 10000, $('runtime-refresh'));
$('runtime-refresh').onclick = refresh;
function navigate(view, focus = false) {
  if (!['overview','connection','settings'].includes(view)) view = 'overview';
  for (const id of ['overview','connection','settings']) $(id + '-view').hidden = id !== view;
  document.querySelectorAll('[data-view]').forEach(el => el.setAttribute('aria-current',el.dataset.view === view ? 'page' : 'false'));
  document.querySelector('.om-nav').dataset.active = String(['overview','connection','settings'].indexOf(view));
  if (focus) $('main').focus({preventScroll:true});
}
document.querySelectorAll('[data-view]').forEach(el => { el.onclick = () => { location.hash = el.dataset.view; }; });
window.addEventListener('hashchange',() => navigate(location.hash.slice(1),true));
navigate(location.hash.slice(1));
for (const [id, dialog] of [['appearance','appearance-dialog'], ['spl','spl-dialog'], ['about','about-dialog']]) {
  $(id + '-open').onclick = () => {
    const sheet = $(dialog);
    sheet.showModal();
    // Some WebViews leave focus on the opener after showModal(); put it in the
    // sheet so touch scrolling cannot fall through to the page behind it.
    requestAnimationFrame(() => sheet.querySelector('input:not([type=hidden]),select,textarea,button')?.focus({preventScroll:true}));
    if (id === 'spl' || id === 'about') return requestRefresh({announceSpl:id === 'spl'});
  };
  $(id + '-close').onclick = () => $(dialog).close();
}
installDialogGuard(open => document.body.classList.toggle('om-dialog-open', open), {includeNative:true});
bindAppearance(() => {
  const relayDirty = savedValues && JSON.stringify(relayValues()) !== JSON.stringify(savedValues);
  const splDirty = savedSplValues && propertyFields.some((field, index) => $(field.id).value !== savedSplValues[index]);
  return (!relayDirty && !splDirty) || window.confirm(t('尚有未保存的修改，继续会丢失。切换语言？'));
});
