# SPDX-License-Identifier: GPL-3.0-only
"""Negative tests for release ZIP verification under normal and optimized Python."""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
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
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_STORED) as package:
            for name, data in entries.items():
                info = zipfile.ZipInfo(name, date_time=(2000, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = 0o100644 << 16
                package.writestr(info, data)

    def variant(self, name, edit):
        entries = self.entries()
        edit(entries)
        path = self.directory / name
        self.write_entries(path, entries)
        return path

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

    def assert_rejected_in_all_modes(self, archive, expected_message):
        for mode in ('normal', 'flag', 'environment'):
            with self.subTest(mode=mode):
                result = self.run_verifier(archive, mode)
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertNotIn('"verified"', result.stdout)
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
