from pathlib import Path
import sys,site,hashlib,json,struct
sys.path.insert(0,site.getusersitepackages())
import pefile
from unicorn import Uc,UC_ARCH_X86,UC_MODE_32,UC_HOOK_CODE
from unicorn.x86_const import *
from capstone import Cs,CS_ARCH_X86,CS_MODE_32
OUT=Path('data/extract/aa-ui/minimap-audit')
STOP,STUB,STACK=0x3000fff0,0x30000000,0x201f0000
ROOT=0x39d29108+5*36
HOST,SYS,FH,TABLE=0x10010000,0x10011000,0x10012000,0x10013000
HVT,SVT,FVT,TVT=0x10014000,0x10015000,0x10016000,0x10017000
BUFFER,VALUE=0x10020000,0x10030000
class Native:
 def __init__(self):
  self.u=Uc(UC_ARCH_X86,UC_MODE_32);self.images={};self.provenance={};self.imports={}
  for a,s in [(0,0x10000),(0x10000000,0x800000),(0x20000000,0x200000),(STUB,0x10000)]:self.u.mem_map(a,s)
  self.next_stub=STUB
  for name in ['x2game_unpacked.dll','bin/xlcommon.dll']:
   p=Path('D:/aa-re/client12')/name;raw=p.read_bytes();pe=pefile.PE(data=raw);base=pe.OPTIONAL_HEADER.ImageBase;data=pe.get_memory_mapped_image()
   self.u.mem_map(base,(len(data)+4095)&~4095);self.u.mem_write(base,data);self.images[name]=(base,data,pe)
   self.provenance[name]=hashlib.sha256(raw).hexdigest()
   for e in pe.DIRECTORY_ENTRY_IMPORT:
    for i in e.imports:
     dest=self.next_stub;self.next_stub+=16;self.put(i.address,dest);self.imports[dest]=(e.dll.decode(),(i.name or b'ordinal').decode())
  assert self.provenance['x2game_unpacked.dll']=='851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
  assert self.provenance['bin/xlcommon.dll']=='0e0881aa837553d7e307a5c6f9d886f3e82a0d72d94b6d666448aedaa94e2203'
  _,_,common=self.images['bin/xlcommon.dll'];exports={s.name:0x33000000+s.address for s in common.DIRECTORY_ENTRY_EXPORT.symbols}
  # Resolve x2game's Data imports to the actual original implementation.
  _,_,game=self.images['x2game_unpacked.dll']
  for e in game.DIRECTORY_ENTRY_IMPORT:
   for i in e.imports:
    if e.dll.lower()==b'xlcommon.dll' and i.name in exports:self.put(i.address,exports[i.name])
  self.endpoints={};self.heap=0x10100000;self.sizes={};self.used=[];self.trace=[];self.tables={};self.returned=None;self.reads=[];self.steps=0
  for address,fn in [(0x5000,self.create_table),(0x5010,self.ref),(0x5020,self.set_field),(0x5030,self.get_field),(0x5040,self.no_results),(0x5050,self.results)]:self.endpoints[address]=fn
  self.put(HOST+0x48,SYS);self.put(SYS,SVT);self.put(SVT+0x30,0x5000)
  self.put(FH,FVT);self.put(FVT+0x2c,0x5040);self.put(FVT+0x28,0x5050)
  self.put(TABLE,TVT)
  for off,addr in [(4,0x5010),(8,0x5010),(0x14,0x5020),(0x18,0x5030)]:self.put(TVT+off,addr)
  self.u.hook_add(UC_HOOK_CODE,self.hook)
 def put(self,a,*v):self.u.mem_write(a,struct.pack('<'+'I'*len(v),*(x&0xffffffff for x in v)))
 def get(self,a):return struct.unpack('<I',self.u.mem_read(a,4))[0]
 def args(self,n):return [self.get(self.u.reg_read(UC_X86_REG_ESP)+4+i*4) for i in range(n)]
 def ret(self,v=0,pop=0):
  sp=self.u.reg_read(UC_X86_REG_ESP);self.u.reg_write(UC_X86_REG_EAX,v&0xffffffff);self.u.reg_write(UC_X86_REG_EIP,self.get(sp));self.u.reg_write(UC_X86_REG_ESP,sp+4+pop)
 def string(self,p):
  if not p:return None
  return bytes(self.u.mem_read(p,8192)).split(b'\0')[0].decode('utf-8',errors='backslashreplace')
 def alloc(self,n):
  a=self.heap;self.heap+=(max(n,1)+15)&~15;assert self.heap<0x10700000;self.u.mem_write(a,bytes(n));self.sizes[a]=n;return a
 def create_table(self):self.tables[TABLE]={};self.ret(TABLE,pop=8)
 def ref(self):self.ret()
 def set_field(self):
  key,ptr,chain=self.args(3);tag=self.get(ptr);bits=self.get(ptr+4);name=self.string(key)
  if tag==4:value=struct.unpack('<f',struct.pack('<I',bits))[0]
  elif tag==2:value=bool(bits&255)
  else:raise AssertionError(('unexpected script output',tag))
  self.tables[TABLE][name]=value;self.trace.append({'set':name,'type':tag,'value':value,'value_bits':f'{bits:08x}'})
  self.ret(1,pop=12)
 def get_field(self):
  key,ptr,chain=self.args(3);name=self.string(key);value=self.tables[TABLE].get(name,'__MISSING__');req=self.get(ptr)
  self.reads.append({'key':name,'requested_type':req,'value':repr(value)})
  if value=='__MISSING__':self.ret(0,pop=12);return
  tag=2 if type(value)is bool else 4 if type(value)in(int,float) else 5 if type(value)is str else 6 if type(value)is dict else 0
  bits=int(value) if tag==2 else struct.unpack('<I',struct.pack('<f',value))[0] if tag==4 else 0
  self.put(ptr,tag,bits);self.ret(1,pop=12)
 def no_results(self):self.returned=None;self.ret(0)
 def results(self):
  ptr,=self.args(1);assert self.get(ptr)==6;self.returned=dict(self.tables[self.get(ptr+4)]);self.ret(1,pop=4)
 def hook(self,uc,a,size,_):
  self.steps+=1
  if a in self.endpoints:self.endpoints[a]();return
  if a in [0x39217640,0x39186810,0x397acbb0]:raise AssertionError(('unexpected SAVE or packet endpoint',hex(a)))
  if a in [0x39443b52,0x39443b63,0x39443b6e]:
   bp=self.u.reg_read(UC_X86_REG_EBP);self.trace.append({'pc':hex(a),'size_local':self.get(bp-0x18),'npc_bool_byte':bytes(self.u.mem_read(bp-0xe,1))[0]})
  if a in self.imports:
   dll,name=self.imports[a];args=self.args(4);self.used.append(dll+'!'+name)
   if name in ['memcpy','memmove']:self.u.mem_write(args[0],bytes(self.u.mem_read(args[1],args[2])));self.ret(args[0]);return
   if name=='memset':self.u.mem_write(args[0],bytes([args[1]&255])*args[2]);self.ret(args[0]);return
   if name=='?_Orphan_all@_Container_base0@std@@QAEXXZ':self.ret();return
   if name=='strlen':self.ret(len(self.string(args[0]).encode()));return
   raise RuntimeError(('unmocked import',dll,name,hex(a)))
  if a in [0x33021f30,0x33021f40]:self.ret(self.alloc(self.args(1)[0]));return
  if a==0x33021650:
   old,n=self.args(2);new=self.alloc(n)
   if old:self.u.mem_write(new,bytes(self.u.mem_read(old,min(n,self.sizes.get(old,0)))))
   self.ret(new);return
  if a in [0x33021f50,0x33021f60,0x33021980]:self.ret();return
 def call(self,a,this,*args,fill=0):
  self.u.mem_write(STACK-0x10000,bytes([fill])*0x10000);self.put(STACK,STOP,*args)
  self.u.reg_write(UC_X86_REG_ESP,STACK);self.u.reg_write(UC_X86_REG_ECX,this);self.u.reg_write(UC_X86_REG_FPCW,0x037f)
  try:self.u.emu_start(a,STOP,count=1000000)
  except Exception as e:raise RuntimeError((hex(a),hex(self.u.reg_read(UC_X86_REG_EIP)),str(e))) from e
  assert self.u.reg_read(UC_X86_REG_EIP)==STOP,hex(self.u.reg_read(UC_X86_REG_EIP))
  return self.u.reg_read(UC_X86_REG_EAX)
 def load(self,text):
  self.u.mem_write(ROOT,bytes(36));self.u.mem_write(BUFFER,text.encode()+b'\0');self.call(0x33004af0,ROOT,BUFFER,len(text.encode()))
 def dump(self,a=ROOT):
  child=self.get(a+4);children=[]
  while child:
   children.append(self.dump(child));child=self.get(child+8);assert len(children)<50
  return {'value':self.string(self.get(a)),'children':children}
 def serialize(self):
  self.u.mem_write(BUFFER,bytes(8192));self.call(0x33002ce0,ROOT,BUFFER,8192);return self.string(BUFFER)
 def getter(self,text,fill=0):
  self.load(text);self.trace=[];self.returned='__NOT_RETURNED__';self.call(0x39443a70,HOST,FH,fill=fill)
  return {'payload':text,'stack_fill':fill,'result':self.returned,'trace':list(self.trace)}
 def setter(self,values,seed='roadmap_option\n  old 99\nother 17\n'):
  self.load(seed);self.tables[TABLE]=values;self.reads=[];self.trace=[];self.call(0x39443bc0,HOST,FH,TABLE)
  return {'input':values,'seed':seed,'tree':self.dump(),'reads':list(self.reads),'trace':list(self.trace),'result':self.returned,'serialized':self.serialize()}
if __name__=='__main__':
 m=Native();getters=[];setters=[];bools=[]
 for label,payload in [('missing','other 7\n'),('empty','roadmap_option\n'),('size_only','roadmap_option\n  size 2\n'),('visible_only','roadmap_option\n  visible false\n'),('npc_only','roadmap_option\n  npcShow false\n'),('size_visible','roadmap_option\n  size 2\n  visible true\n'),('size_npc','roadmap_option\n  size 2\n  npcShow false\n'),('unknown_child','roadmap_option\n  old 8\n')]:
  fills=[0,0x55,0xa5] if label=='npc_only' else [0]
  for fill in fills:getters.append({'case':label,**m.getter(payload,fill)})
 for token in ['1.9','2junk','garbage','-2','']:
  payload='roadmap_option\n  size '+('\"\"' if not token else token)+'\n  npcShow false\n'
  getters.append({'case':'Data_int_'+repr(token),**m.getter(payload)})
 for size in [0,1,2,3,-1]:
  for visible in [False,True]:
   for npc in [False,True]:
    payload=f'roadmap_option\n  size {size}\n  visible {str(visible).lower()}\n  npcShow {str(npc).lower()}\n'
    row=m.getter(payload);row['case']='complete';getters.append(row)
    assert row['result']=={'size':float(size),'visible':visible,'npcShow':float(size)}
 for values in [{},{'size':1},{'visible':False},{'npcShow':True}]+[{'size':s,'visible':v,'npcShow':n}for s in [1,2,3]for v in [False,True]for n in [False,True]]:
  setters.append({'case':'ordinary',**m.setter(values)})
 for field in ['size','visible','npcShow']:
  for label,value in [('bool_true',True),('bool_false',False),('number0',0),('number1',1),('number2',2),('number_fraction',2.9),('number_negative',-1.9),('string_true','true'),('string_1','1'),('nil',None),('table',{})]:
   setters.append({'case':'typed_'+field+'_'+label,**m.setter({field:value})})
 setters.append({'case':'missing_section_created',**m.setter({'size':3,'visible':False,'npcShow':False},'other 17\n')})
 for value in ['true','false','1','0','TRUE','False','yes','-1','2','', 'garbage']:
  text='roadmap_option\n  npcShow '+('""' if not value else value)+'\n'
  m.load(text);key=VALUE+0x100;m.u.mem_write(key,b'roadmap_option\0');section=m.call(0x33002dc0,ROOT,key)
  m.u.mem_write(key,b'npcShow\0');m.put(VALUE,0xaaaaaaaa)
  success=m.call(0x33005210,section,key,VALUE)&255
  bools.append({'token':value,'has_bool':bool(success),'out_byte':m.get(VALUE)&255})
 result={'provenance':{'images':m.provenance,'executed':['39443a70 getter','39443bc0 setter','390309f0 number table write','39009e40 boolean table write','39001d30 numeric table read','39058b50 return table','39217510 category5','xlcommon Data Load/Clear/ChildWithValue/HasIntAttribute/HasBoolAttribute/SetAttribute/SaveToBuffer and their native helpers'], 'controlled_endpoints':['script table allocation/AddRef/Release, raw ScriptAny field get/set and return delivery','heap allocation/reallocation/free','CRT memcpy/memmove/memset/strlen and std iterator orphan bookkeeping'], 'save_or_packet_entries':'39217640/39186810/397acbb0 fail if entered; none entered','stack_fills':'zero, 0x55 and 0xa5 are explicit fixtures, not live stack observations'},'getter_cases':getters,'setter_cases':setters,'data_bool_read_cases':bools,'used_import_endpoints':sorted(set(m.used))}
 (OUT/'roadmap-native-cases.json').write_text(json.dumps(result,indent=2)+'\n',encoding='utf-8')
 print('getter',len(getters),'setter',len(setters),'Data bool',len(bools))
 for row in getters:
  if row['case']!='complete':print(row['case'],row['stack_fill'],row['result'])
 print('typed setters and bool lexemes recorded')
