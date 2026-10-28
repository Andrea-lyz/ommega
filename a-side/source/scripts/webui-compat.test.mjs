// Verifies the detector-compatibility glue inside the prebuilt WebUI bundle.
//
// The WebUI is shipped as a bundle without sources, so the glue is appended by
// scripts/webui-patch.py. This test extracts the appended block from the
// committed bundle and exercises its policy parsing/serialization, which keeps
// the patched bundle honest in CI.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const assets = join(dirname(here), "template", "webroot", "assets");
const bundles = readdirSync(assets).filter(name => /^index-.*\.js$/.test(name));
assert.equal(bundles.length, 1, "expected exactly one WebUI bundle");

const source = readFileSync(join(assets, bundles[0]), "utf8");
const GLUE_START = "// ── Ommega detector compatibility policy";
const start = source.indexOf(GLUE_START);
assert.notEqual(start, -1, "compat glue missing from the WebUI bundle");
const glue = source.slice(start);

function loadGlue({language = "en", preference = null, scopeConfig} = {}) {
  const element = { checked: false, onclick: undefined, addEventListener() {} };
  const context = {
    console,
    TextEncoder,
    TextDecoder,
    atob,
    btoa,
    URL,
    setTimeout,
    clearTimeout,
    ksu: { getPackagesInfo: packages => JSON.stringify(JSON.parse(packages).map(pkg => ({packageName:pkg, appLabel:'App ' + pkg}))) },
    Mi: async () => undefined,
    _: async () => ({ stdout: "" }),
    se: { querySelectorAll: () => [] },
    Dl: new Set(),
    Qe: [],
    ommegaTargetModes: new Map(),
    OMMEGA_TARGET_SECURITY: "/data/adb/ommega/ommegadata/target-security.toml",
    navigator: {language},
    localStorage: {getItem: () => preference},
    document: {
      getElementById: () => element,
      querySelector: () => null,
      createElement: () => ({
        style: {},
        classList: { add() {} },
        appendChild() {},
        setAttribute() {},
        addEventListener() {},
        querySelector: () => null,
      }),
      addEventListener() {},
    },
  };
  vm.createContext(context);
  vm.runInContext(scopeConfig ? glue.replace('"/data/adb/ommega/ommegadata/injector.toml"', JSON.stringify(scopeConfig)) : glue, context);
  return context;
}

test("startup read timeout is bounded and a late original callback cannot change its result", async () => {
  const ctx=loadGlue();let expire,finish;
  ctx.setTimeout=fn=>{expire=fn;return 1;};ctx.clearTimeout=()=>{};
  ctx._=()=>new Promise(resolve=>{finish=resolve;});
  const pending=ctx.ommegaReadOnly('read fixture');
  expire();const result=await pending;
  assert.equal(result.errno,1);assert.equal(result.timedOut,true);
  finish({errno:0,stdout:'late'});await Promise.resolve();
  assert.equal(result.stdout,'');
});

test("health read failure never claims that no failures were recorded", async () => {
  const ctx=loadGlue();const panel={textContent:''};
  ctx.ommegaHealthPanel=()=>panel;
  ctx._=async()=>({errno:1,stdout:'',stderr:'mock read timeout'});
  await ctx.ommegaRefreshHealth();
  assert.match(panel.textContent,/Could not read/);
  assert.doesNotMatch(panel.textContent,/no failures recorded/);
});

test("the glue evaluates without the WebUI globals", () => {
  loadGlue();
});

test("certificate badges only use positive safe counts with explicit sharing metadata", () => {
  const ctx = loadGlue();
  assert.equal(ctx.ommegaCertificateCount(null), null);
  assert.equal(ctx.ommegaCertificateCount({count:0,shared:false}), null);
  assert.equal(ctx.ommegaCertificateCount({count:1}), null);
  assert.equal(ctx.ommegaCertificateCount({count:Number.MAX_SAFE_INTEGER+1,shared:false}), null);
  const count = ctx.ommegaCertificateCount({count:12,shared:true});
  assert.equal(count.count,12);
  assert.equal(count.shared,true);
});

test("certificate badges appear after the app name and identify shared UID counts", () => {
  const ctx = loadGlue({language:'zh-CN'});
  let badge = null, label = '';
  const name = {textContent:'示例应用',querySelector:()=>badge,appendChild:value=>{badge=value;}};
  const card = {dataset:{},querySelector:()=>name,getAttribute:()=> 'com.example.app',setAttribute:(_, value)=>{label=value;}};
  ctx.document.createElement = () => ({textContent:'',title:'',remove(){badge=null;}});
  ctx.fixtureCounts = {'com.example.app':{count:12,shared:true}};
  vm.runInContext('ommegaCertificateCounts = fixtureCounts',ctx);
  ctx.ommegaUpdateAppCount(card);
  assert.match(badge.textContent,/证书 12 次.*共享 UID/);
  assert.match(label,/共享 UID/);
  ctx.fixtureCounts['com.example.app'] = {count:0,shared:false};
  ctx.ommegaUpdateAppCount(card);
  assert.equal(badge,null);
});

test("editing one app mode preserves other app policies and removes only its own override", () => {
  const ctx = loadGlue();
  const source = 'version = 1\r\n[packages]\r\n"com.example.wallet" = "tee" # saved\r\n"com.example.other" = "strongbox"\r\n';
  const read = ctx.ommegaEditSecurityPolicy(source, "com.example.wallet", null);
  assert.equal(read.modes.get("com.example.wallet"), "tee");
  assert.equal(read.replacement, source);
  const changed = ctx.ommegaEditSecurityPolicy(source, "com.example.wallet", "strongbox");
  assert.equal(changed.modes.get("com.example.other"), "strongbox");
  assert.match(changed.replacement, /"com.example.wallet" = "strongbox"/);
  const reset = ctx.ommegaEditSecurityPolicy(changed.replacement, "com.example.wallet", "global_default");
  assert.equal(reset.modes.has("com.example.wallet"), false);
  assert.equal(reset.modes.get("com.example.other"), "strongbox");
  assert.throws(() => ctx.ommegaEditSecurityPolicy('version = 1\n[packages]\n"a.b" = "future"\n', 'a.b', 'tee'), /Unsupported/);
  assert.throws(() => ctx.ommegaEditSecurityPolicy(null, '../escape', 'tee'), /Invalid/);
});

test("scope and mode writes are fixed-path atomic compares and cannot touch each other", () => {
  const ctx = loadGlue();
  const before = 'com.example.wallet\n';
  const command = ctx.ommegaAppFileWriteCommand('/data/adb/ommega/ommegadata/target.txt', before, 'com.example.other\n');
  assert.match(command, /ommega-auth-policy/);
  assert.match(command, /\bwrite-target$/);
  const payload = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], 'base64').toString('utf8'));
  assert.deepEqual(payload, {expected:before, replacement:'com.example.other\n'});
  assert.doesNotMatch(command, /write-target-security|injector\.toml/);
  assert.throws(() => ctx.ommegaAppFileWriteCommand('/tmp/other', null, 'x'), /Invalid/);
});

test("per-app mode saves outside the scope list without dropping another override", async () => {
  const ctx = loadGlue({language:'zh-CN'});
  let contents = 'version = 1\n[packages]\n"com.example.other" = "strongbox"\n';
  const status = {textContent:'',classList:{toggle() {}}};
  ctx.document.getElementById = id => id === 'om-detail-mode-status' ? status : null;
  ctx.document.querySelectorAll = () => [];
  vm.runInContext('ommegaDetail = {pkg:"com.example.unselected",session:1}; ommegaGlobalScopeEnabled = true',ctx);
  ctx._ = async command => {
    if (command.includes('ommega_app_writer=')) {
      const request = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], 'base64').toString('utf8'));
      assert.equal(request.expected, contents);
      assert.match(command, /\bwrite-target(?:-security)?$/);
      contents = request.replacement;
      return {errno:0,stdout:'saved'};
    }
    return {errno:0,stdout:'F:' + Buffer.from(contents).toString('base64')};
  };
  await ctx.ommegaDetailSaveMode({checked:true,value:'tee'});
  assert.match(contents, /"com.example.unselected" = "tee"/);
  assert.match(contents, /"com.example.other" = "strongbox"/);
  assert.match(status.textContent, /已保存/);
});

test("system-app visibility edits only system_app and never target.txt", async () => {
  const ctx = loadGlue({language:'zh-CN'});
  ctx.document.getElementById = () => null;
  let target = 'com.example.other\n', system = null;
  const writes = [];
  ctx._ = async command => {
    if (command.startsWith('pm list packages -s ')) return {errno:0,stdout:'package:com.example.system'};
    if (command.includes('ommega_app_writer=')) {
      const request = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], 'base64').toString('utf8'));
      const isTarget = command.endsWith(' write-target');
      const current = isTarget ? target : system;
      assert.equal(request.expected, current);
      writes.push(isTarget ? 'target' : 'system');
      if (isTarget) target = request.replacement; else system = request.replacement;
      return {errno:0,stdout:'saved'};
    }
    const current = command.includes('/system_app') ? system : target;
    return {errno:0,stdout:current === null ? 'M:' : 'F:' + Buffer.from(current).toString('base64')};
  };
  await assert.rejects(ctx.ommegaWriteSystemApp('com.example;rm', true), /Invalid/);
  await ctx.ommegaWriteSystemApp('com.example.system', true);
  assert.equal(target, 'com.example.other\n');
  assert.equal(system, 'com.example.system\n');
  assert.deepEqual(writes, ['system']);
  await ctx.ommegaWriteSystemApp('com.example.system', false);
  assert.equal(system, '');
  assert.equal(target, 'com.example.other\n');
  assert.deepEqual(writes, ['system','system']);
});

test("per-app scope editing preserves other entries and comments", () => {
  const ctx = loadGlue();
  const source = '# keep\r\ncom.example.first\r\ncom.example.second!\r\n';
  const removed = ctx.ommegaEditAppScope(source, 'com.example.second', false);
  assert.equal(removed.replacement, '# keep\r\ncom.example.first\r\n');
  const added = ctx.ommegaEditAppScope(removed.replacement, 'com.example.third', true);
  assert.equal(added.replacement, '# keep\r\ncom.example.first\r\ncom.example.third\r\n');
  assert.equal(ctx.ommegaEditAppScope(added.replacement, 'com.example.third', true).changed, false);
});

test("per-app scope switch saves only its own entry and honors global scope", async () => {
  const ctx = loadGlue({language:'zh-CN'});
  let contents = 'com.example.other\ncom.example.removed\n';
  const status = {textContent:'',classList:{toggle() {}}};
  ctx.document.getElementById = id => id === 'om-detail-scope-status' ? status : null;
  ctx.Jo = text => text.split(/\r?\n/).map(line=>line.trim().replace(/[!?]+$/,'')).filter(Boolean);
  ctx.Di = () => {};
  ctx.ommegaUpdateAppScope = () => {};
  ctx._ = async command => {
    if (command.includes('ommega_app_writer=')) {
      const request = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], 'base64').toString('utf8'));
      assert.equal(request.expected, contents);
      assert.match(command, /\bwrite-target(?:-security)?$/);
      contents = request.replacement;
      return {errno:0,stdout:'saved'};
    }
    if (command.startsWith('base64 < ')) return {errno:0,stdout:Buffer.from('[filter]\nenabled = true\n').toString('base64')};
    return {errno:0,stdout:'F:' + Buffer.from(contents).toString('base64')};
  };
  ctx.targetFixture = contents;
  ctx.cardFixture = {classList:{toggle() {}}};
  ctx.boxFixture = {checked:false};
  vm.runInContext('ommegaDetail = {pkg:"com.example.app",session:1,card:cardFixture,box:boxFixture}; ommegaTargetSnapshot = targetFixture; ommegaGlobalScopeKnown = true',ctx);
  const input = {checked:true,disabled:false};
  await ctx.ommegaDetailSaveScope(input);
  assert.equal(contents, 'com.example.other\ncom.example.removed\ncom.example.app\n');
  assert.equal(ctx.boxFixture.checked, true);
  assert.match(status.textContent,/已保存/);
});

test("per-app scope switch cannot write when global scope is enabled", async () => {
  const ctx = loadGlue({language:'zh-CN'});
  const status = {textContent:'',classList:{toggle() {}}};
  ctx.document.getElementById = id => id === 'om-detail-scope-status' ? status : null;
  ctx.Jo = text => text.split(/\r?\n/).filter(Boolean);
  ctx._ = async command => {
    assert.doesNotMatch(command, /ommega_app_writer=/);
    return command.startsWith('base64 < ')
      ? {errno:0,stdout:Buffer.from('[filter]\nenabled = false\n').toString('base64')}
      : {errno:0,stdout:'F:' + Buffer.from('com.example.app\n').toString('base64')};
  };
  vm.runInContext('ommegaDetail = {pkg:"com.example.app",session:1}; ommegaTargetSnapshot = "com.example.app\\n"; ommegaGlobalScopeKnown = true',ctx);
  const input = {checked:false,disabled:false};
  await ctx.ommegaDetailSaveScope(input);
  assert.equal(input.checked,true);
  assert.equal(input.disabled,true);
  assert.match(status.textContent,/全局作用域已开启/);
});

test("detail loads explicit app modes without changing their stored values", async () => {
  const ctx = loadGlue({language:'zh-CN'});
  const modes = Object.fromEntries(['global_default','strongbox','tee'].map(value=>[value,{checked:false,disabled:true}]));
  const auth = Object.fromEntries(['auto','compatibility','preserve'].map(value=>[value,{checked:false,disabled:true}]));
  const status = () => ({textContent:'',classList:{toggle() {}}});
  const elements = {'om-detail-mode-status':status(),'om-detail-auth-status':status(),'om-detail-cpu-status':status(),'om-detail-cpu':{},'om-detail-cpu-settings':{}};
  ctx.document.getElementById = id => elements[id] || null;
  ctx.document.querySelector = selector => {
    const match=selector.match(/input\[name="([^"]+)"\]\[value="([^"]+)"\]/);
    return match ? (match[1] === 'om-app-mode' ? modes[match[2]] : auth[match[2]]) : null;
  };
  ctx.document.querySelectorAll = selector => selector.includes('om-app-mode') ? Object.values(modes) : Object.values(auth);
  ctx.ommegaReadAppFile = async () => 'version=1\n[packages]\n"com.example.app"="tee"\n';
  ctx.ommegaAuthRead = async () => ({config:{version:1,default:'auto',apps:{'com.example.app':'preserve'}}});
  ctx.ommegaCpuRead = async () => ({config:{enabled:true,packages:['com.example.app']}});
  vm.runInContext('ommegaDetail = {pkg:"com.example.app",session:7}',ctx);
  await ctx.ommegaDetailLoad(7);
  assert.equal(modes.tee.checked,true);
  assert.equal(auth.preserve.checked,true);
  assert.equal(elements['om-detail-cpu'].checked,true);
  assert.equal(modes.tee.disabled,false);
});

function detailRefreshFixture() {
  const ctx = loadGlue();
  const radios = values => {
    const group = Object.fromEntries(values.map(value => [value, {value, disabled:false, _checked:false}]));
    for (const input of Object.values(group)) Object.defineProperty(input, 'checked', {get() {return this._checked;}, set(value) {
      if (value) Object.values(group).forEach(other => {other._checked=false;}); this._checked=value;
    }});
    return group;
  };
  const modes = radios(['global_default','strongbox','tee']), auth = radios(['auto','compatibility','preserve']);
  modes.tee.checked = true; auth.auto.checked = true;
  const status = () => ({textContent:'', error:false, classList:{toggle(name, value) {this.owner.error=value;}}});
  const elements = Object.fromEntries(['mode','auth','cpu','scope'].map(group => [`om-detail-${group}-status`, status()]));
  Object.values(elements).forEach(element => {element.classList.owner=element;});
  elements['om-detail-cpu'] = {checked:true, disabled:false}; elements['om-detail-cpu-settings'] = {};
  elements['om-detail-scope'] = {checked:true, disabled:false};
  ctx.document.getElementById = id => elements[id] || null;
  ctx.document.querySelector = selector => {
    const match = selector.match(/input\[name="([^"]+)"\]\[value="([^"]+)"\]/);
    return match ? (match[1] === 'om-app-mode' ? modes : auth)[match[2]] : null;
  };
  ctx.document.querySelectorAll = selector => selector.includes('om-app-mode') ? Object.values(modes)
    : selector.includes('om-app-auth') ? Object.values(auth) : [];
  const f = {ctx, modes, auth, elements, target:'com.example.app\n', security:'version=1\n[packages]\n"com.example.app"="tee"\n',
    scope:'[filter]\nenabled=true\n', authConfig:{version:1, default:'auto', apps:{}},
    cpuConfig:{enabled:true, packages:['com.example.app']}, writes:0, gate:null};
  ctx.Jo = raw => raw.split(/\r?\n/).map(value => value.trim()).filter(Boolean);
  ctx.Di = () => {};
  ctx.ommegaReadAppFile = async path => path.endsWith('target-security.toml') ? f.security : f.target;
  ctx.ommegaAuthRead = async () => ({contents:'auth-original', config:structuredClone(f.authConfig)});
  ctx.ommegaCpuRead = async () => ({original:'cpu-original', config:structuredClone(f.cpuConfig)});
  ctx.ommegaReadOnly = async () => ({errno:0, stdout:Buffer.from(f.scope).toString('base64')});
  ctx.ommegaAuthCommit = async (_, config) => {f.writes++; f.authConfig=structuredClone(config); if(f.gate) await f.gate;};
  ctx._ = async command => {
    f.writes++;
    const request = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], 'base64').toString());
    if (command.endsWith('write-target-security')) f.security = request.replacement;
    else if (command.endsWith('write-target')) f.target = request.replacement;
    else f.cpuConfig = {enabled:/^enabled=true$/m.test(request.replacement), packages:[...request.replacement.matchAll(/^package=(.+)$/gm)].map(match=>match[1])};
    if (f.gate) await f.gate;
    return {errno:0, stdout:'saved'};
  };
  ctx.detailBoxFixture = {checked:true}; ctx.detailCardFixture = {classList:{toggle() {}}}; ctx.detailTargetFixture = f.target;
  ctx.Qe = ['com.example.app'];
  vm.runInContext('ommegaDetail = {pkg:"com.example.app",session:7,box:detailBoxFixture,card:detailCardFixture}; ommegaTargetSnapshot = detailTargetFixture; ommegaAppsBaseline = ommegaAppSettingsSnapshot(Qe); ommegaGlobalScopeKnown = true', ctx);
  return f;
}

for (const kind of ['mode','auth','cpu']) test(`detail ${kind} read failures return false and recover without false-success feedback`, async () => {
  const f = detailRefreshFixture(), {ctx} = f;
  const key = kind === 'mode' ? 'ommegaReadAppFile' : kind === 'auth' ? 'ommegaAuthRead' : 'ommegaCpuRead';
  const original = ctx[key];
  ctx[key] = async () => {throw new Error(kind + ' read denied');};
  assert.equal(await ctx.ommegaDetailLoad(7), false);
  assert.match(f.elements[`om-detail-${kind}-status`].textContent, /read denied/);
  assert.equal(kind === 'cpu' ? f.elements['om-detail-cpu'].disabled : Object.values(f[kind === 'mode' ? 'modes' : 'auth']).every(input=>input.disabled), true);
  ctx[key] = original;
  assert.equal(await ctx.ommegaDetailLoad(7), true);
  assert.equal(f.elements[`om-detail-${kind}-status`].error, false);
});

test('malformed detail policy is an incomplete refresh, not an enabled stale choice', async () => {
  const f = detailRefreshFixture(); f.security = 'version=1\n[packages]\n"com.example.app"="unsupported"\n';
  assert.equal(await f.ctx.ommegaDetailLoad(7), false);
  assert.equal(f.modes.tee.disabled, true); assert.equal(f.elements['om-detail-mode-status'].error, true);
});

for (const kind of ['mode','scope','auth','cpu']) for (const completeBeforeRead of [false,true]) {
  test(`detail refresh cannot overwrite ${kind} save ${completeBeforeRead ? 'completed' : 'still active'} during its await`, async () => {
    const f = detailRefreshFixture(), {ctx} = f;
    const original = ctx.ommegaCpuRead;
    let releaseRead, first = true;
    ctx.ommegaCpuRead = () => {
      if (!first) return original();
      first = false; const old = {original:'cpu-old',config:{enabled:true,packages:['com.example.app']}};
      return new Promise(resolve => {releaseRead=()=>resolve(old);});
    };
    const read = ctx.ommegaDetailLoad(7);
    let releaseSave; f.gate = new Promise(resolve => {releaseSave=resolve;});
    let save, control;
    if (kind === 'mode') {control=f.modes.strongbox; control.checked=true; save=ctx.ommegaDetailSaveMode(control);}
    else if (kind === 'scope') {control=f.elements['om-detail-scope']; control.checked=false; save=ctx.ommegaDetailSaveScope(control);}
    else if (kind === 'auth') {control=f.auth.compatibility; control.checked=true; save=ctx.ommegaDetailSaveAuth(control);}
    else {control=f.elements['om-detail-cpu']; control.checked=false; save=ctx.ommegaDetailSaveCpu(control);}
    await new Promise(resolve=>setImmediate(resolve));
    assert.equal(vm.runInContext('ommegaDetail.pending',ctx), 1);
    assert.equal(await ctx.ommegaDetailLoad(7), false, 'no new read starts over a pending mutation');
    if (completeBeforeRead) {releaseSave(); await save;}
    releaseRead(); assert.equal(await read, false);
    if (!completeBeforeRead) {assert.equal(control.disabled, true, 'stale load must not re-enable a saving control'); releaseSave(); await save;}
    assert.equal(vm.runInContext('ommegaDetail.pending',ctx), 0);
    assert.equal(vm.runInContext('ommegaDetail.revision',ctx), 1);
    assert.equal(control.checked, kind === 'mode' || kind === 'auth');
    assert.equal(f.writes, 1, 'refresh must not write or retry the mutation');
  });
}

test('a newer same-session detail read supersedes an older refresh', async () => {
  const f = detailRefreshFixture(), {ctx} = f;
  let release; const original = ctx.ommegaAuthRead;
  ctx.ommegaAuthRead = () => new Promise(resolve => {release=resolve;});
  const old = ctx.ommegaDetailLoad(7);
  ctx.ommegaAuthRead = original; f.authConfig.apps['com.example.app']='preserve';
  assert.equal(await ctx.ommegaDetailLoad(7), true);
  release({config:{version:1,default:'auto',apps:{}}}); assert.equal(await old, false);
  assert.equal(f.auth.preserve.checked, true);
});

test('explicit detail scope refresh reads saved/global state without rebasing list CAS', async () => {
  const f = detailRefreshFixture(), {ctx} = f;
  const baseline = vm.runInContext('ommegaAppsBaseline',ctx), snapshot = vm.runInContext('ommegaTargetSnapshot',ctx);
  f.scope='[filter]\nenabled=false\n';
  assert.equal(await ctx.ommegaDetailLoad(7,true), true);
  assert.equal(vm.runInContext('ommegaGlobalScopeEnabled',ctx), true);
  assert.equal(f.elements['om-detail-scope'].checked,true); assert.equal(f.elements['om-detail-scope'].disabled,true);
  f.scope='[filter]\nenabled=true\n'; f.target='';
  assert.equal(await ctx.ommegaDetailLoad(7,true), false);
  assert.equal(f.elements['om-detail-scope'].checked,false); assert.equal(f.elements['om-detail-scope'].disabled,true);
  assert.match(f.elements['om-detail-scope-status'].textContent,/list.*refresh/i);
  assert.equal(vm.runInContext('ommegaAppsBaseline',ctx),baseline); assert.equal(vm.runInContext('ommegaTargetSnapshot',ctx),snapshot);
  assert.deepEqual(ctx.Qe,['com.example.app']); assert.equal(f.writes,0);
});

test('detail scope read failure returns false without changing its list baseline', async () => {
  const f=detailRefreshFixture(), {ctx}=f;
  ctx.ommegaReadOnly=async()=>({errno:1,stderr:'global scope read denied'});
  assert.equal(await ctx.ommegaDetailLoad(7,true),false);
  assert.match(f.elements['om-detail-scope-status'].textContent,/read denied/); assert.equal(f.elements['om-detail-scope'].disabled,true);
  assert.deepEqual(ctx.Qe,['com.example.app']); assert.equal(f.writes,0);
});

test("CPU app toggle does not activate previously saved unrelated apps", async () => {
  const ctx = loadGlue({language:'zh-CN'});
  const status = {textContent:'',classList:{toggle() {}}};
  ctx.document.getElementById = id => id === 'om-detail-cpu-status' ? status : null;
  const profile = {properties:Object.fromEntries(['ro.product.brand','ro.product.device','ro.product.name','ro.product.manufacturer','ro.product.model','ro.soc.model'].map(k=>[k,'x']))};
  ctx.ommegaCpuRead = async () => ({original:'old',config:{enabled:false,packages:['com.example.other']},profile,engine:'exp6',standalone_active:false});
  let wrote = false;
  ctx._ = async () => {wrote=true; return {errno:0};};
  vm.runInContext('ommegaDetail = {pkg:"com.example.app",session:1}',ctx);
  const box = {checked:true,disabled:false};
  await ctx.ommegaDetailSaveCpu(box);
  assert.equal(wrote,false);
  assert.equal(box.checked,false);
  assert.match(status.textContent,/已有其他保存的应用/);
});

function capabilityFixture(ctx, config = {strongbox:false, device_id_attestation:false}) {
  const elements = {
    "ommega-capability-dialog": {open:true, setAttribute() {}, querySelectorAll:() => []},
    "ommega-capability-status": {}, "ommega-capability-summary": {},
    "ommega-capability-mount-note": {}, "ommega-capability-strongbox": {},
    "ommega-capability-device-id": {},
  };
  const original = "version=1\r\nstrongbox=false\r\ndevice_id_attestation=false\r\n";
  const state = {original, config, mounted:{strongbox:false, device_id_attestation:false}, mount_error:null};
  ctx.document.getElementById = id => elements[id] || null;
  ctx.capabilityFixtureState = state;
  vm.runInContext('ommegaCapabilityState = capabilityFixtureState; ommegaCapabilityDraft = {...capabilityFixtureState.config}', ctx);
  return {elements, state};
}

test("system capabilities save independently and never issue a live apply or restart", () => {
  const ctx = loadGlue();
  for (const strongbox of [false, true]) for (const device_id_attestation of [false, true]) {
    const replacement = ctx.ommegaCapabilityText({strongbox, device_id_attestation});
    const expected = "version=1\r\nstrongbox=false\r\ndevice_id_attestation=false\r\n";
    const command = ctx.ommegaCapabilityCommand("write", {expected, replacement});
    const payload = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], "base64").toString("utf8"));
    assert.equal(payload.expected, expected);
    assert.equal(payload.replacement, `version=1\nstrongbox=${strongbox}\ndevice_id_attestation=${device_id_attestation}\n`);
    assert.doesNotMatch(command, /\b(?:prepare|reboot|restart|kill|mount|setprop)\b|\badb\s|injector\.toml|cpu-mask\.conf/);
  }
  assert.throws(() => ctx.ommegaCapabilityCommand("prepare"), /Unsupported/);
  assert.throws(() => ctx.ommegaCapabilityCommand("write"), /Unsupported/);
  assert.throws(() => ctx.ommegaCapabilityText({strongbox:true, device_id_attestation:"false"}), /Invalid/);
});

test("capability save verifies readback and keeps the restart notice visible in the open dialog", async () => {
  const ctx = loadGlue();
  const f = capabilityFixture(ctx);
  vm.runInContext('ommegaCapabilityDraft.strongbox = true', ctx);
  let calls = 0;
  ctx._ = async command => {
    calls++;
    if (command.endsWith('"$ommega_capability_writer" write')) return {errno:0, stdout:"saved"};
    return {errno:0, stdout:JSON.stringify({...f.state, original:"saved", config:{strongbox:true, device_id_attestation:false}})};
  };
  await ctx.ommegaCapabilitySave();
  assert.equal(calls, 2);
  assert.equal(f.elements["ommega-capability-dialog"].open, true);
  assert.match(f.elements["ommega-capability-status"].textContent, /Saved.*Restart/);
  assert.match(f.elements["ommega-capability-summary"].textContent, /1 \/ 2/);
  assert.equal(vm.runInContext('ommegaCapabilityDraft.device_id_attestation', ctx), false);
});

test("capability write failures and mismatching readbacks retain the user's draft and report failure", async () => {
  for (const writeFails of [true, false]) {
    const ctx = loadGlue();
    const f = capabilityFixture(ctx);
    vm.runInContext('ommegaCapabilityDraft.device_id_attestation = true', ctx);
    ctx._ = async command => command.endsWith('"$ommega_capability_writer" write') ?
      {errno:writeFails ? 1 : 0, stderr:"another editor changed the file"} :
      {errno:0, stdout:JSON.stringify(f.state)};
    await ctx.ommegaCapabilitySave();
    assert.match(f.elements["ommega-capability-status"].textContent, /Could not save/);
    assert.equal(vm.runInContext('ommegaCapabilityDraft.device_id_attestation', ctx), true);
    assert.equal(vm.runInContext('ommegaCapabilityBusy', ctx), false);
    assert.equal(f.elements["ommega-capability-dialog"].open, true);
  }
});

test("capability status distinguishes an absent system mount from an off switch", async () => {
  const ctx = loadGlue();
  const f = capabilityFixture(ctx, {strongbox:true, device_id_attestation:false});
  ctx._ = async () => ({errno:0, stdout:JSON.stringify({...f.state, mounted:null})});
  await ctx.ommegaCapabilityRefresh();
  assert.match(f.elements["ommega-capability-mount-note"].textContent, /not mounted/);
  assert.equal(f.elements["ommega-capability-strongbox"].checked, true);
  assert.equal(f.elements["ommega-capability-device-id"].checked, false);
});

test("a capability read completing after cancellation does not replace a later draft", async () => {
  const ctx = loadGlue();
  const f = capabilityFixture(ctx);
  let finish;
  ctx._ = () => new Promise(resolve => { finish = resolve; });
  const read = ctx.ommegaCapabilityRefresh();
  vm.runInContext('++ommegaCapabilitySession; ommegaCapabilityBusy = false; ommegaCapabilityDraft = {strongbox:true,device_id_attestation:true}', ctx);
  finish({errno:0, stdout:JSON.stringify(f.state)});
  await read;
  assert.equal(vm.runInContext('ommegaCapabilityDraft.strongbox && ommegaCapabilityDraft.device_id_attestation', ctx), true);
});

test("CPU scope is independent, strictly validates packages and preserves source bytes", () => {
  const ctx = loadGlue();
  const config = {enabled:true, packages:["com.user.wallet", "com.example.detector"]};
  const expected = "version=1\nenabled=false\n";
  assert.equal(ctx.ommegaCpuText(config), "version=1\nenabled=true\npackage=com.example.detector\npackage=com.user.wallet\n");
  const command = ctx.ommegaCpuCommand("write", {expected, replacement:ctx.ommegaCpuText(config)});
  const payload = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], "base64").toString("utf8"));
  assert.equal(payload.expected, expected);
  assert.equal(payload.replacement, ctx.ommegaCpuText(config));
  assert.doesNotMatch(command, /injector\.toml|auth-signing|kill|restart|setprop|\badb\s/);
  for (const packages of [["com.safe", "com.safe"], ["com.bad.$(id)"], ["../data"], ["com"]])
    assert.throws(() => ctx.ommegaCpuText({enabled:true, packages}), /Invalid/);
  assert.throws(() => ctx.ommegaCpuText({enabled:"true", packages:[]}), /Invalid/);
  vm.runInContext('ommegaGlobalScopeEnabled = true; ommegaAuthConfig = {version:1,default:"preserve",apps:{}}', ctx);
  assert.equal(ctx.ommegaCpuText(config).includes("package=com.example.detector"), true);
  assert.equal(vm.runInContext('ommegaAuthConfig.default', ctx), "preserve");
});

function cpuAppsFixture(ctx) {
  const dialog = {open:false, showModal() {this.open=true;}, querySelectorAll:()=>[]};
  const elements = {"ommega-cpu-app-dialog":dialog, "ommega-cpu-system":{checked:false},
    "ommega-cpu-search":{value:""}, "ommega-cpu-app-status":{}, "ommega-cpu-more":{},
    "ommega-cpu-app-list":{replaceChildren(){}}};
  ctx.document.getElementById = id => elements[id] || null;
  ctx.ommegaCpuSync = () => {};
  vm.runInContext('ommegaCpuDraft = {enabled:false,packages:["com.system.saved"]}', ctx);
  ctx.ommegaNames = async packages => packages.map(packageName => ({packageName,appName:packageName}));
  let renders = 0;
  ctx.ommegaCpuRenderApps = () => {renders++;};
  return {dialog,elements,renders:()=>renders};
}

test("app icons use the manager scheme without requiring the newer package-info API", () => {
  const ctx = loadGlue();
  const listeners = {};
  const placeholder = {hidden:false};
  const image = {style:{}, previousElementSibling:placeholder, addEventListener:(name,callback)=>{listeners[name]=callback;}};
  ctx.ksu = {exec(){}};
  ctx.ommegaAuthLoadIcon(image, "com.user.app");
  assert.equal(image.src, "ksu://icon/com.user.app");
  listeners.load();
  assert.equal(image.hidden, false); assert.equal(image.style.opacity, "1");
  assert.equal(placeholder.hidden, true, "transparent icons must not overlap a fallback letter");
});

test("a failed app icon scheme falls back to the native drawable before hiding the image", async () => {
  const ctx = loadGlue();
  const listeners = {};
  const image = {style:{}, addEventListener:(name,callback)=>{listeners[name]=callback;}};
  ctx.ksu = {exec(){}}; ctx.dr = new Map();
  ctx.$packageManager = {getApplicationIcon:pkg=>pkg};
  ctx.ks = async () => ({arrayBuffer:async()=>Uint8Array.from([1,2,3]).buffer});
  ctx.Ts = buffer => Buffer.from(buffer).toString("base64");
  ctx.ommegaAuthLoadIcon(image, "com.user.app"); listeners.error();
  await new Promise(resolve=>setImmediate(resolve));
  assert.equal(image.src, "data:image/png;base64,AQID");
  assert.equal(ctx.dr.get("com.user.app"), image.src);
  listeners.load(); assert.equal(image.hidden, false);
  listeners.error(); assert.equal(image.hidden, true);
});

test("CPU scope retains selected apps hidden by the system-app filter", async () => {
  const ctx = loadGlue(), f = cpuAppsFixture(ctx), calls = [];
  ctx.Zo = async type => {calls.push(type); return type === "user" ? ["com.user.wallet"] : ["com.system.saved"];};
  await ctx.ommegaCpuOpenApps();
  assert.deepEqual(calls, ["user"]);
  f.elements["ommega-cpu-system"].checked = true;
  await ctx.ommegaCpuToggleSystem();
  f.elements["ommega-cpu-system"].checked = false;
  await ctx.ommegaCpuToggleSystem();
  assert.deepEqual(calls, ["user", "system"]);
  assert.equal(vm.runInContext('ommegaCpuAppsDraft.has("com.system.saved")', ctx), true);
  assert.equal(vm.runInContext('ommegaCpuDraft.packages.join()', ctx), "com.system.saved");
});

test("CPU scope returning during inventory load never reopens or renders the closed dialog", async () => {
  const ctx = loadGlue(), f = cpuAppsFixture(ctx);
  let resolvePackages;
  ctx.Zo = () => new Promise(resolve => {resolvePackages=resolve;});
  const opening = ctx.ommegaCpuOpenApps();
  f.dialog.open = false;
  resolvePackages(["com.user.wallet"]);
  await opening;
  assert.equal(f.dialog.open, false); assert.equal(f.renders(), 0);
  assert.equal(vm.runInContext('ommegaCpuInventory.length',ctx), 0);
});

test("CPU write failure and mismatching readback keep the dialog open and the draft intact", async () => {
  const ctx = loadGlue();
  let closes = 0;
  const elements = {"ommega-cpu-status":{}, "ommega-cpu-policy-dialog":{close(){closes++;}}};
  ctx.document.getElementById = id => elements[id] || null;
  ctx.ommegaCpuSync = () => {};
  vm.runInContext('ommegaCpuDraft = {enabled:false,packages:["com.new.app"]}; ommegaCpuState = {original:null,config:{enabled:false,packages:[]}}', ctx);
  ctx._ = async () => ({errno:75,stderr:"Config changed; reload"});
  await ctx.ommegaCpuSave();
  assert.match(elements["ommega-cpu-status"].textContent, /Could not save.*Config changed/);
  assert.equal(closes, 0);
  ctx._ = async command => command.endsWith('"$ommega_cpu_writer" status')
    ? {errno:0,stdout:JSON.stringify({original:"version=1\nenabled=false\n",config:{enabled:false,packages:[]},engine:"exp6",standalone_active:false})}
    : {errno:0,stdout:"saved"};
  await ctx.ommegaCpuSave();
  assert.match(elements["ommega-cpu-status"].textContent, /readback did not match/);
  assert.equal(closes, 0);
  assert.equal(vm.runInContext('ommegaCpuDraft.packages.join()', ctx), "com.new.app");
});

test("language changes warn only for real unsaved app selections and modes", async () => {
  const ctx = loadGlue();
  ctx.Qe = ["com.example.wallet", "com.example.checker"];
  ctx.ommegaTargetModes.set("com.example.wallet", "tee");
  const box = {checked:true, closest:()=>({getAttribute:()=>"com.example.wallet"})};
  ctx.se.querySelectorAll = () => [box];
  assert.equal(ctx.ommegaAppsHaveChanges(), false, "initial load has no draft");
  await ctx.Mi();
  assert.equal(ctx.ommegaAppsHaveChanges(), false, "unrendered or filtered cards are not edits");
  ctx.Qe.reverse();
  assert.equal(ctx.ommegaAppsHaveChanges(), false, "sorting is not an edit");
  box.checked = false;
  assert.equal(ctx.ommegaAppsHaveChanges(), true);
  box.checked = true;
  assert.equal(ctx.ommegaAppsHaveChanges(), false, "reverting a selection is clean");
  ctx.ommegaTargetModes.set("com.example.wallet", "strongbox");
  assert.equal(ctx.ommegaAppsHaveChanges(), true);
  ctx.ommegaTargetModes.set("com.example.wallet", "tee");
  assert.equal(ctx.ommegaAppsHaveChanges(), false);
  ctx.ommegaTargetModes.set("com.unselected.app", "strongbox");
  assert.equal(ctx.ommegaAppsHaveChanges(), false, "unselected modes are not saved");
  ctx.Dl.add("com.retained.app");
  assert.equal(ctx.ommegaAppsHaveChanges(), true, "retained scope selections count");
});

test("global compatibility saves independently of unsaved app selections", async () => {
  const ctx = loadGlue();
  ctx.Qe = ["com.example.wallet"];
  await ctx.Mi();
  ctx.Dl.add("com.example.unsaved");
  const box = {checked:true, disabled:false}, status = {};
  ctx.document.getElementById = id => id === "ommega-global-compat" ? box : id === "ommega-global-compat-status" ? status : null;
  let current = null;
  ctx._ = async command => {
    if (command.includes("ommega_compat_current=")) { current = "[positive_key_id]\nenabled=true\n"; return {errno:0, stdout:"saved"}; }
    return {errno:0, stdout:current === null ? "M:" : "F:" + Buffer.from(current).toString("base64")};
  };
  await ctx.ommegaGlobalCompatChange();
  assert.equal(box.checked, true);
  assert.equal(box.disabled, false);
  assert.match(status.textContent, /Saved/);
  assert.equal(ctx.ommegaAppsHaveChanges(), true, "saving compatibility must not discard scope drafts");
});

test("global compatibility reports write failure and restores the actual state", async () => {
  const ctx = loadGlue();
  const box = {checked:true, disabled:false}, status = {};
  ctx.document.getElementById = id => id === "ommega-global-compat" ? box : id === "ommega-global-compat-status" ? status : null;
  ctx._ = async command => command.includes("ommega_compat_current=")
    ? {errno:1, stderr:"write failed"}
    : {errno:0, stdout:"F:" + Buffer.from("[positive_key_id]\nenabled=false\n").toString("base64")};
  await ctx.ommegaGlobalCompatChange();
  assert.equal(box.checked, false);
  assert.equal(box.disabled, false);
  assert.match(status.textContent, /Save failed/);
});

test("compatibility updates preserve other settings and compare the original bytes", () => {
  const ctx = loadGlue();
  const original = 'version = 1\r\n[positive_key_id]\r\npackages = ["com.old.app"]\r\n[other]\r\nvalue = "keep"\r\n';
  const updated = ctx.ommegaTargetCompatUpdate(original, false);
  assert.match(updated, /enabled = false\r\n/);
  assert.match(updated, /packages = \[\]\r\n/);
  assert.match(updated, /\[other\]\r\nvalue = "keep"/);
  assert.equal(ctx.ommegaParseTargetCompat(updated), false);
  const command = ctx.ommegaCompatWriteCommand(original, false);
  assert.ok(command.includes("F:" + Buffer.from(original).toString("base64")));
  assert.match(command, /Config changed; retry/);
  assert.throws(() => ctx.ommegaTargetCompatUpdate('[positive_key_id]\npackages = [\n"a.b"\n]\n', false), /Unsupported/);
  const quoted = '["positive_key_id"]\r\n"enabled" = true # keep\r\n\'packages\' = ["com.old.app"] # keep\r\n';
  const disabled = ctx.ommegaTargetCompatUpdate(quoted, false);
  assert.equal(disabled, '["positive_key_id"]\r\n"enabled" = false # keep\r\n\'packages\' = [] # keep\r\n');
  assert.equal(ctx.ommegaParseTargetCompat(disabled), false);
  assert.equal(ctx.ommegaParseTargetCompat(quoted), true);
});

test("auth policy round-trips default and app exceptions and rejects ambiguous configs", () => {
  const ctx = loadGlue();
  const empty = ctx.ommegaAuthParse(null);
  assert.equal(empty.default, "auto");
  assert.equal(Object.keys(empty.apps).length, 0);
  const config = ctx.ommegaAuthParse('version = 1\r\ndefault = "preserve"\r\n[apps]\r\n"com.example.bank" = "compatibility"\r\n"com.example.probe" = "auto"\r\n');
  assert.equal(config.apps["com.example.bank"], "compatibility");
  assert.equal(ctx.ommegaAuthToml(ctx.ommegaAuthParse(ctx.ommegaAuthToml(config))), ctx.ommegaAuthToml(config));
  for (const invalid of ['', 'version=2\ndefault="auto"', 'version=1\ndefault="auto"\ndefault="preserve"',
    'version=1\ndefault="auto"\n[apps]\n"com.example.bank"="bad"',
    'version=1\ndefault="auto"\n[unknown]', 'version=1\ndefault="auto"\n[apps]\n"com.example.bank"="auto"\n"com.example.bank"="preserve"']) {
    assert.throws(() => ctx.ommegaAuthParse(invalid));
  }
});

test("auth writer transports exact source bytes and validated replacement to the Rust writer", () => {
  const ctx = loadGlue();
  const before = '# 配置\r\nversion = 1\r\ndefault = "auto"\r\n';
  const config = ctx.ommegaAuthParse(before);
  config.apps["com.example.bank"] = "compatibility";
  const command = ctx.ommegaAuthWriteCommand(before, config);
  assert.match(command, /ommega-auth-policy/);
  assert.doesNotMatch(command, /target\.txt|injector\.toml|target-security\.toml|kill|restart/);
  const payload = JSON.parse(Buffer.from(command.match(/printf '%s' '([^']+)'/)[1], "base64").toString("utf8"));
  assert.equal(payload.expected, before);
  assert.equal(payload.replacement, ctx.ommegaAuthToml(config));
  config.apps['com.example.$(whoami)'] = "preserve";
  assert.throws(() => ctx.ommegaAuthWriteCommand(before, config), /Invalid/);
});

function authAppsFixture(ctx) {
  const dialog = {open:false, showModal() {this.open=true;}, querySelectorAll:()=>[]};
  const elements = {
    "ommega-auth-dialog":dialog, "ommega-auth-system":{checked:false},
    "ommega-auth-search":{value:""}, "ommega-auth-app-status":{},
    "ommega-auth-app-list":{replaceChildren() {}}, "ommega-auth-more":{hidden:false},
  };
  ctx.document.getElementById = id=>elements[id] || null;
  vm.runInContext('ommegaAuthConfig = {version:1, default:"auto", apps:{"com.system.service":"preserve"}}',ctx);
  ctx.ommegaNames = async packages=>packages.map(packageName=>({packageName,appName:packageName}));
  let renders=0;
  ctx.ommegaAuthRenderApps = ()=>{renders++;};
  return {elements,dialog,renders:()=>renders};
}

test("exceptions load user apps first and keep hidden system overrides across filtering", async () => {
  const ctx=loadGlue(), f=authAppsFixture(ctx), calls=[];
  ctx.Zo=async type=>{calls.push(type);return type==="user"?["com.user.wallet"]:["com.system.service"];};
  await ctx.ommegaAuthOpenApps();
  assert.deepEqual(calls,["user"]);
  assert.equal(ctx.ommegaAuthFilteredApps("").map(app=>app.packageName).join(),"com.user.wallet");
  f.elements["ommega-auth-system"].checked=true;
  await ctx.ommegaAuthToggleSystem();
  assert.deepEqual(calls,["user","system"]);
  assert.equal(ctx.ommegaAuthFilteredApps("SYSTEM").map(app=>app.packageName).join(),"com.system.service");
  f.elements["ommega-auth-system"].checked=false;
  await ctx.ommegaAuthToggleSystem();
  assert.equal(ctx.ommegaAuthFilteredApps("system").length,0);
  assert.equal(vm.runInContext('ommegaAuthAppsDraft["com.system.service"]',ctx),"preserve");
  assert.equal(vm.runInContext('ommegaAuthConfig.apps["com.system.service"]',ctx),"preserve");
});

test("returning before the app inventory completes does not render or reopen the closed page", async () => {
  const ctx=loadGlue(), f=authAppsFixture(ctx);
  let resolvePackages;
  ctx.Zo=()=>new Promise(resolve=>{resolvePackages=resolve;});
  const opening=ctx.ommegaAuthOpenApps();
  f.dialog.open=false;
  resolvePackages(["com.user.wallet"]);
  await opening;
  assert.equal(f.dialog.open,false); assert.equal(f.renders(),0);
  assert.equal(vm.runInContext('ommegaAuthInventory.length',ctx),0);
});

test("auth read preserves newline bytes, distinguishes missing files, and fails on unreadable files", async () => {
  const ctx = loadGlue();
  const contents = '# 配置\r\nversion=1\r\ndefault="auto"\r\n';
  ctx._ = async () => ({errno: 0, stdout: 'F:' + Buffer.from(contents).toString("base64")});
  assert.equal((await ctx.ommegaAuthRead()).contents, contents);
  ctx._ = async () => ({errno: 0, stdout: 'M:'});
  assert.equal((await ctx.ommegaAuthRead()).contents, null);
  ctx._ = async () => ({errno: 1, stderr: 'permission denied'});
  await assert.rejects(ctx.ommegaAuthRead(), /permission denied/);
});

test("global scope leaves auth exceptions enabled and failed writes never report success", async () => {
  const ctx = loadGlue({language: "zh-CN"});
  const authButton = {};
  const elements = {
    "ommega-auth-signing": {querySelectorAll: () => [authButton]},
    "ommega-auth-apps": authButton, "ommega-auth-status": {},
    "ommega-global-scope-enabled": {}, "ommega-global-scope-status": {},
    "apps-list": {style: {}, setAttribute() {}}, "save": {},
  };
  ctx.document.getElementById = id => elements[id] || null;
  vm.runInContext('ommegaAuthConfig = ommegaAuthParse(null); ommegaGlobalScopeKnown = true; ommegaGlobalScopeEnabled = true; ommegaScopeSync(); ommegaAuthSync()', ctx);
  assert.equal(elements["apps-list"].inert, false);
  assert.equal(authButton.disabled, false);
  ctx._ = async () => ({errno: 75, stderr: 'Config changed; reload before saving'});
  await ctx.ommegaAuthSaveDefault();
  assert.match(elements["ommega-auth-status"].textContent, /保存失败.*Config changed/);
  assert.doesNotMatch(elements["ommega-auth-status"].textContent, /已保存/);
  assert.equal(authButton.disabled, false);
});

test("auth save verifies readback and preserves unrelated app exceptions", async () => {
  const ctx = loadGlue();
  const config = {version: 1, default: "compatibility", apps: {"com.example.probe": "preserve"}};
  const body = ctx.ommegaAuthToml(config);
  let calls = 0;
  ctx._ = async () => ++calls === 1 ? {errno: 0} : {errno: 0, stdout: 'F:' + Buffer.from(body).toString("base64")};
  await ctx.ommegaAuthCommit(null, config);
  assert.equal(vm.runInContext('ommegaAuthConfig.apps["com.example.probe"]', ctx), "preserve");
  calls = 0;
  ctx._ = async () => ++calls === 1 ? {errno: 0} : {errno: 0, stdout: 'M:'};
  await assert.rejects(ctx.ommegaAuthCommit(body, config), /readback/);
});


test("global scope toggles only filter.enabled and restores the saved configuration", () => {
  const ctx = loadGlue();
  const original = [
    "version = 1",
    'scoop = ["com.saved"]',
    "[main]",
    "enabled = true",
    "[filter]",
    "enabled = true # selected apps",
    'deny_packages = ["com.denied"]',
    'system_app_contexts = { "com.factory" = "u:r:factory_app:s0" }',
    "[intercept]",
    "get_security_level = true",
    "",
  ].join("\n");
  const global = ctx.ommegaScopeConfig(original, true);
  assert.equal(ctx.ommegaScopeSetting(global).global, true);
  assert.equal(global, original.replace("enabled = true # selected apps", "enabled = false # selected apps"));
  assert.equal(ctx.ommegaScopeConfig(global, false), original);
  assert.equal(ctx.ommegaScopeSetting(original).global, false);
});

test("scope editing supports defaults, quoted tables, CRLF and rejects ambiguous input", () => {
  const ctx = loadGlue();
  for (const source of [
    "version = 1\n[main]\nenabled = true\n",
    "version = 1\n[filter]\ndeny_packages = []\n",
    "version = 1\r\n[ 'filter' ] # scope\r\n 'enabled' = true # active\r\n",
  ]) {
    const global = ctx.ommegaScopeConfig(source, true);
    assert.equal(ctx.ommegaScopeSetting(global).global, true);
    assert.equal(ctx.ommegaScopeSetting(ctx.ommegaScopeConfig(global, false)).global, false);
  }
  assert.throws(() => ctx.ommegaScopeConfig("[filter]\nenabled = 1", true), /Invalid/);
  assert.throws(() => ctx.ommegaScopeConfig("[filter]\nenabled = true\nenabled = false", true), /Duplicate/);
  assert.throws(() => ctx.ommegaScopeConfig('[main]\nnotes = """\n[filter]\n"""', true), /Multiline/);
});

test("scope writes are atomic, detect stale input and never rewrite the target list", () => {
  const ctx = loadGlue();
  const original = "version = 1\n[filter]\nenabled = true\n";
  const command = ctx.ommegaScopeWriteCommand(original, true);
  assert.match(command, /injector\.toml/);
  assert.match(command, /Config changed; retry/);
  assert.match(command, /exit 75/);
  assert.match(command, /\.scope\.\$\$/);
  assert.match(command, /trap/);
  assert.match(command, /chown 1017:1017/);
  assert.match(command, /mv -f/);
  assert.doesNotMatch(command, /target\.txt|target-security\.toml|target-compat\.toml/);
  const encoded = command.match(/printf '%s' '([^']+)'/)[1];
  assert.equal(Buffer.from(encoded, "base64").toString("utf8"), ctx.ommegaScopeConfig(original, true));
});

test("global scope keeps app details available but disables bulk scope selection", () => {
  const ctx = loadGlue({language: "zh-CN"});
  const elements = {
    "ommega-global-scope-enabled": {},
    "ommega-global-scope-status": {},
    "apps-list": {style: {}, setAttribute() {}},
    "save": {},
    "select-all": {}, "deselect-all": {}, "deselect-unnecessary": {}, "add-system-app": {},
  };
  ctx.document.getElementById = id => elements[id] || null;
  vm.runInContext("ommegaGlobalScopeKnown = true; ommegaGlobalScopeEnabled = true; ommegaScopeSync()", ctx);
  assert.equal(elements["apps-list"].inert, false);
  assert.equal(elements["select-all"].disabled, true);
  assert.equal(elements["deselect-all"].disabled, true);
  assert.equal(elements["deselect-unnecessary"].disabled, false);
  assert.equal(elements["add-system-app"].disabled, false);
  assert.equal(elements.save.disabled, false);
  assert.match(elements["ommega-global-scope-status"].textContent, /用户应用和普通系统应用.*排除核心系统服务/);
  vm.runInContext("ommegaGlobalScopeEnabled = false; ommegaScopeSync()", ctx);
  assert.equal(elements["apps-list"].inert, false);
  assert.equal(elements["select-all"].disabled, false);
  assert.equal(elements.save.disabled, false);
});

test("a failed scope write restores the readback state and reports failure", async () => {
  const ctx = loadGlue({language: "zh-CN"});
  const elements = {
    "ommega-global-scope-enabled": {checked: true},
    "ommega-global-scope-status": {},
    "apps-list": {style: {}, setAttribute() {}},
    "save": {},
  };
  ctx.document.getElementById = id => elements[id] || null;
  vm.runInContext("ommegaGlobalScopeKnown = true", ctx);
  ctx._ = async command => command.startsWith("base64 < ")
    ? {errno: 0, stdout: Buffer.from("[filter]\nenabled = true\n").toString("base64")}
    : {errno: 75, stderr: "Config changed; retry"};
  await ctx.ommegaApplyGlobalScope();
  assert.equal(elements["ommega-global-scope-enabled"].checked, false);
  assert.equal(elements["ommega-global-scope-enabled"].disabled, false);
  assert.equal(elements["apps-list"].inert, false);
  assert.match(elements["ommega-global-scope-status"].textContent, /保存失败.*Config changed/);
  assert.doesNotMatch(elements["ommega-global-scope-status"].textContent, /Could not save scope/);
});

// KernelSU's WebViewInterface.exec uses result.out.joinToString("\n"). Each
// Shell output element is a line without its terminator, including CR in CRLF.
// Execute the generated commands in a real shell, then apply that bridge
// contract rather than handing the UI a byte-exact mocked cat result.
function scopeShell(command) {
  const file = process.platform === "win32" ? "wsl.exe" : "sh";
  const args = process.platform === "win32" ? ["-d", "Ubuntu", "--", "sh"] : [];
  const result = spawnSync(file, args, {input: command, encoding: "utf8"});
  if (result.error) throw result.error;
  const bridgeLines = value => {
    const lines = value.split(/\r\n|[\r\n]/);
    if (/[\r\n]$/.test(value)) lines.pop();
    return lines.join("\n");
  };
  return {errno: result.status, stdout: bridgeLines(result.stdout), stderr: bridgeLines(result.stderr)};
}

test("scope toggles preserve real LF/CRLF files through the line-based Android exec bridge", async () => {
  for (const ending of ["\n", "\r\n"]) {
    const directory = scopeShell("mktemp -d /tmp/ommega-scope-test.XXXXXX");
    assert.equal(directory.errno, 0, directory.stderr);
    assert.match(directory.stdout, /^\/tmp\/ommega-scope-test\.[A-Za-z0-9]+$/);
    const path = directory.stdout + "/injector.toml";
    const ctx = loadGlue({scopeConfig: path});
    const original = ["# 配置保持原样", "version = 1", "[filter]", "enabled = true # selected", 'deny_packages = ["com.saved"]', ""].join(ending);
    const initialize = scopeShell("printf '%s' '" + Buffer.from(original).toString("base64") + "' | base64 -d > '" + path + "'");
    assert.equal(initialize.errno, 0, initialize.stderr);
    const execute = command => scopeShell(command.replace("chown 1017:1017", 'chown "$(id -u):$(id -g)"'));
    const elements = {
      "ommega-global-scope-enabled": {checked: false},
      "ommega-global-scope-status": {},
      "apps-list": {style: {}, setAttribute() {}},
      "save": {},
    };
    ctx.document.getElementById = id => elements[id] || null;
    ctx._ = async command => execute(command);
    try {
      // Prove the previous raw-cat protocol produces the reported failure even
      // when nobody changes the file between reading and writing it.
      const legacy = execute('cat "' + path + '"');
      assert.notEqual(legacy.stdout, original);
      assert.equal(execute(ctx.ommegaScopeWriteCommand(legacy.stdout, true)).errno, 75);

      assert.equal(await ctx.ommegaScopeRead(), original);
      elements["ommega-global-scope-enabled"].checked = true;
      await ctx.ommegaApplyGlobalScope();
      assert.equal(elements["ommega-global-scope-enabled"].checked, true);
      assert.equal(elements["apps-list"].inert, false);
      assert.doesNotMatch(elements["ommega-global-scope-status"].textContent, /Could not/);
      assert.equal(await ctx.ommegaScopeRead(), ctx.ommegaScopeConfig(original, true));

      elements["ommega-global-scope-enabled"].checked = false;
      await ctx.ommegaApplyGlobalScope();
      assert.equal(elements["apps-list"].inert, false);
      assert.equal(await ctx.ommegaScopeRead(), original);

      // Keep the concurrent-change guard: real newer content must survive.
      const snapshot = await ctx.ommegaScopeRead();
      assert.equal(execute("printf '# concurrent edit\\n' >> '" + path + "'").errno, 0);
      const concurrent = await ctx.ommegaScopeRead();
      assert.equal(execute(ctx.ommegaScopeWriteCommand(snapshot, true)).errno, 75);
      assert.equal(await ctx.ommegaScopeRead(), concurrent);
      assert.equal(execute("find '" + directory.stdout + "' -name '*.scope.*'").stdout, "");
    } finally {
      assert.equal(scopeShell("rm -f '" + path + "'; rmdir '" + directory.stdout + "'").errno, 0);
    }
  }
});

test("scope reads preserve UTF-8 bytes, blank lines and an unterminated final line", async () => {
  const ctx = loadGlue();
  const original = '\ufeff# 配置\r\n\r\n[filter]\r\nenabled = true';
  ctx._ = async () => ({errno: 0, stdout: Buffer.from(original).toString("base64").match(/.{1,32}/g).join("\n")});
  assert.equal(await ctx.ommegaScopeRead(), original);
});

test("scope text follows Chinese, English and system-default language selections", async () => {
  for (const [language, preference, chinese] of [
    ["zh-CN", null, true],
    ["zh-CN", "default", true],
    ["zh-CN", "en", false],
    ["en-US", "zh-CN", true],
    ["en-US", "default", false],
  ]) {
    const ctx = loadGlue({language, preference});
    const panel = {style: {}};
    const elements = {
      "om-global-scope-slot": {appendChild(child) { assert.equal(child, panel); }},
      "ommega-global-scope-enabled": {addEventListener() {}},
      "ommega-global-scope-status": {},
      "apps-list": {style: {}, setAttribute() {}, parentElement: {insertBefore() {}}},
      "save": {},
    };
    ctx.document.getElementById = id => elements[id] || null;
    ctx.document.createElement = () => panel;
    ctx._ = async () => ({errno: 0, stdout: Buffer.from("[filter]\nenabled = true\n").toString("base64")});
    assert.equal(ctx.ommegaInstallGlobalScope(), true);
    await ctx.ommegaRefreshGlobalScope();
    const status = elements["ommega-global-scope-status"].textContent;
    assert.match(status, chinese ? /已关闭/ : /^Off:/);
    assert.doesNotMatch(status, chinese ? /Off:/ : /已关闭/);
    assert.match(panel.innerHTML, chinese ? /<strong>全局作用域<\/strong>/ : /<strong>Global scope<\/strong>/);
    assert.doesNotMatch(panel.innerHTML, chinese ? /Global scope|Loading/ : /全局作用域|正在读取/);
    assert.match(panel.innerHTML, chinese ? /正在读取…<\/div>/ : /Loading…<\/div>/);
    vm.runInContext("ommegaGlobalScopeEnabled = true; ommegaScopeSync()", ctx);
    const enabledStatus = elements["ommega-global-scope-status"].textContent;
    assert.match(enabledStatus, chinese ? /排除核心系统服务/ : /core system services are excluded/);
    assert.doesNotMatch(enabledStatus, chinese ? /core system services/ : /核心系统服务/);
  }
});


test("a policy file round-trips through the glue", () => {
  const context = loadGlue();
  const serialized = context.ommegaTargetCompatToml(true);
  assert.match(serialized, /^version = 1/m);
  assert.match(serialized, /^\[positive_key_id\]$/m);
  assert.match(serialized, /^enabled = true$/m);
  assert.equal(context.ommegaParseTargetCompat(serialized), true);
  assert.equal(context.ommegaParseTargetCompat(context.ommegaTargetCompatToml(false)), false);
});

test("an absent, empty or unrelated policy keeps the switch off", () => {
  const context = loadGlue();
  assert.equal(context.ommegaParseTargetCompat(""), false);
  assert.equal(context.ommegaParseTargetCompat("version = 1\n"), false);
  assert.equal(
    context.ommegaParseTargetCompat("[positive_key_id]\nenabled = false\n"),
    false,
  );
  assert.equal(
    context.ommegaParseTargetCompat("[positive_key_id]\npackages = []\n"),
    false,
  );
  assert.equal(
    context.ommegaParseTargetCompat("[something_else]\npackages = [\"x\"]\n"),
    false,
  );
  // A policy written before the switch became global still enables it.
  assert.equal(
    context.ommegaParseTargetCompat("[positive_key_id]\npackages = [\"x\"]\n"),
    true,
  );
});

// The relay verdict is recorded by the keymint daemon in remote-health.json.
// This bundle never read it, so a server-side 401 or 429 stayed invisible and
// users retried the "enable remote" checkbox instead.
test("the relay verdict reaches the dialog the user opens", () => {
  assert.ok(
    source.includes("OMMEGA_REMOTE_HEALTH"),
    "the glue must read the daemon's remote-health.json",
  );
  assert.ok(
    source.includes('if (typeof loadRemoteConfig === "function")'),
    "the hook must be guarded so this harness can evaluate the glue",
  );
  const context = loadGlue();
  assert.equal(typeof context.ommegaHealthReport, "function");
  assert.equal(typeof context.ommegaRefreshHealth, "function");
  assert.equal(typeof context.ommegaHealthPanel, "function");
});

test("a recorded failure names the cause instead of leaving it to guesswork", () => {
  const context = loadGlue();
  const rendered = context.ommegaHealthReport({
    last_ok_unix: 1_700_000_000,
    last_error_unix: 1_700_000_600,
    last_status: 429,
    last_kind: "throttled",
    last_message: "rate limit exceeded",
  });
  assert.match(rendered, /Last relay failure/);
  assert.match(rendered, /429/);
  assert.match(rendered, /rate limit exceeded/);
  // The advice must tell the user that toggling the checkbox is pointless.
  assert.match(rendered, /It recovers on its own/);

  const rejected = context.ommegaHealthReport({
    last_ok_unix: null,
    last_error_unix: 1_700_000_600,
    last_status: 401,
    last_kind: "unauthorized",
    last_message: "missing or invalid X-Relay-Token",
  });
  assert.match(rejected, /rejected this token/);
  assert.match(rejected, /will not change the outcome/);

  const unreachable = context.ommegaHealthReport({
    last_ok_unix: 0,
    last_error_unix: 1_700_000_600,
    last_status: null,
    last_kind: "transport",
    last_message: "connection timed out",
  });
  assert.match(unreachable, /Could not reach the server/);
});

test("a recovered relay stops being reported as failing", () => {
  const context = loadGlue();
  // The daemon keeps the last error alongside the last success, so a later
  // success must win: otherwise the panel would report a fixed outage forever.
  const recovered = context.ommegaHealthReport({
    last_ok_unix: 1_700_000_900,
    last_error_unix: 1_700_000_600,
    last_status: 429,
    last_kind: "throttled",
    last_message: "rate limit exceeded",
  });
  assert.match(recovered, /status: OK/);
  assert.doesNotMatch(recovered, /failing/);
});

test("nothing recorded yet is not the same as a clean bill of health", () => {
  const context = loadGlue();
  assert.equal(context.ommegaHealthReport(null), "");
  assert.equal(context.ommegaHealthReport({}), "");
  assert.equal(context.ommegaHealthReport({ last_ok_unix: 0, last_error_unix: 0 }), "");
});

// The software Soter TA switch: the panel writes a flag file and reads the status
// the module's watchdog publishes. Both are pure file operations, which is why
// this section can be exercised without the WebUI or a device.
test("the Soter switch is part of the shipped bundle", () => {
  assert.ok(
    source.includes("OMMEGA_SOTERTA_FLAG"),
    "the glue must own the enable flag",
  );
  assert.ok(
    source.includes("OMMEGA_SOTERTA_STATUS") && source.includes("/status.json"),
    "the glue must read the published status",
  );
  assert.ok(
    source.includes("ommegaSotertaInstallMenu();"),
    "the menu entry has to be installed",
  );
  const context = loadGlue();
  for (const name of [
    "ommegaSotertaSwitchCommand",
    "ommegaSotertaReport",
    "ommegaSotertaParseJson",
    "ommegaSotertaStale",
  ]) {
    assert.equal(typeof context[name], "function", name);
  }
});

test("disabling only removes the flag, enabling writes it atomically", () => {
  const context = loadGlue();
  const off = context.ommegaSotertaSwitchCommand(false);
  assert.match(off, /^rm -f "/);
  assert.match(off, /\/data\/adb\/ommega\/soterta\/enabled/);
  assert.doesNotMatch(off, />/, "disabling must not write anything");

  const on = context.ommegaSotertaSwitchCommand(true);
  assert.match(on, /mkdir -p/);
  assert.match(on, /enabled\.tmp/);
  assert.match(on, /mv /);
});

test("the switch state is reported honestly", () => {
  const context = loadGlue();
  const now = Math.floor(Date.now() / 1000);

  const off = context.ommegaSotertaReport(true, false, {owner: "hal", hal: "running"});
  assert.match(off, /Switch: off/);
  assert.match(off, /local checks fail/);

  const pending = context.ommegaSotertaReport(true, true, null);
  assert.match(pending, /waiting for the watchdog/);

  const running = context.ommegaSotertaReport(true, true, {
    enabled: 1,
    running: 1,
    pid: 1234,
    mode: "answer",
    hal: "stopped",
    owner: "us",
    device_id: "0000000030ce4217e0ffe879abc0811f",
    ledger: 1,
    failures: 0,
    updated: now,
    note: "",
  });
  assert.match(running, /running \(mode=answer, pid=1234\)/);
  assert.match(running, /service name taken over/);
  assert.match(running, /0000000030ce4217e0ffe879abc0811f/);
  assert.doesNotMatch(running, /may be down/);

  // A watchdog that stopped reporting must not read as healthy.
  const stale = context.ommegaSotertaReport(true, true, {
    running: 1,
    mode: "answer",
    pid: 1,
    hal: "stopped",
    owner: "us",
    updated: now - 600,
  });
  assert.match(stale, /may be down/);

  const failed = context.ommegaSotertaReport(true, true, {
    running: 0,
    hal: "running",
    owner: "hal",
    failures: 2,
    updated: now,
    note: "software TA failed to start",
  });
  assert.match(failed, /Start failures: 2/);
  assert.match(failed, /software TA failed to start/);

  // A moved device id has to be visible: every client that remembers the old
  // one would treat the device as new.
  const drifted = context.ommegaSotertaReport(true, true, {
    running: 1,
    mode: "answer",
    pid: 7,
    hal: "stopped",
    owner: "us",
    device_id: "00000000deadbeefdeadbeefdeadbeef",
    ledger: 1,
    updated: now,
    id_changed: 1,
    id_note: "0000000030ce4217e0ffe879abc0811f -> 00000000deadbeefdeadbeefdeadbeef",
  });
  assert.match(drifted, /device id changed/);
  assert.match(drifted, /deadbeef/);

  const unreadable = context.ommegaSotertaReport(false, false, null);
  assert.match(unreadable, /Could not read the switch state/);
});

test("A app refresh uses installed packages, keeps uninstalled target entries and never writes", async () => {
  const ctx=loadGlue();
  let rendered, boxes=[], counts=0;
  const makeBox=(pkg,checked)=>({checked,closest:()=>({getAttribute:()=>pkg})});
  ctx.se.querySelectorAll=()=>boxes;
  ctx.document.getElementById=()=>null;
  ctx.document.querySelectorAll=()=>[];
  ctx.window={scrollY:70,scrollTo:(x,y)=>assert.equal(y,70)};
  ctx.requestAnimationFrame=fn=>fn();
  ctx.Event=class {};
  ctx.le={value:'wallet',dispatchEvent:()=>{}};
  ctx.$s=apps=>{rendered=apps;boxes=apps.map(app=>makeBox(app.packageName,ctx.Qe.includes(app.packageName)));};
  ctx.ommegaRefreshCertificateCounts=async()=>{counts++;};
  const target='com.example.wallet\ncom.example.removed\n';
  const commands=[];
  ctx._=async command=>{
    commands.push(command);
    if(command.startsWith('pm list packages'))return {errno:0,stdout:'package:com.example.wallet\npackage:com.example.unscoped\n'};
    if(command.includes('injector.toml'))return {errno:0,stdout:Buffer.from('[filter]\nenabled = true\n').toString('base64')};
    if(command.includes('target-security.toml'))return {errno:0,stdout:'M:'};
    if(command.includes('system_app'))return {errno:0,stdout:'M:'};
    if(command.includes('target.txt'))return {errno:0,stdout:'F:'+Buffer.from(target).toString('base64')};
    return {errno:0,stdout:'M:'};
  };
  assert.equal(await ctx.ommegaRefreshApps(),'');
  assert.ok(rendered.some(app=>app.packageName==='com.example.wallet'));
  assert.ok(rendered.some(app=>app.packageName==='com.example.unscoped'),'apps outside target.txt must still be listed');
  assert.ok(!rendered.some(app=>app.packageName==='com.example.removed'),'uninstalled packages must not be listed');
  assert.deepEqual([...ctx.Qe],['com.example.wallet','com.example.removed']);
  assert.equal(ctx.le.value,'wallet');
  assert.equal(vm.runInContext('ommegaTargetSnapshot',ctx),target);
  assert.equal(vm.runInContext('ommegaAppsHaveChanges()',ctx),false);
  assert.equal(counts,1);
  assert.ok(commands.includes('pm list packages -3'));
  assert.ok(commands.every(command=>! /write-target|ommega_app_writer| save|mv |touch |rm /.test(command)));
});

test("app names recover from a failed label batch instead of showing package ids", async () => {
  const ctx=loadGlue();
  const calls=[];
  ctx.ksu={getPackagesInfo:raw=>{
    const list=JSON.parse(raw);calls.push(list.length);
    if(list.length>1)throw new Error('batch rejected');
    return JSON.stringify(list.map(pkg=>({packageName:pkg,appLabel:pkg==='id.my.pjm.qbcd_okr_dvii'?'xka wl rl':'App '+pkg})));
  }};
  const result=await ctx.ommegaAppNames(['id.my.pjm.qbcd_okr_dvii','com.example.other'],{});
  assert.equal(result.find(app=>app.packageName==='id.my.pjm.qbcd_okr_dvii').appName,'xka wl rl');
  assert.equal(result.find(app=>app.packageName==='com.example.other').appName,'App com.example.other');
  assert.ok(calls.includes(1),'failed batches are retried one package at a time');
});

test("settings refresh disables compatibility while reading and rejects a superseded snapshot", async () => {
  const ctx=loadGlue();
  const box={disabled:false,checked:false};
  ctx.document.getElementById=id=>id==='ommega-global-compat'?box:null;
  let release;
  ctx.ommegaReadTargetCompat=()=>new Promise(resolve=>{release=resolve;});
  const pending=ctx.ommegaRefreshActiveView('settings');
  assert.equal(box.disabled,true);
  vm.runInContext('++ommegaCompatRevision',ctx);
  release({enabled:true});
  await assert.rejects(pending,/Settings changed/);
  assert.equal(box.checked,false);assert.equal(box.disabled,false);
});

test("A app refresh read failures leave the existing list and release the button", async () => {
  const ctx=loadGlue();
  const attrs={};const button={disabled:false,setAttribute:(key,value)=>{attrs[key]=value;}};
  ctx.document.getElementById=id=>id==='refresh'?button:null;
  ctx._=async()=>({errno:1,stdout:'',stderr:'mock read failed'});
  await assert.rejects(ctx.ommegaRefreshApps(),/mock read failed/);
  assert.equal(button.disabled,false);assert.equal(attrs['aria-busy'],'false');
});

test("Soter relay settings are separate from the Integrity config", async () => {
  const context = loadGlue();
  assert.equal(vm.runInContext("OMMEGA_SOTER_REMOTE_CONFIG", context),
    "/data/adb/ommega/soterta/remote.conf");
  const parsed = context.ommegaSoterRemoteParse(
    "enabled=true\nurl=https://soter.example\ntoken=soter-only\ndevice_id=b-two\nuid_map=10001=10002",
  );
  assert.equal(parsed.url, "https://soter.example");
  assert.equal(parsed.device_id, "b-two");
  assert.equal(parsed.enabled, true);

  const values = {
    "ommega-soter-remote-enabled": {checked: true},
    "ommega-soter-remote-tls-insecure": {checked: false},
    "ommega-soter-remote-url": {value: "https://soter.example"},
    "ommega-soter-remote-token": {value: "soter-only"},
    "ommega-soter-remote-device": {value: "b-two"},
    "ommega-soter-remote-uid-map": {value: "10001=10002"},
    "ommega-soter-remote-result": {textContent: ""},
  };
  context.document.getElementById = id => values[id] || null;
  let command = "";
  context._ = async value => {
    command = value;
    return {errno: 0};
  };
  await context.ommegaSoterRemoteSave();
  assert.match(command, /remote\.conf\.tmp/);
  assert.doesNotMatch(command, /soter-only/, "the token must not become shell syntax");
  assert.doesNotMatch(command, /ommegadata\/config/, "Integrity config must stay separate");
  const encoded = command.match(/printf '%s' '([^']+)'/)[1];
  const body = Buffer.from(encoded, "base64").toString("utf8");
  assert.match(body, /^enabled=true$/m);
  assert.match(body, /^token=soter-only$/m);
  assert.match(body, /^uid_map=10001=10002$/m);
  assert.doesNotMatch(body, /^remote:/m);
});
