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
    Mi: async () => undefined,
    _: async () => ({ stdout: "" }),
    se: { querySelectorAll: () => [] },
    Dl: new Set(),
    Qe: [],
    ommegaTargetModes: new Map(),
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

test("the glue evaluates without the WebUI globals", () => {
  loadGlue();
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

test("compatibility drafts remain dirty until the successfully saved settings reload", async () => {
  const ctx = loadGlue();
  ctx.Qe = ["com.example.wallet"];
  await ctx.Mi();
  vm.runInContext('ommegaCompatEnabled = true', ctx);
  assert.equal(ctx.ommegaAppsHaveChanges(), true);
  ctx._ = async () => ({errno:1, stdout:"", stderr:"write failed"});
  await ctx.ommegaWriteTargetCompat();
  assert.equal(ctx.ommegaAppsHaveChanges(), true, "failed writes do not clear the baseline");
  ctx._ = async () => ({errno:0, stdout:"[positive_key_id]\nenabled=true\n"});
  await ctx.Mi();
  assert.equal(ctx.ommegaAppsHaveChanges(), false);
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
  assert.equal(elements["apps-list"].inert, true);
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

test("global scope disables list editing and restores it after switching off", () => {
  const ctx = loadGlue({language: "zh-CN"});
  const elements = {
    "ommega-global-scope-enabled": {},
    "ommega-global-scope-status": {},
    "apps-list": {style: {}, setAttribute() {}},
    "save": {},
  };
  ctx.document.getElementById = id => elements[id] || null;
  vm.runInContext("ommegaGlobalScopeKnown = true; ommegaGlobalScopeEnabled = true; ommegaScopeSync()", ctx);
  assert.equal(elements["apps-list"].inert, true);
  assert.equal(elements.save.disabled, true);
  assert.match(elements["ommega-global-scope-status"].textContent, /用户应用和普通系统应用.*排除核心系统服务/);
  vm.runInContext("ommegaGlobalScopeEnabled = false; ommegaScopeSync()", ctx);
  assert.equal(elements["apps-list"].inert, false);
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
      assert.equal(elements["apps-list"].inert, true);
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
