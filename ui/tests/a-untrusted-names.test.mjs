// App labels and shared-storage names are attacker-chosen; the root WebUI must
// neither parse them as HTML nor splice them into ksu.exec shell strings.
import test from 'node:test';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import vm from 'node:vm';

const assets = new URL('../../a-side/source/template/webroot/assets/', import.meta.url);
const [name] = readdirSync(assets).filter(file => /^index-.*\.js$/.test(file));
const bundle = readFileSync(new URL(name, assets), 'utf8');

function helper(fn) {
  const match = bundle.match(new RegExp('function ' + fn + '\\(s\\) \\{[\\s\\S]*?\\n\\}'));
  assert.ok(match, fn + ' is defined');
  return vm.runInNewContext('(' + match[0] + ')');
}

test('vendor app list and file picker do not interpolate raw names', () => {
  for (const raw of ['<div class="app-name">${n}</div>', 'data-package="${a}"', '<span>${n.name}</span>',
    '>${r}</span>', 'e.innerHTML=`${v}<br>${p}`', 'cd "${o}"', 'cat "${D}/',
    '.split("/").filter(Boolean).join(']) {
    assert.ok(!bundle.includes(raw), 'raw interpolation remains: ' + raw);
  }
});

test('ommegaEsc neutralizes markup in labels and file names', () => {
  const esc = helper('ommegaEsc');
  assert.equal(esc('<img src=x onerror="ksu.exec(\'id\')">'),
    '&lt;img src=x onerror=&quot;ksu.exec(&#39;id&#39;)&quot;&gt;');
});

test('ommegaShellQuote keeps hostile directory and file names inert', () => {
  const quote = helper('ommegaShellQuote');
  const dir = mkdtempSync(join(tmpdir(), 'om-picker-'));
  try {
    const marker = join(dir, 'pwned');
    for (const hostile of ['$(touch ' + marker + ')', '`touch ' + marker + '`', "a'; touch " + marker + "; '"]) {
      const sub = join(dir, hostile.replaceAll('/', '_'));
      mkdirSync(sub);
      writeFileSync(join(sub, 'k.xml'), 'ok');
      const out = execFileSync('sh', ['-c', 'cd ' + quote(sub) + ' && cat ' + quote(sub + '/k.xml')], {encoding: 'utf8'});
      assert.equal(out, 'ok');
      assert.equal(existsSync(marker), false, 'command substitution ran for ' + hostile);
    }
  } finally {
    rmSync(dir, {recursive: true, force: true});
  }
});
