# SPDX-License-Identifier: GPL-3.0-only
# Added 10-13-2026. Clean-layout source download builder.
from pathlib import Path
from datetime import datetime, timezone
import argparse
import hashlib
import json
import subprocess
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--version', default='0.2.0')
parser.add_argument('--history-bundle', type=Path)
parser.add_argument('--output', type=Path)
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parent
revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo, text=True).strip()
archive = args.output or repo / 'downloads' / f'MRI-Drive-Maker-v{args.version}.zip'
archive.parent.mkdir(parents=True, exist_ok=True)
prefix = f'MRI-Drive-Maker-v{args.version}/'
entries = {}
origins = {}

def add(source, destination):
    entries[destination] = (repo / source).read_bytes()
    origins[destination] = source

for name in ('START-MRI-DRIVE-MAKER.cmd', 'READ-ME-FIRST.txt', 'LICENSE', 'NOTICE.md'):
    add(name, name)
for name in ('MRI-Batch.ps1', 'MRI.Batch.psm1', 'Copy-MRIFiles.ps1', 'NativeProcess.cs', 'mri-config.example.json', 'LICENSE'):
    add(name, 'App/' + name)
for path in sorted((repo / 'tests').glob('*.ps1')):
    add(str(path.relative_to(repo)), 'App/tests/' + path.name)
for name in ('README.md', 'CHANGELOG.md', 'VALIDATION.txt'):
    add(name, 'Developer/' + name)
for directory in ('legacy', 'validation', 'scripts'):
    for path in sorted((repo / directory).rglob('*')):
        if path.is_file() and '__pycache__' not in path.parts and not path.name.startswith('_'):
            add(str(path.relative_to(repo)), 'Developer/' + str(path.relative_to(repo)))
for name in ('MRI_CreatorV3.ps1', 'MRI_CreatorV4.ps1', 'Start-MRI-Batch.cmd'):
    add(name, 'Developer/compatibility/' + name + '.txt')
if args.history_bundle:
    entries['Developer/Source-History.git.bundle'] = args.history_bundle.read_bytes()
manifest = {
    'version': args.version,
    'created_utc': datetime.now(timezone.utc).isoformat(),
    'source_commit': revision,
    'launcher': 'START-MRI-DRIVE-MAKER.cmd',
    'source_files': origins,
    'sha256': {name: hashlib.sha256(data).hexdigest() for name, data in entries.items()},
}
entries['RELEASE.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
assert [name for name in entries if '/' not in name and name.lower().endswith(('.cmd', '.ps1', '.exe'))] == ['START-MRI-DRIVE-MAKER.cmd']
with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as package:
    for name, data in sorted(entries.items()):
        package.writestr(prefix + name, data)
with zipfile.ZipFile(archive) as package:
    assert package.testzip() is None
    for name, data in entries.items():
        assert package.read(prefix + name) == data
print(json.dumps({'path': str(archive), 'sha256': hashlib.sha256(archive.read_bytes()).hexdigest(), 'bytes': archive.stat().st_size, 'files': len(entries), 'source_commit': revision}))
