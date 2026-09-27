// Verifies spl-control.sh against stubbed device tools.
//
// The real script runs on-device against the live properties and the read-only
// firmware property files. This test replaces getprop/resetprop/setprop/service
// with stubs on PATH and points the state directory and the firmware files at a
// sandbox, so nothing here can touch a device.
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

// The live properties start where an override left them: this is the state a
// device is in while it reports the other side's newer patch date.
const INITIAL_PROPS = [
  'ro.build.version.release=16',
  'ro.build.version.security_patch=2026-08-01',
  'ro.vendor.build.security_patch=2026-08-01',
  'ro.vendor.boot_security_patch=',
  'ro.boot.image.build.security_patch=',
];
// The firmware files keep the values the device shipped with. Every patch
// level and the release are measured against them.
const FIRMWARE = {
  release: '16',
  system: '2025-12-01',
  vendor: '2025-12-01',
};
const INITIAL_SERVICES = [
  '[init.svc.vendor.keymint-qti]: [running]',
  '[init.svc.keystore2]: [running]',
];

function sandbox(t, options = {}) {
  const dir = mkdtempSync(path.join(root, '.spl-test-'));
  const bin = path.join(dir, 'bin');
  const state = path.join(dir, 'state');
  const firmwareDir = path.join(dir, 'firmware');
  mkdirSync(bin);
  mkdirSync(state);
  mkdirSync(firmwareDir);
  const props = toPosix(path.join(dir, 'props'));
  const svc = toPosix(path.join(dir, 'svc'));
  const calls = toPosix(path.join(dir, 'calls'));
  const systemProp = toPosix(path.join(firmwareDir, 'system.prop'));
  const vendorProp = toPosix(path.join(firmwareDir, 'vendor.prop'));
  writeFileSync(props, INITIAL_PROPS.join('\n') + '\n');
  writeFileSync(svc, INITIAL_SERVICES.join('\n') + '\n');
  const firmware = {...FIRMWARE, ...(options.firmware || {})};
  const writeFirmware = current => {
    writeFileSync(systemProp, [
      'ro.build.version.release=' + current.release,
      'ro.build.version.security_patch=' + current.system,
    ].join('\n') + '\n');
    writeFileSync(vendorProp, [
      'ro.vendor.build.security_patch=' + current.vendor,
    ].join('\n') + '\n');
  };
  writeFirmware(firmware);
  const firmwareFiles = options.firmwareUnreadable
    ? toPosix(path.join(dir, 'absent', 'system.prop'))
    : systemProp + ' ' + vendorProp;
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
  const readState = name => {
    const file = path.join(dir, 'state', name);
    return existsSync(file) ? readFileSync(file, 'utf8') : '';
  };
  t.after(() => {
    assert.equal(path.dirname(dir), path.resolve(root));
    assert.ok(path.basename(dir).startsWith('.spl-test-'));
    rmSync(dir, {recursive: true, force: true});
  });
  return {
    dir,
    run: (...args) => spawnSync(shell, ['-c', 'sh ' + quote(script) + ' ' + args.map(quote).join(' ')], {
      encoding: 'utf8',
      env: {
        ...process.env,
        PATH: toPosix(bin) + ':/usr/bin:/bin',
        OMMEGA_STATE_DIR: toPosix(state),
        OMMEGA_PROP_FILES: firmwareFiles,
      },
    }),
    prop: key => {
      const line = readFileSync(props, 'utf8').split('\n').find(row => row.startsWith(key + '='));
      return line === undefined ? null : line.slice(key.length + 1);
    },
    calls: () => (existsSync(calls) ? readFileSync(calls, 'utf8') : ''),
    firmware: current => writeFirmware({...firmware, ...current}),
    state: readState,
  };
}

test('status reports the configuration, the live values and the floor', t => {
  const box = sandbox(t);
  const result = box.run('status');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /^OS_VERSION=$/m);
  assert.match(result.stdout, /^CURRENT_OS_VERSION=16$/m);
  assert.match(result.stdout, /^CURRENT_SYSTEM_SPL=2026-08-01$/m);
  assert.match(result.stdout, /^ORIGIN_SYSTEM_SPL=2025-12-01$/m);
  assert.match(result.stdout, /^FLOOR_SYSTEM_SPL=2025-12-01$/m);
  assert.match(result.stdout, /^FLOOR_OS_VERSION=16$/m);
  // The device's own values are recorded, so a later run knows the floor even
  // when the firmware files cannot be read.
  assert.match(box.state('spl-origin.conf'), /^SYSTEM_SPL=2025-12-01$/m);
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

test('clearing the fields restores the device values', t => {
  const box = sandbox(t);
  assert.equal(box.run('save', '2026-09-01', '2026-09-01', '2026-09-01', '17').status, 0);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-09-01');
  assert.equal(box.prop('ro.build.version.release'), '17');
  const result = box.run('save', '', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.security_patch'), '2025-12-01');
  assert.equal(box.prop('ro.vendor.build.security_patch'), '2025-12-01');
  assert.equal(box.prop('ro.build.version.release'), '16');
  assert.match(box.calls(), /ctl\.restart/);
});

test('lowers a security patch level to the device original', t => {
  const box = sandbox(t);
  const result = box.run('save', '2025-12-01', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.security_patch'), '2025-12-01');
  assert.match(box.calls(), /set ro\.build\.version\.security_patch=2025-12-01/);
  assert.match(box.calls(), /ctl\.restart/);
});

test('applies a newer security patch level', t => {
  const box = sandbox(t);
  const result = box.run('save', '2026-09-01', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-09-01');
  assert.match(box.calls(), /ctl\.restart/);
});

test('refuses a patch level older than the device original', t => {
  const box = sandbox(t);
  const result = box.run('save', '2025-06-01', '2025-06-01', '2025-06-01', '');
  assert.notEqual(result.status, 0, 'a value below the original must not report success');
  assert.match(result.stderr, /refused to set ro\.build\.version\.security_patch to 2025-06-01/);
  assert.match(result.stderr, /original value is 2025-12-01/);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
  // Nothing is written at all: every refused field keeps the live value.
  assert.equal(box.calls(), '');
});

test('holds the boot and vendor patch levels above the original too', t => {
  const box = sandbox(t);
  const result = box.run('save', '', '2024-01-01', '2024-01-01', '');
  assert.notEqual(result.status, 0, 'a boot patch level below the original must not report success');
  assert.match(result.stderr, /refused to set ro\.vendor\.build\.security_patch to 2024-01-01/);
  // The emptied system field still restores the device value; the refused
  // vendor fields keep what the device reports.
  assert.equal(box.prop('ro.build.version.security_patch'), '2025-12-01');
  assert.equal(box.prop('ro.vendor.build.security_patch'), '2026-08-01');
  assert.equal(box.prop('ro.vendor.boot_security_patch'), '');
});

test('refuses to lower the OS release below the device original', t => {
  const box = sandbox(t);
  const result = box.run('save', '2026-08-01', '', '2026-08-01', '15');
  assert.notEqual(result.status, 0, 'lowering the release must not report success');
  assert.match(result.stderr, /refused to set ro\.build\.version\.release to 15/);
  assert.match(result.stderr, /original value is 16/);
  assert.equal(box.prop('ro.build.version.release'), '16');
  assert.doesNotMatch(box.calls(), /ctl\.restart/);
});

test('a baseline recorded under an override never becomes the floor', t => {
  const box = sandbox(t);
  // Values captured while the other device's patch date was in effect must not
  // stop the device from returning to its own.
  writeFileSync(
    path.join(box.dir, 'state', 'spl-baseline.conf'),
    'SYSTEM_SPL=2026-08-01\nVENDOR_SPL=2026-08-01\nOS_VERSION=17\n',
  );
  const result = box.run('save', '', '', '', '');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.security_patch'), '2025-12-01');
  assert.equal(box.prop('ro.build.version.release'), '16');
});

test('without readable firmware files the recorded baseline is the floor', t => {
  const box = sandbox(t, {firmwareUnreadable: true});
  const result = box.run('save', '2025-12-01', '', '', '');
  assert.notEqual(result.status, 0, 'the captured baseline must still hold the floor');
  assert.match(result.stderr, /refused to set ro\.build\.version\.security_patch to 2025-12-01/);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
});

test('a firmware update raises the floor', t => {
  const box = sandbox(t);
  assert.equal(box.run('status').status, 0);
  assert.match(box.state('spl-origin.conf'), /^SYSTEM_SPL=2025-12-01$/m);
  box.firmware({system: '2026-08-01', vendor: '2026-08-01'});
  const result = box.run('save', '2025-12-01', '', '', '');
  assert.notEqual(result.status, 0, 'the new firmware value is the floor');
  assert.match(result.stderr, /original value is 2026-08-01/);
  assert.match(box.state('spl-origin.conf'), /^SYSTEM_SPL=2026-08-01$/m);
});

test('the recorded origin survives firmware files that disappear', t => {
  const box = sandbox(t);
  assert.equal(box.run('status').status, 0);
  rmSync(path.join(box.dir, 'firmware'), {recursive: true, force: true});
  const result = box.run('save', '2025-06-01', '', '', '');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /original value is 2025-12-01/);
});

test('the removed downgrade marker grants nothing', t => {
  const box = sandbox(t);
  writeFileSync(path.join(box.dir, 'state', 'allow-spl-downgrade'), '');
  const result = box.run('save', '2025-06-01', '', '', '');
  assert.notEqual(result.status, 0, 'the floor is not overridable');
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
});

test('a baseline from an older module gains an OS version instead of losing it', t => {
  const box = sandbox(t);
  writeFileSync(
    path.join(box.dir, 'state', 'spl-baseline.conf'),
    'SYSTEM_SPL=2025-12-01\nVENDOR_SPL=2025-12-01\n',
  );
  const result = box.run('save', '', '', '', '17');
  assert.equal(result.status, 0, result.stderr);
  assert.equal(box.prop('ro.build.version.release'), '17');
  assert.match(box.state('spl-baseline.conf'), /^OS_VERSION=16$/m);
  // The recorded baseline is what a cleared field falls back to; here the
  // firmware files answer first and the device returns to its own release.
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

test('reapplying the current values does not restart the stack', t => {
  const box = sandbox(t);
  assert.equal(box.run('save', '2026-08-01', '', '2026-08-01', '16').status, 0);
  assert.equal(box.calls(), '');
});

test('boot-time apply reports a persisted downgrade without failing startup', t => {
  const box = sandbox(t);
  writeFileSync(
    path.join(box.dir, 'state', 'spl.conf'),
    'SYSTEM_SPL=2025-06-01\nBOOT_SPL=\nVENDOR_SPL=2025-06-01\nOS_VERSION=\n',
  );
  const result = box.run('apply');
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stderr, /refused to set ro\.build\.version\.security_patch to 2025-06-01/);
  assert.equal(box.prop('ro.build.version.security_patch'), '2026-08-01');
});
