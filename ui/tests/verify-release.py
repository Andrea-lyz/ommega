"""Validate a local Android release ZIP against current sources, without devices.

Usage: python ui/tests/verify-release.py a|b path/to/module.zip [--publish]
--publish copies the verified ZIP to target/releases/refresh-battery/ and writes
its SHA256 sidecar, keeping the deliverable separate from build-script cleanup.
"""
from __future__ import annotations

import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import struct
import zipfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('side', choices=['a', 'b'])
parser.add_argument('archive', type=Path)
parser.add_argument('--publish', action='store_true')
parser.add_argument('--refresh-webroot', action='store_true', help='Repack only current UI assets after a successful native release build; never changes installer or native entries')
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
source = root / f'{args.side}-side/source'
archive = args.archive.resolve(strict=True)
assert archive.parent == (source / 'target').resolve(), archive

if args.refresh_webroot:
    # Native sources must already have passed their real release build. This
    # copies no executable and changes no native/installer entry from that ZIP.
    with zipfile.ZipFile(archive) as original:
        assert original.testzip() is None
        entries = [(entry, original.read(entry.filename)) for entry in original.infolist() if not entry.filename.startswith('webroot/')]
        hashed_ui = any(name.startswith('webroot/') and name.endswith('.sha256') for name in original.namelist())
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w', zipfile.ZIP_DEFLATED) as rebuilt:
        for entry, content in entries:
            rebuilt.writestr(entry, content)
        for path in sorted((source / 'template/webroot').rglob('*')):
            if not path.is_file() or path.name.endswith('.sha256'):
                continue
            name = path.relative_to(source / 'template').as_posix()
            content = path.read_bytes()
            rebuilt.writestr(name, content)
            if hashed_ui:
                rebuilt.writestr(name + '.sha256', hashlib.sha256(content).hexdigest())
    archive.write_bytes(buffer.getvalue())

with zipfile.ZipFile(archive) as package:
    assert package.testzip() is None, 'ZIP CRC failure'
    names = package.namelist()
    assert len(names) == len(set(names)), 'duplicate archive entries'
    assert all(not name.startswith(('/', '\\')) and '..' not in Path(name).parts for name in names)
    hashes = [name for name in names if name.endswith('.sha256')]
    for name in hashes:
        target = name[:-7]
        assert package.read(name).decode('ascii').strip().lower() == hashlib.sha256(package.read(target)).hexdigest(), name
    webroot = source / 'template/webroot'
    expected = {file.relative_to(source / 'template').as_posix():file for file in webroot.rglob('*') if file.is_file() and not file.name.endswith('.sha256')}
    actual = {name for name in names if name.startswith('webroot/') and not name.endswith('.sha256')}
    assert actual == set(expected), {'missing':sorted(set(expected)-actual),'extra':sorted(actual-set(expected))}
    for name, path in expected.items():
        assert package.read(name) == path.read_bytes(), f'stale packaged UI: {name}'
    binaries = ['keymint','ommega-auth-policy','ommega-cpu-policy','ommega-capability-policy','ommega-inject'] if args.side == 'a' else ['relay']
    for binary in binaries:
        name = f'libs/arm64-v8a/{binary}'
        data = package.read(name)
        assert data == (source / f'target/aarch64-linux-android/release/{binary}').read_bytes(), name
        assert data[:6] == b'\x7fELF\x02\x01' and struct.unpack_from('<H',data,18)[0] == 183, f'not ARM64 ELF: {name}'
    if args.side == 'a':
        modern = package.read('webroot/modern.js').decode('utf-8')
        assert all(token in modern for token in ['battery_updated_at_ms','server_time_ms','om-b-battery','refreshAState'])
        bundle = next(name for name in actual if name.startswith('webroot/assets/index-') and name.endswith('.js'))
        assert 'ommegaRefreshApps' in package.read(bundle).decode('utf-8')
        assert 'refreshScope = false' in package.read(bundle).decode('utf-8')
        assert package.read('libs/arm64-v8a/soterta-svc') == (source / 'target/soterta-svc/arm64-v8a/release/soterta-svc').read_bytes()
    else:
        app = package.read('webroot/app.js').decode('utf-8')
        assert all(token in app for token in ['status --readonly','refreshData(forced)','{read:true}'])
        control = package.read('spl-control.sh').decode('utf-8')
        assert control == (source / 'template/spl-control.sh').read_text(encoding='utf-8').replace('\r\n','\n')
        assert '[ "$1" = "--readonly" ]' in control
        relay = package.read('libs/arm64-v8a/relay')
        assert b'battery_percent=' in relay and b'battery_charging=' in relay

sha256 = hashlib.sha256(archive.read_bytes()).hexdigest().upper()
deliverable = archive
if args.publish:
    directory = source / 'target/releases/refresh-battery'
    directory.mkdir(parents=True, exist_ok=True)
    deliverable = directory / archive.name
    if deliverable.exists():
        assert deliverable.read_bytes() == archive.read_bytes(), 'refuse to overwrite a different published artifact'
    else:
        shutil.copy2(archive, deliverable)
    sidecar = deliverable.with_name(deliverable.name + '.sha256')
    expected_hash = f'{sha256}  {deliverable.name}\n'
    if sidecar.exists():
        assert sidecar.read_text(encoding='utf-8') == expected_hash, 'refuse to overwrite a different published hash'
    else:
        sidecar.write_text(expected_hash, encoding='utf-8', newline='\n')
print(json.dumps({'side':args.side,'artifact':str(deliverable),'bytes':deliverable.stat().st_size,'sha256':sha256,'entries':len(names),'webroot_files':len(actual),'sidecars':len(hashes),'crc':'ok','current_sources':'match','android_abi':'arm64-v8a'},ensure_ascii=False,indent=2))
