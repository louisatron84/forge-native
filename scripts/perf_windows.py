"""Automated Windows process-memory probe with an isolated project and settings.

Window/document readiness is recorded separately from first-paint/input latency.
Only the process launched by this probe is queried and closed.
"""
import argparse
import ctypes
from ctypes import wintypes
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/x86_64-pc-windows-msvc/release/forge.exe')
    args = parser.parse_args()
    if sys.platform != 'win32':
        parser.error('This probe requires a Windows desktop')
    binary = args.binary.resolve(strict=True)
    user32 = ctypes.WinDLL('user32', use_last_error=True)
    kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    psapi = ctypes.WinDLL('psapi', use_last_error=True)
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    user32.EnableWindow.argtypes = [wintypes.HWND, wintypes.BOOL]
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]

    class Memory(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('PageFaultCount', wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in ('PeakWorkingSetSize', 'WorkingSetSize',
            'QuotaPeakPagedPoolUsage', 'QuotaPagedPoolUsage', 'QuotaPeakNonPagedPoolUsage',
            'QuotaNonPagedPoolUsage', 'PagefileUsage', 'PeakPagefileUsage', 'PrivateUsage')]

    psapi.GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(Memory), wintypes.DWORD]
    with tempfile.TemporaryDirectory(prefix='forge-perf-', dir=ROOT / 'target') as temp:
        project = Path(temp) / 'project'
        user = Path(temp) / 'data'
        project.mkdir()
        (user / 'workspaces').mkdir(parents=True)
        document = project / 'notes.txt'
        document.write_text('A small project for process-memory measurement.\n')
        (user / '.electron-imported').write_text('isolated benchmark\n')
        (user / 'settings.json').write_text(json.dumps({'checkForUpdates': False, 'showWelcome': False}))
        name = hashlib.sha1(str(project).replace('/', '\\').lower().encode()).hexdigest()[:24] + '.json'
        state = {'groups': [{'tabs': [{'path': str(document)}], 'active': str(document)}], 'activeGroup': 0, 'task': 'code', 'terminals': []}
        (user / 'workspaces' / name).write_text(json.dumps(state))
        env = dict(os.environ, FORGE_USER_DATA=str(user), FORGE_STARTUP_TIMINGS='1')
        started = time.perf_counter()
        with (Path(temp) / 'stderr.log').open('w') as errors:
            process = subprocess.Popen([str(binary), str(project)], env=env, stderr=errors, stdout=subprocess.DEVNULL)
            window = None
            try:
                @callback_type
                def visit(hwnd, _):
                    nonlocal window
                    pid = wintypes.DWORD()
                    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
                    if pid.value == process.pid and user32.IsWindowVisible(hwnd):
                        title = ctypes.create_unicode_buffer(1024)
                        user32.GetWindowTextW(hwnd, title, len(title))
                        if 'notes.txt' in title.value:
                            window = hwnd
                            return False
                    return True
                while window is None:
                    user32.EnumWindows(visit, 0)
                    if process.poll() is not None or time.perf_counter() - started > 20:
                        raise RuntimeError('Forge did not open the isolated document within 20 seconds')
                    time.sleep(0.01)
                document_ready = (time.perf_counter() - started) * 1000
                # Prevent accidental user edits in the measurement window.
                user32.EnableWindow(window, False)
                time.sleep(6)
                handle = kernel32.OpenProcess(0x0400 | 0x0010, False, process.pid)
                if not handle:
                    raise ctypes.WinError(ctypes.get_last_error())
                try:
                    memory = Memory()
                    memory.cb = ctypes.sizeof(memory)
                    if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(memory), memory.cb):
                        raise ctypes.WinError(ctypes.get_last_error())
                finally:
                    kernel32.CloseHandle(handle)
                report = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    'scenario': 'small project with one text document open, 6 seconds idle',
                    'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                    'document_window_ready_ms': document_ready,
                    'working_set_bytes': memory.WorkingSetSize, 'private_bytes': memory.PrivateUsage,
                    'peak_working_set_bytes': memory.PeakWorkingSetSize,
                    'under_150_mb_main_process': memory.WorkingSetSize <= 150 * 1024 * 1024,
                    'limitations': ['main process only', 'window/title readiness excludes verified first paint',
                                    'input-to-screen latency, scrolling FPS and GPU graph drawing are not measured']}
                stages = {}
                for line in (Path(temp) / 'stderr.log').read_text(errors='replace').splitlines():
                    parts = line.split()
                    if len(parts) == 3 and parts[0] == 'forge_startup':
                        stages[parts[1]] = float(parts[2])
                report['startup_stages_ms'] = stages
                output = ROOT / 'target/perf-windows-desktop.json'
                output.write_text(json.dumps(report, indent=2) + '\n')
                print(output.read_text())
            finally:
                if window:
                    user32.PostMessageW(window, 0x0010, 0, 0)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    # This process only has benchmark data and its window is disabled.
                    process.kill()
                    process.wait()


if __name__ == '__main__':
    main()
