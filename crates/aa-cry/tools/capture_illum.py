"""Matched GPU cases; run copied executables so cargo remains available to other workers."""
from pathlib import Path
import os
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).with_name('punctual_evidence')
CASES = {
    'point': ['--kind', 'point'],
    'point-highlight': ['--kind', 'point', '--term', 'highlight'],
    'point-diffuse': ['--kind', 'point', '--term', 'diffuse'],
    'point-near': ['--kind', 'point', '--distance', '.5'],
    'point-range3': ['--kind', 'point', '--range', '3'],
    'point-outside': ['--kind', 'point', '--distance', '6'],
    'point-shadow': ['--kind', 'point', '--shadows'],
    'point-shadow-highlight': ['--kind', 'point', '--shadows', '--term', 'highlight'],
    'spot': ['--kind', 'spot'],
    'spot-highlight': ['--kind', 'spot', '--term', 'highlight'],
    'spot-range3': ['--kind', 'spot', '--range', '3'],
    'spot-black': ['--kind', 'spot', '--cookie', 'black'],
    'spot-checker': ['--kind', 'spot', '--cookie', 'checker', '--term', 'diffuse'],
    'spot-away': ['--kind', 'spot', '--aim-x', '8'],
    'spot-shadow': ['--kind', 'spot', '--shadows'],
    'spot-shadow-highlight': ['--kind', 'spot', '--shadows', '--term', 'highlight'],
    'directional': ['--kind', 'directional'],
    'none': ['--kind', 'none'],
    'palace-point': ['--kind', 'point', '--material', 'palace', '--specular', '.5'],
    'palace-spot': ['--kind', 'spot', '--material', 'palace', '--specular', '.5'],
    'armour-point': ['--kind', 'point', '--material', 'armour', '--specular', '.5'],
    'armour-spot': ['--kind', 'spot', '--material', 'armour', '--specular', '.5'],
    'armour-point-highlight': ['--kind', 'point', '--material', 'armour', '--specular', '.5', '--term', 'highlight'],
    'palace-point-highlight': ['--kind', 'point', '--material', 'palace', '--specular', '.5', '--term', 'highlight'],
    'point-32': ['--kind', 'point', '--count', '32'],
    'spot-32': ['--kind', 'spot', '--count', '32'],
    'point-diffuse-spec0': ['--kind', 'point', '--term', 'diffuse', '--specular', '0'],
    'spot-diffuse': ['--kind', 'spot', '--term', 'diffuse'],
    'spot-diffuse-spec0': ['--kind', 'spot', '--term', 'diffuse', '--specular', '0'],
    'armour-reflection-spec0': ['--kind', 'point', '--material', 'armour', '--term', 'reflection', '--specular', '0'],
    'armour-reflection-spec1': ['--kind', 'point', '--material', 'armour', '--term', 'reflection', '--specular', '1'],
    'churn': ['--kind', 'none', '--term', 'highlight', '--churn'],
    'churn-final': ['--kind', 'none', '--term', 'highlight', '--churn-final'],
    'point-half-nonbindless': ['--kind', 'point', '--term', 'highlight', '--specular', '.5', '--no-bindless'],
    'point-half-no-buffer-arrays': ['--kind', 'point', '--term', 'highlight', '--specular', '.5', '--no-buffer-arrays'],
}
for kind in ['point', 'spot']:
    for name, value in [('0', '0'), ('002', '.02'), ('third', '.33333334'), ('half', '.5'), ('1', '1')]:
        CASES[f'{kind}-spec{name}'] = ['--kind', kind, '--term', 'highlight', '--specular', value]

if __name__ == '__main__':
    phase = sys.argv[1]
    assert phase in ('before', 'after')
    exe = ROOT / f'data/extract/aa-cry/punctual/illum-{phase}.exe'
    for case in sys.argv[2:] or CASES:
        args = CASES[case]
        log = OUT / f'{case}-{phase}.log'
        shot = OUT / f'{case}-{phase}.png'
        with log.open('w') as f:
            env = os.environ.copy()
            if case == 'churn': env['AA_ILLUM_LIGHT_TRACE'] = '1'
            result = subprocess.run([str(exe), *args, '--size', '1280x720', '--shot', str(shot),
                                     '--shot-after', '6', '--exit-after', '20'], cwd=ROOT,
                                    env=env, stdout=f, stderr=subprocess.STDOUT, timeout=45)
        assert result.returncode == 0 and shot.exists(), f'{case}: see {log}'
        text = log.read_text()
        assert 'ERROR' not in text and 'panicked' not in text, f'{case}: see {log}'
        print(f'{case}-{phase}: captured', flush=True)
