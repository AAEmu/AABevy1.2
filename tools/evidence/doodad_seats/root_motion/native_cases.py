"""Isolated original x86 instructions only. No native DLL load or game process.

Boundaries are explicit: XML virtual methods, character/pose allocations, entry
registers and CHR type are fixtures. This is NOT a native graph/CAF importer run.
"""
import struct, json
from probe import OUT, SOURCES, load_binary, sha, write
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import *

FIX=0x10000000
STACK=0x20000000
H=0x100f0000
ANIM=0x10010000
CHAR=0x10020000
POSE=0x10030000
VT=0x10040000
DATA=0x10050000

class Probe:
    def __init__(self,game=False):
        self.u=Uc(UC_ARCH_X86,UC_MODE_32)
        self.modules=[]
        for which in (['action','anim','game'] if game else ['action','anim']):
            path,raw,pe,img=load_binary(which); base=pe.OPTIONAL_HEADER.ImageBase
            self.u.mem_map(base,(len(img)+0xfff)&~0xfff); self.u.mem_write(base,img)
            self.modules.append({'name':which,'path':str(path),'sha256':sha(raw),'image_base':hex(base)})
        self.u.mem_map(FIX,0x100000); self.u.mem_map(STACK,0x20000)
        self.r(UC_X86_REG_ESP,STACK+0x10000); self.r(UC_X86_REG_EBP,STACK+0x11000)
        self.events=[]; self.hooks={}; self.u.hook_add(UC_HOOK_CODE,self.hook)
    def r(self,reg,v=None):
        if v is None: return self.u.reg_read(reg)
        self.u.reg_write(reg,v)
    def put(self,at,*v): self.u.mem_write(at,struct.pack('<'+'I'*len(v),*v))
    def get(self,at): return struct.unpack('<I',self.u.mem_read(at,4))[0]
    def floats(self,at,*v): self.u.mem_write(at,struct.pack('<'+'f'*len(v),*v))
    def arg(self,n): return self.get(self.r(UC_X86_REG_ESP)+4+n*4)
    def ret(self,value=0,pop=0):
        s=self.r(UC_X86_REG_ESP); dest=self.get(s); self.r(UC_X86_REG_EAX,value); self.r(UC_X86_REG_ESP,s+4+pop); self.r(UC_X86_REG_EIP,dest)
    def hook(self,u,at,size,user):
        if at in self.hooks: self.hooks[at]()
    def run(self,a,b):
        self.u.emu_start(a,b,count=20000)
        assert self.r(UC_X86_REG_EIP)==b,hex(self.r(UC_X86_REG_EIP))

def enum_case():
    p=Probe(); bp=p.r(UC_X86_REG_EBP)
    # Original template loop reads horizontal from [ebp-18], vertical from ebx.
    p.put(bp-0x18,0x30740d30); p.r(UC_X86_REG_EBX,0x30740d10); p.r(UC_X86_REG_EDI,DATA); p.r(UC_X86_REG_ESI,0)
    p.run(0x305f0dd0,0x305f0e47)
    h,v=p.get(DATA+0xc),p.get(DATA+0x10)
    assert (h,v)==(3,5),(h,v)
    return {'case':'native template string-to-enum comparisons','entry':'305f0dd0','stop':'305f0e47','horizontal':'DecoupledCatchUp','vertical':'SmoothedEntity','result':[h,v],'fixture_boundary':'entry registers and string pointers; XML import before loop not executed','passed':True}

def apply_template():
    p=Probe(); bp=p.r(UC_X86_REG_EBP); CTX=DATA+0x1000; CFG=DATA+0x2000; OBJ=DATA+0x3000
    # Runtime CAG interface is the original parsed object +4 (305f0b80).
    p.put(DATA+0xc,3,5); p.r(UC_X86_REG_ECX,DATA+4)
    p.put(p.r(UC_X86_REG_ESP),H+0x40,CTX)
    p.put(CTX,OBJ); p.put(CTX+8,1,CHAR); p.put(OBJ,VT); p.put(VT+0x70,H)
    p.put(CHAR,VT+0x100); p.put(VT+0x1a4,H+0x10); p.put(VT+0x1a8,H+0x20)
    p.put(0x3078d700,CFG); p.put(CFG+0xa4,1)
    def mcm():
        p.events.append({'animated_character_vA4_h_v':[p.arg(0),p.arg(1)]}); p.ret(0,8)
    def smoothing():
        p.events.append({'animated_character_vA8_raw_args':[p.arg(0),p.arg(1)]}); p.ret(0,8)
    p.hooks={H:lambda:p.ret(),H+0x10:mcm,H+0x20:smoothing}
    p.run(0x305f09c0,H+0x40)
    assert p.events[0]['animated_character_vA4_h_v']==[3,5]
    return {'case':'native template application forwards h3 v5','entry':'305f09c0','stop':'fixture return sentinel','events':p.events,'passed':True,'boundary':'game-object/config/context and animated-character endpoints supplied; full template scheduler/priority merge not executed'}

def graph_fields():
    p=Probe(); bp=p.r(UC_X86_REG_EBP); XML=DATA+0x1000; XVT=DATA+0x2000
    p.put(bp-0x14,XML); p.put(XML,XVT); p.put(XVT+0x6c,H); p.put(XVT+0x64,H+0x10); p.put(XVT+0x34,H+0x20)
    p.r(UC_X86_REG_EBX,DATA); p.r(UC_X86_REG_ESI,1)
    def has():
        p.events.append({'xml_has_attribute':hex(p.arg(0))}); p.ret(1,4)
    def getint():
        p.put(p.arg(1),1); p.events.append({'xml_read_int':hex(p.arg(0)),'fixture_value':1}); p.ret(1,8)
    def getbool():
        p.u.mem_write(p.arg(1),b'\x00'); p.events.append({'xml_read_bool':hex(p.arg(0)),'fixture_value':False}); p.ret(1,8)
    p.hooks={H:has,H+0x10:getint,H+0x20:getbool}
    p.run(0x30617085,0x3061713c)
    h,v=p.u.mem_read(DATA+8,2); flag=p.u.mem_read(DATA+6,1)[0]&8
    assert (h,v,flag)==(1,1,0)
    return {'case':'native graph state field storage','entry':'30617085','stop':'3061713c','xml_fixture_values':{'MovementControlMethodH':1,'MovementControlMethodV':1,'animationControlledView':False},'stored_H_V_viewbit':[h,v,flag],'events':p.events,'passed':True,'boundary':'XML endpoint reads return authored values; no full state inheritance/template arbitration'}

def settings(p,present=True):
    bp=p.r(UC_X86_REG_EBP)
    p.put(CHAR+0x73c,ANIM if present else 0); p.put(CHAR+0x740,POSE); p.put(CHAR+0x110,99)
    p.put(POSE,VT); p.put(VT+0x40,H)
    p.put(ANIM,0x316779dc)
    def pose():
        p.events.append({'pose_virtual_slot_0x40_arg':p.arg(0)}); p.ret(0,4)
    p.hooks[H]=pose; p.r(UC_X86_REG_ESI,CHAR)
    p.run(0x3066c003,0x3066c03a)
    return p.u.mem_read(ANIM+0x45c,1)[0]

def root_clear(p,flag,kind,root,pelvis):
    bp=p.r(UC_X86_REG_EBP)
    p.u.mem_write(ANIM+0x45c,bytes([flag])); p.put(bp-0x70,ANIM); p.put(bp-0x34,kind)
    p.r(UC_X86_REG_EDI,DATA); p.floats(DATA,*root,*pelvis)
    p.run(0x3154d2ec,0x3154d32e)
    values=list(struct.unpack('<14f',p.u.mem_read(DATA,56)))
    return values[:7],values[7:]

def policy_cases():
    rows=[]
    p=Probe(); flag=settings(p)
    assert flag==1
    rows.append({'case':'native CryAction skeleton settings calls original CryAnimation setter','entry':'3066c003','stop':'3066c03a','setter':'31554630','native_vtable':'316779dc+10','animationDrivenMotion':flag,'events':p.events,'passed':True,'boundary':'present skeleton and pose interfaces supplied; caller timing/visibility gate not executed'})
    p=Probe(); flag=settings(p,False); assert flag==0
    rows.append({'case':'no skeleton skips native setter','animationDrivenMotion':flag,'passed':True})
    root=[0.1,-0.2,0.3,0.92736185,0.4,0.5,-0.6]; pelvis=[0.11,0.22,0.33,0.9,0.12,-0.4,0.59]
    for flag,kind in [(1,0x11223344),(0,0x11223344),(1,0x55)]:
        p=Probe(); r,b=root_clear(p,flag,kind,root,pelvis)
        expected=[0.,0.,0.,1.,0.,0.,0.] if flag and kind==0x11223344 else root
        assert max(abs(x-y) for x,y in zip(r,expected))<1e-6
        assert max(abs(x-y) for x,y in zip(b,pelvis))<1e-6
        rows.append({'case':'native root postprocess','entry':'3154d2ec','stop':'3154d32e','animationDrivenMotion':flag,'model_type':hex(kind),'input_root':root,'output_root':r,'adjacent_pelvis_unchanged':True,'passed':True,'boundary':'relative-pose[0] pointer, model type, animation object and pre-postprocess pose supplied; upstream CAF import/FIFO sampling not executed'})
    assets=json.loads((OUT/'assets.json').read_text(encoding='utf-8')); samples=[]
    for c in assets['characters']:
        cl=next(x for x in c['clips'] if x['name']=='fist_ac_sit_down_loop'); rb,pb=cl['bones']; opt=cl['position_scale']
        s=rb['samples'][0]; ps=pb['samples'][0]
        root=s['rot']+[x*opt for x in s['pos']]; pelvis=ps['rot']+[x*opt for x in ps['pos']]
        p=Probe(); r,b=root_clear(p,1,0x11223344,root,pelvis)
        assert max(abs(x-y) for x,y in zip(b,pelvis))<1e-6
        samples.append({'character':c['race']+'/'+c['gender'],'clip':cl['path'],'caf_sha256':cl['sha256'],'input_root':root,'output_root':r,'pelvis_position_after_native_root_slice':b[4:],'pelvis_unchanged':True,'passed':True})
    rows.append({'case':'actual CAL-resolved loop data through native root slice','cases':samples,'passed':True,'boundary':'CAF decode is probe.py, not native loader; only the final native root instruction slice executes'})
    return rows

def hold_position_cases():
    rows=[]
    for value in ['1','0']:
        p=Probe(game=True); bp=p.r(UC_X86_REG_EBP); A=CHAR; O=POSE; G=DATA+0x1000
        p.put(A,VT); p.put(VT+0xd8,H); p.put(O,VT+0x200); p.put(VT+0x24c,H+0x10)
        p.put(G,VT+0x400); p.put(VT+0x480,H+0x20); p.u.mem_write(DATA+0x2000,value.encode()+b'\x00')
        p.hooks={H:lambda:p.ret(O),H+0x10:lambda:p.ret(G),H+0x20:lambda:p.ret(DATA+0x2000,4)}
        p.put(bp-0x50,A); p.put(bp-0x30,0x11)
        for off in [0x1c,0x20,0x24,0x68]: p.floats(bp-off,0.7)
        p.r(UC_X86_REG_EDI,ANIM); p.r(UC_X86_REG_ESI,1); p.put(ANIM+0xc4,DATA+0x3000)
        def clear_helper():
            p.events.append({'native_input_reset_helper_39402a10':True}); p.ret()
        p.hooks[0x39402a10]=clear_helper
        p.run(0x39405f98,0x39405fd7)
        values=[struct.unpack('<f',p.u.mem_read(bp-off,4))[0] for off in [0x1c,0x20,0x24,0x68]]
        assert all(abs(x-(0 if value=='1' else 0.7))<1e-6 for x in values)
        rows.append({'case':'native HoldPosition graph-output consumer','entry':'39405f98','stop':'39405fd7','getter_executed':'39402810..3940285b','graph_output_fixture':value,'input_float_values_after':values,'activity_flags_after':p.get(bp-0x30),'events':p.events,'passed':True,'boundary':'actor/graph virtual endpoints return a supplied output string; locals entering this input branch are fixtures; movement/yaw unbond occurs in earlier original instructions, retained separately'})
    return rows

if __name__=='__main__':
    rows=[enum_case(),apply_template(),graph_fields(),*policy_cases(),*hold_position_cases()]
    p=Probe(game=True)
    write('native-cases.json',{'modules':p.modules,'execution':'Unicorn 32-bit only; no original DLL loaded by Windows','cases':rows})
    print(json.dumps({'cases':len(rows),'actual_loop_subcases':6,'all_passed':all(x['passed'] for x in rows)}))
