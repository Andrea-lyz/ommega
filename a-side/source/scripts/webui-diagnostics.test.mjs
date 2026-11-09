
import test from "node:test";
import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import vm from "node:vm";

const source = await readFile(new URL("./webui-diagnostics.js", import.meta.url), "utf8");

function load() {
  const context = {
    console,
    setTimeout,
    clearTimeout,
    btoa: value => Buffer.from(String(value), "binary").toString("base64"),
    localStorage: {getItem: () => null, setItem() {}, removeItem() {}},
    navigator: {},
    location: {hash: ""},
    window: {addEventListener() {}},
    document: {
      readyState: "loading",
      addEventListener() {},
      getElementById: () => null,
      querySelectorAll: () => [],
      createElement: () => ({style: {}, setAttribute() {}, appendChild() {}, remove() {}}),
      body: {appendChild() {}}
    },
    ommegaUiLocalize: value => String(value).split(" / ")[0],
    ommegaReadOnly: async () => ({errno: 0, stdout: "", stderr: ""}),
    _: async () => ({errno: 0, stdout: "", stderr: ""})
  };
  vm.createContext(context);
  vm.runInContext(source, context);
  return context;
}

const diag = load();

test("system identities are separated from app uids", () => {
  assert.equal(diag.ommegaDiagIsAppUid(1017), false);
  assert.equal(diag.ommegaDiagIsAppUid(1000), false);
  assert.equal(diag.ommegaDiagIsAppUid(10000), true);
  assert.equal(diag.ommegaDiagIsAppUid(10501), true);
});

test("repeated failures aggregate with count and last timestamp", () => {
  const rows = diag.ommegaDiagAggregateErrors([
    {code: "-26", uid: "10501", ts: "2026-11-08 22:00:01"},
    {code: "-26", uid: "10501", ts: "2026-11-08 22:05:02"},
    {code: "-33", uid: "10501", ts: "2026-11-08 21:00:00"}
  ]);
  assert.equal(rows.length, 2);
  const first = rows[0];
  assert.equal(first.code, "-26");
  assert.equal(first.count, 2);
  assert.equal(first.ts, "2026-11-08 22:05:02");
  assert.equal(rows[1].code, "-33");
});

test("probe output parses into rows", () => {
  const rows = diag.ommegaDiagParseProbe([
    "RESULT|A07|HIT|Device properties|attempt failed",
    "not a result",
    "RESULT|PT02|PASS|Key generation|ok"
  ].join("\n"));
  assert.equal(rows.length, 2);
  assert.equal(rows[0].id, "A07");
  assert.equal(rows[0].status, "HIT");
});

test("reason text stays human readable", () => {
  assert.match(diag.ommegaDiagReason("-26"), /认证|authenticated/i);
  assert.match(diag.ommegaDiagReason("-66"), /设备 ID|device id/i);
});

test("level-3 routes require the persisted unlock", () => {
  assert.equal(diag.ommegaDiagRouteAllowed("diagnostics", false), true);
  assert.equal(diag.ommegaDiagRouteAllowed("diag-keymint", false), false);
  assert.equal(diag.ommegaDiagRouteAllowed("diag-keymint", true), true);
});

test("a shared-uid package maps every uid it owns", () => {
  const map = diag.ommegaDiagParsePackageUids([
    "package:com.google.android.gms uid:10120,99910120",
    "package:com.microsoft.appmanager uid:10132"
  ].join("\n"));
  assert.equal(map.get("10120"), "com.google.android.gms");
  assert.equal(map.get("99910120"), "com.google.android.gms");
  assert.equal(map.get("10132"), "com.microsoft.appmanager");
  assert.equal(map.get("99999"), undefined);
});

test("keymint AID failures attribute to the requesting app", async () => {
  const ctx = load();
  const errors = [
    "==keymint",
    "2026-11-09 03:26:50 UTC [ERROR] keymint::keymaster::error - KeystoreOperation::finish",
    "Caused by:",
    "    0: src/keymaster/operation.rs:463 Finish failed for AppUid(10134)",
    "    1: Error::Km(ErrorCode(-30)) for AppUid(1017)",
    "==injector"
  ].join("\n");
  ctx.ommegaReadOnly = async command =>
    String(command).includes("pm list packages")
      ? {errno: 0, stdout: "package:com.microsoft.appmanager uid:10134\n"}
      : {errno: 0, stdout: errors};
  const rows = await ctx.ommegaDiagCollectErrors();
  assert.equal(rows.length, 1);
  assert.equal(rows[0].uid, "10134");
  assert.equal(rows[0].code, "-30");
  assert.equal(rows[0].pkg, "com.microsoft.appmanager");
});

test("checklist renders ticks, warnings and advice only when failing", () => {
  const target = {innerHTML: ""};
  diag.ommegaDiagRenderItems(target, [
    {ok: true, zh: "服务在运行", en: "service running", adviceZh: "", adviceEn: ""},
    {ok: false, zh: "失败 3 次", en: "3 failures", adviceZh: "打开请求异常", adviceEn: "open failures"}
  ]);
  assert.match(target.innerHTML, /✓/);
  assert.match(target.innerHTML, /⚠/);
  assert.match(target.innerHTML, /打开请求异常/);
  assert.doesNotMatch(target.innerHTML.split("⚠")[0], /建议/);
});

test("error rows carry the icon and the aggregated count", () => {
  const rows = diag.ommegaDiagErrorRows([
    {code: "-26", uid: "10501", ts: "2026-11-08 22:05:02", count: 3,
     pkg: "com.example.app", label: "Example", icon: "ksu://icon/com.example.app"}
  ]);
  assert.match(rows, /ksu:\/\/icon\/com.example.app/);
  assert.match(rows, /Example/);
  assert.match(rows, /发生 3 次/);
  assert.match(rows, /最后 11-08 22:05:02/);
});

test("keymint page turns failed generations into an advice row", () => {
  const output = [
    "==mode", 'enabled = true', 'device_id = "op11-b"',
    "==signals", "10", "3",
    "==recent", "2026-11-08 22:00:01 UTC [INFO] certificate_policy=local",
    "==tail", "tail line"
  ].join("\n");
  const items = diag.ommegaDiagPageItems({id: "diag-keymint"}, output);
  assert.equal(items.length, 5);
  assert.equal(items[0].ok, true);
  assert.match(items[0].zh, /中继/);
  const failed = items[2];
  assert.equal(failed.ok, false);
  assert.ok(failed.adviceZh.length > 0);
  assert.match(items[3].zh, /最近 15 分钟/);
});
