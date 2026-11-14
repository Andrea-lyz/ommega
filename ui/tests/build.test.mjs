// Build-script mutations are isolated in an owned temporary fixture.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
const source = new URL('../../',import.meta.url);
async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(),'ommega-ui-build-'));
  t.after(() => rm(root,{recursive:true,force:true}));
  await mkdir(join(root,'ui'),{recursive:true});
  await cp(new URL('ui/build.mjs',source),join(root,'ui/build.mjs'));
  await cp(new URL('ui/shared/',source),join(root,'ui/shared'),{recursive:true});
  const webroot = join(root,'a-side/source/template/webroot');
  await cp(new URL('a-side/source/template/webroot/',source),webroot,{recursive:true});
  const run = (...args) => spawnSync(process.execPath,[join(root,'ui/build.mjs'),'--side=a',...args],{cwd:root,encoding:'utf8'});
  const snapshot = async directory => {
    const result = {};
    for (const item of await readdir(directory,{withFileTypes:true})) {
      const path = join(directory,item.name);
      if (item.isDirectory()) Object.assign(result,await snapshot(path));
      else result[path] = createHash('sha256').update(await readFile(path)).digest('hex');
    }
    return result;
  };
  const check = async status => {
    const before = await snapshot(root), result = run('--check');
    assert.equal(result.status,status,result.stdout + result.stderr);
    assert.deepEqual(await snapshot(root),before,'--check must not rewrite resources or shell');
  };
  return {root,webroot,run,check};
}

test('A-only build/check works without a B checkout and is deterministic',async t => {
  const f = await fixture(t);
  await f.check(0);
  const before = await readFile(join(f.webroot,'index.html'),'utf8');
  assert.equal(f.run().status,0);
  assert.equal(await readFile(join(f.webroot,'index.html'),'utf8'),before);
  await assert.rejects(readdir(join(f.root,'b-side')), {code:'ENOENT'});
});

test('builder --check detects actual vendor CSS changes and build updates their hash',async t => {
  const f = await fixture(t);
  const html = await readFile(join(f.webroot,'index.html'),'utf8');
  const css = html.match(/href="(\.\/assets\/index-[^"?]+\.css)/)[1];
  const path = join(f.webroot,css.slice(2));
  await writeFile(path,(await readFile(path,'utf8')) + '\n/* fixture change */\n');
  await f.check(1);
  assert.equal(f.run().status,0);
  const digest = createHash('sha256').update(await readFile(path)).digest('hex').slice(0,10);
  assert.ok((await readFile(join(f.webroot,'index.html'),'utf8')).includes(css + '?v=' + digest));
  await f.check(0);
});

test('builder --check rejects stale queries and missing assets without repair',async t => {
  const f = await fixture(t), page = join(f.webroot,'index.html');
  await writeFile(page,(await readFile(page,'utf8')).replace(/(index-[^"?]+\.css)\?v=[^"]+/,'$1?v=stale'));
  await f.check(1);
  assert.equal(f.run().status,0);
  await rm(join(f.webroot,'a-modern.css'));
  await f.check(1);
  const result = f.run();
  assert.notEqual(result.status,0);
  assert.match(result.stderr,/Missing local WebUI resource/);
});
