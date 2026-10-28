import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import vm from 'node:vm';

const source = path => readFileSync(new URL(path, import.meta.url), 'utf8');
const html = source('../../b-side/source/template/webroot/index.html');
// Execute the actual application and its actual dependency bodies in one VM.
// Only module syntax is removed; no refresh/bridge/polling behavior is stubbed.
const scripts = [
  '../shared/icons.js', '../shared/ui-kit.js',
  '../../b-side/source/template/webroot/relay-config.js',
  '../../b-side/source/template/webroot/i18n.js',
  '../../b-side/source/template/webroot/app.js',
].map(path => source(path).replace(/^import .*;\r?$/gm, '').replace(/^export \{[^\n]*\r?$/gm, '').replace(/\bexport (?=(?:const|let|function|async)\b)/g, '')).join('\n');
const digest = raw => createHash('sha256').update(raw).digest('hex');
const config = (device = 'b-one', url = 'https://relay.example.test') => [
  `OMMEGA_RELAY_SERVER=${url}`, `OMMEGA_RELAY_DEVICE_ID=${device}`,
  'OMMEGA_RELAY_MACHINE_ID=machine-one', 'OMMEGA_RELAY_TOKEN=secret-token',
  'OMMEGA_RELAY_LOG_ENABLED=true', 'OMMEGA_RELAY_LOG_LEVEL=debug',
  'OMMEGA_RELAY_LOGCAT_ENABLED=true', 'OMMEGA_RELAY_LOGCAT_LEVEL=info', '',
].join('\n');
const initialProperties = () => ({
  SYSTEM_SPL:'2026-01-01', BOOT_SPL:'2026-01-01', VENDOR_SPL:'2026-01-01', OS_VERSION:'17',
  ORIGIN_SYSTEM_SPL:'2025-01-01', ORIGIN_VENDOR_SPL:'2025-01-01', ORIGIN_OS_VERSION:'16',
  FLOOR_SYSTEM_SPL:'2026-01-01', FLOOR_VENDOR_SPL:'2026-01-01', FLOOR_OS_VERSION:'17',
  APPLIED_SYSTEM_SPL:'2026-01-01', APPLIED_BOOT_SPL:'2026-01-01', APPLIED_VENDOR_SPL:'2026-01-01', APPLIED_OS_VERSION:'17',
});
const properties = values => Object.entries(values).map(([key, value]) => `${key}=${value}`).join('\n') + '\n';
const settle = () => new Promise(resolve => setImmediate(resolve));

function fixture({missingBridge = false, intercept = null} = {}) {
  let now = Date.parse('2026-10-28T12:00:00Z'), nextTimer = 1, focused = null;
  const timers = new Map(), intervals = new Map(), elements = [], byId = new Map(), observers = [];
  const state = {raw:config(), runtime:{PIDS:'321', SPL:'2026-01-01', OS:'17', VERSION:'2.0-test'}, spl:initialProperties(), online:['b-one']};
  const calls = [], confirmations = [];
  const notify = () => observers.forEach(callback => queueMicrotask(callback));
  class Element extends EventTarget {
    constructor(tag, attributes = {}) {
      super(); this.localName = tag; this.attributes = {...attributes}; this.id = attributes.id || '';
      this.dataset = Object.fromEntries(Object.entries(attributes).filter(([key]) => key.startsWith('data-')).map(([key, value]) => [key.slice(5).replace(/-([a-z])/g, (_, char) => char.toUpperCase()), value]));
      this.value = attributes.value || ''; this.type = attributes.type || ''; this.checked = 'checked' in attributes;
      this.disabled = 'disabled' in attributes; this.hidden = 'hidden' in attributes; this.open = false; this.options = [];
      this.textContent = ''; this.className = attributes.class || ''; this._html = '';
      this.classList = {toggle:() => {}};
    }
    set innerHTML(value) { this._html = value; if (this.id === 'appearance') parse(value); }
    get innerHTML() { return this._html; }
    setAttribute(key, value) { this.attributes[key] = String(value); }
    getAttribute(key) { return this.attributes[key] ?? null; }
    add(option) { this.options.push(option); if (this.options.length === 1) this.value = option.value; }
    showModal() { this.open = true; notify(); }
    close() { this.open = false; notify(); }
    querySelector() { return byId.get(this.id === 'spl-dialog' ? 'system' : this.id === 'about-dialog' ? 'about-close' : 'om-language'); }
    getClientRects() { return this.hidden ? [] : [{}]; }
    focus() { focused = this.id; }
  }
  function parse(markup) {
    for (const match of markup.matchAll(/<([a-z][\w-]*)\b([^>]*)>/gi)) {
      const attributes = Object.fromEntries([...match[2].matchAll(/([\w-]+)(?:="([^"]*)")?/g)].map(attribute => [attribute[1], attribute[2] || '']));
      const element = new Element(match[1], attributes);
      elements.push(element); if (element.id) byId.set(element.id, element);
    }
  }
  parse(html);
  const window = new EventTarget();
  const storage = new Map();
  const historyEntries = [null]; let historyIndex = 0, hash = '#overview';
  const history = {
    get state() { return historyEntries[historyIndex]; },
    pushState(value) { historyEntries.splice(++historyIndex); historyEntries[historyIndex] = value; },
    back() { if (historyIndex) { historyIndex--; queueMicrotask(() => window.dispatchEvent(new Event('popstate'))); } },
  };
  const document = new EventTarget();
  document.hidden = false; document.documentElement = {lang:'en', dataset:{}}; document.body = {classList:{toggle:() => {}}};
  document.getElementById = id => byId.get(id) || null;
  document.querySelectorAll = selector => {
    const data = /^\[data-([\w-]+)\]$/.exec(selector);
    if (data) return elements.filter(element => element.getAttribute(`data-${data[1]}`) !== null);
    if (selector === 'md-dialog[open],dialog[open]' || selector === 'md-dialog[open]') return elements.filter(element => element.open && selector.includes(`${element.localName}[open]`));
    throw new Error('Unmodelled selector: ' + selector);
  };
  document.querySelector = selector => selector === '.om-nav' ? elements.find(element => element.className === 'om-nav') : null;
  const location = {reloads:0, reload() { this.reloads++; }, get hash() { return hash; }, set hash(value) { hash = value; queueMicrotask(() => window.dispatchEvent(new Event('hashchange'))); }};
  window.confirm = message => { confirmations.push(message); return window.confirmResult; };
  window.confirmResult = true;
  const context = vm.createContext({
    window, document, history, location, navigator:{languages:['en']}, console,
    URL, TextEncoder, TextDecoder, Intl, Uint8Array, Event, EventTarget, queueMicrotask,
    atob:value => Buffer.from(value, 'base64').toString('binary'), btoa:value => Buffer.from(value, 'binary').toString('base64'),
    localStorage:{getItem:key => storage.get(key) || null, setItem:(key, value) => storage.set(key, value)},
    matchMedia:() => ({matches:false, addEventListener() {}}),
    requestAnimationFrame:callback => queueMicrotask(callback),
    Option:class { constructor(text, value) { this.text = text; this.value = value; } },
    MutationObserver:class { constructor(callback) { observers.push(callback); } observe() {} },
    Date:class extends Date { constructor(...values) { super(...(values.length ? values : [now])); } static now() { return now; } },
    setTimeout:(callback, delay) => { const id = nextTimer++; timers.set(id, {callback, at:now + delay}); return id; },
    clearTimeout:id => timers.delete(id),
    setInterval:(callback, delay) => { const id = nextTimer++; intervals.set(id, {callback, delay}); return id; },
    clearInterval:id => intervals.delete(id),
  });
  const classify = command => command.includes('expected=') ? 'save' : command.includes('spl-control.sh" save ') ? 'spl-save'
    : command.includes('spl-control.sh" status') ? 'spl' : command.startsWith('ommega_status_url=') ? 'server'
      : command.startsWith('pids=') ? 'runtime' : command.includes('conf=') && command.includes('base64') ? 'config' : 'unexpected';
  function respond(call) {
    let errno = 0, stdout = '', stderr = '';
    if (call.kind === 'config') stdout = state.raw === null ? 'missing\n' : digest(state.raw) + '\n' + Buffer.from(state.raw).toString('base64') + '\n';
    else if (call.kind === 'runtime') stdout = properties(state.runtime);
    else if (call.kind === 'spl') stdout = properties(state.spl);
    else if (call.kind === 'server') stdout = JSON.stringify({status:'ok', online_b_devices:state.online.map(device_id => ({device_id}))});
    else if (call.kind === 'save') {
      const expected = /expected='([^']+)'/.exec(call.command)[1];
      if (expected !== (state.raw === null ? 'missing' : digest(state.raw))) { errno = 3; stderr = 'CONFIG_CHANGED'; }
      else { state.raw = Buffer.from(/printf '%s' '([^']+)' \| base64 -d >/.exec(call.command)[1], 'base64').toString(); stdout = 'SAVED\n'; }
    } else if (call.kind === 'spl-save') {
      const values = [...call.command.matchAll(/'([^']*)'/g)].map(match => match[1]);
      ['SYSTEM_SPL','BOOT_SPL','VENDOR_SPL','OS_VERSION'].forEach((key, index) => { state.spl[key] = values[index]; });
      if (values[0]) state.runtime.SPL = values[0]; if (values[3]) state.runtime.OS = values[3];
    } else throw new Error('Unexpected command: ' + call.command);
    call.reply(errno, stdout, stderr);
  }
  if (!missingBridge) window.ksu = {exec(command, options, callbackName) {
    assert.equal(options, '{}');
    const callback = window[callbackName];
    const call = {kind:classify(command), command, callbackName, callback, replied:false,
      reply(errno, stdout = '', stderr = '') { this.replied = true; queueMicrotask(() => callback(errno, stdout, stderr)); },
    };
    const encoded = /^ommega_status_url=\$\(printf '%s' '([^']+)'/.exec(command);
    if (encoded) call.url = Buffer.from(encoded[1], 'base64').toString();
    calls.push(call);
    if (f.intercept) f.intercept(call, () => respond(call)); else respond(call);
  }};
  const f = {
    state, calls, context, window, document, byId, confirmations, timers, intervals, intercept,
    get focused() { return focused; },
    get:id => byId.get(id), count:kind => calls.filter(call => call.kind === kind).length,
    click:id => byId.get(id).onclick(),
    submit:() => byId.get('relay-form').onsubmit({preventDefault() {}}),
    respond,
    async navigate(view) { location.hash = '#' + view; await settle(); },
    async poll() { for (const entry of intervals.values()) entry.callback(); await settle(); },
    async advance(milliseconds) {
      const end = now + milliseconds;
      while (true) {
        const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
        if (!next) break;
        now = next[1].at; timers.delete(next[0]); next[1].callback(); await settle();
      }
      now = end; await settle();
    },
    async ready() { await settle(); assert.equal(timers.size, 0, 'all automatic initial reads settled'); return f; },
  };
  vm.runInContext(scripts, context, {filename:'b-real-application.js'});
  return f;
}

function assertReadOnly(calls) {
  assert.ok(calls.length > 0);
  for (const call of calls) {
    assert.ok(['config','runtime','spl','server'].includes(call.kind), call.command);
    assert.doesNotMatch(call.command, /\b(?:resetprop|killall|mkdir|mktemp|touch)\b|service\.sh|spl-control\.sh" (?:save|apply)\b/);
    if (call.kind === 'spl') assert.match(call.command, / status --readonly$/);
  }
}
function assertIdle(f) {
  assert.equal(f.get('runtime-refresh').disabled, false);
  for (const id of ['runtime-refresh','relay-reload','spl-reload','server-panel']) assert.equal(f.get(id).getAttribute('aria-busy'), 'false', id);
  assert.equal(f.timers.size, 0);
}


test('B global refresh bypasses the 20s status throttle and awaits config, runtime and server reads', async () => {
  const f = await fixture().ready();
  assert.equal(f.count('server'), 1);
  await f.advance(10000); await f.poll();
  assert.equal(f.count('server'), 1, 'background status remains throttled');
  f.state.runtime.OS = '18'; f.state.runtime.PIDS = '654'; f.state.online = [];
  await f.click('runtime-refresh');
  assert.equal(f.count('server'), 2, 'explicit click is forced inside throttle window');
  assert.equal(f.get('summary-os').textContent, '18');
  assert.equal(f.get('runtime-pid').textContent, 'PID 654');
  assert.equal(f.get('server-state').dataset.tone, 'offline');
  assert.match(f.get('refresh-message').textContent, /refreshed/i);
  assertReadOnly(f.calls); assertIdle(f);
});

test('B global refresh updates saved summaries and device details from every navigation page', async () => {
  const f = await fixture().ready();
  for (const [index, view] of ['overview','connection','settings'].entries()) {
    await f.navigate(view);
    const device = `b-${index}`;
    f.state.raw = config(device, `https://relay${index}.example.test/base`); f.state.online = [device];
    f.state.runtime.VERSION = `version-${index}`; f.state.spl.OS_VERSION = String(18 + index);
    await f.click('runtime-refresh');
    assert.equal(f.get(view + '-view').hidden, false);
    assert.equal(f.get('summary-device').textContent, device);
    assert.equal(f.get('runtime-device').textContent, device);
    assert.equal(f.get('summary-server').textContent, `https://relay${index}.example.test/base`);
    assert.equal(f.get('summary-version').textContent, `version-${index}`);
    assert.equal(f.get('osversion').value, String(18 + index));
    assert.equal(f.get('server-state').dataset.tone, 'online');
    assert.equal(f.get('refresh-message').hidden, false);
  }
  assertReadOnly(f.calls); assertIdle(f);
});

test('B background polling refreshes a changed saved endpoint even inside the throttle window', async () => {
  const f = await fixture().ready();
  f.state.raw = config('b-new', 'https://new.example.test'); f.state.online = ['b-new'];
  await f.poll();
  assert.equal(f.count('server'), 2);
  assert.equal(f.calls.filter(call => call.kind === 'server').at(-1).url, 'https://new.example.test/api/status/');
  assert.equal(f.get('summary-device').textContent, 'b-new');
  assert.equal(f.get('server-state').dataset.tone, 'online');
  assertReadOnly(f.calls);
});

test('B connection and SPL reread buttons refresh all facts but discard only the explicitly approved draft', async () => {
  const f = await fixture().ready();
  f.get('relay-device').value = 'unsaved-device'; f.get('system').value = '2030-01-01';
  f.state.raw = config('b-saved'); f.state.spl.SYSTEM_SPL = '2027-01-01'; f.state.online = ['b-saved']; f.state.runtime.OS = '19';
  f.window.confirmResult = false;
  const before = f.calls.length;
  await f.click('relay-reload');
  assert.equal(f.calls.length, before); assert.equal(f.get('relay-device').value, 'unsaved-device');
  f.window.confirmResult = true;
  await f.click('relay-reload');
  assert.equal(f.get('relay-device').value, 'b-saved'); assert.equal(f.get('system').value, '2030-01-01');
  assert.equal(f.get('summary-os').textContent, '19'); assert.equal(f.get('server-state').dataset.tone, 'online');
  f.get('relay-device').value = 'another-draft';
  await f.click('spl-reload');
  assert.equal(f.get('system').value, '2027-01-01'); assert.equal(f.get('relay-device').value, 'another-draft');
  assert.equal(f.count('server'), 3, 'each section reread bypasses status throttle');
  assertReadOnly(f.calls); assertIdle(f);
});

test('B SPL and About sheets re-read actual application data on open; appearance does not', async () => {
  const f = await fixture().ready();
  f.get('osversion').value = '24'; f.get('relay-device').value = 'draft';
  f.state.runtime.VERSION = 'fresh-module'; f.state.raw = config('b-fresh'); f.state.online = ['b-fresh'];
  await f.click('spl-open');
  assert.equal(f.get('spl-dialog').open, true); assert.equal(f.get('osversion').value, '24');
  assert.equal(f.get('relay-device').value, 'draft'); assert.match(f.get('message').textContent, /kept/i);
  f.click('spl-close'); await settle();
  await f.click('about-open');
  assert.equal(f.get('summary-version').textContent, 'fresh-module'); assert.equal(f.count('server'), 3);
  f.click('about-close'); await settle();
  const before = f.calls.length; f.click('appearance-open'); await settle();
  assert.equal(f.calls.length, before);
  assertReadOnly(f.calls); assertIdle(f);
});

test('B polling preserves edits begun while reads are in flight, while saved summaries and CAS remain accurate', async () => {
  const f = await fixture().ready();
  const originalDigest = digest(f.state.raw), held = [];
  f.state.raw = config('b-external', 'https://external.example.test'); f.state.online = ['b-external'];
  f.intercept = (call, respond) => ['config','spl'].includes(call.kind) ? held.push(call) : respond();
  await f.poll();
  f.get('relay-device').value = 'draft-device'; f.get('relay-token').value = 'draft-token';
  f.get('file-log-enabled').checked = false; f.get('system').value = '2031-01-01'; f.get('osversion').value = '29';
  f.intercept = null; held.forEach(f.respond); await settle();
  assert.equal(f.get('summary-device').textContent, 'b-external'); assert.equal(f.get('summary-server').textContent, 'https://external.example.test/');
  assert.equal(f.get('relay-device').value, 'draft-device'); assert.equal(f.get('relay-token').value, 'draft-token');
  assert.equal(f.get('file-log-enabled').checked, false); assert.equal(f.get('system').value, '2031-01-01'); assert.equal(f.get('osversion').value, '29');
  assertReadOnly(f.calls);
  await f.submit();
  const saveCall = f.calls.find(call => call.kind === 'save');
  assert.match(saveCall.command, new RegExp(`expected='${originalDigest}'`));
  assert.match(f.get('relay-message').textContent, /changed elsewhere/i);
  assert.equal(f.count('save'), 1, 'a read never silently rebases or retries an old draft');
});

test('B initial asynchronous property read preserves a draft typed before its first result', async () => {
  const held = [];
  const f = fixture({intercept:(call, respond) => call.kind === 'spl' ? held.push(call) : respond()});
  await settle(); f.get('system').value = '2032-01-01';
  f.intercept = null; held.forEach(f.respond); await settle();
  assert.equal(f.get('system').value, '2032-01-01'); assert.equal(f.get('save').disabled, false);
  await f.click('runtime-refresh'); assert.equal(f.get('system').value, '2032-01-01');
  assertReadOnly(f.calls); assertIdle(f);
});

test('B manual taps during a running poll await a forced follow-up and cannot render stale status last', async () => {
  const f = await fixture().ready(), held = [];
  await f.advance(20000);
  f.intercept = (call, respond) => call.kind === 'server' ? held.push(call) : respond();
  await f.poll(); assert.equal(held.length, 1);
  let done = 0;
  const first = f.click('runtime-refresh').then(() => { done++; });
  const second = f.click('runtime-refresh').then(() => { done++; });
  await settle(); assert.equal(done, 0); assert.equal(f.get('runtime-refresh').getAttribute('aria-busy'), 'true');
  held[0].reply(0, JSON.stringify({status:'ok', online_b_devices:[{device_id:'b-one'}]}));
  f.state.online = []; await settle();
  assert.equal(held.length, 2, 'forced follow-up is not lost in the throttle or busy guard');
  assert.equal(done, 0, 'returned promises include the queued read');
  f.respond(held[1]); await Promise.all([first, second]);
  assert.equal(done, 2); assert.equal(f.get('server-state').dataset.tone, 'offline');
  assert.equal(f.count('server'), 3); assertIdle(f); assertReadOnly(f.calls);
});

test('B section rereads queue behind an in-flight read and preserve edits made after confirmation', async () => {
  const f = await fixture().ready(), held = [];
  f.intercept = (call, respond) => call.kind === 'server' ? held.push(call) : respond();
  const globalRead = f.click('runtime-refresh'); await settle();
  f.get('relay-device').value = 'confirmed-draft';
  let done = false; const reload = f.click('relay-reload').then(() => { done = true; });
  f.get('relay-device').value = 'newer-draft'; f.state.raw = config('b-external'); f.state.online = ['b-external'];
  await settle(); assert.equal(done, false);
  f.intercept = null; f.respond(held[0]); await Promise.all([globalRead, reload]);
  assert.equal(f.get('relay-device').value, 'newer-draft'); assert.equal(f.get('summary-device').textContent, 'b-external');
  assert.equal(f.count('server'), 3); assertReadOnly(f.calls); assertIdle(f);
});

for (const kind of ['config','runtime','spl','server']) {
  test(`B ${kind} read failures are visible, clear busy states and recover on immediate forced retry`, async () => {
    const f = await fixture().ready();
    f.get('relay-device').value = 'keep-draft'; f.get('system').value = '2033-01-01';
    f.intercept = (call, respond) => call.kind === kind ? call.reply(1, '', `${kind} permission denied`) : respond();
    await f.click('runtime-refresh');
    assert.match(f.get('refresh-message').textContent, new RegExp(`${kind} permission denied`)); assert.match(f.get('refresh-message').className, /error/);
    if (kind === 'config') { assert.equal(f.get('summary-device').textContent, '—'); assert.equal(f.get('server-state').dataset.tone, 'unknown'); }
    if (kind === 'runtime') { assert.equal(f.get('runtime-panel').dataset.tone, 'unknown'); assert.equal(f.get('runtime-pid').textContent, '—'); }
    if (kind === 'spl') assert.equal(f.get('save').disabled, true);
    if (kind === 'server') assert.equal(f.get('server-state').dataset.tone, 'unknown');
    assertIdle(f);
    f.intercept = null; await f.click('runtime-refresh');
    assert.doesNotMatch(f.get('refresh-message').className, /error/); assert.equal(f.get('server-state').dataset.tone, 'online');
    assert.equal(f.get('relay-device').value, 'keep-draft'); assert.equal(f.get('system').value, '2033-01-01');
    assert.equal(f.get('save').disabled, false); assertReadOnly(f.calls); assertIdle(f);
  });
  test(`B absent ${kind} callbacks have a bounded read timeout and late results cannot overwrite a retry`, async () => {
    const f = await fixture().ready(), held = [];
    f.intercept = (call, respond) => call.kind === kind ? held.push(call) : respond();
    let done = false; const refresh = f.click('runtime-refresh').then(() => { done = true; });
    await settle(); assert.equal(done, false); assert.equal(held.length, 1);
    await f.advance(12000); await refresh;
    assert.match(f.get('refresh-message').textContent, /timed out/i); assertIdle(f);
    assert.equal(f.window[held[0].callbackName], undefined, 'timed-out callback registration is cleaned');
    f.intercept = null; f.state.runtime.PIDS = '999'; await f.click('runtime-refresh');
    const rendered = f.get('refresh-message').textContent;
    held[0].reply(0, 'old invalid response'); await settle();
    assert.equal(f.get('refresh-message').textContent, rendered); assert.equal(f.get('runtime-pid').textContent, 'PID 999');
    assert.equal(f.get('server-state').dataset.tone, 'online'); assertReadOnly(f.calls); assertIdle(f);
  });
  test(`B malformed ${kind} success callbacks are errors rather than successful refreshes`, async () => {
    const f = await fixture().ready();
    f.intercept = (call, respond) => call.kind === kind ? call.reply(0, 'not a valid status response') : respond();
    await f.click('runtime-refresh');
    assert.match(f.get('refresh-message').className, /error/); assertIdle(f);
    if (kind === 'server' || kind === 'config') assert.equal(f.get('server-state').dataset.tone, 'unknown');
    if (kind === 'spl') assert.equal(f.get('save').disabled, true);
    assertReadOnly(f.calls);
  });
}

test('B an absent manager bridge fails read feedback without hanging or leaking timers', async () => {
  const f = await fixture({missingBridge:true}).ready();
  await f.click('runtime-refresh');
  assert.match(f.get('refresh-message').textContent, /module manager/i);
  assert.equal(f.get('server-state').dataset.tone, 'unknown'); assert.equal(f.get('save').disabled, true);
  assertIdle(f);
});

for (const kind of ['save','spl-save']) test(`B ${kind} callbacks are not timed out, retried or bypassed by a queued refresh`, async () => {
  const f = await fixture().ready(), held = [];
  f.intercept = (call, respond) => call.kind === kind ? held.push(call) : respond();
  if (kind === 'save') f.get('relay-device').value = 'b-changed'; else f.get('osversion').value = '18';
  let done = false;
  const mutation = (kind === 'save' ? f.submit() : f.click('save')).then(() => { done = true; });
  await settle(); assert.equal(held.length, 1); assert.equal(f.timers.size, 0, 'mutation has no UI timeout');
  const before = f.calls.length; const refresh = f.click('runtime-refresh');
  await f.advance(60000);
  assert.equal(done, false); assert.equal(f.calls.length, before, 'no read may race the unconfirmed mutation');
  assert.equal(f.count(kind), 1, 'no retry of a possibly running mutation');
  f.intercept = null; f.respond(held[0]); await Promise.all([mutation, refresh]);
  assert.equal(done, true); assert.equal(f.count(kind), 1); assertIdle(f);
});

test('B hidden tabs skip background reads but manual refresh remains forced and read-only', async () => {
  const f = await fixture().ready();
  f.document.hidden = true;
  const before = f.calls.length; await f.poll(); assert.equal(f.calls.length, before);
  await f.click('runtime-refresh'); assert.equal(f.calls.length, before + 4);
  assertReadOnly(f.calls); assertIdle(f);
});
