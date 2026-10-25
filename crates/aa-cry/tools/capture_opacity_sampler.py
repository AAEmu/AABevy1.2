"""Short original-material sampler controls using a copied aa-dev executable."""
from pathlib import Path
import argparse,hashlib,json,os,re,subprocess
ROOT=Path(__file__).resolve().parents[3]
OUT=Path(__file__).with_name('opacity_sampler_evidence')
BIN=ROOT/'data/extract/aa-cry/opacity-sampler-20261025/waterfall.exe'
CASES={
    'shield': ['--asset','shield','--uv-window','1.5'],
    'shield-pattern': ['--asset','shield-pattern','--uv-window','1.5'],
    'glass': ['--asset','glass'],
    'glass-minified': ['--asset','glass','--uv-window','32'],
    'waterfall-control': [],
}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cases',nargs='*',default=list(CASES))
    opts=parser.parse_args();OUT.mkdir(exist_ok=True)
    manifest=OUT/'captures.json'
    records=json.loads(manifest.read_text()) if manifest.exists() else {}
    for case in opts.cases:
        variants=['authored','legacy','ordinary'] if case in ('shield','glass') else ['authored','legacy']
        for variant in variants:
            name=case+'-'+variant
            env=os.environ.copy();env.pop('AA_ILLUM_OPACITY_SAMPLER_LEGACY',None)
            if variant=='legacy':env['AA_ILLUM_OPACITY_SAMPLER_LEGACY']='1'
            shot=OUT/(name+'.png');log=OUT/(name+'.log')
            cmd=[str(BIN),'--quad','--unlit','--time','2',*CASES[case],
                *(['--no-bindless'] if variant=='ordinary' else []),'--size','960x720',
                '--shot',str(shot),'--shot-after','4','--exit-after','18']
            with log.open('w') as stream:
                result=subprocess.run(cmd,cwd=ROOT,env=env,stdout=stream,stderr=subprocess.STDOUT,timeout=35)
            text=log.read_text();assert result.returncode==0 and shot.exists(),str(log)
            assert 'ERROR' not in text and 'panicked' not in text,str(log)
            records[name]=dict(command=cmd,legacy=variant=='legacy',
                binary_sha256=hashlib.sha256(BIN.read_bytes()).hexdigest(),
                screenshot_sha256=hashlib.sha256(shot.read_bytes()).hexdigest(),
                fps=[float(x) for x in re.findall(r'aa-dev: fps=([\d.]+)',text)])
            manifest.write_text(json.dumps(records,indent=2)+'\n')
            print(name,records[name]['fps'],flush=True)
if __name__=='__main__':main()
