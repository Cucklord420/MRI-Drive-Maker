# SPDX-License-Identifier: GPL-3.0-only
# Updated 10-15-2026. Small, reproducible source download with one launcher.
from pathlib import Path
import argparse
import hashlib
import json
import re
import stat
import subprocess
import zipfile

parser = argparse.ArgumentParser()
parser.add_argument('--version', default='0.3.0')
parser.add_argument('--output', type=Path)
parser.add_argument('--verify', type=Path)
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parent
VERSION_RE = re.compile(r'^[A-Za-z0-9][A-Za-z0-9._-]{0,48}$')
PACKAGE_ROOT_RE = re.compile(r'^MRI-Drive-Maker-v([A-Za-z0-9][A-Za-z0-9._-]{0,48})$')
PACKAGE_COMPONENT_RE = re.compile(r'^[A-Za-z0-9._-]+$')
WINDOWS_DEVICE_NAMES = {'CON', 'PRN', 'AUX', 'NUL'} | {
    f'{prefix}{number}' for prefix in ('COM', 'LPT') for number in range(1, 10)
}

def require(condition, message):
    """Raise for failed release checks even when Python optimization is enabled."""
    if not condition:
        raise ValueError(message)

def canonical_member_name(name):
    """Accept only canonical ASCII relative file paths safe on Windows."""
    require(isinstance(name, str) and name.isascii(), 'Non-ASCII ZIP member names are not allowed')
    require('\x00' not in name and '\\' not in name, 'Noncanonical ZIP member path')
    parts = name.split('/')
    require(len(parts) >= 2 and all(parts), 'Noncanonical ZIP member path')
    root = parts[0]
    require(PACKAGE_ROOT_RE.fullmatch(root) is not None, 'Invalid package root')
    require(not root.endswith('.'), 'Windows-trimmed package root')
    for component in parts[1:]:
        require(component not in ('.', '..') and PACKAGE_COMPONENT_RE.fullmatch(component) is not None,
                'Unsafe ZIP member path')
        require(not component.endswith('.'), 'Windows-trimmed ZIP path component')
        device_stem = component.split('.', 1)[0].upper()
        require(device_stem not in WINDOWS_DEVICE_NAMES, 'Windows device alias in ZIP member path')
    require(len(name) <= 200, 'ZIP member path is too long')
    return '/'.join(component.lower() for component in parts)

def validate_archive_members(package):
    infos = package.infolist()
    names = [info.filename for info in infos]
    require(bool(names), 'Empty package archive')
    require(all(info.orig_filename == info.filename for info in infos),
            'Noncanonical ZIP member name encoding')
    require(len(names) == len(set(names)), 'Duplicate package entries')
    canonical_names = []
    for info in infos:
        require(not info.is_dir() and (info.external_attr & 0x10) == 0,
                'Unexpected ZIP member type')
        mode = info.external_attr >> 16
        require(info.create_system == 3 and stat.S_IFMT(mode) == stat.S_IFREG,
                'Unexpected ZIP member type')
        canonical_names.append(canonical_member_name(info.filename))
    require(len(canonical_names) == len(set(canonical_names)),
            'Duplicate normalized package entries')
    canonical_set = set(canonical_names)
    for name in canonical_names:
        parts = name.split('/')
        require(all('/'.join(parts[:index]) not in canonical_set for index in range(1, len(parts))),
                'File/directory collision in ZIP member paths')
    return names

def verify(path):
    with zipfile.ZipFile(path) as package:
        require(package.testzip() is None, 'ZIP CRC check failed')
        names = validate_archive_members(package)
        prefixes = {name.split('/')[0] for name in names}
        require(len(prefixes) == 1, 'Expected one top-level directory')
        root = prefixes.pop()
        version_match = PACKAGE_ROOT_RE.fullmatch(root)
        require(version_match is not None, 'Invalid package root')
        prefix = root + '/'
        manifest = json.loads(package.read(prefix + 'RELEASE.json'))
        require(manifest['version'] == version_match.group(1), 'Package version does not match its root')
        revision = manifest['source_commit']
        require(re.fullmatch(r'[0-9a-f]{40}', revision) is not None, 'Invalid source commit')
        require(set(names) == {prefix + name for name in manifest['sha256']} | {prefix + 'RELEASE.json'}, 'Unexpected package entries')
        require(set(manifest['sha256']) == set(manifest['source_files']), 'Every packaged file must have a source origin')
        for name, source in manifest['source_files'].items():
            data = package.read(prefix + name)
            require(hashlib.sha256(data).hexdigest() == manifest['sha256'][name], name)
            expected = subprocess.check_output(['git', 'show', revision + ':' + source], cwd=repo)
            require(data == expected, 'Source differs from its declared Git commit: ' + name)
        launchers = [name for name in manifest['sha256'] if '/' not in name and name.lower().endswith(('.cmd', '.ps1', '.exe'))]
        require(launchers == ['START-MRI-DRIVE-MAKER.cmd'], 'Competing launchers')
        require(not any(name.startswith(('Developer/', 'legacy/')) for name in manifest['sha256']), 'Historical material belongs in the repository')
    return manifest

if args.verify:
    manifest = verify(args.verify)
    print(json.dumps({'verified': str(args.verify), 'source_commit': manifest['source_commit'], 'files': len(manifest['sha256'])}))
else:
    require(VERSION_RE.fullmatch(args.version) is not None and not args.version.endswith('.'),
            'Version contains unsupported path characters')
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
