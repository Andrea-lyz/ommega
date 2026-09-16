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
  const serialized = context.ommegaTargetCompatToml(new Set(["com.b.two", "com.a.one"]));
  assert.match(serialized, /^version = 1/m);
  assert.match(serialized, /^\[positive_key_id\]$/m);
  assert.match(serialized, /^packages = \["com\.a\.one", "com\.b\.two"\]$/m);
  const parsed = context.ommegaParseTargetCompat(serialized);
  assert.deepEqual([...parsed].sort(), ["com.a.one", "com.b.two"]);
});

test("an absent, empty or unrelated policy lists no packages", () => {
  const context = loadGlue();
  assert.equal(context.ommegaParseTargetCompat("").size, 0);
  assert.equal(context.ommegaParseTargetCompat("version = 1\n").size, 0);
  assert.equal(
    context.ommegaParseTargetCompat("[positive_key_id]\npackages = []\n").size,
    0,
  );
  assert.equal(
    context.ommegaParseTargetCompat("[something_else]\npackages = [\"x\"]\n").size,
    0,
  );
});

