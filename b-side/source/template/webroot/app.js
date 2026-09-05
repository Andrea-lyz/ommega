import {KEYS, LEVELS, parseConfig, validateConfig, serializeConfig, readCommand, parseSnapshot, saveCommand} from './relay-config.js';

const $ = id => document.getElementById(id);
let seq = 0;
function exec(command) {
  return new Promise(resolve => {
    const callback = `ommega_b_${Date.now()}_${seq++}`;
    window[callback] = (errno, stdout, stderr) => {
      delete window[callback];
      resolve({errno: Number(errno), stdout: stdout || '', stderr: stderr || ''});
    };
    try { window.ksu.exec(command, '{}', callback); }
    catch { delete window[callback]; resolve({errno: 1, stdout: '', stderr: '请从支持 WebUI 的模块管理器打开此页面。'}); }
  });
}
const moduleDir = '/data/adb/modules/ommegaclient_b';
const relayIds = ['relay-server', 'relay-device', 'relay-machine', 'relay-token', 'file-log-enabled', 'file-log-level', 'logcat-enabled', 'logcat-level'];
const logKeys = KEYS.slice(4);
let snapshot = null;
let savedValues = null;

for (const id of ['file-log-level', 'logcat-level']) {
  for (const level of LEVELS) $(id).add(new Option(level, level));
}
function relayMessage(text, error = false) {
  $('relay-message').textContent = text;
  $('relay-message').className = error ? 'message error' : 'message';
}
function relayBusy(busy) {
  $('relay-fields').disabled = busy || !snapshot;
  $('relay-reload').disabled = busy;
}
function fillRelay(values) {
  relayIds.forEach((id, i) => {
    if ($(id).type === 'checkbox') $(id).checked = values[KEYS[i]] === 'true';
    else $(id).value = values[KEYS[i]];
  });
  $('relay-token').type = 'password';
  $('token-toggle').textContent = '显示';
  $('token-toggle').setAttribute('aria-pressed', 'false');
}
function relayValues() {
  return Object.fromEntries(relayIds.map((id, i) => [KEYS[i], $(id).type === 'checkbox' ? String($(id).checked) : $(id).value]));
}
async function readRelay() {
  relayBusy(true);
  relayMessage('正在读取配置…');
  try {
    const result = await exec(readCommand());
    if (result.errno !== 0) throw new Error(result.stderr || '读取失败，请检查管理器的 root 权限。');
    snapshot = parseSnapshot(result.stdout);
    savedValues = parseConfig(snapshot.raw);
    fillRelay(savedValues);
    relayMessage(snapshot.digest === 'missing' ? '尚无配置，请填写连接参数后保存。' : '已读取保存的配置。');
  } catch (error) { snapshot = null; relayMessage(error.message, true); }
  finally { relayBusy(false); }
}
$('relay-form').onsubmit = async event => {
  event.preventDefault();
  if (!snapshot || $('relay-fields').disabled) return;
  let values;
  try { values = validateConfig(relayValues()); }
  catch (error) { relayMessage(error.message, true); return; }
  const connectionChanged = KEYS.slice(0, 4).some(key => values[key] !== savedValues[key]);
  const logsChanged = logKeys.some(key => values[key] !== savedValues[key]);
  if (!connectionChanged && !logsChanged && snapshot.digest !== 'missing') { relayMessage('配置未改变，无需保存。'); return; }
  const raw = serializeConfig(snapshot.raw, values);
  relayBusy(true);
  relayMessage('正在保存…');
  try {
    const result = await exec(saveCommand(raw, snapshot.digest, connectionChanged));
    if (result.errno !== 0) {
      if ((result.stdout + result.stderr).includes('CONFIG_CHANGED')) {
        snapshot = null;
        throw new Error('配置已被其他操作修改，请重新读取后再保存。');
      }
      throw new Error('保存失败，请重新读取配置后检查。');
    }
    // Read the exact saved snapshot; credentials never go to browser storage or logs.
    const loaded = await exec(readCommand());
    if (loaded.errno !== 0) { snapshot = null; throw new Error('配置已写入，但复读失败，请重新读取确认。'); }
    snapshot = parseSnapshot(loaded.stdout);
    if (snapshot.raw !== raw) { snapshot = null; throw new Error('保存后配置发生变化，请重新读取确认。'); }
    savedValues = values;
    fillRelay(values);
    let notice = '配置已保存。';
    if (connectionChanged) notice += '连接参数将由运行中的 relay 自动加载。';
    if (logsChanged) notice += '日志选项在 relay 下次启动时生效。';
    if (result.stdout.includes('SAVED_RELOAD_MARKER_FAILED')) notice += '加载通知未写入，将由文件监测触发加载。';
    relayMessage(notice);
  } catch (error) { relayMessage(error.message, true); }
  finally { relayBusy(false); }
};
$('relay-reload').onclick = readRelay;
$('token-toggle').onclick = () => {
  const visible = $('relay-token').type === 'password';
  $('relay-token').type = visible ? 'text' : 'password';
  $('token-toggle').textContent = visible ? '隐藏' : '显示';
  $('token-toggle').setAttribute('aria-pressed', String(visible));
};

const fields = ['system', 'boot', 'vendor'];
const message = $('message');
const save = $('save');
const valid = value => value === '' || /^\d{4}-(0[1-9]|1[0-2])-([0-2]\d|3[01])$/.test(value);
async function refreshSpl() {
  const result = await exec(`sh "${moduleDir}/spl-control.sh" status`);
  $('status').textContent = result.stdout || result.stderr || '状态不可用';
  save.disabled = result.errno !== 0;
  if (result.errno === 0) {
    const values = Object.fromEntries(result.stdout.split('\n').map(line => line.split('=', 2)));
    fields.forEach(name => $(name).value = values[`${name.toUpperCase()}_SPL`] || '');
  }
}
save.onclick = async () => {
  const values = fields.map(name => $(name).value.trim());
  if (!values.every(valid)) { message.className = 'message error'; message.textContent = '日期格式必须为 YYYY-MM-DD，或留空使用基线。'; return; }
  save.disabled = true;
  message.className = 'message';
  message.textContent = '正在保存并应用 SPL…';
  const args = values.map(value => `'${value}'`).join(' ');
  const result = await exec(`sh "${moduleDir}/spl-control.sh" save ${args}`);
  message.className = result.errno === 0 ? 'message' : 'message error';
  message.textContent = result.errno === 0 ? 'SPL 配置已应用。' : `应用失败：${result.stderr || result.stdout}`;
  await refreshSpl();
};
$('same').onclick = () => { $('boot').value = $('vendor').value = $('system').value.trim(); };
$('auto').onclick = () => fields.forEach(name => $(name).value = '');
readRelay();
refreshSpl();
exec('pidof relay').then(result => {
  const pids = result.stdout.trim();
  $('relay-status').textContent = result.errno === 0 && /^\d+(\s+\d+)*$/.test(pids) ? `relay 运行中 · PID ${pids}` : '未确认 relay 运行状态';
});
