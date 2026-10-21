// Shared, dependency-free presentation helpers. No credentials are persisted here.
export const preferenceKey = 'ommega.ui.language';
export function readPreference(key, fallback) {
  try { return localStorage.getItem(key) || fallback; } catch { return fallback; }
}
export function resolveLanguage(preference, languages = globalThis.navigator?.languages || [globalThis.navigator?.language || 'en']) {
  if (preference === 'zh-CN' || preference === 'en') return preference;
  return /^zh\b/i.test(languages[0] || '') ? 'zh-CN' : 'en';
}
export const language = resolveLanguage(readPreference(preferenceKey, 'auto'));
export const text = (zh, en) => language === 'zh-CN' ? zh : en;
export function applyAppearance() {
  document.documentElement.lang = language;
  document.documentElement.dataset.theme = readPreference('ommega.ui.theme', 'auto');
}
export function bindAppearance(beforeChange = () => true) {
  for (const [id, key, fallback] of [['om-language', preferenceKey, 'auto'], ['om-theme', 'ommega.ui.theme', 'auto']]) {
    const element = document.getElementById(id);
    if (!element) continue;
    element.value = readPreference(key, fallback);
    element.addEventListener('change', () => {
      if (id === 'om-language' && !beforeChange()) { element.value = readPreference(key, fallback); return; }
      try { localStorage.setItem(key, element.value); } catch { /* ephemeral WebView */ }
      if (id === 'om-language') location.reload(); else applyAppearance();
    });
  }
}
export function appearanceControls() {
  return `<div class="om-preferences"><label>${text('语言', 'Language')}<select id="om-language"><option value="auto">${text('跟随系统', 'System')}</option><option value="zh-CN">中文</option><option value="en">English</option></select></label><label>${text('外观', 'Appearance')}<select id="om-theme"><option value="auto">${text('跟随系统', 'System')}</option><option value="light">${text('浅色', 'Light')}</option><option value="dark">${text('深色', 'Dark')}</option></select></label></div>`;
}
export function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, char => ({'&':'&amp;', '<':'&lt;', '>':'&gt;', '"':'&quot;', "'":'&#39;'}[char]));
}
export function parseProperties(raw) {
  return Object.fromEntries(String(raw || '').split(/\r?\n/).filter(row => row.includes('=')).map(row => {
    const at = row.indexOf('='); return [row.slice(0, at).trim(), row.slice(at + 1).trim()];
  }));
}
export function parseJson(raw) { try { return JSON.parse(raw); } catch { return null; } }
export function freshness(seconds, now = Date.now()) {
  const age = now / 1000 - Number(seconds);
  return !Number.isFinite(age) || !seconds || age < -60 ? 'unknown' : age > 90 ? 'stale' : 'recent';
}
export function formatTime(seconds) {
  return Number(seconds) > 1000000000 ? new Intl.DateTimeFormat(language, {dateStyle:'short', timeStyle:'medium'}).format(new Date(seconds * 1000)) : text('暂无记录', 'No record');
}
export function setText(id, value) { const element = document.getElementById(id); if (element) element.textContent = String(value ?? '—'); }
export function icon(name) {
  const paths = {
    overview:'<rect x="3" y="3" width="7" height="7" rx="2"/><rect x="14" y="3" width="7" height="7" rx="2"/><rect x="3" y="14" width="7" height="7" rx="2"/><rect x="14" y="14" width="7" height="7" rx="2"/>',
    apps:'<rect x="4" y="3" width="16" height="18" rx="3"/><path d="M8 8h8M8 12h8M8 16h4"/>',
    settings:'<path d="M4 7h16M4 17h16"/><circle cx="8" cy="7" r="3"/><circle cx="16" cy="17" r="3"/>',
    refresh:'<path d="M3 12a9 9 0 0 1 9-9c2.52 0 4.93 1 6.74 2.74L21 8"/><path d="M16 8h5V3"/><path d="M21 12a9 9 0 0 1-9 9c-2.52 0-4.93-1-6.74-2.74L3 16"/><path d="M8 16H3v5"/>',
    arrow:'<path d="m9 5 7 7-7 7"/>',
    shield:'<path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6l8-3Z"/><path d="m8 12 3 3 5-6"/>',
  };
  return `<svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.shield}</svg>`;
}
export function createBridge(scope) {
  let sequence = 0;
  return command => new Promise(resolve => {
    const callback = `ommega_${scope}_${Date.now()}_${sequence++}`;
    // Read operations may fail to deliver a callback. Never interrupt a device process.
    const timer = setTimeout(() => { delete window[callback]; resolve({errno:1, stdout:'', stderr:text('读取超时，请刷新重试。', 'Read timed out. Refresh to retry.')}); }, 12000);
    window[callback] = (errno, stdout, stderr) => {
      clearTimeout(timer); delete window[callback]; resolve({errno:Number(errno), stdout:stdout || '', stderr:stderr || ''});
    };
    try { window.ksu.exec(command, '{}', callback); }
    catch { clearTimeout(timer); delete window[callback]; resolve({errno:1, stdout:'', stderr:text('请从支持 WebUI 的模块管理器打开。', 'Open from a module manager with WebUI support.')}); }
  });
}
export function pollWhenVisible(refresh, delay = 10000, button = null) {
  let busy = false, stopped = false, queued = false;
  const label = button?.getAttribute('aria-label') || text('刷新状态', 'Refresh status');
  const setBusy = value => {
    if (!button) return;
    button.disabled = value;
    button.setAttribute('aria-busy', String(value));
    const description = value ? text('正在刷新', 'Refreshing') : label;
    button.setAttribute('aria-label', description);
    button.title = description;
  };
  setBusy(false);
  const run = async (forced = false) => {
    if (stopped) return;
    if (busy) {
      // A tap during a running read is queued, never dropped: the panel has to
      // show the result of the request the user actually asked for.
      if (forced) queued = true;
      return;
    }
    if (document.hidden && !forced) return;
    busy = true;
    setBusy(true);
    try { await refresh(); }
    finally {
      busy = false;
      setBusy(false);
      if (queued) { queued = false; run(true); }
    }
  };
  const timer = setInterval(() => run(false), delay);
  const onVisibility = () => run(false);
  document.addEventListener('visibilitychange', onVisibility);
  window.addEventListener('pagehide', () => { stopped = true; clearInterval(timer); document.removeEventListener('visibilitychange', onVisibility); }, {once:true});
  window.addEventListener('pageshow', event => { if (event.persisted) location.reload(); });
  run(false);
  // The returned function is the button action: it always produces a fresh
  // read, and the panel is never left with a button that does nothing.
  return () => run(true);
}

// The relay reports why a request failed as a JSON object. The text inside is
// what an operator needs; the wrapper only adds noise.
export function healthMessage(health) {
  const raw = String(health?.last_message || '').trim();
  if (!raw) return '';
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed.error === 'string' && parsed.error.trim()) return parsed.error.trim();
  } catch { /* not JSON: show the text as it is */ }
  return raw;
}

// The packaged A bundle opens its own dialogs, so the WebView back gesture has
// to be routed to them: a back press closes the topmost dialog instead of
// switching the dashboard view behind it.
export function installDialogGuard(onChange = () => {}) {
  const tracked = new Set();
  const openDialogs = () => [...document.querySelectorAll('md-dialog[open]')];
  const sync = () => {
    const open = openDialogs();
    for (const dialog of open) {
      if (!tracked.has(dialog)) {
        tracked.add(dialog);
        history.pushState({om: 'dialog'}, '');
      }
    }
    for (const dialog of [...tracked]) {
      if (!dialog.open) {
        tracked.delete(dialog);
        // The dialog closed itself: drop the history entry that was added for
        // it, but only while that entry is still the current one. A back press
        // that already consumed it must not take the dashboard view with it.
        if (history.state?.om === 'dialog') history.back();
        break;
      }
    }
    onChange(open.length > 0);
  };
  window.addEventListener('popstate', () => {
    const open = openDialogs();
    if (!open.length) return;
    const dialog = open[open.length - 1];
    tracked.delete(dialog);
    dialog.close();
  });
  const observer = new MutationObserver(sync);
  observer.observe(document.documentElement, {attributes: true, subtree: true, attributeFilter: ['open']});
  return sync;
}
