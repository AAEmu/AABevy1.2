"""Offline original 1.2 chat drag-preview/release oracle; no live client writes.
Actual frame dispatch, caret predicates/anchor calls, slide repeat and scroll
arithmetic, paired vector reorder, receive routing and release dispatch run in
Unicorn. Geometry, widget/vector primitives and Lua/model delivery are controlled
endpoints. FPCW=037f. Requires pefile, capstone, unicorn; run from repository root.
"""
import hashlib, json, pathlib, struct
import pefile
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import *

PATH = pathlib.Path('D:/aa-re/client12/x2game_unpacked.dll')
pe = pefile.PE(str(PATH)); base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()
W, S, B, M, V, P, SYS = 0x110000, 0x112000, 0x120000, 0x130000, 0x150000, 0x160000, 0x170000
STACK, STOP, STUB = 0x20ff00, 0x4000, 0x5000

def run(op='preview', ids=(0, 2, 5), x=260., y=115., moving=False,
        allow=True, index=1, target=2, before=False, dt=150., elapsed=0., interval=150.,
        left=False, right=False, offset=-80., first=20., last=600., captured=True):
    u = Uc(UC_ARCH_X86, UC_MODE_32)
    for a, n in [(0, 0x10000), (0x100000, 0x100000), (0x200000, 0x10000),
                 (base, (len(image) + 4095) & ~4095)]: u.mem_map(a, n)
    u.mem_write(base, image); u.reg_write(UC_X86_REG_FPCW, 0x37f)
    def put(a, f, *v): u.mem_write(a, struct.pack('<' + f, *v))
    def get(a, f): return struct.unpack('<' + f, u.mem_read(a, struct.calcsize('<' + f)))
    def finish(value=0, args=0):
        sp = u.reg_read(UC_X86_REG_ESP)
        u.reg_write(UC_X86_REG_EAX, value); u.reg_write(UC_X86_REG_EIP, get(sp, 'I')[0])
        u.reg_write(UC_X86_REG_ESP, sp + 4 + args)
    calls = []
    def hook(uc, a, n, _):
        sp = u.reg_read(UC_X86_REG_ESP); obj = u.reg_read(UC_X86_REG_ECX)
        if a == STOP: uc.emu_stop()
        elif a == 0x3921ac90: finish(P)
        elif a == 0x392c8650:
            px, py = get(get(sp + 4, 'I')[0], '2f'); finish(int(100 <= px <= 500 and 100 <= py <= 130), 4)
        elif a == 0x392864a0:
            button, point = get(sp + 4, '2I'); px, py = get(point, '2f')
            cx = 150. + ((button - B) // 0x100) * 100.
            # Return the geometry-provider squared distance on x87.
            put(P + 0x80, 'd', (cx - px) ** 2 + (115. - py) ** 2)
            u.reg_write(UC_X86_REG_EIP, STUB + 0x80)
        elif a == 0x39286400:
            button, out = get(sp + 4, '2I'); put(out, '2f', 150. + ((button-B)//0x100)*100., 115.); finish(out)
        elif a == STUB: # actual raw-zero admission endpoint
            i = get(sp + 4, 'i')[0]; finish(int(i < 0 or i >= len(ids) or ids[i] != 0), 4)
        elif a == 0x39276a70: calls.append(['clear_anchor', obj]); finish()
        elif a == 0x39276dd0: calls.append(['anchor', obj, *get(sp + 4, '6I')]); finish(args=24)
        elif a == 0x39275460: put(P + 0x40, '2f', 100., 100.); finish(P + 0x40)
        elif a == 0x39275480: put(P + 0x48, '2f', 400., 180.); finish(P + 0x48)
        elif a == 0x3922b0b0:
            point = get(sp + 4, 'I')[0]; put(point, '2f', 500., 280.); finish(point, 4)
        elif a == 0x392c97e0: calls.append(['layout']); finish()
        elif a == STUB + 0x10:
            out, src, size = get(sp + 4, '3I'); u.mem_write(out, bytes(u.mem_read(src, size))); finish(out)
        elif a in [0x392cc070, 0x3927ffd0]:
            out, at, source = get(sp + 4, '3I'); begin, end = get(obj, '2I')
            assert begin <= at <= end, ('native out-of-range iterator excluded', hex(at))
            u.mem_write(at + 4, bytes(u.mem_read(at, end-at))); put(at, 'I', get(source, 'I')[0]); put(obj+4, 'I', end+4); finish(out, 12)
        elif a == 0x392aa1f0: finish(SYS)
        elif a == 0x392a69d0: finish(B + index * 0x100 if captured else 0)
        elif a == STUB + 0x20: finish(obj)
        elif a == 0x392cc7d0: calls.append(['cross_owner_fallback']); finish(0, 4)
        elif a == STUB + 0x30: calls.append(['receive']); finish(args=4)
        elif a == STUB + 0x40: calls.append(['mouse_up']); finish(args=4)
        elif a == STUB + 0x50: calls.append(['stop']); finish()
        elif a == 0x392a75c0: finish(args=8)
        elif a == 0x39512a40: calls.append(['configure',obj,get(sp+4,'I')[0]]); finish(args=4)
        elif a == 0x3927b610: calls.append(['layer',obj,*get(sp+4,'3I')]); finish(args=12)
    u.hook_add(UC_HOOK_CODE, hook)
    put(W, 'I', 0x399e7400); put(W+0x314, 'B', int(allow)); put(W+0x380, 'B', int(moving))
    # Use controlled predicates and synthetic vtable; actual branch bodies execute.
    u.mem_write(STUB+0x100, bytes(image[0x399e7400-base:0x399e7400-base+0x1e0]))
    put(W, 'I', STUB+0x100); put(STUB+0x100+0x1d8, 'I', STUB)
    put(W+0x36c, 'I', W+0xe00); put(W+0x360, 'I', W+0xb00); put(W+0x364, 'I', W+0xc00)
    for window in [W,S]:
        put(window+0x394,'I',window+0xf00);put(window+0xf00+0xba,'B',int(window==W and captured))
    put(W+0x34c, '2f', 0., 5.); put(W+0x30c, '2f', interval, elapsed)
    put(W+0x2f8, '3f', offset, last-first, first); put(W+0x304, '2f', last, 50.)
    put(W+0xb00+0xb8, 'B', int(left)); put(W+0xc00+0xb8, 'B', int(right))
    put(W+0x2b8, '3I', V, V+4*len(ids), V+0x100)
    put(W+0x2c8, '3I', V+0x200, V+0x200+4*len(ids), V+0x300)
    put(W+0x2f0, 'i', 1); put(W+0x320, 'B', int(before))
    for i in range(len(ids)):
        button=B+i*0x100; put(V+i*4,'I',button); put(V+0x200+i*4,'I',M+i*0x100)
        put(button,'I',STUB+0x400); put(STUB+0x400+0x14,'I',STUB+0x20)
    put(0x38ff6700,'I',STUB+0x10)
    u.mem_write(STUB+0x80,b'\xdd\x05'+struct.pack('<I',P+0x80)+b'\xc3')
    put(P,'2f',x,y); put(0x39d3156c,'2I',P+0x100,P+0x108); put(P+0x100,'2I',W,S); put(S+0x380,'B',1)
    obj=W
    if op=='preview': entry,args=0x392caff0,[]
    elif op=='repeat': entry,args=0x392ca150,[struct.unpack('<I',struct.pack('<f',dt))[0]]
    elif op=='frame': entry,args=0x392cb310,[struct.unpack('<I',struct.pack('<f',dt))[0]]
    elif op=='reorder': entry,args=0x392cc520,[index,target,int(before)]
    elif op=='receive':
        put(P+0x40,'I',B+target*0x100); entry,args=0x392ccb30,[P+0x40]
    elif op=='release':
        obj=SYS; put(SYS+8,'I',B); put(SYS+4,'I',B); put(SYS+0x2c,'I',B); put(SYS+0x30,'2I',2,1)
        put(B,'I',STUB+0x400); put(STUB+0x400+0x128,'I',STUB+0x30); put(STUB+0x400+0x124,'I',STUB+0x40); put(STUB+0x400+0x130,'I',STUB+0x50)
        entry,args=0x392a7a00,[1,P]
    elif op in ['prepare','restore']:
        obj=SYS;head=P+0x300;n1=P+0x340;n2=P+0x380
        put(SYS+4,'I',head);put(head,'I',n1);put(head+0x11,'B',1)
        put(n1,'4I',head,head,n2,W);put(n2,'4I',head,n1,head,S)
        entry,args=(0x39502100,[]) if op=='prepare' else (0x395021b0,[W])
    put(STACK,'I'+'I'*len(args),STOP,*args); u.reg_write(UC_X86_REG_ESP,STACK); u.reg_write(UC_X86_REG_ECX,obj)
    try: u.emu_start(entry,0,count=200000)
    except Exception as e: raise RuntimeError((hex(u.reg_read(UC_X86_REG_EIP)),calls)) from e
    return dict(calls=calls, shown=get(W+0xe00+0x98,'B')[0], target=get(W+0x31c,'I')[0],
        before=get(W+0x320,'B')[0], offset=get(W+0x2f8,'f')[0], elapsed=get(W+0x310,'f')[0],
        buttons=list(get(V,str(len(ids))+'I')), messages=list(get(V+0x200,str(len(ids))+'I')), selected=get(W+0x2f0,'i')[0])

cases={}
for x in [100.,149.,150.,151.,249.,250.,251.,500.]:
    c=cases[f'caret_{x}']=run(x=x)
    assert bool(c['shown']) == (x>=150.)
    if c['shown']: assert [v for v in c['calls'] if v[0]=='anchor'][0][2:5] == [8,c['target'],int(not c['before'])]
for kw in [dict(x=99.,before=True),dict(y=131.),dict(moving=True),dict(ids=())]:
    c=cases[f'caret_guard_{kw}']=run(**kw); assert not c['shown']
    if kw.get('before'):assert c['before']==1 # Leaving the strip hides but retains side.
for dt in [149.,150.,151.]:
    c=cases[f'repeat_{dt}']=run(op='repeat',dt=dt,right=True)
    assert c['offset']==(-130. if dt>150 else -80.)
for kw in [dict(left=True),dict(right=True,last=520.),dict(left=True,first=80.),dict(right=True,last=500.)]:
    c=cases[f'slide_{kw}']=run(op='repeat',dt=151.,**kw)
    expected=0. if kw.get('first')==80. else -100. if kw.get('last')==520. else -80. if kw.get('last')==500. else -30.
    assert c['offset']==expected,c
for index,target,before in [(0,1,True),(0,1,False),(2,1,True),(2,1,False),(1,1,False)]:
    c=cases[f'reorder_{index}_{target}_{before}']=run(op='reorder',index=index,target=target,before=before)
    assert c['selected']==1
    assert [(v-B)//0x100 for v in c['buttons']]==[(v-M)//0x100 for v in c['messages']]
cases['receive_without_capture']=run(op='receive',captured=False)
cases['receive_same_owner']=run(op='receive',index=0,target=2)
assert cases['receive_same_owner']['buttons']==[B+0x100,B+0x200,B]
cases['release_order']=run(op='release'); assert cases['release_order']['calls']==[['receive'],['mouse_up'],['stop']]
cases['frame_is_preview_only']=run(op='frame',x=260.); assert cases['frame_is_preview_only']['buttons']==[B,B+0x100,B+0x200]
for op in ['prepare','restore']:
    for input_shown in [False,True]:
        c=cases[f'{op}_input_shown_{input_shown}']=run(op=op,captured=input_shown)
        windows=[S] if input_shown else [W,S]
        assert [v[1] for v in c['calls'] if v[0]=='configure']==windows
        for w in windows:
            assert [v for v in c['calls'] if v[0]=='layer' and v[1]==w]==[
                ['layer',w,layer,int(op=='prepare' or w==W),0] for layer in range(3)]
dest=pathlib.Path('data/extract/aa-ui/native-chat-drag-oracle-20261023.json')
dest.write_text(json.dumps(dict(provenance=__doc__,sha256=hashlib.sha256(PATH.read_bytes()).hexdigest(),cases=cases),indent=2),encoding='utf8')
md=Cs(CS_ARCH_X86,CS_MODE_32); md.skipdata=True
with dest.with_name('native-chat-drag-source-20261023.txt').open('w') as f:
    for a,n in [(0x392ca050,0x164),(0x392caff0,0xe7),(0x392cb310,0xc2),(0x392cc520,0xd7),(0x392ccb30,0x59),(0x392a7a00,0x99),(0x39281350,0xec),(0x39502100,0x172),(0x395128e0,0xe),(0x392778a0,0x3d),(0x392c85c0,0x12),(0x392c8710,0x19),(0x392cbd2b,0xd6)]:
        f.write(f'\nSOURCE {a:08x}\n')
        for i in md.disasm(image[a-base:a-base+n],a): f.write(f'{i.address:08x}: {i.mnemonic} {i.op_str}\n')
print(len(cases),'native groups',dest)
