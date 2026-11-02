import {applyAppearance, appearanceControls, bindAppearance, createBridge, formatTime, freshness, healthMessage, icon, installDialogGuard, language, parseJson, pollWhenVisible, setText, text} from './ui-kit.js?v=material-ui-36';
import {installAppSearch,prepareSearchEditor} from './app-search.js?v=app-search-3';
import {mountSoter, soterShow} from './soter.js?v=material-ui-36';

if (typeof document !== 'undefined') {
  const style = document.createElement('link');
  style.rel = 'stylesheet'; style.href = './auth-signing.css?v=capability-mask-1'; document.head.appendChild(style);
}

export function relayVerdict(health, now = Date.now()) {
  const ok = Number(health?.last_ok_unix) || 0;
  const error = Number(health?.last_error_unix) || 0;
  if (!ok && !error) return {tone:'unknown', label:text('尚无请求记录', 'No request recorded'), at:0, age:Infinity, failed:false};
  // The daemon publishes the verdict of the last request. Older snapshots only
  // carry timestamps, and those have one-second precision: a failure and a
  // success inside the same second cannot be ordered by time.
  const verdict = health?.last_verdict;
  const failed = verdict ? verdict === 'error' : Boolean(error) && error >= ok;
  const at = failed ? error : ok;
  const age = now / 1000 - at;
  // A recorded outcome is an event, not a live connection state. Only a recent
  // one may raise an alarm; an older one stays visible with its time.
  const fresh = age >= -60 && age <= 600;
  if (failed) return {tone: fresh ? 'error' : 'warn', label:fresh ? text('最近请求失败', 'Recent request failed') : text('上次请求失败', 'Last request failed'), at, age, failed:true};
  return {tone: fresh ? 'ok' : 'muted', label:fresh ? text('最近请求成功', 'Recent request succeeded') : text('上次请求成功', 'Last request succeeded'), at, age, failed:false};
}
/** Timestamp of the newest keymint log line, which is always UTC. */
export function parseLogTime(line) {
  const match = /(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2}):(\d{2})/.exec(String(line || ''));
  if (!match) return 0;
  const at = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]), Number(match[4]), Number(match[5]), Number(match[6]));
  return Number.isFinite(at) ? at : 0;
}
/** Short "how long ago" text for a timestamp in milliseconds. */
export function formatAge(atMs, now = Date.now()) {
  if (!Number.isSafeInteger(atMs) || atMs <= 0) return '—';
  const seconds = Math.max(0, Math.round((now - atMs) / 1000));
  if (seconds < 60) return text('刚刚', 'just now');
  if (seconds < 3600) {
    const minutes = Math.floor(seconds / 60);
    return text(minutes + ' 分钟前', minutes + ' min ago');
  }
  if (seconds < 86400) {
    const hours = Math.floor(seconds / 3600);
    return text(hours + ' 小时前', hours + ' h ago');
  }
  const days = Math.floor(seconds / 86400);
  return text(days + ' 天前', days + ' d ago');
}
/**
 * The local-mode verdict. There is no relay request to report, so the card
 * follows the newest `event=generate_total` line of the local keymint log: that
 * is the work the software TA actually performed here. The attestation counters
 * in the profile are relay-only, so they must not drive this card.
 */
export function localActivityVerdict(lastActivityMs, now = Date.now()) {
  const at = Number.isSafeInteger(lastActivityMs) && lastActivityMs > 0 ? Math.floor(lastActivityMs / 1000) : 0;
  if (!at) return {tone:'unknown', label:text('尚无本机记录', 'No local record yet'), at:0, age:Infinity, failed:false};
  const age = now / 1000 - at;
  return {tone:age >= -60 && age <= 600 ? 'ok' : 'muted', label:formatAge(at * 1000, now), at, age, failed:false};
}
/**
 * The local-mode emblem. The relay icon drops the shield for a bare clock once a
 * result is old, which would read as "no attestation"; local mode always keeps
 * the shield because the material is on this device, and only the inner mark
 * follows the outcome.
 */
export function localVerdictIcon(tone) {
  const shield = '<path d="M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z"/>';
  const mark = tone === 'ok' ? '<path d="m9 12.2 2 2 4-4"/>'
    : tone === 'error' || tone === 'warn' ? '<path d="M12 8.6v3.7m0 2.3h.01"/>'
    : '<circle cx="12" cy="10.2" r="1.15"/><path d="M12 11.35v3.4"/>';
  return `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${shield}${mark}</svg>`;
}
export function requestIcon(tone) {
  const mark = tone === 'error' ? '<path d="m9.5 9.5 5 5m0-5-5 5"/>'
    : tone === 'warn' ? '<path d="M12 8v4.5m0 3h.01"/>'
    : tone === 'muted' || tone === 'unknown' ? '<circle cx="12" cy="12" r="8.5"/><path d="M12 7v5l3 2"/>'
    : '<path d="m9 12 2 2 4-4"/>';
  const shield = tone === 'muted' || tone === 'unknown' ? '' : '<path d="M12 2.8 19 5.5v5.7c0 4.4-2.6 7.8-7 10-4.4-2.2-7-5.6-7-10V5.5L12 2.8Z"/>';
  return `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${shield}${mark}</svg>`;
}
export function deviceSnapshot(profile) {
  const properties = profile?.properties;
  const value = key => typeof properties?.[key] === 'string' && properties[key].trim() ? properties[key].trim() : '—';
  const brand = value('ro.product.brand'), manufacturer = value('ro.product.manufacturer');
  return {model:value('ro.product.model'), brand:brand === manufacturer ? brand : [brand,manufacturer].filter(item => item !== '—').join(' / ') || '—',
    device:value('ro.product.device'), product:value('ro.product.name'), soc:value('ro.soc.model')};
}
export function formatRelayLatency(value) {
  return Number.isSafeInteger(value) && value >= 0 ? `${value}ms` : '—';
}
/** Flat client-a config (`/data/adb/ommega/ommegadata/config`) → relay fields. */
export function parseRelaySettings(raw) {
  const fields = Object.create(null);
  for (const line of String(raw || '').split(/\r?\n/)) {
    const at = line.indexOf(':');
    if (at < 0) continue;
    const key = line.slice(0,at).trim().toLowerCase();
    if (['url','device_id','remote','tls_insecure','tls_skip_verify','insecure_tls'].includes(key)) fields[key] = line.slice(at+1).trim();
  }
  if (!Object.keys(fields).length) return null;
  const url = fields.url || '';
  const urlOff = !url || /^(none|off)$/i.test(url);
  const flag = fields.remote || '';
  const remote = /^(0|false|no|off)$/i.test(flag) ? false : /^(1|true|yes|on)$/i.test(flag) ? true : !urlOff;
  return {url, deviceId:fields.device_id || '', remote:remote && !urlOff,
    insecure:/^(1|true|yes|on)$/i.test(fields.tls_insecure || fields.tls_skip_verify || fields.insecure_tls || '')};
}
/** `[remote]` table of config.toml, consulted only when the flat file is absent. */
export function parseTomlRemote(raw) {
  const body = String(raw || '');
  const url = /^[ \t]*url[ \t]*=[ \t]*"([^"]*)"/m.exec(body);
  const enabled = /^[ \t]*enabled[ \t]*=[ \t]*(true|false)/m.exec(body);
  if (!url && !enabled) return null;
  return {url:url ? url[1] : '', enabled:enabled ? enabled[1] === 'true' : Boolean(url && url[1])};
}
/** Effective relay mode: the flat file wins, exactly like the daemon config. */
export function relayMode(flatRaw, tomlRaw) {
  const flat = parseRelaySettings(flatRaw);
  if (flat) return flat.remote ? 'remote' : 'local';
  const toml = parseTomlRemote(tomlRaw);
  if (toml) return toml.enabled && toml.url ? 'remote' : 'local';
  return 'unknown';
}
function derLength(bytes, offset) {
  const first = bytes[offset];
  if (first === undefined) return null;
  if (first < 0x80) return {length:first, offset:offset + 1};
  const count = first & 0x7f;
  if (count === 0 || count > 4 || offset + count >= bytes.length) return null;
  let length = 0;
  for (let index = 0; index < count; index += 1) length = length * 256 + bytes[offset + 1 + index];
  return {length, offset:offset + 1 + count};
}
function derItem(bytes, offset) {
  const tag = bytes[offset];
  const head = tag === undefined ? null : derLength(bytes, offset + 1);
  if (!head) return null;
  const end = head.offset + head.length;
  if (end > bytes.length) return null;
  return {tag, start:head.offset, end};
}
function derChildren(bytes, item) {
  const children = [];
  for (let offset = item.start; offset < item.end;) {
    const child = derItem(bytes, offset);
    if (!child || child.end <= offset) break;
    children.push(child);
    offset = child.end;
  }
  return children;
}
const decoder = typeof TextDecoder === 'function' ? new TextDecoder() : null;
function derText(bytes, item) {
  const slice = bytes.subarray(item.start, item.end);
  if (decoder) return decoder.decode(slice);
  let value = '';
  for (const byte of slice) value += String.fromCharCode(byte);
  return value;
}
function derTime(bytes, item) {
  const raw = derText(bytes, item).trim();
  const match = /^(\d{2}|\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})?Z?$/.exec(raw);
  if (!match) return null;
  let year = Number(match[1]);
  if (match[1].length === 2) year += year < 50 ? 2000 : 1900;
  const at = Date.UTC(year, Number(match[2]) - 1, Number(match[3]), Number(match[4]), Number(match[5]), Number(match[6] || 0));
  return Number.isFinite(at) ? at : null;
}
const OID_O = [0x55,0x04,0x0a];
const OID_T = [0x55,0x04,0x0c];
function derName(bytes, item) {
  const name = {o:null, t:null};
  for (const set of derChildren(bytes, item)) for (const attribute of derChildren(bytes, set)) {
    const parts = derChildren(bytes, attribute);
    if (parts.length < 2) continue;
    const oid = bytes.subarray(parts[0].start, parts[0].end);
    const matches = wanted => oid.length === wanted.length && wanted.every((value, index) => oid[index] === value);
    if (matches(OID_O)) name.o = derText(bytes, parts[1]);
    else if (matches(OID_T)) name.t = derText(bytes, parts[1]);
  }
  return name;
}
/**
 * Subject O/T and validity of one DER certificate. The keybox leaf carries the
 * security level it was issued for (`O=TEE` or `O=StrongBox`), which is what a
 * verifier compares against the requested level.
 */
export function certificateSummary(der) {
  const certificate = derItem(der, 0);
  if (!certificate || certificate.tag !== 0x30) return null;
  const tbs = derChildren(der, certificate)[0];
  if (!tbs || tbs.tag !== 0x30) return null;
  const fields = derChildren(der, tbs);
  let index = fields[0] && fields[0].tag === 0xa0 ? 1 : 0;
  index += 3; // serialNumber, signature, issuer
  const validity = fields[index], subject = fields[index + 1];
  if (!validity || validity.tag !== 0x30 || !subject || subject.tag !== 0x30) return null;
  const times = derChildren(der, validity);
  if (times.length < 2) return null;
  const name = derName(der, subject);
  return {o:name.o, t:name.t, notBefore:derTime(der, times[0]), notAfter:derTime(der, times[1])};
}
/**
 * Decode the first `<Certificate>` body (PEM armour or bare base64) into DER
 * bytes. More than one certificate may arrive (the keybox chain is a sequence,
 * and a truncated range glues several of them together), so only the first
 * block is decoded; the leaf is the one that carries the level and the validity.
 */
export function certificateBytes(raw) {
  const text = String(raw || '');
  const block = /-----BEGIN[^-]*-----([\s\S]*?)-----END[^-]*-----/.exec(text);
  const body = (block ? block[1] : text).replace(/<[^>]*>/g, '').replace(/[^A-Za-z0-9+/=]/g, '');
  if (!body) return null;
  try {
    const binary = atob(body);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
    return bytes;
  } catch { return null; }
}
/**
 * One keybox slot: whether it is installed, which level it claims and how long
 * it stays valid. `expected` is the level the slot must carry.
 */
export function keyboxSummary(raw, expected, now = Date.now()) {
  if (!String(raw || '').trim()) return {state:'missing', expected, level:null, mismatch:false, days:null};
  const certificate = certificateBytes(raw);
  if (!certificate) return {state:'invalid', expected, level:null, mismatch:false, days:null};
  const summary = certificateSummary(certificate);
  if (!summary) return {state:'invalid', expected, level:null, mismatch:false, days:null};
  const level = [summary.t, summary.o].find(value => value && /tee|strongbox/i.test(value)) || null;
  const canonical = level && /strongbox/i.test(level) ? 'StrongBox' : level ? 'TEE' : null;
  const days = Number.isFinite(summary.notAfter) ? Math.floor((summary.notAfter - now) / 86400000) : null;
  return {state:'loaded', expected, level:canonical, mismatch:Boolean(canonical) && canonical !== expected,
    days, notAfter:summary.notAfter, o:summary.o, t:summary.t};
}
/** Short "remaining validity" text for the material card. */
export function formatKeyboxDays(days) {
  if (!Number.isSafeInteger(days)) return '—';
  if (days < 0) return text('已过期','Expired');
  if (days === 0) return text('今天到期','Expires today');
  return text(`剩 ${days} 天`, `${days}d left`);
}
export function mergeAttestationHealth(health, completedMs) {
  if (!Number.isSafeInteger(completedMs) || completedMs <= 0) return health;
  const completed = Math.floor(completedMs / 1000);
  if (health?.last_verdict === 'error') return health;
  if (!health?.last_verdict && Number(health?.last_error_unix) >= Number(health?.last_ok_unix || 0)) return health;
  if (completed <= Number(health?.last_ok_unix || 0)) return health;
  return {...health, last_ok_unix:completed, last_verdict:'ok'};
}
export function parseBStatusConfig(raw) {
  const fields = Object.create(null);
  for (const line of String(raw || '').split(/\r?\n/)) {
    const at = line.indexOf(':');
    if (at < 0) continue;
    const key = line.slice(0,at).trim().toLowerCase();
    if (['url','device_id','remote','tls_insecure','tls_skip_verify','insecure_tls'].includes(key)) fields[key] = line.slice(at+1).trim();
  }
  if (!fields.url || !fields.device_id || /^(none|off)$/i.test(fields.url) || /^(0|false|no|off)$/i.test(fields.remote || '')) return null;
  try {
    const url = new URL(fields.url);
    if (!['http:','https:'].includes(url.protocol) || !url.hostname || url.username || url.password || url.search || url.hash) return null;
    const tls = fields.tls_insecure || fields.tls_skip_verify || fields.insecure_tls || '';
    return {url:url.toString().replace(/\/+$/,''), deviceId:fields.device_id, insecure:/^(1|true|yes|on)$/i.test(tls)};
  } catch { return null; }
}
export function bOnlineState(raw, deviceId) {
  const status = typeof raw === 'string' ? parseJson(raw) : raw;
  if (!status || status.status !== 'ok' || !Array.isArray(status.online_b_devices) || !deviceId) return 'unknown';
  if (!status.online_b_devices.every(device => device && typeof device.device_id === 'string')) return 'unknown';
  return status.online_b_devices.some(device => device && device.device_id === deviceId) ? 'online' : 'offline';
}
export function bBatterySnapshot(raw, deviceId, now = Date.now()) {
  const status = typeof raw === 'string' ? parseJson(raw) : raw;
  if (bOnlineState(status, deviceId) !== 'online') return null;
  const device = status.online_b_devices.find(device => device.device_id === deviceId);
  const percent = device.battery_percent, charging = device.battery_charging;
  const at = device.battery_updated_at_ms;
  // Receipt and response clocks are both the server's. A's local clock can be
  // skewed and must not turn a fresh B heartbeat into missing telemetry.
  const reference = Number.isSafeInteger(status.server_time_ms) && status.server_time_ms > 0 ? status.server_time_ms : now;
  if (!Number.isSafeInteger(at) || at <= 0 || reference - at < -60000 || reference - at > 90000) return null;
  if (!Number.isInteger(percent) || percent < 0 || percent > 100) return null;
  return {percent, charging:typeof charging === 'boolean' ? charging : null, at};
}
function renderBBattery(snapshot) {
  const value = snapshot ? `${snapshot.percent}%` + (snapshot.charging === true ? ` · ${text('充电中','Charging')}` : snapshot.charging === false ? ` · ${text('未充电','Not charging')}` : '')
    : text('暂无新数据','No recent data');
  setText('om-b-battery', value);
  const element = document.getElementById('om-b-battery');
  if (element) element.title = snapshot
    ? `${text('Server 收到电量上报于','Battery report received by server at')} ${formatTime(snapshot.at / 1000)}`
    : text('等待已配置 B 端上报电量；离线、过期或不支持时不显示旧值。','Waiting for the configured B device. Offline, stale or unsupported data is not shown.');
}
function keyboxRow(summary) {
  if (summary.state === 'missing') return text('未配置','Not configured');
  if (summary.state === 'invalid') return text('无法解析','Unreadable');
  return (summary.level || text('已加载','Loaded')) + ' · ' + formatKeyboxDays(summary.days);
}
function setBadge(id, label, tone) {
  const badge = document.getElementById(id);
  if (!badge) return;
  badge.textContent = label;
  badge.className = 'badge latency-badge' + (tone === 'ok' ? '' : ' ' + tone);
  badge.dataset.tone = tone;
}
/**
 * Local mode has no relay and no B-side snapshot, so the overview swaps the
 * B-device card for the material this device actually signs with.
 */
function applyOverviewMode(mode) {
  const local = mode === 'local';
  const deviceCard = document.getElementById('om-b-summary');
  const materialCard = document.getElementById('om-local-summary');
  if (deviceCard) deviceCard.hidden = local;
  if (materialCard) materialCard.hidden = !local;
  setText('om-request-kind', local ? text('KeyMint · 本机','KeyMint · local') : 'KeyMint');
  setText('om-verdict-label', local ? text('最近证明','Last attestation') : text('最近请求','Last request'));
  setText('om-verdict-title', local ? text('本机证明','Local attestation') : text('证明链路','Attestation relay'));
  setText('om-health-time-label', local ? text('签发时间','Issued at') : text('记录时间','Recorded'));
  setText('om-health-kind-label', local ? text('执行方','Performed by') : text('请求类型','Request'));
  const route = document.getElementById('om-route');
  if (local) {
    setText('om-route-mid', text('本机 TA','Local TA'));
    setText('om-route-mid-detail', text('软件 KeyMint','Software KeyMint'));
    setText('om-route-end', 'Keybox');
    setText('om-route-end-detail', text('证书材料','Certificate material'));
  } else {
    setText('om-route-mid', 'Server');
    setText('om-route-mid-detail', text('转发','Relay'));
    setText('om-route-end', 'B');
    setText('om-route-end-detail', text('硬件 TEE','Hardware TEE'));
  }
  if (route) route.setAttribute('aria-label', local ? 'A → ' + text('本机 TA','Local TA') + ' → Keybox' : 'A → Server → B');
}
function renderLocalSummary({tee, strongbox, activity, profile, packages}) {
  const now = Date.now();
  const teeBox = keyboxSummary(tee, 'TEE', now);
  const strongBox = keyboxSummary(strongbox, 'StrongBox', now);
  setText('om-local-tee', keyboxRow(teeBox));
  setText('om-local-strongbox', keyboxRow(strongBox));
  setText('om-local-last', activity > 0 ? formatTime(activity / 1000) : text('暂无记录','No record yet'));
  // The status reader already reports the counter table that matches the active
  // mode, so this total is the local one whenever local mode is active.
  const counts = profile?.attestation_counts;
  const total = counts && typeof counts === 'object'
    ? Object.values(counts).reduce((sum, entry) => sum + (Number.isSafeInteger(entry?.count) ? entry.count : 0), 0) : null;
  setText('om-local-attest', total === null ? '—' : total === 0 ? text('暂无','None') : text(total + ' 次', total + ' total'));
  setText('om-local-scope', packages ? String(packages.size) : '—');
  setText('om-local-mode', text('未启用','Disabled'));
  const loaded = [teeBox, strongBox].filter(box => Number.isFinite(box.days)).sort((left, right) => left.days - right.days);
  const soonest = loaded[0];
  setBadge('om-local-expiry', text('最近到期','Next expiry') + ' · ' + (soonest ? formatKeyboxDays(soonest.days) : '—'),
    !soonest ? 'unknown' : soonest.days < 0 ? 'error' : soonest.days <= 7 ? 'warn' : 'ok');
  const warnings = [];
  if (teeBox.state === 'missing') warnings.push([text('TEE 材料缺失','TEE material missing'), 'error']);
  if (teeBox.state === 'invalid') warnings.push([text('TEE 材料无法解析','TEE material unreadable'), 'error']);
  if (teeBox.mismatch) warnings.push([text('TEE 槽位装的是 StrongBox 链','TEE slot holds a StrongBox chain'), 'error']);
  if (strongBox.mismatch) warnings.push([text('StrongBox 槽位装的是 TEE 链','StrongBox slot holds a TEE chain'), 'error']);
  if (Number.isFinite(teeBox.days) && teeBox.days < 0) warnings.push([text('TEE 证书已过期','TEE certificate expired'), 'error']);
  if (Number.isFinite(strongBox.days) && strongBox.days < 0) warnings.push([text('StrongBox 证书已过期','StrongBox certificate expired'), 'error']);
  if (strongBox.state === 'missing') warnings.push([text('StrongBox 未配置（可选）','StrongBox not configured (optional)'), 'muted']);
  const holder = document.getElementById('om-local-warnings');
  if (holder) {
    holder.hidden = !warnings.length;
    holder.replaceChildren(...warnings.map(([label, tone]) => {
      const chip = document.createElement('span');
      chip.className = 'chip ' + tone;
      chip.textContent = label;
      return chip;
    }));
  }
}

// ---------------------------------------------------------------- automation
const RKP_CLI = '/data/adb/modules/ommega/libs/arm64-v8a/ommega-rkp';
const RKP_CLI_X86 = '/data/adb/modules/ommega/libs/x86_64/ommega-rkp';
function shellQuote(value) {
  return "'" + String(value).replace(/'/g, "'\\''") + "'";
}
async function rkpCommand(args) {
  const result = await exec('p=' + RKP_CLI + '\n[ -x "$p" ] || p=' + RKP_CLI_X86
    + '\n[ -x "$p" ] || { echo "ommega-rkp is not installed in this module" >&2; exit 1; }\n"$p" ' + args);
  const output = String(result.stdout || '').trim();
  if (result.errno) {
    throw new Error(String(result.stderr || '').trim() || output || ('exit ' + result.errno));
  }
  try {
    return JSON.parse(output);
  } catch {
    throw new Error(output || text('命令没有返回内容','The command returned nothing'));
  }
}
function keyboxLine(box, slot) {
  if (!box || box.present !== true) return text('未导入','Not imported');
  if (box.unreadable) return text('文件存在但无法解析','The file exists but cannot be parsed');
  const days = Number.isSafeInteger(box.days) ? formatKeyboxDays(box.days) : '—';
  const level = box.level || text('未标注级别','No level');
  const parts = [level, days];
  if (box.rkp) parts.push('RKP');
  if (box.chain) parts.push(box.chain + ' ' + text('张证书','certs'));
  if (box.match === false) parts.push(text('槽位不符','slot mismatch'));
  return parts.join(' · ') + (slot ? '' : '');
}
function automationMessage(id, value, error) {
  const element = document.getElementById(id);
  if (!element) return;
  element.textContent = value;
  element.dataset.tone = error ? 'error' : '';
}
function formatDay(ms) {
  const date = new Date(ms);
  return date.getFullYear() + '/' + (date.getMonth() + 1) + '/' + date.getDate();
}
async function renderAutomation() {
  let status;
  try {
    status = await rkpCommand('status');
  } catch (error) {
    // The page reports its own failure inline; a missing tool must not make the
    // global refresh claim that some other status could not be read.
    setText('om-rkp-seed-state', text('不可用','Unavailable'));
    automationMessage('om-rkp-issue-state', String(error.message || error), true);
    return true;
  }
  const seed = status.seed || {};
  setText('om-rkp-seed-state', seed.present
    ? (seed.decryptable === false ? text('已导入但无法解密（可能来自其他设备）','Imported but undecryptable on this device') : text('已导入','Imported'))
    : text('未导入','Not imported'));
  setText('om-rkp-seed-fingerprint', seed.fingerprint || '—');
  setText('om-rkp-seed-time', Number.isSafeInteger(seed.imported_at_ms) && seed.imported_at_ms > 0 ? formatTime(seed.imported_at_ms / 1000) : '—');
  setText('om-rkp-seed-chain', seed.has_chain ? text('已包含','Included') : text('缺少','Missing'));

  setText('om-rkp-tee-state', keyboxLine(status.tee, 'tee'));
  setText('om-rkp-sb-state', keyboxLine(status.strongbox, 'sb'));

  const policy = status.policy || {};
  const tee = status.tee || {}, sb = status.strongbox || {};
  // The whole card follows the slot that expires first, so the level it names is
  // the level the deadline belongs to.
  const slots = [['TEE', tee], ['StrongBox', sb]]
    .filter(([, box]) => box.present === true && !box.unreadable && Number.isSafeInteger(box.not_after_ms))
    .sort((a, b) => a[1].not_after_ms - b[1].not_after_ms);
  const soonest = slots[0] || null;
  const rkpBox = (soonest && soonest[1].rkp) ? soonest[1] : (tee.rkp ? tee : (sb.rkp ? sb : null));
  setText('om-rkp-kind', rkpBox
    ? 'RKP' + (rkpBox.level ? ' · ' + rkpBox.level : '')
    : text('非 RKP 证书','Not an RKP chain'));
  const expiry = soonest ? soonest[1].not_after_ms : null;
  setText('om-rkp-expiry', expiry
    ? (soonest[1].level || soonest[0]) + ' ' + formatDay(expiry) + ' · ' + formatKeyboxDays(Math.floor((expiry - Date.now()) / 86400000))
    : '—');
  setText('om-rkp-slots', slots.length
    ? text('两个槽位：','Slots: ') + slots.map(([label, box]) =>
        (box.level || label) + ' ' + formatKeyboxDays(Number.isSafeInteger(box.days) ? box.days : 0)).join(' · ')
      + text('；按最早到期者判断临期。',' — the earliest expiry drives the deadline.')
    : text('未安装 RKP 证书。','No RKP chain installed.'));
  setText('om-rkp-policy', (policy.refresh_hours || 0) + ' ' + text('小时','hours') + ' · ' + text('提前量','lead time'));
  setText('om-rkp-due', Number.isSafeInteger(policy.due_at_ms) && policy.due_at_ms > 0
    ? formatDay(policy.due_at_ms) + (policy.due ? ' · ' + text('已临期','due now') : '')
    : '—');
  const auto = document.getElementById('om-rkp-auto');
  if (auto) {
    auto.checked = policy.auto === true;
    auto.disabled = !(seed.present && seed.decryptable !== false && rkpBox && policy.refresh_hours);
  }
  // A manual issuance only needs usable material: it replaces whatever the slot
  // holds, including a static keybox. Automatic renewal needs a chain with a
  // known expiry, so that switch stays tied to an RKP chain.
  const issueable = Boolean(seed.present && seed.decryptable !== false);
  for (const id of ['om-rkp-issue-tee', 'om-rkp-issue-sb']) {
    const button = document.getElementById(id);
    if (button) button.disabled = !issueable;
  }
  if (status.last_feedback && status.last_feedback.message) {
    automationMessage('om-rkp-issue-state', status.last_feedback.message, true);
  }
  return true;
}
let automationIssueArm = null;
function armAutomationIssue(level) {
  const warning = document.getElementById('om-rkp-issue-warning');
  const buttons = { tee: document.getElementById('om-rkp-issue-tee'), strongbox: document.getElementById('om-rkp-issue-sb') };
  const button = buttons[level];
  if (!button) return;
  if (automationIssueArm === level) {
    automationIssueArm = null;
    if (warning) warning.hidden = true;
    button.dataset.armed = 'false';
    if (button.dataset.label) button.textContent = button.dataset.label;
    void runAutomationIssue(level);
    return;
  }
  automationIssueArm = level;
  if (warning) warning.hidden = false;
  button.dataset.armed = 'true';
  button.dataset.label = button.textContent;
  button.textContent = text('再次点击确认签发', 'Click again to issue');
  automationMessage('om-rkp-issue-state', text('已选择 ', 'Selected ')
    + (level === 'strongbox' ? 'StrongBox' : 'TEE')
    + text('，15 秒内再点一次才会真正签发。', '; the next click within 15 seconds issues.'), false);
  window.setTimeout(() => {
    if (automationIssueArm !== level) return;
    automationIssueArm = null;
    if (warning) warning.hidden = true;
    button.dataset.armed = 'false';
    if (button.dataset.label) button.textContent = button.dataset.label;
    automationMessage('om-rkp-issue-state', text('已取消，未签发。', 'Cancelled; nothing was issued.'), false);
  }, 15000);
}
async function runAutomationIssue(level) {
  automationMessage('om-rkp-issue-state', text('正在签发 ', 'Issuing ')
    + (level === 'strongbox' ? 'StrongBox' : 'TEE') + text('…', '…'), false);
  try {
    const result = await rkpCommand('issue ' + level);
    const summary = result.chain_summary && Number.isSafeInteger(result.chain_summary.certificates)
      ? ' · ' + result.chain_summary.certificates + text(' 张证书', ' certificates') : '';
    automationMessage('om-rkp-issue-state', text('已签发并替换 ', 'Issued and installed ')
      + (result.level === 'strongbox' ? 'StrongBox' : 'TEE') + summary, false);
  } catch (error) {
    automationMessage('om-rkp-issue-state', String(error.message || error), true);
  }
  void renderAutomation();
}
function bindAutomation() {
  const page = document.getElementById('om-automation-page');
  if (!page || page.dataset.omBound === 'true') return;
  page.dataset.omBound = 'true';
  const importSeed = document.getElementById('om-rkp-seed-import');
  if (importSeed) importSeed.onclick = async () => {
    const path = document.getElementById('om-rkp-seed-path');
    automationMessage('om-rkp-seed-message', text('正在导入…','Importing…'), false);
    try {
      const result = await rkpCommand('seed-import ' + shellQuote(path ? path.value : ''));
      automationMessage('om-rkp-seed-message', text('已导入，指纹 ','Imported, fingerprint ') + (result.fingerprint || '—'), false);
    } catch (error) {
      automationMessage('om-rkp-seed-message', String(error.message || error), true);
    }
    void renderAutomation();
  };
  const removeSeed = document.getElementById('om-rkp-seed-remove');
  if (removeSeed) removeSeed.onclick = async () => {
    try {
      await rkpCommand('seed-remove');
      automationMessage('om-rkp-seed-message', text('种子已删除','Seed removed'), false);
    } catch (error) {
      automationMessage('om-rkp-seed-message', String(error.message || error), true);
    }
    void renderAutomation();
  };
  const importKeybox = async (level, pathId, message) => {
    const path = document.getElementById(pathId);
    automationMessage(message, text('正在导入…','Importing…'), false);
    try {
      await rkpCommand('keybox-import ' + level + ' ' + shellQuote(path ? path.value : ''));
      automationMessage(message, text('已导入，守护进程会立即重载','Imported; the daemon reloads it now'), false);
    } catch (error) {
      automationMessage(message, String(error.message || error), true);
    }
    void renderAutomation();
  };
  const teeImport = document.getElementById('om-rkp-tee-import');
  if (teeImport) teeImport.onclick = () => importKeybox('tee', 'om-rkp-tee-path', 'om-rkp-keybox-message');
  const sbImport = document.getElementById('om-rkp-sb-import');
  if (sbImport) sbImport.onclick = () => importKeybox('strongbox', 'om-rkp-sb-path', 'om-rkp-keybox-message');
  const auto = document.getElementById('om-rkp-auto');
  if (auto) auto.onchange = async () => {
    try {
      await rkpCommand('policy --auto ' + (auto.checked ? 'on' : 'off'));
    } catch (error) {
      automationMessage('om-rkp-issue-state', String(error.message || error), true);
    }
    void renderAutomation();
  };
  const issueTee = document.getElementById('om-rkp-issue-tee');
  if (issueTee) issueTee.onclick = () => armAutomationIssue('tee');
  const issueSb = document.getElementById('om-rkp-issue-sb');
  if (issueSb) issueSb.onclick = () => armAutomationIssue('strongbox');
}
const exec = createBridge('a_dashboard');
const execBStatus = createBridge('a_b_status');
if (typeof document !== 'undefined') document.addEventListener('ommega-readonly', event => {
  if (typeof event.detail?.command === 'string') event.detail.promise = exec(event.detail.command);
});
let bOnline = 'unknown', lastVerdict = null;
function renderBConnection() {
  const badge = document.getElementById('om-b-connection');
  if (!badge) return;
  const failed = Boolean(lastVerdict?.failed);
  const state = bOnline === 'online' ? text('已连接','Connected') : bOnline === 'offline' ? text('未连接','Disconnected') : text('连接状态未知','Connection unknown');
  badge.textContent = failed ? `${state} · ${text('最近请求失败','Last request failed')}` : state;
  const tone = failed || bOnline === 'offline' ? 'error' : bOnline === 'online' ? '' : 'unknown';
  badge.className = 'badge' + (tone ? ' ' + tone : '');
  badge.dataset.tone = failed || bOnline === 'offline' ? 'error' : bOnline === 'online' ? 'ok' : 'unknown';
  badge.title = text('根据 Server 最近收到的 B 端轮询判断。','Based on the server’s recent B-device polling.');
}
async function readBConnection() {
  const configCommand = `ommega_status_config=/data/adb/ommega/ommegadata/config
[ -r "$ommega_status_config" ] || exit 1
grep -iE '^[[:space:]]*(url|device_id|remote|tls_insecure|tls_skip_verify|insecure_tls)[[:space:]]*:' "$ommega_status_config"`;
  const config = await execBStatus(configCommand);
  if (Number(config.errno) !== 0) {
    // The config could not be read at all: that is a failed read.
    bOnline = 'unknown'; renderBConnection(); renderBBattery(null); return false;
  }
  const settings = parseBStatusConfig(config.stdout);
  if (!settings) {
    // Readable but without an active relay (local mode): there is nothing to
    // read here, so this must not be reported as an incomplete refresh.
    bOnline = 'unknown'; renderBConnection(); renderBBattery(null); return true;
  }
  const bytes = new TextEncoder().encode(settings.url + '/api/status/');
  let binary = ''; for (const byte of bytes) binary += String.fromCharCode(byte);
  const encoded = btoa(binary);
  const output = await execBStatus(`ommega_status_url=$(printf '%s' '${encoded}' | base64 -d)
if command -v curl >/dev/null 2>&1; then
  curl --silent --show-error --fail --connect-timeout 3 --max-time 5 ${settings.insecure ? '--insecure' : ''} -- "$ommega_status_url" | head -c 262145
elif command -v busybox >/dev/null 2>&1; then
  busybox wget -q -O - -T 5 ${settings.insecure ? '--no-check-certificate' : ''} "$ommega_status_url" | head -c 262145
else exit 1; fi`);
  const current = await execBStatus(configCommand);
  const currentSettings = !current.errno && parseBStatusConfig(current.stdout);
  const valid = !output.errno && output.stdout.length <= 262144 && JSON.stringify(currentSettings) === JSON.stringify(settings);
  bOnline = valid ? bOnlineState(output.stdout, settings.deviceId) : 'unknown';
  renderBConnection();
  renderBBattery(valid ? bBatterySnapshot(output.stdout, settings.deviceId) : null);
  return bOnline !== 'unknown';
}
function action(id, zh, en, detailZh, detailEn, image = 'chevron') {
  return `<button type="button" class="action-row" data-om-action="${id}"><span class="row-icon">${icon(image)}</span><span class="row-text"><strong>${text(zh,en)}</strong><small>${text(detailZh,detailEn)}</small></span>${icon('chevron')}</button>`;
}
function card(image, title, valueId, description, descriptionId = '') {
  return `<article class="panel compact-card"><div class="card-meta"><div class="panel-label">${icon(image)}<span>${title}</span></div><span class="tiny-icon">${icon(image)}</span></div><div><h2 id="${valueId}">—</h2><p${descriptionId ? ` id="${descriptionId}"` : ''}>${description}</p></div></article>`;
}
function adaptNativeDialogs() {
  for (const dialog of document.querySelectorAll('dialog')) {
    if (dialog.querySelector(':scope > .sheet-handle')) continue;
    const handle = document.createElement('div');
    handle.className = 'sheet-handle';
    handle.setAttribute('aria-hidden', 'true');
    dialog.prepend(handle);
  }
}

function adaptLegacyDialogs() {
  for (const dialog of document.querySelectorAll('md-dialog')) {
    const root = dialog.shadowRoot;
    if (!root) continue;
    const style = document.createElement('style');
    style.dataset.omSheet = 'true';
    style.textContent = `
      .scrim { background: rgb(24 25 35 / 48%); }
      .headline { position: relative; padding: 20px 20px 8px; }
      .headline::before { content: ''; display: block; width: 34px; height: 4px; border-radius: 99px; background: var(--border, #dcdde8); margin: 0 auto 16px; }
      .headline h2 { margin: 0; padding-right: 48px; font-size: 21px; letter-spacing: -.035em; }
      .om-sheet-close { position: absolute; right: 16px; top: 18px; width: 40px; height: 40px; display: grid; place-items: center; border: 0; border-radius: 50%; background: var(--surface-soft, #eeeff7); color: var(--text, #1b1b25); cursor: pointer; }
      .om-sheet-close:focus { outline: none; }
      .om-sheet-close:focus-visible { outline: 3px solid var(--primary, #485fae); outline-offset: 2px; }
      .om-sheet-close svg { width: 20px; height: 20px; fill: none; stroke: currentColor; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }
      .headline md-divider, .actions md-divider { display: none; }
      .content { padding: 0 20px; }
      .scroller { scrollbar-width: none !important; -ms-overflow-style: none; }
      .scroller::-webkit-scrollbar { width: 0 !important; height: 0 !important; display: none !important; }
      .actions { display: flex; justify-content: flex-end; gap: 8px; padding: 14px 20px calc(20px + env(safe-area-inset-bottom, 0px)); }
      @media (min-width: 761px) { .headline::before { display: none; } .headline { padding-top: 22px; } }
      @media (max-width: 760px) {
        dialog { position: fixed !important; inset: auto 0 0 0 !important; margin: 0 !important; width: 100% !important; max-width: none !important; max-height: 88dvh !important; border-radius: 26px 26px 0 0 !important; }
      }`;
    if (!root.querySelector('style[data-om-sheet]')) root.append(style);
    const headline = root.querySelector('.headline');
    if (headline && !headline.querySelector('.om-sheet-close')) {
      const close = document.createElement('button');
      close.type = 'button';
      close.className = 'om-sheet-close';
      close.setAttribute('aria-label', text('关闭','Close'));
      close.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M18 6 6 18"/></svg>';
      close.addEventListener('click', () => dialog.close());
      headline.append(close);
    }
  }
}

function adaptRemoteDialog() {
  const content = document.querySelector('#remote-config-dialog .remote-config-content');
  const options = content?.querySelector('.remote-config-options');
  if (!content || !options || options.dataset.omAdapted) return;
  options.dataset.omAdapted = 'true';
  const advanced = document.createElement('details');
  advanced.className = 'om-remote-advanced';
  const summary = document.createElement('summary');
  summary.textContent = text('高级选项','Advanced options');
  advanced.append(summary);
  options.append(advanced);
  const groupOptions = () => {
    for (const label of [...options.children]) {
      if (!label.matches('label.config-option')) continue;
      if (label.querySelector('#remote-config-remote')) label.classList.add('om-remote-enable');
      else advanced.append(label);
    }
  };
  groupOptions();
  // The legacy bundle can append the StrongBox option after this module runs.
  new MutationObserver(groupOptions).observe(options, {childList:true});

  const token = document.getElementById('remote-config-token');
  if (!token) return;
  const tokenRow = document.createElement('div'); tokenRow.className = 'om-remote-token';
  token.before(tokenRow); tokenRow.append(token);
  const reveal = document.createElement('button'); reveal.type = 'button'; reveal.className = 'om-button secondary';
  tokenRow.append(reveal);
  const setVisible = visible => {
    token.type = visible ? 'text' : 'password';
    token.setAttribute('type', token.type);
    reveal.textContent = visible ? text('隐藏','Hide') : text('显示','Show');
    reveal.setAttribute('aria-label', visible ? text('隐藏 Token','Hide token') : text('显示 Token','Show token'));
    reveal.setAttribute('aria-pressed',String(visible));
  };
  reveal.addEventListener('click', () => setVisible(token.type === 'password'));
  document.getElementById('remote-config')?.addEventListener('click', () => setVisible(false));
  setVisible(false);
}
function install() {
  applyAppearance();
  document.body.classList.add('om-modern');
  const shell = document.createElement('div');
  shell.id = 'om-shell';
  shell.innerHTML = `
    <div class="om-rail-brand"><span class="om-logo">${icon('shield')}</span><div><div class="om-title">Ommega <span class="om-caption">A</span></div><small>${text('请求端','Client')}</small></div></div>
    <header class="om-topbar"><div class="om-brand"><span class="om-logo">${icon('shield')}</span><div><div class="om-title">Ommega <span class="om-caption">A</span></div><small>${text('请求端','Client')}</small></div></div><div class="om-top-actions"><button type="button" class="om-button secondary om-icon-button" id="om-theme-toggle" aria-label="${text('切换深色主题','Switch to dark theme')}" title="${text('切换主题','Switch theme')}">${icon('moon')}</button><button type="button" class="om-button secondary om-icon-button" id="om-refresh" aria-label="${text('刷新状态','Refresh status')}">${icon('refresh')}</button></div></header>
    <main class="om-content" id="om-main" tabindex="-1">
      <section id="om-overview" class="om-view" aria-label="${text('概览','Overview')}">
        <div class="page-heading"><div><span class="section-kicker">OVERVIEW</span><h1>${text('概览','Overview')}</h1><p>${text('查看请求端最近的运行记录。','View recent client activity.')}</p></div></div>
        <div class="dashboard-grid" data-dashboard-side="a">
          <article class="panel status-panel">
            <div class="status-head"><div class="panel-label">${icon('clock')}<span id="om-verdict-label">${text('最近请求','Last request')}</span></div><span class="badge unknown" id="om-relay-state">${text('读取状态中','Reading status')}</span></div>
            <div class="status-body"><div><h2 id="om-verdict-title">${text('证明链路','Attestation relay')}</h2><p id="om-health-detail">—</p></div><span class="status-emblem unknown" id="om-request-icon">${requestIcon('unknown')}</span></div>
            <div class="status-footer"><div><small id="om-health-time-label">${text('记录时间','Recorded')}</small><strong id="om-health-time">—</strong></div><div><small id="om-health-kind-label">${text('请求类型','Request')}</small><strong id="om-request-kind">KeyMint</strong></div></div>
            <p class="note" id="om-health-message"></p>
          </article>
          <article class="panel metric-card b-summary" id="om-b-summary">
            <div class="b-summary-header"><div class="b-summary-title"><div class="panel-label">${icon('phone')}<span>${text('B 端概览','B device')}</span></div><h2 id="om-b-model">—</h2></div><div class="b-summary-statuses"><span class="badge unknown" id="om-b-connection">${text('连接状态未知','Connection unknown')}</span><span class="badge unknown latency-badge" id="om-b-latency">${text('最近延迟','Last latency')} · —</span></div></div>
            <dl class="b-summary-details"><div><dt>${text('品牌 / 厂商','Brand / manufacturer')}</dt><dd id="om-b-brand">—</dd></div><div><dt>${text('设备代号','Device code')}</dt><dd id="om-b-device">—</dd></div><div><dt>${text('产品代号','Product code')}</dt><dd id="om-b-product">—</dd></div><div><dt>CPU / SoC</dt><dd id="om-b-soc">—</dd></div><div><dt>${text('已配置设备 ID','Configured device ID')}</dt><dd id="om-b-relay-id">—</dd></div></dl>
            <div class="battery-summary">${icon('battery')}<span>${text('电量','Battery')}</span><strong id="om-b-battery">${text('暂无新数据','No recent data')}</strong></div>
            <p class="note">${text('设备信息来自本地保存的 B 端属性快照。','Device details come from the locally saved B property snapshot.')}</p>
          </article>
          <article class="panel metric-card b-summary" id="om-local-summary" hidden>
            <div class="b-summary-header"><div class="b-summary-title"><div class="panel-label">${icon('shield')}<span>${text('本机证明材料','Local material')}</span></div><h2>${text('本地模式','Local mode')}</h2></div><div class="b-summary-statuses"><span class="badge neutral" id="om-local-relay">${text('中继未启用','Relay disabled')}</span><span class="badge unknown latency-badge" id="om-local-expiry">${text('最近到期','Next expiry')} · —</span></div></div>
            <dl class="b-summary-details"><div><dt>TEE</dt><dd id="om-local-tee">—</dd></div><div><dt>StrongBox</dt><dd id="om-local-strongbox">—</dd></div><div><dt>${text('最近签发','Last issuance')}</dt><dd id="om-local-last">—</dd></div><div><dt>${text('累计证明','Attestations')}</dt><dd id="om-local-attest">—</dd></div><div><dt>${text('应用作用域','App scope')}</dt><dd id="om-local-scope">—</dd></div><div><dt>${text('中继','Relay')}</dt><dd id="om-local-mode">—</dd></div></dl>
            <div class="metric-chips om-local-warnings" id="om-local-warnings" hidden></div>
            <p class="note">${text('本机软件 TA 用自己的 keybox 直接签发证明，材料不经过网络。','This device signs attestations with its own software TA and keybox; no material leaves the device.')}</p>
          </article>
          ${card('cpu','KEYMINT','om-keymint',text('本机守护进程','Local daemon'))}
          ${card('key','SOTER','om-soter',text('软件 TA 状态','Software TA status'),'om-soter-detail')}
          ${card('apps',text('应用范围','App scope'),'om-scope-count',text('已保存的单独作用域','Saved individual scope'))}
          ${card('file',text('模块版本','MODULE VERSION'),'om-version',text('当前安装','Installed build'))}
          <article class="panel wide-card"><div class="card-heading"><h2>${text('证明路径','Attestation path')}</h2><small>${text('示意图','Diagram')}</small></div><div class="route" id="om-route" aria-label="A → Server → B"><div class="route-node"><strong>A</strong><span>${text('请求端','Client')}</span></div><span class="route-arrow" aria-hidden="true">${icon('arrow')}</span><div class="route-node"><strong id="om-route-mid">Server</strong><span id="om-route-mid-detail">${text('转发','Relay')}</span></div><span class="route-arrow" aria-hidden="true">${icon('arrow')}</span><div class="route-node"><strong id="om-route-end">B</strong><span id="om-route-end-detail">${text('硬件 TEE','Hardware TEE')}</span></div></div><p class="dashboard-note">${text('路径仅展示组成，不表示各节点当前的连接状态。','The diagram does not report current connection status.')}</p></article>
        </div>
      </section>
      <section id="om-apps-heading" class="om-view" hidden><div class="om-page-heading"><span class="om-eyebrow">APPLICATIONS</span><h1>${text('应用','Apps')}</h1><p>${text('管理应用作用域与策略。','Manage app scope and policies.')}</p></div></section>
      <section id="om-settings" class="om-view" hidden><div class="page-heading"><div><span class="section-kicker">PREFERENCES</span><h1>${text('设置','Settings')}</h1><p>${text('管理外观、证明链路与本机策略。','Manage appearance, relay and local policies.')}</p></div></div>
        <div class="content-columns"><div class="stack">
        <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">${text('常用设置','General')}</h2><p>${text('界面语言与外观','Interface language and theme')}</p></div>
        <button type="button" class="action-row" id="om-appearance-open"><span class="row-icon">${icon('palette')}</span><span class="row-text"><strong>${text('语言与外观','Language and appearance')}</strong><small>${text('选择界面语言与配色','Choose the interface language and theme')}</small></span>${icon('chevron')}</button>
        <dialog id="om-appearance-dialog" class="om-preference-dialog" aria-labelledby="om-appearance-title" data-om-return-focus="om-appearance-open"><div class="sheet-handle" aria-hidden="true"></div><div class="sheet-header"><div><span class="section-kicker">A ${text('端设置','SETTINGS')}</span><h2 id="om-appearance-title">${text('语言与外观','Language and appearance')}</h2></div><button type="button" class="icon-button" id="om-appearance-close" aria-label="${text('关闭','Close')}">${icon('close')}</button></div><div class="sheet-body"><p>${text('选择界面语言与配色。','Choose the interface language and theme.')}</p>${appearanceControls()}</div><div class="sheet-actions"><button type="button" id="om-appearance-done" class="primary-button">${text('完成','Done')}</button></div></dialog>
        </section><section class="panel settings-group"><div class="group-header"><h2 class="group-heading">${text('证明与兼容性','Attestation and compatibility')}</h2><p>${text('按需调整设备侧能力','Tune device-side capabilities')}</p></div>
        <div id="om-compat-settings-slot"></div>
        <div id="om-capability-mask-slot"></div>
        ${action('ommega-soterta','SOTER','SOTER','软件 TA 与远程转发','Software TA and remote relay','key')}
        </section></div><div class="stack">
        <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">${text('连接与配置','Connection and configuration')}</h2></div>
        ${action('remote-config','远程连接','Remote connection','服务器、设备 ID 与凭据','Server, device ID and credentials','server')}
        ${action('om-automation-open','自动化','Automation','CDI 种子、Keybox 与 RKP 续签','CDI seed, keybox and RKP renewal','sliders')}
        ${action('boot-hash','启动哈希','Verified boot hash','配置证明参数','Configure attestation parameters','shield')}
        ${action('boot-key','启动公钥','Verified boot key','配置证明公钥','Configure the attestation public key','key')}
        </section><section class="panel settings-group om-hidden-policy-launchers"><div class="group-header"><h2 class="group-heading">${text('应用策略','App policies')}</h2></div>
        <div id="om-auth-signing-slot"></div>
        <div id="om-cpu-mask-slot"></div>
        </section><section class="panel settings-group"><div class="group-header"><h2 class="group-heading">${text('更多','More')}</h2></div>
        ${action('help','使用帮助','Help','配置说明','Configuration guide','help')}
        ${action('about','关于 Ommega','About Ommega','版本与项目说明','Version and project details','info')}
        </section></div></div>
        <p class="om-description" id="om-action-message" role="status"></p>
      </section>
      <section id="om-automation-page" class="om-view" hidden aria-label="${text('自动化','Automation')}">
        <div class="page-heading"><div><span class="section-kicker">AUTOMATION</span><h1>${text('自动化','Automation')}</h1><p>${text('CDI 种子、Keybox 与 RKP 续签。','CDI seed, keybox and RKP renewal.')}</p></div></div>
        <div class="content-columns"><div class="stack">
          <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">${text('CDI 种子','CDI seed')}</h2><p>${text('由你自己导入并加密保存在本机；模块不内置任何种子','Imported by you and stored encrypted on this device; the module ships no seed')}</p></div>
            <div class="om-automation-field wide"><label for="om-rkp-seed-path">${text('种子包路径','Seed bundle path')}</label><input id="om-rkp-seed-path" type="text" value="/sdcard/Download/rkp-seed.json" spellcheck="false" autocomplete="off"><button type="button" id="om-rkp-seed-import" class="om-button secondary">${text('导入','Import')}</button><button type="button" id="om-rkp-seed-remove" class="om-button secondary">${text('删除','Remove')}</button></div>
            <dl class="b-summary-details"><div><dt>${text('状态','State')}</dt><dd id="om-rkp-seed-state">—</dd></div><div><dt>${text('指纹','Fingerprint')}</dt><dd id="om-rkp-seed-fingerprint">—</dd></div><div><dt>${text('导入时间','Imported')}</dt><dd id="om-rkp-seed-time">—</dd></div><div><dt>DICE ${text('链','chain')}</dt><dd id="om-rkp-seed-chain">—</dd></div></dl>
            <p class="note">${text('种子包为 JSON：version、dice_chain_hex（真机 DICE 链的 CBOR hex）、signing（kind 与 bytes_hex）、device、issuance（fingerprint 与 server_url）。','The bundle is JSON with version, dice_chain_hex (the CBOR DICE chain in hex), signing (kind and bytes_hex), device and issuance (fingerprint and server_url).')}</p>
            <p class="note" id="om-rkp-seed-message" role="status"></p>
          </section>
          <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">${text('Keybox 导入','Keybox import')}</h2><p>${text('TEE 与 StrongBox 各自一个槽位，互不复用','TEE and StrongBox keep separate slots and never share material')}</p></div>
            <div class="om-automation-field"><label for="om-rkp-tee-path">TEE keybox</label><input id="om-rkp-tee-path" type="text" value="/sdcard/Download/keybox.xml" spellcheck="false" autocomplete="off"><button type="button" id="om-rkp-tee-import" class="om-button secondary">${text('导入','Import')}</button></div>
            <p class="note" id="om-rkp-tee-state">—</p>
            <div class="om-automation-field"><label for="om-rkp-sb-path">StrongBox keybox</label><input id="om-rkp-sb-path" type="text" value="/sdcard/Download/keybox-strongbox.xml" spellcheck="false" autocomplete="off"><button type="button" id="om-rkp-sb-import" class="om-button secondary">${text('导入','Import')}</button></div>
            <p class="note" id="om-rkp-sb-state">—</p>
            <p class="note" id="om-rkp-keybox-message" role="status"></p>
          </section>
          <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">RKP ${text('续签','renewal')}</h2><p>${text('按服务器下发的 time_to_refresh_hours 判断临期','Renew when the server-provided time_to_refresh_hours says the chain is near expiry')}</p></div>
            <dl class="b-summary-details"><div><dt>${text('链路类型','Chain kind')}</dt><dd id="om-rkp-kind">—</dd></div><div><dt>${text('到期（最早）','Expires (earliest)')}</dt><dd id="om-rkp-expiry">—</dd></div><div><dt>${text('续签策略','Policy')}</dt><dd id="om-rkp-policy">—</dd></div><div><dt>${text('临期时间','Refresh due')}</dt><dd id="om-rkp-due">—</dd></div></dl>
            <p class="note" id="om-rkp-slots">—</p>
            <button type="button" class="action-row" id="om-rkp-auto-row"><span class="row-icon">${icon('refresh')}</span><span class="row-text"><strong>${text('临期自动签发','Automatic renewal')}</strong><small>${text('到期前按策略重新签发并替换 keybox；仅在存在有效种子与本机 RKP 证书时生效','Re-issue and replace the keybox before expiry, only with a valid seed and an RKP chain')}</small></span><span class="switch"><input id="om-rkp-auto" type="checkbox" role="switch"><span></span></span></button>
            <div class="om-automation-actions"><button type="button" id="om-rkp-issue-tee" class="om-button secondary">${text('签发并替换 TEE','Issue and replace TEE')}</button><button type="button" id="om-rkp-issue-sb" class="om-button secondary">${text('签发并替换 StrongBox','Issue and replace StrongBox')}</button></div>
            <p class="note" id="om-rkp-issue-warning" hidden>${text('频繁签发有风险：同一台设备的注册身份被 Google 记录，短时间重复申请可能被标记或限流。确认要现在签发吗？','Frequent issuance is risky: the registration identity is tracked by Google, and repeated requests in a short window can be flagged or throttled. Issue now?')}</p>
            <p class="note" id="om-rkp-issue-state" role="status"></p>
          </section>
        </div></div>
      </section>
      <section id="om-soter-page" class="om-view" hidden aria-label="${text('SOTER 身份','SOTER identities')}"></section>
      <section id="om-soter-identity" class="om-view" hidden aria-label="${text('身份详情','Identity detail')}"></section>
      <p class="om-description" id="om-updated" role="status" aria-live="polite" aria-atomic="true"></p>
      </main>
    <nav class="om-nav" aria-label="${text('主导航','Main navigation')}">${[['overview','概览','Overview'],['apps','应用','Apps'],['settings','设置','Settings']].map(([id,zh,en]) => `<button type="button" data-om-view="${id}" aria-current="${id==='overview'?'page':'false'}">${icon(id)}${text(zh,en)}</button>`).join('')}</nav>`;
  document.body.prepend(shell);
  mountSoter(exec);
  adaptRemoteDialog();
  document.getElementById('om-appearance-open').onclick = () => document.getElementById('om-appearance-dialog').showModal();
  const closeAppearance = () => document.getElementById('om-appearance-dialog').close();
  document.getElementById('om-appearance-close').onclick = closeAppearance;
  document.getElementById('om-appearance-done').onclick = closeAppearance;
  document.getElementById('security-patch-auto-toggle')?.closest('.setting-section')?.setAttribute('hidden', '');
  const scopeSlot = document.createElement('div');
  scopeSlot.id = 'om-global-scope-slot';
  document.querySelector('.header')?.after(scopeSlot);
  const menu = document.querySelector('.header > .menu');
  if (menu) {
    const toolbar = document.createElement('div'); toolbar.id = 'om-app-operations';
    const label = document.createElement('span'); label.textContent = text('应用操作','App actions');
    const menuButton = document.getElementById('menu-button');
    menuButton?.setAttribute('aria-label',text('应用操作','App actions'));
    scopeSlot.after(toolbar); toolbar.append(label, menu);
    const popup = document.getElementById('menu-options');
    let menuClosing = false, openAfterClose = false;
    popup?.addEventListener('closing', () => { menuClosing = true; });
    popup?.addEventListener('closed', () => {
      requestAnimationFrame(() => {
        menuClosing = false;
        if (openAfterClose) { openAfterClose = false; popup.open = true; }
      });
    });
    menuButton?.addEventListener('click', event => {
      // The legacy Material menu finishes closing asynchronously. A second
      // tap during that animation must wait, rather than becoming stuck open
      // with an invisible surface.
      if (!menuClosing) return;
      event.preventDefault(); event.stopImmediatePropagation(); openAfterClose = true;
    }, true);
    document.addEventListener('keydown', event => {
      if (event.key !== 'Escape' || !popup?.open) return;
      menuClosing = true;
      popup.close();
      event.preventDefault();
      menuButton?.focus({preventScroll:true});
    });
  }
  installAppSearch({text});
  const searchRegion = document.getElementById('om-app-search-region');
  for (const [id,zh,en] of [['select-all','全选当前列表','Select visible apps'],['deselect-all','取消全选当前列表','Deselect visible apps'],['refresh','刷新应用列表','Refresh apps']]) {
    const button = document.getElementById(id);
    if (!button?.parentNode) continue;
    button.setAttribute('aria-label',text(zh,en));
    const item = document.createElement('div'); item.className = 'om-app-menu-item';
    button.before(item); item.append(button);
    const caption = document.createElement('small'); caption.textContent = text(zh,en); item.append(caption);
    caption.addEventListener('click', () => { if (!button.disabled) button.click(); });
  }
  const denylist = document.getElementById('select-denylist'); if (denylist) denylist.hidden = true;

  function navigate(raw) {
    const parts = String(raw || '').split('/');
    const known = ['overview','apps','settings','soter','automation'];
    let view = known.includes(parts[0]) ? parts[0] : 'overview';
    if (view === 'soter' && parts[1]) view = 'soter-identity';
    document.body.dataset.omView = view;
    // SOTER and automation live behind "-page" ids, so the target id cannot be
    // derived from the route name alone.
    const activeId = view === 'soter' ? 'om-soter-page' : view === 'automation' ? 'om-automation-page' : 'om-' + view;
    for (const id of ['overview','settings','soter-page','soter-identity','automation-page']) {
      const section = document.getElementById('om-' + id);
      if (section) section.hidden = 'om-' + id !== activeId;
    }
    if (view === 'soter') soterShow('list');
    if (view === 'soter-identity') soterShow('detail', parts[1]);
    if (view === 'automation') { bindAutomation(); void renderAutomation(); }
    document.getElementById('om-apps-heading').hidden = view !== 'apps';
    const navView = (view === 'soter' || view === 'soter-identity' || view === 'automation') ? 'settings' : view;
    document.querySelectorAll('[data-om-view]').forEach(button => button.setAttribute('aria-current', button.dataset.omView === navView ? 'page' : 'false'));
    document.querySelector('.om-nav').dataset.active = String(['overview','apps','settings'].indexOf(navView));
    if (searchRegion) searchRegion.hidden = view !== 'apps';
    // A parsed input never gets an Android text-editor connection; swap in a
    // fresh node before the user taps it, and leave focus to that tap.
    if (view === 'apps') prepareSearchEditor();
    // Keep the user's navigation focus. A route change is not an edit action.
  }
  document.querySelectorAll('[data-om-view]').forEach(button => button.onclick = () => { location.replace('#om-' + button.dataset.omView); });
  window.addEventListener('hashchange', () => { navigate(location.hash.startsWith('#om-') ? location.hash.slice(4) : 'overview'); });
  navigate(location.hash.startsWith('#om-') ? location.hash.slice(4) : 'overview');
  for (const button of document.querySelectorAll('[data-om-action]')) button.onclick = () => {
    if (button.dataset.omAction === 'ommega-soterta') { location.hash = 'om-soter'; return; }
    if (button.dataset.omAction === 'om-automation-open') { location.hash = 'om-automation'; return; }
    const target = document.getElementById(button.dataset.omAction);
    if (target) { target.click(); setText('om-action-message',''); }
    else setText('om-action-message',text('界面尚未就绪，请稍后重试。','The interface is still loading. Try again shortly.'));
  };
  bindAppearance(() => {
    const changes = {dirty:false};
    document.dispatchEvent(new CustomEvent('ommega-app-changes', {detail:changes}));
    return !changes.dirty || window.confirm(text('尚有未保存的应用设置，切换语言会重新加载。继续？','You have unsaved app changes. Changing language reloads the page. Continue?'));
  });
  const themeButton = document.getElementById('om-theme-toggle');
  const preferredTheme = () => {
    try { return localStorage.getItem('ommega.ui.theme') || 'auto'; } catch { return 'auto'; }
  };
  const effectiveTheme = () => preferredTheme() === 'auto'
    ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : preferredTheme();
  const paintTheme = () => {
    const dark = effectiveTheme() === 'dark';
    themeButton.innerHTML = icon(dark ? 'sun' : 'moon');
    const label = dark ? text('切换浅色主题','Switch to light theme') : text('切换深色主题','Switch to dark theme');
    themeButton.setAttribute('aria-label', label); themeButton.title = label;
  };
  themeButton.onclick = () => {
    try { localStorage.setItem('ommega.ui.theme', effectiveTheme() === 'dark' ? 'light' : 'dark'); } catch { /* ephemeral WebView */ }
    applyAppearance(); paintTheme();
  };
  matchMedia('(prefers-color-scheme: dark)').addEventListener?.('change', paintTheme);
  paintTheme();
  const refreshButton = document.getElementById('om-refresh');
  const refresh = pollWhenVisible(refreshAState, 15000, refreshButton);
  refreshButton.onclick = refresh;
  // A dialog covers the page, so the navigation must not look actionable while
  // it is open. The back gesture closes the dialog instead of the view.
  installDialogGuard(open => document.body.classList.toggle('om-dialog-open', open), {includeNative:true});
  adaptLegacyDialogs();
  adaptNativeDialogs();
  customElements.whenDefined('md-dialog').then(adaptLegacyDialogs).catch(() => {});
  document.addEventListener('click', () => requestAnimationFrame(() => { adaptLegacyDialogs(); adaptNativeDialogs(); }), true);
  document.addEventListener('opened', () => { adaptLegacyDialogs(); adaptNativeDialogs(); }, true);
  new MutationObserver(records => {
    for (const record of records) {
      for (const node of record.addedNodes) {
        if (node.nodeType !== 1) continue;
        if (node.localName === 'md-dialog' || node.localName === 'dialog' || node.querySelector?.('md-dialog, dialog')) {
          adaptLegacyDialogs(); adaptNativeDialogs(); return;
        }
      }
    }
  }).observe(document.body, {childList: true, subtree: true});
  window.addEventListener('load', adaptLegacyDialogs);
  window.dispatchEvent(new Event('ommega-settings-ready'));
}
export async function refreshAState(forced = false) {
  setText('om-updated',text('正在刷新…','Refreshing…'));
  const tasks = [readStatus(), readBConnection()];
  if (document.body.dataset.omView === 'automation') tasks.push(renderAutomation());
  if (forced) {
    const detail = {view:document.body.dataset.omView, promise:null};
    document.dispatchEvent(new CustomEvent('ommega-refresh', {detail}));
    if (detail.promise) tasks.push(detail.promise);
  }
  const results = await Promise.allSettled(tasks);
  const errors = results.flatMap(result => result.status === 'rejected'
    ? [String(result.reason?.message || result.reason)] : result.value === false ? [text('部分状态无法读取，请重试。','Some status could not be read. Retry.')] : []);
  if (errors.length) {
    setText('om-updated',`${text('刷新不完整','Refresh incomplete')} · ${[...new Set(errors)].join(' · ')}`);
    return false;
  }
  const notes = results.filter(result => result.status === 'fulfilled' && typeof result.value === 'string' && result.value).map(result => result.value);
  setText('om-updated',`${text('状态读取于','Status read at')} ${new Intl.DateTimeFormat(document.documentElement.lang,{timeStyle:'medium'}).format(new Date())}${notes.length ? ' · ' + notes.join(' · ') : ''}`);
  return true;
}
async function readStatus() {
  const result = await exec(`printf '\\n[HEALTH]\\n'
cat /data/misc/keystore/ommega/remote-health.json 2>/dev/null
printf '\\n[SOTER]\\n'
cat /data/adb/ommega/soterta/status.json 2>/dev/null
printf '\\n[VERSION]\\n'
grep '^version=' /data/adb/modules/ommega/module.prop 2>/dev/null
printf '\\n[DAEMON]\\n'
p=$(cat /data/adb/ommega/keymint-daemon.pid 2>/dev/null)
case "$p" in ''|*[!0-9]*) ;; *) [ -d "/proc/$p" ] && printf '%s' "$p" ;; esac
printf '\\n[PROFILE]\\n'
p=/data/adb/modules/ommega/libs/arm64-v8a/ommega-cpu-policy
[ -x "$p" ] || p=/data/adb/modules/ommega/libs/x86_64/ommega-cpu-policy
[ ! -x "$p" ] || "$p" status 2>/dev/null || true
printf '\\n[REMOTE_DEVICE]\\n'
sed -n 's/^device_id:[[:space:]]*//p' /data/adb/ommega/ommegadata/config 2>/dev/null | head -n 1
printf '\\n[MODE]\\n'
if [ -r /data/adb/ommega/ommegadata/config ]; then grep -iE '^[[:space:]]*(url|device_id|remote|tls_insecure|tls_skip_verify|insecure_tls)[[:space:]]*:' /data/adb/ommega/ommegadata/config 2>/dev/null; else printf 'absent\\n'; fi
printf '\\n[REMOTE]\\n'
grep -A10 -E '^\\[remote\\]' /data/misc/keystore/ommega/config.toml 2>/dev/null | grep -iE '^[[:space:]]*(url|enabled)[[:space:]]*='
printf '\\n[KEYBOX_TEE]\\n'
awk '/<Certificate[ >]/{held=1} held{print} held&&index($0,"</Certificate>"){exit}' /data/misc/keystore/ommega/keybox.xml 2>/dev/null
printf '\\n[KEYBOX_STRONGBOX]\\n'
awk '/<Certificate[ >]/{held=1} held{print} held&&index($0,"</Certificate>"){exit}' /data/misc/keystore/ommega/keybox-strongbox.xml 2>/dev/null
printf '\\n[ACTIVITY]\\n'
grep 'event=generate_total' /data/misc/keystore/ommega/logs/keymint.log 2>/dev/null | tail -n 1
printf '\\n[TARGET]\\n'
if [ -f /data/adb/ommega/ommegadata/target.txt ]; then printf 'present\n'; cat /data/adb/ommega/ommegadata/target.txt; else printf 'missing'; fi
printf '\\n[END]\\n'`);
  result.stdout = result.stdout.replace(/\r\n/g, '\n');
  const markers = new Set(result.stdout.split('\n'));
  const complete = ['HEALTH','SOTER','VERSION','DAEMON','PROFILE','REMOTE_DEVICE','MODE','REMOTE','KEYBOX_TEE','KEYBOX_STRONGBOX','ACTIVITY','TARGET','END'].every(name => markers.has('[' + name + ']'));
  if (result.errno || !complete) {
    for (const id of ['om-keymint','om-soter']) setText(id,text('不可用','Unavailable'));
    const badge = document.getElementById('om-relay-state');
    badge.textContent = text('状态不可用','Status unavailable'); badge.className = 'badge unknown'; badge.dataset.tone = 'unknown';
    const emblem = document.getElementById('om-request-icon'); emblem.className = 'status-emblem unknown'; emblem.dataset.tone = 'unknown'; emblem.innerHTML = requestIcon('unknown');
    lastVerdict = null; renderBConnection();
    const latency = document.getElementById('om-b-latency'); latency.className = 'badge latency-badge unknown'; latency.dataset.tone = 'unknown';
    setText('om-b-latency',`${text('最近延迟','Last latency')} · —`);
    setText('om-health-time','—'); setText('om-health-detail',text('状态读取失败。','Could not read status.')); setText('om-health-message','');
    for (const id of ['om-b-model','om-b-brand','om-b-device','om-b-product','om-b-soc','om-b-relay-id','om-scope-count']) setText(id,'—');
    applyOverviewMode('unknown');
    for (const id of ['om-local-tee','om-local-strongbox','om-local-last','om-local-attest','om-local-scope','om-local-mode']) setText(id,'—');
    throw new Error(result.stderr || text('本机状态读取失败或响应不完整。','Local status read failed or returned an incomplete response.'));
  }
  const section = name => result.stdout.split('['+name+']\n')[1]?.split('\n[')[0]?.trim() || '';
  // Local mode has no relay to report on, so the overview renders the material
  // this device signs with instead of a B-device snapshot.
  const mode = relayMode(section('MODE'), section('REMOTE'));
  applyOverviewMode(mode);
  const profile = parseJson(section('PROFILE'));
  const health = mergeAttestationHealth(parseJson(section('HEALTH')), profile?.attestation_completed_at_ms);
  const soter = parseJson(section('SOTER'));
  // Local mode has no relay request to report, so the card follows the last
  // local attestation instead of a possibly stale relay verdict.
  const verdict = mode === 'local' ? localActivityVerdict(parseLogTime(section('ACTIVITY'))) : relayVerdict(health);
  lastVerdict = verdict;
  const badge = document.getElementById('om-relay-state');
  const tone = verdict.tone === 'ok' ? '' : ' ' + verdict.tone;
  badge.textContent = verdict.label; badge.className = 'badge' + tone; badge.dataset.tone = verdict.tone;
  const emblem = document.getElementById('om-request-icon');
  emblem.className = 'status-emblem' + tone; emblem.dataset.tone = verdict.tone;
  emblem.innerHTML = mode === 'local' ? localVerdictIcon(verdict.tone) : requestIcon(verdict.tone);
  const latency = document.getElementById('om-b-latency');
  renderBConnection();
  latency.className = 'badge latency-badge' + (verdict.failed ? ' error' : verdict.tone === 'unknown' ? ' unknown' : '');
  latency.dataset.tone = verdict.failed ? 'error' : verdict.tone === 'unknown' ? 'unknown' : 'ok';
  setText('om-b-latency',`${text('最近延迟','Last latency')} · ${verdict.failed ? '—' : formatRelayLatency(profile?.last_b_latency_ms)}`);
  const b = deviceSnapshot(profile?.profile);
  for (const [id,value] of Object.entries(b)) setText('om-b-' + id,value);
  setText('om-b-relay-id',section('REMOTE_DEVICE') || '—');
  const targets = section('TARGET');
  const lines = targets.split(/\r?\n/);
  const packages = lines.shift() === 'present' ? new Set(lines.map(line => line.trim().replace(/[!?]+$/, '')).filter(line => /^[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)+$/.test(line))) : null;
  setText('om-scope-count',packages ? String(packages.size) : '—');
  renderLocalSummary({tee:section('KEYBOX_TEE'),strongbox:section('KEYBOX_STRONGBOX'),activity:parseLogTime(section('ACTIVITY')),profile,packages});
  setText('om-keymint', /^\d+$/.test(section('DAEMON')) ? `PID ${section('DAEMON')}` : text('未确认运行','Not confirmed'));
  setText('om-version',section('VERSION').replace(/^version=/,'') || '—');
  const recent = freshness(soter?.updated) === 'recent';
  setText('om-soter',!soter ? text('未上报','Not reported') : !recent ? text('记录已过期','Record is stale') : soter.enabled ? soter.running ? text('软件 TA 运行中','Software TA active') : text('等待软件 TA','Waiting for software TA') : text('原生 HAL','Native HAL'));
  setText('om-soter-detail',soter ? `${soter.ledger ? text('本地账本已就绪','Local ledger ready') : text('本地账本尚未建立','Local ledger not created')} · ${formatTime(soter.updated)}` : text('暂无记录','No record'));
  setText('om-health-time',formatTime(verdict.at));
  // The snapshot is event driven, not a heartbeat. Never infer a live connection from its age.
  const advice = {unauthorized:text('服务器拒绝凭据，请检查 Token。','The server rejected the credentials. Check the token.'),throttled:text('服务器限流或暂时不可用。','The server is busy or rate limiting.'),transport:text('最近一次网络请求失败。','The last network request failed.')};
  setText('om-health-detail',verdict.failed ? `${health?.last_status || '—'} · ${advice[health?.last_kind] || text('请查看诊断详情。','See diagnostic details.')}`
    : mode === 'local' ? text('本机软件 TA 最近一次生成密钥的时间，不代表当前连接心跳。','Time of the most recent local key generation, not a live connection heartbeat.')
    : text('这是最近一次请求的结果，不代表当前连接心跳。','This is the result of the last request, not a live connection heartbeat.'));
  setText('om-health-message',mode === 'local' || !verdict.failed ? '' : healthMessage(health));
  return true;
}
if (typeof document !== 'undefined') install();
