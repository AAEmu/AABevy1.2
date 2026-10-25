"""Original terrain projection distance producer/getters and uniform-upload oracle.

Reads retained original DLLs and pak shader/CFGs; writes only a NEW ignored directory.
No native process, server, window or GPU. Requires pefile, capstone and unicorn.
"""
from pathlib import Path
import hashlib
import json
import struct
import subprocess
import xml.etree.ElementTree as ET

import capstone
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EDI, UC_X86_REG_EIP, UC_X86_REG_ESI, UC_X86_REG_ESP

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "data/extract/aa-world/terrain-projection-distance-20261025"
OUT.mkdir(parents=True, exist_ok=True)
provenance = {}


def original(path, sha):
    raw = (ROOT / path).read_bytes()
    actual = hashlib.sha256(raw).hexdigest()
    assert actual == sha, (path, actual)
    provenance[path] = actual
    return pefile.PE(data=raw)


engine = original("data/extract/aa-world/environment/source/bin32/cry3dengine.dll",
                  "34d6b73690d1a9d8d0ea0f5eb743fd0624107cfda28c1302826b19a3bc9546de")
renderer = original("data/extract/aa-cry/ida/render/cryrenderd3d10.dll",
                    "ebf051bba5d349a6b3bf6d4eb676908115b73bb22d35d5a0cc917c325426589d")
u = Uc(UC_ARCH_X86, UC_MODE_32)
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
assembly = []
for pe, ranges in [(engine, [(0x3000978D, 0x3000980F), (0x30121AF0, 0x30121B22),
                             (0x300DC9B0, 0x300DCAE1), (0x30131A3B, 0x30131A82)]),
                   (renderer, [(0x3806F9A0, 0x3806FA22)])]:
    base = pe.OPTIONAL_HEADER.ImageBase
    mapped = pe.get_memory_mapped_image()
    u.mem_map(base, (len(mapped) + 4095) & ~4095)
    u.mem_write(base, mapped)
    for start, end in ranges:
        assembly.append(f"RANGE {start:08X}..{end:08X}")
        assembly.extend(f"{i.address:08X} {i.mnemonic:8s} {i.op_str}"
                        for i in md.disasm(mapped[start-base:end-base], start))
(OUT / "native-producer-getters-registration-upload.asm").write_text("\n".join(assembly) + "\n")

image = engine.get_memory_mapped_image()
base = engine.OPTIONAL_HEADER.ImageBase
# The producer uses the real engine vtable, including BOTH original getter bodies.
assert struct.unpack_from("<II", image, 0x30263718 + 1208 - base) == (0x30121AF0, 0x30121B10)
# CVar registration: named Z points to config+0x648; named XY to config+0x64c.
assert image[0x3024E760-base:0x3024E760-base+31].split(b"\0")[0] == b"e_detail_materials_view_dist_z"
assert image[0x3024E710-base:0x3024E710-base+32].split(b"\0")[0] == b"e_detail_materials_view_dist_xy"
assert bytes.fromhex("8d8e48060000") in image[0x3000978D-base:0x300097CC-base]
assert bytes.fromhex("8d864c060000") in image[0x300097CE-base:0x3000980F-base]

for path in ["shaders/hwscripts/cryfx/terrain.cfx", "shaders/hwscripts/cryfx/common.cfi",
             "game/config/cvargroups/option_terrain_detail.cfg",
             "game/config64/cvargroups/option_terrain_detail.cfg"]:
    raw = subprocess.check_output([str(ROOT / "tools/aa-pak.exe"), "cat", path])
    provenance[path] = hashlib.sha256(raw).hexdigest()
    (OUT / Path(path).name.replace("option_", "config64-option_" if "/config64/" in path else "option_")).write_bytes(raw)
for prefix in ["", "config64-"]:
    cfg = (OUT / f"{prefix}option_terrain_detail.cfg").read_text(encoding="utf-8-sig")
    groups, group = {}, "default"
    for line in cfg.splitlines():
        line = line.split(";", 1)[0].strip()
        if line.startswith("["):
            group = line.strip("[]")
        elif "=" in line:
            key, value = (s.strip() for s in line.split("=", 1))
            groups.setdefault(group, {})[key] = float(value)
    pairs = []
    for level in range(1, 5):
        values = groups["default"] | groups.get(str(level), {})
        pairs.append((values["e_detail_materials_view_dist_xy"], values["e_detail_materials_view_dist_z"]))
    assert pairs == [(256,64),(512,128),(1024,128),(1024,256)], pairs
shader = (OUT / "terrain.cfx").read_text(encoding="utf-8-sig")
assert provenance["shaders/hwscripts/cryfx/terrain.cfx"] == "e83a8db0f0b89c41feb495d58570c4fdebdf7892d8768704f8a42a3bea384678"
for term in ["fDist = pow(min(fDist/max(0.001, LayerTexGen[0].w), 1), 4);",
             "OUT.Color.g = fWeight * (1-fDist);",
             "sqrt( saturate(pPass.IN.Color.g) )"]:
    assert "".join(term.split()) in "".join(shader.split()), term

# Concrete original consumers used by the real library tests and world fixture.
consumers = {}
for cell in ["012_014", "015_014"]:
    path = f"game/worlds/main_world/cells/{cell}/client/leveldata.xml"
    raw = subprocess.check_output([str(ROOT / "tools/aa-pak.exe"), "cat", path])
    provenance[path] = hashlib.sha256(raw).hexdigest()
    (OUT / f"{cell}-leveldata.xml").write_bytes(raw)
    consumers[cell] = [surface.attrib for surface in ET.fromstring(raw).findall("SurfaceTypes/SurfaceType")]
for name in ["field/tr_field_grass_006a", "canyon/tr_canyon_rock3d_504c"]:
    path = f"game/materials/aa_terrain/{name}.mtl"
    raw = subprocess.check_output([str(ROOT / "tools/aa-pak.exe"), "cat", path])
    provenance[path] = hashlib.sha256(raw).hexdigest()
    (OUT / Path(path).name).write_bytes(raw)
    material = ET.fromstring(raw)
    if "grass" in name:
        assert material.get("Shader") == "Terrain.Layer" and material.find("SubMaterials") is None
    else:
        assert [m.get("Name") for m in material.findall("SubMaterials/Material")] == ["x", "y", "z"]

h = 0x70000000
u.mem_map(h, 0x20000)
obj, config, surface, output = h+0x100, h+0x2000, h+0x3000, h+0x4000
state, re, stack, stop = h+0x5000, h+0x8000, h+0x1F000, h+0x1FFF0


def words(address, *values):
    u.mem_write(address, struct.pack("<" + "I" * len(values), *values))


def floats(address, *values):
    u.mem_write(address, struct.pack("<" + "f" * len(values), *values))


def call(address, this, *args):
    words(stack, stop, *args)
    u.reg_write(UC_X86_REG_ECX, this)
    u.reg_write(UC_X86_REG_ESP, stack)
    u.emu_start(address, stop, count=1000)
    assert u.reg_read(UC_X86_REG_EIP) == stop


words(0x302DCD3C, obj)
words(0x302DCD40, config)
words(obj, 0x30263718)
words(config, 1)  # e_detail_materials enabled; disabled producer deliberately leaves w untouched.
# Execute the constructor's exact initialization interval, not the full constructor.
u.reg_write(UC_X86_REG_ESI, obj)
u.reg_write(UC_X86_REG_EAX, 0)
u.reg_write(UC_X86_REG_EDI, 0)
u.emu_start(0x30131A3B, 0x30131A6A, count=100)
assert struct.unpack("<2f", u.mem_read(obj+1204, 8)) == (1, 1)
rows = []
# Includes asymmetric ratios to catch swapped getters, plus tiny denominators.
for level, xy, z, ratio_xy, ratio_z in [(i+1, xy, z, 1, 1) for i, (xy, z) in
                                      enumerate([(256, 64), (512, 128), (1024, 128), (1024, 256)])] + [
        (0, 640, 80, 0.75, 1.5), (0, 0.0001, 0.0002, 1, 1)]:
    floats(config+1608, z, xy)
    floats(obj+1204, ratio_z, ratio_xy)
    for axis in range(7):
        # Native axis1/4=X=(Y,-Z); 2/5=Y=(X,-Z); 0/3/6=Z=(X,-Y).
        expected = xy*ratio_xy if axis in (1, 2, 4, 5) else z*ratio_z
        floats(surface+76, 0.2)
        floats(output, *([12345.0] * 16))
        call(0x300DC9B0, 0, surface, output, axis, 1, 0x3F800000)
        result = struct.unpack("<8f", u.mem_read(output, 32))
        assert abs(result[3] - expected) < 1e-7, (axis, result, expected)
        vectors = ([0, 0.2, 0, expected, 0, 0, -0.2, 0] if axis in (1, 4) else
                   [0.2, 0, 0, expected, 0, 0, -0.2, 0] if axis in (2, 5) else
                   [0.2, 0, 0, expected, 0, -0.2, 0, 0])
        assert all(abs(a-b) < 1e-7 for a, b in zip(result, vectors))
        # Actual PB_TerrainLayerGen copies all 16 floats, including the chosen distance.
        words(state+0x1DB4, re)
        words(re+24, output)
        call(0x3806F9A0, 0, state)
        uploaded = bytes(u.mem_read(u.reg_read(UC_X86_REG_EAX), 64))
        assert uploaded == bytes(u.mem_read(output, 64))
        samples = []
        for distance in [0, 0.0005, 32, 64, 96, 128, 192, 256, 384, 512, 1024]:
            fade = 1 - min(distance / max(0.001, result[3]), 1)**4
            samples.append(dict(distance=distance, fade=fade, half_surface_alpha=(0.5*fade)**0.5))
        rows.append(dict(level=level, xy=xy, z=z, ratio_xy=ratio_xy, ratio_z=ratio_z,
                         axis=axis, producer=list(result), source_shader_samples=samples))
# Producer disabled branch preserves w; it is not a zero-distance assignment.
words(config, 0)
floats(output, *([12345.0] * 16))
call(0x300DC9B0, 0, surface, output, 0, 1, 0x3F800000)
assert struct.unpack("<f", u.mem_read(output+12, 4))[0] == 12345.0

result = dict(provenance=provenance, active_cell_surfaces=consumers, controls=rows,
              contract="Native X/Y projections use XY; native Z projection uses Z, including single ground material. Defaults1024/256 from option11 level4; constructor ratios1; shader denominator max(.001,D).",
              boundaries="Original constructor ratio-initialization interval + producer + real engine getter bodies + real uniform upload execute offline. Object/CVar/surface/render-state memory are explicit fixtures; full constructor/CVar registration, group callback, live original draw and native shader execution are not claimed. Shader arithmetic samples are independent source calculations. Vertex-stage fade/axis interpolation remains an aa-world approximation.")
(OUT / "native-contract.json").write_text(json.dumps(result, indent=2) + "\n")
print(f"PASS: {len(rows)} original producer/getter/upload projection controls; source denominator + disabled gate")
