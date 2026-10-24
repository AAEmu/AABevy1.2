"""Bounded original roadmap projection CPU oracle. No renderer, clients or production writes.
Actual shared world->Map math, player caller, NPC/given-quest anchor loops and
zone/city rect selector execute. World-origin conversion, static DB row lookup,
heading, drawable binding and descriptor visibility are explicit endpoints.
"""
from pathlib import Path
import sys,site,struct,hashlib,json,sqlite3
sys.path.insert(0,site.getusersitepackages())
import pefile
from unicorn import Uc,UC_ARCH_X86,UC_MODE_32,UC_HOOK_CODE
from unicorn.x86_const import *
OUT=Path('data/extract/aa-ui/minimap-audit/projection')
GAME=Path('D:/aa-re/client12/x2game_unpacked.dll')
M,PT,RESULT,ENTITY,WORLD,ENGINE,ORIGIN,OVT= [0x10000000+i*0x1000 for i in range(8)]
DRAW,ARRAY,ROW,MANAGER,HEAD,DESC,ZROW,CROW=[0x10010000+i*0x1000 for i in range(8)]
STACK,STOP=0x201f0000,0x3000fff0
class Native:
 def __init__(self):
  self.u=Uc(UC_ARCH_X86,UC_MODE_32);raw=GAME.read_bytes();self.sha=hashlib.sha256(raw).hexdigest();assert self.sha=='851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
  p=pefile.PE(data=raw);base=p.OPTIONAL_HEADER.ImageBase;data=p.get_memory_mapped_image()
  self.u.mem_map(base,(len(data)+4095)&~4095);self.u.mem_write(base,data)
  for a,n in [(0,0x10000),(0x10000000,0x200000),(0x20000000,0x200000),(0x30000000,0x10000)]:self.u.mem_map(a,n)
  self.u.hook_add(UC_HOOK_CODE,self.hook);self.anchors=[];self.static_requests=[];self.kind='';self.origin=(10240.,11264.)
  self.put(0x39b02294,WORLD);self.put(WORLD+0x10c,ENTITY);self.put(0x3a5bb1d8,ENGINE);self.put(ENGINE+0x84,ORIGIN);self.put(ORIGIN,OVT);self.put(OVT+0x24,0x5000)
  self.put(0x39affa1c,MANAGER);self.put(MANAGER+0x59c,HEAD);self.put(HEAD+4,DESC);self.put(DESC,HEAD,HEAD,HEAD,0);self.u.mem_write(HEAD+0x59,b'\x01');self.u.mem_write(DESC+0x59,b'\0')
  self.put(DESC+0x10+0x44,4) # descriptor's own anchor point CENTER
 def put(self,a,*v):self.u.mem_write(a,struct.pack('<'+'I'*len(v),*(x&0xffffffff for x in v)))
 def floats(self,a,*v):self.u.mem_write(a,struct.pack('<'+'f'*len(v),*v))
 def get(self,a):return struct.unpack('<I',self.u.mem_read(a,4))[0]
 def fg(self,a,n=1):return list(struct.unpack('<'+'f'*n,self.u.mem_read(a,n*4)))
 def args(self,n):return [self.get(self.u.reg_read(UC_X86_REG_ESP)+4+4*i)for i in range(n)]
 def ret(self,value=0,pop=0):
  sp=self.u.reg_read(UC_X86_REG_ESP);self.u.reg_write(UC_X86_REG_EAX,value);self.u.reg_write(UC_X86_REG_EIP,self.get(sp));self.u.reg_write(UC_X86_REG_ESP,sp+4+pop)
 def hook(self,u,a,n,_):
  if a==0x5000:
   xp,yp=self.args(2);self.floats(xp,self.fg(xp)[0]+self.origin[0]);self.floats(yp,self.fg(yp)[0]+self.origin[1]);self.ret(pop=8)
  elif a==0x390b3850:self.ret(PT)
  elif a==0x390b3870:self.ret(PT+0x100)
  elif a==0x39097bd0:self.ret(pop=4) # heading-only conversion outside projection
  elif a==0x39276a70:self.ret() # remove anchors, recorded anchor is supplied next
  elif a==0x39276dd0:
   point,target,rel,x,y,notify=self.args(6);self.anchors.append({'point':point,'target_is_Map':target==M,'relative_point':rel,'xy':list(struct.unpack('<2f',struct.pack('<2I',x,y)))});self.ret(pop=24)
  elif a==0x392af950 and self.kind=='player':u.emu_stop() # centre anchor complete; stop before heading/visibility paint state
  elif a==0x392aed20:self.ret(1,pop=16) # marker descriptor visibility admission is an explicit endpoint
  elif a==0x397f2180:self.static_requests.append({'kind':'zone','id':self.args(1)[0]});self.ret(ZROW)
  elif a==0x3976ade0:self.static_requests.append({'kind':'city','id':self.args(1)[0]});self.ret(CROW)
 def call(self,entry,*args,end=STOP):
  self.put(STACK,STOP,*args);self.u.reg_write(UC_X86_REG_ESP,STACK);self.u.reg_write(UC_X86_REG_ECX,M);self.u.reg_write(UC_X86_REG_FPCW,0x37f);self.u.reg_write(UC_X86_REG_MXCSR,0x1f80)
  try:self.u.emu_start(entry,end,count=200000)
  except Exception as e:raise RuntimeError((hex(entry),hex(self.u.reg_read(UC_X86_REG_EIP)),str(e)))from e
  assert self.u.reg_read(UC_X86_REG_EIP)==end,hex(self.u.reg_read(UC_X86_REG_EIP))
 def setup(self,rect,size,point):
  self.u.mem_write(M,bytes(0x900));self.floats(M+0x8c,*size);self.floats(M+0x2c8,*rect);self.floats(PT,*point,0.);self.put(M+0x324,DRAW);self.anchors=[]
 def project(self,rect,size,point,kind):
  self.setup(rect,size,point);self.kind=kind
  if kind=='direct':self.call(0x392ab830,PT,RESULT);return self.fg(RESULT,2)
  if kind=='player':
   self.floats(PT,point[0]-self.origin[0],point[1]-self.origin[1],0.)
   self.call(0x392af8b0,end=0x392af950);assert len(self.anchors)==1;return self.anchors[0]['xy']
  field={'npc':0x338,'given_quest':0x304}[kind];self.put(M+field,ARRAY,ARRAY+8);self.put(ARRAY,DRAW,ROW);self.floats(ROW,*point,0.);self.put(ROW+0xc,0)
  self.call(0x392b4b20);assert len(self.anchors)==1,self.anchors;return self.anchors[0]['xy']
 def select(self,zone,city):
  self.u.mem_write(ZROW,bytes(0x100));self.u.mem_write(CROW,bytes(0x100))
  self.floats(ZROW+0x38,zone[0],zone[1]);self.floats(ZROW+0x34,zone[2]);self.floats(ZROW+0x18,zone[3])
  self.floats(CROW+0x24,city[0],city[1]);self.floats(CROW+0x20,city[2]);self.floats(CROW,city[3])
  self.put(M+0x2b8,5);self.put(M+0x2bc,335);self.static_requests=[];out=[]
  for level in [2,3]:self.call(0x392ab770,RESULT,level);out.append({'level':level,'rect':self.fg(RESULT,4)})
  return {'rectangles':out,'lookups':self.static_requests}
if __name__=='__main__':
 m=Native(); data=json.loads(Path('tools/evidence/roadmap/projection/native-cases.json').read_text());count=0
 for row in data['cases']:
  for kind in ['direct','player','npc','given_quest']:
   actual=m.project(row['world_rect'],row['widget_extent'],row['world_point'],kind)
   assert actual==row['native_centres'][kind],(row['map'],row['size'],kind,actual)
   count+=1
 print(count,'original native projection paths matched retained cases')
