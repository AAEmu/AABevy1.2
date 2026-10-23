"""Execute selected *original 1.2 HLSL source bodies*, not a handwritten lighting oracle.

python crates/aa-cry/tools/illum_source_oracle.py [--check]
Extracts only through tools/aa-pak.exe. numpy supplies vector arithmetic. This is a
source oracle for the forward Illum pass, not execution of a compiled client shader.
The separate deferred/tiled paths are deliberately not presented as equivalent.
"""
from pathlib import Path
from types import SimpleNamespace as S
import hashlib
import json
import re
import subprocess
import sys
import numpy as np

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'data/extract/aa-cry/punctual'
BASE = 'game/shaders/hwscripts/cryfx/'
FLAGS = dict(X2_UNLIGHT=False, ANISO_SPECULAR=False, MUSCLE_BUMP_MAPPING=False,
             SPLIT_GLOSS_MAP=True, _RT_ACTOR_MODEL=False, X2_ACTOR_MODEL=False,
             _RT_HDR_MODE=True, HDR_MODIFIER=True, _RT_AMBIENT=False,
             _RT_FOG=True, _RT_PARTICLE=False)


def source(name):
    dest = OUT / BASE / name
    if not dest.exists():
        subprocess.run([str(ROOT / 'tools/aa-pak.exe'), 'x', BASE + name, str(OUT)], check=True)
    return dest.read_text()


def body(src, signature):
    start = src.index('{', src.index(signature)) + 1
    depth = 1
    for i in range(start, len(src)):
        depth += (src[i] == '{') - (src[i] == '}')
        if depth == 0:
            return src[start:i]
    raise ValueError(signature)


def condition(s):
    s = re.sub(r'%?(\w+)', lambda m: str(FLAGS.get(m[1], False)), s)
    return bool(eval(s.replace('&&', ' and ').replace('||', ' or ').replace('!', ' not ')))


def preprocess(src):
    stack, active, lines = [], True, []
    for line in src.splitlines():
        t = line.strip()
        if t.startswith('#if '):
            stack.append((active, condition(t[4:])))
            active = active and stack[-1][1]
        elif t.startswith('#ifdef '):
            stack.append((active, t[7:].strip() in FLAGS))
            active = active and stack[-1][1]
        elif t.startswith('#else'):
            active = stack[-1][0] and not stack[-1][1]
        elif t.startswith('#endif'):
            active = stack.pop()[0]
        elif active:
            lines.append(line)
    assert not stack
    return '\n'.join(lines)


def compile_body(name, args, src, scope, result=None):
    src = re.sub(r'//[^\n]*|/\*.*?\*/', '', preprocess(src), flags=re.S)
    src = re.sub(r'\b([0-9]+)\.([fh])\b', r'\1.0', src)
    src = re.sub(r'(?<=[0-9.])[fh]\b', '', src)
    src = re.sub(r'\.xyz\b', '', src)
    src = re.sub(r'\.([xyzw])\b', lambda m: '[' + str('xyzw'.index(m[1])) + ']', src)
    lines, depth = [f'def {name}({args}):'], 1
    for t in re.split(r'([{};])', src):
        t = t.strip()
        if t == '{':
            depth += 1
        elif t == '}':
            depth -= 1
        elif t and t != ';':
            t = re.sub(r'\b(?:float[234]?|half[234]?)\s+', '', t)
            t = t.replace('&&', ' and ').replace('||', ' or ')
            if t.startswith('if'):
                t = re.sub(r'^if\s*\((.*)\)$', r'if \1:', t, flags=re.S)
            elif t == 'else':
                t = 'else:'
            else:
                # HLSL permits scalar broadcast to a float3 on assignment/multiply.
                t = re.sub(r'^(.*?)\s*([+*])=\s*(.*)$', r'\1 = \1 \2 (\3)', t, flags=re.S)
            lines.append('    ' * depth + t)
    if result:
        lines.append('    return ' + result)
    code = '\n'.join(lines) + '\n'
    exec(code, scope)
    return code


def load_scope():
    shade, illum, frag = source('shadelib.cfi'), source('illum.cfx'), source('fraglib.cfi')
    scope = dict(np=np, dot=np.dot, saturate=lambda x: np.clip(x, 0, 1), pow=np.power,
                 normalize=lambda x: x / np.linalg.norm(x), exp2=np.exp2,
                 max=np.maximum, QUALITY_LOW=0, LT_DIRECTIONAL=0,
                 MatDifColor=np.full(3, .25), MatSpecColor=np.full(3, .7),
                 DiffuseModifierInHDR=1., SpecularModifierInHDR=1.)
    translated = compile_body('GetAttenuation', 'L, fInvRadius', body(shade, 'float GetAttenuation('), scope)
    translated += compile_body('Phong', 'R, L, Exp', body(shade, 'float Phong(float3 R,'), scope)
    translated += compile_body('per_light', 'pPass, pLight', body(illum, 'void frag_custom_per_light('), scope)
    translated += compile_body('composition', 'pPass, cFinal', body(illum, 'void frag_custom_end('), scope, 'cFinal')
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'executed_forward_source.py').write_text(translated)
    return scope, illum, shade, frag


def run():
    scope, illum, shade, frag = load_scope()
    rows = []
    # Aligned view/normal/light isolates falloff, occlusion floor, filter and material masks.
    for radius in [2., 5., 11.]:
        for fraction in [.1, .25, .5, .75, .99, 1., 1.2]:
            for shadow in [0., .35, 1.]:
                for filt in [0., .4, 1.]:
                    light = np.array([0., 0., radius * fraction])
                    attenuation = float(scope['GetAttenuation'](light, 1. / radius))
                    p = S(nQuality=1, pCustom=S(fOcclShadowAcc=0., cSunhalf=0.),
                          cDiffuseAcc=np.zeros(3), cSpecularAcc=np.zeros(3), cAmbientAcc=np.zeros(3),
                          cEnvironment=np.zeros(3), cGlossMap=np.ones(3), cDiffuseMap=np.ones(3),
                          vReflVec=np.array([0., 0., 1.]), fSpecPow=16., nReflectionMapping=0)
                    l = S(nType=1, fNdotL=1., fOcclShadow=shadow, cDiffuse=np.full(3, .35),
                          cSpecular=np.full(3, .35), fFallOff=attenuation,
                          cFilter=np.full(3, filt), vLight=np.array([0., 0., 1.]))
                    scope['per_light'](p, l)
                    final = scope['composition'](p, np.zeros(3))
                    rows.append(dict(radius=radius, fraction=fraction, shadow=shadow, filter=filt,
                                     attenuation=attenuation, diffuse=float(p.cDiffuseAcc[0]),
                                     highlight=float(p.cSpecularAcc[0]), composed=float(final[0])))
    # Original forward Phong is independent of N.L and deliberately has a 20% shadow floor.
    p.vReflVec = np.array([0., 0., 1.])
    l.fNdotL, l.fOcclShadow, l.fFallOff, l.cFilter = -.5, 0., 1., np.ones(3)
    p.cDiffuseAcc, p.cSpecularAcc = np.zeros(3), np.zeros(3)
    scope['per_light'](p, l)
    assert p.cDiffuseAcc[0] == 0 and abs(p.cSpecularAcc[0] - .07) < 1e-9
    # Native *deferred* paths are separate: record distinguishing expressions as evidence.
    deferred = source('deferredshading.cfx')
    assert 'fAttenuation *= fAttenuation;' in body(deferred, 'float GetDeferredLightAttenuation(')
    assert 'Blinn(vNormal, vView, vLight, vNormal.w)' in body(deferred, 'pixoutMRT LightPassPS(')
    assert 'saturate( 6.645 * RdotL + -5.645 )' in body(deferred, 'pixoutMRT LightingTiledPS(')
    assert 'pPass.bDeferredSpecularShading = true;' in illum
    assert 'pPass.cSpecularAcc = pPass.cSpecularAccRT;' in frag
    compile_body('DeferredPointAttenuation', 'L, fInvRadius', body(deferred, 'float GetDeferredLightAttenuation('), scope)
    FLAGS['_RT_SAMPLE0'] = True
    compile_body('DeferredProjectorAttenuation', 'L, fInvRadius', body(deferred, 'float GetDeferredLightAttenuation('), scope)
    FLAGS.pop('_RT_SAMPLE0')
    compile_body('Blinn', 'N, V, L, Gloss', body(shade, 'float Blinn('), scope)
    aligned = np.array([0., 0., 1.])
    distinctions = dict(forward_half=float(scope['GetAttenuation'](aligned, .5)),
                        deferred_point_half=float(scope['DeferredPointAttenuation'](aligned, .5)),
                        deferred_projector_half=float(scope['DeferredProjectorAttenuation'](aligned, .5)),
                        forward_aligned_highlight=float(scope['Phong'](aligned, aligned, 16.)),
                        deferred_aligned_highlight=float(scope['Blinn'](aligned, aligned, aligned, 16./255.)))
    assert distinctions['forward_half'] == .75 and distinctions['deferred_point_half'] == .5625
    assert distinctions['deferred_projector_half'] == .75 and distinctions['forward_aligned_highlight'] == 1.
    assert abs(distinctions['deferred_aligned_highlight'] - 1.) > .3
    mapping = []
    for radius in [2., 5., 11.]:
        for coefficient in [.16, 16/75, .25]:  # world point, world projector, fx
            intensity = coefficient * radius**2
            at_half = intensity * (15/16)**2 / (radius/2)**2
            amplitude = intensity / radius**2 * (75/16)
            assert abs(amplitude * .75 - at_half) < 1e-12
            mapping.append(dict(radius=radius, coefficient=coefficient, amplitude=amplitude, half=at_half))
    return dict(pass_scope='Illum forward frag_custom_per_light + frag_custom_end; not opaque deferred/tiled',
                flags=FLAGS, deferred_distinctions=distinctions,
                sha256={n: hashlib.sha256((OUT / BASE / n).read_bytes()).hexdigest()
                        for n in ['illum.cfx', 'shadelib.cfi', 'fraglib.cfi', 'deferredshading.cfx']},
                cases=rows, intensity_mapping=mapping)


if __name__ == '__main__':
    result = run()
    golden = Path(__file__).with_name('illum_source_golden.json')
    if '--check' in sys.argv:
        assert result == json.loads(golden.read_text()), 'original source/oracle changed'
        print(f"PASS: {len(result['cases'])} executed original forward source cases; 9 intensity mappings; deferred distinctions")
    else:
        golden.write_text(json.dumps(result, indent=2) + '\n')
        print(f"Recorded {len(result['cases'])} executed original forward source cases")
