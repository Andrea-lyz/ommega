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
