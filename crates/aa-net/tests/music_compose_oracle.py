"""Native composition cast writer oracle (no client/server process).

Run from repo root: python crates/aa-net/tests/music_compose_oracle.py
Requires pefile, unicorn, capstone. Executes original39044360 and397cad20,
including397c85b0/397bfa90/397bfd50. Only archive primitives and BC3 are mocked.
Retains full packet goldens and original disassembly at the timing boundaries.
The ordinary skill planner, compilation and runtime callbacks are not emulated.
"""
import hashlib
from pathlib import Path
import struct

from capstone import Cs, CS_ARCH_X86, CS_MODE_32
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP

OUT = Path(__file__).parent / 'fixtures/music'
DLL = Path('<client12-unpacked>/x2game_unpacked.dll')
SHA = '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
assert hashlib.sha256(DLL.read_bytes()).hexdigest() == SHA
pe = pefile.PE(str(DLL))
base = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()
u = Uc(UC_ARCH_X86, UC_MODE_32)
u.mem_map(base, (len(image) + 4095) & ~4095)
u.mem_write(base, image)
for a in [0x10000000, 0x20000000, 0x30000000]:
    u.mem_map(a, 0x10000)


def put(a, *v):
    u.mem_write(a, struct.pack('<' + 'I' * len(v), *(x & 0xffffffff for x in v)))


def get(a):
    return struct.unpack('<I', u.mem_read(a, 4))[0]


def ret(sp, argc):
    u.reg_write(UC_X86_REG_EIP, get(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4 + 4 * argc)


packet, archive, vt = 0x10000000, 0x10003000, 0x10004000
stack, stop = 0x2000ff00, 0x30001000
put(archive, vt)
for off in [0x14, 0x18, 0x1c, 0x38, 0x3c, 0x44, 0x4c, 0x78, 0xd0]:
    put(vt + off, 0x30000000 + off)
wire = bytearray()


def hook(_, a, n, data):
    sp = u.reg_read(UC_X86_REG_ESP)
    if a == 0x391b8780:
        wire.extend(u.mem_read(u.reg_read(UC_X86_REG_ECX), 3))
        ret(sp, 1)
    elif 0x30000000 <= a < 0x30000100:
        off = a - 0x30000000
        if off in [0x14, 0x18, 0x1c]:
            u.reg_write(UC_X86_REG_EAX, int(off == 0x14))
            ret(sp, 2 if off == 0x14 else 0)
        else:
            ptr = get(sp + 8)
            size = get(sp + 12) if off == 0xd0 else {0x38: 8, 0x3c: 4, 0x44: 1, 0x4c: 4, 0x78: 1}[off]
            wire.extend(u.mem_read(ptr, size))
            ret(sp, 3)


u.hook_add(UC_HOOK_CODE, hook)


def run(address, this, *args):
    put(stack, stop, *args)
    u.reg_write(UC_X86_REG_ESP, stack)
    u.reg_write(UC_X86_REG_ECX, this)
    u.emu_start(address, stop, count=10000)
    assert u.reg_read(UC_X86_REG_EIP) == stop


rows = []
for actor, uid, item_grade, maker in [(0x010203, 0xfedcba9876543210, 6, 0x11223344), (0xffffff, 0xffffffffffffffff, 255, 0xffffffff)]:
    for grade in range(11):
        u.mem_write(packet, bytes(512))
        put(packet + 12, 22215)
        run(0x39044360, packet + 16, actor, uid & 0xffffffff, uid >> 32, 27902, item_grade, maker)
        put(packet + 48, 0, 0, actor)  # unit target, same native SaveNotes actor
        put(packet + 104, 0x100, 5, grade)  # local147=1, type5, grade widened
        wire.clear()
        run(0x397cad20, packet, archive)
        expected = struct.pack('<I', 22215) + b'\x02' + actor.to_bytes(3, 'little')
        expected += struct.pack('<QIBI', uid, 27902, item_grade, maker)
        expected += b'\x00' + actor.to_bytes(3, 'little') + b'\x05' + struct.pack('<I', grade)
        assert wire == expected, (wire.hex(), expected.hex())
        rows.append(f'{actor:x}\t{uid:x}\t{item_grade}\t{maker:x}\t{grade}\t{wire.hex()}')
(OUT / 'native-compose.tsv').write_text('\n'.join(rows) + '\n', encoding='utf8')

cs = Cs(CS_ARCH_X86, CS_MODE_32)
# These contiguous excerpts identify registration/received dispatch (not function names),
# the action97 branch, cooldown callback, and native type5 archive width.
ranges = [(0x39079640, 432), (0x39083c60, 166), (0x39081f80, 119),
          (0x391eb9d0, 7829), (0x391bf000, 145), (0x391ab7c0, 101),
          (0x391d58a0, 181), (0x391d57f0, 168),
          (0x3939b010, 2218), (0x3939ce90, 128),
          (0x3939a810, 2036), (0x397bff73, 26)]
source = [f'Original DLL SHA256 {SHA}. Read-only disassembly; no runtime timing inferred from names.']
for a, n in ranges:
    source.append(f'\nSOURCE {a:08x}')
    instructions = list(cs.disasm(image[a-base:a-base+n], a))
    if a == 0x391eb9d0:
        at = next(i for i, ins in enumerate(instructions)
                  if ins.mnemonic == 'call' and ins.op_str == '0x391bf000')
        instructions = instructions[at-5:at+1]
    elif a == 0x3939b010:
        instructions = [i for i in instructions if 0x3939b190 <= i.address <= 0x3939b1e0
                        or 0x3939b5b0 <= i.address <= 0x3939b785]
    elif a == 0x3939a810:
        instructions = [i for i in instructions if 0x3939add0 <= i.address <= 0x3939ae40]
    source.extend(f'{i.address:08x}: {i.mnemonic} {i.op_str}'.rstrip() for i in instructions)
(OUT / 'native-compose-source.txt').write_text('\n'.join(source) + '\n', encoding='utf8')
print(f'{len(rows)} original full CS052 writer cases passed; timing/source excerpts retained')

# Actual lane SkillObject reader, executed by Rust tests with the existing music
# PacketStream fixture plus the exact additional primitive reader methods.
root = Path('<aaemu-src>/rust/crates')
types_path = root / 'aaemu-game/src/systems/skills/types.rs'
stream_path = root / 'aaemu-commons/src/packet_stream.rs'
types = types_path.read_text(encoding='utf8')
stream = stream_path.read_text(encoding='utf8')


def block(text, marker):
    start = text.index(marker)
    brace = text.index('{', start)
    level = 1
    at = brace + 1
    while level:
        level += (text[at] == '{') - (text[at] == '}')
        at += 1
    return text[start:at]


lane = ['// Generated by music_compose_oracle.py: actual lane reader; no handler execution.',
        '#![allow(dead_code)]',
        'use crate::music::tests::lane::{PacketStream, MarshalError};',
        'type ReadResult<T> = Result<T, MarshalError>;']
for path in [types_path, stream_path]:
    lane.append(f'// {path.as_posix()} SHA256 {hashlib.sha256(path.read_bytes()).hexdigest()}')
lane += [block(types, 'pub fn convert_long_back'), block(types, 'pub mod skill_object_type'),
         '#[derive(Debug, Clone, PartialEq, Default)]', block(types, 'pub enum SkillObject'),
         'impl SkillObject {', block(types[types.index('impl SkillObject {'):], 'pub fn read('), '}',
         'impl PacketStream {']
for name in ['read_i32', 'read_i64', 'read_f32', 'read_bool']:
    lane.append(block(stream, f'pub fn {name}(').replace('Result<', 'ReadResult<'))
lane.append('}')
(OUT / 'lane-skill-object.rs').write_text('\n\n'.join(lane) + '\n', encoding='utf8')
