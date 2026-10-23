import {applyAppearance, appearanceControls, bindAppearance, createBridge, formatTime, freshness, healthMessage, icon, installDialogGuard, language, parseJson, pollWhenVisible, setText, text} from './ui-kit.js?v=auth-settings-2';

if (typeof document !== 'undefined') {
  const style = document.createElement('link');
  style.rel = 'stylesheet'; style.href = './auth-signing.css?v=auth-settings-2'; document.head.appendChild(style);
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
  const clock = new Intl.DateTimeFormat(language, {hour:'2-digit', minute:'2-digit'}).format(new Date(at * 1000));
  if (failed) return {tone: fresh ? 'error' : 'warn', label:`${fresh ? text('最近请求失败', 'Recent request failed') : text('上次请求失败', 'Last request failed')} · ${clock}`, at, age, failed:true};
  return {tone: fresh ? 'ok' : 'muted', label:`${fresh ? text('最近请求成功', 'Recent request succeeded') : text('上次请求成功', 'Last request succeeded')} · ${clock}`, at, age, failed:false};
}
const exec = createBridge('a_dashboard');
function action(id, zh, en, detailZh, detailEn) {
  return `<button type="button" class="om-action" data-om-action="${id}"><span>${text(zh,en)}<small>${text(detailZh,detailEn)}</small></span>${icon('arrow')}</button>`;
}
function install() {
  applyAppearance();
  document.body.classList.add('om-modern');
  const shell = document.createElement('div');
  shell.id = 'om-shell';
  shell.innerHTML = `
    <header class="om-topbar"><div class="om-brand"><span class="om-logo">${icon('shield')}</span><div><div class="om-title">Ommega <span class="om-caption">A</span></div><div class="om-eyebrow">ATTESTATION / CLIENT</div></div></div><button type="button" class="om-button secondary om-icon-button" id="om-refresh" aria-label="${text('刷新状态','Refresh status')}">${icon('refresh')}</button></header>
    <main class="om-content" id="om-main" tabindex="-1">
      <section id="om-overview" class="om-view" aria-label="${text('概览','Overview')}">
        <div class="om-hero"><span class="om-status" id="om-relay-state">${text('读取状态中','Reading status')}</span><h1>${text('证明链路，一目了然。','Your attestation. In view.')}</h1><p>${text('查看远程证明与 SOTER 的最近状态，管理这台设备的应用范围。','View remote attestation and SOTER status. Manage the apps on this device.')}</p>
          <div class="om-flow" aria-label="A → Server → B"><div class="om-node">A<small>KeyMint · SOTER</small></div><span class="om-connector" aria-hidden="true">${icon('arrow')}</span><div class="om-node">Server<small>${text('转发','Relay')}</small></div><span class="om-connector" aria-hidden="true">${icon('arrow')}</span><div class="om-node">B<small>${text('硬件 TEE','Hardware TEE')}</small></div></div>
        </div>
        <div class="om-grid">
          <article class="om-card"><div class="om-eyebrow">KEYMINT</div><div class="om-stat om-stat-small" id="om-keymint">—</div><p class="om-description">${text('本机守护进程','Local daemon')}</p></article>
          <article class="om-card"><div class="om-eyebrow">SOTER</div><div class="om-stat om-stat-small" id="om-soter">—</div><p class="om-description" id="om-soter-detail">${text('软件 TA 状态','Software TA status')}</p></article>
          <article class="om-card"><div class="om-eyebrow">${text('模块版本','MODULE VERSION')}</div><div class="om-stat om-stat-small" id="om-version">—</div><p class="om-description">${text('当前安装','Installed build')}</p></article>
        </div>
        <article class="om-card"><h2>${text('最近链路记录','Last relay result')}</h2><div id="om-health-time" class="om-caption">—</div><p class="om-description" id="om-health-detail">—</p><p class="om-note" id="om-health-message"></p><p class="om-note">${text('来自守护进程保存的最近结果；读取状态不会发起 TEE 调用。','The last result recorded by the daemon. Reading status does not invoke the TEE.')}</p></article>
        <p id="om-updated" class="om-updated" role="status" aria-live="polite"></p>
      </section>
      <section id="om-settings" class="om-view" hidden><h1 class="om-section-title">${text('设置','Settings')}</h1>
        ${appearanceControls()}
        <div id="om-global-scope-slot"></div>
        <div id="om-auth-signing-slot"></div>
        ${action('remote-config','远程连接','Remote connection','服务器、设备 ID 与凭据','Server, device ID and credentials')}
        ${action('ommega-soterta','SOTER','SOTER','软件 TA 与远程转发','Software TA and remote relay')}
        ${action('automation-settings','自动化','Automation','Keybox 与更新选项','Keybox and update options')}
        ${action('boot-hash','启动哈希','Verified boot hash','配置证明参数','Configure attestation parameters')}
        ${action('boot-key','启动公钥','Verified boot key','配置证明公钥','Configure the attestation public key')}
        ${action('security-patch','安全补丁','Security patch','查看与设置补丁信息','View and configure patch values')}
        ${action('help','使用帮助','Help','配置说明','Configuration guide')}
        ${action('about','关于 Ommega','About Ommega','版本与项目说明','Version and project details')}
        <p class="om-description" id="om-action-message" role="status"></p>
      </section>
    </main>
    <nav class="om-nav" aria-label="${text('主导航','Main navigation')}">${[['overview','概览','Overview'],['apps','应用','Apps'],['settings','设置','Settings']].map(([id,zh,en]) => `<button type="button" data-om-view="${id}" aria-current="${id==='overview'?'page':'false'}">${icon(id)}${text(zh,en)}</button>`).join('')}</nav>`;
  document.body.prepend(shell);
  function navigate(view, focus = false) {
    if (!['overview','apps','settings'].includes(view)) view = 'overview';
    document.body.dataset.omView = view;
    for (const id of ['overview','settings']) document.getElementById('om-' + id).hidden = view !== id;
    document.querySelectorAll('[data-om-view]').forEach(button => button.setAttribute('aria-current', button.dataset.omView === view ? 'page' : 'false'));
    if (focus) (view === 'apps' ? document.getElementById('search-button') : document.getElementById('om-main'))?.focus({preventScroll:true});
  }
  document.querySelectorAll('[data-om-view]').forEach(button => button.onclick = () => { location.hash = 'om-' + button.dataset.omView; });
  window.addEventListener('hashchange', () => { navigate(location.hash.startsWith('#om-') ? location.hash.slice(4) : 'overview', true); });
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
  const refreshButton = document.getElementById('om-refresh');
  const refresh = pollWhenVisible(readStatus, 10000, refreshButton);
  refreshButton.onclick = refresh;
  // A dialog covers the page, so the navigation must not look actionable while
  // it is open. The back gesture closes the dialog instead of the view.
  installDialogGuard(open => document.body.classList.toggle('om-dialog-open', open), {includeNative:true});
  window.dispatchEvent(new Event('ommega-settings-ready'));
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
printf '\\n[END]\\n'`);
  if (result.errno) {
    setText('om-updated',result.stderr);
    for (const id of ['om-keymint','om-soter']) setText(id,text('不可用','Unavailable'));
    const badge = document.getElementById('om-relay-state');
    badge.textContent = text('状态不可用','Status unavailable'); badge.dataset.tone = 'unknown'; return;
  }
  const section = name => result.stdout.split('['+name+']\n')[1]?.split('\n[')[0]?.trim() || '';
  const health = parseJson(section('HEALTH'));
  const soter = parseJson(section('SOTER'));
  const verdict = relayVerdict(health);
  const badge = document.getElementById('om-relay-state');
  badge.textContent = verdict.label; badge.dataset.tone = verdict.tone;
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
  setText('om-updated',`${text('状态读取于','Status read at')} ${new Intl.DateTimeFormat(document.documentElement.lang,{timeStyle:'medium'}).format(new Date())}`);
}
if (typeof document !== 'undefined') install();
