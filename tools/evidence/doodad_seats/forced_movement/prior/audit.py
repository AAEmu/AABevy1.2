"""Read-only native seat research. Writes only this ignored evidence directory.

Run: python -B data/extract/aa-client/seat-presentation/audit.py
No live process, game execution, socket, server, Cargo, or database write.
Engine helper/entity/animation endpoints are fixture boundaries, not live proof.
"""
from pathlib import Path
import hashlib
import json
import math
import sqlite3
import struct
import sys
import xml.etree.ElementTree as ET

sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "tools"))
from user_music_lifecycle_oracle import Native, GAME, SHA, RUNTIME_DATA, RUNTIME_SHA
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from unicorn.x86_const import UC_X86_REG_EBP, UC_X86_REG_EDI

OUT = Path(__file__).parent
DB = "file:<server>/Data/compact.sqlite3?mode=ro"


def write(name, value):
    (OUT / name).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def matmul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def asset_data():
    models = {}
    for path in OUT.rglob("*.cgf"):
        data = path.read_bytes()
        version, table = struct.unpack_from("<II", data, 12)
        count = struct.unpack_from("<I", data, table)[0]
        nodes = []
        for i in range(count):
            kind, ver, offset, ident = struct.unpack_from("<4I", data, table + 4 + i * (20 if version == 0x745 else 16))
            if kind != 0xCCCC000B:
                continue
            assert ver == 0x823
            at = offset + 16
            tm = struct.unpack_from("<16f", data, at + 84)
            matrix = [[tm[j * 4 + i] for j in range(4)] for i in range(4)]
            matrix[3] = [0, 0, 0, 1]
            for i in range(3):
                matrix[i][3] *= 0.01
            nodes.append({"chunk_id": ident, "name": data[at:at + 64].split(b"\0")[0].decode(),
                          "object_chunk_id": struct.unpack_from("<i", data, at + 64)[0],
                          "parent_id": struct.unpack_from("<i", data, at + 68)[0],
                          "raw_tm_cm": tm, "local_metres": matrix})
        by_id = {x["chunk_id"]: x for x in nodes}
        def chain(node):
            p = by_id.get(node["parent_id"])
            return matmul(chain(p), node["local_metres"]) if p else node["local_metres"]
        for node in nodes:
            node["chain_metres"] = chain(node)
        models[path.name] = {"file": str(path), "sha256": hashlib.sha256(data).hexdigest(), "nodes": nodes}
    path = OUT / "game/prefabs/interaction_c.xml"
    xml = path.read_bytes()
    prefabs = []
    for p in ET.fromstring(xml).iter("Prefab"):
        if p.get("Name") in ("furniture.piano_a", "furniture.piano_b", "furniture.percussion_a"):
            prefabs.append({"name": p.get("Name"), "objects": [dict(x.attrib) for x in p.find("Objects")]})
    authored_seats = []
    for prefab in prefabs:
        for obj in prefab["objects"]:
            model = models.get(obj.get("Prefab", "").replace("\\", "/").rsplit("/", 1)[-1])
            if not model:
                continue
            for node in model["nodes"]:
                if node["name"] != "$bond0":
                    continue
                # All three actual seat-bearing chair parts have authored identity
                # rotation/scale. The piano body's +90deg is a different part.
                assert "Rotate" not in obj and "Scale" not in obj
                pos = list(map(float, obj["Pos"].split(",")))
                matrix = [row.copy() for row in node["chain_metres"]]
                for i in range(3):
                    matrix[i][3] += pos[i]
                authored_seats.append({"prefab": prefab["name"], "part_name": obj["Name"],
                                       "helper": node["name"], "prefab_local_matrix": matrix})
    write("assets.json", {"note": "CGF matrix units/parent chain match aa_cry::mesh::Model parsing. These are asset-derived matrices; engine helper lookup is a native fixture boundary.",
                          "prefab_sha256": hashlib.sha256(xml).hexdigest(), "authored_seats": authored_seats,
                          "prefabs": prefabs, "models": models})
    return models, prefabs


def catalog():
    db = sqlite3.connect(DB, uri=True)
    db.execute("PRAGMA query_only=ON")
    db.row_factory = sqlite3.Row
    queries = [
        "SELECT id,name,model,model_kind_id,no_collision FROM doodad_almighties WHERE id IN (1236,6200,8065,8220,8255)",
        "SELECT id,doodad_almighty_id,model FROM doodad_func_groups WHERE id IN (1951,16414,22477,22863,23046)",
        "SELECT * FROM model_attach_point_strings WHERE id=2",
        "SELECT * FROM anim_actions WHERE id=19",
        "SELECT id,anim_start_id,anim_end_id,idle_anim,anim_action_id,ag_stance,root,fastened,cannot_jump,remove_on_move,active_weapon_id FROM buffs WHERE id IN (5068,8079,7011,8216)",
        "SELECT * FROM anims WHERE id IN (584,616)",
        "SELECT id,start_anim_id,fire_anim_id,channeling_anim_id FROM skills WHERE id IN (20847,26592)",
    ]
    write("catalog.json", {"database_uri": DB, "query_only": True,
                           "queries": [{"sql": q, "rows": [dict(x) for x in db.execute(q)]} for q in queries]})
    db.close()


def native():
    n = Native()
    runtime = RUNTIME_DATA.read_bytes()
    assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
    constants = [0x39ac38c8, 0x39ac38cc, 0x39ac38e8, 0x39ac38ec, 0x39ac38f0,
                 0x39ae6d38, 0x3a1699f4, 0x39ac7464, 0x39ac7468, 0x39ac746c, 0x39ac7470]
    for a in constants:
        n.put(a, struct.unpack_from("<I", runtime, a - 0x39ac3000)[0])
    A, B, D, DV, R, RV, C, CV, E, EV, V = [0x10070000 + i * 0x1000 for i in range(11)]
    observations = []
    supplied, fallback, helper_ok = None, None, True
    def endpoint(a, f):
        n.hooks[a] = f
        return a
    n.put(A, V)
    n.put(A + 0x164, 0x10090000)
    for obj, vt in [(D, DV), (R, RV), (C, CV), (E, EV)]:
        n.put(obj, vt)
    n.put(DV + 0xc, endpoint(0x30001000, lambda: n.ret(R)))
    n.put(RV + 0x68, endpoint(0x30001010, lambda: n.ret(C)))
    n.put(V + 0x1b8, endpoint(0x30001020, lambda: n.ret(E)))
    def format_name():
        spot = struct.unpack("<i", struct.pack("<I", n.arg(3)))[0]
        observations.append({"format": n.string(n.arg(2)).decode(), "spot": spot})
        n.u.mem_write(n.arg(0), ("bond%d" % spot).encode() + b"\0")
        n.ret()
    n.put(0x38ff6f2c, endpoint(0x30001030, format_name))
    def helper():
        observations.append({"helper_name": n.string(n.arg(0)).decode(), "world": n.arg(2), "found": helper_ok})
        if helper_ok:
            n.u.mem_write(n.arg(1), struct.pack("<12f", *supplied))
        n.ret(int(helper_ok), pop=12)
    n.put(RV + 0x88, endpoint(0x30001040, helper))
    def set_matrix():
        observations.append({"set_world_matrix": struct.unpack("<12f", n.u.mem_read(n.arg(0), 48)), "flags": n.arg(1)})
        n.ret(pop=8)
    n.put(EV + 0x58, endpoint(0x30001050, set_matrix))
    n.put(CV + 0x3c, endpoint(0x30001060, lambda: (observations.append({"attach_entity": n.arg(0), "flag": n.arg(1)}), n.ret(pop=8))))
    n.hooks[0x39155ed0] = lambda: (observations.append({"anim_action": n.arg(0), "enabled": n.arg(1), "extra": n.arg(2)}), n.ret(pop=12))
    def get_fallback():
        out = n.arg(0)
        observations.append({"fallback_owner_matrix": True})
        n.u.mem_write(out, struct.pack("<12f", *fallback))
        n.ret(out, pop=4)
    n.put(RV + 0x4c, endpoint(0x30001070, get_fallback))
    cases = []
    for label, spot, space, point, found, matrix in [
        ("identity", 0, 1, 2, True, [1,0,0,11, 0,1,0,22, 0,0,1,33]),
        ("rotation_and_scale", 0, 1, 2, True, [0,-3,0,11, 2,0,0,22, 0,0,4,33]),
        ("spot_is_signed_not_attach_point", -2, 9, 254, True, [1,0,0,11, 0,1,0,22, 0,0,1,33]),
        ("missing_helper_native_fallback", 7, 1, 2, False, [1,0,0,11, 0,1,0,22, 0,0,1,33]),
    ]:
        observations.clear()
        supplied, fallback, helper_ok = matrix, [1,0,0,101, 0,1,0,202, 0,0,1,303], found
        n.put(B, 123, spot, space, point, 2)
        n.call(0x390cec40, A, B, D)
        assert observations[-1] == {"anim_action": 19, "enabled": 1, "extra": 0}
        assert n.get(A + 0x160) == 123
        cases.append({"case": label, "observations": observations.copy()})
    observations.clear()
    n.call(0x390c1420, A, B)
    assert observations == [{"anim_action": 19, "enabled": 0, "extra": 0}]
    cases.append({"case": "unseat_clears_action19", "observations": observations.copy()})
    # Real layout lookup391047b0 and doodad world-matrix wrapper393abe50.
    # A normalized type8 helper part is supplied; CGF-to-part loading is not run.
    RR, LL, PP, PV, SS, VV, SV, OO, NN, MM = [0x100c0000 + i * 0x1000 for i in range(10)]
    n.put(RR + 0xc4, LL)
    n.put(LL + 0x10, SS)
    n.put(LL + 0x18, VV, VV + 4)
    n.put(VV, PP)
    n.put(PP, PV)
    n.put(PP + 0x38, NN)
    n.u.mem_write(NN, b"bond0\0")
    n.put(SS, SV)
    n.u.mem_write(PP + 8, struct.pack("<12f", 1,0,0,0, 0,1,0,.41318312, 0,0,1,0))
    n.u.mem_write(MM, struct.pack("<12f", 0,-2,0,10, 2,0,0,20, 0,0,2,30))
    n.put(PV + 4, endpoint(0x30002000, lambda: n.ret(8)))
    n.put(SV + 0x60, endpoint(0x30002010, lambda: n.ret(MM)))
    n.put(0x38ff6c60, endpoint(0x30002020, lambda: n.ret(0 if n.string(n.arg(0)).lower() == n.string(n.arg(1)).lower() else 1)))
    for world in [0, 1]:
        for name in [b"bond0", b"$bond0"]:
            n.u.mem_write(NN + 0x100, name + b"\0")
            ok = n.call(0x393abe50, RR, NN + 0x100, OO, world)
            assert ok & 255 == 1
            matrix = struct.unpack("<12f", n.u.mem_read(OO, 48))
            assert abs(matrix[3] - (10 - 2 * .41318312 if world else 0)) < 1e-5
            cases.append({"case": "native_layout_helper_and_world_product", "helper_name": name.decode(), "world": world, "matrix": matrix})
    # Execute only the actual input-unbond gate, without substituting its branches.
    # The earlier movement/turn calculation is retained as source, not emulated here.
    I, F = 0x100a0000, 0x100a1000
    n.put(0x39aff9b8, 0x100b0000)
    n.hooks[0x394fab30] = lambda: (observations.append({"cancel_endpoint": "394fab30"}), n.ret())
    n.hooks[0x3935a520] = lambda: (observations.append({"unbond_endpoint": "3935a520"}), n.ret())
    for move, turn in [(0,0), (1,0), (0,1), (1,1)]:
        observations.clear()
        n.u.mem_write(I + 0xb8, bytes([move,turn]))
        n.put(F - 0x34, A)
        n.u.reg_write(UC_X86_REG_EDI, I)
        n.u.reg_write(UC_X86_REG_EBP, F)
        n.u.emu_start(0x39405f73, 0x39405f98, count=1000)
        assert bool(observations) == bool(move or turn)
        cases.append({"case": "input_unbond_gate", "activity_flag_b8": move, "yaw_flag_b9": turn, "observations": observations.copy()})
    image = n.images[0][3]
    base = n.images[0][2]
    ranges = [(0x393635e0,0x393637ec), (0x390cec40,0x390cee28), (0x390c1420,0x390c14ec),
              (0x39104630,0x391046a3), (0x391047b0,0x3910499f), (0x393abe50,0x393ac016),
              (0x390c5840,0x390c59af), (0x3908c930,0x3908ca93), (0x39155ed0,0x39155f6e),
              (0x393566f0,0x3935678f), (0x3935a520,0x3935a602),
              (0x39404d50,0x394064b0), (0x39402e00,0x39402f5b), (0x39403110,0x3940316c)]
    cs = Cs(CS_ARCH_X86, CS_MODE_32)
    lines = ["x2game SHA256 " + SHA, "runtime data SHA256 " + RUNTIME_SHA,
             "actor vtable399a4f68: +238=390cec40 +23c=390c1420",
             "doodad render vtable399d2a50: +88=393abe50", "format399a4330=bond%d"]
    for a,b in ranges:
        raw = image[a-base:b-base]
        lines.append("\nSOURCE %08x:%08x SHA256 %s" % (a,b,hashlib.sha256(raw).hexdigest()))
        lines.extend("%08x: %s %s" % (i.address,i.mnemonic,i.op_str) for i in cs.disasm(raw,a))
    (OUT / "native-source.txt").write_text("\n".join(lines) + "\n")
    write("native-cases.json", {"game_sha256": SHA, "runtime_sha256": RUNTIME_SHA,
                                "limits": "Original seat application/decomposition, layout helper lookup/world product, and input gate instructions execute. Apply's helper/entity/animation/format endpoints are substituted; layout fixtures supply a normalized type8 helper part and world matrix, with only string comparison/type/entity endpoints substituted. No native renderer/CGF-to-part loader or full input producer execution.",
                                "cases": cases})
    print("Retained", len(cases), "original-instruction cases; native-source.txt, assets.json, catalog.json")


if __name__ == "__main__":
    asset_data()
    catalog()
    native()
