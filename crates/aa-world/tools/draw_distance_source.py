"""Retain original 1.2 option tables and the native distance contract, read-only.

python crates/aa-world/tools/draw_distance_source.py
Output stays in ignored data/extract/aa-world/draw-distance/source/.
"""
from pathlib import Path
import hashlib
import json
import subprocess

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "data/extract/aa-world/draw-distance/source"
OUT.mkdir(parents=True, exist_ok=True)
sources = {}
tables = {}
for config in ("config", "config64"):
    for name in ("option_view_distance", "option_view_dist_ratio", "option_view_dist_ratio_vegetation"):
        path = f"game/{config}/cvargroups/{name}.cfg"
        data = subprocess.run([str(ROOT / "tools/aa-pak.exe"), "cat", path], check=True, capture_output=True).stdout
        (OUT / f"{config}-{name}.cfg").write_bytes(data)
        sources[path] = hashlib.sha256(data).hexdigest()
        groups = {}
        group = "default"
        for line in data.decode("utf-8-sig").splitlines():
            line = line.split(";", 1)[0].strip()
            if line.startswith("["):
                group = line.strip("[]")
            elif "=" in line:
                key, value = (s.strip() for s in line.split("=", 1))
                if key:
                    groups.setdefault(group, {})[key] = float(value)
        tables[path] = groups

table = tables["game/config64/cvargroups/option_view_distance.cfg"]
assert [table[g]["e_max_view_dst"] for g in ("1", "2", "3", "default")] == [600, 1000, 1500, 2000]
table = tables["game/config/cvargroups/option_view_distance.cfg"]
assert [table[g]["e_max_view_dst"] for g in ("1", "2", "3", "default")] == [500, 600, 700, 800]
table = tables["game/config64/cvargroups/option_view_dist_ratio.cfg"]
assert [table[g]["e_view_dist_ratio"] for g in ("1", "2", "3", "default")] == [30, 30, 40, 60]
table = tables["game/config64/cvargroups/option_view_dist_ratio_vegetation.cfg"]
assert [table[g]["e_view_dist_ratio_vegetation"] for g in ("1", "2", "3", "default")] == [30, 40, 50, 60]

# Prior world-worker reverse-engineering outputs are retained beside the unmodified
# shipping DLL. Check the original strings too; these are contract/source checks,
# not execution or a new claim about the native far-plane producer.
dll = ROOT / "data/extract/aa-cry/ida/cry3dengine.dll"
native = dll.read_bytes()
for literal in (b"e_max_view_dst", b"Far clipping plane distance",
                b"View distance ratio for objects", b"View distance ratio for vegetation"):
    assert literal in native, literal
evidence = ROOT / "data/extract/aa-world/distance/ida"
for name, marker, following in (("cv2.txt", '"e_max_view_dst"', 32),
                                ("cvars.txt", '"e_view_dist_ratio"', 40),
                                ("mvd.txt", "// ==== 300F0C30", 65)):
    data = (evidence / name).read_bytes()
    sources[str((evidence / name).relative_to(ROOT))] = hashlib.sha256(data).hexdigest()
    lines = data.decode("utf-8-sig").splitlines()
    start = next(i for i, line in enumerate(lines) if marker in line)
    (OUT / name).write_text("\n".join(lines[max(0, start - 3):start + following]) + "\n", encoding="utf-8")
result = dict(pak_sources=sources, tables=tables, native_dll_sha256=hashlib.sha256(native).hexdigest(),
              scope="Original option files, native registration descriptions and prior 300F0C30 disassembly/decompilation; no native execution")
(OUT / "contract.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
print("PASS: original 32/64-bit option tables, independent object/vegetation ratios, and far-clip registration contract")
