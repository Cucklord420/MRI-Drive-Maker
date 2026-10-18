#!/usr/bin/env python3
"""Regression coverage for release provenance under inherited Git overrides."""
from pathlib import Path
import hashlib
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
import zipfile


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / 'scripts' / 'Build-Download.py'
SPEC = importlib.util.spec_from_file_location('mri_release_builder', SCRIPT)
BUILDER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILDER)


def clean_environment():
    return {key: value for key, value in os.environ.items()
            if not key.upper().startswith('GIT_')}


def run_git(repo, arguments, environment=None):
    result = subprocess.run(['git', *arguments], cwd=repo,
                            env=clean_environment() if environment is None else environment,
                            check=True, capture_output=True)
    return result.stdout


def create_repo(path, source, launcher):
    path.mkdir(parents=True)
    env = clean_environment()
    run_git(path, ['init', '-q'], env)
    run_git(path, ['config', 'core.autocrlf', 'false'], env)
    run_git(path, ['config', 'user.name', 'MRI provenance test'], env)
    run_git(path, ['config', 'user.email', 'mri-provenance@example.invalid'], env)
    # Keep the synthetic Git blob identical on Windows and Unix. Text-mode
    # writes otherwise translate LF to CRLF before the failing-environment
    # reproduction compares the bytes returned by git show.
    (path / 'SOURCE.txt').write_bytes(source.encode('utf-8'))
    (path / 'START-MRI-DRIVE-MAKER.cmd').write_text(launcher, encoding='utf-8', newline='')
    run_git(path, ['add', 'SOURCE.txt', 'START-MRI-DRIVE-MAKER.cmd'], env)
    run_git(path, ['commit', '-m', 'synthetic provenance fixture'], env)
    return run_git(path, ['rev-parse', 'HEAD'], env).decode().strip()


class GitContextTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mri-release-git-context-')
        self.root = Path(self.temp.name)
        self.intended = self.root / 'intended'
        self.alternate = self.root / 'alternate'
        self.intended_commit = create_repo(self.intended, 'intended source bytes\n', '@echo intended\r\n')
        self.alternate_commit = create_repo(self.alternate, 'alternate source bytes\n', '@echo alternate\r\n')
        self.config = self.root / 'attacker.gitconfig'
        self.config.write_text('[mri]\n    injected = attacker\n', encoding='utf-8')
        self.sources = [
            ('START-MRI-DRIVE-MAKER.cmd', 'START-MRI-DRIVE-MAKER.cmd'),
            ('SOURCE.txt', 'SOURCE.txt'),
        ]
        self.scenarios = [
            ('GIT_DIR', {'GIT_DIR': str(self.alternate / '.git')}),
            ('GIT_WORK_TREE', {'GIT_WORK_TREE': str(self.alternate)}),
            ('GIT_COMMON_DIR', {'GIT_COMMON_DIR': str(self.alternate / '.git')}),
            ('GIT_OBJECT_DIRECTORY', {'GIT_OBJECT_DIRECTORY': str(self.alternate / '.git' / 'objects')}),
            ('GIT_ALTERNATE_OBJECT_DIRECTORIES', {
                'GIT_ALTERNATE_OBJECT_DIRECTORIES': str(self.alternate / '.git' / 'objects')}),
            ('GIT_CONFIG_COUNT', {
                'GIT_CONFIG_COUNT': '1', 'GIT_CONFIG_KEY_0': 'mri.injected',
                'GIT_CONFIG_VALUE_0': 'attacker'}),
            ('GIT_CONFIG', {'GIT_CONFIG': str(self.config)}),
            ('GIT_CONFIG_GLOBAL', {'GIT_CONFIG_GLOBAL': str(self.config)}),
            ('GIT_CONFIG_SYSTEM', {'GIT_CONFIG_SYSTEM': str(self.config)}),
            ('GIT_CONFIG_PARAMETERS', {'GIT_CONFIG_PARAMETERS': "'mri.injected'='attacker'"}),
        ]

    def tearDown(self):
        self.temp.cleanup()

    def injected_environment(self, overrides):
        environment = clean_environment()
        environment.update(overrides)
        return environment

    def test_original_redirect_is_reproduced_and_every_override_is_removed(self):
        raw = run_git(self.intended, ['show', 'HEAD:SOURCE.txt'],
                      self.injected_environment({'GIT_DIR': str(self.alternate / '.git')}))
        self.assertEqual(raw, b'alternate source bytes\n')
        self.assertNotEqual(self.intended_commit, self.alternate_commit)

        for name, overrides in self.scenarios:
            with self.subTest(override=name):
                environment = self.injected_environment(overrides)
                if name == 'GIT_WORK_TREE':
                    raw_root = run_git(self.intended,
                                       ['rev-parse', '--path-format=absolute', '--show-toplevel'],
                                       environment).decode().strip()
                    self.assertEqual(Path(raw_root), self.alternate.resolve())
                elif name in ('GIT_COMMON_DIR', 'GIT_OBJECT_DIRECTORY'):
                    raw = subprocess.run(['git', 'show', 'HEAD:SOURCE.txt'], cwd=self.intended,
                                         env=environment, capture_output=True)
                    self.assertNotEqual(raw.returncode, 0)
                elif name == 'GIT_ALTERNATE_OBJECT_DIRECTORIES':
                    raw = run_git(self.intended, ['cat-file', '-e', self.alternate_commit], environment)
                    self.assertEqual(raw, b'')
                    with self.assertRaises(subprocess.CalledProcessError):
                        BUILDER._git(self.intended, ['cat-file', '-e', self.alternate_commit], environment)
                if name.startswith('GIT_CONFIG'):
                    raw_config = run_git(self.intended, ['config', '--get', 'mri.injected'], environment)
                    self.assertEqual(raw_config, b'attacker\n')
                    with self.assertRaises(subprocess.CalledProcessError):
                        BUILDER._git(self.intended, ['config', '--get', 'mri.injected'], environment)

                archive = self.root / f'{name}.zip'
                manifest = BUILDER.build(self.intended, archive, '0.3.0',
                                         source_files=self.sources, environment=environment)
                self.assertEqual(manifest['source_commit'], self.intended_commit)
                self.assertNotEqual(manifest['source_commit'], self.alternate_commit)
                with zipfile.ZipFile(archive) as package:
                    prefix = 'MRI-Drive-Maker-v0.3.0/'
                    self.assertEqual(package.read(prefix + 'SOURCE.txt'), b'intended source bytes\n')
                    self.assertEqual(package.read(prefix + 'START-MRI-DRIVE-MAKER.cmd'),
                                     b'@echo intended\r\n')
                self.assertEqual(BUILDER.verify(archive, self.intended, environment), manifest)

    def test_linked_worktree_build_and_verify_are_supported(self):
        linked = self.root / 'linked-worktree'
        run_git(self.intended, ['worktree', 'add', '--detach', str(linked), 'HEAD'])
        environment = clean_environment()
        archive = self.root / 'linked-worktree.zip'
        manifest = BUILDER.build(linked, archive, '0.3.0', source_files=self.sources,
                                 environment=environment)
        self.assertEqual(manifest['source_commit'], self.intended_commit)
        self.assertEqual(BUILDER.verify(archive, linked, environment), manifest)

    def test_python_cli_build_and_verify_survive_combined_git_context_injection(self):
        archive = self.root / 'cli-release.zip'
        combined = {
            'GIT_DIR': str(self.alternate / '.git'),
            'GIT_WORK_TREE': str(self.alternate),
            'GIT_COMMON_DIR': str(self.alternate / '.git'),
            'GIT_OBJECT_DIRECTORY': str(self.alternate / '.git' / 'objects'),
            'GIT_ALTERNATE_OBJECT_DIRECTORIES': str(self.alternate / '.git' / 'objects'),
            'GIT_CONFIG_COUNT': '1',
            'GIT_CONFIG_KEY_0': 'mri.injected',
            'GIT_CONFIG_VALUE_0': 'attacker',
            'GIT_CONFIG': str(self.config),
            'GIT_CONFIG_GLOBAL': str(self.config),
            'GIT_CONFIG_SYSTEM': str(self.config),
            'GIT_CONFIG_PARAMETERS': "'mri.injected'='attacker'",
        }
        environment = self.injected_environment(combined)
        built = subprocess.run([sys.executable, str(SCRIPT), '--output', str(archive)],
                               cwd=ROOT, env=environment, check=True, text=True,
                               capture_output=True)
        result = json.loads(built.stdout)
        expected_commit = run_git(ROOT, ['rev-parse', 'HEAD']).decode().strip()
        self.assertEqual(result['source_commit'], expected_commit)
        with zipfile.ZipFile(archive) as package:
            names = package.namelist()
            prefix = names[0].split('/')[0] + '/'
            manifest = json.loads(package.read(prefix + 'RELEASE.json'))
            self.assertEqual(manifest['source_commit'], expected_commit)
            self.assertEqual(package.read(prefix + 'START-MRI-DRIVE-MAKER.cmd'),
                             run_git(ROOT, ['show', 'HEAD:START-MRI-DRIVE-MAKER.cmd']))
        before = hashlib.sha256(archive.read_bytes()).hexdigest()
        verified = subprocess.run([sys.executable, str(SCRIPT), '--verify', str(archive)],
                                  cwd=ROOT, env=environment, check=True, text=True,
                                  capture_output=True)
        self.assertEqual(json.loads(verified.stdout)['source_commit'], expected_commit)
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), before)


if __name__ == '__main__':
    unittest.main()
