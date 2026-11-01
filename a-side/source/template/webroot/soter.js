// SOTER identities: full page (list) and detail page in the modern shell.
// Store: /data/adb/ommega/soterta/identities.json; key material in identities/<id>.pem.
import {icon, text} from './ui-kit.js';
import {describe} from './soter-key.js';

const DIR = '/data/adb/ommega/soterta';
const STORE = DIR + '/identities.json';
const KEYDIR = DIR + '/identities';

const TYPES = [
  ['local', '本地随机身份', 'Local random', '本机现场生成的随机设备身份'],
  ['key', '私钥身份', 'Private key', '导入一把 ATTK 私钥，对应已登记的设备 id'],
  ['relay', '远程中继身份', 'Remote relay', '转发到另一台设备的软件 TA'],
];

let exec = null;
let store = null;
let adding = false;

function esc(value) {
  return String(value === undefined || value === null ? '' : value)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}
function blank() { return {version: 1, default: '', identities: [], apps: {}}; }
function label(type) { const e = TYPES.find(t => t[0] === type); return e ? text(e[1], e[2]) : type; }

function injectStyle() {
  if (document.getElementById('om-soter-style')) return;
  const style = document.createElement('style');
  style.id = 'om-soter-style';
  style.textContent = [
    '.om-soter-toolbar{display:flex;gap:8px;margin:0 0 14px}a.action-row{text-decoration:none;color:inherit}.om-soter-list{display:flex;flex-direction:column;gap:10px}',
    '.om-soter-row{display:flex;align-items:center;gap:12px;width:100%;text-align:left;padding:16px 18px;border:1px solid var(--om-line);border-radius:16px;background:var(--om-raised);color:var(--om-ink)}',
    '.om-soter-row .name{font-weight:650;font-size:15px}',
    '.om-soter-row .meta{font-size:12px;color:var(--om-muted);margin-top:3px;overflow-wrap:anywhere}',
    '.om-soter-row .tag{margin-left:auto;flex:0 0 auto;font-size:11px;padding:4px 9px;border-radius:999px;background:var(--om-card);color:var(--om-muted)}',
    '.om-soter-types{display:grid;gap:10px;grid-template-columns:repeat(3,minmax(0,1fr));margin-top:12px}',
    '.om-soter-type{display:flex;flex-direction:column;gap:6px;padding:14px;border:1px solid var(--om-line);border-radius:16px;background:var(--om-raised);color:var(--om-ink);text-align:left}',
    '.om-soter-type b{font-size:14px}.om-soter-type small{font-size:11px;color:var(--om-muted);line-height:1.4}',
    '.om-soter-field{display:flex;flex-direction:column;gap:6px;margin:12px 0}',
    '.om-soter-field label{font-size:12px;color:var(--om-muted)}',
    '.om-soter-field input{min-height:44px;padding:10px 12px;border-radius:12px;border:1px solid var(--om-line);background:var(--om-raised);color:var(--om-ink);font-size:13px}',
    '.om-soter-kv{display:grid;grid-template-columns:110px minmax(0,1fr);gap:6px 12px;font-size:13px;margin:10px 0}',
    '.om-soter-kv .k{color:var(--om-muted)}.om-soter-kv .v{overflow-wrap:anywhere}',
    '.om-soter-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}',
    '.om-soter-msg{font-size:12px;color:var(--om-muted);line-height:1.5;margin:10px 0 0;overflow-wrap:anywhere}',
  ].join('');
  document.head.appendChild(style);
}

async function loadStore() {
  const result = await exec('cat "' + STORE + '" 2>/dev/null');
  let data = null;
  try { data = JSON.parse(String(result.stdout || '')); } catch (error) { data = null; }
  if (!data || !Array.isArray(data.identities)) data = blank();
  if (!data.apps || typeof data.apps !== 'object') data.apps = {};
  return data;
}
function b64utf8(value) {
  const bytes = new TextEncoder().encode(value);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
async function saveStore() {
  const payload = b64utf8(JSON.stringify(store, null, 2) + '\n');
  const command = ['set -e', 'mkdir -p "' + DIR + '"',
    'printf %s \'' + payload + '\' | base64 -d > "' + STORE + '.tmp"',
    'chmod 600 "' + STORE + '.tmp"', 'mv "' + STORE + '.tmp" "' + STORE + '"'].join('\n');
  const result = await exec(command);
  if (result && result.errno) throw new Error(String(result.stderr || ('errno ' + result.errno)).trim());
}

function headHtml(title, subtitle, backId, extra) {
  return '<div class="page-heading"><div><span class="section-kicker">SOTER</span><h1>' + title + '</h1><p>' + subtitle + '</p></div></div>' +
    '<div class="om-soter-toolbar"><button type="button" class="om-button secondary" id="' + backId + '">' + text('返回', 'Back') + '</button>' +
    (extra || '') + '</div>';
}

function panel(title, body) {
  return '<article class="panel settings-group"><div class="group-header"><h2 class="group-heading">' + title +
    '</h2></div>' + body + '</article>';
}

function listHtml() {
  const rows = store.identities.map(item => {
    let meta = label(item.type);
    if (item.type === 'key') meta += ' · ' + (item.cpu_id || text('未填写私钥', 'no key yet'));
    if (item.type === 'relay') meta += ' · ' + (item.url || text('未填写服务器', 'no server yet'));
    return '<a class="action-row" href="#om-soter/' + esc(item.id) + '" data-open="' + esc(item.id) + '">' +
      '<span class="row-icon">' + icon(item.type === 'key' ? 'key' : item.type === 'relay' ? 'server' : 'shield') + '</span>' +
      '<span class="row-text"><strong>' + esc(item.name) + (store.default === item.id ? ' · ' + text('默认', 'default') : '') +
      '</strong><small>' + esc(meta) + '</small></span>' + icon('chevron') + '</a>';
  }).join('');
  const types = TYPES.map(t => '<button type="button" class="action-row" data-add="' + t[0] + '">' +
    '<span class="row-icon">' + icon(t[0] === 'key' ? 'key' : t[0] === 'relay' ? 'server' : 'shield') + '</span>' +
    '<span class="row-text"><strong>' + text(t[1], t[2]) + '</strong><small>' + text(t[3], t[3]) + '</small></span>' +
    icon('chevron') + '</button>').join('');
  const empty = '<p class="note" style="padding:0 20px 18px">' +
    text('还没有身份。点「添加身份」创建第一个。', 'No identity yet. Use Add identity to create one.') + '</p>';
  return headHtml(text('SOTER 身份', 'SOTER identities'),
      text('每个身份是一套设备凭证；应用策略里选择用哪一套。', 'Each identity is a device credential; pick one per app policy.'),
      'om-soter-back',
      '<button type="button" class="om-button" id="om-soter-add">+ ' + text('添加身份', 'Add identity') + '</button>') +
    '<div class="stack">' +
    (adding ? panel(text('选择身份类型', 'Choose a type'), types) : '') +
    panel(text('已添加的身份', 'Identities'), store.identities.length ? rows : empty) +
    '</div>';
}

function detailHtml(item, status) {
  const rows = [['类型 / Type', label(item.type)]];
  if (status) {
    rows.push(['软件 TA / TA', status.running ? text('运行中', 'running') + ' · pid ' + (status.pid || '-') : text('未运行', 'not running')]);
    if (status.device_id && status.device_id !== '-') rows.push(['当前设备 / Active id', status.device_id]);
  }
  if (item.type === 'key') {
    rows.push(['cpu_id', item.cpu_id || '—']);
    rows.push(['私钥 / Key', item.pem ? text('已导入', 'imported') : text('未导入', 'missing')]);
    if (item.fingerprint) rows.push(['指纹 / Fingerprint', 'SHA-256 ' + item.fingerprint + '…']);
    if (item.cpu_id && status && status.device_id) {
      rows.push(['生效 / Applied', status.device_id === item.cpu_id ? text('是', 'yes') : text('否', 'no')]);
    }
  } else if (item.type === 'relay') {
    rows.push(['服务器 / Server', item.url || '—']);
    rows.push(['设备 ID / Device', item.device_id || '—']);
    rows.push(['Token', item.token ? text('已填写', 'set') : text('未填写', 'missing')]);
  } else {
    rows.push(['说明 / Note', text('使用本机账本当前的随机身份', 'Uses the local ledger identity')]);
  }
  const kv = rows.map(r => { const parts = r[0].split(' / '); return '<div class="k">' + esc(text(parts[0], parts[1] || parts[0])) + '</div><div class="v">' + esc(r[1]) + '</div>'; }).join('');
  let editor = '';
  if (item.type === 'key') {
    editor = '<div class="om-soter-field"><label>' + text('私钥文件路径', 'Private key file path') + '</label>' +
      '<input id="om-soter-key-path" placeholder="/sdcard/Download/ommega-soter.json"></div>' +
      '<div class="om-soter-actions"><button type="button" class="om-button secondary" id="om-soter-key-import">' +
      text('导入私钥', 'Import key') + '</button></div>';
  } else if (item.type === 'relay') {
    editor = '<div class="om-soter-field"><label>' + text('服务器 URL', 'Server URL') + '</label><input id="om-soter-relay-url" value="' + esc(item.url || '') + '"></div>' +
      '<div class="om-soter-field"><label>' + text('设备 ID', 'Device id') + '</label><input id="om-soter-relay-device" value="' + esc(item.device_id || '') + '"></div>' +
      '<div class="om-soter-field"><label>Token</label><input id="om-soter-relay-token" type="password" value="' + esc(item.token || '') + '"></div>' +
      '<div class="om-soter-actions"><button type="button" class="om-button secondary" id="om-soter-relay-save">' +
      text('保存', 'Save') + '</button></div>';
  }
  const body = '<div style="padding:2px 20px 6px"><div class="om-soter-kv">' + kv + '</div>' + editor + '</div>';
  const settings = '<div style="padding:2px 20px 18px">' +
    '<div class="om-soter-field"><label>' + text('名称', 'Name') + '</label><input id="om-soter-name" value="' + esc(item.name) + '"></div>' +
    '<div class="om-soter-actions">' +
      '<button type="button" class="om-button" id="om-soter-apply">' + text('重启软件 TA', 'Restart software TA') + '</button>' +
      '<button type="button" class="om-button secondary" id="om-soter-rename">' + text('保存名称', 'Save name') + '</button>' +
      '<button type="button" class="om-button secondary" id="om-soter-default">' + text('设为默认', 'Set as default') + '</button>' +
      '<button type="button" class="om-button secondary" id="om-soter-delete">' + text('删除', 'Delete') + '</button>' +
    '</div><p class="om-soter-msg" id="om-soter-msg"></p></div>';
  return headHtml(esc(item.name), label(item.type), 'om-soter-detail-back', '') +
    '<div class="stack">' + panel(text('身份信息', 'Identity'), body) +
    panel(text('操作', 'Actions'), settings) + '</div>';
}

async function readTaStatus() {
  try {
    const result = await exec('cat /data/adb/ommega/soterta/status.json 2>/dev/null');
    return JSON.parse(String(result.stdout || ''));
  } catch (error) { return null; }
}

async function restartSoftwareTa() {
  const dir = '/data/adb/ommega/soterta';
  await exec('rm -f ' + dir + '/enabled');
  await new Promise(resolve => setTimeout(resolve, 3500));
  await exec('mkdir -p ' + dir + ' && date +%s > ' + dir + '/enabled.tmp && chmod 0600 ' + dir + '/enabled.tmp && mv ' + dir + '/enabled.tmp ' + dir + '/enabled');
  await new Promise(resolve => setTimeout(resolve, 3500));
}

async function renderList() {
  const host = document.getElementById('om-soter-page');
  if (!host) return;
  store = await loadStore();
  host.innerHTML = listHtml();
  document.getElementById('om-soter-back').onclick = () => { history.back(); };
  document.getElementById('om-soter-add').onclick = () => { adding = !adding; renderList(); };
  host.onclick = async event => {
    const row = event.target.closest ? event.target.closest('[data-open]') : null;
    if (row) { location.hash = 'om-soter/' + row.dataset.open; return; }
    const button = event.target.closest ? event.target.closest('[data-add]') : null;
    if (!button) return;
    {
    const type = button.dataset.add;
    const id = type + '-' + Date.now().toString(36);
    const item = {id: id, name: text('身份 ', 'Identity ') + (store.identities.length + 1), type: type};
    if (type === 'relay') { item.url = ''; item.device_id = ''; item.token = ''; item.uid_map = ''; item.tls_insecure = false; }
    store.identities.push(item);
    try { await saveStore(); } catch (error) { window.alert(String(error)); return; }
    adding = false;
    location.hash = 'om-soter/' + id;
    }
  };
}

async function renderDetail(id) {
  const host = document.getElementById('om-soter-identity');
  if (!host) return;
  store = await loadStore();
  const item = store.identities.find(entry => entry.id === id);
  if (!item) { host.innerHTML = '<p class="note" style="padding:0 20px">' + text('身份不存在', 'Identity not found') + '</p>'; return; }
  const status = await readTaStatus();
  host.innerHTML = detailHtml(item, status);
  const msg = value => { const node = document.getElementById('om-soter-msg'); if (node) node.textContent = value; };
  document.getElementById('om-soter-detail-back').onclick = () => { history.back(); };
  document.getElementById('om-soter-rename').onclick = async () => {
    const value = document.getElementById('om-soter-name').value.trim();
    if (!value) { msg(text('名称不能为空', 'The name cannot be empty')); return; }
    item.name = value;
    try { await saveStore(); await renderDetail(id); msg(text('名称已保存', 'Name saved')); } catch (error) { msg(String(error)); }
  };
  document.getElementById('om-soter-default').onclick = async () => {
    store.default = item.id;
    try { await saveStore(); msg(text('已设为默认', 'Set as default')); } catch (error) { msg(String(error)); }
  };
  document.getElementById('om-soter-delete').onclick = async () => {
    if (!window.confirm(text('删除这个身份？', 'Delete this identity?'))) return;
    store.identities = store.identities.filter(entry => entry.id !== item.id);
    if (store.default === item.id) store.default = '';
    try { await saveStore(); history.back(); } catch (error) { msg(String(error)); }
  };
  document.getElementById('om-soter-apply').onclick = async () => {
    msg(text('正在重启软件 TA…', 'Restarting the software TA…'));
    try {
      await restartSoftwareTa();
      await renderDetail(id);
      msg(text('已重启；身份分配由 TA 按应用实时读取', 'Restarted; the TA reads app assignments live'));
    } catch (error) { msg(String(error)); }
  };
  const keyButton = document.getElementById('om-soter-key-import');
  if (keyButton) keyButton.onclick = async () => {
    const path = document.getElementById('om-soter-key-path').value.trim();
    if (!/^[A-Za-z0-9._\/-]+$/.test(path)) { msg(text('路径非法', 'Invalid path')); return; }
    msg(text('读取中…', 'Reading…'));
    try {
      const cat = await exec('cat "' + path + '" 2>/dev/null');
      const raw = String(cat.stdout || '').trim();
      if (!raw) throw new Error(text('文件为空', 'the file is empty'));
      let pem = raw; let cpu = '';
      if (raw.startsWith('{')) {
        const bundle = JSON.parse(raw);
        cpu = String(bundle.cpu_id || '').toLowerCase();
        pem = String(bundle.attk_private_pem || bundle.attk_priv || bundle.private_pem || '');
      }
      if (!/^[0-9a-f]{32}$/.test(cpu)) throw new Error(text('cpu_id 必须是 32 位十六进制', 'cpu_id must be 32 hex'));
      const info = await describe(pem);
      await exec('mkdir -p "' + KEYDIR + '" && printf %s \'' + b64utf8(info.privatePem) + '\' | base64 -d > "' + KEYDIR + '/' + item.id + '.pem" && chmod 600 "' + KEYDIR + '/' + item.id + '.pem"');
      item.cpu_id = cpu; item.fingerprint = info.fingerprint; item.pem = KEYDIR + '/' + item.id + '.pem';
      await saveStore();
      await renderDetail(id);
      msg(text('私钥已保存，TA 下一次调用即生效', 'Key saved; the TA picks it up on the next call'));
    } catch (error) { msg(String(error)); }
  };
  const relaySave = document.getElementById('om-soter-relay-save');
  if (relaySave) relaySave.onclick = async () => {
    item.url = document.getElementById('om-soter-relay-url').value.trim();
    item.device_id = document.getElementById('om-soter-relay-device').value.trim();
    item.token = document.getElementById('om-soter-relay-token').value.trim();
    try { await saveStore(); msg(text('已保存', 'Saved')); } catch (error) { msg(String(error)); }
  };
}

export function mountSoter(bridge) { exec = bridge; injectStyle(); }
export function soterShow(page, id) { return page === 'detail' ? renderDetail(id) : renderList(); }
