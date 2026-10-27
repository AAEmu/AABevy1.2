"""Inventory original 1.2 ForceGeneration through the prebuilt aa-pak only.

No library is extracted wholesale. Retain hashes, authored emitter rows and three
exact effect trees used as controls; run from any directory to refresh evidence.
"""
import collections
import csv
import hashlib
import json
import subprocess
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent
PAK = ROOT / 'tools/aa-pak.exe'
fields = ['ForceGeneration', 'Enabled', 'Count', 'Continuous', 'EmitterLifeTime',
          'SpawnDelay', 'PulsePeriod', 'ParticleLifeTime', 'PositionOffset',
          'SpawnIndirection', 'IgnoreAttractor', 'GeometryInPieces']
rows, libraries, examples = [], [], []
selected = {('abillity_skill_table_m', 'test.fire_flame_2'),
            ('abillity_skill_table_m', 'magic.vital_absorb_launch'),
            ('abillity_skill_table_m', 'death.spell_move_launch')}


def walk(node, library, effect, ancestry):
    name = node.get('Name', '')
    ancestry = ancestry + [name]
    params = node.find('Params')
    if params is not None and params.get('ForceGeneration') is not None:
        rows.append([library, effect, '/'.join(ancestry[1:])] + [params.get(k, '') for k in fields])
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
        name = effect.get('Name', '')
        walk(effect, library, name, [])
        if (library, name) in selected:
            examples.append({'pak_path': path, 'effect': name,
                             'original_xml': ET.tostring(effect, encoding='unicode')})
    libraries.append({'path': path, 'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest(),
                      'authored_force_emitters': len(rows) - before})
counts = collections.Counter(r[3] for r in rows)
summary = {'source': '<client>/game_pak via tools/aa-pak.exe', 'libraries': libraries,
           'authored_force_emitters': len(rows), 'force_modes': counts, 'examples': examples}
(OUT / 'force_generation_assets.json').write_text(json.dumps(summary, indent=2) + '\n', encoding='utf-8')
with (OUT / 'force_generation_assets.tsv').open('w', encoding='utf-8', newline='') as output:
    csv.writer(output, delimiter='\t', quoting=csv.QUOTE_ALL, lineterminator='\n').writerows(
        [['library', 'effect', 'emitter'] + fields, *rows])
print(f'{len(libraries)} original libraries; {len(rows)} authored force emitters; {dict(counts)}')
