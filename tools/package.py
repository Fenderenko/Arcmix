#!/usr/bin/env python3
"""Packs a release: Arcmix-<version>.zip in the repository root with Arcmix.exe, README.txt,
LICENSE.txt, THIRD_PARTY.md and OFL-Onest.txt inside an Arcmix-<version> folder, and the bare
Arcmix-<version>.exe next to it (for the app's "Update").
Usage: python tools/package.py [path to the exe]   (default: rust/target/release/arcmix.exe)"""
import hashlib, os, re, sys, zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def version():
    toml = open(os.path.join(ROOT, 'rust', 'Cargo.toml'), encoding='utf-8').read()
    return re.search(r'^version = "([^"]+)"', toml, re.M).group(1)


def crlf(path):
    # text files in the zip open well in Notepad on any Windows
    return open(path, 'rb').read().replace(b'\r\n', b'\n').replace(b'\n', b'\r\n')


def main():
    ver = version()
    exe = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, 'rust', 'target', 'release', 'arcmix.exe')
    title = f'Arcmix {ver}'
    readme = open(os.path.join(ROOT, 'tools', 'release', 'README.txt'), encoding='utf-8').read()
    readme = readme.replace('{version}', ver).replace('{rule}', '=' * len(title))
    items = [
        ('Arcmix.exe', open(exe, 'rb').read()),
        ('README.txt', readme.replace('\r\n', '\n').replace('\n', '\r\n').encode('utf-8')),
        ('LICENSE.txt', crlf(os.path.join(ROOT, 'LICENSE'))),
        ('THIRD_PARTY.md', open(os.path.join(ROOT, 'THIRD_PARTY.md'), 'rb').read()),
        ('OFL-Onest.txt', crlf(os.path.join(ROOT, 'web', 'OFL-Onest.txt'))),
    ]
    out = os.path.join(ROOT, f'Arcmix-{ver}.zip')
    with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name, data in items:
            # a fixed date: the same files give the same zip
            z.writestr(zipfile.ZipInfo(f'Arcmix-{ver}/{name}', date_time=(2026, 1, 1, 12, 0, 0)), data, zipfile.ZIP_DEFLATED)
    data = open(out, 'rb').read()
    print(f'{out}: {len(data)} bytes, sha256 {hashlib.sha256(data).hexdigest()}')
    # the bare exe next to the zip: "Update" in the app downloads it (update::install)
    exe_out = os.path.join(ROOT, f'Arcmix-{ver}.exe')
    open(exe_out, 'wb').write(items[0][1])
    print(f'{exe_out}: {len(items[0][1])} bytes')


main()
