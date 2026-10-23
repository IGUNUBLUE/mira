#!/usr/bin/env python3
"""Build the bounded, platform-specific metadata authenticated by release signatures."""
import hashlib
import json
import pathlib
import re
import subprocess
import sys


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def manifest(archive, binary, tag, arch):
    if not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', tag):
        raise ValueError('Expected a stable version tag')
    if arch not in ('x86_64', 'aarch64'):
        raise ValueError('Unsupported architecture')
    version = tag[1:]
    if archive.name != f'mira-{tag}-{arch}-linux.tar.gz':
        raise ValueError('Unexpected archive name')
    if subprocess.check_output([str(binary.resolve()), '--version'], text=True).strip() != f'mira {version}':
        raise ValueError('Tag does not match the binary version')
    sizes = archive.stat().st_size, binary.stat().st_size
    if not 0 < sizes[0] <= 64 * 1024**2 or not 64 <= sizes[1] <= 128 * 1024**2:
        raise ValueError('Package exceeds the updater size limits')
    return dict(format=1, version=version, arch=arch, archive=archive.name,
                archive_sha256=digest(archive),
                binary_sha256=digest(binary),
                archive_size=sizes[0], binary_size=sizes[1])


if __name__ == '__main__':
    archive, binary = map(pathlib.Path, sys.argv[1:3])
    data = manifest(archive, binary, *sys.argv[3:5])
    archive.with_suffix('').with_suffix('.update.json').write_text(
        json.dumps(data, sort_keys=True, separators=(',', ':')) + '\n')
