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
