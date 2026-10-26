# SPDX-License-Identifier: GPL-3.0-only
# Updated 10-15-2026. Small, reproducible source download with one launcher.
from pathlib import Path
import argparse
import hashlib
import json
import re
import subprocess
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--version', default='0.3.0')
parser.add_argument('--output', type=Path)
parser.add_argument('--verify', type=Path)
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parent

def verify(path):
    with zipfile.ZipFile(path) as package:
        assert package.testzip() is None, 'ZIP CRC check failed'
        names = package.namelist()
        prefixes = {name.split('/')[0] for name in names}
        assert len(prefixes) == 1, 'Expected one top-level directory'
        prefix = prefixes.pop() + '/'
        manifest = json.loads(package.read(prefix + 'RELEASE.json'))
        revision = manifest['source_commit']
        assert re.fullmatch(r'[0-9a-f]{40}', revision), 'Invalid source commit'
        assert set(names) == {prefix + name for name in manifest['sha256']} | {prefix + 'RELEASE.json'}, 'Unexpected package entries'
        assert set(manifest['sha256']) == set(manifest['source_files']), 'Every packaged file must have a source origin'
        for name, source in manifest['source_files'].items():
            data = package.read(prefix + name)
            assert hashlib.sha256(data).hexdigest() == manifest['sha256'][name], name
            expected = subprocess.check_output(['git', 'show', revision + ':' + source], cwd=repo)
            assert data == expected, 'Source differs from its declared Git commit: ' + name
        launchers = [name for name in manifest['sha256'] if '/' not in name and name.lower().endswith(('.cmd', '.ps1', '.exe'))]
        assert launchers == ['START-MRI-DRIVE-MAKER.cmd'], 'Competing launchers'
        assert not any(name.startswith(('Developer/', 'legacy/')) for name in manifest['sha256']), 'Historical material belongs in the repository'
    return manifest

if args.verify:
    manifest = verify(args.verify)
    print(json.dumps({'verified': str(args.verify), 'source_commit': manifest['source_commit'], 'files': len(manifest['sha256'])}))
else:
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
    archive = args.output or repo / 'downloads' / f'MRI-Drive-Maker-v{args.version}.zip'
    archive.parent.mkdir(parents=True, exist_ok=True)
    entries = {}
    origins = {}

    def add(source, destination):
        # Read committed bytes so checkout line-ending conversion cannot change a release.
        entries[destination] = subprocess.check_output(['git', 'show', revision + ':' + source], cwd=repo)
        origins[destination] = source

    for name in ('START-MRI-DRIVE-MAKER.cmd', 'READ-ME-FIRST.txt', 'LICENSE', 'NOTICE.md'):
        add(name, name)
    for name in ('MRI-Batch.ps1', 'MRI.Batch.psm1', 'Copy-MRIFiles.ps1', 'NativeProcess.cs', 'mri-config.example.json', 'LICENSE'):
        add(name, 'App/' + name)
    for path in sorted((repo / 'tests').glob('*.ps1')):
        add(path.relative_to(repo).as_posix(), 'App/tests/' + path.name)
    manifest = {
        'version': args.version,
        'source_commit': revision,
        'launcher': 'START-MRI-DRIVE-MAKER.cmd',
        'source_files': origins,
        'sha256': {name: hashlib.sha256(data).hexdigest() for name, data in entries.items()},
    }
    entries['RELEASE.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    # A fixed timestamp and stored entries make the output byte-reproducible
    # across Python/zlib versions. The small text-source package needs no history bundle.
    prefix = f'MRI-Drive-Maker-v{args.version}/'
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_STORED) as package:
        for name, data in sorted(entries.items()):
            info = zipfile.ZipInfo(prefix + name, date_time=(2000, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            package.writestr(info, data)
    verify(archive)
    print(json.dumps({'path': str(archive), 'sha256': hashlib.sha256(archive.read_bytes()).hexdigest(), 'bytes': archive.stat().st_size, 'files': len(entries), 'source_commit': revision}))
