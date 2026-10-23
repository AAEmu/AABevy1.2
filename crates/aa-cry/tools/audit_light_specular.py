"""Small targeted original-world audit, read only through aa-pak; includes inactive rows.

python crates/aa-cry/tools/audit_light_specular.py [--check]
Cells: dock + neighbours, airport + neighbours, three known lamp cells, login stage.
This is a risk sample, not a census or a claim about currently visible lights.
"""
from collections import Counter
from pathlib import Path
import hashlib
import json
import re
import subprocess
import sys
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'data/extract/aa-cry/punctual'
CELLS = [('main_world', x, y) for x, y in [(18, 7), (18, 8), (19, 7), (10, 14), (9, 14), (10, 15), (9, 12), (21, 8), (14, 13)]] + [('login2', 1, 0)]


def extract(path):
    dest = OUT / path
    if not dest.exists():
        subprocess.run([str(ROOT / 'tools/aa-pak.exe'), 'x', path, str(OUT)], check=True)
    return dest


def run():
    lua_paths = ['game/scripts/entities/lights/light.lua', 'game/scripts/entities/lights/simplelight.lua']
    lua = [extract(p).read_text() for p in lua_paths]
    assert re.search(r'fSpecularMultiplier\s*=\s*1\s*,', lua[0])
    assert re.search(r'fDiffuseMultiplier\s*=\s*1\s*,', lua[0])
    assert re.search(r'fSpecularPercentage\s*=\s*100\s*,', lua[1])
    assert 'lt.specular_multiplier = specular_mul / diffuse_mul;' in lua[0]
    assert 'Color.fSpecularPercentage*0.01' in lua[1]
    assert re.search(r'bDeferredLight\s*=\s*1\s*,', lua[0])
    rows, hashes = [], {p: hashlib.sha256(extract(p).read_bytes()).hexdigest() for p in lua_paths}
    for world, x, y in CELLS:
        path = f'game/worlds/{world}/cells/{x:03}_{y:03}/client/entities.xml'
        dest = extract(path)
        hashes[path] = hashlib.sha256(dest.read_bytes()).hexdigest()
        tree = ET.fromstring(dest.read_text(encoding='utf-8-sig'))
        entities = tree.find('Objects').findall('Entity')
        nodes = {e.get('EntityGuid'): e for e in entities}

        def hidden(e, seen=()):
            if e.get('HiddenInGame') == '1':
                return True
            parent = e.get('ParentGUID')
            if parent is None:
                return False
            if parent in seen or parent not in nodes:
                return None
            return hidden(nodes[parent], (*seen, parent))

        for e in entities:
            cls = e.get('EntityClass')
            if cls not in ('Light', 'SimpleLight'):
                continue
            props = e.find('Properties')
            if props is None:
                continue
            color, options, projector, test = (props.find(k) for k in ('Color', 'Options', 'Projector', 'Test'))
            c = color.attrib if color is not None else {}
            o = options.attrib if options is not None else {}
            p = projector.attrib if projector is not None else {}
            t = test.attrib if test is not None else {}
            simple = cls == 'SimpleLight'
            diffuse = float(c.get('fColorMultiplier' if simple else 'fDiffuseMultiplier', 1))
            raw = float(c.get('fSpecularPercentage' if simple else 'fSpecularMultiplier', 100 if simple else 1))
            specular = raw * .01 if simple else raw / diffuse if diffuse != 0 else 1.
            h = hidden(e)
            eligible = props.get('bActive', '1') != '0' and h is False and float(props.get('Radius', 10)) > 0 and diffuse > 0 and t.get('bNegativeLight', '0') == '0' and o.get('bFakeLight', '0') == '0' and o.get('bDeferredClipBounds', '0') == '0'
            rows.append(dict(cell=f'{world}/{x:03}_{y:03}', name=e.get('Name'), guid=e.get('EntityGuid'),
                             cls=cls, diffuse_multiplier=diffuse, raw_specular=raw,
                             specular_multiplier=specular, specular_defaulted=('fSpecularPercentage' if simple else 'fSpecularMultiplier') not in c,
                             hidden=h, active=props.get('bActive', '1') != '0', eligible=eligible,
                             projector=p.get('texture_Texture', ''), deferred=o.get('bDeferredLight', 'default:1' if not simple else 'not in SimpleLight schema')))

    def summary(values):
        return dict(count=len(values), zero=sum(r['specular_multiplier'] == 0 for r in values),
                    one=sum(r['specular_multiplier'] == 1 for r in values),
                    non_one=sum(r['specular_multiplier'] != 1 for r in values),
                    defaulted=sum(r['specular_defaulted'] for r in values),
                    distribution=dict(sorted(Counter(f"{r['specular_multiplier']:.8g}" for r in values).items())))
    eligible = [r for r in rows if r['eligible']]
    return dict(scope='10 targeted XML cells; eligibility is static active/nonhidden/positive diffuse and supported flag approximation, no distance/budget/cookie availability test',
                native_defaults=dict(Light_specular=1, Light_diffuse=1, Light_deferred=1, SimpleLight_percent=100),
                sha256=hashes, total=summary(rows), eligible=summary(eligible),
                by_cell={f'{w}/{x:03}_{y:03}': summary([r for r in rows if r['cell'] == f'{w}/{x:03}_{y:03}']) for w, x, y in CELLS},
                rows=rows)


if __name__ == '__main__':
    result = run()
    golden = Path(__file__).with_name('punctual_evidence') / 'authored_light_audit.json'
    if '--check' in sys.argv:
        assert result == json.loads(golden.read_text()), 'authored light data/audit changed'
    else:
        golden.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({k: result[k] for k in ('native_defaults', 'total', 'eligible', 'by_cell')}, indent=2))
