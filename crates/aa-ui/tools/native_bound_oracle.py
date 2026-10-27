"""Offline original ArcheAge 1.2 event-33 registration and delivery oracle.
Actual name lookup, handler storage/replacement, size bindings/bodies, event
marshalling, anchor readiness/geometry, chat wrappers and dependency propagation
execute under Unicorn. Alloc/free/memmove, imported string comparison, script VM
primitives, host casts, chat layout, clip-cache and final scale geometry are
controlled endpoints. Callback actions reenter actual native code. No live
client, assets, configuration, database, network or RE project is modified.
Requires Python pefile, capstone, unicorn and read-only <re-notes> binaries.
"""
import hashlib,json,pathlib,struct
import pefile
from capstone import Cs,CS_ARCH_X86,CS_MODE_32
from unicorn import Uc,UC_ARCH_X86,UC_MODE_32,UC_HOOK_CODE
from unicorn.x86_const import *
path=pathlib.Path('<client12-unpacked>/x2game_unpacked.dll')
p=pefile.PE(str(path));base=p.OPTIONAL_HEADER.ImageBase;data=p.get_memory_mapped_image()
u=Uc(UC_ARCH_X86,UC_MODE_32)
STOP,STUB,STACK,D=0x4000,0x5000,0x20f000,0x100000
SYS,SVT,FH,FVT,HOST,HVT=0x108000,0x109000,0x10a000,0x10b000,0x10c000,0x10d000
W,T,C,G,OTHER=[0x110000+i*0x1000 for i in range(5)]
for a,n in [(0,0x10000),(D,0x100000),(0x200000,0x10000),(base,(len(data)+4095)&~4095)]:u.mem_map(a,n)
u.mem_write(base,data)
cry_path=pathlib.Path('<client12-unpacked>/bin/cryscriptsystem.dll')
cry=pefile.PE(str(cry_path));u.mem_map(cry.OPTIONAL_HEADER.ImageBase,(cry.OPTIONAL_HEADER.SizeOfImage+4095)&~4095)
u.mem_write(cry.OPTIONAL_HEADER.ImageBase,cry.get_memory_mapped_image())
u.mem_write(0x6100,b'\xdd\x05'+struct.pack('<I',0x10e000)+b'\xc3')
def put(a,f,*v):u.mem_write(a,struct.pack('<'+f,*v))
def get(a,f):return struct.unpack('<'+f,u.mem_read(a,struct.calcsize('<'+f)))
def finish(value=0,args=0):
 sp=u.reg_read(UC_X86_REG_ESP);u.reg_write(UC_X86_REG_EAX,value);u.reg_write(UC_X86_REG_EIP,get(sp,'I')[0]);u.reg_write(UC_X86_REG_ESP,sp+4+args)
def reenter(entry,obj,args):
 reenter_all([(entry,obj,args)])
def reenter_all(sequence):
 # Preserve the host script-EndCall ABI around a real nested native invocation.
 code=b'\x60'
 for entry,obj,args in sequence:
  code+=b''.join(b'\x68'+struct.pack('<I',a)for a in reversed(args))+b'\xb9'+struct.pack('<I',obj)+b'\xb8'+struct.pack('<I',entry)+b'\xff\xd0'
 code+=b'\x61\xb8\x01\x00\x00\x00\xc3'
 u.mem_write(STUB,code);u.ctl_remove_cache(STUB,STUB+len(code));u.reg_write(UC_X86_REG_EIP,STUB)
heap=0x140000;delivered=[];released=[];callback_actions={};events=[];script_args=[];params=[];arguments=[];cast=True
def hook(uc,a,n,_):
 global heap,script_args
 sp=uc.reg_read(UC_X86_REG_ESP)
 if a==STOP:uc.emu_stop()
 elif a==0x3950a1b0:
  size=get(sp+4,'I')[0];out=heap;heap+=(size+15)&~15;u.mem_write(out,bytes(size));finish(out)
 elif a==0x3950a1f0:finish()
 elif a==0x6000:
  out,src,size=get(sp+4,'3I');u.mem_write(out,bytes(u.mem_read(src,size)));finish(out)
 elif a in [0x392c97e0,0x3927a520]:finish() # chat layout and clip-cache endpoints; event/propagation run
 elif a==0x6010:finish(SYS) # receiver's script table -> script system
 elif a==0x6020:finish(1) # scripting enabled
 elif a==0x6030:
  script_args=[];delivered.append(get(sp+4,'I')[0]);finish(1,args=4) # BeginCall
 elif a==0x6040:
  anyptr=get(sp+4,'I')[0];script_args.append(list(get(anyptr,'2I')));finish(args=4)
 elif a==0x6050:
  # Original event helper pushed only the receiver table; EndCall has no arguments.
  assert len(script_args)==1 and script_args[0][0]==6,script_args
  receiver=script_args[0][1];event=get(SYS+0x100,'I')[0]
  events.append([receiver,event,list(get(receiver-0x800+0x84,'4f'))])
  pending=callback_actions.get((receiver-0x800,event),[]);action=pending.pop(0) if pending else None
  if isinstance(action,list):reenter_all(action)
  elif action:reenter(*action)
  else:finish(1)
 elif a==0x6060:finish(args=0) # script table AddRef/Release
 elif a==0x6070:finish(HOST)
 elif a==0x6080:finish(W if cast else 0)
 elif a==0x6090:finish(len(arguments))
 elif a==0x60c0:finish(0,args=4)
 elif a==0x35546ae0:finish(len(arguments))
 elif a==0x35546cc0:
  index=get(sp+8,'i')[0];params.append(index);v=arguments[index-1]if 1<=index<=len(arguments)else None
  finish(0 if v is None else 1 if isinstance(v,bool)else 3 if isinstance(v,(int,float))else 4)
 elif a==0x35546ed0:
  index=get(sp+8,'i')[0];put(0x10e000,'d',arguments[index-1]);u.reg_write(UC_X86_REG_EIP,0x6100)
 elif a==0x35546f60:finish(int(arguments[get(sp+8,'i')[0]-1]))
 elif a in [0x3550b450,0x3550ac50]:finish()
 elif a==0x392758a0:finish(args=20) # final clip-cache geometry endpoint
 elif a==0x60d0:finish() # chat-specific extent continuation
 elif a==0x60b0:finish() # zero Lua results
 elif a==0x3927b8b0:put(SYS+0x100,'I',get(sp+4,'I')[0])
 elif a==0x60e0:
  left,right=get(sp+4,'2I');s=lambda p:bytes(u.mem_read(p,128)).split(b'\0')[0]
  finish(0 if s(left)==s(right) else 1) # imported XlStringCompare endpoint
 elif a==0x60f0:released.append(get(sp+4,'I')[0]);finish(args=4)
u.hook_add(UC_HOOK_CODE,hook)
def setup():
 global heap,cast
 u.mem_write(D,bytes(0x100000));events.clear();delivered.clear();released.clear();callback_actions.clear();params.clear();arguments.clear();heap=0x140000;cast=True
 u.reg_write(UC_X86_REG_FPCW,0x037f)
 put(0x38ff6c60,'I',0x60e0);put(SVT+0x5c,'I',0x60f0);put(0x38ff6700,'I',0x6000);put(0x38ff65e4,'I',0x6060)
 put(SYS,'2I',SVT,0x10f000);put(FH+8,'I',SYS)
 for off,entry in [(0xe0,0x6020),(0x40,0x6030),(0x60,0x6040),(0x44,0x6050)]:put(SVT+off,'I',entry)
 put(FH,'I',FVT)
 for off,entry in [(4,0x6070),(0x10,0x6090),(0x14,0x60c0),(0x18,0x3550a460),(0x2c,0x60b0)]:put(FVT+off,'I',entry)
 put(HOST,'I',HVT);put(HVT+4,'I',0x6080)
 for o in [W,T,C,G,OTHER]:
  # Copy original script-chat VT; only +164 chat-specific extent continuation is controlled.
  u.mem_write(o+0xa00,data[0x399e7400-base:0x399e7400-base+0x200]);put(o+0xa00+0x164,'I',0x60d0)
  put(o,'I',o+0xa00);put(o+0x64,'2f',1,1);put(o+0x74,'2f',100,60);put(o+0x8c,'2f',100,60)
  put(o+0x14,'I',o+0x800);put(o+0x800,'I',o+0x900)
  for off,entry in [(0,0x6010),(4,0x6060),(8,0x6060)]:put(o+0x900+off,'I',entry)
  put(o+0x184+32*4,'I',0xabc020);put(o+0x184+33*4,'I',0xabc021)
  put(o+0x184+42*4,'I',0xabc02a)
def call(entry,obj=W,args=()):
 put(STACK,str(len(args)+1)+'I',STOP,*args);u.reg_write(UC_X86_REG_ESP,STACK);u.reg_write(UC_X86_REG_ECX,obj)
 u.emu_start(entry,0,count=1000000)
 assert u.reg_read(UC_X86_REG_EIP)==STOP,hex(u.reg_read(UC_X86_REG_EIP))
def f(v):return struct.unpack('<I',struct.pack('<f',v))[0]
def anchor(obj,target,point=0,rel=0,x=0,y=0):call(0x39276dd0,obj,[point,target,rel,f(x),f(y),1])
def trace():return [[['W','T','C','G','O'][[W,T,C,G,OTHER].index(o-0x800)],event,rect]for o,event,rect in events]
def clear():events.clear();delivered.clear()

def expect(name, expected):
 actual=trace();assert [row[:2]for row in actual]==expected,(name,actual,expected)
 cases[name]=actual

def watch(obj,event,sequence):callback_actions.setdefault((obj,event),[]).append(sequence)

def registration(obj,name,handle):
 u.mem_write(0x13d000,name.encode()+b'\0');call(0x392792e0,obj,[0x13d000,handle])

cases={}
setup();put(W+0x184+33*4,'I',0)
registration(W,'OnBoundChanged',0xabc021)
assert get(W+0x184+33*4,'I')[0]==0xabc021 and not events
cases['registration_is_event33_and_silent']={'handler':get(W+0x184+33*4,'I')[0],'events':trace()}
registration(W,'OnBoundChanged',0xabc121)
assert released==[0xabc021] and not events
cases['replacement_is_silent']={'released':released.copy(),'events':trace()}
call(0x392763c0,W,[f(100),1]);assert delivered[-1]==0xabc121
cases['replacement_used_by_same_value_setter']=trace()
registration(W,'OnBoundChanged',0);clear();call(0x392763c0,W,[f(100),1])
expect('removed_handler_has_no_later_delivery',[['W',32]])

for entry,label,args in [(0x392763c0,'width',[f(100)]),(0x39276410,'height',[f(60)]),(0x39276460,'extent',[f(100),f(60)])]:
 for notify in [0,1]:
  setup();anchor(C,W);anchor(G,C);anchor(OTHER,W);clear()
  call(entry,W,args+[notify]);expect(f'identical_{label}_notify{notify}',
   [['G',32],['C',32],['O',32],['W',32],['W',33],['C',33],['G',33],['O',33]]if notify else [])

for entry,label,values,nativeargs in [(0x39261740,'width',[100],[f(100)]),(0x392617c0,'height',[60],[f(60)]),(0x39261840,'extent',[100,60],[f(100),f(60)])]:
 for control in [False,True,0,1,None,'invalid']:
  setup();arguments.extend(values+[control]);call(entry,HOST,[FH]+nativeargs)
  expected=[['W',32],['W',33]] if (label=='extent' or control not in [False,0])else []
  expect(f'binding_{label}_{control}',expected)
setup();arguments.extend([100,0,True]);call(0x39261840,HOST,[FH,f(100),0]);expect('extent_zero_height_silent',[])

setup();anchor(OTHER,W);anchor(C,W);anchor(G,OTHER);anchor(G,C,point=3,rel=3);clear()
call(0x392763c0,W,[f(101),1])
expect('diamond_repeats_bound_in_preorder',[['G',32],['O',32],['G',32],['C',32],['W',32],['W',33],['O',33],['G',33],['C',33],['G',33]])
setup();anchor(C,W);record=get(C+0x54,'I')[0];put(record+4,'B',0);clear()
call(0x39512990,W);expect('bound_continuation_ignores_anchor_readiness',[['W',33],['C',33]])

setup();anchor(C,W);clear();anchor(W,T,x=30,y=40)
expect('position_anchor_does_not_emit_bound',[['C',32],['W',32]])
clear();anchor(W,T,x=30,y=40);expect('identical_anchor_does_not_emit_bound',[['C',32],['W',32]])
clear();call(0x39276a70,W);expect('remove_anchors_silent',[])
setup();anchor(C,W);clear();call(0x39276a80,W);expect('detach_notifies_anchor_only',[['C',32]])
setup();anchor(C,W);anchor(C,W,point=3,rel=3);clear();call(0x392763c0,W,[f(150),1])
expect('parent_resize_fills_anchored_child',[['C',32],['W',32],['W',33],['C',33]])
assert get(C+0x8c,'2f')==(150.,60.)
setup();put(W+0xc8,'2I',W+0xe00,W+0xe04);put(W+0xe00,'I',C);put(C+0xc0,'I',W);clear()
call(0x392763c0,W,[f(150),1]);expect('unanchored_owned_child_not_bound_dependent',[['W',32],['W',33]])

setup();anchor(C,W);clear();watch(W,32,[(0x392763c0,W,[f(150),0])]);call(0x392763c0,W,[f(120),1])
expect('anchor_handler_silent_resize_precedes_bound',[['C',32],['W',32],['W',33],['C',33]])
assert [row[2][2]for row in trace()if row[1]==33]==[150.,100.]
setup();anchor(C,W);clear();watch(W,33,[(0x392763c0,W,[f(150),1])]);call(0x392763c0,W,[f(120),1])
expect('bound_handler_reentry_finishes_before_outer_dependents',[['C',32],['W',32],['W',33],['C',32],['W',32],['W',33],['C',33],['C',33]])
setup();anchor(C,W);clear();watch(W,32,[(0x39279260,W,[33,0])]);call(0x392763c0,W,[f(120),1])
expect('anchor_handler_removes_bound_handler_but_continuation_runs',[['C',32],['W',32],['C',33]])
setup();anchor(C,W);clear();watch(W,33,[(0x39279260,C,[33,0])]);call(0x392763c0,W,[f(120),1])
expect('bound_handler_changes_later_handler_live',[['C',32],['W',32],['W',33]])
# The source callback runs BEFORE its iterator is captured: these mutations do not
# invalidate an active native vector iterator. No removal/growth during traversal.
setup();anchor(C,W);clear();watch(W,33,[(0x39276a70,C,[])]);call(0x392763c0,W,[f(120),1])
expect('source_callback_removes_dependent_before_walk',[['C',32],['W',32],['W',33]])
setup();anchor(C,W);clear();watch(W,33,[(0x39276dd0,OTHER,[0,W,0,0,0,1])]);call(0x392763c0,W,[f(120),1])
expect('source_callback_adds_dependent_before_walk',[['C',32],['W',32],['W',33],['O',32],['C',33],['O',33]])
setup();anchor(OTHER,W);anchor(C,W);anchor(T,C);clear()
put(0x13e000,'I',0x399c4f18);put(0x13e000+0x68,'f',1)
watch(OTHER,32,[(0x39276dd0,G,[0,T,0,0,0,1]),(0x392763c0,G,[f(110),1])])
anchor(W,0x13e000,x=30,y=40)
expect('reentrant_size_notifies_bound_with_unready_anchor',[['O',32],['G',33],['G',32],['T',32],['C',32],['W',32]])

setup();anchor(C,W);anchor(G,C);put(W+0xc8,'2I',W+0xe00,W+0xe04);put(W+0xe00,'I',C);put(C+0xc0,'I',W);clear()
call(0x3927d120,W,[f(1),4]);expect('same_scale_has_no_bound',[[n,e]for n,e in [('G',32),('C',32),('G',32),('C',32),('W',32),('W',42)]])
clear();call(0x3927d120,W,[f(2),4]);assert all(row[1]!=33 for row in trace());cases['changed_scale_has_no_bound']=trace()
setup();anchor(C,W);clear();u.mem_write(W+0xa00,data[0x399c6c90-base:0x399c6c90-base+0x100])
call(0x392763c0,W,[f(120),1]);expect('drawable_bound_virtual_does_not_propagate',[['C',32]])
setup();anchor(C,W);clear();put(W+0xa00+0xd8,'I',0x3927bd60)
call(0x392763c0,W,[f(120),1]);expect('generic_widget_bound_virtual',[['C',32],['W',32],['W',33],['C',33]])

outdir=pathlib.Path('data/extract/aa-ui');outdir.mkdir(parents=True,exist_ok=True)
out=outdir/'native-bound-oracle-20261023.json'
out.write_text(json.dumps({'provenance':__doc__,'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'cry_sha256':hashlib.sha256(cry_path.read_bytes()).hexdigest(),'event33':'OnBoundChanged','fpcw':'037f','cases':cases},indent=2))
md=Cs(CS_ARCH_X86,CS_MODE_32);lines=[]
for a,n in [(0x39269420,0x50),(0x39264ff0,0x5e),(0x392792e0,0x30),(0x39279260,0x7e),(0x3922c160,0x3a),(0x392763c0,0xf8),(0x39261740,0x17e),(0x39512990,0x12),(0x3927b8b0,0x68),(0x39279ae0,0x76),(0x3927bd60,0x37),(0x392ca020,0x20),(0x3927b5e0,0x28),(0x3927a520,0x195),(0x39276350,0x65),(0x39275f10,0x3f2),(0x39276dd0,0xce),(0x39276940,0xa2),(0x39276a80,0x221),(0x3927d120,0x58),(0x3927b2f0,0x12a),(0x3928aa80,0x31),(0x3927a435,0x90)]:
 lines.append(f'\nSOURCE {a:x} size {n:x}');lines.extend(f'{i.address:x}: {i.mnemonic} {i.op_str}'for i in md.disasm(data[a-base:a-base+n],a))
event_ptr=get(0x39ac6ab0+33*4,'I')[0]
lines.insert(0,f'Event-name table39ac6ab0[33] -> {event_ptr:x}: '+bytes(u.mem_read(event_ptr,15)).split(b'\0')[0].decode())
(outdir/'native-bound-source-20261023.txt').write_text('\n'.join(lines))
print(len(cases),'native groups PASS',out)
