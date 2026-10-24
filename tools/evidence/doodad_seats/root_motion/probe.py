"""Read-only seat audit. Writes only this ignored directory. No build/live DLL load.

Original assets are read ONLY through tools/aa-pak.exe cat; retained PE images
are mapped as bytes for static disassembly or isolated Unicorn instruction probes.
"""
from pathlib import Path
import sys, struct, hashlib, json, subprocess, re, math, zlib
import pefile
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

OUT = Path(__file__).resolve().parent
ROOT = OUT.parents[3]
SOURCES = {
    'action': ROOT/'data/extract/aa-world/environment/source/bin32/cryaction.dll',
    'anim': ROOT/'data/extract/equip/ida_anim/cryanimation.dll',
    'game': Path('D:/aa-re/client12/x2game_unpacked.dll'),
}

def sha(b): return hashlib.sha256(b).hexdigest()
def write(name, value):
    (OUT/name).write_text(json.dumps(value, indent=2)+'\n', encoding='utf-8')

def read(path):
    p = subprocess.run([str(ROOT/'tools/aa-pak.exe'), 'cat', path], capture_output=True, check=True)
    return p.stdout

def norm(p): return p.lower().replace('\\','/').removeprefix('game/').strip('./')

def chunks(d):
    assert d[:6] == b'CryTek'
    ft, ver, table = struct.unpack_from('<III',d,8)
    n = struct.unpack_from('<I',d,table)[0]
    rows = [list(struct.unpack_from('<IIIi'+('I' if ver==0x745 else ''), d, table+4+i*(20 if ver==0x745 else 16))) for i in range(n)]
    if ver == 0x744:
        ends=sorted(set([x[2] for x in rows]+[table,len(d)]))
        for x in rows: x.append(next(e for e in ends if e>x[2])-x[2])
    for ty,cv,at,cid,size in rows:
        b=d[at:at+size]
        if struct.unpack_from('<IIIi',b)==(ty,cv,at,cid): b=b[16:]
        yield ty,cv,cid,b

def cal(path):
    state={'folder':'','height':0.0}; result={}; sources=[]; base=0
    def parse(path,depth=0):
        nonlocal base
        assert depth<9
        raw=read(path); sources.append({'path':path,'sha256':sha(raw),'bytes':len(raw)})
        for line in raw.decode('latin1').splitlines():
            line=line.split('//')[0].strip()
            if '=' not in line: continue
            k,v=map(str.strip,line.split('=',1)); kl=k.lower()
            if not v: continue
            if kl=='#filepath': state['folder']=norm(v)
            elif kl=='#basemodelheight': base=float(v)
            elif kl=='#animmodelheight': state['height']=float(v)
            elif kl=='$include': parse(norm(v),depth+1)
            elif not kl.startswith(('#','$')):
                rel=norm(v)
                if not re.search(r'\.[a-z0-9]{1,4}$',rel): rel+='.caf'
                result[kl]={'path':state['folder']+'/'+rel,'anim_height':state['height'],'declaration':line,'source_cal':path}
    parse(path)
    return base,result,sources

def quat(k,f):
    if f==1: q=list(struct.unpack('<4f',k))
    else:
        v=int.from_bytes(k,'little'); ix=(v>>(46 if f==5 else 62))&3
        fields=[(15*n,15) for n in range(3)] if f==5 else [(0,21),(21,21),(42,20)]
        small=[((v>>s)&((1<<n)-1))/(((1<<n)-1)/(math.sqrt(2)))-1/math.sqrt(2) for s,n in fields]
        q=[]; j=0
        for i in range(4):
            if i==ix: q.append(math.sqrt(max(0,1-sum(x*x for x in small))))
            else: q.append(small[j]); j+=1
    mag=math.sqrt(sum(x*x for x in q))
    return [x/mag for x in q]

def clip(path):
    d=read(path); cs=list(chunks(d)); fps=30.; start=end=0
    for ty,cv,cid,b in cs:
        if ty==0xCCCC000E:
            spt,tpf=struct.unpack_from('<fi',b); start,end=struct.unpack_from('<ii',b,40); fps=1/(spt*tpf)
    tracks={}; skipped=[]
    for ty,cv,cid,b in cs:
        if ty!=0xCCCC000D: continue
        if cv!=0x829: skipped.append(hex(cv)); continue
        ctrl,nr,np=struct.unpack_from('<IHH',b); rf,rtf,pf,pki,ptf,al=struct.unpack_from('<6B',b,8); o=16
        def align():
            nonlocal o
            if al: o=(o+3)&~3
        def times(n,f):
            nonlocal o
            fmt,size={0:('f',4),1:('H',2),2:('B',1)}[f]
            a=struct.unpack_from('<'+str(n)+fmt,b,o); o+=size*n
            return [(t-start)/fps for t in a]
        rt=[]; r=[]; pt=[]; p=[]
        if nr:
            stride={1:16,5:6,8:8}[rf]
            r=[quat(b[o+i*stride:o+(i+1)*stride],rf) for i in range(nr)]
            o+=nr*stride; align(); rt=times(nr,rtf); align()
        if np:
            assert pf==2
            p=[list(struct.unpack_from('<3f',b,o+i*12)) for i in range(np)]
            o+=np*12; align()
            if pki==1: pt=times(np,ptf)
            elif np==nr: pt=rt[:]
            elif np==1: pt=[0.]
            else: raise ValueError((path,np,nr,pki))
        tracks[ctrl]={'rt':rt,'rot':r,'pt':pt,'pos':p,'format':[rf,rtf,pf,pki,ptf,al]}
    dur=(end-start)/fps
    if dur<=0: dur=max([t for tr in tracks.values() for t in tr['rt']+tr['pt']]+[0])
    return {'path':path,'sha256':sha(d),'bytes':len(d),'fps':fps,'start_frame':start,'end_frame':end,'duration':dur,'tracks':len(tracks),'skipped':skipped},tracks

def sample(times,values,t,rotation=False):
    if not times: return None
    if t<=times[0]: return values[0]
    if t>=times[-1]: return values[-1]
    j=next(i for i,x in enumerate(times) if x>t); i=j-1; a=values[i]; b=values[j]; f=(t-times[i])/(times[j]-times[i])
    if rotation and sum(x*y for x,y in zip(a,b))<0: b=[-x for x in b]
    q=[x*(1-f)+y*f for x,y in zip(a,b)]
    if rotation:
        mag=math.sqrt(sum(x*x for x in q)); q=[x/mag for x in q]
    return q

def dist(a,b): return math.sqrt(sum((x-y)**2 for x,y in zip(a,b)))
def angle(a,b): return math.degrees(2*math.acos(min(1,abs(sum(x*y for x,y in zip(a,b))))))

def assets():
    names=['fist_ac_sit_down_start','fist_ac_sit_down_loop','fist_ac_sit_up','music_co_sk_sit_down_piano_start','music_co_sk_sit_down_piano_cast','music_co_sk_sit_down_drum_start','music_co_sk_sit_down_drum_cast']
    index=[line.split('\t')[0].removeprefix('game/') for line in (ROOT/'data/pak12.tsv').read_text(encoding='utf-8').splitlines()]
    results=[]; clip_cache={}
    for race,prefix in [('nuian','nu'),('elf','el'),('hariharan','ha')]:
        for gender,g in [('male','m'),('female','f')]:
            cp=f'objects/characters/{race}/{gender}/'
            cals=[p for p in index if p.startswith(cp) and p.endswith(f'{prefix}_{g}_base.cal')]
            assert len(cals)==1,cals
            base,entries,sources=cal(cals[0]); row={'race':race,'gender':gender,'cal':cals[0],'base_height':base,'cal_sources':sources,'clips':[]}
            cdfs=[p for p in index if p.startswith(cp) and p.endswith(f'{prefix}_{g}_base.cdf')]
            row['cdf']=[]
            for p in cdfs:
                d=read(p); row['cdf'].append({'path':p,'sha256':sha(d),'text':d.decode('utf-8',errors='replace')})
            skeleton=cals[0][:-4]+'.chr'; sd=read(skeleton)
            bones=[]
            for ty,cv,cid,b in chunks(sd):
                if ty==0xACDC0000:
                    assert cv==0x801
                    for at in range(32,len(b)-323,324):
                        ctrl=struct.unpack_from('<I',b,at+208)[0]; name=b[at+216:at+264].split(b'\0')[0].decode('latin1'); po=struct.unpack_from('<i',b,at+264)[0]
                        if po==0 or name=='Bip01 Pelvis': bones.append({'name':name,'controller':ctrl,'parent_offset':po,'bind_model':struct.unpack_from('<12f',b,at+276)})
            row['skeleton']={'path':skeleton,'sha256':sha(sd),'selected_bones':bones}
            for name in names:
                e=entries.get(name)
                if not e: row['clips'].append({'name':name,'missing':True}); continue
                scale=base/e['anim_height'] if base>0 and e['anim_height']>0 else 1
                if e['path'] not in clip_cache: clip_cache[e['path']]=clip(e['path'])
                meta,tracks=clip_cache[e['path']]; item={'name':name,**e,**meta,'position_scale':scale,'bones':[]}
                for bone in bones:
                    tr=tracks.get(bone['controller'])
                    if not tr: item['bones'].append({'name':bone['name'],'missing_track':True}); continue
                    p=tr['pos']; q=tr['rot']; dur=meta['duration']
                    samples=[{'t':dur*i/8,'pos':sample(tr['pt'],p,dur*i/8),'rot':sample(tr['rt'],q,dur*i/8,True)} for i in range(9)]
                    summary={'name':bone['name'],'controller':bone['controller'],'format':tr['format'],'position_keys':len(p),'rotation_keys':len(q),'position_time_range':[tr['pt'][0],tr['pt'][-1]] if p else None,'rotation_time_range':[tr['rt'][0],tr['rt'][-1]] if q else None,'samples':samples}
                    if p:
                        summary.update({'first':p[0],'last':p[-1],'endpoint_delta_scaled':[scale*(y-x) for x,y in zip(p[0],p[-1])],'max_displacement_scaled':max(dist(p[0],x) for x in p)*scale,'max_horizontal_displacement_scaled':max(dist(p[0][:2],x[:2]) for x in p)*scale,'axis_min_scaled':[min(x[j] for x in p)*scale for j in range(3)],'axis_max_scaled':[max(x[j] for x in p)*scale for j in range(3)],'endpoint_gap_scaled':dist(p[0],p[-1])*scale})
                    if q: summary.update({'rotation_endpoint_gap_degrees':angle(q[0],q[-1]),'rotation_max_from_first_degrees':max(angle(q[0],x) for x in q)})
                    item['bones'].append(summary)
                row['clips'].append(item)
            results.append(row)
            print(race,gender,'base',base,'cdf',len(cdfs))
            for x in row['clips']:
                if x.get('missing'): print(' MISSING',x['name']); continue
                b=x['bones'][0]; print(' ',x['name'],'scale',round(x['position_scale'],6),'dur',round(x['duration'],4),'root',b.get('first'),b.get('last'),'reach',b.get('max_displacement_scaled'),'rot',b.get('rotation_max_from_first_degrees'))
    write('assets.json',{'provenance':'Asset decode by probe.py matching aa-anim caf.rs/cal.rs and aa-cry skeleton.rs; all pak reads via prebuilt aa-pak.exe cat. Values in Cry model space/metres, not native evaluated pose.','characters':results})

def load_binary(which):
    p=SOURCES[which]; d=p.read_bytes(); pe=pefile.PE(data=d); return p,d,pe,pe.get_memory_mapped_image()

def refs(image,base,needle):
    a=0; out=[]
    while (a:=image.find(needle,a))>=0: out.append(base+a); a+=1
    return out

def native(which,args):
    p,d,pe,image=load_binary(which); base=pe.OPTIONAL_HEADER.ImageBase
    lines=[f'{p} SHA256 {sha(d)} ImageBase {base:08x}']
    cs=Cs(CS_ARCH_X86,CS_MODE_32)
    dest=f'{which}-latest.txt'
    if '--out' in args:
        at=args.index('--out'); dest=args[at+1]; assert Path(dest).name==dest; del args[at:at+2]
    for arg in args:
        if ':' in arg and re.fullmatch('[0-9a-fA-F]+:[0-9a-fA-F]+',arg):
            a,b=(int(x,16) for x in arg.split(':')); raw=image[a-base:b-base]
            lines.append(f'\nSOURCE {a:08x}:{b:08x} SHA256 {sha(raw)}')
            lines.extend(f'{i.address:08x}: {i.mnemonic} {i.op_str}' for i in cs.disasm(raw,a))
        elif re.fullmatch('#[0-9a-fA-F]+:[0-9a-fA-F]+',arg):
            at,count=(int(x,16) for x in arg[1:].split(':'))
            for i in range(count):
                target=struct.unpack_from('<I',image,at-base+i*4)[0]
                string=image[target-base:target-base+100].split(b'\0')[0] if base<=target<base+len(image) else b''
                desc=repr(string) if string and all(32<=x<127 for x in string) else ''
                lines.append(f'table[{i}] {at+i*4:08x} -> {target:08x} {desc}')
        elif arg.startswith('@'):
            target=int(arg[1:],16); lines.append(f'\nREFS {target:08x}')
            for a in refs(image,base,struct.pack('<I',target)): lines.append(f'absolute {a:08x}')
            for s in pe.sections:
                if not s.Characteristics&0x20000000: continue
                a=s.VirtualAddress; end=a+s.Misc_VirtualSize
                while a<end-5:
                    a=image.find(b'\xe8',a,end)
                    if a<0: break
                    if base+a+5+struct.unpack_from('<i',image,a+1)[0]==target: lines.append(f'call {base+a:08x}')
                    a+=1
        else:
            for a in refs(image,base,arg.encode()+b'\0'):
                lines.append(f'\nSTRING {arg} {a:08x}')
                lines.extend(f'xref {r:08x}' for r in refs(image,base,struct.pack('<I',a)))
    result='\n'.join(lines)+'\n'
    (OUT/dest).write_text(result,encoding='utf-8')
    print(result)

if __name__=='__main__':
    if sys.argv[1]=='assets': assets()
    else: native(sys.argv[1],sys.argv[2:])
