// Verifies spl-control.sh OS version handling against stubbed device tools.
//
// The real script runs on-device against live properties. This test replaces
// getprop/resetprop/setprop/service with stubs on PATH and points the state
// directory at a sandbox, so nothing here can touch a device.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync, writeFileSync, mkdtempSync, mkdirSync, rmSync, chmodSync, existsSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import path from 'node:path';
import {spawnSync} from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const shell = process.env.WEBUI_TEST_SHELL || (process.platform === 'win32' ? 'C:/Program Files/Git/bin/bash.exe' : '/bin/sh');

// Git Bash accepts drive-letter paths with forward slashes, and they survive
// being embedded in the stub scripts without further quoting.
const toPosix = value => value.replace(/\\/g, '/');
const script = toPosix(path.join(root, 'template', 'spl-control.sh'));

const INITIAL_PROPS = [
  'ro.build.version.release=16',
  'ro.build.version.security_patch=2026-08-01',
  'ro.vendor.build.security_patch=2026-08-01',
  'ro.vendor.boot_security_patch=',
  'ro.boot.image.build.security_patch=',
];
const INITIAL_SERVICES = [
  '[init.svc.vendor.keymint-qti]: [running]',
  '[init.svc.keystore2]: [running]',
];

function sandbox(t) {
  const dir = mkdtempSync(path.join(root, '.spl-test-'));
  const bin = path.join(dir, 'bin');
  const state = path.join(dir, 'state');
  mkdirSync(bin);
  mkdirSync(state);
  const props = toPosix(path.join(dir, 'props'));
  const svc = toPosix(path.join(dir, 'svc'));
  const calls = toPosix(path.join(dir, 'calls'));
  writeFileSync(props, INITIAL_PROPS.join('\n') + '\n');
  writeFileSync(svc, INITIAL_SERVICES.join('\n') + '\n');
  const write = (name, body) => {
    const file = path.join(bin, name);
    writeFileSync(file, body.split('\n').join('\n') + '\n');
    chmodSync(file, 0o755);
  };
  write('getprop', [
    '#!/bin/sh',
    'if [ -n "$1" ]; then',
    '  sed -n "s|^$1=||p" ' + props + ' | tail -n 1',
    'else',
    '  cat ' + svc,
    'fi',
  ].join('\n'));
  write('resetprop', [
    '#!/bin/sh',
    'if [ "$1" = "--delete" ]; then',
    '  sed -i "/^$2=/d" ' + props,
    '  echo "delete $2" >> ' + calls,
    'elif [ "$1" = "-n" ]; then',
    '  sed -i "/^$2=/d" ' + props,
    '  printf "%s=%s\\n" "$2" "$3" >> ' + props,
    '  echo "set $2=$3" >> ' + calls,
    'fi',
    'exit 0',
  ].join('\n'));
  write('setprop', ['#!/bin/sh', 'echo "setprop $*" >> ' + calls, 'exit 0'].join('\n'));
  write('service', ['#!/bin/sh', 'echo found', 'exit 0'].join('\n'));
  const quote = value => "'" + String(value).replace(/'/g, "'\\''") + "'";
  t.after(() => {
    assert.equal(path.dirname(dir), path.resolve(root));
    assert.ok(path.basename(dir).startsWith('.spl-test-'));
    rmSync(dir, {recursive: true, force: true});
  });
  return {
    dir,
    run: (...args) => spawnSync(shell, ['-c', 'sh ' + quote(script) + ' ' + args.map(quote).join(' ')], {
      encoding: 'utf8',
      env: {...process.env, PATH: toPosix(bin) + ':/usr/bin:/bin', OMMEGA_STATE_DIR: toPosix(state)},
    }),
    prop: key => {
      const line = readFileSync(props, 'utf8').split('\n').find(row => row.startsWith(key + '='));
      return line === undefined ? null : line.slice(key.length + 1);
    },
    calls: () => (existsSync(calls) ? readFileSync(calls, 'utf8') : ''),
  };
}

test('status reports the configured and effective OS version', t => {
  const box = sandbox(t);
  const result = box.run('status');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^OS_VERSION=$/m);
  assert.match(result.stdout, /^CURRENT_OS_VERSION=16$/m);
});

test('saving an OS version rewrites the property and restarts the stack', t => {
  const box = sandbox(t);
  const result = box.run('save', '', '', '', '17');
  assert.equal(result.status, 0, result.stderr + result.stdout);
  assert.equal(box.prop('ro.build.version.release'), '17');
  const calls = box.calls();
  assert.match(calls, /set ro\.build\.version\.release=17/);
  // A release change alone must restart KeyMint: the HAL reads it at startup.
  assert.match(calls, /setprop ctl\.restart vendor\.keymint-qti/);
  assert.match(calls, /setprop ctl\.restart keystore2/);
});

test('an empty OS version restores the recorded baseline only upwards', t => {
  const box = sandbox(t);
  assert.equal(box.run('save', '', '', '', '17').status, 0);
  assert.equal(box.prop('ro.build.version.release'), '17');
  // Clearing the field asks for the recorded baseline (16). The release is the
  // same one-way ratchet as the patch levels, so the downgrade is refused and
  // the device keeps the newer value.
  const result = box.run('save', '', '', '', '');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /refused to lower ro\.build\.version\.release from 17 to 16/);
  assert.equal(box.prop('ro.build.version.release'), '17');
});

test('a baseline from an older module gains an OS version instead of losing it', t => {
  const box = sandbox(t);
  writeFileSync(
    path.join(box.dir, 'state', 'spl-baseline.conf'),
    'SYSTEM_SPL=2026-08-01\nVENDOR_SPL=2026-08-01\n',
  );
  const result = box.run('save', '', '', '', '17');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.release'), '17');
  assert.match(readFileSync(path.join(box.dir, 'state', 'spl-baseline.conf'), 'utf8'), /^OS_VERSION=16$/m);
  // The late-recorded baseline is still the value a cleared field asks for; the
  // explicit marker is what makes that lowering deliberate.
  writeFileSync(path.join(box.dir, 'state', 'allow-spl-downgrade'), '');
  assert.equal(box.run('save', '', '', '', '').status, 0);
  assert.equal(box.prop('ro.build.version.release'), '16');
});

test('an unset OS version never deletes the property', t => {
  const box = sandbox(t);
  const result = box.run('save', '', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.release'), '16');
  assert.doesNotMatch(box.calls(), /delete ro\.build\.version\.release/);
});

test('rejects a malformed OS version', t => {
  const box = sandbox(t);
  for (const bad of ['abc', '-1', '1.7', '123']) {
    const result = box.run('save', '', '', '', bad);
    assert.notEqual(result.status, 0, 'expected ' + bad + ' to be rejected');
    assert.equal(box.prop('ro.build.version.release'), '16');
  }
});

test('reapplying the current value does not restart the stack', t => {
  const box = sandbox(t);
  assert.equal(box.run('save', '', '', '', '16').status, 0);
  assert.doesNotMatch(box.calls(), /ctl\.restart/);
});

test('refuses to lower a security patch level', t => {
  const box = sandbox(t);
  const result = box.run('save', '2025-12-01', '', '', '');
  assert.notEqual(result.status, 0, 'a downgrade must not report success');
  assert.match(
    result.stderr,
    /refused to lower ro\.build\.version\.security_patch from 2026-08-01 to 2025-12-01/,
  );
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
  assert.doesNotMatch(box.calls(), /set ro\.build\.version\.security_patch=/);
  assert.doesNotMatch(box.calls(), /ctl\.restart/);
});

test('applies a newer security patch level', t => {
  const box = sandbox(t);
  const result = box.run('save', '2026-09-01', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-09-01');
  assert.match(box.calls(), /ctl\.restart/);
});

test('refuses to lower the OS release', t => {
  const box = sandbox(t);
  const result = box.run('save', '', '', '', '15');
  assert.notEqual(result.status, 0, 'lowering the release must not report success');
  assert.match(result.stderr, /refused to lower ro\.build\.version\.release from 16 to 15/);
  assert.equal(box.prop('ro.build.version.release'), '16');
  assert.doesNotMatch(box.calls(), /ctl\.restart/);
});

test('a baseline recorded under an override cannot lower the patch level', t => {
  const box = sandbox(t);
  writeFileSync(
    path.join(box.dir, 'state', 'spl-baseline.conf'),
    'SYSTEM_SPL=2025-12-01\nVENDOR_SPL=2025-12-01\nOS_VERSION=16\n',
  );
  const result = box.run('save', '', '', '', '');
  assert.notEqual(result.status, 0, 'the recorded baseline must not be applied downwards');
  assert.match(result.stderr, /refused to lower ro\.build\.version\.security_patch/);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
});

test('boot-time apply reports a persisted downgrade without failing startup', t => {
  const box = sandbox(t);
  writeFileSync(
    path.join(box.dir, 'state', 'spl.conf'),
    'SYSTEM_SPL=2025-12-01\nBOOT_SPL=\nVENDOR_SPL=2025-12-01\nOS_VERSION=\n',
  );
  const result = box.run('apply');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stderr, /refused to lower ro\.build\.version\.security_patch/);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
});

test('the downgrade marker allows one deliberate lowering', t => {
  const box = sandbox(t);
  writeFileSync(path.join(box.dir, 'state', 'allow-spl-downgrade'), '');
  const result = box.run('save', '2025-12-01', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.security_patch'), '2025-12-01');
  assert.match(box.calls(), /ctl\.restart/);
});
