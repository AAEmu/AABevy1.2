"""Read-only terrain quality contract and original x86 group-apply oracle.

python crates/aa-world/tools/terrain_quality_source.py
Requires the retained original DLLs in <client12-unpacked>; writes only ignored research.
The oracle executes native option selection and CVarGroup callback/iteration, with
explicit map-lookup and console-write endpoints. No client/server process or GPU.
"""
from pathlib import Path
import hashlib
import json
import struct
import subprocess

import capstone
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "data/extract/aa-world/terrain-quality/source"
OUT.mkdir(parents=True, exist_ok=True)
provenance = {}


def digest(path):
    data = path.read_bytes()
    provenance[str(path)] = hashlib.sha256(data).hexdigest()
    return data


tables = {}
for config in ("config", "config64"):
    path = f"game/{config}/cvargroups/option_terrain_detail.cfg"
    data = subprocess.run([str(ROOT / "tools/aa-pak.exe"), "cat", path], check=True, capture_output=True).stdout
    (OUT / f"{config}-option_terrain_detail.cfg").write_bytes(data)
    provenance[path] = hashlib.sha256(data).hexdigest()
    group = "default"
    groups = {}
    for line in data.decode("utf-8-sig").splitlines():
        line = line.split(";", 1)[0].strip()
        if line.startswith("["):
            group = line.strip("[]")
        elif "=" in line:
            key, value = (s.strip() for s in line.split("=", 1))
            if key:
                groups.setdefault(group, {})[key] = value
    tables[config] = groups

game = digest(Path("<client12-unpacked>/x2game_unpacked.dll"))
system = digest(Path("<client12-unpacked>/ida/crysystem.dll"))
source_path = Path("<client12-unpacked>/ida/crysystem.c")
source = digest(source_path).decode("utf-8-sig")
pe = pefile.PE(data=game)
mapped = pe.get_memory_mapped_image()
base = pe.OPTIONAL_HEADER.ImageBase
disassembler = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
instructions = []
for start, end in ((0x39206060, 0x39206086), (0x39205740, 0x39205973)):
    instructions.append(f"FUNCTION {start:08X}")
    for instruction in disassembler.disasm(mapped[start-base:end-base], start):
        instructions.append(f"{instruction.address:08X} {instruction.mnemonic:8s} {instruction.op_str}")
(OUT / "native-indexed-setter.asm").write_text("\n".join(instructions) + "\n")
addresses = ("36553940", "365780D0", "36577EF0", "36582D20", "3657DA20", "36583710", "36582AB0", "36581AD0")
excerpts = []
for address in addresses:
    start = source.index(f"// ==== {address} ")
    end = source.find("// ==== ", start + 12)
    excerpts.append(source[start:end])
(OUT / "native-group-apply-source.c").write_text("\n".join(excerpts), encoding="utf-8")

u = Uc(UC_ARCH_X86, UC_MODE_32)
for data in (game, system):
    pe = pefile.PE(data=data)
    base = pe.OPTIONAL_HEADER.ImageBase
    mapped = pe.get_memory_mapped_image()
    u.mem_map(base, (len(mapped) + 4095) & ~4095)
    u.mem_write(base, mapped)
u.mem_map(0, 0x10000)
u.mem_map(0x10000000, 0x200000)
heap = 0x10010000
STACK, STOP, CONSOLE, CVAR, WRITE = 0x101f0000, 0x1000fff0, 0x10001000, 0x10003000, 0x10002010


def put(address, *values):
    u.mem_write(address, struct.pack("<" + "I" * len(values), *values))


def get(address):
    return struct.unpack("<I", u.mem_read(address, 4))[0]


def string(address):
    return bytes(u.mem_read(address, 256)).split(b"\0")[0].decode()


def alloc(data):
    global heap
    address = heap
    heap += (len(data) + 15) & ~15
    u.mem_write(address, data)
    return address


def ret(value=0, pop=0):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, value)
    u.reg_write(UC_X86_REG_EIP, get(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4 + pop)


def tree(entries):
    """Minimal original MSVC tree layout, sorted right chain, sentinel at +21."""
    header = alloc(bytes(32))
    u.mem_write(header + 21, b"\1")
    nodes = []
    for key, value in sorted(entries):
        node = alloc(bytes(32))
        put(node, header, header, header, key, value)
        nodes.append(node)
    for index, node in enumerate(nodes):
        put(node + 4, nodes[index - 1] if index else header)
        put(node + 8, nodes[index + 1] if index + 1 < len(nodes) else header)
    put(header, nodes[0] if nodes else header, nodes[0] if nodes else header, nodes[-1] if nodes else header)
    return header


def section(values):
    fields = [(alloc(k.encode() + b"\0"), alloc(v.encode() + b"\0")) for k, v in sorted(values.items())]
    address = alloc(bytes(12))
    put(address + 4, tree(fields), len(fields))
    return address


trace = []
selected_names = []


def hook(uc, address, size, _):
    sp = u.reg_read(UC_X86_REG_ESP)
    if address == 0x39205740:
        # Native indexed wrapper has already selected the option name. The option
        # manager's map/string implementation is an explicit endpoint here.
        selected_names.append(string(get(sp + 4)))
        ret(1, 12)
    elif address == 0x365833E0:
        # CVarGroup's integer-level lookup; the callback/entry traversal is original.
        result, key = get(sp + 4), get(get(sp + 8))
        header = get(u.reg_read(UC_X86_REG_ECX) + 4)
        node = get(header + 4)
        while node != header and get(node + 12) < key:
            node = get(node + 8)
        found = node if node != header and get(node + 12) == key else header
        put(result, found)
        ret(result, 8)
    elif address == 0x365814D0:
        ret(alloc(b"option_terrain_detail\0"))
    elif address == WRITE:
        trace.append((string(get(sp + 4)), string(get(sp + 8))))
        ret(1, 8)
    elif address == 0x365C4580:
        # std::map lower_bound for default-vs-selected exclusion.
        key = string(get(get(sp + 4)))
        header = get(u.reg_read(UC_X86_REG_ECX) + 4)
        node = get(header + 4)
        while node != header and string(get(node + 12)) < key:
            node = get(node + 8)
        ret(node, 4)


# Resolve only imports encountered by the original callback's sys_spec prefix and
# default exclusion checks. Unknown imports remain unmapped and fail closed.
for data in (game, system):
    pe = pefile.PE(data=data)
    for dll in pe.DIRECTORY_ENTRY_IMPORT:
        for item in dll.imports:
            if item.name in (b"strncmp", b"_strnicmp", b"strnicmp", b"strcmp"):
                endpoint = alloc(b"\xcc" * 16)
                put(item.address, endpoint)
                def compare(uc, address, size, _, endpoint=endpoint, name=item.name):
                    if address != endpoint:
                        return
                    sp = u.reg_read(UC_X86_REG_ESP)
                    a, b = string(get(sp + 4)), string(get(sp + 8))
                    if name != b"strcmp":
                        n = get(sp + 12)
                        a, b = a[:n].lower(), b[:n].lower()
                    ret((int(a > b) - int(a < b)) & 0xffffffff)
                u.hook_add(UC_HOOK_CODE, compare)
u.hook_add(UC_HOOK_CODE, hook)
put(CONSOLE, CONSOLE + 0x100)
put(CONSOLE + 0x100 + 236, WRITE)
put(0x366381B4, CONSOLE + 0x200)
put(CONSOLE + 0x200 + 28, CONSOLE)


def call(address, this, *args):
    put(STACK, STOP, *args)
    u.reg_write(UC_X86_REG_ESP, STACK)
    u.reg_write(UC_X86_REG_ECX, this)
    u.emu_start(address, STOP, count=100000)
    assert u.reg_read(UC_X86_REG_EIP) == STOP


rows = []
for option in (11, 12):
    call(0x39206060, 0, option, struct.unpack("<I", struct.pack("<f", 1.0))[0], 1)
assert selected_names == ["option_terrain_detail", "e_detail_materials_view_dist_xy"], selected_names
groups = tables["config64"]
defaults = section(groups["default"])
sections = [(int(k), section(v)) for k, v in groups.items() if k != "default"]
put(CVAR, 0x366178F0)
put(CVAR + 296 + 4, get(defaults + 4))
put(CVAR + 316, tree(sections))
for level in range(1, 5):
    put(CVAR + 32, level)
    trace.clear()
    call(0x36582AB0, 0, CVAR)
    actual = dict(trace)
    expected = dict(groups["default"], **groups.get(str(level), {}))
    assert actual == expected, (level, actual, expected)
    assert len(trace) == len(expected), (level, trace)
    rows.append(dict(level=level, writes=list(trace)))
assert [dict(r["writes"])["e_detail_materials_view_dist_xy"] for r in rows] == ["256", "512", "1024", "1024"]
assert [dict(r["writes"])["e_detail_materials_view_dist_z"] for r in rows] == ["64", "128", "128", "256"]
result = dict(provenance=provenance, option_names=selected_names, tables=tables, native_callback=rows,
              boundaries="Original indexed option wrapper; map/string setter endpoint; original group callback and apply loop; explicit section lookup/comparison and console-write endpoints. Group registration and Set(float)->Set(int)->callback linkage retained as original source, not executed here.")
(OUT / "contract.json").write_text(json.dumps(result, indent=2) + "\n")
print("PASS: native option11 selects full terrain-detail group; original callback applies both XY/Z at all four levels")
