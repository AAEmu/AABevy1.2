"""Write bounded graph/source/provenance evidence under this ignored directory."""
from pathlib import Path
import json, xml.etree.ElementTree as ET
from probe import OUT, ROOT, SOURCES, sha, read, native, write

def main():
    path='animations/graphs/player_full_body.xml'; raw=read(path); graph=ET.fromstring(raw)
    names={'fist_ac_sit_down_start','fist_ac_sit_down_loop','fist_ac_sit_up','music_co_sk_sit_down_piano_start','music_co_sk_sit_down_piano_cast','music_co_sk_sit_down_drum_start','music_co_sk_sit_down_drum_cast'}
    text=[f'Original pak {path} SHA256 {sha(raw)} (selected states/links only)']
    for e in graph.iter():
        if e.tag=='State' and e.get('id') in names: text.append(ET.tostring(e,encoding='unicode'))
        elif e.tag=='Link' and (e.get('from') in names or e.get('to') in names): text.append(ET.tostring(e,encoding='unicode'))
    graph_sources=[{'pak_path':path,'sha256':sha(raw),'bytes':len(raw)}]
    for name in ['action_start_no_ik','action_loop_no_ik','action_end_no_ik','action_loop']:
        path=f'libs/animationgraphtemplates/{name}.xml'; raw=read(path)
        text.append(f'Original pak {path} SHA256 {sha(raw)}\n'+raw.decode('utf-8'))
        graph_sources.append({'pak_path':path,'sha256':sha(raw),'bytes':len(raw)})
    (OUT/'graph-source.txt').write_text('\n\n'.join(text)+'\n',encoding='utf-8')
    native('action',['#30778998:7','305f0d00:305f0e80','305f09c0:305f0a2d','30617085:3061713c','3066bf93:3066c03a','--out','action-source.txt'])
    native('anim',['#316779dc:6','31551610:31551674','31554630:31554640','3154b648:3154b65d','3154bc7a:3154bc99','3154d2ec:3154d32e','315c4bc3:315c4c64','--out','anim-source.txt'])
    native('game',['39402810:3940285b','39405f73:39405fd7','--out','game-source.txt'])
    code=[('crates/aa-client/src/ingame/anim.rs',[(30,71)]),('crates/aa-anim/src/pose.rs',[(130,205)]),('crates/aa-anim/src/animator.rs',[(30,65),(194,215),(246,298)]),('crates/aa-anim/src/cal.rs',[(47,55),(99,147)]),('crates/aa-anim/src/caf.rs',[(1,53),(95,146),(149,239)])]
    excerpts=[]; source_records=[]
    for rel,ranges in code:
        raw=(ROOT/rel).read_bytes(); lines=raw.decode('utf-8').splitlines()
        source_records.append({'path':rel,'sha256':sha(raw),'line_ranges':ranges})
        excerpts.append(f'CURRENT WORKSPACE SOURCE {rel} SHA256 {sha(raw)}')
        for a,b in ranges:
            excerpts.extend(f'{i}: {lines[i-1]}' for i in range(a,min(b,len(lines))+1))
    (OUT/'sampler-source.txt').write_text('\n'.join(excerpts)+'\n',encoding='utf-8')
    prior=[]
    for name in ['report.txt','graph-source.txt','lifecycle-report.txt','lifecycle-cases.json','assets.json']:
        p=ROOT/'data/extract/aa-client/seat-presentation'/name
        b=p.read_bytes(); prior.append({'path':str(p),'sha256':sha(b),'bytes':len(b)})
    binaries=[{'role':role,'path':str(p),'sha256':sha(p.read_bytes())} for role,p in SOURCES.items()]
    write('provenance.json',{'scope':'No tracked edits/builds/Cargo/GPU/device/server/live-process work or subagents. Only ignored seat-root-motion evidence written. Original pak assets read only through prebuilt aa-pak.exe cat. Retained PE files read as bytes; original instructions executed only in Unicorn.','native_binaries':binaries,'pak_reader':{'path':str(ROOT/'tools/aa-pak.exe'),'sha256':sha((ROOT/'tools/aa-pak.exe').read_bytes())},'graph_sources':graph_sources,'workspace_sources':source_records,'prior_seating_evidence':prior,'supporting_later_source_not_used_as_native_proof':'Retained 2001-2019 Crytek SkeletonPose_Process.cpp/SkeletonAnim.cpp inspected for orientation only. Native 1.2 binary slices and asset data are the proof; later CryEngine source is not asserted to match 1.2.','reproduce':['python -B data/extract/aa-anim/seat-root-motion/probe.py assets','python -B data/extract/aa-anim/seat-root-motion/measure.py','python -B data/extract/aa-anim/seat-root-motion/native_cases.py','python -B data/extract/aa-anim/seat-root-motion/finalize.py']})
    prov=json.loads((OUT/'provenance.json').read_text(encoding='utf-8'))
    prov['artifacts']=[{'file':p.name,'bytes':p.stat().st_size,'sha256':sha(p.read_bytes())} for p in sorted(OUT.iterdir()) if p.is_file() and p.name!='provenance.json']
    write('provenance.json',prov)

if __name__=='__main__': main()
