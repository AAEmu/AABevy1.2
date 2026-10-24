"""Execute 477 bounded original Illum input controls; no host DLL/device/scene.

Requires pefile and unicorn. Reads only the hash-pinned original 32-bit renderer;
--dll selects its retained path. Writes no files. See docs/research/illum-pass-inputs.md.
"""
from pathlib import Path
import argparse
import hashlib
import itertools
import json
import struct
from types import SimpleNamespace
import pefile
import unicorn as u
from unicorn.x86_const import *

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
SHA = 'ebf051bba5d349a6b3bf6d4eb676908115b73bb22d35d5a0cc917c325426589d'
DEFAULT_DLL = REPO / 'data/extract/aa-cry/ida/render/cryrenderd3d10.dll'
GOLDEN = ROOT / 'evidence/illum_inputs/native-cases.json'
NATIVE_RANGES = [(0x38178cd0, 0x38179070), (0x3817d100, 0x3817d26f),
                 (0x380733b0, 0x3807350f), (0x38073510, 0x38073661),
                 (0x380d6db1, 0x380d6e1a), (0x380d88fe, 0x380d8912),
                 (0x38003a5a, 0x38003a8e)]
n = SimpleNamespace(BASE=0x38000000, R=0x60000000, S=0x61000000,
                    M=0x62000000, O=0x63000000, T=0x64000000,
                    P=0x65000000, SYS=0x66000000, STACK=0x70000000,
                    END=0x71000000, GFX=0x71001000, SHA=SHA)


def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()


def initialize(dll):
    raw = dll.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == SHA, 'wrong original renderer bytes'
    pe = pefile.PE(data=raw)
    assert pe.FILE_HEADER.Machine == 0x14c and pe.OPTIONAL_HEADER.ImageBase == n.BASE
    n.IM = pe.get_memory_mapped_image()
    n.DLL = dll


def new():
    c = u.Uc(u.UC_ARCH_X86, u.UC_MODE_32)
    c.mem_map(n.BASE, (len(n.IM) + 4095) & ~4095)
    c.mem_write(n.BASE, n.IM)
    c.mem_map(0, 0x1000)  # Isolated mapped page; no host TEB.
    for a in [n.R, n.S, n.M, n.O, n.T, n.P, n.SYS, n.STACK, n.END]:
        c.mem_map(a, 0x20000)
    return c


def write(c, address, fmt, *values):
    c.mem_write(address, struct.pack(fmt, *values))


def read(c, address, fmt='<I'):
    return struct.unpack(fmt, c.mem_read(address, struct.calcsize(fmt)))[0]


def common(c):
    for address, value in [(0x383aa9e0, n.SYS), (0x383aa9e8, n.R), (0x383b17c4, n.R),
                           (0x383c7028, 1), (0x3838c478, 1), (0x3838b608, 1),
                           (n.R + 0x1da8, n.S), (n.R + 0x1db0, n.O),
                           (n.R + 0x1dd4, n.M), (n.R + 0x1dc4, n.T),
                           (n.R + 0x2050, 0), (n.R + 0x3068, 0x21),
                           (n.M + 48, n.M + 0x1000), (n.R, n.R + 0x10000),
                           (n.R + 0x10000 + 1576, n.GFX)]:
        write(c, address, '<I', value)
    for address, value in [(0x384fe700, 1 << 40), (0x384fe680, 1 << 8), (0x384fe688, 1 << 9)]:
        write(c, address, '<Q', value)
    write(c, n.O + 8, '<i', -1)
    for address in [n.M + 0x1000 + 12, n.O + 12, n.R + 65568]:
        write(c, address, '<f', 1.)


def ret(c, count=0, result=0):
    sp = c.reg_read(UC_X86_REG_ESP)
    target = read(c, sp)
    c.reg_write(UC_X86_REG_ESP, sp + 4 + count)
    c.reg_write(UC_X86_REG_EAX, result)
    c.reg_write(UC_X86_REG_EIP, target)


n.new, n.common, n.write, n.read, n.ret = new, common, write, read, ret

def execute(c, entry, end=n.END, args=(), ranges=None, hooks=None, frame=None):
    if frame is None:
        sp = n.STACK + 0x10000
        n.write(c, sp, '<' + 'I' * (1 + len(args)), n.END, *args)
        c.reg_write(UC_X86_REG_ESP, sp)
        c.reg_write(UC_X86_REG_EBP, 0)
        c.reg_write(UC_X86_REG_ECX, n.R)
    c.reg_write(UC_X86_REG_FPCW, 0x37f)
    c.reg_write(UC_X86_REG_MXCSR, 0x1f80)
    trace = []
    endpoints = []
    ranges = ranges or [(entry, end)]

    def code(c, ea, size, _):
        trace.append(ea)
        if hooks and ea in hooks:
            hooks[ea](c, endpoints)
        elif not any(a <= ea < b for a, b in ranges):
            raise AssertionError(f'unexpected original execution target {ea:08x}')

    h = c.hook_add(u.UC_HOOK_CODE, code)
    try:
        c.emu_start(entry, end, timeout=2_000_000, count=50_000)
    finally:
        c.hook_del(h)
    assert c.reg_read(UC_X86_REG_EIP) == end, hex(c.reg_read(UC_X86_REG_EIP))
    return dict(instructions=len(trace), trace_sha256=hashlib.sha256(
        struct.pack('<' + 'I' * len(trace), *trace)).hexdigest(), endpoints=endpoints)


def f32(x):
    return struct.unpack('<f', struct.pack('<f', x))[0]


def qualifier(c, mode, alias, flags, decal, opacity, recursion):
    item = n.S + 0x10000
    n.write(c, item, '<4I', n.S, n.M, 0, 0)
    n.write(c, n.S + 64, '<II', n.S + 0x1000, 1)
    n.write(c, n.S + 0x1000, '<I', n.T)
    n.write(c, n.S + 36, '<I', 0x80 if decal else 0)
    n.write(c, n.S + 40, '<I', 0)
    n.write(c, n.T + 28, '<b', 1)
    n.write(c, n.T + 35, '<b', alias)
    n.write(c, n.O, '<I', flags)
    n.write(c, n.M + 0x1000 + 12, '<f', opacity)
    n.write(c, 0x383c7028, '<I', recursion)
    n.write(c, 0x383b1284, '<i', mode)
    trace = execute(c, 0x38178cd0, args=(item, n.O, 0, 0), ranges=[(0x38178cd0, 0x38179070)])
    out = c.reg_read(UC_X86_REG_EAX)
    expected = recursion == 1 and alias > 0 and bool(flags & 0x80000000) and mode != 0 \
        and not decal and not flags & 0x40000400
    assert bool(out & 0x100) == expected, (mode, alias, hex(flags), decal, opacity, recursion, hex(out))
    return dict(mode=mode, signed_technique_alias=alias, object_flags=hex(flags),
                shader_decal=decal, material_opacity=opacity, recursion=recursion,
                batch_flags=hex(out), motion=bool(out & 0x100), **trace)


def motion_case(values):
    c = n.new()
    n.common(c)
    return qualifier(c, *values)


def frame_sync(mode, manager):
    c = n.new()
    n.common(c)
    n.write(c, 0x383b1294, '<i', mode)
    n.write(c, 0x383b1284, '<I', 0x12345678)
    # Null manager or an actually empty vector takes the original early return.
    n.write(c, n.R + 0x103f8, '<I', n.P if manager else 0)
    if manager:
        n.write(c, n.P + 0x5c, '<II', n.P + 0x1000, n.P + 0x1000)
    trace = execute(c, 0x3817d100, ranges=[(0x3817d100, 0x3817d26f)])
    output = n.read(c, 0x383b1284, '<i')
    assert output == mode
    return dict(configured_mode=mode, empty_manager_present=manager, effective_mode=output, **trace)


def scene_scope(mode, scene_flags):
    c = n.new()
    n.common(c)
    frame = n.STACK + 0x10000
    n.write(c, frame + 8, '<I', scene_flags)
    n.write(c, 0x383b1284, '<i', mode)
    c.reg_write(UC_X86_REG_EBP, frame)
    c.reg_write(UC_X86_REG_EBX, 0)
    c.reg_write(UC_X86_REG_ESP, frame - 0x200)
    tr1 = execute(c, 0x380d6db1, 0x380d6e1a, frame=frame)
    inside = n.read(c, 0x383b1284, '<i')
    assert inside == (0 if scene_flags & 0x40 else mode)
    assert n.read(c, frame - 0x44, '<i') == mode
    # This original restore slice only needs its already-saved motion value.
    c.reg_write(UC_X86_REG_ECX, 0)
    c.reg_write(UC_X86_REG_EDX, 0)
    tr2 = execute(c, 0x380d88fe, 0x380d8912, frame=frame)
    after = n.read(c, 0x383b1284, '<i')
    assert after == mode
    return dict(mode=mode, scene_flags=hex(scene_flags), inside_effective=inside,
                restored_effective=after, save_and_gate=tr1, restore=tr2,
                boundary='two original slices; no scene draw or its intervening work executed')


def registration():
    c = n.new()
    n.common(c)
    console, table = n.SYS + 0x1000, n.SYS + 0x2000
    n.write(c, n.SYS + 28, '<I', console)
    n.write(c, console, '<I', table)
    n.write(c, table + 36, '<I', n.GFX)
    c.reg_write(UC_X86_REG_ESP, n.STACK + 0x10000)

    def sink(c, records):
        sp = c.reg_read(UC_X86_REG_ESP)
        a = struct.unpack('<8I', c.mem_read(sp + 4, 32))
        name = bytes(c.mem_read(a[1], 64)).split(b'\0')[0].decode('ascii')
        assert c.reg_read(UC_X86_REG_ECX) == console
        assert name == 'r_MotionBlur' and a[2] == 0x383b1294 and a[3] == 0
        records.append(dict(name=name, string_flags=a[0], storage=hex(a[2]),
                            requested_default=a[3], flags=a[4], description_pointer=hex(a[5]),
                            callback=hex(a[6]), final_argument=a[7],
                            boundary='console sink records call only; no CVar/profile policy executed'))
        n.ret(c, 32)

    tr = execute(c, 0x38003a5a, 0x38003a8e, ranges=[(0x38003a5a, 0x38003a8e)],
                 hooks={n.GFX: sink}, frame=1)
    assert len(tr['endpoints']) == 1
    return tr


def binding_case(kind, threshold, dissolve, bias, positive, forced, shadow, material, list_id, dynamic, global_table):
    c = n.new()
    n.common(c)
    flags = (0x2000000 if positive else 0) | (0x80 if global_table else 0)
    n.write(c, n.O, '<I', flags)
    n.write(c, n.O + 98, '<BB', dissolve, bias)
    n.write(c, n.M + 20, '<f', threshold)
    n.write(c, n.R + 0x1dd4, '<I', n.M if material else 0)
    n.write(c, n.S + 40, '<I', 0x800000 if forced else 0)
    n.write(c, n.R + 0x2054, '<I', 0x100000 if shadow else 0)
    n.write(c, n.R + 0x3070, '<I', list_id)
    n.write(c, 0x383b12c4, '<I', 0)  # r_ssdo off for this isolated PB time-channel control.
    if dynamic is not None:
        n.write(c, n.O + 8, '<I', 0)
        n.write(c, n.O + 100, '<I', 0x100)
        n.write(c, 0x383b1800, '<I', 1)
        n.write(c, 0x383b17fc, '<I', n.P)
        n.write(c, 0x383b0f64, '<I', 1)
        n.write(c, 0x383b0f68, '<I', n.P)
        n.write(c, n.P + 48, '<f', dynamic)
    entry, end = (0x380733b0, 0x3807350f) if kind == 'PI' else (0x38073510, 0x38073661)
    trace = execute(c, entry, ranges=[(entry, end)])
    output_bytes = bytes(c.mem_read(0x38390660, 16))
    output = struct.unpack('<4f', output_bytes)
    scale = f32(1 / 255)
    expected_w = f32(threshold) if material else 0.
    if forced and not shadow:
        expected_w = f32(.51)
    if kind == 'PI' and bias:
        expected_w = f32(f32(f32(bias) * scale) + expected_w)
    if dynamic is not None and list_id in (7, 17):
        expected_w = f32(dynamic)
    expected = (f32(f32(dissolve) * scale) if kind == 'PI' else 0.,
                1. if positive else -1., 0., expected_w)
    expected_bytes = struct.pack('<4f', *expected)
    assert output_bytes == expected_bytes, (kind, output, expected)
    return dict(binding=kind, material_threshold=threshold, dissolve_byte=dissolve, bias_byte=bias,
                sign_positive=positive, forced_shader_bit=forced, shadow=shadow, material=material,
                list_id=list_id, dynamic_alpha=dynamic, global_table=global_table,
                output=list(output), output_hex=output_bytes.hex(), **trace)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--dll', type=Path, default=DEFAULT_DLL)
    args = parser.parse_args()
    initialize(args.dll)
    before = sha(args.dll)
    cases = [motion_case(v) for v in itertools.product(
        [0, 1, 4, -1], [-1, 0, 1], [0, 0x80000000, 0xc0000000, 0x80000400],
        [False, True], [1., .5], [1, 2])]
    sync = [frame_sync(mode, manager) for mode, manager in itertools.product([0, 1, 2, 4, 101, -1], [False, True])]
    scene = [scene_scope(mode, flags) for mode, flags in itertools.product([0, 1, 4, -1], [0, 1, 0x40, 0x41, 0x2040])]
    tuples = [
        # threshold,dissolve,bias,positive,forced,shadow,material,list,dynamic,global
        (.059, 0, 0, False, False, False, True, 2, None, False),
        (.059, 51, 128, True, False, False, True, 2, None, False),
        (.9, 255, 255, False, False, False, True, 2, None, False),
        (.059, 51, 128, False, True, False, True, 2, None, False),
        (.059, 51, 128, True, True, True, True, 2, None, False),
        (.059, 255, 128, False, False, False, False, 2, None, False),
    ]
    tuples.extend((.059, 51, 128, positive, True, shadow, True, li, .73, glob)
                  for li, glob, positive, shadow in itertools.product([2, 7, 17], [False, True], [False, True], [False, True]))
    bindings = [binding_case(kind, *v) for kind in ['PI', 'PB'] for v in tuples]
    result = dict(module_sha256=n.SHA,
                  boundary='original isolated functions/slices; fixture state; no runtime profile, scene, driver, render success or native output claim',
                  registration=registration(), motion_qualifiers=cases,
                  frame_sync=sync, scene_scopes=scene, bindings=bindings)
    assert sha(args.dll) == before
    result['native_ranges'] = {f'{a:08x}..{b:08x}': hashlib.sha256(
        n.IM[a - n.BASE:b - n.BASE]).hexdigest() for a, b in NATIVE_RANGES}
    assert json.loads(GOLDEN.read_text(encoding='utf-8')) == result
    print(json.dumps(dict(motion=len(cases), frame_sync=len(sync), scene_scopes=len(scene),
                          bindings=len(bindings), registration=1,
                          total=len(cases)+len(sync)+len(scene)+len(bindings)+1,
                          golden_sha256=sha(GOLDEN))))


if __name__ == '__main__':
    main()
