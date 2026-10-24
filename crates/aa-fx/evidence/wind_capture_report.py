"""Retain and verify the owned paired aa-dev Wind capture runs."""
from pathlib import Path
import json,re,hashlib,shutil,statistics
from PIL import Image,ImageChops,ImageStat
ROOT=Path(__file__).resolve().parents[3];OUT=Path(__file__).resolve().parent
SCRATCH=ROOT/'data/extract/aa-fx/wind-implementation-20261023'
DEST=OUT/'wind-captures';DEST.mkdir(exist_ok=True)
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
rows=json.loads((SCRATCH/'runs.json').read_text(encoding='utf-8-sig'))
for run in rows:
 p=SCRATCH/(run['name']+'.log');b=p.read_bytes();s=b.decode('utf-16') if b.startswith(b'\xff\xfe') else b.decode('utf-8',errors='replace')
 records=[' '.join(r.split()) for r in re.split(r'(?=\d{4}-\d{2}-\d{2}T\d{2}:)',s)]
 run['fps']=[float(n) for n in re.findall(r'aa-dev: fps=([0-9.]+)',s)]
 run['capture_times']=[float(n) for n in re.findall(r'wind-capture .*?\+([0-9.]+)',s)]
 run['identity_records']=[r.split('wind-trial ',1)[1] for r in records if 'wind-trial ' in r and 'bits=' in r]
 run['costs']=[]
 for r in records:
  m=re.search(r'wind-trial \+([0-9.]+).*?particles=(\d+) sim_ms=([0-9.]+) upload_ms=([0-9.]+)',r)
  if m:run['costs'].append(dict(age=float(m[1]),particles=int(m[2]),sim_ms=float(m[3]),upload_ms=float(m[4])))
 run['warnings_or_errors']=[r for r in records if re.search(r'\b(WARN|ERROR)\b|panicked',r)]
 run['log_sha256']=sha(p)
 saved=DEST/p.name;saved.write_bytes(s.replace('\r\n','\n').encode('utf-8'));run['retained_log']=saved.relative_to(ROOT).as_posix();run['retained_log_sha256']=sha(saved)
 run['images']=[]
 for i in [0,1]:
  img=ROOT/f"shots/aa-fx/wind-20261023/{run['name']}_{i}.png";saved=DEST/img.name;shutil.copyfile(img,saved)
  run['images'].append(dict(path=img.relative_to(ROOT).as_posix(),retained_path=saved.relative_to(ROOT).as_posix(),sha256=sha(img)))
 assert run['exit_code']==0 and run['capture_times']==[.2,.8] and not run['warnings_or_errors'],run['name']
 active=[r for r in run['costs'] if r['age']<=.4]
 print(run['name'],'fps',run['fps'],'active median sim/upload',statistics.median(r['sim_ms'] for r in active),statistics.median(r['upload_ms'] for r in active))
comparisons=[]
for case in ['external','same','outside','no-force']:
 off=next(r for r in rows if r['name']==case+'-off');on=next(r for r in rows if r['name']==case+'-on')
 assert off['identity_records']==on['identity_records'],case
 for i in [0,1]:
  a=Image.open(DEST/f'{case}-off_{i}.png').convert('RGB');b=Image.open(DEST/f'{case}-on_{i}.png').convert('RGB');d=ImageChops.difference(a,b)
  c=dict(case=case,time=[.2,.8][i],pixel_identical=d.getbbox() is None,mean_absolute_rgb=ImageStat.Stat(d).mean,difference_bounds=d.getbbox())
  if case!='external':assert c['pixel_identical'],c
  if case=='external' and i==1:assert not c['pixel_identical'],c
  comparisons.append(c)
report=dict(boundary='Paired current implementation on/off, original-pak receiver params/textures. Same-owner is an explicit native-tree test composition. Fixed generic game Time 60Hz; real Time and wall FPS diagnostics unchanged. Shared machine, no matching live-client reference and no full native-output/town performance claim.',runs=rows,comparisons=comparisons,executable_sha256=sha(SCRATCH/'wind.exe'),production_frozen_sha256={p.relative_to(ROOT).as_posix():sha(p) for p in [ROOT/'crates/aa-fx/src/lib.rs',ROOT/'crates/aa-fx/src/params.rs',ROOT/'crates/aa-fx/src/sim.rs',ROOT/'crates/aa-fx/src/wind.rs']})
(OUT/'wind_capture_runs.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
print('verified all8 paired runs; retained16 PNGs/8 logs in owned crate')
