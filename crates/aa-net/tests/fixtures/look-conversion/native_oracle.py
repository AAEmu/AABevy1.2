"""Isolated pinned-PE execution; controlled fixtures/endpoints are not live admission.
Only writes this research directory. No OS loading, networking, process access or game asset access.
"""
from pathlib import Path
import hashlib,json,struct
import pefile
from unicorn import Uc,UC_ARCH_X86,UC_MODE_32,UC_HOOK_CODE
from unicorn.x86_const import *
from capstone import Cs,CS_ARCH_X86,CS_MODE_32
OUT=Path(__file__).resolve().parent
PATH=Path('<client12-unpacked>/x2game_unpacked.dll')
RAW=PATH.read_bytes(); SHA=hashlib.sha256(RAW).hexdigest()
assert SHA=='851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
PE=pefile.PE(data=RAW);BASE=PE.OPTIONAL_HEADER.ImageBase;IMAGE=PE.get_memory_mapped_image()
STACK,STOP=0x2001ff00,0x30001000
PACKET,ARCHIVE,VTABLE=0x10000000,0x10003000,0x10004000
GAME,ACTOR=0x10005000,0x10010000
MANAGER,BAG=0x3a5ad990,0x3a5ad9a0
class Native:
 def __init__(self):
  self.u=Uc(UC_ARCH_X86,UC_MODE_32);self.u.mem_map(BASE,(len(IMAGE)+4095)&~4095);self.u.mem_write(BASE,IMAGE)
  for a,n in [(0,0x10000),(0x10000000,0x80000),(0x20000000,0x20000),(0x30000000,0x10000)]:self.u.mem_map(a,n)
  self.state={};self.events=[];self.endpoints={};self.u.hook_add(UC_HOOK_CODE,self.hook)
  self.put(ARCHIVE,VTABLE)
  for o in [0x14,0x18,0x1c,0x3c,0x44,0x38,0x78,0xd0]:self.put(VTABLE+o,0x30000000+o)
 def put(self,a,*xs):self.u.mem_write(a,struct.pack('<'+'I'*len(xs),*[x&0xffffffff for x in xs]))
 def get(self,a):return struct.unpack('<I',self.u.mem_read(a,4))[0]
 def args(self,n):return [self.get(self.u.reg_read(UC_X86_REG_ESP)+4+i*4) for i in range(n)]
 def ret(self,v=0,pop=0):
  sp=self.u.reg_read(UC_X86_REG_ESP);self.u.reg_write(UC_X86_REG_EAX,v&0xffffffff);self.u.reg_write(UC_X86_REG_EIP,self.get(sp));self.u.reg_write(UC_X86_REG_ESP,sp+4+pop)
 def hook(self,u,a,n,_):
  if a in self.endpoints:self.endpoints[a]();return
  if a==0x391b8780:
   ptr=u.reg_read(UC_X86_REG_ECX)
   if self.state['read']:
    self.u.mem_write(ptr,self.state['wire'][self.state['pos']:self.state['pos']+3]);self.state['pos']+=3
   else:self.state['out']+=bytes(self.u.mem_read(ptr,3))
   self.ret(pop=4);return
  if 0x30000000<=a<0x30000100:
   off=a-0x30000000
   if off==0x1c:self.ret(self.state.get('read',0));return
   if off==0x14:self.ret(1,pop=8);return
   if off==0x18:self.ret();return
   name,ptr,extra=self.args(3);size=extra if off==0xd0 else {0x3c:4,0x44:1,0x38:8,0x78:1}[off]
   if self.state['read']:
    value=self.state['wire'][self.state['pos']:self.state['pos']+size];assert len(value)==size
    self.u.mem_write(ptr,value);self.state['pos']+=size
   else:self.state['out']+=bytes(self.u.mem_read(ptr,size))
   self.ret(pop=12);return
  if a==0x398b562a:
   dest,value,size=self.args(3);self.u.mem_write(dest,bytes([value&255])*size);self.ret(dest);return
 def call(self,a,this,*args):
  self.put(STACK,STOP,*args);self.u.reg_write(UC_X86_REG_ESP,STACK);self.u.reg_write(UC_X86_REG_ECX,this)
  try:self.u.emu_start(a,STOP,count=200000)
  except Exception as e:raise RuntimeError((hex(a),hex(self.u.reg_read(UC_X86_REG_EIP)),str(e))) from e
  assert self.u.reg_read(UC_X86_REG_EIP)==STOP,hex(self.u.reg_read(UC_X86_REG_EIP))
  return self.u.reg_read(UC_X86_REG_EAX)
 def archive(self,a,reading=False,wire=b''):
  self.state.update(read=int(reading),wire=wire,pos=0,out=b'');self.call(a,PACKET,ARCHIVE)
  return self.state['out']
 def bag(self):
  self.u.mem_write(MANAGER,bytes(0xc400));self.u.mem_write(BAG+8,bytes([2,10,1]))
  sentinel=0x10040000;self.put(MANAGER+0xc34c,sentinel);self.put(sentinel+4,sentinel);self.u.mem_write(sentinel+0x11,b'\x01')
 def item(self,slot,identity,template,grade=1,made=0):
  a=BAG+0x10+slot*0xc0;self.u.mem_write(a,bytes(0xc0));self.u.mem_write(a,struct.pack('<QIBBi',identity,template,grade,0,1));self.put(a+0xa4,made);return a
 def convert(self,name,base,look,missing=False,blocked=False):
  self.bag();self.events=[];self.endpoints={}
  self.put(0x39adcf20,1);self.put(0x39adcf14,0x03000200,0x04000200)
  self.put(0x39b02294,GAME)
  self.item(3,base,8001);self.item(4,look,8002)
  if missing:self.u.mem_write(BAG+9,b'\x04')
  self.endpoints[0x397cc350]=lambda:self.ret(int(blocked))
  def check_combination():
   self.put(self.args(4)[2],1);self.events.append({'endpoint':'combination397d0500','fixture_result':0});self.ret(0)
  self.endpoints[0x397d0500]=check_combination
  self.endpoints[0x397f2850]=lambda:self.ret(0x10030000)
  self.endpoints[0x397cf380]=lambda:self.ret(1,pop=4)
  def send():
   p=self.args(1)[0];self.events.append({'endpoint':'send39186810','vtable':hex(self.get(p)),'opcode':self.get(p+4),'body':bytes(self.u.mem_read(p+0x10,16)).hex()});self.ret(pop=4)
  self.endpoints[0x39186810]=send
  def error():self.events.append({'endpoint':'error39428370','code':self.args(1)[0]});self.ret()
  self.endpoints[0x39428370]=error
  self.call(0x39047650,0x39adcf14)
  return {'case':name,'base_id':base,'look_id':look,'trace':list(self.events)}
 def cursor(self,skill,source_template,source_grade=6,target_grade=7):
  self.endpoints={};self.bag();self.events=[]
  source_id,target_id=0x0102030405060708,0x1112131415161718
  self.item(3,source_id,source_template,source_grade,0x66778899);self.item(4,target_id,0xaabbccdd,target_grade)
  self.put(0x39b02294,GAME);self.put(GAME+0x10c,ACTOR);self.put(ACTOR+8,0x010203)
  # Supplied native runtime cursor/caster state, not an asserted skill admission result.
  self.put(0x3a5a4ea0,4);self.put(0x3a5a4eb0,skill)
  c=bytearray(32);struct.pack_into('<I',c,0,2);struct.pack_into('<Q',c,8,source_id);struct.pack_into('<II',c,16,source_template,0x010203);c[24]=source_grade;struct.pack_into('<I',c,28,0x66778899)
  self.u.mem_write(0x3a5a4eb8,bytes(c));self.put(0x39ac97a0,0xf001);self.put(0x39ac97a8,0xf002);self.put(0x39ac97ac,0xf003);self.put(0x39ac97b8,0xf004)
  captured={}
  def planner():
   output,actor,skill,caster,target,obj,flags=self.args(7)
   captured.update(actor=actor,skill=skill,caster=bytes(self.u.mem_read(caster,32)).hex(),target=bytes(self.u.mem_read(target,56)).hex(),object=bytes(self.u.mem_read(obj,0xd8)).hex(),flags=flags)
   self.u.mem_write(output,bytes(8));self.ret(output,pop=28)
  self.endpoints[0x39396e20]=planner
  self.call(0x3942fb50,0,0x04000200)
  assert captured and captured['skill']==skill
  self.u.mem_write(PACKET,bytes(512));self.put(PACKET+0xc,skill);self.u.mem_write(PACKET+0x10,bytes.fromhex(captured['caster']));self.u.mem_write(PACKET+0x30,bytes.fromhex(captured['target']));self.u.mem_write(PACKET+0x68,bytes.fromhex(captured['object']))
  out=self.archive(0x397cad20)
  expected=struct.pack('<IB',skill,2)+bytes.fromhex('030201')+struct.pack('<QIBI',source_id,source_template,source_grade,0x66778899)+bytes.fromhex('03030201')+struct.pack('<QIBB',target_id,0xaabbccdd,target_grade,0)
  assert out==expected,(out.hex(),expected.hex())
  return {'skill':skill,'source_template':source_template,'native_cursor_after':self.get(0x3a5a4ea0),'planner_input':captured,'serialized_body':out.hex(),'length':len(out)}
if __name__=='__main__':
 n=Native();convert=[]
 for base,look in [(0,0),(0x0102030405060708,0x1112131415161718),(0xffffffffffffffff,0x8877665544332211)]:
  n.u.mem_write(PACKET,bytes(64));n.call(0x397abb40,PACKET,base&0xffffffff,base>>32,look&0xffffffff,look>>32)
  wire=n.archive(0x397b0950);assert wire==struct.pack('<QQ',base,look)
  n.u.mem_write(PACKET+0x10,bytes(16));n.archive(0x397b0950,True,wire);assert bytes(n.u.mem_read(PACKET+0x10,16))==wire
  convert.append({'base_id':base,'look_id':look,'wire':wire.hex()})
 routes=[n.convert('selected_slot_ids',0x0102030405060708,0x1112131415161718),n.convert('selected_slots_reread_new_ids',0x2122232425262728,0x3132333435363738),n.convert('selected_image_slot_missing',7,8,missing=True),n.convert('pending_item_rejected',7,8,blocked=True)]
 assert [x for x in routes[0]['trace'] if x['endpoint']=='send39186810'][0]['body']==convert[1]['wire']
 assert not [x for x in routes[2]['trace'] if x['endpoint']=='send39186810']
 assert not [x for x in routes[3]['trace'] if x['endpoint']=='send39186810']
 cursors=[n.cursor(23457,29704),n.cursor(23741,29880)]
 result={'module':str(PATH),'sha256':SHA,'image_base':hex(BASE),'qualification':{'convert_codec':'Native397abb40 constructor and397b0950 serializer executed read/write. Archive u64 primitive is explicit controlled endpoint; raw zeros/max values are serializer fixtures, not admissible UI requests.','convert_route':'Native39047650 selected physical slots, native390474a0 validation order,3961e850/3961e700 physical lookup and3961eaf0/3961e9a0 ID lookup execute. Pending397cc350, combination397d0500, descriptor397f2850, money397cf380 and send/error endpoints are controlled. Requirement vector is explicitly empty and money allowed; no catalog/planner admission claim. Native send captured before network.','cursor_route':'Native3942fb50 executes, including actual empty pending-map lookup390457b0/39045820, actual397cce50 physical target and397ccfb0 source lookup, clearing selection and constructing full target and zero skill object. Only39396e20 protected ordinary planner is intercepted. Captured structs are then passed to native397cad20/397c85b0/397bfa90/397bfd50 serializer independently. This proves input/wire mapping, not execution of the protected planner, skill admission, network send or receipt correlation.','other_endpoints':'Archive primitives, BC391b8780 raw3-byte wire copy, CRT memset398b562a are explicit mocks; no native process, OS LoadLibrary, socket or server used.','runtime_fixtures':'Game/actor/cursor, singleton initialized flag, physical BAG header/configuration, complete item records, empty pending map, generic unrelated mode sentinel skill IDs are supplied fixtures; no live global-value claims.'},'convert_codec_cases':convert,'convert_native_routes':routes,'cursor_native_routes':cursors}
 (OUT/'native-goldens.json').write_text(json.dumps(result,indent=2)+'\n')
 for name,row in zip(['convert_zero','convert','convert_high'],convert):
  (OUT/(name+'.bin')).write_bytes(bytes.fromhex(row['wire']))
 for name,row in zip(['cleanup','skinize'],cursors):
  (OUT/(name+'.bin')).write_bytes(bytes.fromhex(row['serialized_body']))
 # Retain exact relevant native instruction bytes and labels for repeatable static audit.
 ranges=[(0x390474a0,0x1a2),(0x39047650,0x46),(0x39066780,0x1b),(0x39067969,0x6e),(0x397abb40,0x1f),(0x397b0950,0x33),(0x3942fb50,0x202),(0x39396e20,5),(0x394815a0,0x80),(0x39481720,0x90),(0x397cfd40,0xa0),(0x397cdc10,0x80),(0x397cdc90,0x70)]
 md=Cs(CS_ARCH_X86,CS_MODE_32);lines=[str(PATH),'SHA256 '+SHA]
 for a,size in ranges:
  lines.append('\nRANGE '+hex(a)+' size '+hex(size))
  for i in md.disasm(IMAGE[a-BASE:a-BASE+size],a):lines.append(f'{i.address:08x}: {i.bytes.hex():24} {i.mnemonic} {i.op_str}')
 (OUT/'native-disassembly.txt').write_text('\n'.join(line.rstrip() for line in lines)+'\n')
 print('PASS: 3 convert read/write goldens; 4 converter caller cases; 2 cursor caller + serializer cases.')
 for row in cursors:print(row['skill'],row['length'],row['serialized_body'])
