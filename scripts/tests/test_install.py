"""Exercise the actual installer offline, with test-only signing keys and HTTPS fixtures."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class InstallerTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='mira-installer-test-')
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.prefix = self.base / 'installed prefix'
        self.fixtures = self.base / 'fixtures'
        self.fixtures.mkdir()
        self.key = self.base / 'private.pem'
        subprocess.run(['openssl', 'genpkey', '-algorithm', 'ED25519', '-out', self.key], check=True)
        public = subprocess.check_output(['openssl', 'pkey', '-in', self.key, '-pubout'], text=True)
        self.installer = self.base / 'install.sh'
        self.installer.write_text((ROOT / 'install.sh').read_text().replace(
            (ROOT / 'assets/update-public.pem').read_text().strip(), public.strip()))
        self.arch = {'aarch64': 'aarch64', 'arm64': 'aarch64'}.get(os.uname().machine, 'x86_64')
        self.name = f'mira-v0.11.0-{self.arch}-linux'
        self.binary = b'#!/bin/sh\necho mira 0.11.0\n'
        archive = self.fixtures / (self.name + '.tar.gz')
        with tarfile.open(archive, 'w:gz') as package:
            for filename, data in {'mira': self.binary, 'mira.desktop': b'[Desktop Entry]\nExec=mira\n',
                                   'mira.svg': b'<svg/>', 'LICENSE-MIT': b'test notice'}.items():
                info = tarfile.TarInfo(self.name + '/' + filename)
                info.size = len(data)
                info.mode = 0o755 if filename == 'mira' else 0o644
                package.addfile(info, io.BytesIO(data))
        self.manifest = dict(format=1, version='0.11.0', arch=self.arch, archive=archive.name,
                             archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),
                             binary_sha256=hashlib.sha256(self.binary).hexdigest(),
                             archive_size=archive.stat().st_size, binary_size=len(self.binary))
        self.sign()
        commands = self.base / 'commands'
        commands.mkdir()
        curl = commands / 'curl'
        curl.write_text('''#!/usr/bin/env python3
import os, pathlib, shutil, sys
args=sys.argv[1:]
url=next(a for a in args if a.startswith('https://'))
source=pathlib.Path(os.environ['MIRA_TEST_FIXTURES'])/url.rsplit('/',1)[1]
if not source.exists(): sys.exit(22)
if '-o' in args: shutil.copyfile(source,args[args.index('-o')+1])
else: sys.stdout.buffer.write(source.read_bytes())
''')
        curl.chmod(0o755)
        self.env = os.environ | {'PATH': str(commands) + ':' + os.environ['PATH'],
                                 'MIRA_PREFIX': str(self.prefix), 'MIRA_TEST_FIXTURES': str(self.fixtures)}

    def sign(self):
        meta = self.fixtures / (self.name + '.update.json')
        meta.write_text(json.dumps(self.manifest, sort_keys=True, separators=(',', ':')) + '\n')
        subprocess.run(['openssl', 'pkeyutl', '-sign', '-rawin', '-inkey', self.key,
                        '-in', meta, '-out', str(meta) + '.sig'], check=True)

    def install(self):
        return subprocess.run(['sh', self.installer, '--version', 'v0.11.0'], env=self.env,
                              text=True, capture_output=True, timeout=20)

    def test_installs_verified_package_and_records_only_owned_prefix(self):
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.prefix / 'bin/mira').read_bytes(), self.binary)
        marker = self.prefix / 'share/mira/install.json'
        self.assertEqual(json.loads(marker.read_text())['binary_sha256'], self.manifest['binary_sha256'])
        self.assertEqual(marker.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.prefix / 'share/mira').stat().st_mode & 0o777, 0o700)
        self.assertIn(f'Exec={self.prefix}/bin/mira', (self.prefix / 'share/applications/mira.desktop').read_text())

    def test_rejects_tampered_signature_archive_and_signed_identity_before_replacing_binary(self):
        self.assertEqual(self.install().returncode, 0)
        installed = self.prefix / 'bin/mira'
        for field, wrong in [('version', '0.11.1'), ('arch', 'other'), ('binary_sha256', 'a' * 64),
                             ('archive_sha256', 'b' * 64)]:
            previous = self.manifest[field]
            self.manifest[field] = wrong
            self.sign()
            result = self.install()
            self.assertNotEqual(result.returncode, 0, field)
            self.assertEqual(installed.read_bytes(), self.binary)
            self.manifest[field] = previous
        self.sign()
        signature = self.fixtures / (self.name + '.update.json.sig')
        signature.write_bytes(b'0' * 64)
        self.assertNotEqual(self.install().returncode, 0)
        self.assertEqual(installed.read_bytes(), self.binary)
        self.sign()
        archive = self.fixtures / (self.name + '.tar.gz')
        archive.write_bytes(archive.read_bytes() + b'changed')
        self.assertNotEqual(self.install().returncode, 0)
        self.assertEqual(installed.read_bytes(), self.binary)

    def test_missing_signature_cannot_fall_back_to_unsigned_installation(self):
        (self.fixtures / (self.name + '.update.json.sig')).unlink()
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('missing update signature', result.stderr)
        self.assertFalse((self.prefix / 'bin/mira').exists())


class MetadataTest(unittest.TestCase):
    def test_refuses_version_and_architecture_mismatch(self):
        spec = importlib.util.spec_from_file_location('metadata', ROOT / 'scripts/update-manifest.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaises(ValueError):
            module.manifest(Path('invalid'), Path('invalid'), 'v01.2.3', 'x86_64')
        with self.assertRaises(ValueError):
            module.manifest(Path('invalid'), Path('invalid'), 'v1.2.3', 'other')


if __name__ == '__main__':
    unittest.main()
