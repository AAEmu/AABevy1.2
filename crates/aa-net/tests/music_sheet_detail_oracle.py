"""Execute original 1.2 MusicSheet archives, constructors and item-task projection.

Run from repo root: python -B crates/aa-net/tests/music_sheet_detail_oracle.py
Requires pefile/capstone/unicorn for regeneration only. No process/server/socket or
original client installation is accessed. Pinned read-only RE images supply code
and captured sentinel values. Only archive primitives, memcpy/memset and observer
sinks are mocked. Actual constructors, serializers, dispatch, physical lookup,
assignment and output-copy instructions execute. Nonzero reserved bytes are
deliberate synthetic inputs, not an assertion about server-generated padding.
"""
from pathlib import Path
import hashlib
import json
import struct

from capstone import Cs, CS_ARCH_X86, CS_MODE_32
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP,
)

OUT = Path(__file__).parent / 'fixtures/music-sheet-detail'
OUT.mkdir(parents=True, exist_ok=True)
DLL = Path('<client12-unpacked>/x2game_unpacked.dll')
SHA = '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
RUNTIME = Path('<re-notes>/housing-audit/native12-data-readonly.bin')
RUNTIME_SHA = '7082573df48d7d5229e1f909845733f0cf9ded6ea1a4e3c41a5e276fccde8c9e'
assert hashlib.sha256(DLL.read_bytes()).hexdigest() == SHA
runtime = RUNTIME.read_bytes()
assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
p = pefile.PE(str(DLL))
base, image = p.OPTIONAL_HEADER.ImageBase, p.get_memory_mapped_image()
u = Uc(UC_ARCH_X86, UC_MODE_32)
u.mem_map(base, (len(image) + 4095) & ~4095)
u.mem_write(base, image)
for address, size in [(0x10000000, 0x40000), (0x20000000, 0x10000), (0x30000000, 0x10000)]:
    u.mem_map(address, size)
for address, size in [(0x3a5e1178, 12), (0x39adacf0, 12)]:
    u.mem_write(address, runtime[address - 0x39ac3000:address - 0x39ac3000 + size])


def put(address, *values):
    u.mem_write(address, struct.pack('<' + 'I' * len(values), *(v & 0xffffffff for v in values)))


def get(address):
    return struct.unpack('<I', u.mem_read(address, 4))[0]


def ret(value=0, pop=0):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, value)
    u.reg_write(UC_X86_REG_EIP, get(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4 + pop)


STOP, STACK = 0x3000fff0, 0x2000ff00
manager, row, context, observer = 0x10001000, 0x10010000, 0x10011000, 0x10012000
output, live = row + 0x1a8, manager + 0x20 + 3 * 0xc0
archive, vt, scratch, union = 0x10020000, 0x10021000, 0x10022000, 0x10023000
put(context, 0, manager)
put(observer, observer + 0x100)
for i in range(4):
    put(observer + 0x100 + i * 4, 0x30002000 + i * 16)
put(archive, vt)
for off in (0x1c, 0x38, 0x3c, 0x44, 0x48, 0x4c, 0xcc, 0xd0, 0xd8, 0xdc):
    put(vt + off, 0x30000100 + off)
wire, incoming, at, reading, trace = bytearray(), b'', 0, False, []


def hook(_, address, size, data):
    global at
    sp = u.reg_read(UC_X86_REG_ESP)
    arg = lambda i: get(sp + 4 + i * 4)
    if address == 0x398b5446:
        u.mem_write(arg(0), bytes(u.mem_read(arg(1), arg(2))))
        trace.append(['copy', arg(2)])
        ret(arg(0))
    elif address == 0x398b562a:
        u.mem_write(arg(0), bytes([arg(1) & 255]) * arg(2))
        ret(arg(0))
    elif 0x30002000 <= address <= 0x30002030:
        trace.append(['observer', bytes(u.mem_read(live + 0x18, 128)).hex()])
        ret(pop=8 if address == 0x30002020 else 4)
    elif 0x30000100 <= address <= 0x300001dc:
        off = address - 0x30000100
        if off == 0x1c:
            ret(int(reading))
            return
        ptr = arg(1)
        if off in (0xd8, 0xdc):
            assert arg(2) == 128  # Original serializer selected fixed capacity.
            if reading:
                length = struct.unpack_from('<h', incoming, at)[0]
                at += 2
                assert length == 128
            else:
                wire.extend(struct.pack('<h', arg(2)))
            width = 128
        elif off in (0xcc, 0xd0):
            width = arg(2)  # Actual tag switch selects the opaque prefix size.
        else:
            width = {0x38: 8, 0x3c: 4, 0x44: 1, 0x48: 8, 0x4c: 4}[off]
        if reading:
            assert at + width <= len(incoming)
            u.mem_write(ptr, incoming[at:at + width])
            at += width
        else:
            wire.extend(u.mem_read(ptr, width))
        ret(width, pop=12)


u.hook_add(UC_HOOK_CODE, hook)


def call(address, this, *args):
    put(STACK, STOP, *args)
    u.reg_write(UC_X86_REG_ESP, STACK)
    u.reg_write(UC_X86_REG_ECX, this)
    u.emu_start(address, STOP, count=30000)
    assert u.reg_read(UC_X86_REG_EIP) == STOP, 'instruction budget exhausted'
    return u.reg_read(UC_X86_REG_EAX) & 255


def write_archive(address, this):
    global reading
    reading = False
    wire.clear()
    call(address, this, archive)
    return bytes(wire)


def read_archive(address, this, body):
    global reading, incoming, at
    reading, incoming, at = True, body, 0
    call(address, this, archive)
    assert at == len(body)


# Both default constructors clear the entire union. Tag8 itself imposes no ID
# validity/range check: zero, high bits, MAX and the live gap's note7 all round-trip.
for ctor in (0x397ccbf0, 0x3901cb10):
    u.mem_write(scratch, bytes([0xa5]) * 0xc0)
    call(ctor, scratch)
    assert bytes(u.mem_read(scratch + 0x18, 128)) == bytes(128)
for note in (0, 7, 0x8877665544332211, 0xffffffffffffffff):
    prefix = b'\x08' + struct.pack('<Q', note)
    u.mem_write(union, bytes([0xa5]) * 128)
    read_archive(0x3961edf0, union, prefix)
    assert bytes(u.mem_read(union + 9, 119)) == bytes([0xa5]) * 119
    assert write_archive(0x3961edf0, union) == prefix

rows, cases = [], []
for name, action, note in [('add', 5, 7), ('take', 6, 7), ('owner', 15, 7),
                           ('update', 9, 7), ('update_zero', 9, 0),
                           ('update_high', 9, 0x8877665544332211),
                           ('update_max', 9, 0xffffffffffffffff)]:
    # A known physical old row with independent ID and metadata.
    u.mem_write(manager, bytes(0xc000))
    u.mem_write(manager + 0x18, bytes([2, 20, 2]))
    call(0x3901cb10, live)
    put(live, 0x55667788, 0x11223344, 8001)
    u.mem_write(live + 12, bytes([6, 2]))
    put(live + 0x10, 5, 47)
    put(live + 0x98, 0xfffffff9, 0xffffffff, 11, 0)
    u.mem_write(live + 0xa8, bytes([7]))
    put(live + 0xb0, 12, 0, 0xfffffff3, 0xffffffff)
    old = bytes(u.mem_read(live, 0xc0))
    block = b'\x08' + struct.pack('<Q', note) + bytes((i * 17 + 3) & 255 for i in range(119))
    u.mem_write(row, bytes(0x300))
    put(row, action)
    payload = row + 8
    put(payload, (2 << 8) | (3 << 24))
    if action == 9:
        put(payload + 8, 999, 0)  # Deliberately different packet ID.
        u.mem_write(payload + 0x10, block)
        serializer = 0x397cfd40
    else:
        src = payload + 8
        call(0x397ccbf0, src)
        put(src, 28051)
        put(src + 8, 0x05060708, 0x01020304)
        u.mem_write(src + 0x10, bytes([8, 4]))
        put(src + 0x14, 1)
        u.mem_write(src + 0x18, block)
        put(src + 0x98, 0xfffffff1, 0xffffffff, 22, 0)
        put(src + 0xa8, 55)
        u.mem_write(src + 0xac, bytes([7]))
        put(src + 0xb0, 66, 0, 77, 0)
        serializer = 0x397d0fe0 if action == 15 else 0x397d0f90
    body = write_archive(serializer, payload)
    # Actual read side, overwriting every byte of the padded union after ctor.
    u.mem_write(payload, bytes(0x180))
    if action != 9:
        call(0x397ccbf0, payload + 8)
    read_archive(serializer, payload, body)
    offset = 0x10 if action == 9 else 0x20
    assert bytes(u.mem_read(payload + offset, 128)) == block
    trace.clear()
    admitted = call(0x397d1030, row, context, observer)
    assert admitted == 1
    assert bytes(u.mem_read(live + 0x18, 128)) == block
    assert bytes(u.mem_read(output + 0x18, 128)) == block
    expected = bytearray(old)
    if action == 9:
        expected[0x18:0x98] = block
        # Native copy excludes compiler padding; every serialized field survives.
        assert bytes(u.mem_read(live, 0xc0)) == expected
    else:
        assert get(live) == 0x05060708 and get(live + 8) == 28051
    assert all(bytes.fromhex(event[1]) == old[0x18:0x98]
               for event in trace if event[0] == 'observer')
    expected_item = write_archive(0x3961f500, live)
    assert write_archive(0x3961f500, output) == expected_item
    rows.append(f'{name}\t{(bytes([action]) + body).hex()}\t{expected_item.hex()}')
    cases.append({'name': name, 'action': action, 'note_id': note,
                  'admitted': True, 'raw_detail_block': block.hex(), 'trace': list(trace)})

(OUT / 'native.tsv').write_text(
    '# original x2game SHA256 ' + SHA + '\n# case\ttask wire hex\tpost-write Item wire hex\n'
    + '\n'.join(rows) + '\n', encoding='utf8')
(OUT / 'native.json').write_text(json.dumps({
    'code_sha256': SHA, 'runtime_sha256': RUNTIME_SHA,
    'provenance': __doc__, 'cases': cases,
}, indent=2) + '\n', encoding='utf8')
c = Cs(CS_ARCH_X86, CS_MODE_32)
source = ['Original x2game SHA256 ' + SHA, 'All ranges are half-open.']
for start, end in [(0x3961edf0, 0x3961ee89), (0x397ccbf0, 0x397ccc65),
                   (0x394eb0e2, 0x394eb137),  # Actual use consumes tag8's full u64.
                   (0x3901cb10, 0x3901cb99), (0x397d0a00, 0x397d0b57),
                   (0x397d0f90, 0x397d1029), (0x397cfd40, 0x397cfdc9),
                   (0x397cda20, 0x397cdacc), (0x397cdc10, 0x397cdc8e),
                   (0x397cdef0, 0x397cdfb0), (0x397ad440, 0x397ad4fa),
                   (0x3901c0a0, 0x3901c153), (0x397d1030, 0x397d1249)]:
    source.append(f'\n{start:08x}..{end:08x}')
    source.extend(f'{i.address:08x}: {i.mnemonic} {i.op_str}'
                  for i in c.disasm(image[start - base:end - base], start))
for action in (5, 6, 9, 15):
    target = struct.unpack_from('<I', image, 0x397d124c - base + (action - 1) * 4)[0]
    source.append(f'dispatch action{action} -> {target:08x}')
source.append('tag8 -> ' + hex(struct.unpack_from('<I', image, 0x3961ee8c - base + 7 * 4)[0]))
(OUT / 'source.txt').write_text('\n'.join(source) + '\n', encoding='utf8')
print('MusicSheet native oracle: 2 constructors, 4 tag8 read/write, 7 padded task read/write/dispatch cases passed')
