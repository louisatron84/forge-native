"""Validate real installed apply-on-exit updates on a disposable Windows runner.

Packages a console fixture using Forge's updater as ForgeUpgradeSmoke 1.0.0/1.0.1.
It does not install Forge, publish a feed, or create a desktop window.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def wait_for(predicate, message, process=None, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        if process is not None and process.poll() is not None:
            raise RuntimeError(f'{message}: fixture exited with {process.returncode}')
        time.sleep(0.05)
    raise RuntimeError(message)


def main():
    if (sys.platform != 'win32' or os.environ.get('GITHUB_ACTIONS') != 'true'
            or os.environ.get('RUNNER_ENVIRONMENT') != 'github-hosted'):
        raise RuntimeError('This fixture requires a disposable GitHub-hosted Windows runner')
    root = Path(os.environ['RUNNER_TEMP']).resolve() / 'forge-installed-update-smoke'
    root.mkdir()  # Never overwrite an earlier installation or test run.
    log_path = ROOT / 'target/installed-update-smoke.log'
    log_path.parent.mkdir(exist_ok=True)
    flags = subprocess.CREATE_NO_WINDOW
    with log_path.open('wb') as log:
        def run(*command):
            print(f'Installed-update smoke: {command[0]}', flush=True)
            subprocess.run([str(arg) for arg in command], cwd=ROOT, stdout=log,
                           stderr=subprocess.STDOUT, creationflags=flags, check=True, timeout=1800)

        run('cargo', 'build', '--locked', '--release', '--target', 'x86_64-pc-windows-msvc',
            '-p', 'forge-update', '--features', 'release-validation', '--example', 'installed_update_smoke', '-j', '1')
        binary = ROOT / 'target/x86_64-pc-windows-msvc/release/examples/installed_update_smoke.exe'
        feeds = []
        for version in ('1.0.0', '1.0.1'):
            stage = root / version
            payload = stage / 'payload'
            payload.mkdir(parents=True)
            shutil.copy2(binary, payload / binary.name)
            # Reuse the already verified release CRT payload; never copy system DLLs.
            with zipfile.ZipFile(ROOT / 'releases/Forge-win-x64-Portable.zip') as archive:
                for name in archive.namelist():
                    if name.startswith('current/') and name.count('/') == 1 and name.endswith('.dll'):
                        (payload / Path(name).name).write_bytes(archive.read(name))
            if not (payload / 'vcruntime140.dll').is_file():
                raise RuntimeError('Verified Windows package has no app-local runtime')
            (payload / 'smoke-version.txt').write_text(version)
            notes = stage / 'notes.md'
            notes.write_text(f'Installed update smoke fixture {version}\n')
            feed = stage / 'feed'
            run(os.environ.get('VPK', 'vpk'), '--skip-updates', '--yes', 'pack',
                '--packId', 'ForgeUpgradeSmoke', '--packTitle', 'Forge upgrade smoke fixture',
                '--packAuthors', 'Forge', '--packVersion', version, '--packDir', payload,
                '--mainExe', binary.name, '--runtime', 'win-x64', '--channel', 'win-x64',
                '--outputDir', feed, '--releaseNotes', notes)
            feeds.append(feed)

        install = root / 'installed'
        setups = list(feeds[0].glob('*Setup.exe'))
        if len(setups) != 1:
            raise RuntimeError('Expected exactly one fixture setup executable')
        run(setups[0], '--silent', '--installto', install)
        data = root / 'user-data'
        data.mkdir()
        (data / 'keep.txt').write_text('preserve user data')
        armed, release = root / 'armed', root / 'exit'
        installed_binary = install / 'current' / binary.name
        process = subprocess.Popen([str(installed_binary), 'apply', str(feeds[1]), str(data), str(armed), str(release)],
                                   stdout=log, stderr=subprocess.STDOUT, creationflags=flags)
        try:
            wait_for(armed.exists, 'Update did not become ready and arm apply-on-exit', process)
            # A scheduled updater must leave the installed payload alone while its owner lives.
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError('Fixture exited before the controller allowed closing')
                if (install / 'current/smoke-version.txt').read_text() != '1.0.0':
                    raise RuntimeError('Update replaced files before the owner exited')
                time.sleep(0.05)
            release.write_text('close accepted')
            if process.wait(timeout=30) != 0:
                raise RuntimeError('Installed fixture failed while closing')

            def updated():
                try:
                    return (install / 'current/smoke-version.txt').read_text() == '1.0.1'
                except OSError:
                    return False  # The updater swaps the current directory atomically.

            wait_for(updated, 'Installed payload did not update after exit')
            run(installed_binary, 'verify', feeds[1], data)
            if (data / 'keep.txt').read_text() != 'preserve user data':
                raise RuntimeError('Update changed user data')
            run(install / 'Update.exe', '--silent', 'uninstall')
            wait_for(lambda: not installed_binary.exists(), 'Fixture uninstall left the installed binary')
            if (data / 'keep.txt').read_text() != 'preserve user data':
                raise RuntimeError('Uninstall changed user data')
        finally:
            if process.poll() is None:
                process.kill()  # Only this fixture's process on the disposable runner.
                process.wait()
    report = {'package': 'ForgeUpgradeSmoke', 'from': '1.0.0', 'to': '1.0.1',
              'ready_notes_persisted': True, 'waited_for_exit': True,
              'installed_payload_replaced': True, 'new_installation_detected': True,
              'uninstall_preserved_data': True,
              'limitations': ['console fixture uses Forge updater code, not the GPUI application',
                              'desktop restart, unsaved-file prompts and Welcome rendering require separate validation']}
    output = ROOT / 'target/installed-update-smoke.json'
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(output.read_text())


if __name__ == '__main__':
    main()
