import sys,json,struct,hashlib,math
from pathlib import Path
sys.path.insert(0,'crates/aa-fx/evidence')
from force_generation import Native,DLL,ROOT
from unicorn.x86_const import UC_X86_REG_EAX,UC_X86_REG_ECX,UC_X86_REG_ESP,UC_X86_REG_EIP
import pefile
O=Path(__file__).resolve().parent
assert hashlib.sha256(DLL.read_bytes()).hexdigest()=='34d6b73690d1a9d8d0ea0f5eb743fd0624107cfda28c1302826b19a3bc9546de'
B=lambda f:struct.unpack('<I',struct.pack('<f',f))[0]
class Area(Native):
 def __init__(self):
  self.handlers={}; self.events=[]; self.areas={}; self.newbox=None; self.fail_create=False
  super().__init__()
  self.vm.mem_map(0,4096); self.u32(0,0xFFFFFFFF)
  self.call(0x30247650); self.call(0x302476B0)
  crt=pefile.PE(str(self.crt_path)); exports={e.name:crt.OPTIONAL_HEADER.ImageBase+e.address for e in crt.DIRECTORY_ENTRY_EXPORT.symbols if e.name}
  for entry in self.pe.DIRECTORY_ENTRY_IMPORT:
   for i in entry.imports:
    if i.name in exports: self.u32(i.address,exports[i.name])
  self.world,self.wvt,self.manager,self.mvt=[self.alloc(512) for _ in range(4)]
  self.u32(self.world,self.wvt); self.u32(self.manager,self.mvt); self.u32(0x302DCD28,self.world)
  self.u32(self.wvt+40,self.stub(lambda:self.ret(self.manager)))
  self.u32(self.mvt+28,self.stub(self.make_box))
  self.u32(self.wvt+328,self.stub(self.add_area))
  self.u32(self.wvt+100,self.stub(self.destroy))
  self.u32(0x302A9730,0)
 def hook(self,vm,address,size,_):
  if address in self.handlers:self.handlers[address]()
  else:super().hook(vm,address,size,_)
 def ret(self,value=1,arg_bytes=0):
  sp=self.vm.reg_read(UC_X86_REG_ESP); self.vm.reg_write(UC_X86_REG_EAX,value); self.vm.reg_write(UC_X86_REG_EIP,self.get_u32(sp)); self.vm.reg_write(UC_X86_REG_ESP,sp+4+arg_bytes)
 def args(self,n):
  sp=self.vm.reg_read(UC_X86_REG_ESP); return [self.get_u32(sp+4+4*i) for i in range(n)]
 def stub(self,h):
  p=self.alloc(16); self.vm.mem_write(p,b'\xc3'); self.handlers[p]=h; return p
 def make_box(self):
  kind,ptr=self.args(2); self.newbox=self.floats(ptr,16); self.events.append(dict(op='CreatePrimitive',kind=kind,box=self.newbox)); self.ret(self.alloc(64),8)
 def add_area(self):
  geom,pos,q,scale=self.args(4)
  if self.fail_create:self.events.append(dict(op='AddArea-failed')); self.ret(0,16); return
  obj,vt,g,gv=[self.alloc(128) for _ in range(4)]; self.u32(obj,vt); self.u32(g,gv)
  self.areas[obj]=dict(box=self.newbox,pos=self.floats(pos,3),q=self.floats(q,4),scale=struct.unpack('<f',struct.pack('<I',scale))[0],geom=g)
  self.u32(vt+28,self.stub(self.set_params)); self.u32(vt+36,self.stub(self.status)); self.u32(gv+24,self.stub(self.primitive))
  self.events.append(dict(op='AddArea',area=obj,**self.areas[obj])); self.ret(obj,16)
 def primitive(self):
  obj=self.vm.reg_read(UC_X86_REG_ECX); ptr=self.args(1)[0]
  ar=next(a for a in self.areas.values() if a['geom']==obj); self.vec(ptr,ar['box']); self.ret(1,4)
 def status(self):
  obj=self.vm.reg_read(UC_X86_REG_ECX); ptr=self.args(1)[0]; a=self.areas[obj]
  self.vec(ptr+24,a['pos']); self.vec(ptr+60,a['q']); self.f32(ptr+76,a['scale']); self.u32(ptr+92,a['geom']); self.ret(1,4)
 def set_params(self):
  obj=self.vm.reg_read(UC_X86_REG_ECX); ptr,thread=self.args(2); kind=self.get_u32(ptr); size={0:56,11:24,18:48,12:80}[kind]
  raw=bytes(self.vm.mem_read(ptr,size)); data=list(struct.unpack('<'+'I'*(size//4),raw))
  self.events.append(dict(op='SetParams',area=obj,type=kind,u32=data,f32=self.floats(ptr,size//4)))
  if kind==0:self.areas[obj].update(pos=self.floats(ptr+4,3),q=self.floats(ptr+16,4),scale=self.floats(ptr+32,1)[0])
  self.ret(1,8)
 def destroy(self):
  obj,mode,thread=self.args(3); self.events.append(dict(op='DestroyPhysicalEntity',area=obj,mode=mode,thread=thread)); self.ret(1,12)
 def fixture(self,mode=1,now=.1,start=0,end=0,life=.4,speed=50,size=.1,count=0,continuous=False,focus=(0,0),rot=(0,0,0,1),translation=(2,3,4),offset=(0,0,0),random_offset=(0,0,0),roundness=0,angle=(180,1),scale=1,source_scale=1,geometry_scale=1,stretch=1,air=0,alpha=1):
  sub,params,src,fx=self.emitter(mode=mode,start=start,end=end,now=now,offset=offset,translation=translation,scale=scale)
  self.u32(fx+112,1); self.f32(fx+128,source_scale); self.vec(fx+508,(0,0,-9.81)); self.vec(fx+532,(0,0,1)); self.f32(fx+544,4)
  self.f32(src+60,geometry_scale); self.vec(src+8,rot)
  self.f32(params+24,count); self.f32(params+60,life); self.f32(params+148,alpha); self.f32(params+248,size); self.f32(params+404,speed); self.f32(params+416,air)
  self.vec(params+380,random_offset); self.f32(params+392,roundness); self.f32(params+588,focus[0]); self.f32(params+600,focus[1]); self.f32(params+616,angle[0]); self.f32(params+620,angle[1]); self.vm.mem_write(params+5,bytes([continuous]))
  self.f32(params+312,stretch)
  # Native container particle lifetime bound, independent of actual Count/live list.
  self.f32(self.get_u32(sub+8)+60,life)
  return sub,params,src,fx
 def update(self,sub):
  self.events=[]; self.call(0x3008BB90,sub); return dict(events=self.events,handle=self.get_u32(sub+128),clock=self.floats(sub+60,4),edit_counter=self.get_u32(0x302A9730))

def run():
 n=Area(); cases=[]
 for name,kw in [('wind-count0',{}),('wind-count100',{'count':100}),('gravity',{'mode':2}),('none',{'mode':0}),('target',{'mode':3}),('before',{'now':-.01}),('zero-at',{'now':0}),('life-end',{'now':.4}),('after-life',{'now':.401}),('directional',{'angle':(0,0)}),('cone',{'angle':(45,0)}),('random-offset',{'random_offset':(1,2,3)}),('offset-rot-scale',{'offset':(1,2,3),'scale':2,'source_scale':3,'rot':(0,0,math.sqrt(.5),math.sqrt(.5))}),('continuous-start',{'continuous':True,'end':1,'now':.1}),('continuous-steady',{'continuous':True,'end':1,'now':.5}),('continuous-tail',{'continuous':True,'end':1,'now':1.2}),('air-resistance',{'air':2}),('negative-speed',{'speed':-50})]:
  n.u32(0x302A9730,0); s,p,src,fx=n.fixture(**kw)
  try: result=n.update(s)
  except Exception as e:
   print(name,type(e).__name__,str(e),'EIP',hex(n.vm.reg_read(UC_X86_REG_EIP))); raise
  cases.append(dict(name=name,input=kw,**result))
 s,p,src,fx=n.fixture(); n.u32(0x302A9730,0); seq=[]
 for label in ['create','unchanged','move','shape-change','zero-speed','recreate','stop']:
  if label=='move':n.vec(src+24,(4,5,6))
  if label=='shape-change':n.f32(params:=p+380,2)
  if label=='zero-speed':n.f32(p+404,0)
  if label=='recreate':n.f32(p+404,50)
  if label=='stop':n.f32(s+116,2)
  n.u32(0x302A9730,0); seq.append(dict(name=label,**n.update(s)))
 s,p,src,fx=n.fixture(); n.u32(0x302A9730,0); n.update(s); n.events=[]; n.call(0x30089EB0,s); reset=dict(events=n.events,handle=n.get_u32(s+128))
 out=dict(dll=str(DLL),sha256=hashlib.sha256(DLL.read_bytes()).hexdigest(),crt_sha256=hashlib.sha256(n.crt_path.read_bytes()).hexdigest(),cases=cases,lifecycle=seq,reset=reset,boundary='Original producer instructions and CRT math; world/geometry vtables record calls, supply prior state, accept creation. No physics solver, renderer or live process.')
 (O/'wind_producer_native.json').write_text(json.dumps(out,indent=2)+'\n'); print('wrote native-producer.json; cases',len(cases))
 for c in cases:
  ev=c['events']; add=next((x for x in ev if x['op']=='AddArea'),{}); ap=next((x['f32'] for x in ev if x.get('type')==18),[]); bp=next((x['f32'] for x in ev if x.get('type')==12),[]); print(c['name'],[x['op']+(':'+str(x['type']) if 'type' in x else '') for x in ev],'scale',add.get('scale'),'box',add.get('box',[])[10:],'area',ap[1:9],'flow',bp[6:9])
 for c in seq: print('LIFECYCLE',c['name'],[x['op']+(':'+str(x['type']) if 'type' in x else '') for x in c['events']])
 print('RESET',reset)
if __name__=='__main__':run()
