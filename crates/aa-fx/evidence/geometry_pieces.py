"""Execute original 1.2 compound-mesh emission and pivot code in isolated Unicorn.

Resource vtables, allocation admission and the particle-constructor boundary are
fixture inputs. Eligibility, iteration, transform decomposition/composition,
counts and bounding-box pivot math execute the retained original instructions.
No live process or original client file is changed. --write retains the results.
"""
import argparse
import hashlib
import json
import math
import struct

import capstone
import pefile
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_ESP, UC_X86_REG_EIP

from force_generation import Native, ROOT, DLL, OUT


class Pieces(Native):
    def __init__(self):
        self.handlers = {}
        self.created = []
        self.allocations = 0
        self.admit = 10000
        self.accept = True
        super().__init__()
        # These are real CRT static-initializer routines from this DLL. The
        # quaternion SIMD sign masks live in zero-initialized data until run.
        self.call(0x30247650)
        self.call(0x302476B0)
        random = self.get_u32(0x3024C2FC)
        self.vm.mem_write(random, bytes.fromhex('b800000080c3'))
        crt = pefile.PE(str(self.crt_path))
        pow_addr = next(e.address for e in crt.DIRECTORY_ENTRY_EXPORT.symbols if e.name == b'__libm_sse2_pow')
        self.u32(0x3024C200, crt.OPTIONAL_HEADER.ImageBase + pow_addr)
        self.handlers[0x30063E60] = self.allocate_particle
        self.handlers[0x3005F860] = self.construct_particle
        self.handlers[0x30064040] = lambda: self.ret(0, 4)
        # Runtime emission density is an environmental input held at one.
        self.one = self.alloc(4)
        self.f32(self.one, 1)
        self.fpu_one = self.alloc(16)
        self.vm.mem_write(self.fpu_one, b'\xd9\x05' + struct.pack('<I', self.one) + b'\xc3')
        self.handlers[0x3008F280] = lambda: self.vm.reg_write(UC_X86_REG_EIP, self.fpu_one)

    def ret(self, value, arg_bytes=0):
        sp = self.vm.reg_read(UC_X86_REG_ESP)
        self.vm.reg_write(UC_X86_REG_EAX, value)
        self.vm.reg_write(UC_X86_REG_EIP, self.get_u32(sp))
        self.vm.reg_write(UC_X86_REG_ESP, sp + 4 + arg_bytes)

    def hook(self, vm, address, size, _):
        if address in self.handlers:
            self.handlers[address]()
        else:
            super().hook(vm, address, size, _)

    def stub(self, handler):
        p = self.alloc(16)
        self.vm.mem_write(p, b'\xc3')
        self.handlers[p] = handler
        return p

    def allocate_particle(self):
        self.allocations += 1
        self.ret(self.alloc(192) if self.allocations <= self.admit else 0, 8)

    def construct_particle(self):
        sp = self.vm.reg_read(UC_X86_REG_ESP)
        args = [self.get_u32(sp + 4 + i * 4) for i in range(7)]
        self.created.append({'geometry': args[4], 'transform_xyzw_xyz_scale': self.floats(args[3], 8),
                             'accepted': self.accept})
        self.ret(int(self.accept), 28)

    def resource(self, children, bounds=(-1, -2, -3, 3, 4, 5)):
        obj, vtable, bbox = self.alloc(512), self.alloc(512), self.alloc(24)
        self.vec(bbox, bounds)
        self.u32(obj, vtable)
        self.u32(vtable + 252, self.stub(lambda: self.ret(len(children))))
        def lookup():
            sp = self.vm.reg_read(UC_X86_REG_ESP)
            index = self.get_u32(sp + 4)
            self.ret(children[index] if index < len(children) else 0, 4)
        self.u32(vtable + 264, self.stub(lookup))
        self.u32(vtable + 156, self.stub(lambda: self.ret(bbox, 4)))
        return obj

    def fixture(self, specs, pieces=True, frame=(0, 0, 0, 1, 0, 0, 0, 1)):
        self.sub = self.alloc(176)
        self.container = self.alloc(300)
        self.params = self.alloc(850)
        self.parent = self.alloc(256)
        self.u32(self.sub + 8, self.container)
        self.u32(self.sub + 16, self.parent)
        self.u32(self.container + 32, self.params)
        self.vm.mem_write(self.params + 116, bytes([int(pieces)]))
        self.vec(self.parent + 8, frame)
        children = []
        for kind, present, tm in specs:
            child = self.alloc(128)
            geometry = self.resource([]) if present else 0
            self.u32(child, kind)
            self.vec(child + 64, tm)
            self.u32(child + 112, geometry)
            children.append(child)
        self.geometry = self.resource(children)
        self.u32(self.params + 800, self.geometry)
        return self.sub

    def emit(self, supplied=0):
        self.created.clear()
        self.allocations = 0
        result = self.call(0x3008AB30, self.sub, (0, 0, 0, 0, supplied, 0))
        # Native resource pointers are fixture identity, not stable evidence.
        created = []
        for c in self.created:
            c = dict(c)
            geometry = c.pop('geometry')
            c['geometry_argument'] = ('default_parent' if geometry == 0 else
                                      'parent' if geometry == self.geometry else 'child_or_supplied')
            created.append(c)
        return {'returned': result, 'allocation_attempts': self.allocations, 'created': created}

    def capacity(self, count):
        self.u32(self.container + 16, self.alloc(64))
        self.u32(self.get_u32(self.container + 16) + 48, 1)
        self.u32(self.container + 32, self.params)
        self.u32(self.container + 164, self.parent)
        self.f32(self.params + 24, count)
        self.call(0x300619B0, self.container)
        return self.get_u32(self.container + 48)

    def pivot(self, no_offset, facing=0, quaternion=(0, 0, 0, 1)):
        p, output, emitter = self.alloc(192), self.alloc(32), self.alloc(256)
        self.u32(p + 168, self.sub)
        self.u32(p + 172, self.geometry)
        self.u32(self.parent + 124, emitter)
        self.u32(emitter + 112, 1)
        self.f32(emitter + 128, 2)
        self.f32(self.params + 248, 1)
        self.vm.mem_write(p + 144, bytes([240]))
        self.vec(p, (10, 20, 30))
        self.vec(p + 80, quaternion)
        self.vm.mem_write(self.params + 728, bytes([int(no_offset)]))
        self.vm.mem_write(self.params + 676, bytes([facing]))
        self.call(0x3005BFE0, p, (output,))
        return self.floats(output, 8)


IDENTITY = (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0)


def run():
    eligibility = [(0, True, IDENTITY), (1, True, IDENTITY), (0, False, IDENTITY), (0, True, IDENTITY)]
    cases = []
    for name, specs, pieces, admit, accept, supplied in [
        ('two_meshes_skip_helper_and_null', eligibility, True, 10000, True, False),
        ('false_chooses_one_child', eligibility, False, 10000, True, False),
        ('no_subobjects_uses_parent', [], True, 10000, True, False),
        ('helper_only_emits_nothing', [(1, True, IDENTITY)], True, 10000, True, False),
        ('allocator_exhaustion_stops_in_order', eligibility, True, 1, True, False),
        ('constructor_rejection_is_not_counted', eligibility, True, 10000, False, False),
        ('supplied_geometry_bypasses_split', eligibility, True, 10000, True, True),
    ]:
        n = Pieces()
        n.fixture(specs, pieces)
        capacity = n.capacity(2.25)
        n.admit, n.accept = admit, accept
        cases.append({'name': name, 'capacity_for_count_2_25': capacity,
                      **n.emit(n.resource([]) if supplied else 0)})
    transforms = []
    for name, tm, frame in [
        ('translated_child', (1, 0, 0, 2, 0, 1, 0, -3, 0, 0, 1, 4), (0, 0, 0, 1, 10, 20, 30, 2)),
        ('rotated_scaled_child_and_parent', (0, -3, 0, 2, 2, 0, 0, -3, 0, 0, 4, 4),
         (0, 0, math.sqrt(.5), math.sqrt(.5), 10, 20, 30, 2)),
    ]:
        n = Pieces()
        n.fixture([(0, True, tm)], frame=frame)
        transforms.append({'name': name, 'child_matrix_3x4': tm, 'parent_xyzw_xyz_scale': frame, **n.emit()})
    n = Pieces()
    n.fixture([])
    pivots = {'bounds': [-1, -2, -3, 3, 4, 5], 'particle_xyz': [10, 20, 30], 'size': 2,
              'centred': n.pivot(False), 'no_offset': n.pivot(True),
              'rigid_body_bypasses_centre': n.pivot(False, 3),
              'centred_rotated': n.pivot(False, quaternion=(0, 0, math.sqrt(.5), math.sqrt(.5)))}
    assert [c['returned'] for c in cases] == [2, 1, 1, 0, 1, 0, 1]
    assert cases[0]['capacity_for_count_2_25'] == 6
    assert cases[1]['capacity_for_count_2_25'] == 3
    assert transforms[0]['created'][0]['transform_xyzw_xyz_scale'][4:] == [14, 14, 38, 2]
    assert pivots['centred'][4:7] == [8, 18, 28]
    assert pivots['no_offset'][4:7] == [10, 20, 30]
    return {'dll': str(DLL.relative_to(ROOT)), 'dll_sha256': hashlib.sha256(DLL.read_bytes()).hexdigest(),
            'crt_sha256': hashlib.sha256(n.crt_path.read_bytes()).hexdigest(),
            'boundaries': 'fixture resource vtables; allocator admission; constructor acceptance; environmental density=1; CryRandom=0x80000000',
            'emission': cases, 'transforms': transforms, 'pivots': pivots}


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    result = run()
    output = json.dumps(result, indent=2) + '\n'
    print(output)
    if args.write:
        (OUT / 'geometry_pieces_native.json').write_text(output, encoding='utf8')
        rows = ['# name child_matrix_3x4(12) parent_xyzw_xyz_scale(8) output_xyzw_xyz_scale(8)']
        for t in result['transforms']:
            rows.append('\t'.join(map(str, [t['name'], *t['child_matrix_3x4'], *t['parent_xyzw_xyz_scale'],
                                           *t['created'][0]['transform_xyzw_xyz_scale']])))
        (OUT / 'geometry_pieces_transforms.tsv').write_text('\n'.join(rows)+'\n', encoding='utf8')
        source_path = ROOT / 'data/extract/aa-fx/r6/ida/cry3dengine.c'
        source = source_path.read_text(encoding='utf8')
        excerpts = [f'Original 1.2 DLL SHA256: {result["dll_sha256"]}',
                    f'Hex-Rays source SHA256: {hashlib.sha256(source_path.read_bytes()).hexdigest()}']
        for name in ('sub_3008AB30', 'sub_300619B0', 'sub_3005BFE0', 'sub_30061430'):
            start = source.index('// ===== ' + name + ' ')
            end = source.index('// ===== ', start + 10)
            excerpts.append(source[start:end])
        start = source.index('    v173 = *(float *)(v38 + 132);')
        end = source.index('    Src = ', start)
        excerpts.append('CStatObj::LoadFromContentCGF subobject construction: v170 is the 188-byte subobject; '
                        'v173 starts at +16 (world matrix), v185 at +64 (local matrix), v197 at +112 (mesh resource).\n'
                        + source[start:end])
        (OUT / 'geometry_pieces_source.txt').write_text('\n\n'.join(excerpts), encoding='utf8')
        n = Pieces()
        dis = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
        lines = []
        for start, end in [(0x3008AB30, 0x3008B400), (0x300619B0, 0x30061B1C), (0x3005BFE0, 0x3005C1B0)]:
            lines.extend(f'{i.address:08x} {i.bytes.hex():24} {i.mnemonic} {i.op_str}'.rstrip()
                         for i in dis.disasm(bytes(n.vm.mem_read(start, end-start)), start))
        (OUT / 'geometry_pieces_native.asm').write_text('\n'.join(lines)+'\n', encoding='utf8')
