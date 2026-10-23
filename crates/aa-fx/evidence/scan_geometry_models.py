"""Inspect CGF chunk/node metadata of authored GeometryInPieces models via aa-pak.

No geometry is extracted to disk. Mesh payload metadata is compiled-mesh v0x800.
"""
import hashlib
import json
import struct
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
inventory = json.loads((OUT / 'geometry_pieces_assets.json').read_text())
paths = {r['params'].get('Geometry', '').replace('\\', '/').lower()
         for r in inventory['rows'] if r['params'].get('GeometryInPieces') == 'true'
         and r['params'].get('Geometry')}
index = {line.split('\t')[0].lower() for line in (ROOT / 'data/pak12.tsv').read_text().splitlines()}
out = []
for path in sorted(paths):
    actual = path if path in index else 'game/' + path
    if actual not in index:
        out.append({'path': path, 'missing': True})
        continue
    d = subprocess.check_output([str(ROOT / 'tools/aa-pak.exe'), 'cat', actual], cwd=ROOT)
    ft, ver, tofs = struct.unpack_from('<III', d, 8)
    count = struct.unpack_from('<I', d, tofs)[0]
    fmt = '<IIII' + ('I' if ver == 0x745 else '')
    entries = [struct.unpack_from(fmt, d, tofs + 4 + i * struct.calcsize(fmt)) for i in range(count)]
    chunks = {e[3]: e for e in entries}
    nodes = []
    for t, v, o, cid, *_ in entries:
        if t != 0xcccc000b:
            continue
        o += 16
        name = d[o:o + 64].split(b'\0')[0].decode('utf8', 'replace')
        obj, parent, nchild, mat = struct.unpack_from('<iiii', d, o + 64)
        tm = struct.unpack_from('<16f', d, o + 84)
        mesh = chunks.get(obj)
        nverts = nidx = 0
        if mesh and mesh[0] == 0xcccc0000 and mesh[1] == 0x800:
            nverts, nidx = struct.unpack_from('<ii', d, mesh[2] + 16 + 8)
        nodes.append({'name': name, 'chunk': cid, 'object': obj, 'parent': parent, 'material': mat,
                      'tm': tm, 'object_type': hex(mesh[0]) if mesh else None,
                      'vertices': nverts, 'indices': nidx})
    plain = [n for n in nodes if n['indices'] and not n['name'].lower().startswith(
        ('$', 'proxy', 'physicsproxy', 'collision'))]
    out.append({'path': actual, 'bytes': len(d), 'sha256': hashlib.sha256(d).hexdigest(),
                'nonempty_plain_meshes': len(plain), 'nodes': nodes})
    if len(plain) > 1 or any(k in path for k in ('chain03', 'stone_solzreed', 'bonewall', 'golem_hand_piece', 'fx_stone')):
        print(actual, len(d), 'nodes', len(nodes), 'nonempty plain', len(plain),
              [(n['name'], n['vertices'], n['indices'], n['parent']) for n in plain][:12])
(OUT / 'geometry_pieces_models.json').write_text(json.dumps(out, indent=2) + '\n', encoding='utf8')
print('unique normalized geometry paths', len(paths), 'present', sum('nodes' in r for r in out),
      'missing', sum('missing' in r for r in out))
