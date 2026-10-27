"""READ-ONLY native status23/cached-FX boundary audit. No production implementation.
World native-physics.py is read as evidence, NOT imported (it writes world files).
Reuse only the prior aa-fx VM class; all new output stays in this ignored folder.
"""
from pathlib import Path
import sys,json,struct,math,hashlib
sys.dont_write_bytecode=True
ROOT=Path('.');OUT=Path(__file__).resolve().parent
WORLD=ROOT/'data/extract/aa-world/local-wind-receiver-audit-20261023'
OLD=OUT
sys.path.insert(0,str(OLD));from wind_area import Area,DLL
import pefile,capstone
from unicorn import UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_XMM1,UC_X86_REG_XMM2,UC_X86_REG_XMM4,UC_X86_REG_MXCSR
PHYS=Path('<re-notes>/cryphysics/work_x86/cryphysics_unpacked.dll')
PINS={str(DLL):'34d6b73690d1a9d8d0ea0f5eb743fd0624107cfda28c1302826b19a3bc9546de',str(PHYS):'32f6e4c316f589eb51988a9f2343405eec179b42eaab6ef8f045efd5bc9ce713'}
for p,h in PINS.items():assert hashlib.sha256(Path(p).read_bytes()).hexdigest()==h,p
f32=lambda x:struct.unpack('<f',struct.pack('<f',x))[0]
bits=lambda x:struct.unpack('<I',struct.pack('<f',x))[0]
frombits=lambda x:struct.unpack('<f',struct.pack('<I',x))[0]
BEVY=lambda v:[v[0],v[2],-v[1]]
class Kernel:
 def __init__(self):
  self.n=Area();n=self.n;self.trace={};self.inside_calls=0
  self.initial_mxcsr=n.vm.reg_read(UC_X86_REG_MXCSR)
  pe=pefile.PE(str(PHYS));self.pe=pe;base=pe.OPTIONAL_HEADER.ImageBase
  n.vm.mem_map(base,(pe.OPTIONAL_HEADER.SizeOfImage+4095)&-4096);n.vm.mem_write(base,pe.get_memory_mapped_image())
  crt=pefile.PE(str(n.crt_path));exports={e.name:crt.OPTIONAL_HEADER.ImageBase+e.address for e in crt.DIRECTORY_ENTRY_EXPORT.symbols if e.name}
  for line in Path('<re-notes>/cryphysics/x86_12/iat_map.txt').read_text().splitlines():
   address,qualified=line.split(maxsplit=1);module,name=qualified.split('!',1)
   if module.upper()=='MSVCR100.DLL' and name.encode() in exports:n.u32(int(address,16),exports[name.encode()])
  self.world=n.alloc(24000);self.status=n.alloc(160);self.point=n.alloc(12);self.gravity=n.alloc(12);self.wind=n.alloc(12)
  self.cache=n.alloc(120);self.uncached=n.alloc(120)
  self.geom,self.gvt=n.alloc(64),n.alloc(128);n.u32(self.geom,self.gvt)
  n.u32(self.gvt+4,n.stub(lambda:n.ret(0)))
  def inside():
   p=n.args(1)[0];self.inside_calls+=1;n.ret(int(max(abs(x) for x in n.floats(p,3))<=1),4)
  n.u32(self.gvt+32,n.stub(inside))
  self.engine=n.alloc(24000);self.output=n.alloc(16);self.bbox=n.alloc(24);self.head=n.alloc(32);self.current_areas=[]
  n.u32(0x302F93D8,1);n.vec(self.engine+284,(0,0,0));cv=n.get_u32(0x302DCD40);n.u32(cv+328,1);n.u32(cv+332,1)
  n.u32(0x302A8FB8,n.get_u32(0x302A8FB8)|1);n.u32(0x302A8C30,n.get_u32(0x302A8C30)|1)
  def iterate():
   prev=n.args(1)[0];lst=[self.head]+self.current_areas
   n.ret(lst[0] if prev==0 else lst[lst.index(prev)+1] if prev in lst and lst.index(prev)+1<len(lst) else 0,4)
  n.u32(n.wvt+340,n.stub(iterate));n.vm.hook_add(UC_HOOK_CODE,self.hook)
 def hook(self,vm,address,size,_):
  probes={0x3504F88E:('status_metric2',UC_X86_REG_XMM1),0x3504FA8F:('status_world_distance2',UC_X86_REG_XMM1),0x300880AD:('cache_metric2',UC_X86_REG_XMM1),0x30088174:('cache_norm2_plus_min_normal',UC_X86_REG_XMM4)}
  if address in probes:
   key,reg=probes[address];v=vm.reg_read(reg)&0xFFFFFFFF;self.trace[key]=dict(bits=f'{v:08x}',value=frombits(v))
 def area(self,radius=7.84,speed=78.4,center=(0,0,0),aabb=None,rotation=None):
  n=self.n;ar=n.alloc(512);n.call(0x3504BA10,ar,(self.world,));R=f32(radius);c=tuple(map(f32,center))
  n.vec(ar+4,aabb if aabb else tuple(f32(x-R) for x in c)+tuple(f32(x+R) for x in c));n.vec(ar+84,c);n.f32(ar+132,R);n.f32(ar+136,1/R)
  if rotation:n.vec(ar+96,rotation)
  n.u32(ar+188,self.geom);n.f32(ar+320,0);n.f32(ar+332,0);n.vec(ar+340,(0,0,speed));n.u32(ar+388,1);n.vec(ar+356,(0,0,1));n.vec(ar+368,(0,0,-4));n.vec(ar+404,(1,1,1));n.vec(ar+416,(1,1,1));n.u32(ar+428,0);n.f32(ar+432,0);n.f32(ar+436,1)
  return ar
 def status23(self,ar,point,half=(0,0,0),flags=0):
  n=self.n;n.call(0x300899F0,self.status);n.vec(self.status+4,point);n.vec(self.status+16,half);n.vm.mem_write(self.status+40,bytes([flags]));self.trace={};self.inside_calls=0
  ret=n.call(0x350581E0,ar,(self.status,));return dict(ret=ret,flow_raw=n.floats(self.status+80,3),flow=n.floats(self.status+80,3) if ret else [0,0,0],trace=dict(self.trace),geometry_inside_calls=self.inside_calls)
 def fx(self,ar,point,cached):
  n=self.n;obj=self.cache if cached else self.uncached
  n.u32(obj,ar);n.u32(obj+4,4);n.vm.mem_write(obj+12,bytes([cached,1]));c=n.floats(ar+84,3);inv=n.floats(ar+136,1)[0]
  n.vec(obj+16,c);n.vec(obj+28,(inv,0,0,-f32(c[0]*inv),0,inv,0,-f32(c[1]*inv),0,0,inv,-f32(c[2]*inv)));n.f32(obj+76,0);n.vec(obj+80,(0,0,0));n.vec(obj+92,n.floats(ar+340,3))
  n.vec(self.point,point);n.vec(self.gravity,(0,0,0));n.vec(self.wind,(0,0,0));self.trace={};self.inside_calls=0;n.call(0x30087F40,obj,(self.point,self.gravity,self.wind))
  return dict(flow=n.floats(self.wind,3),trace=dict(self.trace),geometry_inside_calls=self.inside_calls)
 def row(self,label,ar,point,half=(0,0,0)):
  p=tuple(map(f32,point));h=tuple(map(f32,half));return dict(name=label,point=p,point_bits=[f'{bits(x):08x}' for x in p],half=h,radius=self.n.floats(ar+132,1)[0],status=self.status23(ar,p,h),fx_uncached=self.fx(ar,p,False),fx_cached=self.fx(ar,p,True))
 def getwind(self,areas,point,half=(0,0,0),ambient=(0,0,0)):
  n=self.n;self.current_areas=areas;n.vec(self.engine+284,ambient);n.vec(self.bbox,tuple(f32(point[i]-half[i]) for i in range(3))+tuple(f32(point[i]+half[i]) for i in range(3)));n.call(0x3012BB80,self.engine,(self.output,self.bbox,0));box=n.floats(self.bbox,6);center=[f32(f32(box[i]+box[i+3])*.5) for i in range(3)];return dict(flow=n.floats(self.output,3),query_center=center,bbox=box)

def run():
 world_inputs={}
 k=Kernel();n=k.n;ar=k.area();rows=[];small=frombits(bits(math.sqrt(frombits(0x1E3CE508))))
 for label,x in [('zero',0),('min-subnormal',frombits(1)),('cache-min-normal-scale',math.sqrt(frombits(0x00800000))),('tiny-1e-30',1e-30),('tiny-1e-19',1e-19),('tiny-1e-11',1e-11),('ordinary-1e-9',1e-9)]:rows.append(k.row(label,ar,(x,0,0)))
 for i in range(-4,5):
  x=frombits(bits(small)+i)
  for axis in range(3):
   for sign in [-1,1]:
    p=[0,0,0];p[axis]=x*sign;rows.append(k.row(f'norm-threshold-{i:+d}-axis{axis}-sign{sign}',ar,p))
 # Exact squared-distance equality is constructed from the serialized native
 # constant and adjacent f32 values; no hand-picked epsilon is introduced.
 E=frombits(0x1E3CE508);xlo=frombits(bits(small)-1);y=f32(math.sqrt(E-f32(xlo*xlo)))
 assert bits(f32(f32(xlo*xlo)+f32(y*y)))==0x1E3CE508
 for axis_pair in [(0,1),(0,2),(1,2)]:
  for sign in [-1,1]:
   p=[0,0,0];p[axis_pair[0]]=sign*xlo;p[axis_pair[1]]=y
   rows.append(k.row(f'exact-distance2-equality-{axis_pair}-{sign}',ar,p))
 for i in range(-8,9):
  x=frombits(bits(7.84)+i)
  for sign in [-1,1]:rows.append(k.row(f'radius-ulp-{i:+d}-sign{sign}',ar,(x*sign,0,0)))
 for i in range(-3,4):
  x=frombits(bits(7.84/math.sqrt(2))+i);rows.append(k.row(f'diagonal-radius-ulp-{i:+d}',ar,(x,x,0)))
 for R in [.5,31.36,128]:
  a=k.area(R)
  for i in [-1,0,1]:rows.append(k.row(f'world-threshold-R{R}-{i:+d}',a,(frombits(bits(small)+i),0,0)))
  for i in [-1,0,1]:rows.append(k.row(f'radius-R{R}-{i:+d}',a,(frombits(bits(R)+i),0,0)))
 for R in [1e-9,1e-10,1e-11]:
  a=k.area(R)
  for fraction in [0,.5,1,1.01]:rows.append(k.row(f'tiny-radius-{R}-fraction{fraction}',a,(f32(f32(R)*fraction),0,0)))
 for c in [(2,3,4),(100000,200000,300000)]:
  a=k.area(center=c)
  for label,p in [('center',c),('next-x',(frombits(bits(c[0])+1),c[1],c[2])),('half-x',(f32(c[0]+f32(7.84)*.5),c[1],c[2]))]:rows.append(k.row(f'translated-{c[0]}-{label}',a,p))
 neg=k.area(speed=-78.4)
 for p in [(0,0,0),(1e-11,0,0),(3.92,0,0),(7.84,0,0)]:rows.append(k.row('negative-speed-'+str(p),neg,p))
 # Half-extent and overlap branch proof, including intentionally inconsistent
 # AABB fixtures to isolate overlap rejection (NOT a production field state).
 overlap=[]
 for p in [(0,0,0),(3.92,0,0),(7.84,0,0),(7.85,0,0),(5.6,5.6,0)]:
  for h in [(0,0,0),(.1,.2,1),(20,20,20)]:overlap.append(k.row('consistent-overlap',ar,p,h))
 inconsistent=k.area(aabb=(10,10,10,11,11,11))
 for h in [(0,0,0),(frombits(1),0,0),(.1,.2,1),(frombits(bits(10)-1),)*3,(10,10,10),(frombits(bits(10)+1),)*3]:overlap.append(k.row('inconsistent-aabb-gate-only',inconsistent,(0,0,0),h))
 # Original bUniform/cache request classification, not a point-value query.
 cache_request=[]
 for p,h in [((0,0,0),(0,0,0)),((3.92,0,0),(.1,.2,1)),((7.84,0,0),(.1,.2,1)),((7.85,0,0),(.1,.2,1)),((100,0,0),(.1,.2,1))]:
  cache_request.append(dict(point=p,half=h,**k.status23(ar,p,h,flags=1)))
 # Native GetWind -> real status chain; per-field singular behavior must not
 # erase contributions already accumulated from another field.
 aggregate=[]
 for label,p,speedA in [('near-center',(0,0,0),78.4),('tiny-center',(1e-11,0,0),78.4),('radius',(7.84,0,0),78.4),('outside',(frombits(bits(7.84)+2),0,0),78.4),('negative-center',(0,0,0),-78.4)]:
  A=k.area(speed=speedA);B=k.area(center=(f32(p[0]-f32(3.92)),p[1],p[2]));singleA=k.status23(A,p)['flow'];singleB=k.status23(B,p)['flow']
  for order,areas in [('AB',[A,B]),('BA',[B,A])]:
   got=k.getwind(areas,p,ambient=(1,2,3));aggregate.append(dict(name=label,order=order,point=p,per_field=[singleA,singleB],native_getwind=got,bevy_total=BEVY(got['flow'])))
 # Separate local delta cannot preserve every ambient-first f32 rounding result.
 # Two admitted-recipe fields with opposite radial directions isolate that
 # API boundary without changing the field kernel or adding a public method.
 ambient_rounding=[];opposed=[k.area(center=(-1e-9,0,0)),k.area(center=(1e-9,0,0))]
 for order,areas in [('AB',opposed),('BA',opposed[::-1])]:
  flows=[k.status23(a,(0,0,0))['flow'] for a in areas];delta=[f32(flows[0][i]+flows[1][i]) for i in range(3)];ambient=(f32(1e-6),0,0)
  ambient_rounding.append(dict(order=order,per_field=flows,local_delta=delta,ambient=ambient,separate_delta_then_ambient=[f32(delta[i]+ambient[i]) for i in range(3)],native_getwind=k.getwind(areas,(0,0,0),ambient=ambient)))
 # Reproduce every executed world status/getter case without touching world output.
 previous=json.loads((OUT/'wind_world_status_fixture.json').read_text(encoding='utf-8'));reproduced=[]
 for old,chain in zip(previous['cases'],previous['getwind_chain']):
  got=k.status23(ar,old['center'],old['half']);g=k.getwind([ar],old['center'],old['half'],(1,1,0));assert got['ret']==old['status_return'] and got['flow']==old['accepted_flow'],(old,got);assert g['flow']==chain['total_wind'],(g,chain);point_result=k.status23(ar,g['query_center'])['flow'];assert [f32(point_result[i]+[1,1,0][i]) for i in range(3)]==g['flow'],(g,point_result);reproduced.append(dict(center=old['center'],half=old['half'],actual_center=g['query_center'],status_equal=True,getwind_equal=True,center_point_equals_getwind=True))
 # Explicit private VM FP controls, not a claim about a running client.
 fp_profiles=[]
 for profile in [0,0x1F80,0x9FC0]:
  for label,p in [('center',(0,0,0)),('below-T',(xlo,0,0)),('equal-T',(xlo,y,0)),('above-T',(small,0,0)),('radius',(f32(7.84),0,0)),('subnormal-distance2',(f32(math.sqrt(frombits(0x00800000)*.5)),0,0))]:
   n.vm.reg_write(UC_X86_REG_MXCSR,profile);st=k.status23(ar,p)
   n.vm.reg_write(UC_X86_REG_MXCSR,profile);ca=k.fx(ar,p,True)
   fp_profiles.append(dict(mxcsr=f'{profile:08x}',name=label,point=p,status=st,fx_cached=ca))
 n.vm.reg_write(UC_X86_REG_MXCSR,0)
 # Replay all previously accepted cached-FX wind goldens exactly. Preserve
 # their original native ambient accumulation and repeated-add rounding.
 cache_goldens=[];old_cache=json.loads((OUT/'wind_extra_native.json').read_text(encoding='utf-8'))['cached_receiver'];original_cache_ar=k.area(center=(2,3,4))
 for old in old_cache:
  p=old.get('pos',(2+7.84*.5,3,4));k.fx(original_cache_ar,p,True);n.vec(k.wind,(1,2,3))
  for _ in range(2 if old['name']=='overlap-two' else 1):n.call(0x30087F40,k.cache,(k.point,k.gravity,k.wind))
  got=n.floats(k.wind,3);assert got==old['wind'],(old,got);cache_goldens.append(dict(name=old['name'],flow=got,unchanged=True))
 constants={}
 for label,address in [('status_distance2_threshold',0x351FA2A0),('cache_min_normal',0x3025551C)]:
  v=n.get_u32(address);constants[label]=dict(address=f'{address:08x}',bits=f'{v:08x}',value=frombits(v))
 assert all(hashlib.sha256((WORLD/name).read_bytes()).hexdigest()==h for name,h in world_inputs.items()),'world evidence changed during run'
 data=dict(pins=PINS,consumed_world_inputs=world_inputs,vm_default_mxcsr=f'{k.initial_mxcsr:08x}',fp_profiles=fp_profiles,cached_fx_goldens=cache_goldens,constants=constants,boundaries=rows,overlap=overlap,cache_request=cache_request,aggregate=aggregate,ambient_rounding=ambient_rounding,reproduced_world_cases=reproduced,boundary='Original engine and retained unpacked original physics instructions in private Unicorn. Area state from world fixture, geometry type/inside and registry are explicit fixtures. No loader, creation, live process, physics solver, Cargo, GPU or production source execution.')
 (OUT/'wind_kernel_native.json').write_text(json.dumps(data,indent=2)+'\n',encoding='utf-8')
 md=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_32);asm=[]
 for s,e in [(0x3504F84E,0x3504FB94),(0x350581E0,0x35059090),(0x30087F40,0x30088310),(0x30087DC0,0x30087F40)]:
  asm.append(f'\n# {s:08x}..{e:08x}')
  asm += [f'{i.address:08x} {i.bytes.hex():30} {i.mnemonic:8} {i.op_str}' for i in md.disasm(n.vm.mem_read(s,e-s),s)]
 (OUT/'wind_kernel_native.asm').write_text('\n'.join(line.rstrip() for line in asm)+'\n',encoding='utf-8')
 print('native boundary cases',len(rows),'overlap',len(overlap),'aggregate',len(aggregate),'world status/getwind reproduced',len(reproduced)*2);print('constants',constants)
 for name in ['zero','tiny-1e-11','ordinary-1e-9','norm-threshold-+0-axis0-sign1','radius-ulp-+0-sign1','radius-ulp-+1-sign1']:
  row=next((r for r in rows if r['name']==name),None)
  if row:print(name,'status',row['status']['flow'],'cache',row['fx_cached']['flow'],'trace',row['status']['trace'])
if __name__=='__main__':run()
