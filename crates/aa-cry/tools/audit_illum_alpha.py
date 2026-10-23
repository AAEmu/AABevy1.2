"""Inventory flagged Illum materials; verify mask-qualified candidates from the pak.

Uses the existing full-material TSV inventory, then extracts only candidate .mtl files
through aa-pak. No native client installation files are opened directly.
"""
import csv
import hashlib
import json
import xml.etree.ElementTree as ET
from illum_alpha_source import ROOT, OUT, source, extract

ORDINARY = 0x4|0x8|0x10|0x80|0x400|0x1000|0x2000|0x400000|0x1000000|0x800000000

def main():
    inventory = ROOT/'data/extract/aa-cry/mtl_all.tsv'
    rows = list(csv.DictReader(inventory.open(),delimiter='\t'))
    flagged = [r for r in rows if r['shader'].lower()=='illum' and int(r['genmask'],16)&0x2400]
    result, files = [],{}
    for r in flagged:
        mask = int(r['genmask'],16)
        entry = dict(file=r['file'],sub=int(r['sub']),mask=hex(mask),alpha_test=float(r['alpha_test']),opacity=float(r['opacity']))
        if mask&~ORDINARY:
            entry['declined'] = 'unaudited permutation bits '+hex(mask&~ORDINARY)
        else:
            try:
                root = ET.fromstring(source(r['file']))
                files[r['file']] = hashlib.sha256(extract(r['file']).read_bytes()).hexdigest()
                children = root.find('SubMaterials')
                m = children[int(r['sub'])] if children is not None else root
                tex = m.find('Textures')
                diffuse = next((t for t in tex if t.attrib.get('Map')=='Diffuse'),None) if tex is not None else None
                if diffuse is None: entry['declined'] = 'no diffuse texture'
                else:
                    mod = diffuse.find('TexMod')
                    unsupported = {k:v for k,v in (mod.attrib if mod is not None else {}).items()
                        if k not in ['TileU','TileV','OffsetU','OffsetV','RotateW'] or k=='RotateW' and float(v)!=0}
                    if unsupported: entry['declined'] = 'unaudited diffuse TexMod '+str(unsupported)
                    else: entry['admitted'] = True
            except (ET.ParseError, IndexError) as e: entry['declined'] = 'XML audit unresolved '+str(e)
        result.append(entry)
    summary = dict(total_flagged=len(result),
        vertex_alpha=sum(bool(int(r['mask'],16)&0x400) for r in result),
        alpha_glow=sum(bool(int(r['mask'],16)&0x2000) for r in result),
        admitted=sum(r.get('admitted',False) for r in result),
        admitted_alpha_test=sum(r.get('admitted',False) and r['alpha_test']>0 for r in result),
        declined=sum('declined' in r for r in result))
    OUT.mkdir(exist_ok=True)
    (OUT/'inventory.json').write_text(json.dumps(dict(summary=summary,
        full_material_inventory_sha256=hashlib.sha256(inventory.read_bytes()).hexdigest(),
        checked_material_sha256=files,materials=result),indent=2)+'\n')
    print(summary,flush=True)

if __name__ == '__main__': main()
