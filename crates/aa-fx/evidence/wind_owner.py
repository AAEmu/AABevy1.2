import sys,json,struct
sys.dont_write_bytecode=True
from pathlib import Path
sys.path.insert(0,str(Path(__file__).parent));from wind_area import Area
from unicorn.x86_const import UC_X86_REG_ECX,UC_X86_REG_EIP
n=Area();n.u32(0x302A96C8,n.get_u32(0x302A96C8)|1)
area,vt=[n.alloc(256) for _ in range(2)];n.u32(area,vt);owner=n.alloc(600);other=n.alloc(600);status_calls=[];param_calls=[]
def iterate():
 prev=n.args(1)[0];n.ret(area if prev==0 else 0,4)
def params():
 p=n.args(1)[0];kind=n.get_u32(p);param_calls.append(kind)
 if kind==18:
  n.u32(p+4,0xFFBFFFFF);n.vec(p+16,(1,1,1));n.f32(p+28,0);n.u32(p+32,0)
 elif kind==12:
  n.u32(p+72,1);n.vec(p+24,(0,0,78.4))
 elif kind==11:
  n.u32(p+4,owner);n.u32(p+8,0);n.u32(p+12,0)
 else:raise AssertionError(kind)
 n.ret(1,4)
def status():
 p=n.args(1)[0];kind=n.get_u32(p);status_calls.append(kind);assert kind==23,kind
 n.vec(p+80,(7,8,9));n.u32(p+128,1);n.ret(1,4)
n.u32(n.wvt+340,n.stub(iterate));n.u32(vt+32,n.stub(params));n.u32(vt+36,n.stub(status))
bbox=n.alloc(24);n.vec(bbox,(-1,-1,-1,1,1,1));out=n.alloc(64);cases=[]
for label,query_owner in [('same-owner',owner),('other-owner',other),('no-owner-filter',0)]:
 status_calls.clear();param_calls.clear()
 try:n.call(0x300888D0,out,(query_owner,bbox,4,0))
 except Exception as e:print(label,e,hex(n.vm.reg_read(UC_X86_REG_EIP)));raise
 cases.append(dict(name=label,wind=n.floats(out+12,3),flags=n.get_u32(out+44),status_calls=list(status_calls),param_calls=list(param_calls)))
print(json.dumps(cases,indent=2));assert cases[0]['wind']==[0,0,0] and not cases[0]['status_calls'];assert cases[1]['wind']==[7,8,9] and cases[1]['status_calls']==[23];assert cases[2]['wind']==[7,8,9]
(Path(__file__).parent/'wind_owner_native.json').write_text(json.dumps(dict(cases=cases,boundary='Original SPhysEnviron::GetPhysAreas traversal/medium/foreign-owner eligibility executes. Physics area GetParams/GetStatus and world NextArea are explicit fixtures; no physics sampling/solver executed.'),indent=2)+'\n')
