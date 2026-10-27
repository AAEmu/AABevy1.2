"""Regenerate the tiny item-record fixtures by executing the original 1.2 x86 reader.

Optional research tool, not a Cargo test dependency. Requires pefile + unicorn (2.1.4).
python crates/aa-ui/tests/fixtures/item_record_native.py [read-only x2game_unpacked.dll]
No client/server process is opened. Semantic input values below are explicit test vectors.
"""
import pathlib
import struct
import sys

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP

SOURCE = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else "<client12-unpacked>/x2game_unpacked.dll")
PE = pefile.PE(str(SOURCE))
IMAGE = PE.get_memory_mapped_image()
BASE = PE.OPTIONAL_HEADER.ImageBase
CTOR, READER, MEMSET, ENCODER = 0x3901CB10, 0x3961F500, 0x398B562A, 0x394DF700
STACK, HEAP, STUBS, STOP = 0x100000, 0x200000, 0x300000, 0x300F00
RECORD, STREAM, VTABLE = HEAP, HEAP + 0x1000, HEAP + 0x2000


def fixture(kind, detail):
    wire = struct.pack("<IQBBiB", 24488, 0x8877665544332211, 4, 0x18, 17, kind)
    wire += detail + struct.pack("<qiIBqq", 1710000000, 360, 12345, 2, 1710000300, 1710000600)
    uc = Uc(UC_ARCH_X86, UC_MODE_32)
    uc.mem_map(BASE, (len(IMAGE) + 4095) & ~4095)
    uc.mem_write(BASE, IMAGE)
    uc.mem_map(STACK, 0x10000)
    uc.mem_map(HEAP, 0x10000)
    uc.mem_map(STUBS, 0x1000)
    uc.mem_write(STREAM, struct.pack("<I", VTABLE))
    for slot in [0x1C, 0x38, 0x3C, 0x44, 0x48, 0x4C, 0xCC]:
        uc.mem_write(VTABLE + slot, struct.pack("<I", STUBS + slot))
    position = 0
    calls = []

    def read_u32(address):
        return struct.unpack("<I", uc.mem_read(address, 4))[0]

    def return_from_call(clean=0):
        sp = uc.reg_read(UC_X86_REG_ESP)
        uc.reg_write(UC_X86_REG_EIP, read_u32(sp))
        uc.reg_write(UC_X86_REG_ESP, sp + 4 + clean)

    def hook(_uc, address, _size, _user):
        nonlocal position
        sp = uc.reg_read(UC_X86_REG_ESP)
        if address == MEMSET:
            dest, value, count = [read_u32(sp + i) for i in [4, 8, 12]]
            uc.mem_write(dest, bytes([value & 255]) * count)
            return_from_call()
        elif STUBS <= address < STUBS + 0x100:
            slot = address - STUBS
            if slot == 0x1C:  # ISerialize::IsReading
                uc.reg_write(UC_X86_REG_EAX, 1)
                return_from_call()
                return
            count = read_u32(sp + 12) if slot == 0xCC else {0x38: 8, 0x3C: 4, 0x44: 1, 0x48: 8, 0x4C: 4}[slot]
            dest = read_u32(sp + 8)
            assert position + count <= len(wire)
            uc.mem_write(dest, wire[position:position + count])
            calls.append((dest - RECORD, count))
            position += count
            return_from_call(12)

    uc.hook_add(UC_HOOK_CODE, hook)

    def run(entry, *args):
        sp = STACK + 0xF000
        uc.mem_write(sp, struct.pack("<" + "I" * (len(args) + 1), STOP, *args))
        uc.reg_write(UC_X86_REG_ESP, sp)
        uc.reg_write(UC_X86_REG_ECX, RECORD)
        uc.emu_start(entry, STOP, count=200000)
        assert uc.reg_read(UC_X86_REG_EIP) == STOP, hex(uc.reg_read(UC_X86_REG_EIP))

    # Prove these gaps are uninitialized alignment, not native zero-valued instance fields.
    uc.mem_write(RECORD, b"\xCD" * 192)
    run(CTOR)
    gaps = [range(14, 16), range(169, 176)]
    initialized = bytes(uc.mem_read(RECORD, 192))
    assert all(initialized[i] == 0xCD for gap in gaps for i in gap)
    assert initialized[24:152] == bytes(128)
    assert initialized[164:168] == bytes(4)  # Upper half of the lifespan qword is ctor-zeroed.
    # App canonicalization chooses zero for the untransported alignment bytes only.
    for gap in gaps:
        uc.mem_write(RECORD + gap.start, bytes(len(gap)))
    run(READER, STREAM)
    assert position == len(wire), (position, len(wire))
    output = bytes(uc.mem_read(RECORD, 192))
    assert output[24] == kind
    print(f"kind{kind}: consumed{position}, native field writes {calls}")
    uc.reg_write(UC_X86_REG_EAX, 192)  # Native encoder's register length argument.
    run(ENCODER, HEAP + 0x3000, 383, RECORD)
    encoded = bytes(uc.mem_read(HEAP + 0x3000, 257))
    assert encoded[256] == 0
    return output, encoded[:256]


vectors = {
    "none": (0, b""),
    "equip": (1, struct.pack("<IB2sIqI7IHH", 14698, 83, b"\0\0", 567, 1709999999, 678,
                              101, 202, 303, 404, 505, 606, 707, 1234, 4321)),
    "mate": (3, struct.pack("<iBB", 54321, 1, 42)),
    "opaque": (4, bytes(range(1, 10))),
}
for name, args in vectors.items():
    path = pathlib.Path(__file__).with_name(f"item-record-{name}.bin")
    record, encoded = fixture(*args)
    path.write_bytes(record)
    path.with_suffix('.txt').write_bytes(encoded)
    print(path, path.stat().st_size)
