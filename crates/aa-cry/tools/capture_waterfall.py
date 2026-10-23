"""Matched unattended captures, from copied executables so cargo stays available."""
from pathlib import Path
import argparse
import hashlib
import json
import re
import subprocess

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).with_name('waterfall_evidence')
BIN = ROOT / 'data/extract/aa-cry/waterfall'
CASES = {
    **{f'mesh-yaw-90-t{t}': ['--yaw', '-90', '--time', str(t)] for t in [0, 1, 2]},
    **{f'quad-t{t}': ['--quad', '--unlit', '--time', str(t)] for t in [0, 1, 2, 4]},
    'quad-token-zero': ['--quad', '--unlit', '--time', '2', '--token-zero'],
    'quad-t2-ordinary': ['--quad', '--unlit', '--time', '2', '--no-bindless'],
    **{f'second-t{t}': ['--quad', '--unlit', '--asset', 'second', '--time', str(t)]
       for t in [.25, 3599.75, 3600.25, 3601.25]},
    'second-t3600.25-ordinary': ['--quad', '--unlit', '--asset', 'second', '--time', '3600.25', '--no-bindless'],
    'ma-t2': ['--quad', '--unlit', '--asset', 'ma', '--time', '2'],
}


def capture(phase, case, executable='waterfall', args=None):
    args = CASES[case] if args is None else args
    exe = BIN / f'{executable}-{phase}.exe'
    shot, log = OUT / f'{case}-{phase}.png', OUT / f'{case}-{phase}.log'
    command = [str(exe), *args, '--size', '960x720', '--shot', str(shot),
               '--shot-after', '5', '--exit-after', '18']
    with log.open('w') as stream:
        result = subprocess.run(command, cwd=ROOT, stdout=stream, stderr=subprocess.STDOUT, timeout=35)
    text = log.read_text()
    assert result.returncode == 0 and shot.exists(), log
    assert 'ERROR' not in text and 'panicked' not in text, log
    record = dict(command=command, binary_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
                  screenshot_sha256=hashlib.sha256(shot.read_bytes()).hexdigest(),
                  fps=[float(n) for n in re.findall(r'aa-dev: fps=([\d.]+)', text)])
    manifest = OUT / 'captures.json'
    entries = json.loads(manifest.read_text()) if manifest.exists() else {}
    entries[f'{case}-{phase}'] = record
    manifest.write_text(json.dumps(entries, indent=2) + '\n')
    print(f'{case}-{phase}: {record["fps"]}', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=['before', 'after'])
    parser.add_argument('cases', nargs='*')
    args = parser.parse_args()
    for case in args.cases or CASES:
        if case == 'colors':
            capture(args.phase, case, 'colors', [])
        elif case.startswith('light-'):
            capture(args.phase, case, 'lights', ['--kind', case[6:], '--specular', '.5'])
        else:
            capture(args.phase, case)


if __name__ == '__main__':
    main()
