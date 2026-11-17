"""Run the workspace suite with aliased Windows temp paths to catch identity bugs.

Uses real 8.3 names when enabled on the volume, otherwise a parent-path alias.
The ordinary test output is retained on stdout, including process-abort details.
"""
import argparse
import ctypes
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--offline', action='store_true')
    args = parser.parse_args()
    if sys.platform != 'win32':
        parser.error('This regression runner requires Windows')
    (ROOT / 'target').mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='forge-path-alias-', dir=ROOT / 'target',
                                     ignore_cleanup_errors=True) as temp:
        assert Path(temp).resolve().parent == (ROOT / 'target').resolve()
        api = ctypes.WinDLL('kernel32', use_last_error=True).GetShortPathNameW
        api.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_uint]
        api.restype = ctypes.c_uint
        buffer = ctypes.create_unicode_buffer(32768)
        length = api(temp, buffer, len(buffer))
        if not length or length >= len(buffer):
            raise ctypes.WinError(ctypes.get_last_error())
        alias = buffer.value
        if alias.lower() == temp.lower():
            (Path(temp) / 'nested').mkdir()
            alias = str(Path(temp) / 'nested' / '..')
        print(f'Workspace tests use TEMP/TMP={alias}', flush=True)
        command = ['cargo', 'test', '--workspace', '--locked', '-j', '1']
        if args.offline:
            command.append('--offline')
        command += ['--', '--nocapture']
        result = subprocess.run(command, cwd=ROOT, env=dict(os.environ, TEMP=alias, TMP=alias))
        return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
