"""CPU-only execution of pinned original 1.2 instructions. No host DLL loading.

Only declared fixture endpoints (XML attribute reader, D3D descriptor sink,
token stream, CRT string/memset, static texture resource loader) are replaced.
All policy decisions and the sampler interner execute original x86 bytes.
"""
from pathlib import Path
import hashlib, itertools, json, struct
import pefile
import unicorn as u
from unicorn.x86_const import *

REPO=Path(__file__).resolve().parents[3]
ROOT=REPO/'data/extract/aa-cry/sampler-producer-20261024'
MODULES=json.loads((ROOT/'modules.json').read_text())
IMAGES=[]
for key in ('3d','render'):
    row=MODULES[key]; raw=Path(row['module']).read_bytes()
    assert hashlib.sha256(raw).hexdigest()==row['sha256']
    pe=pefile.PE(data=raw)
    IMAGES.append((pe.OPTIONAL_HEADER.ImageBase,pe.get_memory_mapped_image()))
xml=json.loads((ROOT/'xml-scan.json').read_text());raw=Path(xml['module']).read_bytes()
assert hashlib.sha256(raw).hexdigest()==xml['sha256']
pe=pefile.PE(data=raw);IMAGES.append((pe.OPTIONAL_HEADER.ImageBase,pe.get_memory_mapped_image()))
R=0x60000000; OBJ=0x61000000; AUX=0x62000000; CACHE=0x63000000
SYS=0x64000000; STACK=0x70000000; END=0x71000000
DEVICE=END+0x1000; STRCHR=END+0x1100; MEMSET=END+0x1200
FEATURE=END+0x1300; CONSOLE=END+0x1400; TEXLOAD=END+0x1500
ATTR_BOOL=END+0x1600; ATTR_INT=END+0x1700; STRICMP=END+0x1800
ADDREF=END+0x1900; RELEASE=END+0x1A00
ATOI=END+0x1B00
RESOURCE_UPDATE=END+0x1C00
RESULTS=[]; TRACE=bytearray(); NATIVE_RANGES={}


def w(c,a,fmt,*v): c.mem_write(a,struct.pack(fmt,*v))

def r(c,a,fmt='<I'): return struct.unpack(fmt,c.mem_read(a,struct.calcsize(fmt)))[0]

def cs(c,a):
    b=bytearray()
    while a and (v:=r(c,a,'<B')):
        b.append(v); a+=1
        assert len(b)<4096
    return b.decode('ascii')

def ret(c,n=0,value=0):
    sp=c.reg_read(UC_X86_REG_ESP); target=r(c,sp)
    c.reg_write(UC_X86_REG_ESP,sp+4+n)
    c.reg_write(UC_X86_REG_EAX,value&0xffffffff)
    c.reg_write(UC_X86_REG_EIP,target)

def args(c,n): return struct.unpack('<'+'I'*n,c.mem_read(c.reg_read(UC_X86_REG_ESP)+4,n*4))

def new():
    c=u.Uc(u.UC_ARCH_X86,u.UC_MODE_32)
    for base,im in IMAGES:
        c.mem_map(base,(len(im)+4095)&~4095); c.mem_write(base,im)
    c.mem_map(0,4096)
    for a in (R,OBJ,AUX,CACHE,SYS,STACK,END): c.mem_map(a,0x20000)
    w(c,0x383AA9E8,'<I',R); w(c,0x383B17C4,'<I',R); w(c,0x383AA9E0,'<I',SYS)
    w(c,R,'<I',R+0x10000); w(c,R+0x10000+36,'<I',FEATURE)
    w(c,R+0x10000+572,'<I',TEXLOAD); w(c,R+71276,'<I',16)
    w(c,R+71044,'<I',AUX); w(c,AUX,'<I',AUX+0x1000)
    w(c,AUX+0x1000+92,'<I',DEVICE)
    w(c,SYS+28,'<I',SYS+0x1000); w(c,SYS+0x1000,'<I',SYS+0x2000)
    w(c,SYS+0x2000+96,'<I',CONSOLE)
    w(c,0x383DE160,'<III',CACHE,CACHE+20,CACHE+0x10000)
    c.mem_write(CACHE,bytes([1,1,1,1,1,1,1,0])+bytes(12))
    w(c,0x383DDBD8,'<I',0); w(c,0x383B1644,'<I',0)
    # No imported CRT or host graphics code is allowed to execute.
    w(c,0x382D32A8,'<I',STRCHR)
    w(c,0x3660846C,'<I',ATOI)
    return c

def run(c,entry,argv=(),this=OBJ,stop=END,hooks=None,regs=None):
    sp=STACK+0x10000
    w(c,sp,'<'+'I'*(len(argv)+1),END,*[v&0xffffffff for v in argv])
    c.reg_write(UC_X86_REG_ESP,sp); c.reg_write(UC_X86_REG_EBP,0)
    c.reg_write(UC_X86_REG_ECX,this)
    c.reg_write(UC_X86_REG_FPCW,0x37F); c.reg_write(UC_X86_REG_MXCSR,0x1F80)
    if regs:
        for reg,value in regs.items(): c.reg_write(reg,value)
    initial_flags=c.reg_read(UC_X86_REG_EFLAGS)
    trace=[]; events=[]
    def device(c,events):
        obj,desc,out=args(c,3)
        values=struct.unpack('<13I',c.mem_read(desc,52))
        events.append(dict(endpoint='D3D11 CreateSamplerState descriptor sink',address=hex(DEVICE),
            descriptor_u32=list(values),descriptor_bytes=bytes(c.mem_read(desc,52)).hex(),
            result='S_OK fixture, no graphics validation'))
        handle=AUX+0x3000; w(c,handle,'<I',AUX+0x4000)
        w(c,AUX+0x4000+4,'<I',ADDREF); w(c,AUX+0x4000+8,'<I',RELEASE)
        w(c,out,'<I',handle); ret(c,12)
    def strchr(c,events):
        a,ch=args(c,2); data=cs(c,a).encode(); pos=data.find(bytes([ch&255]))
        events.append(dict(endpoint='CRT strchr',input=data.decode(),character=ch,result_offset=pos))
        ret(c,0,a+pos if pos>=0 else 0)
    def memset(c,events):
        dest,value,count=args(c,3); c.mem_write(dest,bytes([value&255])*count)
        events.append(dict(endpoint='CRT memset',bytes=count,value=value&255)); ret(c,0,dest)
    def features(c,events):
        value=r(c,R+0x1f000)
        events.append(dict(endpoint='renderer feature fixture',value=value)); ret(c,0,value)
    def console(c,events):
        _,name=args(c,2); events.append(dict(endpoint='console lookup absent fixture',name=cs(c,name)))
        ret(c,8)
    def addref(c,events): events.append(dict(endpoint='inert sampler COM AddRef fixture')); ret(c,4,2)
    def release(c,events): events.append(dict(endpoint='inert sampler COM Release fixture')); ret(c,4,1)
    def atoi(c,events):
        import re
        text=cs(c,args(c,1)[0]);m=re.match(r'\s*([+-]?\d+)',text)
        value=int(m.group(1)) if m else 0
        assert -2147483648<=value<=2147483647,'atoi overflow deliberately outside this fixture contract'
        events.append(dict(endpoint='MSVCR100 atoi import fixture',text=text,value=value,
            scope='CRT decimal conversion only; lookup/absence/store/Boolean decisions execute native code'))
        ret(c,0,value)
    dispatch={DEVICE:device,STRCHR:strchr,MEMSET:memset,0x38282CAC:memset,FEATURE:features,CONSOLE:console,
              ADDREF:addref,RELEASE:release,ATOI:atoi}
    dispatch.update(hooks or {})
    def step(c,ea,size,_):
        trace.append(ea)
        if ea in dispatch: dispatch[ea](c,events)
        elif not (0x30001000<=ea<0x3024C000 or 0x36501000<=ea<0x36608000 or 0x38001000<=ea<0x382D3000):
            raise AssertionError('undeclared code endpoint '+hex(ea))
    hook=c.hook_add(u.UC_HOOK_CODE,step)
    def bad(c,access,a,size,value,_):
        raise AssertionError(f'unsupported memory at {c.reg_read(UC_X86_REG_EIP):08x}: {a:08x}/{size}')
    mh=c.hook_add(u.UC_HOOK_MEM_INVALID,bad)
    try: c.emu_start(entry,stop,timeout=2000000,count=20000)
    except Exception as error:
        raise AssertionError(f'execution {entry:08x}: eip={c.reg_read(UC_X86_REG_EIP):08x}, last={[hex(x) for x in trace[-8:]]}') from error
    finally: c.hook_del(hook); c.hook_del(mh)
    assert c.reg_read(UC_X86_REG_EIP)==stop,hex(c.reg_read(UC_X86_REG_EIP))
    blob=struct.pack('<'+'I'*len(trace),*trace); off=len(TRACE); TRACE.extend(blob)
    return dict(entry=hex(entry),stop=hex(stop),instructions=len(trace),
        argv=[hex(v&0xffffffff) for v in argv],initial_this=hex(this),
        initial_eflags=hex(initial_flags),final_eflags=hex(c.reg_read(UC_X86_REG_EFLAGS)),
        initial_register_overrides={str(k):hex(v) for k,v in (regs or {}).items()},
        trace_byte_offset=off,trace_bytes=len(blob),trace_sha256=hashlib.sha256(blob).hexdigest(),endpoints=events)

def save(kind,inputs,result,execution):
    row=dict(id=len(RESULTS),kind=kind,inputs=inputs,result=result,execution=execution); RESULTS.append(row); return row

def state(c,p=OBJ):
    b=bytes(c.mem_read(p,20)); return dict(raw=b.hex(),filter=list(struct.unpack('<3b',b[:3])),
        address=list(b[3:6]),anisotropy=b[6],active=b[16],comparison=b[17],srgb=b[18])

def constructor():
    c=new(); ex=run(c,0x30055AE0)
    assert (r(c,OBJ+9,'<B'),r(c,OBJ+10,'<B'),r(c,OBJ+11,'<b'),r(c,OBJ+36,'<B'))==(1,1,-1,1)
    return save('slot_constructor',{},dict(tile_u=1,tile_v=1,filter=-1,tex_type=1,raw=bytes(c.mem_read(OBJ,60)).hex()),ex)

def xml_case(attrs):
    c=new();init=run(c,0x30055AE0)
    frame=STACK+0x18000; slot=frame-0x588; c.mem_write(slot,bytes(c.mem_read(OBJ,60)))
    # Native CXmlNode vtable, original lookup/ASCII-insensitive comparator,
    # original typed getter functions, and directly authored string pair storage.
    # Parsing XML punctuation/allocating strings is not claimed by this oracle.
    node=AUX+0x8000;w(c,node,'<I',0x366184D8);w(c,frame-20,'<I',node)
    pairs=AUX+0x9000;strings=AUX+0xa000
    for i,(key,value) in enumerate(attrs.items()):
        val=str(value);c.mem_write(strings,key.encode()+b'\0');w(c,pairs+8*i,'<I',strings);strings+=len(key)+1
        c.mem_write(strings,val.encode()+b'\0');w(c,pairs+8*i+4,'<I',strings);strings+=len(val)+1
    w(c,node+40,'<II',pairs,pairs+8*len(attrs))
    ex=run(c,0x301A8091,stop=0x301A8117,
           regs={UC_X86_REG_EBP:frame,UC_X86_REG_EBX:0})
    got=dict(tile_u=r(c,slot+9,'<B'),tile_v=r(c,slot+10,'<B'),filter=r(c,slot+11,'<b'),tex_type=r(c,slot+36,'<B'))
    import re
    def number(name,default):
        value=next((str(v) for k,v in attrs.items() if k.lower()==name.lower()),None)
        if value is None:return default
        m=re.match(r'\s*([+-]?\d+)',value);return int(m.group(1)) if m else 0
    assert got==dict(tile_u=int(number('IsTileU',1)!=0),tile_v=int(number('IsTileV',1)!=0),
        filter=struct.unpack('<b',bytes([number('Filter',-1)&255]))[0],tex_type=number('TexType',1)&255)
    return save('material_xml_native_getters',attrs,got,[init,ex])

def filter_case(code,quality=0,cap=16,default=(1,1,1),initial_aniso=0,compare=0):
    c=new(); w(c,0x383B1644,'<i',quality); w(c,R+71276,'<i',cap)
    w(c,CACHE,'<3b',*default); w(c,OBJ+6,'<B',initial_aniso)
    w(c,OBJ+17,'<B',compare)
    ex=[run(c,0x380FA400,(code,))]; success=c.reg_read(UC_X86_REG_EAX)&255
    ex.append(run(c,0x380FA2C0,(0,0,0)))
    before=state(c); ex.append(run(c,0x380FA580))
    desc=next(e for e in ex[-1]['endpoints'] if 'descriptor_u32' in e)['descriptor_u32']
    if code>=0 and code<=7:
        assert success==1
        assert desc[0]==[0,20,20,21,85,85,85,85][code]+(128 if compare else 0)
        assert desc[12]==(0 if code<2 else 0x42c80000)
    return save('filter_descriptor',dict(code=code,quality=quality,hardware_cap=cap,default=list(default),initial_aniso=initial_aniso,comparison=compare),
        dict(filter_return=success,state=before,descriptor_u32=desc),ex)

def material_case(tile_u,tile_v,code=-1):
    c=new(); w(c,OBJ+9,'<BBb',tile_u,tile_v,code); frame=STACK+0x18000
    ex=run(c,0x38259838,stop=0x38259892,regs={UC_X86_REG_EBP:frame,UC_X86_REG_EBX:0,UC_X86_REG_ESI:OBJ})
    ix=r(c,OBJ+32); got=state(c,CACHE+20*ix)
    assert got['address']==[1 if tile_u else 3,1 if tile_v else 3,1 if tile_u else 3]
    return save('material_sampler_block',dict(tile_u=tile_u,tile_v=tile_v,filter=code),dict(index=ix,state=got),ex)

def texture_source_branch(code,slot):
    c=new();frame=STACK+0x18000;w(c,OBJ+36,'<B',code)
    reached=[]
    labels={0x38259C4D:'ordinary named texture helper',0x38259C41:'type6 clear reference',
        0x38259C7D:'Auto source/render target setup',0x38259BCF:'skip this Auto slot'}
    def boundary(c,events):
        ea=c.reg_read(UC_X86_REG_EIP);reached.append(labels[ea]);
        events.append(dict(endpoint='instruction boundary stop; no branch replaced',address=hex(ea),label=labels[ea]))
        c.reg_write(UC_X86_REG_EIP,END)
    ex=run(c,0x38259C32,hooks={a:boundary for a in labels},regs={UC_X86_REG_EBP:frame,
        UC_X86_REG_ECX:slot,UC_X86_REG_ESI:OBJ})
    expected=('Auto source/render target setup' if slot in (0,3) else 'skip this Auto slot') if code in (4,5) else 'type6 clear reference' if code==6 else 'ordinary named texture helper'
    assert reached==[expected]
    return save('native_tex_type_source_branch',dict(tex_type=code,slot=slot),dict(branch=reached[0]),ex)

def static_texture_case(code,name='fixture/static.dds',flags=0x100):
    c=new(); w(c,0x38504548,'<I',1); c.mem_write(OBJ,name.encode()+b'\0')
    def load(c,events):
        file,fl=args(c,2); events.append(dict(endpoint='IRenderer::EF_LoadTexture static resource fixture',
            name=cs(c,file),flags=hex(fl),ecx=hex(c.reg_read(UC_X86_REG_ECX)),
            contract='no requested TexType argument exists at this call'))
        ret(c,8,AUX+0x10000)
    ex=run(c,0x38258930,(OBJ,flags,code),hooks={TEXLOAD:load})
    assert len([e for e in ex['endpoints'] if e['endpoint'].startswith('IRenderer')])==1
    return save('static_texture_request',dict(tex_type=code,name=name,flags=hex(flags)),
        dict(texture=hex(c.reg_read(UC_X86_REG_EAX)),next_texture=r(c,AUX+0x10000+172)),ex)

def resource_anisotropy_case(minimum,maximum):
    c=new(); resources=AUX+0x10000
    for slot in range(13):
        address=OBJ+0x100*slot; w(c,address+32,'<I',0)
        w(c,resources+64+4*slot,'<I',address)
    w(c,0x383B15F4,'<I',minimum); w(c,0x383B15F0,'<I',maximum)
    ex=run(c,0x380DE230,this=resources)
    indices=[r(c,OBJ+0x100*slot+32) for slot in range(13)]
    assert indices[0]==indices[1]==indices[2] and all(x==0 for x in indices[3:])
    return save('resource_anisotropy_refresh',dict(minimum=minimum,maximum=maximum),
        dict(slot_indices=indices,refreshed_state=state(c,CACHE+20*indices[0]),opacity_slot=12,
             opacity_state=state(c,CACHE+20*indices[12])),ex)

def postload_sampler_case(code,minimum=1,maximum=16):
    c=new();resources=AUX+0x10000;item=SYS+0x8000;shader=SYS+0x9000;temporary=SYS+0x10000
    ex=[run(c,0x380FA400,(code,),this=temporary),run(c,0x380FA2C0,(0,0,0),this=temporary),
        run(c,0x38015FC0,(temporary,))]
    initial=c.reg_read(UC_X86_REG_EAX);before=state(c,CACHE+20*initial)
    for slot in range(13):
        address=OBJ+0x100*slot;w(c,address+32,'<I',initial)
        w(c,resources+64+4*slot,'<I',address)
    w(c,resources,'<I',0x382EC7D8) # original resource vtable; +4 ->382574D0
    w(c,item,'<4I',shader,resources,0,0);w(c,0x385041C0,'<I',1)
    w(c,0x383B15F4,'<I',minimum);w(c,0x383B15F0,'<I',maximum)
    # Renderer+3168 is explicitly zero: original constant-update wrapper takes
    # its no-render-thread branch. No resource virtual function is replaced.
    ex.append(run(c,0x382582B0,this=item))
    indices=[r(c,OBJ+0x100*slot+32) for slot in range(13)]
    assert indices[0]==indices[1]==indices[2] and all(x==initial for x in indices[3:])
    return save('shader_item_postload_sampler_override',dict(authored_filter=code,minimum=minimum,maximum=maximum,
        render_thread_pointer=0,resource_vtable='0x382ec7d8',constant_update_function='0x382574d0'),
        dict(initial_index=initial,initial=before,slot_indices=indices,
            diffuse_after=state(c,CACHE+20*indices[0]),opacity_after=state(c,CACHE+20*indices[12])),ex)


def precedence(c,shader_index,material_index):
    w(c,OBJ+12,'<I',AUX+0x10000); w(c,OBJ+32,'<I',material_index)
    ex=run(c,0x38079031,stop=0x38079056,regs={UC_X86_REG_EBP:STACK+0x18000,UC_X86_REG_EAX:OBJ,UC_X86_REG_EBX:shader_index&0xffffffff})
    return c.reg_read(UC_X86_REG_EBX),ex

def precedence_case(ix,active):
    c=new(); w(c,CACHE+20+16,'<B',active)
    chosen,ex=precedence(c,ix,7)
    assert chosen==(ix if ix>=0 and active else 7)
    return save('runtime_material_shader_selection',dict(shader_index=ix,shader_cached_active=active,material_index=7),dict(selected=chosen),ex)

def shader_case(tokens):
    # Execute original parser initializer, switch/loop, mode resolution, clamp,
    # border, and interner. Tokenization/string ownership are explicit fixtures.
    c=new(); frame=STACK+0x18000; stream=AUX+0x8000; values=AUX+0x9000
    w(c,stream+8,'<I',values); w(c,stream+160,'<I',0)
    w(c,frame+16,'<I',1234)
    token_iter=iter(tokens)
    def next_token(c,events):
        item=next(token_iter,None)
        if item:
            key,value=item; w(c,stream+80,'<I',key); w(c,values,'<I',value)
        events.append(dict(endpoint='pretokenized shader declaration fixture',token=item))
        ret(c,8,int(item is not None))
    def string_lookup(c,events):
        token,_=args(c,2); events.append(dict(endpoint='token string fixture',token=token))
        c.mem_write(AUX+0xa000,b'fixture\0'); ret(c,8,AUX+0xa000)
    def string_copy(c,events):
        events.append(dict(endpoint='sampler name string ownership fixture; no state bytes touched'))
        ret(c,4,c.reg_read(UC_X86_REG_ECX))
    def bool_token(c,events): events.append(dict(endpoint='shader Boolean true fixture')); ret(c,4,1)
    def texture_token(c,events):
        out,_=args(c,2); w(c,out,'<I',AUX+0xb000); c.mem_write(AUX+0xb000,b'$Opacity\0')
        events.append(dict(endpoint='shader Texture string token fixture',value='$Opacity'))
        ret(c,8,out)
    def texture_copy(c,events):
        events.append(dict(endpoint='Texture name ownership fixture; no state bytes touched')); ret(c,4)
    def string_destroy(c,events):
        events.append(dict(endpoint='temporary string destruction fixture; no state bytes touched')); ret(c)
    ex=run(c,0x38242ED7,stop=0x382431D6,hooks={0x38211AC0:next_token,0x38210780:string_lookup,
        0x3800D340:string_copy,0x38210250:bool_token,0x38211570:texture_token,
        0x3800D3C0:texture_copy,0x380D9E50:string_destroy},regs={UC_X86_REG_EBP:frame,UC_X86_REG_EBX:0,UC_X86_REG_ESI:stream})
    ix=c.reg_read(UC_X86_REG_EAX); fresh=state(c,frame-0x68); cached=state(c,CACHE+20*ix)
    assert fresh['active']==int(any(208<=t[0]<=214 for t in tokens))
    return save('shader_sampler_producer',dict(tokens=tokens),dict(local=fresh,index=ix,cached=cached),ex)


def main():
    import argparse
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--record',action='store_true',help="record ONLY this tool's new evidence")
    opts=parser.parse_args()
    constructor(); xml_case({})
    for u,v in itertools.product((0,1),repeat=2):
        xml_case({'IsTileU':u,'IsTileV':v,'Filter':-1,'TexType':1})
        material_case(u,v)
    for code in range(4):
        xml_case({'Filter':code,'TexType':0})
        filter_case(code)
        postload_sampler_case(code)
    for default in ((0,0,-1),(1,1,1)):
        filter_case(-1,default=default)
    for code in (0,1,4,6): texture_source_branch(code,12)
    for code in (0,1): static_texture_case(code)
    resource_anisotropy_case(4,16)
    shader_case([(207,0)]) # OPACITYMAP Texture-only declaration has no active state override
    for ix,active in ((-1,0),(1,0),(1,1)): precedence_case(ix,active)
    import subprocess,xml.etree.ElementTree as ET
    sources={}
    for path in ('shaders/hwscripts/cryfx/fxsamplerdefs.cfi',
        'materials/effects/keffect/z_tes_shield_a_type9.mtl',
        'materials/effects/keffect/zz_shield_01_d1.mtl','materials/effects/broken_glass2.mtl'):
        raw=subprocess.run([str(REPO/'tools/aa-pak.exe'),'cat',path],check=True,capture_output=True).stdout
        sources[path]=dict(sha256=hashlib.sha256(raw).hexdigest())
        if path.endswith('.mtl'):
            node=ET.fromstring(raw)
            sources[path]['material']=dict(node.attrib)
            sources[path]['opacity']=next(t.attrib for t in node.find('Textures') if t.attrib.get('Map')=='Opacity')
        else:
            sources[path]['opacity_declaration']=next(line for line in raw.decode().splitlines() if line.startswith('#define OPACITYMAP '))
            assert sources[path]['opacity_declaration']=='#define OPACITYMAP sampler2D opacityMapSampler = sampler_state { Texture = $Opacity; };'
    evidence=dict(sources=sources,module_hashes={k:v['sha256'] for k,v in MODULES.items()},
        xml_module_sha256=xml['sha256'],cases=RESULTS,
        trace_sha256=hashlib.sha256(TRACE).hexdigest(),
        scope='Original x86 policy execution with explicit fixture globals and declared sinks; no host DLL loading or GPU execution. Missing/-1 global filter is variable; no native live-profile inference.')
    out=Path(__file__).with_name('opacity_sampler_evidence')/'native.json'
    if opts.record:
        out.parent.mkdir(exist_ok=True); out.write_text(json.dumps(evidence,indent=2)+'\n')
    else:
        assert json.loads(json.dumps(evidence))==json.loads(out.read_text()),'native evidence mismatch'
    # Recheck every original input after execution. Never write the retained evidence.
    for row in [*MODULES.values(),xml]:
        assert hashlib.sha256(Path(row['module']).read_bytes()).hexdigest()==row['sha256']
    print(json.dumps(dict(cases=len(RESULTS),trace_sha256=evidence['trace_sha256'],status='pass')))

if __name__=='__main__': main()
