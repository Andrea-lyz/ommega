/* Standalone design preview: all state is in memory and all data is fictional. */
const root = document.documentElement;
const page = document.querySelector('#page');
const sheet = document.querySelector('#sheet');
const sheetTitle = document.querySelector('#sheet-title');
const sheetBody = document.querySelector('#sheet-body');
const sheetActions = document.querySelector('#sheet-actions');
const toast = document.querySelector('#toast');

const makeApp = details => ({
  enabled: false, takeover: 'default', authPolicy: 'auto',
  cpuProperties: false, unnecessary: false, system: false, certificateCalls: 0, ...details
});
const state = {
  side: 'a',
  view: { a: 'overview', b: 'overview' },
  theme: matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  requestStatus: 'ok',
  globalScope: false,
  positiveKeyId: false,
  selectedApp: null,
  appSearch: '',
  appListScrollY: 0,
  masks: { strongbox: false, deviceId: false },
  appearance: { language: 'system', theme: 'system' },
  remote: { enabled: true, server: 'https://relay.example.com:8443', device: 'demo-b', token: 'preview-token', insecureTls: false, debug: false, disableNativeStrongbox: false },
  soter: { software: true, relay: false, server: '', device: '', token: '', uidMap: '', insecureTls: false },
  automation: { enabled: false, source: 'yurikey', interval: '24', customHours: '' },
  boot: { hash: '', key: '' },
  bProperties: { system: '', boot: '', vendor: '', os: '' },
  bApplied: { system: '2026-10-05', boot: '2026-10-05', vendor: '2026-10-05', os: '17' },
  apps: [
    makeApp({ name: '示例钱包', package: 'com.example.wallet', initials: 'W', enabled: true, certificateCalls: 12 }),
    makeApp({ name: '示例银行', package: 'com.example.bank', initials: 'B', enabled: true, certificateCalls: 3 }),
    makeApp({ name: '示例商店', package: 'com.example.store', initials: 'S' }),
    makeApp({ name: '示例应用', package: 'com.example.app', initials: 'A', unnecessary: true })
  ],
  logging: { file: true, logcat: true },
  connection: { server: 'https://relay.example.com:8443', device: 'demo-b', token: 'preview-token', machine: '' }
};
let toastTimer;
let lastSetting;
const systemCandidates = [
  { name: '示例系统服务', package: 'com.example.system', initials: '系', system: true },
  { name: '示例系统商店', package: 'com.example.systemstore', initials: '店', system: true }
];

const icon = name => `<svg class="icon" aria-hidden="true"><use href="#i-${name}"></use></svg>`;
const escapeHtml = value => String(value).replace(/[&<>"']/g, ch => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[ch]);
const requestStates = {
  ok: { label: '最近请求成功', tone: 'ok', icon: 'shield', description: '最近一次请求已成功，并记录在本机。', time: '今天 09:41' },
  error: { label: '最近请求失败', tone: 'error', icon: 'shield-error', description: '最近一次请求失败，请查看链路记录。', time: '今天 09:41' },
  muted: { label: '上次请求成功', tone: 'muted', icon: 'clock', description: '这是较早的成功记录，并非实时连接状态。', time: '昨天 21:16' },
  warn: { label: '上次请求失败', tone: 'warn', icon: 'shield-alert', description: '这是较早的失败记录，不能代表当前状态。', time: '昨天 21:16' },
  unknown: { label: '尚无请求记录', tone: 'unknown', icon: 'info', description: '尚无请求记录，结果将在请求完成后显示。', time: '—' }
};
const heading = (kicker, title, description, trailing = '') => `
  <div class="page-heading"><div><span class="section-kicker">${kicker}</span><h1>${title}</h1><p>${description}</p></div>${trailing}</div>`;
const label = (name, text) => `<div class="panel-label">${icon(name)}<span>${text}</span></div>`;
const card = (name, title, value, description) => `
  <article class="panel compact-card"><div class="card-meta">${label(name, title)}<span class="tiny-icon">${icon(name)}</span></div><div><h2>${value}</h2><p>${description}</p></div></article>`;
const row = (name, title, description, setting) => `
  <button class="action-row" type="button" data-setting="${setting}"><span class="row-icon">${icon(name)}</span><span class="row-text"><strong>${title}</strong><small>${description}</small></span>${icon('chevron')}</button>`;
const toggle = (title, description, key, checked, extra = '') => `
  <div class="toggle-row"><div class="toggle-copy"><strong>${title}</strong><small>${description}</small></div><label class="switch"><input type="checkbox" role="switch" aria-label="${title}" data-switch="${key}" ${checked ? 'checked' : ''} ${extra}><span></span></label></div>`;
const appearanceSummary = () => {
  const languages = { system: '跟随系统', 'zh-CN': '简体中文', en: 'English' };
  const themes = { system: '跟随系统', light: '浅色', dark: '深色' };
  return `${languages[state.appearance.language]} · ${themes[state.appearance.theme]}主题`;
};

function overview() {
  const a = state.side === 'a';
  const selected = state.apps.filter(app => app.enabled).length;
  const request = requestStates[state.requestStatus];
  const requestFailed = request.tone === 'error' || request.tone === 'warn';
  const failureLabel = request.tone === 'warn' ? '上次请求失败' : '最近请求失败';
  const latency = request.tone === 'unknown' ? '—' : '100ms';
  const demoStatus = `<label class="demo-status"><span>预览状态</span><select id="preview-request-status" aria-label="切换最近请求的示例状态">${Object.entries(requestStates).map(([key, value]) => `<option value="${key}" ${key === state.requestStatus ? 'selected' : ''}>${value.label}</option>`).join('')}</select></label>`;
  return `${heading('OVERVIEW', '概览', a ? '查看请求端最近的运行记录。' : '查看设备端的本机状态与连接配置。', a ? demoStatus : undefined)}
    <div class="dashboard-grid" data-dashboard-side="${state.side}">
      <article class="panel status-panel" data-tone="${a ? request.tone : 'ok'}">
        <div class="status-head">${label(a ? 'clock' : 'cpu', a ? '最近请求' : '本机 Relay 进程')}<span class="badge ${a ? request.tone : ''}">${a ? request.label : '进程运行中'}</span></div>
        <div class="status-body"><div><h2>${a ? '证明链路' : '硬件证明设备'}</h2><p>${a ? request.description : '设备端进程正在运行，等待处理任务。'}</p></div><span class="status-emblem ${a ? request.tone : ''}">${icon(a ? request.icon : 'cpu')}</span></div>
        <div class="status-footer"><div><small>${a ? '记录时间' : '本机进程'}</small><strong>${a ? request.time : 'PID 1042'}</strong></div><div><small>${a ? '请求类型' : '设备标识'}</small><strong>${a ? request.tone === 'unknown' ? '—' : 'KeyMint' : 'demo-b'}</strong></div></div>
      </article>
      ${a ? `<article class="panel metric-card b-summary">
        <div class="b-summary-header">
          <div class="b-summary-title">${label('phone','B 端概览')}<h2>示例设备</h2></div>
          <div class="b-summary-statuses">
            <span class="badge ${requestFailed ? 'error' : ''}" aria-label="${requestFailed ? `已连接，示例；${failureLabel}` : '已连接，示例'}" title="${requestFailed ? `${failureLabel}；连接状态为示例` : '连接状态为示例'}">已连接 · 示例</span>
            <span class="badge latency-badge ${requestFailed ? 'error' : request.tone === 'unknown' ? 'neutral' : ''}" title="${requestFailed ? `${failureLabel}；此处显示最近一次完整请求的示例耗时` : '从 A 发出任务到收到 B 回传证书链的示例耗时'}">最近延迟 · ${latency}</span>
          </div>
        </div>
        <dl class="b-summary-details">
          <div><dt>设备型号</dt><dd>DEMO-B1</dd></div>
          <div><dt>品牌 / 厂商</dt><dd>示例品牌</dd></div>
          <div><dt>设备代号</dt><dd>demo_device</dd></div>
          <div><dt>产品代号</dt><dd>demo_product</dd></div>
          <div><dt>主板 / 硬件</dt><dd>demo_board / demo_hw</dd></div>
          <div><dt>设备 ID</dt><dd>demo-b</dd></div>
        </dl>
        <div class="battery-summary">${icon('battery')}<span>电量</span><strong>待接入</strong><span class="battery-placeholder" aria-hidden="true"></span></div>
      </article>` : `<article class="panel metric-card">
        <div>${label('server','服务器连接')}<h2>连接状态未知</h2><p>本机进程状态不能代表服务器已连接。</p></div>
        <div class="metric-chips"><span class="chip">relay.example.com</span><span class="chip">配置示例</span></div>
      </article>`}
      ${a ? card('cpu','KEYMINT','运行中','本机守护进程 · 示例') : card('phone','ANDROID','17','当前系统版本 · 示例')}
      ${a ? card('key','SOTER','软件 TA 运行中','本地账本已就绪 · 示例') : ''}
      ${a ? card('apps','应用范围',`${selected} / ${state.apps.length}`,'已选择的示例应用') : card('shield','SYSTEM SPL','2026-10-05','当前系统属性 · 示例')}
      ${a ? card('file','模块版本','2.0.0','当前安装 · 示例') : card('phone','设备 ID','demo-b','专用设备 · 示例')}
      <article class="panel wide-card"><div class="card-heading"><h2>证明路径</h2><small>示意图</small></div>
        <div class="route"><div class="route-node"><strong>A</strong><span>请求端</span></div><span class="route-arrow">${icon('arrow')}</span><div class="route-node"><strong>Server</strong><span>任务转发</span></div><span class="route-arrow">${icon('arrow')}</span><div class="route-node"><strong>B</strong><span>硬件 TEE</span></div></div>
        <p class="dashboard-note">路径仅展示组成，不表示各节点当前的连接状态。</p>
      </article>
    </div>`;
}

function visibleApps() {
  const term = state.appSearch.trim().toLocaleLowerCase();
  return state.apps.filter(app => `${app.name} ${app.package}`.toLocaleLowerCase().includes(term));
}

function apps() {
  return `${heading('APPLICATIONS', '应用', '为每个应用设置作用域与策略。', '')}
    <div class="apps-page">
      <div class="search-wrap">${icon('search')}<input id="app-search" type="search" value="${escapeHtml(state.appSearch)}" placeholder="搜索应用或包名" aria-label="搜索应用或包名"></div>
      <section class="panel global-scope-card" aria-label="全局作用域">
        ${toggle('全局作用域',state.globalScope ? '已启用 · 单独作用域设置暂不生效' : '关闭 · 按应用单独设置','global',state.globalScope)}
      </section>
      <div class="filter-line">
        <span id="app-count" aria-live="polite"></span>
        <details class="app-menu">
          <summary aria-haspopup="true"><span>应用操作</span>${icon('more')}</summary>
          <div class="app-menu-panel" aria-label="应用操作">
            <button type="button" data-app-action="select-all" aria-label="全选当前列表并启用作用域" ${state.globalScope ? 'disabled title="全局作用域已启用"' : ''}><strong>全选当前列表</strong><small>并启用作用域</small></button>
            <button type="button" data-app-action="deselect-all" aria-label="取消全选当前列表并关闭作用域" ${state.globalScope ? 'disabled title="全局作用域已启用"' : ''}><strong>取消全选当前列表</strong><small>关闭作用域</small></button>
            <button type="button" data-app-action="refresh" aria-label="刷新应用列表"><strong>刷新应用列表</strong></button>
            <button type="button" data-app-action="unnecessary" aria-label="取消非必要应用"><strong>取消非必要应用</strong></button>
            <button type="button" data-app-action="add-system" aria-label="添加系统应用"><strong>添加系统应用</strong></button>
          </div>
        </details>
      </div>
      <div class="panel settings-group" id="app-list"></div>
    </div>`;
}

function renderApps() {
  const list = document.querySelector('#app-list');
  if (!list) return;
  const visible = visibleApps();
  list.innerHTML = visible.length ? visible.map(app => {
    const index = state.apps.indexOf(app);
    const scopeText = state.globalScope ? '全局控制' : app.enabled ? '作用域已启用' : '作用域未启用';
    const calls = Number.isSafeInteger(app.certificateCalls) && app.certificateCalls > 0 ? app.certificateCalls : 0;
    const callText = calls ? `，证书调用 ${calls} 次` : '';
    return `<button class="app-row app-entry" type="button" data-open-app="${index}" aria-label="${escapeHtml(app.name)}${callText}，${scopeText}，打开应用设置"><span class="app-symbol" aria-hidden="true">${escapeHtml(app.initials)}</span><span class="row-text"><span class="app-name-line"><strong>${escapeHtml(app.name)}</strong>${calls ? `<span class="certificate-count">证书 ${calls} 次</span>` : ''}</span><small>${escapeHtml(app.package)}</small></span><span class="app-scope-hint ${app.enabled && !state.globalScope ? 'active' : ''}">${scopeText}</span>${icon('chevron')}</button>`;
  }).join('') : '<div class="empty-state">没有找到匹配的示例应用</div>';
  const selected = state.apps.filter(app => app.enabled).length;
  document.querySelector('#app-count').textContent = state.globalScope ? `已保存 ${selected} / ${state.apps.length} 个单独作用域` : `已启用 ${selected} / ${state.apps.length} 个应用`;
  document.querySelectorAll('[data-app-action="select-all"], [data-app-action="deselect-all"]').forEach(button => {
    button.disabled = state.globalScope || !visible.length;
    button.title = state.globalScope ? '全局作用域已启用' : !visible.length ? '当前列表为空' : '';
  });
}

function policyRow(title, description, property, value, checked, kind = 'checkbox', disabled = false) {
  const radio = kind === 'radio';
  const accessibleLabel = property === 'cpuProperties' ? '启用 CPU 与设备属性' : title;
  return `<label class="policy-row ${disabled ? 'is-disabled' : ''}"><span class="toggle-copy"><strong>${title}</strong><small>${description}</small></span><span class="switch"><input type="${kind}" ${radio ? `name="${property}" value="${value}"` : 'role="switch"'} data-app-policy="${property}" aria-label="${accessibleLabel}" ${checked ? 'checked' : ''} ${disabled ? 'disabled' : ''}><span></span></span></label>`;
}

function policyCard(title, image, rows, note = '') {
  const body = note ? `<div role="radiogroup" aria-label="${title}">${rows}</div>` : rows;
  return `<section class="panel policy-card"><div class="policy-heading"><span class="row-icon">${icon(image)}</span><div><h2>${title}</h2>${note ? `<p>${note}</p>` : ''}</div></div>${body}</section>`;
}

function appDetail(app) {
  const scopeDescription = state.globalScope ? '全局作用域已开启，单独设置暂不生效。' : '控制此应用是否纳入作用域。';
  return `<div class="app-detail">
    <button class="back-button" type="button" data-app-back>${icon('back')}<span>返回应用列表</span></button>
    <div class="app-detail-heading"><span class="app-symbol app-symbol-large" aria-hidden="true">${escapeHtml(app.initials)}</span><div><span class="section-kicker">APPLICATION</span><h1 tabindex="-1">${escapeHtml(app.name)}</h1><p>${escapeHtml(app.package)}</p></div></div>
    <div class="app-policy-stack">
      ${policyCard('作用域','apps',policyRow('启用作用域',scopeDescription,'enabled','',app.enabled,'checkbox',state.globalScope))}
      ${policyCard('接管模式','shield',[
        policyRow('全局默认','跟随全局接管模式。','takeover','default',app.takeover === 'default','radio'),
        policyRow('StrongBox','为此应用选择 StrongBox 模式。','takeover','strongbox',app.takeover === 'strongbox','radio'),
        policyRow('TEE','为此应用选择 TEE 模式。','takeover','tee',app.takeover === 'tee','radio')
      ].join(''),'选择一项')}
      ${policyCard('认证签名策略','lock',[
        policyRow('自动','按默认策略处理认证签名。','authPolicy','auto',app.authPolicy === 'auto','radio'),
        policyRow('支付兼容','在本机验证认证，由远端签名。','authPolicy','compatibility',app.authPolicy === 'compatibility','radio'),
        policyRow('保持认证要求','远端保留认证要求。','authPolicy','preserve',app.authPolicy === 'preserve','radio')
      ].join(''),'选择一项')}
      ${policyCard('CPU 与设备属性','cpu',policyRow('启用','为此应用启用 CPU 与设备属性策略。','cpuProperties','',app.cpuProperties))}
    </div>
  </div>`;
}

function aSettings() {
  const count = Number(state.masks.strongbox) + Number(state.masks.deviceId);
  return `${heading('PREFERENCES', '设置', '管理外观、证明链路与本机策略。')}
    <div class="content-columns"><div class="stack">
      <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">常用设置</h2><p>界面语言与外观</p></div>
        ${row('palette','语言与外观',appearanceSummary(),'appearance')}
      </section>
      <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">证明与兼容性</h2><p>按需调整设备侧能力</p></div>
        ${toggle('正数密钥 ID 兼容','对所有调用者生效','positive-key-id',state.positiveKeyId)}
        ${row('shield','系统能力屏蔽',count ? `已选择 ${count} / 2 项屏蔽` : '未启用屏蔽','capability')}
        ${row('key','SOTER','软件 TA 与远程转发','soter')}
      </section>
    </div><div class="stack">
      <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">连接与配置</h2></div>
        ${row('server','远程连接','服务器、设备 ID 与凭据','remote')}
        ${row('sliders','自动化','Keybox 自动获取与间隔','automation')}
        ${row('shield','启动哈希与公钥','配置证明参数','boot')}
      </section>
      <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">更多</h2></div>
        ${row('help','使用帮助','了解配置和常见问题','help')}
        ${row('info','关于 Ommega','版本与项目说明','about')}
      </section>
    </div></div>`;
}

function connection() {
  const c = state.connection;
  return `${heading('CONNECTION', '连接', '配置 B 端与转发服务器的连接。')}
    <div class="content-columns"><div class="stack">
      <section class="panel form-card"><h2>Relay 配置</h2><p class="note">保存后自动加载；正在处理的任务沿用原连接。</p>
        <label class="field"><span>服务器地址</span><input type="url" inputmode="url" data-connection="server" value="${escapeHtml(c.server)}" spellcheck="false"></label>
        <label class="field"><span>设备 ID</span><input data-connection="device" value="${escapeHtml(c.device)}" spellcheck="false"><small>与 Server 注册的设备 ID 一致。</small></label>
        <label class="field"><span>Token</span><div class="input-action"><input id="token-input" type="password" data-connection="token" value="${escapeHtml(c.token)}" autocomplete="off"><button class="secondary-button" type="button" data-token-toggle>显示</button></div></label>
        <label class="field"><span>机器 ID · 可选</span><input data-connection="machine" value="${escapeHtml(c.machine)}" placeholder="未分配时留空"></label>
        <div class="form-actions"><button class="primary-button" type="button" data-demo-action="演示配置已更新">保存连接配置</button></div>
      </section>
    </div><div class="stack"><section class="panel settings-group"><div class="group-header"><h2 class="group-heading">日志</h2><p>开关和级别在 relay 下次启动时生效。</p></div>
      ${toggle('文件日志','记录至本机日志文件','log-file',state.logging.file)}
      ${toggle('系统日志（logcat）','记录至 Android 系统日志','log-logcat',state.logging.logcat)}
      <div class="form-card"><label class="field"><span>文件日志级别</span><select aria-label="文件日志级别"><option>Debug</option><option>Info</option><option>Warn</option><option>Error</option></select></label>
      <label class="field"><span>系统日志级别</span><select aria-label="系统日志级别"><option>Info</option><option>Debug</option><option>Warn</option><option>Error</option></select></label></div>
    </section><div class="notice"><strong>连接状态</strong>进程运行只说明本机服务已启动，不能单独证明服务器连接成功。</div></div></div>`;
}

function bSettings() {
  return `${heading('PREFERENCES', '设置', '查看外观与设备属性。')}
    <div class="content-columns"><div class="stack">
      <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">通用</h2><p>调整界面显示</p></div>
        ${row('palette','语言与外观',appearanceSummary(),'appearance')}
      </section>
      <section class="panel settings-group"><div class="group-header"><h2 class="group-heading">设备信息</h2><p>本机报告的版本与日期</p></div>
        ${row('shield','设备属性','SPL 与 OS 版本','device-properties')}
        ${row('info','关于 Ommega','版本与项目说明','about')}
      </section>
    </div><div class="stack"><div class="notice">${icon('info')}<strong>设备属性说明</strong>日期与 OS 版本只能升不能降。已上报过的日期不会被收回；恢复出厂日期需要恢复出厂设置。</div>
    <article class="panel form-card"><h2>显示原则</h2><p class="note">页面显示的是设备上报的系统属性；Boot SPL 是否被硬件接受，需要以新生成的证明证书为准。</p></article></div></div>`;
}

function renderNavigation() {
  const entries = state.side === 'a'
    ? [['overview','grid','概览'], ['apps','apps','应用'], ['settings','settings','设置']]
    : [['overview','grid','概览'], ['connection','link','连接'], ['settings','settings','设置']];
  document.querySelectorAll('[data-navigation]').forEach(nav => {
    if (!nav.children.length) {
      if (nav.classList.contains('bottom-navigation')) {
        const indicator = document.createElement('span');
        indicator.className = 'nav-indicator';
        indicator.setAttribute('aria-hidden', 'true');
        nav.append(indicator);
      }
      entries.forEach(() => {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = 'nav-button';
        nav.append(button);
      });
    }
    nav.querySelectorAll('.nav-button').forEach((button, index) => {
      const [view, image, title] = entries[index];
      if (button.dataset.view !== view) {
        button.dataset.view = view;
        button.innerHTML = `<span class="nav-icon">${icon(image)}</span><span>${title}</span>`;
      }
      button.setAttribute('aria-current', state.view[state.side] === view ? 'page' : 'false');
    });
    nav.dataset.active = String(entries.findIndex(([view]) => state.view[state.side] === view));
  });
}

function render() {
  root.dataset.side = state.side;
  root.dataset.theme = state.theme;
  document.querySelectorAll('[data-side-letter]').forEach(el => { el.textContent = state.side.toUpperCase(); });
  document.querySelectorAll('[data-side-subtitle]').forEach(el => { el.textContent = state.side === 'a' ? '请求端' : '设备端'; });
  document.querySelectorAll('[data-side-choice]').forEach(el => { el.setAttribute('aria-pressed', String(el.dataset.sideChoice === state.side)); });
  const themeButton = document.querySelector('#theme-toggle');
  themeButton.setAttribute('aria-label', state.theme === 'dark' ? '切换浅色主题' : '切换深色主题');
  themeButton.innerHTML = icon(state.theme === 'dark' ? 'sun' : 'moon');
  renderNavigation();
  const view = state.view[state.side];
  const app = view === 'apps' && state.selectedApp !== null ? state.apps[state.selectedApp] : null;
  page.innerHTML = view === 'overview' ? overview() : view === 'apps' ? app ? appDetail(app) : apps() : view === 'connection' ? connection() : state.side === 'a' ? aSettings() : bSettings();
  if (view === 'apps' && !app) renderApps();
}

function clearAppDetail() {
  state.selectedApp = null;
  if (history.state?.ommegaPreviewApp !== undefined) {
    const { ommegaPreviewApp, ...rest } = history.state;
    history.replaceState(rest, '', location.href);
  }
}

function openApp(index) {
  if (!state.apps[index]) return;
  state.appListScrollY = window.scrollY;
  state.selectedApp = index;
  history.pushState({ ...(history.state || {}), ommegaPreviewApp: index }, '', location.href);
  render();
  window.scrollTo({ top: 0, behavior: 'instant' });
  page.querySelector('.app-detail-heading h1')?.focus({ preventScroll: true });
}

function returnToApps() {
  if (state.selectedApp === null) return;
  if (history.state?.ommegaPreviewApp === state.selectedApp) {
    history.back();
  } else {
    const previous = state.selectedApp;
    state.selectedApp = null;
    render();
    window.scrollTo({ top: state.appListScrollY, behavior: 'instant' });
    page.querySelector(`[data-open-app="${previous}"]`)?.focus({ preventScroll: true });
  }
}

function openSystemApps() {
  lastSetting = 'app-menu';
  sheetTitle.textContent = '添加系统应用';
  document.querySelector('#sheet-kicker').textContent = '应用操作';
  const available = systemCandidates.filter(candidate => !state.apps.some(app => app.package === candidate.package));
  sheetBody.innerHTML = available.length
    ? `<p>选择要添加到列表的系统应用。</p>${available.map(candidate => `<label class="system-app-choice"><span class="app-symbol" aria-hidden="true">${escapeHtml(candidate.initials)}</span><span class="row-text"><strong>${escapeHtml(candidate.name)}</strong><small>${escapeHtml(candidate.package)}</small></span><input type="checkbox" data-system-package="${escapeHtml(candidate.package)}" aria-label="添加${escapeHtml(candidate.name)}"></label>`).join('')}`
    : '<p>所有示例系统应用已添加到列表。</p>';
  sheetActions.innerHTML = `<button class="secondary-button" type="button" data-close-sheet>取消</button>${available.length ? '<button class="primary-button" type="button" data-add-system-confirm>添加</button>' : ''}`;
  sheet.showModal();
}

function showToast(message) {
  clearTimeout(toastTimer);
  toast.textContent = message;
  toast.classList.add('visible');
  toastTimer = setTimeout(() => toast.classList.remove('visible'), 2600);
}

const sheetChoices = (name, choices, selected) => `<div class="sheet-choice-grid">${choices.map(([value, title]) => `<label><input type="radio" name="${name}" value="${value}" ${selected === value ? 'checked' : ''}><span>${title}</span></label>`).join('')}</div>`;
const sheetSwitch = (key, title, description, checked) => `<label class="sheet-switch-row"><span class="toggle-copy"><strong>${title}</strong><small>${description}</small></span><span class="switch"><input type="checkbox" role="switch" name="${key}" aria-label="${title}" ${checked ? 'checked' : ''}><span></span></span></label>`;
const sheetField = (name, title, value, options = {}) => {
  const type = options.secret ? 'password' : options.type || 'text';
  const hint = options.hint ? `<small>${options.hint}</small>` : '';
  const input = `<input id="sheet-${name}" name="${name}" type="${type}" value="${escapeHtml(value)}" placeholder="${escapeHtml(options.placeholder || '')}" ${options.inputmode ? `inputmode="${options.inputmode}"` : ''} autocomplete="off" spellcheck="false">`;
  return `<label class="field sheet-field" for="sheet-${name}"><span>${title}</span></label><div class="sheet-field-control ${options.secret ? 'input-action' : ''}">${input}${options.secret ? `<button type="button" class="secondary-button reveal-button" data-reveal="sheet-${name}" aria-label="显示${title}" aria-pressed="false">显示</button>` : ''}</div>${hint ? `<div class="sheet-field-hint">${hint}</div>` : ''}`;
};
const sheetSection = (title, body, description = '') => `<section class="sheet-section">${title ? `<div class="sheet-section-heading"><h3>${title}</h3>${description ? `<p>${description}</p>` : ''}</div>` : ''}${body}</section>`;
const sheetActionsFor = (action, label = '保存') => `<button class="secondary-button" type="button" data-close-sheet>取消</button><button class="primary-button" type="button" data-sheet-save="${action}">${label}</button>`;
const sheetText = name => String(sheetBody.querySelector(`[name="${name}"]`)?.value ?? '').trim();
const sheetChecked = name => Boolean(sheetBody.querySelector(`[name="${name}"]`)?.checked);
const sheetChoice = name => sheetBody.querySelector(`[name="${name}"]:checked`)?.value;
function clearSheetError() {
  sheetBody.querySelectorAll('[aria-invalid="true"]').forEach(input => {
    input.removeAttribute('aria-invalid');
    input.removeAttribute('aria-describedby');
  });
  const error = sheetBody.querySelector('.sheet-error');
  if (error) error.hidden = true;
}
function sheetError(message, field) {
  clearSheetError();
  const error = sheetBody.querySelector('.sheet-error');
  if (error) {
    error.id = 'sheet-validation-error';
    error.hidden = false;
    error.textContent = message;
    const control = field?.closest('.sheet-field-control');
    if (control) control.after(error);
  }
  if (field) {
    field.setAttribute('aria-invalid', 'true');
    field.setAttribute('aria-describedby', 'sheet-validation-error');
    field.focus({ preventScroll: true });
    error?.scrollIntoView({ block: 'nearest' });
  }
}
function isHttpUrl(value) {
  try { const parsed = new URL(value); return ['http:', 'https:'].includes(parsed.protocol) && Boolean(parsed.hostname) && !parsed.username && !parsed.password; }
  catch { return false; }
}

function openSetting(key) {
  lastSetting = key;
  const titles = {capability:'系统能力屏蔽',appearance:'语言与外观',soter:'SOTER',remote:'远程连接',automation:'密钥箱自动化',boot:'启动哈希与公钥',help:'使用帮助',about:'关于 Ommega','device-properties':'设备属性'};
  sheetTitle.textContent = titles[key] || '设置';
  document.querySelector('#sheet-kicker').textContent = state.side === 'a' ? 'A 端设置' : 'B 端设置';
  sheetBody.scrollTop = 0;
  if (key === 'capability') {
    sheetBody.innerHTML = `<p class="sheet-intro">选择需要屏蔽的系统能力声明，保存后重启设备生效。</p>
      ${sheetSection('屏蔽项目',sheetSwitch('strongbox','屏蔽 StrongBox','android.hardware.strongbox_keystore',state.masks.strongbox)+sheetSwitch('deviceId','屏蔽设备 ID 证明','android.software.device_id_attestation',state.masks.deviceId))}
      <div class="sheet-tip">${icon('info')}<span>如已启用独立的能力屏蔽模块，请先停用，再使用此处控制。</span></div>`;
    sheetActions.innerHTML = sheetActionsFor('capability');
  } else if (key === 'appearance') {
    sheetBody.innerHTML = `<p class="sheet-intro">选择界面语言与配色。</p>
      ${sheetSection('语言',sheetChoices('language',[['system','跟随系统'],['zh-CN','简体中文'],['en','English']],state.appearance.language))}
      ${sheetSection('外观',sheetChoices('theme',[['system','跟随系统'],['light','浅色'],['dark','深色']],state.appearance.theme))}
      <div class="sheet-tip">${icon('info')}<span>本地原型的语言切换仅展示选项，页面文案暂为中文。</span></div>`;
    sheetActions.innerHTML = sheetActionsFor('appearance');
  } else if (key === 'remote') {
    const cfg = state.remote;
    sheetBody.innerHTML = `<p class="sheet-intro">配置 KeyMint 远程中继的服务器与设备。</p>
      ${sheetSection('',sheetSwitch('enabled','启用远程中继','使用远程服务器处理证明请求。',cfg.enabled))}
      ${sheetSection('连接信息',sheetField('server','服务器地址',cfg.server,{type:'url',inputmode:'url',placeholder:'https://relay.example.com:8443'})+sheetField('device','B 端设备 ID',cfg.device,{placeholder:'设备 ID'})+sheetField('token','Token',cfg.token,{secret:true,placeholder:'连接凭据'}))}
      <details class="sheet-advanced"><summary>高级选项 ${icon('chevron')}</summary><div class="sheet-advanced-content">${sheetSwitch('insecureTls','禁用 TLS 验证','允许使用未受信任的证书。',cfg.insecureTls)}${sheetSwitch('debug','调试日志','记录更详细的运行信息。',cfg.debug)}${sheetSwitch('disableNativeStrongbox','对全局默认应用禁用原生 StrongBox','仅影响使用全局默认模式的应用。',cfg.disableNativeStrongbox)}</div></details>
      <div class="sheet-tip">${icon('clock')}<span>最近请求成功 · 今天 09:41（示例记录）</span></div><p class="sheet-error" role="alert" hidden></p>`;
    sheetActions.innerHTML = sheetActionsFor('remote');
  } else if (key === 'soter') {
    const cfg = state.soter;
    sheetBody.innerHTML = `<div class="sheet-status"><span class="row-icon">${icon('key')}</span><span><strong>${cfg.software ? '软件 TA 运行中' : '软件 TA 未启用'}</strong><small>${cfg.software ? '本地账本已就绪' : '当前使用原生 HAL'} · 示例状态</small></span><span class="badge ${cfg.software ? '' : 'neutral'}">${cfg.software ? '运行中' : '未启用'}</span></div>
      ${sheetSection('',sheetSwitch('software','启用软件 TA','接管原生 SOTER HAL。',cfg.software))}
      ${sheetSection('远程中继',sheetSwitch('relay','启用 SOTER 远程中继','使用独立服务器处理 SOTER 请求。',cfg.relay)+`<div id="soter-relay-fields" ${cfg.relay ? '' : 'hidden'}>`+sheetField('server','服务器地址',cfg.server,{type:'url',inputmode:'url',placeholder:'https://soter.example.com'})+sheetField('device','B 端设备 ID',cfg.device,{placeholder:'设备 ID'})+sheetField('token','Token',cfg.token,{secret:true,placeholder:'连接凭据'})+sheetField('uidMap','UID 映射 · 可选',cfg.uidMap,{placeholder:'A=B，可留空'})+sheetSwitch('insecureTls','允许自签 TLS 证书','使用未受信任的证书。',cfg.insecureTls)+'</div>')}
      <div class="sheet-tip">${icon('info')}<span>启用软件 TA 时，关闭远程中继将使用本地账本；远程请求失败时不会混用本地密钥。</span></div><p class="sheet-error" role="alert" hidden></p>`;
    sheetActions.innerHTML = sheetActionsFor('soter');
  } else if (key === 'automation') {
    const cfg = state.automation;
    sheetBody.innerHTML = `<p class="sheet-intro">设置密钥箱的自动获取方式。</p>
      ${sheetSection('',sheetSwitch('enabled','自动获取密钥箱','按设置的来源和间隔获取。',cfg.enabled))}
      ${sheetSection('获取来源',sheetChoices('source',[['yurikey','Yurikey'],['upstream','KOW'],['unknown','设备生成'],['aosp','AOSP']],cfg.source))}
      ${sheetSection('获取间隔',sheetChoices('interval',[['1','1 小时'],['6','6 小时'],['12','12 小时'],['24','24 小时'],['72','3 天'],['168','7 天'],['custom','自定义']],cfg.interval)+`<div id="custom-interval" ${cfg.interval === 'custom' ? '' : 'hidden'}>`+sheetField('customHours','自定义间隔（小时）',cfg.customHours,{type:'number',inputmode:'numeric',placeholder:'至少 1 小时'})+'</div>')}
      ${sheetSection('手动操作','<button type="button" class="secondary-button sheet-inline-action" data-sheet-demo-action>立即获取密钥箱</button><p class="sheet-action-status" role="status"></p>')}
      <p class="sheet-error" role="alert" hidden></p>`;
    sheetActions.innerHTML = sheetActionsFor('automation');
  } else if (key === 'boot') {
    sheetBody.innerHTML = `<p class="sheet-intro">分别填写证明中使用的启动哈希与公钥。</p>
      ${sheetSection('启动哈希',`<label class="field sheet-field" for="sheet-hash"><span>Verified Boot Hash</span></label><textarea id="sheet-hash" name="hash" rows="3" placeholder="输入或粘贴启动哈希" spellcheck="false">${escapeHtml(state.boot.hash)}</textarea>`)}
      ${sheetSection('启动公钥',`<label class="field sheet-field" for="sheet-key"><span>Verified Boot Key</span></label><textarea id="sheet-key" name="key" rows="3" placeholder="输入或粘贴启动公钥" spellcheck="false">${escapeHtml(state.boot.key)}</textarea>`)}`;
    sheetActions.innerHTML = sheetActionsFor('boot');
  } else if (key === 'device-properties') {
    const cfg = state.bProperties;
    sheetBody.innerHTML = `<p class="sheet-intro">查看设备属性并设置目标值。留空保持当前生效值。</p>
      <div class="notice"><strong>日期与 OS 版本只能升不能降</strong>已上报的日期不会被收回；恢复出厂日期需要恢复出厂设置。</div>
      ${sheetSection('目标值',sheetField('system','System SPL',cfg.system,{placeholder:'YYYY-MM-DD',hint:`示例下限：${state.bApplied.system}`})+sheetField('boot','Boot SPL',cfg.boot,{placeholder:'YYYY-MM-DD',hint:`示例下限：${state.bApplied.boot}`})+sheetField('vendor','Vendor SPL',cfg.vendor,{placeholder:'YYYY-MM-DD',hint:`示例下限：${state.bApplied.vendor}`})+sheetField('os','OS 版本',cfg.os,{placeholder:'留空保持当前值',inputmode:'numeric',hint:`示例下限：${state.bApplied.os}`})+'<div class="sheet-inline-actions"><button type="button" class="secondary-button" data-copy-system>三项使用 System 值</button><button type="button" class="secondary-button" data-clear-properties>留空保持当前值</button></div>')}
      ${sheetSection('当前记录',`<dl class="sheet-facts"><div><dt>原始日期</dt><dd>2026-10-05 · 示例</dd></div><div><dt>System 已应用 / 下限</dt><dd>${state.bApplied.system} · 示例</dd></div><div><dt>Boot 已应用 / 下限</dt><dd>${state.bApplied.boot} · 示例</dd></div><div><dt>Vendor 已应用 / 下限</dt><dd>${state.bApplied.vendor} · 示例</dd></div><div><dt>OS 已应用 / 下限</dt><dd>${state.bApplied.os} · 示例</dd></div></dl>`)}
      <div class="sheet-tip">${icon('info')}<span>修改会重载 KeyMint 与 keystore2；Boot SPL 是否被硬件接受以新生成的证明证书为准。</span></div>
      <div class="sheet-tip">${icon('info')}<span>OS 版本只影响上报的系统版本与证明 OS_VERSION，SDK 等级不变。</span></div><p class="sheet-error" role="alert" hidden></p>`;
    sheetActions.innerHTML = sheetActionsFor('device-properties','保存示例');
  } else if (key === 'help') {
    sheetBody.innerHTML = `<p class="sheet-intro">使用 Ommega 的常用操作。</p>
      ${sheetSection('应用与作用域','<p>在应用页选择作用域，并为每个应用查看接管模式与兼容策略。</p>')}
      ${sheetSection('远程连接','<p>在 A、B 端填写服务器地址、设备 ID 和各自的连接凭据。</p>')}
      ${sheetSection('系统能力屏蔽','<p>选择需要屏蔽的能力后保存，重启设备后生效。</p>')}
      ${sheetSection('SOTER','<p>软件 TA 可使用本地账本，也可配置独立的远程中继。</p>')}`;
    sheetActions.innerHTML = '<button class="primary-button" type="button" data-close-sheet>完成</button>';
  } else if (key === 'about') {
    sheetBody.innerHTML = `<div class="about-hero"><span class="identity-icon">${icon('shield')}</span><strong>Ommega</strong><span>远程认证中继</span></div>
      ${sheetSection('版本信息','<dl class="sheet-facts"><div><dt>模块</dt><dd>Ommega '+state.side.toUpperCase()+' 端</dd></div><div><dt>版本</dt><dd>2.0.0 · 示例</dd></div></dl>')}
      <p class="sheet-about-copy">通过远程中继执行 KeyMint 认证，并管理本机应用与连接配置。</p>`;
    sheetActions.innerHTML = '<button class="primary-button" type="button" data-close-sheet>完成</button>';
  } else {
    return;
  }
  sheet.showModal();
}

function saveSetting(key) {
  if (key === 'capability') {
    state.masks = { strongbox: sheetChecked('strongbox'), deviceId: sheetChecked('deviceId') };
  } else if (key === 'appearance') {
    state.appearance = { language: sheetChoice('language'), theme: sheetChoice('theme') };
    state.theme = state.appearance.theme === 'system'
      ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')
      : state.appearance.theme;
  } else if (key === 'remote' || key === 'soter') {
    const enabled = sheetChecked(key === 'remote' ? 'enabled' : 'relay');
    const server = sheetText('server');
    const device = sheetText('device');
    const token = sheetText('token');
    if (enabled && !isHttpUrl(server)) return sheetError('请输入有效的 HTTP(S) 服务器地址。', sheetBody.querySelector('[name="server"]'));
    if (enabled && !device) return sheetError('请输入 B 端设备 ID。', sheetBody.querySelector('[name="device"]'));
    if (enabled && !token) return sheetError('请输入连接 Token。', sheetBody.querySelector('[name="token"]'));
    if (key === 'remote') state.remote = { enabled, server, device, token, insecureTls: sheetChecked('insecureTls'), debug: sheetChecked('debug'), disableNativeStrongbox: sheetChecked('disableNativeStrongbox') };
    else state.soter = { software: sheetChecked('software'), relay: enabled, server, device, token, uidMap: sheetText('uidMap'), insecureTls: sheetChecked('insecureTls') };
  } else if (key === 'automation') {
    const interval = sheetChoice('interval');
    const customHours = sheetText('customHours');
    if (interval === 'custom' && (!Number.isSafeInteger(Number(customHours)) || Number(customHours) < 1)) {
      return sheetError('自定义间隔需填写大于或等于 1 的整数小时。', sheetBody.querySelector('[name="customHours"]'));
    }
    state.automation = { enabled: sheetChecked('enabled'), source: sheetChoice('source'), interval, customHours };
  } else if (key === 'boot') {
    state.boot = { hash: sheetText('hash'), key: sheetText('key') };
  } else if (key === 'device-properties') {
    const values = Object.fromEntries(['system','boot','vendor','os'].map(name => [name, sheetText(name)]));
    for (const name of ['system','boot','vendor']) {
      if (!values[name]) continue;
      const date = /^\d{4}-\d{2}-\d{2}$/.test(values[name]) ? new Date(values[name] + 'T00:00:00Z') : null;
      if (!date || Number.isNaN(date.getTime()) || date.toISOString().slice(0, 10) !== values[name]) {
        return sheetError('请输入有效的日期，格式为 YYYY-MM-DD。', sheetBody.querySelector(`[name="${name}"]`));
      }
      if (values[name] < state.bApplied[name]) return sheetError(`目标日期不能低于示例下限 ${state.bApplied[name]}。`, sheetBody.querySelector(`[name="${name}"]`));
    }
    if (values.os && (!/^\d+$/.test(values.os) || Number(values.os) < Number(state.bApplied.os))) {
      return sheetError(`OS 版本不能低于示例下限 ${state.bApplied.os}。`, sheetBody.querySelector('[name="os"]'));
    }
    state.bProperties = values;
    for (const name of ['system','boot','vendor','os']) if (values[name]) state.bApplied[name] = values[name];
  } else {
    return;
  }
  sheet.close();
  showToast(key === 'capability' ? '演示设置已更新；实际生效需要重启' : '演示设置已更新');
}

document.addEventListener('click', event => {
  const menu = document.querySelector('.app-menu[open]');
  if (menu && !menu.contains(event.target)) menu.open = false;
  const side = event.target.closest('[data-side-choice]');
  if (side) { clearAppDetail(); state.side = side.dataset.sideChoice; if (sheet.open) sheet.close(); render(); return; }
  const nav = event.target.closest('[data-view]');
  if (nav) { clearAppDetail(); state.view[state.side] = nav.dataset.view; render(); window.scrollTo({ top: 0, behavior: 'instant' }); return; }
  if (event.target.closest('#theme-toggle')) { state.theme = state.theme === 'light' ? 'dark' : 'light'; state.appearance.theme = state.theme; render(); return; }
  if (event.target.closest('#refresh')) { showToast('已刷新示例数据'); return; }
  const appEntry = event.target.closest('[data-open-app]');
  if (appEntry) { openApp(Number(appEntry.dataset.openApp)); return; }
  if (event.target.closest('[data-app-back]')) { returnToApps(); return; }
  const appAction = event.target.closest('[data-app-action]');
  if (appAction) {
    if (appAction.disabled) return;
    const action = appAction.dataset.appAction;
    const current = visibleApps();
    if (menu) menu.open = false;
    if (action === 'select-all' || action === 'deselect-all') {
      if (state.globalScope) return;
      current.forEach(app => { app.enabled = action === 'select-all'; });
      renderApps();
      showToast(action === 'select-all' ? '已启用当前列表的作用域' : '已取消当前列表的作用域');
    } else if (action === 'unnecessary') {
      state.apps.filter(app => app.unnecessary).forEach(app => { app.enabled = false; });
      renderApps();
      showToast(state.globalScope ? '已取消非必要应用；全局作用域仍生效' : '已取消非必要应用');
    } else if (action === 'refresh') {
      renderApps();
      showToast('示例应用列表已刷新');
    } else if (action === 'add-system') {
      openSystemApps();
    }
    return;
  }
  const setting = event.target.closest('[data-setting]');
  if (setting) { openSetting(setting.dataset.setting); return; }
  if (event.target.closest('[data-close-sheet]')) { sheet.close(); return; }
  const settingSave = event.target.closest('[data-sheet-save]');
  if (settingSave) { saveSetting(settingSave.dataset.sheetSave); return; }
  const reveal = event.target.closest('[data-reveal]');
  if (reveal) {
    const input = document.getElementById(reveal.dataset.reveal);
    if (!input) return;
    const visible = input.type === 'password';
    input.type = visible ? 'text' : 'password';
    reveal.textContent = visible ? '隐藏' : '显示';
    reveal.setAttribute('aria-pressed', String(visible));
    reveal.setAttribute('aria-label', visible ? '隐藏 Token' : '显示 Token');
    return;
  }
  if (event.target.closest('[data-sheet-demo-action]')) {
    const status = sheetBody.querySelector('.sheet-action-status');
    if (status) status.textContent = '已触发示例获取';
    return;
  }
  if (event.target.closest('[data-copy-system]')) {
    const value = sheetText('system');
    if (!value) { sheetError('请先填写 System SPL。', sheetBody.querySelector('[name="system"]')); return; }
    for (const name of ['boot','vendor']) sheetBody.querySelector(`[name="${name}"]`).value = value;
    clearSheetError();
    return;
  }
  if (event.target.closest('[data-clear-properties]')) {
    for (const name of ['system','boot','vendor','os']) sheetBody.querySelector(`[name="${name}"]`).value = '';
    clearSheetError();
    return;
  }
  if (event.target.closest('[data-add-system-confirm]')) {
    const selected = [...sheet.querySelectorAll('[data-system-package]:checked')].map(input => input.dataset.systemPackage);
    if (!selected.length) { showToast('请选择要添加的系统应用'); return; }
    selected.forEach(pkg => {
      const candidate = systemCandidates.find(item => item.package === pkg);
      if (candidate && !state.apps.some(app => app.package === pkg)) state.apps.push(makeApp(candidate));
    });
    sheet.close();
    showToast(`已添加 ${selected.length} 个示例系统应用`);
    return;
  }
  const action = event.target.closest('[data-demo-action]');
  if (action) { showToast(action.dataset.demoAction); return; }
  const tokenButton = event.target.closest('[data-token-toggle]');
  if (tokenButton) { const input = document.querySelector('#token-input'); input.type = input.type === 'password' ? 'text' : 'password'; tokenButton.textContent = input.type === 'password' ? '显示' : '隐藏'; }
});

document.addEventListener('change', event => {
  const input = event.target;
  if (sheet.open && sheetBody.contains(input)) {
    if (input.name === 'theme') root.dataset.theme = input.value === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : input.value;
    if (input.name === 'enabled') clearSheetError();
    if (input.name === 'relay') { sheetBody.querySelector('#soter-relay-fields').hidden = !input.checked; clearSheetError(); }
    if (input.name === 'interval') { sheetBody.querySelector('#custom-interval').hidden = input.value !== 'custom'; clearSheetError(); }
    return;
  }
  if (input.id === 'preview-request-status') {
    state.requestStatus = input.value;
    render();
    document.querySelector('#preview-request-status')?.focus({ preventScroll: true });
    return;
  }
  if (input.dataset.appPolicy && state.selectedApp !== null) {
    const app = state.apps[state.selectedApp];
    const key = input.dataset.appPolicy;
    if (key === 'enabled' && state.globalScope) return;
    app[key] = input.type === 'radio' ? input.value : input.checked;
    return;
  }
  if (input.dataset.switch === 'global') {
    state.globalScope = input.checked;
    const position = window.scrollY;
    render();
    window.scrollTo({ top: position, behavior: 'instant' });
    page.querySelector('[data-switch="global"]')?.focus({ preventScroll: true });
    return;
  }
  if (input.dataset.switch === 'positive-key-id') state.positiveKeyId = input.checked;
  if (input.dataset.switch === 'log-file') state.logging.file = input.checked;
  if (input.dataset.switch === 'log-logcat') state.logging.logcat = input.checked;
});
document.addEventListener('input', event => {
  if (sheet.open && sheetBody.contains(event.target) && event.target.getAttribute('aria-invalid') === 'true') {
    clearSheetError();
  }
  if (event.target.id === 'app-search') { state.appSearch = event.target.value; renderApps(); }
  if (event.target.dataset.connection) state.connection[event.target.dataset.connection] = event.target.value;
});
document.addEventListener('keydown', event => {
  if (event.key !== 'Escape' || sheet.open) return;
  const menu = document.querySelector('.app-menu[open]');
  if (menu) { menu.open = false; menu.querySelector('summary')?.focus(); event.preventDefault(); return; }
  if (state.side === 'a' && state.view.a === 'apps' && state.selectedApp !== null) {
    returnToApps(); event.preventDefault();
  }
});
window.addEventListener('popstate', event => {
  const previous = state.selectedApp;
  const index = event.state?.ommegaPreviewApp;
  state.selectedApp = state.side === 'a' && state.view.a === 'apps' && Number.isInteger(index) && state.apps[index] ? index : null;
  render();
  window.scrollTo({ top: state.selectedApp === null ? state.appListScrollY : 0, behavior: 'instant' });
  if (state.selectedApp === null && previous !== null) page.querySelector(`[data-open-app="${previous}"]`)?.focus({ preventScroll: true });
});
sheet.addEventListener('close', () => {
  root.dataset.theme = state.theme;
  render();
  if (lastSetting === 'app-menu') page.querySelector('.app-menu summary')?.focus({ preventScroll: true });
  else page.querySelector(`[data-setting="${lastSetting}"]`)?.focus({ preventScroll: true });
});
render();
