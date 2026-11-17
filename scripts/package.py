"""Build native website downloads. Run on the OS being packaged; never publishes."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    'x86_64-pc-windows-msvc': ('windows', 'x64'),
    'x86_64-unknown-linux-gnu': ('linux', 'x64'),
    'aarch64-apple-darwin': ('macos', 'arm64'),
    'x86_64-apple-darwin': ('macos', 'x64'),
}


def run(*args):
    subprocess.run([str(a) for a in args], cwd=ROOT, check=True)


def windows_runtime(payload):
    """Deploy the toolchain's release CRT locally; no system-wide prerequisite install.

    https://learn.microsoft.com/cpp/windows/choosing-a-deployment-method
    FORGE_VC_REDIST points to a Visual Studio Redist/MSVC/<version> directory.
    """
    configured = os.environ.get('FORGE_VC_REDIST') or os.environ.get('VCToolsRedistDir')
    if configured:
        redist = Path(configured)
    else:
        vswhere = Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)')) / 'Microsoft Visual Studio/Installer/vswhere.exe'
        if not vswhere.is_file():
            raise ValueError('Set FORGE_VC_REDIST to the Visual Studio release runtime directory')
        installation = subprocess.check_output([str(vswhere), '-latest', '-products', '*',
            '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
            '-property', 'installationPath'], text=True).strip()
        versions = list((Path(installation) / 'VC/Redist/MSVC').glob('[0-9]*')) if installation else []
        if not versions:
            raise ValueError('Visual Studio release runtime missing; set FORGE_VC_REDIST')
        redist = max(versions, key=lambda p: tuple(int(n) for n in p.name.split('.')))
    candidates = sorted(redist.glob('x64/Microsoft.VC*.CRT'))
    crt = next((p for p in reversed(candidates) if (p / 'vcruntime140.dll').is_file()), None)
    if crt is None:
        raise ValueError(f'Missing x64 release vcruntime140.dll in {redist}')
    for name in ('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'msvcp140_1.dll',
                 'msvcp140_2.dll', 'msvcp140_atomic_wait.dll', 'msvcp140_codecvt_ids.dll', 'concrt140.dll'):
        source = crt / name
        if source.is_file():
            shutil.copy2(source, payload / name)


def update_packages(payload, folder, version, system, arch, stage):
    """Keep feed/package names intact: the feed references those exact filenames."""
    output = stage / 'updates'
    output.mkdir()
    command = [os.environ.get('VPK', 'vpk'), '--skip-updates', '--yes', 'pack',
               '--packId', 'Forge', '--packTitle', 'Forge', '--packAuthors', 'Forge',
               '--packVersion', version, '--packDir', str(payload), '--mainExe',
               'forge.exe' if system == 'windows' else 'forge',
               '--runtime', ('win' if system == 'windows' else 'linux') + '-' + arch,
               '--channel', ('win' if system == 'windows' else 'linux') + '-' + arch,
               '--outputDir', str(output), '--releaseNotes', str(ROOT / 'packaging/RELEASE_NOTES.md')]
    if system != 'windows':
        # A small raster version of the repository's F mark, without build-time image libraries.
        def chunk(kind, data):
            return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
        rows = b''.join(b'\0' + b''.join(bytes((239, 155, 83, 255) if
            (35 <= x < 54 and 27 <= y < 104) or (35 <= x < 99 and 27 <= y < 44) or
            (35 <= x < 90 and 61 <= y < 78) else (32, 21, 16, 255)) for x in range(128)) for y in range(128))
        icon = stage / 'forge.png'
        icon.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 128, 128, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))
        command += ['--icon', str(icon), '--categories', 'Development;IDE;']
    run(*command)
    products = []
    for path in sorted(output.iterdir()):
        if path.suffix.lower() not in ('.exe', '.zip', '.appimage', '.nupkg', '.json'):
            continue
        destination = folder / path.name
        shutil.copy2(path, destination)
        kind = 'update-package' if path.suffix == '.nupkg' else 'update-feed' if path.suffix == '.json' else 'portable' if path.suffix == '.zip' else 'installer'
        products.append((destination, kind))
    if not any(kind == 'update-feed' for _, kind in products) or not any(kind == 'update-package' for _, kind in products):
        raise RuntimeError('Velopack did not generate an update feed and package')
    return products


def manifest(folder, version):
    artifacts = []
    for part in sorted(folder.glob('*.artifact.json')):
        data = json.loads(part.read_text())
        if data['version'] != version:
            raise ValueError(f'Mixed release versions: {part.name}')
        artifacts.extend(data['downloads'])
    for item in artifacts:
        if Path(item['file']).name != item['file'] or '/' in item['file'] or '\\' in item['file']:
            raise ValueError('Artifact filename must be a plain filename')
        path = folder / item['file']
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        if path.stat().st_size != item['bytes'] or digest != item['sha256']:
            raise ValueError(f'Artifact changed: {path.name}')
    if not artifacts:
        raise ValueError('No packaged downloads found')
    (folder / 'downloads.json').write_text(json.dumps({'version': version, 'downloads': artifacts}, indent=2) + '\n')
    (folder / 'SHA256SUMS').write_text(''.join(f"{a['sha256']}  {a['file']}\n" for a in artifacts))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=TARGETS)
    parser.add_argument('--version', default=tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version'])
    parser.add_argument('--binary', type=Path, help='Package an already built release binary')
    parser.add_argument('--output', type=Path, default=ROOT / 'releases')
    parser.add_argument('--merge', action='store_true', help='Merge artifact metadata from all native builders')
    parser.add_argument('--velopack', action='store_true', help='Produce update-enabled Windows installers or Linux AppImages and feeds')
    args = parser.parse_args()
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[A-Za-z0-9.-]+)?', args.version):
        parser.error('Use a semantic version without path characters')
    folder = args.output.resolve()
    folder.mkdir(parents=True, exist_ok=True)
    if args.merge:
        manifest(folder, args.version)
        return
    if not args.target:
        parser.error('--target is required')
    system, arch = TARGETS[args.target]
    if platform.system() != {'windows': 'Windows', 'linux': 'Linux', 'macos': 'Darwin'}[system]:
        parser.error('Package on a native builder for the selected operating system')
    binary = args.binary
    if not binary:
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        if args.version != version:
            parser.error('Set workspace.package.version to the release version before building')
        if system == 'macos':
            os.environ.setdefault('MACOSX_DEPLOYMENT_TARGET', '12.0')
        run('cargo', 'build', '--locked', '--release', '-p', 'forge-app', '--target', args.target, '-j', '1')
        binary = ROOT / 'target' / args.target / 'release' / ('forge.exe' if system == 'windows' else 'forge')
    binary = binary.resolve(strict=True)
    prefix = f'Forge-{args.version}-{system}-{arch}'
    products = []
    with tempfile.TemporaryDirectory(prefix='forge-package-') as temp:
        stage = Path(temp)
        payload = stage / 'payload'
        payload.mkdir()
        executable_name = 'forge.exe' if system == 'windows' else 'forge'
        shutil.copy2(binary, payload / executable_name)
        if system == 'windows':
            windows_runtime(payload)
        for name in ('LICENSE', 'THIRD_PARTY.md'):
            shutil.copy2(ROOT / name, payload / name)
        instructions = (ROOT / 'packaging/README.txt').read_text(encoding='utf-8')
        (payload / 'README.txt').write_text(instructions.replace('development build', f'{args.version} build'), encoding='utf-8')
        (payload / 'licenses').mkdir()
        for license in (ROOT / 'assets/fonts').glob('OFL-*.txt'):
            shutil.copy2(license, payload / 'licenses' / license.name)
        if args.velopack and system in ('windows', 'linux'):
            products.extend(update_packages(payload, folder, args.version, system, arch, stage))
        if system == 'windows' and not args.velopack:
            output = folder / f'{prefix}-setup.exe'
            compiler = shutil.which('makensis') or str(Path(os.environ.get('ProgramFiles(x86)', r'C:\Program Files (x86)')) / 'NSIS/makensis.exe')
            run(compiler, f'/DVERSION={args.version}', f'/DPAYLOAD={payload}', f'/DOUTPUT={output}', ROOT / 'packaging/windows.nsi')
            products.append((output, 'installer'))
            portable = folder / f'{prefix}-portable.zip'
            with zipfile.ZipFile(portable, 'w', zipfile.ZIP_DEFLATED) as archive:
                for path in sorted(payload.rglob('*')):
                    if path.is_file():
                        archive.write(path, Path('Forge') / path.relative_to(payload))
            products.append((portable, 'portable'))
        elif system == 'linux':
            linked = subprocess.run(['ldd', str(binary)], check=True, capture_output=True, text=True)
            if 'not found' in linked.stdout:
                raise RuntimeError(f'Missing Linux runtime libraries:\n{linked.stdout}')
            binary.chmod(0o755)
            (payload / 'forge').chmod(0o755)
            archive = folder / f'{prefix}.tar.gz'
            with tarfile.open(archive, 'w:gz') as tar:
                tar.add(payload, arcname='Forge')
            products.append((archive, 'portable'))
            deb = stage / 'deb'
            (deb / 'usr/bin').mkdir(parents=True)
            shutil.copy2(binary, deb / 'usr/bin/forge')
            for destination, source in [('usr/share/applications/forge.desktop', 'packaging/forge.desktop'), ('usr/share/icons/hicolor/scalable/apps/forge.svg', 'packaging/forge.svg')]:
                dest = deb / destination
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / source, dest)
            shutil.copytree(payload / 'licenses', deb / 'usr/share/doc/forge/licenses')
            shutil.copy2(ROOT / 'LICENSE', deb / 'usr/share/doc/forge/copyright')
            shutil.copy2(payload / 'README.txt', deb / 'usr/share/doc/forge/README')
            (deb / 'DEBIAN').mkdir()
            dependencies = 'libc6 (>= 2.35), libgcc-s1, libstdc++6, libfontconfig1, libfreetype6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libx11-6, libxcb1, libxcb-shape0, libxcb-xfixes0, libvulkan1'
            deb_version = args.version.replace('-', '~', 1)
            (deb / 'DEBIAN/control').write_text(f'Package: forge\nVersion: {deb_version}\nArchitecture: amd64\nMaintainer: Forge\nSection: devel\nPriority: optional\nReplaces: ideforfun\nBreaks: ideforfun\nDepends: {dependencies}\nDescription: Forge native Python IDE\n')
            output = folder / f'{prefix}.deb'
            run('dpkg-deb', '--root-owner-group', '--build', deb, output)
            run('dpkg-deb', '--info', output)
            products.append((output, 'installer'))
        elif system == 'macos':
            app = stage / 'image/Forge.app'
            executable = app / 'Contents/MacOS/forge'
            executable.parent.mkdir(parents=True)
            shutil.copy2(binary, executable)
            executable.chmod(0o755)
            resources = app / 'Contents/Resources'
            shutil.copytree(payload, resources)
            (resources / executable_name).unlink()
            with (app / 'Contents/Info.plist').open('wb') as stream:
                plistlib.dump({'CFBundleName': 'Forge', 'CFBundleDisplayName': 'Forge', 'CFBundleIdentifier': 'uk.forgecode.forge', 'CFBundleExecutable': 'forge', 'CFBundlePackageType': 'APPL', 'CFBundleShortVersionString': args.version.split('-')[0], 'CFBundleVersion': args.version.split('-')[0], 'LSMinimumSystemVersion': '12.0', 'NSHighResolutionCapable': True, 'NSPrincipalClass': 'NSApplication'}, stream)
            identity = os.environ.get('MACOS_SIGN_IDENTITY', '-')
            run('codesign', '--force', '--options', 'runtime', '--timestamp' if identity != '-' else '--timestamp=none', '--sign', identity, app)
            run('codesign', '--verify', '--strict', app)
            (stage / 'image/Applications').symlink_to('/Applications')
            output = folder / f'{prefix}.dmg'
            run('hdiutil', 'create', '-volname', 'Forge', '-srcfolder', stage / 'image', '-ov', '-format', 'UDZO', output)
            if os.environ.get('MACOS_NOTARY_PROFILE'):
                if identity == '-':
                    raise ValueError('Notarization requires a Developer ID signing identity')
                run('xcrun', 'notarytool', 'submit', output, '--keychain-profile', os.environ['MACOS_NOTARY_PROFILE'], '--wait')
                run('xcrun', 'stapler', 'staple', output)
            run('hdiutil', 'verify', output)
            products.append((output, 'installer'))
    downloads = []
    for path, kind in products:
        with path.open('rb') as stream:
            digest = hashlib.file_digest(stream, 'sha256').hexdigest()
        downloads.append({'os': system, 'arch': arch, 'kind': kind, 'file': path.name, 'sha256': digest, 'bytes': path.stat().st_size, 'signed': system == 'macos' and os.environ.get('MACOS_SIGN_IDENTITY', '-') != '-', 'notarized': system == 'macos' and bool(os.environ.get('MACOS_NOTARY_PROFILE'))})
    (folder / f'{prefix}.artifact.json').write_text(json.dumps({'version': args.version, 'downloads': downloads}, indent=2) + '\n')
    manifest(folder, args.version)


if __name__ == '__main__':
    main()
