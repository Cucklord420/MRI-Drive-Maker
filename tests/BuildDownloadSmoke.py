# SPDX-License-Identifier: GPL-3.0-only
"""Negative tests for release ZIP verification under normal and optimized Python."""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parents[1]
VERIFIER = ROOT / 'scripts' / 'Build-Download.py'


class BuildDownloadSmoke(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='mri-release-verifier-')
        cls.directory = Path(cls.temp.name)
        cls.valid = cls.directory / 'valid.zip'
        subprocess.run(
            [sys.executable, str(VERIFIER), '--output', str(cls.valid)],
            cwd=ROOT,
            check=True,
            capture_output=True,
            text=True,
        )

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def entries(self):
        with zipfile.ZipFile(self.valid) as package:
            return {info.filename: package.read(info.filename) for info in package.infolist()}

    def manifest(self, entries):
        path = next(name for name in entries if name.endswith('/RELEASE.json'))
        return path, json.loads(entries[path])

    def write_entries(self, path, entries):
        self.write_records(path, [(name, data, stat.S_IFREG | 0o644) for name, data in entries.items()])

    def write_records(self, path, records):
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_STORED) as package:
            for name, data, mode in records:
                info = zipfile.ZipInfo(name, date_time=(2000, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = mode << 16
                package.writestr(info, data)

    def variant(self, name, edit):
        entries = self.entries()
        edit(entries)
        path = self.directory / name
        self.write_entries(path, entries)
        return path

    def add_declared_source(self, entries, name, source):
        manifest_path, manifest = self.manifest(entries)
        data = (ROOT / source).read_bytes()
        prefix = manifest_path.rsplit('/', 1)[0] + '/'
        entries[prefix + name] = data
        manifest['sha256'][name] = hashlib.sha256(data).hexdigest()
        manifest['source_files'][name] = source
        entries[manifest_path] = (json.dumps(manifest, indent=2) + '\n').encode()

    def run_verifier(self, archive, mode):
        command = [sys.executable]
        environment = os.environ.copy()
        environment.pop('PYTHONOPTIMIZE', None)
        if mode == 'flag':
            command.append('-O')
        elif mode == 'environment':
            environment['PYTHONOPTIMIZE'] = '1'
        command.extend([str(VERIFIER), '--verify', str(archive)])
        return subprocess.run(command, cwd=ROOT, env=environment, capture_output=True, text=True)

    def assert_rejected_in_all_modes(self, archive, expected_message=None):
        for mode in ('normal', 'flag', 'environment'):
            with self.subTest(mode=mode):
                result = self.run_verifier(archive, mode)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn('"verified"', result.stdout)
                if expected_message is not None:
                    self.assertIn(expected_message, result.stdout + result.stderr)

    def test_valid_release_remains_verifiable_in_all_modes(self):
        for mode in ('normal', 'flag', 'environment'):
            with self.subTest(mode=mode):
                result = self.run_verifier(self.valid, mode)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn('"verified"', result.stdout)

    def test_tampered_payload_is_rejected_in_all_modes(self):
        entries = self.entries()
        member = next(name for name in entries if name.endswith('/App/MRI-Batch.ps1'))
        entries[member] += b'\n# altered fixture\n'
        archive = self.directory / 'payload-tamper.zip'
        self.write_entries(archive, entries)
        self.assert_rejected_in_all_modes(archive, 'MRI-Batch.ps1')

    def test_unauthorized_extra_member_is_rejected_in_all_modes(self):
        def edit(entries):
            prefix = next(name.split('/', 1)[0] for name in entries)
            entries[prefix + '/unexpected.txt'] = b'unapproved'

        self.assert_rejected_in_all_modes(self.variant('extra-member.zip', edit), 'Unexpected package entries')

    def test_changed_manifest_hash_is_rejected_in_all_modes(self):
        def edit(entries):
            manifest_path, manifest = self.manifest(entries)
            name = next(name for name in manifest['sha256'] if name.endswith('MRI-Batch.ps1'))
            manifest['sha256'][name] = '0' * 64
            entries[manifest_path] = (json.dumps(manifest, indent=2) + '\n').encode()

        self.assert_rejected_in_all_modes(self.variant('manifest-hash.zip', edit), 'MRI-Batch.ps1')

    def test_missing_launcher_is_rejected_in_all_modes(self):
        def edit(entries):
            name = next(name for name in entries if name.endswith('/START-MRI-DRIVE-MAKER.cmd'))
            del entries[name]

        self.assert_rejected_in_all_modes(self.variant('missing-launcher.zip', edit), 'Unexpected package entries')

    def test_bad_source_commit_is_rejected_in_all_modes(self):
        def edit(entries):
            manifest_path, manifest = self.manifest(entries)
            manifest['source_commit'] = 'not-a-commit'
            entries[manifest_path] = (json.dumps(manifest, indent=2) + '\n').encode()

        self.assert_rejected_in_all_modes(self.variant('bad-commit.zip', edit), 'Invalid source commit')

    def test_modified_payload_with_matching_hash_still_requires_git_source_bytes(self):
        def edit(entries):
            manifest_path, manifest = self.manifest(entries)
            name = next(name for name in manifest['sha256'] if name.endswith('MRI-Batch.ps1'))
            archive_name = manifest_path.rsplit('/', 1)[0] + '/' + name
            altered = entries[archive_name] + b'\n# altered fixture\n'
            entries[archive_name] = altered
            manifest['sha256'][name] = hashlib.sha256(altered).hexdigest()
            entries[manifest_path] = (json.dumps(manifest, indent=2) + '\n').encode()

        self.assert_rejected_in_all_modes(
            self.variant('source-byte-mismatch.zip', edit),
            'Source differs from its declared Git commit',
        )

    def test_competing_launcher_is_rejected_in_all_modes(self):
        def edit(entries):
            manifest_path, manifest = self.manifest(entries)
            original = next(name for name in manifest['sha256'] if name == 'START-MRI-DRIVE-MAKER.cmd')
            alternate = 'START-ALTERNATE.cmd'
            prefix = manifest_path.rsplit('/', 1)[0] + '/'
            entries[prefix + alternate] = entries[prefix + original]
            manifest['sha256'][alternate] = hashlib.sha256(entries[prefix + alternate]).hexdigest()
            manifest['source_files'][alternate] = manifest['source_files'][original]
            entries[manifest_path] = (json.dumps(manifest, indent=2) + '\n').encode()

        self.assert_rejected_in_all_modes(self.variant('competing-launcher.zip', edit), 'Competing launchers')

    def test_excluded_legacy_tree_is_rejected_in_all_modes(self):
        def edit(entries):
            manifest_path, manifest = self.manifest(entries)
            member = 'legacy/old.ps1'
            source = 'legacy/MRI_CreatorV3.ps1.txt'
            data = (ROOT / source).read_bytes()
            entries[manifest_path.rsplit('/', 1)[0] + '/' + member] = data
            manifest['sha256'][member] = hashlib.sha256(data).hexdigest()
            manifest['source_files'][member] = source
            entries[manifest_path] = (json.dumps(manifest, indent=2) + '\n').encode()

        self.assert_rejected_in_all_modes(self.variant('excluded-legacy.zip', edit), 'Historical material belongs in the repository')

    def test_duplicate_payload_member_is_rejected_before_name_lookup(self):
        entries = self.entries()
        target = next(name for name in entries if name.endswith('/App/MRI-Batch.ps1'))
        records = []
        for name, data in entries.items():
            if name == target:
                records.append((name, b'not approved payload', stat.S_IFREG | 0o644))
            records.append((name, data, stat.S_IFREG | 0o644))
        path = self.directory / 'duplicate-payload.zip'
        self.write_records(path, records)
        self.assert_rejected_in_all_modes(path, 'Duplicate package entries')

    def test_duplicate_release_manifest_is_rejected(self):
        entries = self.entries()
        manifest_path = next(name for name in entries if name.endswith('/RELEASE.json'))
        records = []
        for name, data in entries.items():
            if name == manifest_path:
                records.append((name, b'not the approved manifest', stat.S_IFREG | 0o644))
            records.append((name, data, stat.S_IFREG | 0o644))
        path = self.directory / 'duplicate-release-manifest.zip'
        self.write_records(path, records)
        self.assert_rejected_in_all_modes(path, 'Duplicate package entries')

    def test_case_insensitive_windows_alias_is_rejected(self):
        entries = self.entries()
        self.add_declared_source(entries, 'app/MRI-Batch.ps1', 'MRI-Batch.ps1')
        path = self.directory / 'case-alias.zip'
        self.write_entries(path, entries)
        self.assert_rejected_in_all_modes(path, 'Duplicate normalized package entries')

    def test_unicode_member_name_is_rejected(self):
        entries = self.entries()
        self.add_declared_source(entries, 'App/caf\u00e9.txt', 'README.md')
        path = self.directory / 'unicode-name.zip'
        self.write_entries(path, entries)
        self.assert_rejected_in_all_modes(path, 'Non-ASCII ZIP member names')

    def test_file_directory_path_collision_is_rejected(self):
        entries = self.entries()
        self.add_declared_source(entries, 'App/path-conflict', 'README.md')
        self.add_declared_source(entries, 'App/path-conflict/child.txt', 'LICENSE')
        path = self.directory / 'file-directory-collision.zip'
        self.write_entries(path, entries)
        self.assert_rejected_in_all_modes(path, 'File/directory collision')

    def test_unsafe_windows_extraction_paths_are_rejected(self):
        cases = (
            ('../../outside.txt', 'README.md', 'Unsafe ZIP member path'),
            ('App\\outside.txt', 'README.md', None),
            ('App/CON.txt', 'README.md', 'Windows device alias'),
            ('App/trailing.', 'README.md', 'Windows-trimmed ZIP path component'),
        )
        for index, (member, source, expected) in enumerate(cases):
            with self.subTest(member=member):
                entries = self.entries()
                self.add_declared_source(entries, member, source)
                path = self.directory / f'unsafe-path-{index}.zip'
                self.write_entries(path, entries)
                self.assert_rejected_in_all_modes(path, expected)

    def test_symlink_member_type_is_rejected(self):
        entries = self.entries()
        path = self.directory / 'symlink-member.zip'
        target = next(name for name in entries if name.endswith('/App/MRI-Batch.ps1'))
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_STORED) as package:
            for name, data in entries.items():
                info = zipfile.ZipInfo(name, date_time=(2000, 1, 1, 0, 0, 0))
                info.create_system = 3
                file_mode = stat.S_IFLNK | 0o777 if name == target else stat.S_IFREG | 0o644
                info.external_attr = file_mode << 16
                package.writestr(info, data)
        self.assert_rejected_in_all_modes(path, 'Unexpected ZIP member type')

    def test_directory_member_type_is_rejected(self):
        entries = self.entries()
        prefix = next(name.split('/', 1)[0] for name in entries)
        entries[prefix + '/App/empty-directory/'] = b''
        path = self.directory / 'directory-member.zip'
        self.write_entries(path, entries)
        self.assert_rejected_in_all_modes(path, 'Unexpected ZIP member type')

    def test_windows_trimmed_package_root_is_rejected(self):
        entries = self.entries()
        root = next(name.split('/', 1)[0] for name in entries)
        renamed = {name.replace(root + '/', root + './', 1): data for name, data in entries.items()}
        path = self.directory / 'trimmed-package-root.zip'
        self.write_entries(path, renamed)
        self.assert_rejected_in_all_modes(path, 'Windows-trimmed package root')

    def test_builder_rejects_path_shaped_version_before_writing(self):
        archive = self.directory / 'invalid-version.zip'
        result = subprocess.run(
            [sys.executable, str(VERIFIER), '--version', '../../outside', '--output', str(archive)],
            cwd=ROOT, capture_output=True, text=True,
        )
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('unsupported path characters', result.stdout + result.stderr)
        self.assertFalse(archive.exists())

    def test_bad_zip_crc_is_rejected_in_all_modes(self):
        raw = bytearray(self.valid.read_bytes())
        with zipfile.ZipFile(io.BytesIO(raw)) as package:
            info = next(item for item in package.infolist() if item.filename.endswith('/App/MRI-Batch.ps1'))
        filename_length = struct.unpack_from('<H', raw, info.header_offset + 26)[0]
        extra_length = struct.unpack_from('<H', raw, info.header_offset + 28)[0]
        data_start = info.header_offset + 30 + filename_length + extra_length
        raw[data_start] ^= 1
        archive = self.directory / 'bad-crc.zip'
        archive.write_bytes(raw)
        self.assert_rejected_in_all_modes(archive, 'ZIP CRC check failed')


if __name__ == '__main__':
    unittest.main()
