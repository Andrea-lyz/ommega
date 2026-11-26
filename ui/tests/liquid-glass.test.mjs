import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {liquidGlassMap, liquidGlassIndex} from '../../a-side/source/template/webroot/liquid-glass.js';

test('displacement map is neutral inside and points inwards at the bezel', () => {
  const width = 120, height = 40, map = liquidGlassMap(width, height, 20, 10);
  const at = (x, y) => [...map.slice((y * width + x) * 4, (y * width + x) * 4 + 4)];
  assert.deepEqual(at(60, 20), [128, 128, 128, 255]);
  // Left edge: the outward normal is -x, so R rises above 128 (pull right).
  assert.ok(at(1, 20)[0] > 200);
  // Right edge pulls left, bottom edge pulls up.
  assert.ok(at(118, 20)[0] < 60);
  assert.ok(at(60, 39)[1] < 80);
  assert.equal(map.length, width * height * 4);
});

test('finger offset resolves to the nearest real button, gaps included', () => {
  const slots = [{x: 0, w: 100}, {x: 106, w: 100}, {x: 212, w: 100}];
  assert.equal(liquidGlassIndex(-5, slots), 0);
  assert.equal(liquidGlassIndex(102, slots), 0);
  assert.equal(liquidGlassIndex(104, slots), 1);
  assert.equal(liquidGlassIndex(260, slots), 2);
  assert.equal(liquidGlassIndex(900, slots), 2);
});

function loadBridge(ksu) {
  const shared = readFileSync(new URL('../shared/ui-kit.js', import.meta.url), 'utf8');
  const part = shared.slice(shared.indexOf('export function createBridge('), shared.indexOf('export function pollWhenVisible(')).replaceAll('export ', '');
  const listeners = {};
  const window = {ksu, addEventListener: (type, fn) => { listeners[type] = fn; }};
  // Answer the one-time capability probe the way a working manager does.
  if (ksu.spawn && !ksu.silent) {
    const original = ksu.spawn;
    ksu.spawn = (command, args, options, name) => command === 'true' ? window[name].emit('exit', 0) : original(command, args, options, name);
  }
  let now = 0;
  const timers = [];
  const context = vm.createContext({window, text: (_, en) => en, performance: {now: () => now},
    setTimeout: (fn, ms) => { timers.push({at: now + ms, fn}); return timers.length; }, clearTimeout() {}});
  vm.runInContext(part, context);
  const advance = ms => { now += ms; for (const t of timers.splice(0).sort((a, b) => a.at - b.at)) { if (t.at <= now) t.fn(); else timers.push(t); } };
  return {window, listeners, advance, create: (...args) => vm.runInContext('createBridge', context)(...args)};
}

test('spawn bridge streams lines and matches ksu.exec output and exit status', async () => {
  const started = [];
  const env = loadBridge({spawn(command, args, options, name) { started.push({command, args, name}); }});
  const exec = env.create('t', 12000, {spawn: true});
  const pending = exec('echo a\necho\necho b');
  env.advance(200);
  assert.equal(started.length, 1);
  assert.equal(started[0].command, 'echo a\necho\necho b');
  assert.equal(started[0].args, '[]');
  const child = env.window[started[0].name];
  for (const line of ['a', '', 'b']) child.stdout.emit('data', line);
  child.stderr.emit('data', 'warn');
  child.emit('exit', 1);
  child.emit('error', {exitCode: 1, message: ''});
  assert.deepEqual({...await pending}, {errno: 1, stdout: 'a\n\nb', stderr: 'warn'});
  // A late event after settling must not throw.
  env.window[started[0].name].emit('exit', 0);
});

test('spawn bridge waits for the finger to lift and keeps issue order', async () => {
  const started = [];
  const env = loadBridge({spawn(command, args, options, name) { started.push(command); env.window[name].emit('exit', 0); }});
  const exec = env.create('t', 12000, {spawn: true});
  exec('warm');
  env.advance(200);
  assert.deepEqual(started, ['warm']);
  env.listeners.pointerdown();
  const first = exec('first'), second = exec('second');
  env.advance(400);
  assert.deepEqual(started, ['warm']);
  env.listeners.pointerup();
  env.advance(100);
  assert.deepEqual(started, ['warm']);
  env.advance(100);
  env.advance(0);
  assert.deepEqual(started, ['warm', 'first', 'second']);
  assert.equal((await first).errno, 0);
  assert.equal((await second).errno, 0);
});

test('spawned commands run one at a time like exec', async () => {
  const started = [];
  const env = loadBridge({spawn(command, args, options, name) { started.push({command, name}); }});
  const exec = env.create('t', 0, {spawn: true});
  env.advance(200);
  const write = exec('write'), read = exec('read');
  env.advance(50);
  assert.deepEqual(started.map(s => s.command), ['write']);
  env.advance(5000);
  assert.deepEqual(started.map(s => s.command), ['write'], 'a write without timeout holds the queue until it exits');
  env.window[started[0].name].emit('exit', 0);
  env.advance(0);
  assert.deepEqual(started.map(s => s.command), ['write', 'read']);
  env.window[started[1].name].emit('exit', 0);
  assert.equal((await write).errno, 0);
  assert.equal((await read).errno, 0);
});

test('a read timeout starts with the command, not with its queue wait', async () => {
  const started = [];
  const env = loadBridge({spawn(command, args, options, name) { started.push({command, name}); }});
  const exec = env.create('t', 1000, {spawn: true});
  env.advance(200);
  const slow = exec('slow'), queued = exec('queued');
  env.advance(900);
  assert.equal(started.length, 1);
  env.advance(200);
  assert.equal((await slow).errno, 1);
  env.advance(0);
  assert.equal(started.length, 2);
  env.advance(900);
  env.window[started[1].name].emit('exit', 0);
  assert.equal((await queued).errno, 0);
});

test('a held pointer cannot starve the bridge past its bound', () => {
  const started = [];
  const env = loadBridge({spawn(command, args, options, name) { started.push(command); env.window[name].emit('exit', 0); }});
  const exec = env.create('t', 0, {spawn: true});
  exec('warm'); env.advance(200); env.advance(0);
  env.listeners.pointerdown();
  exec('read');
  env.advance(500); assert.deepEqual(started, ['warm']);
  env.advance(150); assert.deepEqual(started, ['warm', 'read']);
});

test('without spawn the bridge keeps the synchronous exec path', async () => {
  let callback;
  const env = loadBridge({exec(command, options, name) { callback = name; }});
  const pending = env.create('t', 0, {spawn: true})('x');
  env.window[callback](0, 'ok', '');
  assert.equal((await pending).stdout, 'ok');
});

test('a manager whose spawn never answers keeps using exec', async () => {
  let callback;
  const env = loadBridge({silent: true, spawn() {}, exec(command, options, name) { callback = name; }});
  const pending = env.create('t', 0, {spawn: true})('x');
  env.advance(2900);
  assert.equal(callback, undefined);
  env.advance(200);
  env.window[callback](0, 'ok', '');
  assert.equal((await pending).stdout, 'ok');
});
