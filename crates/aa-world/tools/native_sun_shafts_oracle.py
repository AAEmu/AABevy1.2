"""Execute original 1.2 x86 shaft producers. Requires pefile and unicorn.
Run from repo root; no game data is modified. JSON is small, executable evidence.
"""
from pathlib import Path
import json, struct, hashlib, sys
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import *
ROOT=Path(__file__).resolve().parents[3]
def machine(path):
 p=pefile.PE(str(ROOT/path));u=Uc(UC_ARCH_X86,UC_MODE_32)
 u.mem_map(p.OPTIONAL_HEADER.ImageBase,(p.OPTIONAL_HEADER.SizeOfImage+4095)&~4095)
 u.mem_write(p.OPTIONAL_HEADER.ImageBase,p.get_memory_mapped_image())
 u.mem_map(0x50000000,0x40000);u.mem_map(0x60000000,0x10000)
 return u
def put(u,a,fmt,*v):u.mem_write(a,struct.pack('<'+fmt,*v))
def get(u,a,fmt):return struct.unpack('<'+fmt,u.mem_read(a,struct.calcsize('<'+fmt)))
def frame(u):
 u.reg_write(UC_X86_REG_ESP,0x60008000);u.reg_write(UC_X86_REG_EBP,0x60008800)
u=machine('data/extract/aa-cry/ida/cry3dengine.dll')
put(u,0x50000010,'I',0x50001000);put(u,0x50000100,'I',0x50000200)
put(u,0x50000538,'I',0x50000800);u.mem_write(0x50000800,b'\xc2\x0c\0')
bind=[]
def setter(uc,a,n,_):
 if a!=0x50000800:return
 sp=uc.reg_read(UC_X86_REG_ESP);name,value,force=get(uc,sp+4,'IfI')
 s=bytes(uc.mem_read(name,64)).split(b'\0')[0].decode();bind.append((s,value,force))
u.hook_add(UC_HOOK_CODE,setter)
rows=[]
for amount,rays,atten in [(0,0,5),(-1,0,5),(.05,0,5),(.049999997,0,5),(.3,2.5,5),(1,.05,-1),(.2,8,12),(0,.049999997,5),(0,.05,5)]:
 frame(u);u.reg_write(UC_X86_REG_EDI,0x50000000);u.reg_write(UC_X86_REG_ESI,0x50000100)
 for off,v in [(0x554,amount),(0x564,rays),(0x574,atten)]:put(u,0x50001000+off,'f',v)
 bind.clear();u.emu_start(0x300a78d1,0x300a79c4,count=1000)
 assert len(bind)==4 and all(f==0 for n,v,f in bind)
 rows.append(dict(input=[amount,rays,atten],params={n:v for n,v,f in bind},force=[f for n,v,f in bind]))
r=machine('data/extract/aa-cry/ida/render/cryrenderd3d10.dll')
obj=0x50000000
for off,ptr,value in [(0x18,0x50001000,.25),(0x1c,0x50001020,2.5),(0x20,0x50001040,5),(0x24,0x50001060,1)]:
 put(r,obj+off,'I',ptr);put(r,ptr,'If',0x3830c9b0,value)
# Original CParamVec4 layout/vtable from constructor 38260610, called by
# the SunShafts_RaysCustomColor registration at 382618E0. Default = white.
put(r,obj+0x28,'I',0x50001080)
put(r,0x50001080,'I4f',0x3830c9dc,1,1,1,1)
colors=[]
for rays,sun in [(2.5,[1,1,1]),(1,[4,2,1]),(.1,[0,0,0]),(.21299468,[.2,.15,.1])]:
 put(r,0x50001024,'f',rays);frame(r)
 put(r,0x60008000,'II',0x5003f000,0x50002000);r.reg_write(UC_X86_REG_ECX,obj)
 r.emu_start(0x3806e3c0,0x5003f000,count=1000)
 params=get(r,0x50002000,'8f')
 frame(r);bp=r.reg_read(UC_X86_REG_EBP)
 put(r,bp-0xa4,'8f',*params);put(r,bp-0x4c,'3f',*sun)
 r.emu_start(0x3806d661,0x3806d75d,count=1000)
 colors.append(dict(rays=rays,sun=sun,packed=list(params),color=list(get(r,bp-0x40,'3f'))))
# Original HDR scan/branch. Stop before graphics calls on either outcome.
# The effect's runtime flag and Active/Amount deliberately vary; no mocks inside the tested slice.
gates=[]
for active,amount,mode,merge,post,special,present in [(0,0,1,1,1,0,1),(1,.3,1,1,1,0,1),(0,0,0,1,1,0,1),(1,.3,1,0,1,0,1),(1,.3,1,1,0,0,1),(1,.3,1,1,1,1,1),(1,.3,1,1,1,0,0)]:
 frame(r);bp=r.reg_read(UC_X86_REG_EBP)
 renderer=0x50008000;manager=0x50020000;effects=0x50021000;hdr=0x50022000;caller=0x60009000
 put(r,0x383b17c4,'I',renderer);put(r,renderer+66552,'I',manager)
 put(r,manager+92,'II',effects,effects+4 if present else effects);put(r,effects,'I',obj)
 r.mem_write(obj+5,b'\x05');r.mem_write(obj+12,bytes([active]));put(r,0x50001004,'f',amount)
 put(r,obj+8,'I',0x500010a0);put(r,0x500010a0,'If',0x3830c9b0,active)
 put(r,0x383b1200,'I',mode);put(r,0x383b11a8,'I',post);r.mem_write(caller+8,bytes([merge]));r.mem_write(hdr,bytes([special]))
 r.reg_write(UC_X86_REG_EBX,caller);r.reg_write(UC_X86_REG_EDI,hdr)
 visited=[]
 def boundary(uc,a,n,_):
  if a in (0x3806ce58,0x3806cee3):visited.append(a);uc.emu_stop()
 hook=r.hook_add(UC_HOOK_CODE,boundary);r.emu_start(0x3806cdb9,0x3806d000,count=1000);r.hook_del(hook)
 gates.append(dict(active=active,amount=amount,mode=mode,merge=merge,post=post,special=special,present=present,generates=visited==[0x3806ce58]))
out=dict(sha256={p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in ['data/extract/aa-cry/ida/cry3dengine.dll','data/extract/aa-cry/ida/render/cryrenderd3d10.dll']},tod=rows,hdr_colors=colors,hdr_gates=gates)
p=ROOT/'data/extract/aa-world/wr3/native-golden.json'
p.parent.mkdir(parents=True,exist_ok=True)
p.write_text(json.dumps(out,indent=2))
if '--check' in sys.argv:
 expected=json.loads(Path(__file__).with_name('native_sun_shafts_golden.json').read_text())
 assert out==expected, 'Original DLL hash or executable oracle outputs changed; inspect the evidence.'
 print('PASSED: 9 native TOD bindings, 4 native HDR color probes, 7 native HDR list/gates.')
else:
 print(json.dumps(out,indent=2))
