"""Offline original 1.2 SetTabButtonAlpha storage and draw-invariance oracle.
Executes original registration slice, typed two-float marshaller (including
CryScriptSystem type conversion), class binding, setter, configuration copy,
script-chat frame, inherited draw preparation/render traversal and layer pushes.
Lua VM primitives, host casts, geometry/capture queries, font submission and
render-device submission are explicit endpoints. No live process or original
file writes. Requires pefile, capstone, unicorn; run from the repository root.
"""
import hashlib
import json
import pathlib
import struct

import pefile
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE, UC_HOOK_MEM_READ
from unicorn.x86_const import *

PATH = pathlib.Path('<client12-unpacked>/x2game_unpacked.dll')
CRY = pathlib.Path('<client12-unpacked>/bin/cryscriptsystem.dll')
pe = pefile.PE(str(PATH)); base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()
cry = pefile.PE(str(CRY))
u = Uc(UC_ARCH_X86, UC_MODE_32)
D, STACK, STOP, STUB = 0x100000, 0x20f000, 0x4000, 0x5000
W, B, ADD, C, COPY = [D + i * 0x4000 for i in range(5)]
FH, FVT, HOST, HVT, SYS, SVT, REG, THUNK = [D + 0x20000 + i * 0x1000 for i in range(8)]
DRAW, DVT, TEMP, MANAGER = D + 0x30000, D + 0x31000, D + 0x32000, 0x3a5bb0a0
for a, n in [(0, 0x10000), (D, 0x100000), (0x200000, 0x10000),
             (base, (len(image) + 4095) & ~4095),
             (cry.OPTIONAL_HEADER.ImageBase, (cry.OPTIONAL_HEADER.SizeOfImage + 4095) & ~4095)]:
    u.mem_map(a, n)
u.mem_write(base, image)
u.mem_write(cry.OPTIONAL_HEADER.ImageBase, cry.get_memory_mapped_image())
u.mem_write(STUB + 0x100, b'\xdd\x05' + struct.pack('<I', TEMP) + b'\xc3')


def put(a, fmt, *v): u.mem_write(a, struct.pack('<' + fmt, *v))
def get(a, fmt): return struct.unpack('<' + fmt, u.mem_read(a, struct.calcsize('<' + fmt)))
def bits(v): return struct.unpack('<I', struct.pack('<f', v))[0]
def string(a): return bytes(u.mem_read(a, 80)).split(b'\0')[0].decode()
def finish(value=0, args=0):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, value)
    u.reg_write(UC_X86_REG_EIP, get(sp, 'I')[0])
    u.reg_write(UC_X86_REG_ESP, sp + 4 + args)


arguments, calls, reads = [], [], []
cast, inside, captured = True, False, False


def hook(uc, a, n, _):
    sp = u.reg_read(UC_X86_REG_ESP); obj = u.reg_read(UC_X86_REG_ECX)
    if a in [STOP, 0x3925af68, 0x392cb8ac, 0x392cb9ed]: uc.emu_stop()
    elif a == STUB:
        descriptor = get(sp + 4, 'I')[0]
        words = get(descriptor, '10I')
        calls.append(['register', string(words[0]), string(words[1]), hex(words[8]),
                      [hex(v) for v in get(words[9], '2I')]])
        u.mem_write(THUNK, bytes(u.mem_read(words[9], 8)))
        finish(args=4)
    elif a == STUB + 0x10: finish(HOST)
    elif a == STUB + 0x20: finish(W if cast else 0)
    elif a == STUB + 0x30: calls.append(['zero_results']); finish()
    elif a == STUB + 0x60: finish(0, 4)
    elif a == 0x35546ae0: finish(len(arguments))
    elif a == 0x35546cc0:
        i = get(sp + 8, 'i')[0]
        value = arguments[i - 1] if i <= len(arguments) else None
        calls.append(['get_type', i])
        finish(0 if value is None else 1 if isinstance(value, bool)
               else 3 if isinstance(value, (float, int)) else 4)
    elif a == 0x35546ed0:
        put(TEMP, 'd', arguments[get(sp + 8, 'i')[0] - 1])
        u.reg_write(UC_X86_REG_EIP, STUB + 0x100)
    elif a in [0x3550b450, 0x3550ac50]: finish()
    elif a == 0x3921ac90: finish(TEMP + 0x100)
    elif a == 0x392774c0: finish(int(inside), 4)
    elif a == 0x392aa1f0: finish(SYS)
    elif a == 0x392a69e0: finish(int(captured), 4)
    elif a == 0x392c96b0: finish(2)
    elif a == 0x3927de60:
        # Button/text/animation frame primitives are separate already-audited paths.
        # Script chat frame is invoked directly below with its original body.
        finish(args=4)
    elif a == 0x3927b8b0: calls.append(['event', obj, get(sp + 4, 'I')[0]]); finish(args=4)
    elif a == 0x39280de0: calls.append(['text', obj, get(obj + 0xd8, '2f')]); finish(args=4)
    elif a == 0x3921f030: finish()  # Device color submission; native stack remains real.
    elif a == 0x39277a10:
        put(get(sp + 4, 'I')[0], '4f', 100., 100., 500., 280.)
        finish(args=4)
    elif a in [0x3921f480, 0x3921f1e0]: finish()  # Device clip push/pop.
    elif a == STUB + 0x40:
        calls.append(['draw_prepare', obj]); finish(args=4)
    elif a == STUB + 0x50:
        calls.append(['draw', obj, get(0x39ac6804, 'f')[0]]); finish(args=4)
    elif a == 0x39507a60: finish(MANAGER)


def memory_read(uc, access, address, size, value, _):
    if address < W + 0x360 and address + size > W + 0x358:
        reads.append([hex(uc.reg_read(UC_X86_REG_EIP)), hex(address), size])


u.hook_add(UC_HOOK_CODE, hook)
u.hook_add(UC_HOOK_MEM_READ, memory_read)


def call(entry, obj=W, args=()):
    put(STACK, str(len(args) + 1) + 'I', STOP, *args)
    u.reg_write(UC_X86_REG_ESP, STACK); u.reg_write(UC_X86_REG_ECX, obj)
    try: u.emu_start(entry, 0, count=500000)
    except Exception as e: raise RuntimeError((hex(u.reg_read(UC_X86_REG_EIP)), calls)) from e
    assert u.reg_read(UC_X86_REG_EIP) in [STOP, 0x3925af68, 0x392cb8ac, 0x392cb9ed], (hex(u.reg_read(UC_X86_REG_EIP)), calls)


def setup():
    global cast, inside, captured
    u.mem_write(D, bytes(0x100000)); arguments.clear(); calls.clear(); reads.clear()
    cast, inside, captured = True, False, False
    u.reg_write(UC_X86_REG_FPCW, 0x037f)
    put(FH, 'I', FVT); put(FH + 8, 'I', SYS); put(HOST, 'I', HVT); put(SYS, 'I', SVT)
    for off, entry in [(4, STUB + 0x10), (0x14, STUB + 0x60),
                       (0x18, 0x3550a460), (0x2c, STUB + 0x30)]:
        put(FVT + off, 'I', entry)
    put(HVT + 0x64, 'I', STUB + 0x20); put(SVT + 0x50, 'I', STUB)
    put(REG + 0x44, 'I', SYS)
    put(THUNK, '2I', REG, 0x39258d90)
    for j, (obj, vt) in enumerate([(W, 0x399e7400), (B, 0x399c5c00),
                                  (ADD, 0x399c5c00), (C, 0x399e7400)]):
        put(obj, 'I', vt); put(obj + 0xba, 'B', 1)
        put(obj + 0xd8, '2f', .8, .9)
        put(obj + 0x178, '3f', .2, 1., .83)
        put(obj + 0x16c, '4B', 1, 1, 1, 0)
        vector = D + 0x34000 + 0x100 * j
        for layer in range(4):
            draw = DRAW + 0x100 * j + 0x20 * layer
            put(draw, 'I', DVT); put(vector + layer * 4, 'I', draw)
            put(obj + 0xec + layer * 0x20, '2I', vector + layer * 4, vector + layer * 4 + 4)
            put(obj + 0xfc + layer * 0x20, '3f', .61, .3, .9)
            put(obj + 0x108 + layer * 0x20, 'B', 1)
    put(DVT + 0x108, 'I', STUB + 0x40); put(DVT + 0xf0, 'I', STUB + 0x50)
    put(W + 0xc8, '2I', TEMP + 0x200, TEMP + 0x208)
    put(TEMP + 0x200, '2I', B, ADD); put(B + 0xc0, 'I', W); put(ADD + 0xc0, 'I', W)
    put(W + 0x2b8, '2I', TEMP + 0x300, TEMP + 0x304); put(TEMP + 0x300, 'I', B)
    put(W + 0x358, '2f', 1., .3)
    for off in [0x394, 0x39c, 0x3a0]: put(W + off, 'I', C)
    put(MANAGER + 0x70, '2f', .7, .5)
    head, node = TEMP + 0x600, TEMP + 0x640
    put(MANAGER + 4, 'I', head); put(head, '3I', node, node, node)
    put(head + 0x11, 'B', 1); put(node, '4I', head, head, head, W)
    put(0x39d3156c, '2I', TEMP + 0x400, TEMP + 0x404); put(TEMP + 0x400, 'I', W)
    put(0x39ac6804, 'f', .4)


def snapshot():
    return {hex(obj): {'alpha_and_fade': get(obj + 0xd8, '2f'),
                      'layers': [get(obj + 0xfc + i * 0x20, 'f')[0] for i in range(4)],
                      'content': get(obj + 0x178, '3f')}
            for obj in [W, B, ADD]}


cases = {}
setup()
put(STACK - 0x100, 'I', 0)
u.reg_write(UC_X86_REG_EBP, STACK - 0x100)
u.reg_write(UC_X86_REG_ESI, REG); u.reg_write(UC_X86_REG_EDI, 0)
u.reg_write(UC_X86_REG_EBX, 0); u.reg_write(UC_X86_REG_EAX, 0)
u.reg_write(UC_X86_REG_ESP, STACK - 0x200)
u.emu_start(0x3925af17, 0, count=10000)
assert calls[0][1:4] == ['SetTabButtonAlpha', 'selectedAlpha, unselectedAlpha', '0x39259fa0'], calls
cases['registration'] = calls.copy()

for label, args in [('ordinary', [.8, .2]), ('raw', [1.5, -.25]),
                    ('zero', [-0., 0.]), ('infinite', [float('inf'), -float('inf')]),
                    ('nan', [float('nan'), float('nan')]), ('extra', [.8, .2, 42]),
                    ('missing', []), ('missing_second', [.8]), ('nil_second', [.8, None]),
                    ('string_first', ['.8', .2]), ('string_second', [.8, '.2']),
                    ('boolean', [True, False])]:
    setup(); arguments.extend(args); before = snapshot()
    call(0x39259fa0, FH, [FH, THUNK])
    stored = get(W + 0x358, '2I')
    accepted = label in ['ordinary', 'raw', 'zero', 'infinite', 'nan', 'extra']
    if accepted and label != 'nan': assert stored == tuple(map(bits, args[:2]))
    if label == 'nan': assert all((v & 0x7fffffff) > 0x7f800000 for v in stored)
    if not accepted: assert stored == (bits(1.), bits(.3))
    assert snapshot() == before and calls[-1] == ['zero_results']
    cases['binding_' + label] = {'stored_bits': stored, 'calls': calls.copy(), 'widgets': snapshot()}

setup(); cast = False; arguments.extend([.1, .2]); call(0x39259fa0, FH, [FH, THUNK])
assert get(W + 0x358, '2I') == (bits(1.), bits(.3)) and calls[-1] == ['zero_results']
cases['wrong_self'] = calls.copy()

setup(); u.reg_write(UC_X86_REG_ESI, W)
u.reg_write(UC_X86_REG_EAX, TEMP + 0x500)
put(TEMP + 0x500, '2f', 4., 5.); put(STACK, 'I', STOP)
u.reg_write(UC_X86_REG_ESP, STACK)
u.emu_start(0x392cb9c2, 0, count=10000)
assert get(W + 0x358, '2I') == (bits(1.), bits(.3))
cases['native_constructor_defaults'] = get(W + 0x358, '2f')

setup(); call(0x392c8730, W, [bits(1.25), bits(-.2)])
u.reg_write(UC_X86_REG_ESI, COPY); u.reg_write(UC_X86_REG_EAX, W)
u.emu_start(0x392cb75b, 0, count=10000)
assert get(COPY + 0x358, '2I') == get(W + 0x358, '2I')
cases['configuration_copy'] = {'stored': get(COPY + 0x358, '2f'), 'reads': reads.copy()}

for over in [False, True]:
    for capture in [False, True]:
        outputs = []
        for pair in [(1., .3), (.05, .95), (1.5, -.25)]:
            setup(); inside, captured = over, capture
            call(0x392c8730, W, list(map(bits, pair))); calls.clear(); reads.clear()
            call(0x395130e0, W, [bits(250.)])
            after_frame = snapshot()
            call(0x3927b420, W, [0])
            call(0x3927b010, W, [0])
            assert not reads, reads
            assert abs(get(0x39ac6804, 'f')[0] - .4) < 1e-6
            assert all(v['alpha_and_fade'] == snapshot()[hex(W)]['alpha_and_fade']
                       for v in snapshot().values())
            outputs.append({'frame': after_frame, 'draw': snapshot(), 'calls': calls.copy(),
                            'restored_alpha': get(0x39ac6804, 'f')[0]})
        assert outputs[0] == outputs[1] == outputs[2]
        cases[f'frame_prepare_draw_hover{over}_capture{capture}'] = outputs[0]

outputs = []
for pair in [(1., .3), (.05, .95), (1.5, -.25)]:
    setup(); inside = True; put(W + 0xa4, 'B', 1)
    call(0x392c8730, W, list(map(bits, pair))); calls.clear(); reads.clear()
    call(0x395130e0, W, [bits(1000.)]); after_frame = snapshot()
    call(0x3927b420, W, [0]); call(0x3927b010, W, [0])
    assert not reads and abs(get(0x39ac6804, 'f')[0] - .4) < 1e-6
    outputs.append({'frame': after_frame, 'draw': snapshot(), 'calls': calls.copy(),
                    'restored_alpha': get(0x39ac6804, 'f')[0]})
assert outputs[0] == outputs[1] == outputs[2]
cases['clipped_frame_prepare_draw_invariance'] = outputs[0]

for shown in [False, True]:
    for owner in [False, True]:
        outputs = []
        for pair in [(1., .3), (.05, .95), (1.5, -.25)]:
            setup(); put(C + 0xba, 'B', int(shown))
            call(0x392c8730, W, list(map(bits, pair))); reads.clear()
            call(0x39502100, MANAGER)
            prepared = snapshot()
            call(0x395130e0, W, [bits(1000.)])
            held_frame = snapshot()
            call(0x3927b010, W, [0])
            call(0x395021b0, MANAGER, [W if owner else COPY])
            restored = snapshot()
            assert not reads, reads
            assert get(W + 0x358, '2I') == tuple(map(bits, pair))
            if not shown:
                assert all(abs(v - .7) < 1e-6 for v in prepared[hex(B)]['layers'][:3])
                assert held_frame[hex(B)]['layers'] == prepared[hex(B)]['layers']
                assert all(abs(v - (.7 if owner else .5)) < 1e-6
                           for v in restored[hex(B)]['layers'][:3])
            outputs.append({'prepared': prepared, 'held_frame': held_frame, 'restored': restored,
                            'draw_calls': calls.copy(), 'enabled': get(W + 0x16c, '4B')})
        assert outputs[0] == outputs[1] == outputs[2]
        cases[f'drag_prepare_restore_inputShown{shown}_owner{owner}'] = outputs[0]

outdir = pathlib.Path('data/extract/aa-ui')
out = outdir / 'native-chat-tab-alpha-oracle-20261023.json'
out.write_text(json.dumps({'provenance': __doc__, 'sha256': hashlib.sha256(PATH.read_bytes()).hexdigest(),
                          'cry_sha256': hashlib.sha256(CRY.read_bytes()).hexdigest(),
                          'fpcw': '037f', 'cases': cases}, indent=2))
md = Cs(CS_ARCH_X86, CS_MODE_32); md.skipdata = True
with (outdir / 'native-chat-tab-alpha-source-20261023.txt').open('w') as f:
    for a, n in [(0x3925af17, 0x51), (0x39259fa0, 0x59), (0x3922f380, 0xf7),
                 (0x39258d90, 0x4a), (0x392c8730, 0x1f), (0x392cb9c2, 0x2b),
                 (0x392cb742, 0x16b), (0x395130e0, 0x91), (0x3927b420, 0x112),
                 (0x3927b010, 0x23c), (0x3927af50, 0x73), (0x392cb310, 0xc2),
                 (0x39502100, 0x172), (0x39512a40, 0x105), (0x395128e0, 0xe)]:
        f.write(f'\nSOURCE {a:08x}\n')
        for i in md.disasm(image[a-base:a-base+n], a):
            f.write(f'{i.address:08x}: {i.mnemonic} {i.op_str}\n')
    f.write('\nCHAT VTABLE inherited prep/draw/text and script frame\n')
    for vt in [0x399c9b40, 0x399e7400, 0x399c5c00]:
        f.write(str((hex(vt), {hex(o): hex(struct.unpack_from('<I', image, vt-base+o)[0])
                              for o in [0x114, 0x13c, 0x154, 0x158]})) + '\n')
    md.detail = True
    f.write('\nSTATIC property accesses in complete chat-widget / script-chat bodies\n')
    for lo, hi in [(0x392c8000, 0x392cd000), (0x39512800, 0x39513180)]:
        for i in md.disasm(image[lo-base:hi-base], lo):
            if i.id and any(o.type == 3 and 0x358 <= o.mem.disp <= 0x35c for o in i.operands):
                f.write(f'{i.address:08x}: {i.mnemonic} {i.op_str}\n')
    cry_image = cry.get_memory_mapped_image(); cry_base = cry.OPTIONAL_HEADER.ImageBase
    for a, n in [(0x3550a460, 0x6b), (0x3550bf90, 0x170)]:
        f.write(f'\nCRYSCRIPTSYSTEM SOURCE {a:08x}\n')
        for i in md.disasm(cry_image[a-cry_base:a-cry_base+n], a):
            f.write(f'{i.address:08x}: {i.mnemonic} {i.op_str}\n')
print(len(cases), 'native groups PASS', out)
