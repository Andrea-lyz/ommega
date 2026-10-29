import {applyAppearance, appearanceControls, bindAppearance, createBridge, formatTime, freshness, healthMessage, icon, installDialogGuard, language, parseJson, pollWhenVisible, setText, text} from './ui-kit.js?v=material-ui-9';
import {installAppSearch,prepareSearchEditor} from './app-search.js?v=app-search-3';

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
  const settings = !config.errno && parseBStatusConfig(config.stdout);
  if (!settings) { bOnline = 'unknown'; renderBConnection(); renderBBattery(null); return false; }
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
            <div class="status-head"><div class="panel-label">${icon('clock')}<span>${text('最近请求','Last request')}</span></div><span class="badge unknown" id="om-relay-state">${text('读取状态中','Reading status')}</span></div>
            <div class="status-body"><div><h2>${text('证明链路','Attestation relay')}</h2><p id="om-health-detail">—</p></div><span class="status-emblem unknown" id="om-request-icon">${requestIcon('unknown')}</span></div>
            <div class="status-footer"><div><small>${text('记录时间','Recorded')}</small><strong id="om-health-time">—</strong></div><div><small>${text('请求类型','Request')}</small><strong>KeyMint</strong></div></div>
            <p class="note" id="om-health-message"></p>
          </article>
          <article class="panel metric-card b-summary">
            <div class="b-summary-header"><div class="b-summary-title"><div class="panel-label">${icon('phone')}<span>${text('B 端概览','B device')}</span></div><h2 id="om-b-model">—</h2></div><div class="b-summary-statuses"><span class="badge unknown" id="om-b-connection">${text('连接状态未知','Connection unknown')}</span><span class="badge unknown latency-badge" id="om-b-latency">${text('最近延迟','Last latency')} · —</span></div></div>
            <dl class="b-summary-details"><div><dt>${text('品牌 / 厂商','Brand / manufacturer')}</dt><dd id="om-b-brand">—</dd></div><div><dt>${text('设备代号','Device code')}</dt><dd id="om-b-device">—</dd></div><div><dt>${text('产品代号','Product code')}</dt><dd id="om-b-product">—</dd></div><div><dt>CPU / SoC</dt><dd id="om-b-soc">—</dd></div><div><dt>${text('已配置设备 ID','Configured device ID')}</dt><dd id="om-b-relay-id">—</dd></div></dl>
            <div class="battery-summary">${icon('battery')}<span>${text('电量','Battery')}</span><strong id="om-b-battery">${text('暂无新数据','No recent data')}</strong></div>
            <p class="note">${text('设备信息来自本地保存的 B 端属性快照。','Device details come from the locally saved B property snapshot.')}</p>
          </article>
          ${card('cpu','KEYMINT','om-keymint',text('本机守护进程','Local daemon'))}
          ${card('key','SOTER','om-soter',text('软件 TA 状态','Software TA status'),'om-soter-detail')}
          ${card('apps',text('应用范围','App scope'),'om-scope-count',text('已保存的单独作用域','Saved individual scope'))}
          ${card('file',text('模块版本','MODULE VERSION'),'om-version',text('当前安装','Installed build'))}
          <article class="panel wide-card"><div class="card-heading"><h2>${text('证明路径','Attestation path')}</h2><small>${text('示意图','Diagram')}</small></div><div class="route" aria-label="A → Server → B"><div class="route-node"><strong>A</strong><span>${text('请求端','Client')}</span></div><span class="route-arrow" aria-hidden="true">${icon('arrow')}</span><div class="route-node"><strong>Server</strong><span>${text('转发','Relay')}</span></div><span class="route-arrow" aria-hidden="true">${icon('arrow')}</span><div class="route-node"><strong>B</strong><span>${text('硬件 TEE','Hardware TEE')}</span></div></div><p class="dashboard-note">${text('路径仅展示组成，不表示各节点当前的连接状态。','The diagram does not report current connection status.')}</p></article>
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
        ${action('automation-settings','自动化','Automation','Keybox 与更新选项','Keybox and update options','sliders')}
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
      <p class="om-description" id="om-updated" role="status" aria-live="polite" aria-atomic="true"></p>
      </main>
    <nav class="om-nav" aria-label="${text('主导航','Main navigation')}">${[['overview','概览','Overview'],['apps','应用','Apps'],['settings','设置','Settings']].map(([id,zh,en]) => `<button type="button" data-om-view="${id}" aria-current="${id==='overview'?'page':'false'}">${icon(id)}${text(zh,en)}</button>`).join('')}</nav>`;
  document.body.prepend(shell);
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
  function navigate(view) {
    if (!['overview','apps','settings'].includes(view)) view = 'overview';
    document.body.dataset.omView = view;
    for (const id of ['overview','settings']) document.getElementById('om-' + id).hidden = view !== id;
    document.getElementById('om-apps-heading').hidden = view !== 'apps';
    document.querySelectorAll('[data-om-view]').forEach(button => button.setAttribute('aria-current', button.dataset.omView === view ? 'page' : 'false'));
    document.querySelector('.om-nav').dataset.active = String(['overview','apps','settings'].indexOf(view));
    if (searchRegion) searchRegion.hidden = view !== 'apps';
    // A parsed input never gets an Android text-editor connection; swap in a
    // fresh node before the user taps it, and leave focus to that tap.
    if (view === 'apps') prepareSearchEditor();
    // Keep the user's navigation focus. A route change is not an edit action.
  }
  document.querySelectorAll('[data-om-view]').forEach(button => button.onclick = () => { location.hash = 'om-' + button.dataset.omView; });
  window.addEventListener('hashchange', () => { navigate(location.hash.startsWith('#om-') ? location.hash.slice(4) : 'overview'); });
  navigate(location.hash.startsWith('#om-') ? location.hash.slice(4) : 'overview');
  for (const button of document.querySelectorAll('[data-om-action]')) button.onclick = () => {
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
printf '\\n[TARGET]\\n'
if [ -f /data/adb/ommega/ommegadata/target.txt ]; then printf 'present\n'; cat /data/adb/ommega/ommegadata/target.txt; else printf 'missing'; fi
printf '\\n[END]\\n'`);
  result.stdout = result.stdout.replace(/\r\n/g, '\n');
  const markers = new Set(result.stdout.split('\n'));
  const complete = ['HEALTH','SOTER','VERSION','DAEMON','PROFILE','REMOTE_DEVICE','TARGET','END'].every(name => markers.has('[' + name + ']'));
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
    throw new Error(result.stderr || text('本机状态读取失败或响应不完整。','Local status read failed or returned an incomplete response.'));
  }
  const section = name => result.stdout.split('['+name+']\n')[1]?.split('\n[')[0]?.trim() || '';
  const profile = parseJson(section('PROFILE'));
  const health = mergeAttestationHealth(parseJson(section('HEALTH')), profile?.attestation_completed_at_ms);
  const soter = parseJson(section('SOTER'));
  const verdict = relayVerdict(health);
  lastVerdict = verdict;
  const badge = document.getElementById('om-relay-state');
  const tone = verdict.tone === 'ok' ? '' : ' ' + verdict.tone;
  badge.textContent = verdict.label; badge.className = 'badge' + tone; badge.dataset.tone = verdict.tone;
  const emblem = document.getElementById('om-request-icon');
  emblem.className = 'status-emblem' + tone; emblem.dataset.tone = verdict.tone; emblem.innerHTML = requestIcon(verdict.tone);
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
  setText('om-keymint', /^\d+$/.test(section('DAEMON')) ? `PID ${section('DAEMON')}` : text('未确认运行','Not confirmed'));
  setText('om-version',section('VERSION').replace(/^version=/,'') || '—');
  const recent = freshness(soter?.updated) === 'recent';
  setText('om-soter',!soter ? text('未上报','Not reported') : !recent ? text('记录已过期','Record is stale') : soter.enabled ? soter.running ? text('软件 TA 运行中','Software TA active') : text('等待软件 TA','Waiting for software TA') : text('原生 HAL','Native HAL'));
  setText('om-soter-detail',soter ? `${soter.ledger ? text('本地账本已就绪','Local ledger ready') : text('本地账本尚未建立','Local ledger not created')} · ${formatTime(soter.updated)}` : text('暂无记录','No record'));
  setText('om-health-time',formatTime(verdict.at));
  // The snapshot is event driven, not a heartbeat. Never infer a live connection from its age.
  const advice = {unauthorized:text('服务器拒绝凭据，请检查 Token。','The server rejected the credentials. Check the token.'),throttled:text('服务器限流或暂时不可用。','The server is busy or rate limiting.'),transport:text('最近一次网络请求失败。','The last network request failed.')};
  setText('om-health-detail',verdict.failed ? `${health?.last_status || '—'} · ${advice[health?.last_kind] || text('请查看诊断详情。','See diagnostic details.')}` : text('这是最近一次请求的结果，不代表当前连接心跳。','This is the result of the last request, not a live connection heartbeat.'));
  setText('om-health-message',verdict.failed ? healthMessage(health) : '');
  return true;
}
if (typeof document !== 'undefined') install();
