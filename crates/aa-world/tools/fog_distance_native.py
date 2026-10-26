"""Original 1.2 fog bounds -> ratio -> octree maximum -> per-view distance.

Offline CPU only. Declared inputs are post-parent Matrix34/local bounds, live
normal-view CVar, camera FOV/position and proxy/node storage. Native geometry,
ratio setters, getters, octree cache store and distance math are never replaced.
The consumer interval excludes frustum/submission. XML/OnInit source connection
is separately preserved in area-subject-20261025-r1/INITIAL_LOAD_REPORT.md.
Requires pefile/capstone/unicorn; writes only a NEW ignored evidence directory.
"""
from pathlib import Path
import argparse,hashlib,json,math,struct,subprocess,xml.etree.ElementTree as ET
import capstone,pefile
from unicorn import Uc,UC_ARCH_X86,UC_MODE_32,UC_HOOK_CODE
from unicorn.x86_const import *

ROOT=Path(__file__).resolve().parents[3]
args=argparse.ArgumentParser()
args.add_argument('--out',required=True,type=Path)
out=args.parse_args().out.resolve()
allowed=(ROOT/'data/extract/aa-world/area-subject-20261025-r1').resolve()
assert allowed in out.parents and not out.exists(), 'Use a NEW ignored child directory; preserve all old proofs'
out.mkdir(parents=True)
pins={}
u=Uc(UC_ARCH_X86,UC_MODE_32)
md=capstone.Cs(capstone.CS_ARCH_X86,capstone.CS_MODE_32)
sources={
 'entity':('cryentitysystem','d1f0623a45c491888b9a2f3de54270d142d17afe2a63dd4c4ae95a3ea2a7377e'),
 'engine':('cry3dengine','34d6b73690d1a9d8d0ea0f5eb743fd0624107cfda28c1302826b19a3bc9546de'),
}
pes={}
for name,(file,expected) in sources.items():
    p=ROOT/f'data/extract/aa-world/environment/source/bin32/{file}.dll'
    raw=p.read_bytes();actual=hashlib.sha256(raw).hexdigest();assert actual==expected
    pins[name]=dict(path=str(p),sha256=actual)
    pe=pefile.PE(data=raw);pes[name]=pe;im=pe.get_memory_mapped_image()
    u.mem_map(pe.OPTIONAL_HEADER.ImageBase,(len(im)+4095)&~4095)
    u.mem_write(pe.OPTIONAL_HEADER.ImageBase,im)
u.mem_map(0x70000000,0x400000)
NODE,ENGINE,CVARS,PROXY,SLOTS,SLOT,CAMERA=[0x70000000+i for i in (0x1000,0x2000,0x3000,0x4000,0x5000,0x6000,0x7000)]
STACK,FRAME,STOP=0x703d0000,0x703e0000,0x703fff00
def words(a,*v):u.mem_write(a,struct.pack('<'+'I'*len(v),*(x&0xffffffff for x in v)))
def floats(a,*v):u.mem_write(a,struct.pack('<'+'f'*len(v),*v))
def flt(a):return struct.unpack('<f',u.mem_read(a,4))[0]
def fs(a,n):return list(struct.unpack('<'+'f'*n,u.mem_read(a,4*n)))
def f32(v):return struct.unpack('<f',struct.pack('<f',v))[0]
def bits(v):return struct.unpack('<I',struct.pack('<f',v))[0]
def reset():
    u.reg_write(UC_X86_REG_FPCW,0x37f);u.reg_write(UC_X86_REG_FPTAG,0xffff)
    u.reg_write(UC_X86_REG_MXCSR,0x1f80)
    u.reg_write(UC_X86_REG_ESP,STACK);u.reg_write(UC_X86_REG_EBP,FRAME)
def call(a,this,*v):
    reset();words(STACK,STOP,*v);u.reg_write(UC_X86_REG_ECX,this)
    u.emu_start(a,STOP,count=20000);assert u.reg_read(UC_X86_REG_EIP)==STOP
stops=set();reached=set()
def hook(uc,at,size,_):
    reached.add(at)
    if at in stops:uc.emu_stop()
u.hook_add(UC_HOOK_CODE,hook)
words(NODE,0x3025ce10);words(ENGINE,0x30263718)
words(0x302dcd3c,ENGINE);words(0x302dcd40,CVARS)
floats(ENGINE+1216,1.0);floats(CVARS+1404,0.0)
words(PROXY,0x320cd730,0x320cd608)
words(PROXY+0x50,SLOTS,SLOTS+4);words(SLOTS,SLOT);words(SLOT+0x74,NODE)

def bounds(rows,size):
    # Real world-AABB helper consumes native row-major Matrix34 and local AABB.
    floats(NODE+0x3c,*[v for row in rows for v in row])
    floats(NODE+0x15c,*[-v*.5 for v in size],*[v*.5 for v in size])
    call(0x300b54f0,NODE)
    return fs(NODE+404,6)
def compile_max(ratio,general):
    floats(CVARS+1376,general)
    call(0x3207b420,PROXY+4,ratio)
    reset();u.reg_write(UC_X86_REG_ESI,NODE)
    # Actual octree instructions call the node's real virtual getter, store its
    # return to the octree local and node+8, and clear original flag bits.
    u.emu_start(0x301e8914,0x301e892a,count=20000)
    assert u.reg_read(UC_X86_REG_EIP)==0x301e892a
    return flt(NODE+8)
def fov_scale(fov):
    reset();floats(CAMERA+0x30,fov);u.reg_write(UC_X86_REG_EDI,CAMERA)
    # Actual normal upload; ends immediately after the global f32 store.
    u.emu_start(0x30151de8,0x30151e18,count=1000)
    assert u.reg_read(UC_X86_REG_EIP)==0x30151e18
    return flt(0x30290924)
def admitted(eye,fov):
    scale=fov_scale(fov)
    reset();u.reg_write(UC_X86_REG_EDI,NODE)
    floats(FRAME-0x18,eye[0]);floats(FRAME-0x1c,eye[1]);floats(FRAME-0x20,eye[2])
    words(FRAME+0x18,1) # Declared already-frustum-admitted interval input.
    stops.update([0x301edcc2,0x301edf47])
    # Real consumer virtual world-AABB copy + distance interval + strict gate.
    u.emu_start(0x301edbd3,STOP,count=2000)
    ok=u.reg_read(UC_X86_REG_EIP)==0x301edcc2
    assert u.reg_read(UC_X86_REG_EIP) in stops
    stops.clear()
    return dict(eye=list(eye),fov=fov,fov_scale=scale,fov_scale_bits=bits(scale),scaled_distance=flt(FRAME-0x10),admitted=ok)

# Retained original asset pin, read through aa-pak only. No asset rewrites.
xml=subprocess.check_output([str(ROOT/'tools/aa-pak.exe'),'cat','worlds/main_world/cells/021_008/client/entities.xml'])
pins['authored_xml']=dict(path='game/worlds/main_world/cells/021_008/client/entities.xml',sha256=hashlib.sha256(xml).hexdigest())
assert pins['authored_xml']['sha256']=='648900163ac2bbb2b20a5266a21d9edd205519e8bf4af013d9db032f2e2b14c3'
fog4=next(e for e in ET.fromstring(xml).iter('Entity') if e.get('EntityGuid')=='4133521fcc7b527e')
assert fog4.get('ViewDistRatio')=='10' and fog4.find('Properties').get('GlobalDensity')=='0.01'

cases=[
 ('fog4_quarter_turn_world_control',[[0,1,0,22064.125],[-1,0,0,8376.24854],[0,0,1,447.38528]],[60,60,40],10,60),
 ('fog4_legacy100_control',[[0,1,0,22064.125],[-1,0,0,8376.24854],[0,0,1,447.38528]],[60,60,40],100,60),
 ('origin',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],10,60),
 ('parent_rotation_nonuniform',[[0,-3,0,100],[2,0,0,200],[0,0,4,300]],[60,60,40],10,60),
 ('parent_mirror_shear',[[1,.25,-.125,100],[.5,-2,.25,200],[0,.5,4,300]],[60,60,40],10,60),
 ('parent_precise_fraction',[[.73,-1.11,.29,22064.125],[1.43,.27,-.51,8376.24854],[-.19,.87,1.33,447.38528]],[60,60,40],10,60),
 ('zero',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],0,60),
 ('negative_clamp',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],-7,60),
 ('sentinel',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],255,60),
 ('over_clamp',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],999,60),
 ('live_general6',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],10,6),
 ('live_general120',[[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40],10,120),
]
rows=[]
for label,matrix,size,ratio,general in cases:
    b=bounds(matrix,size);limit=compile_max(ratio,general)
    q=u.mem_read(NODE+20,1)[0];assert q==min(255,max(0,ratio))
    logical=1000 if q==255 else q
    radius=math.sqrt(sum((b[i+3]-b[i])**2 for i in range(3)))*.5
    assert math.isclose(limit,general*radius*logical*.01,rel_tol=3e-7,abs_tol=1e-6)
    row=dict(label=label,matrix=matrix,size=size,ratio=ratio,node_byte=q,general=general,
             bounds=b,bounds_bits=[bits(x) for x in b],maximum=limit,maximum_bits=bits(limit))
    if label.startswith('fog4_'):
        assert bits(limit)==bits(281.4249267578125 if ratio==10 else 2814.249267578125)
        cx,cy,cz=[(b[i]+b[i+3])*.5 for i in range(3)]
        row['views']=[admitted([cx,cy-220,cz],f32(math.radians(50))),admitted([cx,cy-450,cz],f32(math.radians(50))),
                      admitted([cx,cy-400,cz],f32(math.radians(30))),admitted([cx,cy-400,cz],f32(math.radians(80)))]
        assert [v['admitted'] for v in row['views']]==([True,False,True,False] if ratio==10 else [True]*4)
    rows.append(row)

# A valid thin fog box makes the actual native maximum/subtraction exactly
# representable at this eye. Adjacent eye float values exercise strict equality.
b=bounds([[1,0,0,0],[0,1,0,0],[0,0,1,0]],[2,.01,.01]);limit=compile_max(100,60)
at=f32(b[3]+limit);assert f32(at-b[3])==limit
neighbors=[struct.unpack('<f',struct.pack('<I',bits(at)+i))[0] for i in (-1,0,1)]
strict=[admitted([x,0,0],f32(math.radians(60))) for x in neighbors]
assert [v['admitted'] for v in strict]==[True,False,False]
assert strict[1]['scaled_distance']==limit and strict[1]['fov_scale']==1
axis_bounds=bounds([[1,0,0,0],[0,1,0,0],[0,0,1,0]],[60,60,40]);compile_max(10,60)
axis_precision=admitted([31,4126,21.5],f32(math.radians(60)))
assert axis_precision['scaled_distance']==4096.0

required={0x300b54f0,0x3207b420,0x3001a990,0x301e8914,0x300b6900,0x300b8f70,
          0x30121b30,0x30151de8,0x3001a920,0x301edbd3,0x301edbfa}
assert required<=reached,[hex(x) for x in required-reached]
assembly=[]
for key,a,end_address in [('engine',0x300b54f0,0x300b5810),('engine',0x300b6900,0x300b6aa0),
                ('engine',0x301e8914,0x301e892a),('engine',0x30151de8,0x30151e18),
                ('engine',0x301edbd3,0x301edcc2)]:
    p=pes[key];raw=p.get_data(a-p.OPTIONAL_HEADER.ImageBase,end_address-a)
    assembly.append(f'; {key} {a:08X}..{end_address:08X} sha256={hashlib.sha256(raw).hexdigest()}')
    assembly.extend(f'{i:08X} {op} {s}' for i,_,op,s in md.disasm_lite(raw,a))
result=dict(source_pins=pins,cases=rows,strict_boundary=dict(bounds=b,maximum=limit,cases=strict),axis_precision=axis_precision,
    required_original_entries=sorted(hex(x) for x in required),native_endpoint_replacements=[],
    limits=['Input Matrix34/local box are explicit post-parent fixtures; native XML quaternion/parent construction is not emulated.',
            'Quarter-turn world control matches authored Fog4 final AABB quantization; actual unmodified XML is exercised by fog_distance example.',
            'Original bounds helper, setters, max getter, octree store, FOV upload, world-AABB copy and strict consumer are executed.',
            'Frustum admission and final GPU submission are excluded; no camera/actor identity or detail/reflection flags are invented.',
            'Ordinary general branch only, engine factor1/minimum0, x87 CW037f / SSE MXCSR1f80.'])
for name,content in [('native.json',json.dumps(result,indent=2)+'\n'),('native.asm','\n'.join(assembly)+'\n')]:
    with (out/name).open('x') as f:f.write(content)
print(f'PASS: {len(rows)} connected original bounds/ratio/octree-maximum cases, 8 authored-size normal-view gates +3 strict adjacent-f32 +1 Cry-axis precision gate; no modeled native endpoint.')
print('native.json SHA256',hashlib.sha256((out/'native.json').read_bytes()).hexdigest())
