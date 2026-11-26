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
// The icon set is shared with the design prototype; see icons.js.
export {icon} from './icons.js';

// `spawn: true` starts commands through ksu.spawn when the manager offers it.
// ksu.exec runs the whole command inside the JavaScript call, so a 400 ms read
// freezes scrolling and taps for 400 ms; spawn returns after a fixed start cost
// and streams the output back. Spawned commands still run one at a time in the
// order they were issued, exactly like the synchronous exec they replace, and
// each one waits until the finger is lifted and scrolling has settled.
export function createBridge(scope, timeoutMs = 12000, {spawn = false} = {}) {
  let sequence = 0;
  const unsupported = () => ({errno:1, stdout:'', stderr:text('请从支持 WebUI 的模块管理器打开。', 'Open from a module manager with WebUI support.')});
  return command => new Promise(resolve => {
    const callback = `ommega_${scope}_${Date.now()}_${sequence++}`;
    let settled = false, viaSpawn = false, timer = null, release = () => {};
    const settle = result => {
      if (settled) return;
      settled = true; clearTimeout(timer);
      // A late spawn event must still find a receiver after the promise settled.
      if (viaSpawn) { window[callback] = bridgeInertReceiver; setTimeout(() => { delete window[callback]; }, 3000); }
      else delete window[callback];
      release();
      resolve(result);
    };
    // Read operations may fail to deliver a callback. Never interrupt a device
    // process. The read timeout covers the command itself, not its queue wait.
    const arm = () => {
      if (timeoutMs > 0) timer = setTimeout(() => settle({errno:1, stdout:'', stderr:text('读取超时，请刷新重试。', 'Read timed out. Refresh to retry.')}), timeoutMs);
    };
    const runExec = () => {
      window[callback] = (errno, stdout, stderr) => settle({errno:Number(errno), stdout:stdout || '', stderr:stderr || ''});
      try { window.ksu.exec(command, '{}', callback); }
      catch { settle(unsupported()); }
    };
    if (!spawn || typeof window.ksu?.spawn !== 'function') { arm(); runExec(); return; }
    bridgeSpawnProbe(ok => { if (ok) bridgeEnqueue(runSpawn); else { arm(); runExec(); } });
    function runSpawn(done) {
      release = done;
      viaSpawn = true;
      const out = [], err = [];
      window[callback] = {
        stdout: {emit: (event, data) => { if (event === 'data') out.push(String(data ?? '')); }},
        stderr: {emit: (event, data) => { if (event === 'data') err.push(String(data ?? '')); }},
        // A failing command reports `exit` and then `error`; a command that could
        // not start reports only `error`. ksu.exec also drops the final newline.
        emit: (event, data) => {
          if (event === 'exit') settle({errno:Number(data) || 0, stdout:out.join('\n'), stderr:err.join('\n')});
          else if (event === 'error') settle({errno:Number(data?.exitCode) || 1, stdout:out.join('\n'), stderr:err.join('\n') || String(data?.message || '')});
        }
      };
      arm();
      try { window.ksu.spawn(command, '[]', '{}', callback); }
      catch { settle(unsupported()); }
    }
  });
}
const bridgeInertReceiver = {stdout:{emit() {}}, stderr:{emit() {}}, emit() {}};
// A manager may expose spawn without ever answering it. One `true` must report
// a clean exit within 3 s before any command relies on spawn; otherwise every
// command keeps using exec.
let bridgeProbe = null;
function bridgeSpawnProbe(next) {
  if (bridgeProbe?.done) { next(bridgeProbe.ok); return; }
  if (bridgeProbe) { bridgeProbe.waiters.push(next); return; }
  const probe = bridgeProbe = {done:false, ok:false, waiters:[next]};
  const name = 'ommega_spawn_probe_' + Date.now();
  const finish = ok => {
    if (probe.done) return;
    probe.done = true; probe.ok = ok;
    window[name] = bridgeInertReceiver; setTimeout(() => { delete window[name]; }, 3000);
    for (const waiter of probe.waiters.splice(0)) waiter(ok);
  };
  window[name] = {stdout:{emit() {}}, stderr:{emit() {}}, emit: (event, code) => { if (event === 'exit') finish(Number(code) === 0); else if (event === 'error') finish(false); }};
  setTimeout(() => finish(false), 3000);
  try { window.ksu.spawn('true', '[]', '{}', name); } catch { finish(false); }
}
const bridgeQueue = [];
let bridgeBusy = false, bridgeHeadSince = 0, bridgeTimer = 0;
let bridgeLastInput = 0, bridgePointers = 0, bridgeTracking = false;
function bridgeTrack() {
  if (bridgeTracking || typeof window.addEventListener !== 'function') return;
  bridgeTracking = true;
  const mark = () => { bridgeLastInput = performance.now(); };
  const lift = () => { bridgePointers = Math.max(0, bridgePointers - 1); mark(); };
  const options = {capture:true, passive:true};
  window.addEventListener('pointerdown', () => { bridgePointers++; mark(); }, options);
  window.addEventListener('pointerup', lift, options);
  window.addEventListener('pointercancel', lift, options);
  window.addEventListener('scroll', mark, options);
}
function bridgeEnqueue(job) {
  bridgeTrack();
  bridgeQueue.push(job);
  bridgePump();
}
// Start the next command once input has been quiet for 160 ms, or after the
// head has waited 600 ms so a long scroll cannot starve reads or a write the
// user just confirmed. Each start is its own task, so frames run in between.
function bridgePump() {
  if (bridgeBusy || bridgeTimer || !bridgeQueue.length) return;
  const now = performance.now();
  if (!bridgeHeadSince) bridgeHeadSince = now;
  if ((bridgePointers || now - bridgeLastInput < 160) && now - bridgeHeadSince < 600) {
    bridgeTimer = setTimeout(() => { bridgeTimer = 0; bridgePump(); }, 50);
    return;
  }
  bridgeHeadSince = 0;
  bridgeBusy = true;
  let released = false;
  bridgeQueue.shift()(() => {
    if (released) return;
    released = true; bridgeBusy = false;
    bridgeTimer = setTimeout(() => { bridgeTimer = 0; bridgePump(); }, 0);
  });
}
export function pollWhenVisible(refresh, delay = 10000, button = null) {
  let running = null, queued = null, stopped = false;
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
  const run = (forced = false) => {
    if (stopped || document.hidden && !forced) return Promise.resolve();
    if (running) {
      if (!forced) return running;
      // Coalesce taps during a read into one fresh follow-up. Its promise waits
      // for that follow-up (even if the preceding automatic read failed).
      if (!queued) queued = running.catch(() => {}).then(() => {
        queued = null;
        return run(true);
      });
      return queued;
    }
    setBusy(true);
    running = Promise.resolve().then(() => refresh(forced)).finally(() => {
      running = null;
      if (!queued) setBusy(false);
    });
    return running;
  };
  // Read callbacks own their error feedback. Automatic retries must not create
  // unhandled rejections, while callers of the manual action can await errors.
  const automatic = () => { void run(false).catch(() => {}); };
  const timer = setInterval(automatic, delay);
  const onVisibility = automatic;
  document.addEventListener('visibilitychange', onVisibility);
  window.addEventListener('pagehide', () => { stopped = true; clearInterval(timer); document.removeEventListener('visibilitychange', onVisibility); }, {once:true});
  window.addEventListener('pageshow', event => { if (event.persisted) location.reload(); });
  automatic();
  // The returned function is the button action: it always produces a fresh
  // read, and the panel is never left with a button that does nothing.
  return () => run(true);
}

// The relay reports why a request failed as a JSON object. The text inside is
// what an operator needs; the wrapper only adds noise.
export function healthMessage(health) {
  if (health?.last_kind === 'attestation') return text('证书链验证或密钥保存失败。', 'Certificate validation or key storage failed.');
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
export function installDialogGuard(onChange = () => {}, {includeNative = false} = {}) {
  const tracked = [];
  let sequence = 0, rewinding = false, pendingFocus = null;
  const cancelBound = new WeakSet();
  const openDialogs = () => [...document.querySelectorAll(includeNative ? 'md-dialog[open],dialog[open]' : 'md-dialog[open]')];
  const restoreFocus = () => {
    if (!pendingFocus || rewinding) return;
    const target = document.getElementById(pendingFocus);
    if (!target || target.disabled || !target.getClientRects().length) { pendingFocus=null; return; }
    target.focus(); pendingFocus=null;
  };
  const scheduleFocus = () => {
    if (!pendingFocus || rewinding) return;
    if (typeof requestAnimationFrame === 'function') requestAnimationFrame(restoreFocus);
    else queueMicrotask(restoreFocus);
  };
  const sync = () => {
    const open = openDialogs();
    onChange(open.length > 0);
    if (rewinding) return;
    // Closing a dialog with its own button consumes only its own history
    // entry. The resulting popstate must not close the parent as well.
    while (tracked.length && !tracked.at(-1).dialog.open) {
      const entry = tracked.pop();
      pendingFocus = entry.returnFocus || pendingFocus;
      if (!entry.consumed && history.state?.omDialog === entry.token) {
        rewinding = true;
        history.back();
        return;
      }
    }
    for (const dialog of open) {
      if (includeNative && dialog.localName === 'dialog' && !cancelBound.has(dialog)) {
        cancelBound.add(dialog);
        dialog.addEventListener('cancel', event => {
          event.preventDefault();
          closeTop(false);
        });
      }
      if (!tracked.some(entry => entry.dialog === dialog)) {
        const entry = {dialog, token: ++sequence, consumed: false, closing: false,
          returnFocus:dialog.getAttribute?.('data-om-return-focus') || null};
        tracked.push(entry);
        history.pushState({...history.state, omDialog: entry.token}, '');
      }
    }
    scheduleFocus();
  };
  const closeTop = consumed => {
    const entry = [...tracked].reverse().find(item => item.dialog.open && !item.closing);
    if (!entry) return false;
    entry.consumed = consumed;
    entry.closing = true;
    // md-dialog.close() completes after its closing animation. Keep the entry
    // until then so an observer cannot register the closing dialog again.
    Promise.resolve(entry.dialog.close()).finally(() => { entry.closing = false; sync(); });
    return true;
  };
  window.addEventListener('popstate', () => {
    if (rewinding) { rewinding = false; sync(); return; }
    // Forward traversal into an already closed dialog must not close its
    // still-open parent. Native back and the matching history event also
    // must not consume two dialog levels.
    const token = history.state?.omDialog;
    const top = [...tracked].reverse().find(entry => entry.dialog.open);
    if (!top || token === top.token || (token != null && !tracked.some(entry => entry.token === token))) return;
    closeTop(true);
  });
  // KernelSU-style WebViews also dispatch a native back event. Handle it
  // before the packaged handler, which searches dialogs in DOM order.
  const nativeBack = event => {
    sync();
    if (!closeTop(false)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
  };
  window.addEventListener('back', nativeBack, true);
  window.addEventListener('backbutton', nativeBack, true);
  const observer = new MutationObserver(sync);
  observer.observe(document.documentElement, {attributes: true, subtree: true, attributeFilter: ['open']});
  sync();
  return sync;
}
