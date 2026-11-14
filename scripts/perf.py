"""Record reproducible model timings and installer sizes; desktop budgets stay explicit."""
import datetime
import json
import platform
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    report = {'at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'platform': platform.platform(), 'machine': platform.machine(),
              'rust': subprocess.check_output(['rustc', '--version'], text=True).strip(), 'measurements': {}}
    for package, example in [('forge-text', 'perf_text'), ('forge-core', 'perf_graph')]:
        result = subprocess.run(['cargo', 'run', '--locked', '--release', '-j', '1', '-p', package,
                                 '--example', example, '--quiet'], cwd=ROOT, check=True, capture_output=True, text=True)
        report['measurements'][package] = json.loads(result.stdout.strip().splitlines()[-1])
    report['installers'] = [{'file': p.name, 'bytes': p.stat().st_size, 'under_40_mb': p.stat().st_size <= 40 * 1024 * 1024}
                            for p in sorted((ROOT / 'releases').glob('*.exe')) if 'setup' in p.name.lower()]
    report['unmeasured_desktop_budgets'] = ['idle process memory', 'cold start to editable window',
        'keystroke to screen', 'scrolling FPS', 'graph layout plus GPU draw', 'Linux and macOS parity']
    output = ROOT / 'target/perf-report.json'
    output.parent.mkdir(exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + '\n')
    print(output.read_text())


if __name__ == '__main__':
    main()
