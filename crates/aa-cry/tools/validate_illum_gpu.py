"""Compare rendered fixture pixels to executed original forward HLSL, and matched controls.

Run capture_illum.py first. Requires Pillow/numpy, no additional Rust dependencies.
"""
from pathlib import Path
from types import SimpleNamespace as S
import json
import math
import re
import numpy as np
from PIL import Image
from capture_illum import CASES, OUT
from illum_source_oracle import load_scope

scope, _, _, _ = load_scope()


def image(case, phase='after'):
    return np.asarray(Image.open(OUT / f'{case}-{phase}.png').convert('RGB'))


def number(args, key, default):
    return float(args[args.index(key) + 1]) if key in args else default


def srgb(x):
    x = np.clip(x, 0, 1)
    return np.where(x <= .0031308, x * 12.92, 1.055 * x**(1/2.4) - .055) * 255


def expected(case, x, y):
    args = CASES[case]
    radius, distance = number(args, '--range', 5), number(args, '--distance', 2)
    delta = np.array([number(args, '--light-x', .7) - x, .5 - y, distance])
    l = delta / np.linalg.norm(delta)
    attenuation = scope['GetAttenuation'](delta, 1 / radius)
    filt = 1.
    if args[args.index('--kind') + 1] == 'spot':
        axis = np.array([number(args, '--aim-x', 0) - number(args, '--light-x', .7), -.5, -distance])
        axis /= np.linalg.norm(axis)
        cone = np.clip((np.dot(axis, -l) - math.cos(.65)) / (math.cos(.3) - math.cos(.65)), 0, 1)
        filt = cone * cone  # Bevy cone bridge, not a recovered native projector function.
    p = S(nQuality=1, pCustom=S(fOcclShadowAcc=0., cSunhalf=0.),
          cDiffuseAcc=np.zeros(3), cSpecularAcc=np.zeros(3), cAmbientAcc=np.zeros(3),
          cEnvironment=np.zeros(3), cGlossMap=np.ones(3), cDiffuseMap=np.ones(3),
          vReflVec=np.array([0., 0., 1.]), fSpecPow=16., nReflectionMapping=0)
    # Camera default exposure; the fixture uses LIGHT_UNIT_LUX=pi*998.
    amplitude = .35 * 998 / (1.2 * 2**9.7)
    light = S(nType=1, fNdotL=l[2], fOcclShadow=1., cDiffuse=np.full(3, amplitude),
              cSpecular=np.full(3, amplitude * number(args, '--specular', 1)), fFallOff=attenuation, cFilter=np.full(3, filt), vLight=l)
    scope['per_light'](p, light)
    if '--term' in args and args[args.index('--term') + 1] == 'highlight':
        linear = p.cSpecularAcc * .7
    elif '--term' in args and args[args.index('--term') + 1] == 'diffuse':
        linear = p.cDiffuseAcc * .25
    else:
        linear = scope['composition'](p, np.zeros(3))
    return srgb(linear)


def run():
    results = {}
    cases = ['point', 'point-highlight', 'point-diffuse', 'point-near', 'point-range3', 'point-outside', 'spot', 'spot-highlight', 'spot-range3', 'spot-away']
    cases += [f'{k}-spec{v}' for k in ['point', 'spot'] for v in ['0', '002', 'third', 'half', '1']]
    for case in cases:
        pixels = image(case)
        h, w, _ = pixels.shape
        errors = []
        for py in range(160, h - 160, 31):
            for px in range(210, w - 210, 37):
                x = (px + .5 - w/2) * 4.5/h
                y = (h/2 - py - .5) * 4.5/h
                value = expected(case, x, y)
                errors.append(float(np.max(np.abs(value - pixels[py, px]))))
        results[case] = dict(samples=len(errors), max_srgb_error=max(errors), mean_srgb_error=float(np.mean(errors)))
        assert max(errors) < 1.1, (case, results[case])
    for case in ['none', 'directional']:
        count = int(np.count_nonzero(np.any(image(case) != image(case, 'before'), axis=2)))
        assert count == 0, (case, count)
        results[case] = dict(changed_pixels=count)
    assert image('point-highlight', 'before').max() == 0
    assert image('point-highlight').max() > 100
    for case in ['spot-black', 'point-outside']:
        assert image(case).max() == 0, (case, int(image(case).max()))
    assert np.count_nonzero(image('spot-checker')) > 100
    assert np.count_nonzero(image('spot-checker')) < np.count_nonzero(image('spot'))
    for kind in ['point', 'spot']:
        assert np.array_equal(image(f'{kind}-spec1'), image(f'{kind}-highlight')), 'missing must mean 1'
        assert np.array_equal(image(f'{kind}-diffuse-spec0'), image(f'{kind}-diffuse')), 'specular cannot change diffuse'
    assert np.array_equal(image('armour-reflection-spec0'), image('armour-reflection-spec1')), 'specular cannot change reflection'
    assert np.array_equal(image('churn'), image('churn-final')), 'despawn/reindex/component removal must reach static final state'
    assert np.array_equal(image('point-half-nonbindless'), image('point-spechalf')), 'non-bindless table must agree'
    assert np.array_equal(image('point-half-no-buffer-arrays'), image('point-spechalf')), 'texture arrays alone must agree'
    churn_log = (OUT / 'churn-after.log').read_text()
    import ast
    states = [ast.literal_eval(x) for x in re.findall(r'illum_light_indices: \(main entity bits, GPU index, specular\) (\[.*\])', churn_log)]
    values = [sorted(v for _, _, v in rows) for rows in states]
    for required in [[0., 1./3., .5], [0., .5, 1.], [.02, 1./3., .5]]:
        assert any(np.allclose(v, required, atol=1e-6) for v in values if len(v) == 3), (required, values)
    assert 'material_bindless=false' in (OUT / 'point-half-nonbindless-after.log').read_text()
    assert 'material_bindless=true' in (OUT / 'point-half-no-buffer-arrays-after.log').read_text()
    assert 'material_bindless=true' in churn_log
    results['bridge_controls'] = dict(missing_equals_one=True, diffuse_unchanged=True, reflection_unchanged=True,
                                     churn_matches_static=True, ordinary_bindings_match=True,
                                     texture_arrays_without_buffer_arrays_match=True, gpu_index_trace_states=values)
    for case in ['palace-point', 'palace-spot', 'armour-point', 'armour-spot']:
        before, after = image(case, 'before'), image(case)
        count = int(np.count_nonzero(np.any(before != after, axis=2)))
        assert count > 100, (case, count)
        results[case] = dict(changed_pixels=count)
    for case in ['point-shadow', 'spot-shadow']:
        pixels = image(case)
        assert pixels.max() > 0
        # Independently compare shadow-enabled output with the same light without the occluder.
        unshadowed = image(case.replace('-shadow', ''))
        assert np.count_nonzero(pixels < unshadowed) > 100, case
    for kind in ['point', 'spot']:
        def linear(values):
            v = values.astype(float) / 255
            return np.where(v <= .04045, v/12.92, ((v+.055)/1.055)**2.4)
        base = linear(image(f'{kind}-highlight'))[:, :, 0]
        shadowed = linear(image(f'{kind}-shadow-highlight'))[:, :, 0]
        mask = base > .02
        ratio = shadowed[mask] / base[mask]
        # Fully occluded specular retains the original 20% floor, not zero or 4%.
        count = int(np.count_nonzero((ratio > .18) & (ratio < .22)))
        assert count > 100, (kind, count)
        results[f'{kind}-shadow-highlight'] = dict(shadow_floor_pixels=count)
    fps = {}
    for log in OUT.glob('*.log'):
        raw = log.read_bytes()
        text = raw.decode('utf-16') if raw.startswith(b'\xff\xfe') else raw.decode('utf-8', errors='replace')
        values = [float(x) for x in re.findall(r'aa-dev:\s+fps=([0-9.]+)', text)]
        if values:
            fps[log.stem] = [min(values), max(values)]
    results['fps'] = fps
    (OUT / 'validation.json').write_text(json.dumps(results, indent=2) + '\n')
    print(json.dumps(results, indent=2))


if __name__ == '__main__':
    run()
