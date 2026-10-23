"""Execute original TOD7 -> C3DEngine sun-specular storage, including the real setter.
Requires pefile/unicorn; existing read-only extracted DLL, no game/executable modification.
This proves the binding/storage only, not native terrain GPU pass equivalence.
"""
from pathlib import Path
import hashlib
import json
import struct
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_EBP, UC_X86_REG_ESP, UC_X86_REG_EDI, UC_X86_REG_ESI

root = Path(__file__).resolve().parents[3]
path = root / "data/extract/aa-cry/ida/cry3dengine.dll"
pe = pefile.PE(str(path))
machine = Uc(UC_ARCH_X86, UC_MODE_32)
machine.mem_map(pe.OPTIONAL_HEADER.ImageBase, (pe.OPTIONAL_HEADER.SizeOfImage + 4095) & ~4095)
machine.mem_write(pe.OPTIONAL_HEADER.ImageBase, pe.get_memory_mapped_image())
machine.mem_map(0x50000000, 0x20000)
machine.mem_map(0x60000000, 0x10000)

def put(address, fmt, *values):
    machine.mem_write(address, struct.pack("<" + fmt, *values))

rows = []
for value in [0.0, 0.37, 1.0, 2.5, -1.0]:
    machine.reg_write(UC_X86_REG_ESP, 0x60008000)
    machine.reg_write(UC_X86_REG_EBP, 0x60008800)
    machine.reg_write(UC_X86_REG_EDI, 0x50000000)
    machine.reg_write(UC_X86_REG_ESI, 0x50010000)
    put(0x50000010, "I", 0x50001000)
    put(0x50001074, "f", value)  # TOD7 value at +4+7*16
    put(0x50010000, "I", 0x30263718)  # real C3DEngine vtable
    machine.emu_start(0x300a6130, 0x300a6145, count=20)
    # Dispatches slot+4AC to original 30137970; no mocked call.
    machine.emu_start(0x300a72ef, 0x300a7309, count=50)
    actual = bytes(machine.mem_read(0x50010444, 4))
    expected = struct.pack("<f", value)
    assert actual == expected
    rows.append({"input": value, "stored": struct.unpack("<f", actual)[0], "bits": actual.hex()})
out = {"dll_sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "cases": rows}
(root / "data/extract/aa-world/terrain-spec/native-sun-specular.json").write_text(json.dumps(out, indent=2) + "\n")
print("PASS: 5 original TOD7/virtual-dispatch/setter probes, bit exact")
