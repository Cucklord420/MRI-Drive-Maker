# SPDX-License-Identifier: GPL-3.0-only
# Updated 10-15-2026. Small, reproducible source download with one launcher.
from pathlib import Path
from collections.abc import Mapping
import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import zipfile


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
    """Accept only canonical ASCII relative paths safe on Windows."""
    require(isinstance(name, str) and name.isascii(),
            'Non-ASCII ZIP member names are not allowed')
    require('\x00' not in name and '\\' not in name,
            'Noncanonical ZIP member path')
    parts = name.split('/')
    require(len(parts) >= 2 and all(parts),
            'Noncanonical ZIP member path')
    root = parts[0]
    require(PACKAGE_ROOT_RE.fullmatch(root) is not None, 'Invalid package root')
    require(not root.endswith('.'), 'Windows-trimmed package root')
    for component in parts[1:]:
        require(component not in ('.', '..')
                and PACKAGE_COMPONENT_RE.fullmatch(component) is not None,
                'Unsafe ZIP member path')
        require(not component.endswith('.'), 'Windows-trimmed ZIP path component')
        device_stem = component.split('.', 1)[0].upper()
        require(device_stem not in WINDOWS_DEVICE_NAMES,
                'Windows device alias in ZIP member path')
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
        require(all('/'.join(parts[:index]) not in canonical_set
                    for index in range(1, len(parts))),
                'File/directory collision in ZIP member paths')
    return names


def package_sources(repo):
    """Return (source path, package path) pairs for the official download."""
    sources = []
    for name in ('START-MRI-DRIVE-MAKER.cmd', 'READ-ME-FIRST.txt', 'LICENSE', 'NOTICE.md'):
        sources.append((name, name))
    for name in ('MRI-Batch.ps1', 'MRI.Batch.psm1', 'Copy-MRIFiles.ps1',
                 'NativeProcess.cs', 'mri-config.example.json', 'LICENSE'):
        sources.append((name, 'App/' + name))
    sources.extend(
        (path.relative_to(repo).as_posix(), 'App/tests/' + path.name)
        for path in sorted((repo / 'tests').glob('*.ps1'))
    )
    return sources


def _git_environment(environment=None):
    """Copy the process environment while removing every Git override."""
    source = os.environ if environment is None else environment
    if not isinstance(source, Mapping):
        source = dict(source)
    return {key: value for key, value in source.items()
            if not key.upper().startswith('GIT_')}


def _repository_layout(repo):
    root = Path(repo).resolve(strict=True)
    if not root.is_dir():
        raise RuntimeError('The intended Git worktree is not a directory')
    metadata = root / '.git'
    if metadata.is_dir():
        git_dir = metadata.resolve(strict=True)
        common_dir = git_dir
    elif metadata.is_file():
        lines = metadata.read_text(encoding='utf-8').splitlines()
        if len(lines) != 1 or not lines[0].startswith('gitdir: '):
            raise RuntimeError('The intended Git worktree has an invalid .git file')
        git_dir_ref = Path(lines[0][len('gitdir: '):])
        if not git_dir_ref.is_absolute():
            git_dir_ref = root / git_dir_ref
        git_dir = git_dir_ref.resolve(strict=True)
        common_file = git_dir / 'commondir'
        if not common_file.is_file():
            raise RuntimeError('Only linked Git worktrees with a commondir file are supported')
        common_text = common_file.read_text(encoding='utf-8')
        common_value = common_text.rstrip('\r\n')
        if not common_value or '\n' in common_value or '\r' in common_value:
            raise RuntimeError('The linked worktree has an invalid commondir file')
        common_ref = Path(common_value)
        if not common_ref.is_absolute():
            common_ref = git_dir / common_ref
        common_dir = common_ref.resolve(strict=True)
        if git_dir.parent != (common_dir / 'worktrees').resolve(strict=True):
            raise RuntimeError('The linked Git worktree metadata is outside its common repository')
    else:
        raise RuntimeError('The intended source directory is not a Git worktree')
    return root, git_dir, common_dir


def _git_unchecked(repo, arguments, environment):
    return subprocess.check_output(['git', *arguments], cwd=repo, env=environment)


def _git_path(output, label):
    try:
        value = output.decode('utf-8')
    except UnicodeDecodeError as error:
        raise RuntimeError(f'Git returned a non-UTF-8 {label}') from error
    if not value.endswith('\n'):
        raise RuntimeError(f'Git returned an incomplete {label}')
    value = value[:-1]
    if value.endswith('\r'):
        value = value[:-1]
    if '\n' in value or '\r' in value:
        raise RuntimeError(f'Git returned an ambiguous {label}')
    return Path(value).resolve(strict=True)


def _same_filesystem_path(left, right):
    """Compare existing paths by filesystem identity, including Windows aliases."""
    try:
        return os.path.samefile(left, right)
    except OSError:
        return False


def _assert_git_identity(repo, environment):
    expected_root, expected_git_dir, expected_common_dir = _repository_layout(repo)
    actual_root = _git_path(_git_unchecked(
        expected_root, ['rev-parse', '--path-format=absolute', '--show-toplevel'], environment),
        'worktree root')
    actual_git_dir = _git_path(_git_unchecked(
        expected_root, ['rev-parse', '--absolute-git-dir'], environment), 'Git directory')
    actual_common_dir = _git_path(_git_unchecked(
        expected_root, ['rev-parse', '--path-format=absolute', '--git-common-dir'], environment),
        'common Git directory')
    if (not _same_filesystem_path(actual_root, expected_root)
            or not _same_filesystem_path(actual_git_dir, expected_git_dir)
            or not _same_filesystem_path(actual_common_dir, expected_common_dir)):
        raise RuntimeError('Git resolved a different worktree or repository than the intended source')


def _git(repo, arguments, environment=None):
    clean_environment = _git_environment(environment)
    _assert_git_identity(repo, clean_environment)
    return _git_unchecked(Path(repo).resolve(strict=True), arguments, clean_environment)


def verify(path, repo=None, environment=None):
    repo = Path(__file__).resolve().parent.parent if repo is None else Path(repo)
    clean_environment = _git_environment(environment)
    _assert_git_identity(repo, clean_environment)
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
        require(manifest['version'] == version_match.group(1),
                'Package version does not match its root')
        revision = manifest['source_commit']
        require(re.fullmatch(r'[0-9a-f]{40}', revision) is not None,
                'Invalid source commit')
        require(set(names) == {prefix + name for name in manifest['sha256']}
                | {prefix + 'RELEASE.json'}, 'Unexpected package entries')
        require(set(manifest['sha256']) == set(manifest['source_files']),
                'Every packaged file must have a source origin')
        for name, source in manifest['source_files'].items():
            data = package.read(prefix + name)
            require(hashlib.sha256(data).hexdigest() == manifest['sha256'][name], name)
            expected = _git(repo, ['show', revision + ':' + source], clean_environment)
            require(data == expected,
                    'Source differs from its declared Git commit: ' + name)
        launchers = [name for name in manifest['sha256'] if '/' not in name and name.lower().endswith(('.cmd', '.ps1', '.exe'))]
        require(launchers == ['START-MRI-DRIVE-MAKER.cmd'], 'Competing launchers')
        require(not any(name.startswith(('Developer/', 'legacy/'))
                        for name in manifest['sha256']),
                'Historical material belongs in the repository')
    return manifest


def build(repo, archive, version='0.3.0', source_files=None, environment=None):
    require(VERSION_RE.fullmatch(version) is not None and not version.endswith('.'),
            'Version contains unsupported path characters')
    repo = Path(repo)
    archive = Path(archive)
    clean_environment = _git_environment(environment)
    _assert_git_identity(repo, clean_environment)
    revision = _git(repo, ['rev-parse', 'HEAD'], clean_environment).decode('utf-8').strip()
    archive.parent.mkdir(parents=True, exist_ok=True)
    entries = {}
    origins = {}

    for source, destination in (package_sources(repo) if source_files is None else source_files):
        entries[destination] = _git(repo, ['show', revision + ':' + source], clean_environment)
        origins[destination] = source
    manifest = {
        'version': version,
        'source_commit': revision,
        'launcher': 'START-MRI-DRIVE-MAKER.cmd',
        'source_files': origins,
        'sha256': {name: hashlib.sha256(data).hexdigest() for name, data in entries.items()},
    }
    entries['RELEASE.json'] = (json.dumps(manifest, indent=2) + '\n').encode()
    # A fixed timestamp and stored entries make the output byte-reproducible
    # across Python/zlib versions. The small text-source package needs no history bundle.
    prefix = f'MRI-Drive-Maker-v{version}/'
    with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_STORED) as package:
        for name, data in sorted(entries.items()):
            info = zipfile.ZipInfo(prefix + name, date_time=(2000, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            package.writestr(info, data)
    checked = verify(archive, repo, clean_environment)
    if checked != manifest:
        raise RuntimeError('Release verification differs from original manifest')
    return manifest


def main(argv=None):
    parser = argparse.ArgumentParser()
    parser.add_argument('--version', default='0.3.0')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--verify', type=Path)
    args = parser.parse_args(argv)
    repo = Path(__file__).resolve().parent.parent
    if args.verify:
        manifest = verify(args.verify, repo)
        print(json.dumps({'verified': str(args.verify), 'source_commit': manifest['source_commit'], 'files': len(manifest['sha256'])}))
    else:
        archive = args.output or repo / 'downloads' / f'MRI-Drive-Maker-v{args.version}.zip'
        manifest = build(repo, archive, args.version)
        print(json.dumps({'path': str(archive), 'sha256': hashlib.sha256(archive.read_bytes()).hexdigest(),
                          'bytes': archive.stat().st_size, 'files': len(manifest['sha256']) + 1,
                          'source_commit': manifest['source_commit']}))


if __name__ == '__main__':
    main()
