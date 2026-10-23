"""Rebuild the small QA ledger from retained local logs/captures. No game data writes."""
from pathlib import Path
import hashlib,json,re
ROOT=Path(__file__).resolve().parents[3]
LOGS=ROOT/'data/extract/aa-world/mesh-lod-native'
OUT=Path(__file__).with_name('lod_evidence')/'qa-summary.json'
def read(p):
    data=p.read_bytes()
    return data.decode('utf-16' if data.startswith(b'\xff\xfe') else 'utf-8',errors='replace')
def digest(p):
    return dict(sha256=hashlib.sha256(p.read_bytes()).hexdigest(),bytes=p.stat().st_size)
previous=json.loads(OUT.read_text()) if OUT.exists() else {}
tests=re.search(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',read(LOGS/'tests-final.log'))
assert tests
assert 'Finished `dev`' in read(LOGS/'check-final.err')
out=dict(date='2026-10-23',adapter='AMD Radeon RX 9070 XT / Vulkan / AMD 26.10.44',
         tests=dict(zip(('passed','failed','ignored'),map(int,tests.groups()))),cargo_check_exit=0,
         clippy='Run once; completed with warnings. New ECS-query complexity warnings addressed; no second clippy run.',
         timing='Distance state, not elapsed time; traced original caller uses no .3s/.05 settings.',
         scope='Ordinary brushes, default normal world view. Sprites unchanged from 2bf76a02; no aa-cry/aa-client edits.',
         cost_limit='Matched camera/geometry controls, not an isolated benchmark. Counts are visible mesh/material draw groups, not driver-wide draw calls. No town FPS inference.',
         binaries={},cases={},coverage=[],images={})
for name in ['lod-dissolve-run.exe','object-lod-run.exe']:
    p=LOGS/name
    out['binaries'][name]=digest(p) if p.exists() else previous['binaries'][name]
    out['binaries'][name]['cleanup']='Temporary copied executable removed after captures.'
for name in ['grid_final_before','grid_final_standard','grid_final_standard_prepass','grid_final_illum','grid_final_illum_prepass','rock_before','rock_after','world_final_lifecycle']:
    s=read(LOGS/(name+'.err'))
    assert not re.search(r' ERROR |panicked|validation error',s),name
    fps=[float(x) for x in re.findall(r'fps=([0-9.]+)',s)]
    out['cases'][name]=dict(fps_samples=fps,fps_range=[min(fps),max(fps)],
        draw_groups=sorted({int(x) for x in re.findall(r'mesh_material_draw_groups=(\d+)',s)}),
        warnings=len(re.findall(r' WARN ',s)),passed=[x for x in s.splitlines() if 'PASSED' in x])
out['coverage']=[json.loads(line) for line in read(LOGS/'gpu-final-pixels.jsonl').splitlines() if line.strip()]
assert len(out['coverage'])==5
assert all(x['holes']==x['duplicates']==x['mask_mismatches']==0 for x in out['coverage'])
out['grid']='Final secondary plane is 1cm nearer: wrong opaque prepass depth would occlude farther colour. Five PNGs, 69,632 interior pixels each.'
out['controls']=dict(rock_before='Same-binary hard-switch control: 1 group, 168 triangles.',
    rock_after='Same camera/light/material: held half pair, 2 groups, 488 triangles.',
    grid_before='17 visible groups, 34 drawn mesh triangles; 34 prepared parts.',
    grid_after='32 visible groups, 64 drawn mesh triangles; 34 prepared parts.',
    world='Original cell9,12 brush6: 2484/1004-triangle meshes, pair3488 triangles, one extra part; 47 tracked brushes, 3 rebuilds, stable model cache21.')
for p in sorted((ROOT/'shots/aa-world/mesh_lod').glob('*.png')):
    out['images'][p.relative_to(ROOT).as_posix()]=digest(p)
OUT.write_text(json.dumps(out,indent=2)+'\n',encoding='utf-8')
print(json.dumps({k:v['fps_range'] for k,v in out['cases'].items()},indent=2))
