"""Original ambient cold-init sound-proxy path, without a client/audio device.

python crates/aa-world/tools/ambient_proxy_source.py
Writes small ignored evidence under data/extract/aa-world/ambient-proxy-20261025.
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
OUT = ROOT / "data/extract/aa-world/ambient-proxy-20261025"
OUT.mkdir(parents=True, exist_ok=True)
path = ROOT / "data/extract/aa-world/environment/source/bin32/cryentitysystem.dll"
raw = path.read_bytes()
sha = hashlib.sha256(raw).hexdigest()
assert sha == "d1f0623a45c491888b9a2f3de54270d142d17afe2a63dd4c4ae95a3ea2a7377e"
pe = pefile.PE(data=raw)
base = pe.OPTIONAL_HEADER.ImageBase
native = pe.get_memory_mapped_image()
u = Uc(UC_ARCH_X86, UC_MODE_32)
u.mem_map(base, (len(native) + 4095) & ~4095)
u.mem_write(base, native)
u.mem_map(0, 0x10000)
u.mem_map(0x70000000, 0x200000)
ENTITY, FH, FVT, STOP, FINISH, STACK = 0x70001000, 0x70002000, 0x70002100, 0x7000fff0, 0x7000ffe0, 0x701f0000
heap = 0x70010000
calls, endpoints = [], []
labels = {0x3202E5C0: "binding", 0x3200FB50: "GetProxy", 0x32015470: "CreateProxy",
          0x32087700: "SoundProxyCtor", 0x3200E410: "SetProxy", 0x3200E350: "GrowProxyVector",
          0x32066160: "ProxyType3", 0x32088390: "SetEffectRadius"}


def put(address, *values):
    u.mem_write(address, struct.pack("<" + "I" * len(values), *values))


def get(address):
    return struct.unpack("<I", u.mem_read(address, 4))[0]


def ret(value=0, pop=0):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, value)
    u.reg_write(UC_X86_REG_EIP, get(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4 + pop)


def hook(uc, address, size, _):
    global heap
    if address in labels:
        calls.append(labels[address])
    sp = u.reg_read(UC_X86_REG_ESP)
    if address == 0x3202B460:
        endpoints.append("script-handle entity lookup")
        assert get(sp + 4) == FH
        ret(ENTITY, 4)
    elif address in (0x32006190, 0x320061A0):
        count = get(sp + 4)
        assert 0 < count <= 104
        result = heap
        heap += (count + 15) & ~15
        endpoints.append(f"allocate {count}")
        ret(result)
    elif address == 0x3200AE40:
        target = u.reg_read(UC_X86_REG_ECX)
        source = get(sp + 4)
        assert bytes(u.mem_read(source, 8)) == b"outdoor\0"
        put(target, source)
        endpoints.append("assign outdoor string")
        ret(target, 4)
    elif address == FINISH:
        endpoints.append("script-handle return")
        ret()


u.hook_add(UC_HOOK_CODE, hook)
put(ENTITY, 0x320C41D0)
put(FH, FVT)
put(FVT + 44, FINISH)
# The original entity matrix getter runs too; clean identity pose, no dirty flags.
matrix_getter = get(0x320C41D0 + 96)
put(ENTITY + 0x30, *([0] * 48))
rows = []
for radius in (40.0, 17.25, 0.0, -3.0):
    before = heap
    calls.clear()
    endpoints.clear()
    bits = struct.unpack("<I", struct.pack("<f", radius))[0]
    put(STACK, STOP, FH, bits)
    u.reg_write(UC_X86_REG_ESP, STACK)
    u.reg_write(UC_X86_REG_ECX, 0)
    u.emu_start(0x3202E5C0, STOP, count=100000)
    assert u.reg_read(UC_X86_REG_EIP) == STOP
    assert get(ENTITY + 8) & 0x40
    assert get(ENTITY + 184) == 4
    vector = get(ENTITY + 180)
    proxy = get(vector + 12)
    assert get(proxy) == 0x320CDBB8
    assert get(proxy + 8) == ENTITY
    assert get(proxy + 16) == bits
    assert get(proxy + 88) == get(proxy + 92) == get(proxy + 96) == 0
    assert get(ENTITY + 24) == 0  # No public volume/area trigger flags added.
    if radius == 40.0:
        assert "SoundProxyCtor" in calls and "SetProxy" in calls
        assert heap - before == 112 + 16
    else:
        assert "SoundProxyCtor" not in calls and "SetProxy" not in calls
        assert heap == before
    rows.append(dict(radius=radius, calls=list(calls), endpoints=list(endpoints), proxy=hex(proxy)))

md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
assembly = []
for address, size in ((0x3202E5C0, 124), (0x32087700, 264), (0x3200E410, 82),
                      (0x3200E350, 181), (0x32088390, 17)):
    assembly.append(f"FUNCTION {address:08X}")
    assembly.extend(f"{x.address:08X} {x.mnemonic:8s} {x.op_str}"
                    for x in md.disasm(native[address-base:address-base+size], address))
(OUT / "native-cold-init.asm").write_text("\n".join(assembly) + "\n")
pak_hashes = {}
for name in ("game/scripts/entities/sound/ambientvolume.lua", "game/scriptsbin/entities/sound/ambientvolume.alb",
             "game/worlds/main_world/cells/014_015/client/entities.xml"):
    data = subprocess.run([str(ROOT / "tools/aa-pak.exe"), "cat", name], check=True, capture_output=True).stdout
    pak_hashes[name] = hashlib.sha256(data).hexdigest()
    (OUT / Path(name).name).write_bytes(data)
result = dict(dll_sha256=sha, pak_hashes=pak_hashes, entity_vtable="320C41D0", sound_vtable="320CDBB8",
              initialize_slot="320819A0 (ret; no-op)", matrix_getter=f"{matrix_getter:08X}", calls=rows,
              scope="Original binding, proxy lookup/creation/constructor/install/vector growth and radius setter. Explicit allocation, outdoor-string assignment, script entity lookup/return endpoints. No native client, sound engine, gameplay dispatch or autoload execution is claimed.")
(OUT / "native-cold-init.json").write_text(json.dumps(result, indent=2) + "\n")
print("PASS: native cold creation installs only sound proxy; repeated radius calls store the float without area/trigger creation")
