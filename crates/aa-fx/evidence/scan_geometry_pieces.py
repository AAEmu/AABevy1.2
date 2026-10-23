"""Inventory original 1.2 GeometryInPieces through the prebuilt aa-pak only.

Read each particle XML in memory; retain hashes and exact authored parameter rows.
Run from any directory. No original client file is written or bulk-extracted.
"""
import collections
import hashlib
import json
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
PAK = ROOT / 'tools/aa-pak.exe'
libraries, rows = [], []


def walk(node, library, effect, ancestry):
    ancestry = ancestry + [node.get('Name', '')]
    params = node.find('Params')
    if params is not None and (params.get('Geometry') or params.get('GeometryInPieces')):
        rows.append({'library': library, 'effect': effect, 'emitter': '/'.join(ancestry[1:]),
                     'params': dict(params.attrib)})
    for child in node.findall('./Childs/Particles'):
        walk(child, library, effect, ancestry)


listing = subprocess.check_output([str(PAK), 'ls', 'libs/particles/'], cwd=ROOT, text=True)
for line in listing.splitlines():
    path = line.split('\t')[0]
    if not path.endswith('.xml'):
        continue
    data = subprocess.check_output([str(PAK), 'cat', path], cwd=ROOT)
    root = ET.fromstring(data.decode('utf-8-sig'))
    library = Path(path).stem
    before = len(rows)
    for effect in root.findall('Particles'):
        walk(effect, library, effect.get('Name', ''), [])
    libraries.append({'path': path, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
                      'geometry_emitters': len(rows) - before})

counts = collections.Counter(r['params'].get('GeometryInPieces', '<omitted>') for r in rows)
pieces = [r for r in rows if r['params'].get('GeometryInPieces', '').lower() in ('true', '1')]
summary = {'source': 'D:/AA1.2/game_pak via tools/aa-pak.exe', 'libraries': libraries,
           'geometry_or_piece_emitters': len(rows), 'piece_flags': counts,
           'piece_geometry_paths': dict(collections.Counter(r['params'].get('Geometry', '') for r in pieces)),
           'rows': pieces}
(OUT / 'geometry_pieces_assets.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
print(f'{len(libraries)} original libraries; {len(rows)} geometry/piece emitters; flags {dict(counts)}')
for r in pieces:
    p = r['params']
    print(r['library'], r['effect'], r['emitter'], p.get('Enabled', '<omitted>'),
          p.get('Geometry', '<none>'), p.get('Count', '<omitted>'), sep='\t')
