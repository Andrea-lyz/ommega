// Tests the complete packaged A-side scripts, not only a modern/hidden subset.
// No browser server, dependency download, device bridge or HAL is used.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFileSync, readdirSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import vm from 'node:vm';
const root = new URL('../../', import.meta.url);
const webroot = new URL('a-side/source/template/webroot/', root);
const read = relative => readFileSync(new URL(relative, root), 'utf8');
const html = read('a-side/source/template/webroot/index.html');
const modern = read('a-side/source/template/webroot/modern.js');
const bundles = readdirSync(new URL('assets/', webroot)).filter(name => /^index-.*\.js$/.test(name));
assert.equal(bundles.length, 1);
const bundle = readFileSync(new URL('assets/' + bundles[0], webroot), 'utf8');
const header = '\n// ── Ommega detector compatibility policy';
const base = bundle.slice(0, bundle.indexOf(header));
const forbidden = [
  'security-patch-dialog', 'security-patch-auto-toggle', 'set-patch-now',
  'save-patch', 'get-patch', 'patchAuto', '--security-patch', 'window.formatDate',
  'devconfig-securityPatch', 'source.android.com/docs/security/bulletin/pixel',
  'Tricky-Addon-Update-Target-List/main/.patch',
];
function scripts(directory) {
  return readdirSync(directory, {withFileTypes:true}).flatMap(entry => {
    const path = new URL(entry.name + (entry.isDirectory() ? '/' : ''), directory);
    return entry.isDirectory() ? scripts(path) : entry.name.endsWith('.js') ? [path] : [];
  });
}

test('complete shipped JavaScript and HTML contain no legacy SPL writers or fetchers', () => {
  for (const path of scripts(webroot)) {
    const source = readFileSync(path, 'utf8');
    for (const token of forbidden) assert.ok(!source.includes(token), fileURLToPath(path) + ': ' + token);
    assert.doesNotMatch(source, /ommegaTrust\s*\(\s*['"]patch['"]/);
    assert.doesNotMatch(source, /\b(?:resetprop|setprop)\b[^\n`]*(?:security_patch|patchlevel)/);
  }
  for (const token of forbidden) assert.ok(!html.includes(token), 'HTML: ' + token);
  assert.doesNotMatch(html, /id="(?:all|system|vendor|boot)-patch"/);
});

test('the reviewed whole vendor base is pinned and appended sources match the shipped suffix', () => {
  const patcher = read('a-side/source/scripts/webui-patch.py');
  const pinned = patcher.match(/^BASELINE = "([0-9a-f]{64})"$/m)?.[1];
  assert.equal(createHash('sha256').update(base).digest('hex'), pinned);
  const sources = ['compat','auth-signing','cpu-mask','capability-mask','app-detail','diagnostics']
    .map(name => read('a-side/source/scripts/webui-' + name + '.js').replaceAll('\r\n','\n')).join('');
  assert.equal(bundle.slice(base.length), sources);
  assert.equal(bundle.split(header).length, 2);
  new vm.Script(bundle, {filename:bundles[0]});
});

test('unrelated vendor startup remains intact without dangling SPL identifiers', () => {
  assert.match(base, /await Os\(\),await Nl\(\),Wl\(\),Fl\(\),Ms\(\),Mi\(\),Hl\(\),jl\(\)/);
  for (const name of ['De','Le','Il','Nt','Ft','Yr','Sl','We','je','Ke','Ye','Rl','Ll','Ol','Pl','nn','Oe','mr','Gr','ui','zl','Bl','Ml','ql']) {
    assert.doesNotMatch(base, new RegExp('\\b' + name + '\\b'), 'removed SPL symbol ' + name);
  }
  for (const id of ['boot-hash','boot-key','boot-hash-save-button','boot-key-save-button','remote-config','automation-settings']) {
    assert.ok(html.includes('id="' + id + '"'), 'preserved vendor target: ' + id);
  }
});

test('real modern appearance Done and Close bindings both dismiss their dialog', () => {
  const start = modern.indexOf("  document.getElementById('om-appearance-open').onclick");
  const end = modern.indexOf('  const scopeSlot', start);
  assert.ok(start >= 0 && end > start);
  const nodes = new Map(['om-appearance-open','om-appearance-close','om-appearance-done','om-appearance-dialog'].map(id => [id, {open:false,
    showModal() {this.open = true;}, close() {this.open = false;}}]));
  vm.runInNewContext(modern.slice(start,end), {document:{getElementById:id => nodes.get(id)}});
  for (const id of ['om-appearance-done','om-appearance-close']) {
    nodes.get('om-appearance-open').onclick();
    assert.equal(nodes.get('om-appearance-dialog').open, true);
    assert.equal(typeof nodes.get(id).onclick, 'function', id + ' must not be commented out');
    nodes.get(id).onclick();
    assert.equal(nodes.get('om-appearance-dialog').open, false);
  }
});

test('active shipped VB hash/key rebindings still validate, save and clear through their own commands', async () => {
  const nodes = new Map();
  const make = id => {
    const node = {id, value:'', close() {this.closed = true;}, addEventListener(name, handler) {this[name] = handler;},
      cloneNode() {return make(id);}, parentNode:{replaceChild(replacement) {nodes.set(id,replacement);}}};
    return node;
  };
  for (const id of ['boot-hash-input','boot-hash-dialog','boot-hash-save-button','boot-key-input','boot-key-dialog','boot-key-save-button']) nodes.set(id,make(id));
  const commands = [], messages = [];
  const context = {I:'/data/adb/modules/ommega', document:{getElementById:id => nodes.get(id) || null,querySelector:() => null},
    _:async command => {commands.push(command); return {errno:0};}, y:key => messages.push(key)};
  const start = base.indexOf('function ommegaTrustSh()');
  const end = base.indexOf('document.addEventListener("DOMContentLoaded", () => {\n  const path', start);
  assert.ok(start >= 0 && end > start);
  vm.runInNewContext(base.slice(start,end),context);
  for (const [prefix, action] of [['boot-hash','hash'],['boot-key','key']]) {
    nodes.get(prefix + '-input').value = 'a'.repeat(64);
    await nodes.get(prefix + '-save-button').click();
    assert.ok(commands.at(-1).endsWith("'" + action + "' '" + 'a'.repeat(64) + "'"));
    nodes.get(prefix + '-input').value = '';
    await nodes.get(prefix + '-save-button').click();
    assert.ok(commands.at(-1).endsWith("'" + action + "' 'clear'"));
    const count = commands.length;
    nodes.get(prefix + '-input').value = 'invalid';
    await nodes.get(prefix + '-save-button').click();
    assert.equal(commands.length,count,'invalid VB input must not write');
    assert.equal(messages.at(-1),'trust_value_invalid');
  }
  assert.ok(commands.every(command => !/patch|curl|wget|HAL/.test(command)));
});

test('all actual shell resource URLs carry the hash of their shipped bytes', () => {
  for (const match of html.matchAll(/\b(?:src|href)=["'](\.\/[^"'?]+\.(?:js|css))(?:\?v=([^"']+))?["']/g)) {
    const bytes = readFileSync(new URL(match[1], webroot));
    assert.equal(match[2], createHash('sha256').update(bytes).digest('hex').slice(0,10), match[1]);
  }
});

test('English Chinese and translation template make no SPL editor/auto/PIF promises', () => {
  for (const name of ['strings/en.xml','strings/zh-CN.xml','template.xml']) {
    const locale = read('a-side/source/template/webroot/locales/' + name);
    assert.doesNotMatch(locale, /PIF|Set Security Patch|Set custom security patch|设置安全补丁|设置自定义安全补丁|security_patch_auto|automation_(?:set_)?patch|prompt_.*patch/);
    assert.match(locale, /read-only|只读/);
    assert.match(locale, /trust_value_invalid/);
    assert.match(locale, /trust_save_failed/);
  }
});
