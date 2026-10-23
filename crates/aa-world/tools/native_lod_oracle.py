"""Execute shipping Cry3DEngine 301126F0; no game files are modified.
python crates/aa-world/tools/native_lod_oracle.py [--check]
Requires pefile, capstone and unicorn. Native x87 return is stored by an fstp stub.
"""
from pathlib import Path
import hashlib, json, struct, sys
import capstone, pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_ESP, UC_X86_REG_EDI, UC_X86_REG_XMM1

ROOT = Path(__file__).resolve().parents[3]
DLL = ROOT / 'data/extract/aa-cry/ida/cry3dengine.dll'
p = pefile.PE(str(DLL))
u = Uc(UC_ARCH_X86, UC_MODE_32)
base = p.OPTIONAL_HEADER.ImageBase
u.mem_map(base, (p.OPTIONAL_HEADER.SizeOfImage + 4095) & ~4095)
u.mem_write(base, p.get_memory_mapped_image())
u.mem_map(0x50000000, 0x40000)
u.mem_map(0x60000000, 0x10000)
def put(a, f, *v): u.mem_write(a, struct.pack('<' + f, *v))
def get(a, f): return list(struct.unpack('<' + f, u.mem_read(a, struct.calcsize('<' + f))))
put(0x302DCD40, 'I', 0x50001000)
put(0x5000173c, 'f', 5.)
put(0x30290920, 'I', 0)
put(0x302DCD70, 'I', 0)
# fld/fstp are the original ABI, avoiding Unicorn's opaque ST register encoding.
u.mem_write(0x5003f000, b'\xd9\x1d' + struct.pack('<I', 0x50002000))
cases = {
    'out_hold_finish': [(100, 0), (100, 1), (101, 1), (102.5, 1), (102.5, 1), (102.5, 1), (105, 1), (105.01, 1)],
    'in_hold_finish': [(100, 1), (100, 0), (99, 0), (97.5, 0), (97.5, 0), (95, 0), (94.99, 0)],
    'near_in_short_band': [(10, 1), (10, 0), (9, 0), (8, 0), (6, 0), (5.99, 0)],
    'reverse_request': [(100, 0), (100, 1), (102.5, 1), (101, 0), (100, 0)],
    'reverse_motion': [(100, 0), (100, 1), (102.5, 1), (99.9, 1), (99.8, 0)],
    'teleport': [(100, 0), (100, 1), (1000, 1)],
    'zero_initializer': [(0, 0), (0, 1), (1, 1)],
}
rows = {}
for name, steps in cases.items():
    u.mem_write(0x50000000, bytes(32))
    rows[name] = []
    for d, lod in steps:
        put(0x60008000, 'IIfI', 0x5003f000, 0x50000000, d, lod)
        u.reg_write(UC_X86_REG_ESP, 0x60008000)
        u.emu_start(0x301126F0, 0x5003f006, count=1000)
        state = get(0x50000010, 'fBBB')
        fraction = get(0x50002000, 'f')[0]
        rows[name].append(dict(distance=d, requested=lod, state=state, fraction=fraction, reference=int(fraction*255)))
# Narrow FOV hysteresis, original uploader instructions (no camera-motion inference).
put(0x50003030,'f', 1.0)
fov_rows=[]
for degrees in [60,30,30,30,30,30,60,60,60,60,60]:
    put(0x50003030,'f', degrees*3.141592653589793/180)
    u.reg_write(UC_X86_REG_EDI,0x50003000)
    u.reg_write(UC_X86_REG_XMM1,struct.unpack('<I',struct.pack('<f',.7))[0])
    u.emu_start(0x30151DE8,0x30151E3A,count=100)
    fov_rows.append(dict(degrees=degrees,frames=get(0x302DCD70,'I')[0]))
put(0x302DCD70,'I',1)
u.mem_write(0x50000000,bytes(32))
skip=[]
for d,lod in [(100,0),(100,1),(102.5,1),(102,0)]:
    put(0x60008000,'IIfI',0x5003f000,0x50000000,d,lod)
    u.reg_write(UC_X86_REG_ESP,0x60008000)
    u.emu_start(0x301126F0,0x5003f006,count=1000)
    skip.append(dict(distance=d,requested=lod,state=get(0x50000010,'fBBB'),fraction=get(0x50002000,'f')[0]))
# Bounded full executable-section scan: the .3/.05 cvars are registration-only.
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
md.detail = True
md.skipdata = True
refs = []
for section in p.sections:
    if not section.Characteristics & 0x20000000: continue
    for ins in md.disasm(section.get_data(), base + section.VirtualAddress):
        if ins.id and any(o.type == capstone.x86.X86_OP_MEM and o.mem.disp in (1704,1708) for o in ins.operands):
            refs.append(dict(address=hex(ins.address), instruction=ins.mnemonic+' '+ins.op_str))
out = dict(sha256=hashlib.sha256(DLL.read_bytes()).hexdigest(), cvar_references=refs, cases=rows, narrow_fov_frames=fov_rows, suppressed=skip)
golden = Path(__file__).with_name('native_lod_golden.json')
if '--check' in sys.argv:
    assert out == json.loads(golden.read_text()), 'Native DLL/state changed; inspect before accepting.'
    print('PASS: original mesh LOD state, stationary holds, reversal, near band, teleport and registration-only time/speed cvars')
else:
    golden.write_text(json.dumps(out,indent=2)+'\n')
    print(json.dumps(out,indent=2))
