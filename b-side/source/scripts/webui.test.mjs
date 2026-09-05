import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, rmSync, readdirSync, existsSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = readFileSync(new URL('../template/webroot/relay-config.js', import.meta.url), 'utf8');
const config = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const {KEYS, parseConfig, validateConfig, serializeConfig, readCommand, parseSnapshot, saveCommand} = config;
const fixture = '# 保留注释\r\nOMMEGA_RELAY_SERVER=https://example.test:8443\r\nOMMEGA_RELAY_DEVICE_ID=device-b\r\nOMMEGA_RELAY_TOKEN=old-token==\r\nEXTRA_KEY=kept\r\n';
const digest = raw => createHash('sha256').update(raw).digest('hex');
const shell = process.env.WEBUI_TEST_SHELL || (process.platform === 'win32' ? 'C:/Program Files/Git/bin/bash.exe' : '/bin/sh');
function run(command, cwd) {
  const result = spawnSync(shell, ['-s'], {input: command, encoding: 'utf8', cwd});
  if (result.error) throw result.error;
  return result;
}
function sandbox(t) {
  const dir = mkdtempSync(path.join(root, '.webui-test-'));
  // This directory is generated under this repository, never under a device path.
  t.after(() => {
    assert.equal(path.dirname(dir), path.resolve(root));
    assert.ok(path.basename(dir).startsWith('.webui-test-'));
    rmSync(dir, {recursive: true, force: true});
  });
  return {dir, file: path.join(dir, 'relay.conf').replaceAll('\\', '/'), marker: path.join(dir, 'restart.all').replaceAll('\\', '/')};
}

test('reads existing config with daemon defaults and equals characters', () => {
  const values = parseConfig(fixture);
  assert.equal(values[KEYS[3]], 'old-token==');
  assert.deepEqual(KEYS.slice(4).map(k => values[k]), ['true', 'debug', 'true', 'info']);
  assert.equal(parseConfig('OMMEGA_RELAY_LOGCAT_ENABLED=1\nOMMEGA_RELAY_LOG_LEVEL=WARNING')[KEYS[5]], 'warn');
});
test('preserves comments and unknown lines while consolidating duplicates', () => {
  const input = fixture + 'OMMEGA_RELAY_TOKEN=last-value\r\n';
  const values = validateConfig(parseConfig(input));
  const saved = serializeConfig(input, values);
  assert.match(saved, /# 保留注释\n/);
  assert.match(saved, /EXTRA_KEY=kept\n/);
  assert.equal(saved.match(/^OMMEGA_RELAY_TOKEN=/gm).length, 1);
  assert.deepEqual(parseConfig(saved), values);
});
test('validates required fields, URI scheme and multiline/header injection', () => {
  for (const [key, bad] of [[KEYS[0], 'file:///tmp/x'], [KEYS[0], 'https://user:pass@example.test'], [KEYS[0], 'https://example.test?q=x'], [KEYS[1], ''], [KEYS[3], ''], [KEYS[3], 'one\nKEY=two'], [KEYS[3], '秘密'], [KEYS[5], 'invalid']]) {
    assert.throws(() => validateConfig({...parseConfig(fixture), [key]: bad}));
  }
  assert.doesNotThrow(() => validateConfig({...parseConfig(fixture), [KEYS[0]]: 'http://[::1]:8443/path/'}));
});
test('read command snapshots exact bytes and handles an absent config', t => {
  const {dir, file} = sandbox(t);
  assert.deepEqual(parseSnapshot(run(readCommand(file), dir).stdout), {digest: 'missing', raw: ''});
  writeFileSync(file, fixture);
  const result = run(readCommand(file), dir);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(parseSnapshot(result.stdout), {digest: digest(fixture), raw: fixture});
});
test('atomic save preserves literal shell metacharacters and exact previous backup', t => {
  const {dir, file, marker} = sandbox(t);
  writeFileSync(file, fixture);
  const token = "a'\"$(touch${IFS}BAD)`touch${IFS}BAD2`;=$HOME";
  const values = validateConfig({...parseConfig(fixture), [KEYS[3]]: token, [KEYS[2]]: '机器B'});
  const raw = serializeConfig(fixture, values);
  const command = saveCommand(raw, digest(fixture), true, file, marker);
  assert.ok(!command.includes(token));
  const result = run(command, dir);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(file, 'utf8'), raw);
  assert.equal(readFileSync(file + '.webui.bak', 'utf8'), fixture);
  assert.deepEqual(readdirSync(dir).sort(), ['relay.conf', 'relay.conf.webui.bak', 'restart.all']);
  assert.deepEqual(parseSnapshot(run(readCommand(file), dir).stdout), {digest: digest(raw), raw});
});
test('external changes reject stale saves without modifying config or backup', t => {
  const {dir, file, marker} = sandbox(t);
  writeFileSync(file, fixture + '# external edit\n');
  const result = run(saveCommand(fixture, digest(fixture), true, file, marker), dir);
  assert.equal(result.status, 3);
  assert.match(result.stderr, /CONFIG_CHANGED/);
  assert.equal(readFileSync(file, 'utf8'), fixture + '# external edit\n');
  assert.deepEqual(readdirSync(dir), ['relay.conf']);
});
test('first save does not overwrite a config created after loading', t => {
  const {dir, file, marker} = sandbox(t);
  writeFileSync(file, fixture);
  assert.equal(run(saveCommand('new', 'missing', true, file, marker), dir).status, 3);
  assert.equal(readFileSync(file, 'utf8'), fixture);
});
test('log-only save does not create a reload marker; first save succeeds', t => {
  const {dir, file, marker} = sandbox(t);
  const result = run(saveCommand(fixture, 'missing', false, file, marker), dir);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(file, 'utf8'), fixture);
  assert.equal(existsSync(marker), false);
  assert.equal(existsSync(file + '.webui.bak'), false);
});
test('failure before final rename leaves old config and removes staging files', t => {
  const {dir, file, marker} = sandbox(t);
  writeFileSync(file, fixture);
  const command = saveCommand('new', digest(fixture), true, file, marker).replace('mv -f "$next" "$conf"', 'false');
  assert.notEqual(run(command, dir).status, 0);
  assert.equal(readFileSync(file, 'utf8'), fixture);
  assert.equal(readFileSync(file + '.webui.bak', 'utf8'), fixture);
  assert.deepEqual(readdirSync(dir).sort(), ['relay.conf', 'relay.conf.webui.bak']);
});
test('relay config writer has no service, SPL, eval or reboot actions', () => {
  const command = saveCommand(fixture, 'missing', true);
  assert.doesNotMatch(command, /spl-control|service\.sh|\b(?:reboot|kill|pkill|eval|source|resetprop)\b/);
});
