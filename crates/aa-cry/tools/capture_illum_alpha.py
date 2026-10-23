"""Matched copied-executable captures; keep commands, hashes and aa-dev FPS."""
from pathlib import Path
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).with_name('illum_alpha_evidence')
CASES = {
    'bow-mesh': ['--full'],
    'bow-mesh-zero': ['--full', '--token-zero'],
    'bow-side': ['--full', '--yaw', '90'],
    'bow-side-zero': ['--full', '--yaw', '90', '--token-zero'],
    'bow-quad': ['--quad', '--unlit'],
    'bow-glow': ['--quad', '--full', '--glow-only'],
    'road-quad': ['--asset', 'road', '--quad', '--unlit'],
    'hammer-mesh': ['--asset', 'hammer', '--full'],
    'hammer-mesh-zero': ['--asset', 'hammer', '--full', '--token-zero'],
    'hammer-quad': ['--asset', 'hammer', '--quad', '--unlit'],
    'bow-ordinary': ['--quad', '--unlit', '--no-bindless'],
    'road-ordinary': ['--asset', 'road', '--quad', '--unlit', '--no-bindless'],
    'bow-no-colors': ['--quad', '--unlit', '--no-colors'],
    'bow-token-zero': ['--quad', '--unlit', '--token-zero'],
    'plant-quad': ['--asset', 'plant', '--quad', '--unlit'],
    'plant-quad-zero': ['--asset', 'plant', '--quad', '--unlit', '--token-zero'],
    'plant-mesh': ['--asset', 'plant', '--full'],
    'plant-mesh-zero': ['--asset', 'plant', '--full', '--token-zero'],
    'hammer-prepass': ['--asset', 'hammer', '--quad', '--unlit', '--prepass'],
    'hammer-prepass-ordinary': ['--asset', 'hammer', '--quad', '--unlit', '--prepass', '--no-bindless'],
    'hammer-zero': ['--asset', 'hammer', '--quad', '--unlit', '--token-zero'],
    'hammer-zero-prepass': ['--asset', 'hammer', '--quad', '--unlit', '--token-zero', '--prepass'],
    'hammer-dissolve': ['--asset', 'hammer', '--quad', '--unlit', '--dissolve'],
    'hammer-dissolve-prepass': ['--asset', 'hammer', '--quad', '--unlit', '--dissolve', '--prepass'],
    'hammer-shadows': ['--asset', 'hammer', '--quad', '--full', '--prepass', '--shadows'],
    'road-window': ['--asset', 'road', '--quad', '--unlit', '--uv-window', '.25'],
    'plant-window': ['--asset', 'plant', '--quad', '--unlit', '--uv-window', '.25'],
    'coral-window': ['--asset', 'coral', '--quad', '--unlit', '--uv-window', '.25'],
    'coral-prepass': ['--asset', 'coral', '--quad', '--unlit', '--uv-window', '.25', '--prepass'],
    'coral-prepass-ordinary': ['--asset', 'coral', '--quad', '--unlit', '--uv-window', '.25', '--prepass', '--no-bindless'],
}
CONTROLS = {
    'colors': ('illum_colors', []),
    'point': ('illum_lights', ['--kind', 'point', '--specular', '.5']),
    'waterfall': ('waterfall', ['--quad', '--unlit', '--time', '3600.25']),
}

def main():
    phase = sys.argv[1]
    assert phase in ('before', 'after')
    OUT.mkdir(exist_ok=True)
    manifest = OUT / 'captures.json'
    records = json.loads(manifest.read_text()) if manifest.exists() else {}
    for case in sys.argv[2:] or [*CASES, *CONTROLS]:
        name, args = CONTROLS.get(case, ('illum_alpha', CASES.get(case)))
        exe = ROOT / f'data/extract/aa-cry/illum-alpha/{name}-{phase}.exe'
        shot = OUT / f'{case}-{phase}.png'
        log = OUT / f'{case}-{phase}.log'
        command = [str(exe), *args, '--size', '960x720', '--shot', str(shot),
                   '--shot-after', '5', '--exit-after', '18']
        with log.open('w') as f:
            run = subprocess.run(command, cwd=ROOT, env=os.environ.copy(), stdout=f,
                                 stderr=subprocess.STDOUT, timeout=40)
        text = log.read_text()
        assert run.returncode == 0 and shot.exists() and not any(bad in text for bad in
            ['ERROR', 'panicked', 'error:', 'unresolved import', 'failed to process']), text
        records[f'{case}-{phase}'] = dict(command=command,
            exe_sha256=hashlib.sha256(exe.read_bytes()).hexdigest(),
            png_sha256=hashlib.sha256(shot.read_bytes()).hexdigest(),
            fps=[float(x) for x in re.findall(r'aa-dev: fps=([0-9.]+)',text)],
            fixture=[s for s in text.splitlines() if 'illum_alpha:' in s])
        manifest.write_text(json.dumps(records,indent=2)+'\n')
        print(f'{case}-{phase}: {records[f"{case}-{phase}"]["fps"]}',flush=True)

if __name__ == '__main__':
    main()
