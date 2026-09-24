// Verifies the detector-compatibility glue inside the prebuilt WebUI bundle.
//
// The WebUI is shipped as a bundle without sources, so the glue is appended by
// scripts/webui-patch.py. This test extracts the appended block from the
// committed bundle and exercises its policy parsing/serialization, which keeps
// the patched bundle honest in CI.
import assert from "node:assert/strict";
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

function loadGlue() {
  const element = { checked: false, onclick: undefined, addEventListener() {} };
  const context = {
    console,
    setTimeout,
    Mi: async () => undefined,
    _: async () => ({ stdout: "" }),
    se: { querySelectorAll: () => [] },
    Dl: new Set(),
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
  vm.runInContext(glue, context);
  return context;
}

test("the glue evaluates without the WebUI globals", () => {
  loadGlue();
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
