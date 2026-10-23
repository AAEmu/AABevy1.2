"""Execute retained original 1.2 particle code in a private x86 Unicorn VM.

No client process is opened or changed. Requires pefile, capstone and unicorn.
Run from the repository root; --write refreshes the adjacent executed evidence.
The DLL is the aa-fx round-6 retained original, never a newer CryEngine build.
"""
import argparse
import hashlib
import json
import struct
from pathlib import Path

import capstone
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_ESP, UC_X86_REG_EIP

ROOT = Path(__file__).resolve().parents[3]
DLL = ROOT / "data/extract/aa-fx/r6/ida/cry3dengine.dll"
OUT = Path(__file__).resolve().parent


class Native:
    def __init__(self):
        self.pe = pefile.PE(str(DLL))
        self.vm = Uc(UC_ARCH_X86, UC_MODE_32)
        size = (self.pe.OPTIONAL_HEADER.SizeOfImage + 4095) & -4096
        self.vm.mem_map(self.pe.OPTIONAL_HEADER.ImageBase, size)
        self.vm.mem_write(self.pe.OPTIONAL_HEADER.ImageBase, self.pe.get_memory_mapped_image())
        self.crt_path = Path('C:/Windows/SysWOW64/msvcr100.dll')
        crt = pefile.PE(str(self.crt_path))
        base = crt.OPTIONAL_HEADER.ImageBase
        self.vm.mem_map(base, (crt.OPTIONAL_HEADER.SizeOfImage + 4095) & -4096)
        self.vm.mem_write(base, crt.get_memory_mapped_image())
        exp = next(e.address for e in crt.DIRECTORY_ENTRY_EXPORT.symbols if e.name == b'__libm_sse2_expf')
        self.u32(0x3024C144, base + exp)
        self.vm.mem_map(0x10000000, 0x100000)
        self.vm.mem_map(0x20000000, 0x100000)
        self.next = 0x10000000
        self.invalidations = 0
        self.vm.hook_add(UC_HOOK_CODE, self.hook)
        self.env = self.alloc(512)
        self.u32(0x30358168, self.env)
        self.u32(self.env + 144, 10)
        self.u32(0x302DCD40, self.alloc(2048))
        # A fixed seed at the imported RNG boundary; clock fixtures below have
        # zero variation, so no numerical clock result depends on this seed.
        random = self.alloc(16)
        self.vm.mem_write(random, bytes.fromhex('b801000000c3'))
        self.u32(0x3024C2FC, random)

    def hook(self, vm, address, size, _):
        # Bounds invalidation has no bearing on target selection. Do not enter
        # the engine's renderer/resource ownership from this isolated fixture.
        if address == 0x30061F90:
            self.invalidations += 1
            sp = vm.reg_read(UC_X86_REG_ESP)
            vm.reg_write(UC_X86_REG_EIP, self.get_u32(sp))
            vm.reg_write(UC_X86_REG_ESP, sp + 4)

    def alloc(self, size):
        p = self.next
        self.next = (p + size + 15) & -16
        return p

    def u32(self, p, v):
        self.vm.mem_write(p, struct.pack('<I', v))

    def f32(self, p, v):
        self.vm.mem_write(p, struct.pack('<f', v))

    def vec(self, p, v):
        self.vm.mem_write(p, struct.pack('<' + 'f' * len(v), *v))

    def get_u32(self, p):
        return struct.unpack('<I', self.vm.mem_read(p, 4))[0]

    def floats(self, p, n):
        return list(struct.unpack('<' + 'f' * n, self.vm.mem_read(p, n * 4)))

    def call(self, address, this=0, args=()):
        sp = 0x200F0000
        self.u32(sp, 0x200FF000)
        for i, arg in enumerate(args):
            self.u32(sp + 4 + 4 * i, arg)
        self.vm.reg_write(UC_X86_REG_ECX, this)
        self.vm.reg_write(UC_X86_REG_ESP, sp)
        self.vm.emu_start(address, 0x200FF000, count=100_000)
        assert self.vm.reg_read(UC_X86_REG_EIP) == 0x200FF000
        return self.vm.reg_read(UC_X86_REG_EAX)

    def emitter(self, parent=0, mode=0, start=0.0, end=100.0, now=1.0,
                offset=(0, 0, 0), translation=(0, 0, 0), velocity=(0, 0, 0),
                scale=1.0, ignore=False):
        sub = self.alloc(160)
        container = self.alloc(280)
        params = self.alloc(840)
        source = self.alloc(176)
        effect = self.alloc(600)
        empty = self.alloc(8) + 4
        self.u32(sub + 8, container)
        self.u32(sub + 12, parent)
        self.u32(sub + 16, source)
        self.u32(container + 32, params)
        self.u32(container + 156, empty)
        self.u32(source + 124, effect)
        self.f32(source + 152, 1000)
        self.vec(source + 8, (0, 0, 0, 1))
        self.vec(source + 24, translation)
        self.f32(source + 36, scale)
        self.vec(source + 128, velocity)
        self.vm.mem_write(params + 701, bytes([mode]))
        self.vm.mem_write(params + 726, bytes([ignore]))
        self.vec(params + 368, offset)
        self.f32(params + 60, 1)
        self.f32(sub + 60, start)
        self.f32(sub + 64, end)
        self.f32(sub + 116, now)
        self.u32(sub + 120, 10)
        return sub, params, source, effect

    def resolve(self, sub):
        self.call(0x3008A430, sub)
        return {'position': self.floats(sub + 20, 3),
                'velocity': self.floats(sub + 32, 3),
                'flags': list(self.vm.mem_read(sub + 44, 6))}


def run():
    n = Native()
    bits = lambda x: struct.unpack('<I', struct.pack('<f', x))[0]
    clocks = []
    for continuous, life, delay, pulse in [(True, .5, .25, 0), (False, .5, 0, 0),
                                         (True, 0, 0, 0), (True, .2, .1, .5)]:
        sub, params, _, _ = n.emitter(now=0)
        n.u32(n.env + 144, 0)
        n.vm.mem_write(params + 5, bytes([continuous]))
        n.f32(params + 36, life)
        n.f32(params + 44, delay)
        n.f32(params + 52, pulse)
        n.call(0x3008B410, sub, (bits(0),))
        clocks.append({'continuous': continuous, 'life': life, 'delay': delay, 'pulse': pulse,
                       'start_end_pulse': n.floats(sub + 60, 3)})
    n.u32(n.env + 144, 10)
    cases = []
    for label, now, start, end in [('before', .249, .25, .75), ('start', .25, .25, .75),
                                  ('end', .75, .25, .75), ('lapsed', .751, .25, .75),
                                  ('zero-at', 0, 0, 0), ('zero-after', .001, 0, 0)]:
        source, params, _, _ = n.emitter(mode=3, now=now, start=start, end=end,
                                         offset=(0, 0, 1.5), translation=(2, 3, 4),
                                         velocity=(.5, 1, 2), scale=2)
        # Count=0 deliberately: target production is independent of emission.
        n.f32(params + 24, 0)
        out = n.alloc(32)
        ok = n.call(0x3008A3B0, source, (out,)) & 255
        cases.append({'name': label, 'accepted': ok, 'target': n.floats(out, 6),
                      'flags': list(n.vm.mem_read(out + 24, 6))})
    far, _, _, _ = n.emitter(mode=3, offset=(1, 0, 0))
    near, _, _, _ = n.emitter(parent=far, mode=3, offset=(2, 0, 0))
    child, _, _, effect = n.emitter(parent=near)
    n.vec(effect + 356, (9, 8, 7, 6, 5, 4))
    n.vm.mem_write(effect + 380, bytes([1, 0, 1, 1, 0, 1]))
    cases.append({'name': 'nearest-over-supplied', **n.resolve(child)})
    n.vm.mem_write(child + 45, b'\1')
    cases.append({'name': 'priority-supplied', **n.resolve(child)})
    n.f32(near + 64, .5)
    cases.append({'name': 'lapsed-nearest-falls-back', **n.resolve(child)})
    ignored, _, _, _ = n.emitter(parent=near, ignore=True)
    cases.append({'name': 'ignore-attractor', **n.resolve(ignored)})
    # Real x86 acceleration helper, both no-drag and drag branches. The latter
    # calls the actual imported 32-bit CRT expf; no host approximation is used.
    acceleration = []
    for drag in (0.0, 2.0):
        out, distance, relative, wind = [n.alloc(16) for _ in range(4)]
        n.vec(distance, (2, 1, -1))
        n.vec(relative, (1, 0, 0))
        n.vec(wind, (0, 0, 0))
        n.call(0x30060420, args=(out, distance, relative, bits(2), wind, bits(drag)))
        acceleration.append({'drag': drag, 'value': n.floats(out, 3)})
    updates = []
    for label, velocity, target_velocity, drag, dt in [('steer', (1, 0, 0), (0, 0, 0), 0, .1),
                                                      ('drag', (1, 0, 0), (0, 0, 0), 2, .1),
                                                      ('moving', (1, 0, 0), (.5, 0, 0), 0, .1),
                                                      ('shorten', (8, 4, -4), (0, 0, 0), 0, .1),
                                                      ('arrival', (8, 4, -4), (0, 0, 0), 0, .3)]:
        sub, params, _, _ = n.emitter()
        particle, context = n.alloc(192), n.alloc(256)
        empty = n.alloc(16) + 8
        for offset in (76, 108, 112):
            n.u32(params + offset, empty)
        n.u32(particle + 168, sub)
        n.vec(particle + 12, velocity)
        n.f32(particle + 40, 2)
        n.f32(particle + 44, .5)
        n.f32(particle + 52, .25)
        n.f32(particle + 92, 1)
        n.vm.mem_write(particle + 150, bytes([240]))
        n.f32(params + 416, drag)
        n.f32(params + 248, 1)
        n.vec(sub + 20, (2, 1, -1))
        n.vec(sub + 32, target_velocity)
        n.vm.mem_write(sub + 44, bytes([1, 0, 0, 0, 1, 0]))
        alive = n.call(0x3005CD20, particle, (context, bits(dt))) & 255
        updates.append({'name': label, 'input_velocity': velocity, 'target_velocity': target_velocity,
                        'drag': drag, 'dt': dt, 'alive': alive, 'position': n.floats(particle, 3),
                        'velocity': n.floats(particle + 12, 3),
                        'life': n.floats(particle + 40, 1)[0], 'age': n.floats(particle + 44, 1)[0]})
    modes = []
    for address in range(0x3029030C, 0x3029033C, 12):
        value, name, _ = struct.unpack('<III', n.vm.mem_read(address, 12))
        modes.append([value, bytes(n.vm.mem_read(name, 80)).split(b'\0')[0].decode('ascii')])
    result = {'dll': str(DLL.relative_to(ROOT)), 'sha256': hashlib.sha256(DLL.read_bytes()).hexdigest(),
              'crt': str(n.crt_path), 'crt_sha256': hashlib.sha256(n.crt_path.read_bytes()).hexdigest(),
              'unicorn': __import__('unicorn').__version__, 'force_enum': modes, 'clocks': clocks, 'selection': cases,
              'acceleration': acceleration, 'updates': updates,
              'bounds_invalidations_skipped': n.invalidations}
    return result


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--write', action='store_true')
    args = p.parse_args()
    result = run()
    output = json.dumps(result, indent=2) + '\n'
    print(output)
    if args.write:
        (OUT / 'force_generation_native.json').write_text(output, encoding='utf-8')
        rows = ['# name drag input_velocity(3) target_velocity(3) output_position(3) output_velocity(3) life age']
        for u in result['updates']:
            if not u['alive']:
                continue
            row = [u['name'], u['drag'], *u['input_velocity'], *u['target_velocity'], *u['position'], *u['velocity'], u['life'], u['age']]
            rows.append('\t'.join(map(str, row)))
        (OUT / 'force_generation_updates.tsv').write_text('\n'.join(rows) + '\n', encoding='utf-8')
        n = Native()
        dis = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
        ranges = [(0x30089F80, 0x3008A0C0), (0x3008A3B0, 0x3008A540),
                  (0x3008EA70, 0x3008EB80), (0x30060420, 0x300605A0),
                  (0x3008B410, 0x3008B6E0)]
        text = []
        for start, end in ranges:
            text += [f'{i.address:08x} {i.bytes.hex():24} {i.mnemonic} {i.op_str}'.rstrip()
                     for i in dis.disasm(bytes(n.vm.mem_read(start, end - start)), start)]
        (OUT / 'force_generation_native.asm').write_text('\n'.join(text) + '\n', encoding='utf-8')
        source_path = ROOT / 'data/extract/aa-fx/r6/ida/cry3dengine.c'
        source = source_path.read_text(encoding='utf-8')
        excerpts = ['Retained original 1.2 Hex-Rays source evidence',
                    f'Source: {source_path.relative_to(ROOT)}',
                    f'Source SHA256: {hashlib.sha256(source_path.read_bytes()).hexdigest()}',
                    f'DLL SHA256: {result["sha256"]}']
        for name in ('sub_30070D30', 'sub_3008A3B0', 'sub_3008A430', 'sub_3008B410',
                     'sub_30060420', 'sub_300600B0'):
            start = source.index('// ===== ' + name + ' ')
            end = source.index('// ===== ', start + 10)
            excerpts.append(source[start:end])
        start = source.index('      if ( v415[24] != 0 )')
        end = source.index('      v112 = ', start)
        excerpts += ['Target consumer inside sub_3005CD20:', source[start:end]]
        (OUT / 'force_generation_source.txt').write_text('\n\n'.join(excerpts).rstrip() + '\n', encoding='utf-8')
