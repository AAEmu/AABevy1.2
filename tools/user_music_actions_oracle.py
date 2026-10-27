"""Read-only catalog joins and isolated execution of original 1.2 mode/bond code.

Run: python -B tools/user_music_actions_oracle.py
No process, network, game window, audio device, or server is accessed.
"""
from pathlib import Path
import hashlib
import json
import sqlite3
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from user_music_lifecycle_oracle import Native, PLAYER, WORLD, RUNTIME_DATA, RUNTIME_SHA
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

OUT = Path(__file__).parent / "evidence" / "user_music_actions"
OUT.mkdir(parents=True, exist_ok=True)
DB = "file:<server>/Data/compact.sqlite3?mode=ro"
PROVIDER, BAR, VECTOR = PLAYER + 0x3E80, 0x100B0000, 0x100C0000
LISTS = [(0x20, 0x28), (0xD28, 0xD30), (0x16F0, 0x16F8)]


def main():
    db = sqlite3.connect(DB, uri=True)
    db.execute("PRAGMA query_only=ON")
    db.row_factory = sqlite3.Row
    queries = [
        "SELECT id,impl_id,category_id,use_skill_id FROM items WHERE id IN (27902,28051) ORDER BY id",
        "SELECT f.*,g.doodad_almighty_id,g.doodad_func_group_kind_id,a.attach_point_id,a.space,a.bond_kind_id "
        "FROM doodad_funcs f JOIN doodad_func_groups g ON g.id=f.doodad_func_group_id "
        "JOIN doodad_func_attachments a ON a.id=f.actual_func_id "
        "WHERE f.actual_func_type='DoodadFuncAttachment' AND f.func_skill_id IN (20847,26592) ORDER BY f.id",
        "SELECT se.id,se.skill_id,se.effect_id,se.application_method_id,e.actual_type,e.actual_id,"
        "b.buff_id,i.wi_id FROM skill_effects se JOIN effects e ON e.id=se.effect_id "
        "LEFT JOIN buff_effects b ON e.actual_type='BuffEffect' AND b.id=e.actual_id "
        "LEFT JOIN interaction_effects i ON e.actual_type='InteractionEffect' AND i.id=e.actual_id "
        "WHERE se.skill_id IN (20847,26592) ORDER BY se.id",
        "SELECT * FROM buff_skills WHERE buff_id IN (5068,8079,6010,7011,8216) ORDER BY id",
        "SELECT * FROM unit_reqs WHERE owner_type='Skill' AND owner_id IN (20847,26592,22213,22214,22217) ORDER BY id",
        "SELECT id,name,kind_id,system,duration,remove_on_move,remove_on_start_skill,remove_on_interaction,fastened,active_weapon_id "
        "FROM buffs WHERE id IN (5068,8079,6010,7011,8216) ORDER BY id",
        "SELECT t.buff_id,t.event_id,t.effect_id,e.actual_type,s.id,s.special_effect_type_id "
        "FROM buff_triggers t JOIN effects e ON e.id=t.effect_id "
        "JOIN special_effects s ON e.actual_type='SpecialEffect' AND s.id=e.actual_id "
        "WHERE t.buff_id IN (5068,8079,6010,7011,8216) ORDER BY t.id",
        "SELECT se.skill_id,se.effect_id,e.actual_type,e.actual_id,s.special_effect_type_id,d.buff_tag_id "
        "FROM skill_effects se JOIN effects e ON e.id=se.effect_id "
        "LEFT JOIN special_effects s ON e.actual_type='SpecialEffect' AND s.id=e.actual_id "
        "LEFT JOIN dispel_effects d ON e.actual_type='DispelEffect' AND d.id=e.actual_id "
        "WHERE se.skill_id IN (22213,22214,22217) ORDER BY se.id",
    ]
    catalog = [{"sql": sql, "rows": [dict(row) for row in db.execute(sql)]} for sql in queries]
    (OUT / "catalog.json").write_text(json.dumps({"database_uri": DB, "query_only": True, "queries": catalog}, indent=2) + "\n")
    skills = {}
    for row in db.execute("SELECT buff_id,skill_id FROM buff_skills WHERE buff_id IN (5068,8079,6010) ORDER BY id"):
        skills.setdefault(row[0], []).append(row[1])
    skills.setdefault(8079, [])

    n = Native()
    runtime = RUNTIME_DATA.read_bytes()
    assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
    constants = {}
    for addr in [0x39D1FB9C, 0x39AC62BC, 0x3A1699F4, 0x39AC7464, 0x39AC7468, 0x39AC746C, 0x39AC7470,
                 0x39AC76F0,0x39AC76F4,0x39AC76F8,0x39AC76FC,0x39AC9B10,0x39AC9B24]:
        value = struct.unpack_from("<I", runtime, addr - 0x39AC3000)[0]
        constants[f"{addr:08x}"] = value
        n.put(addr, value)
    n.put(0x39AFFA38, BAR)
    n.put(0x3A1744F0, VECTOR, VECTOR, VECTOR + 0x800)
    n.put(PLAYER + 0x3AF0, 0)
    n.hooks[0x39360020] = lambda: n.ret(0)
    # Mount action population is a boundary, not a replacement for buff population.
    n.hooks[0x39204500] = lambda: n.ret()
    observations = []
    n.hooks[0x39424220] = lambda: (observations.append({"event": n.arg(0)}), n.ret())
    n.hooks[0x393FE030] = lambda: (observations.append({"input_mode_index": n.arg(0), "enabled": n.arg(1)}), n.ret(pop=8))
    descs = {}
    for idx, (buff, rows) in enumerate(skills.items()):
        desc = 0x100D0000 + idx * 0x1000
        ptrs = desc + 0x400
        descs[buff] = desc
        n.put(desc + 0x284, ptrs, ptrs + len(rows)*4)
        for j, skill in enumerate(rows):
            n.put(ptrs + 4*j, ptrs + 0x100 + 4*j)
            n.put(ptrs + 0x100 + 4*j, skill)
    n.hooks[0x396F63D0] = lambda: n.ret(descs.get(n.arg(0), 0))
    cases = [("empty", [[], [], []], 0), ("drum", [[(2,8079)], [], []], 0),
             ("piano_and_score", [[(2,5068)], [(3,6010)], []], 0),
             ("remove_piano_instance", [[(2,5068)], [(3,6010)], []], 2),
             ("duplicate_template_instances", [[(2,5068),(3,5068)], [], []], 0)]
    for kind in range(3):
        rows = [[], [], []]
        rows[kind] = [(2,5068)]
        cases.append((f"piano_list_{kind}", rows, 0))
    cases.append(("restored_empty", [[], [], []], 0))
    results = []
    for name, lists, exclude in cases:
        n.u.mem_write(PROVIDER, bytes(0x2100))
        n.put(PROVIDER, 0x399D1990)
        n.put(0x39B02294, WORLD)
        n.put(WORLD + 0x10C, PLAYER)
        for (count, start), entries in zip(LISTS, lists):
            n.put(PROVIDER + count, len(entries))
            for j, (instance, template) in enumerate(entries):
                n.put(PROVIDER + start + 0x68*j, instance, template)
        observations.clear()
        n.call(0x39389360, 0, exclude)
        count = n.call(0x393871A0)
        actual = [n.get(n.call(0x393871B0, 0, j)) for j in range(count)]
        expected = [skill for entries in lists for instance, buff in entries if instance != exclude for skill in skills[buff]]
        assert actual == expected, (name, actual, expected)
        mode_count = n.get(BAR + 0x13B0)
        mode = [n.get(BAR + 0x150*j) for j in range(mode_count)]
        assert mode == actual[:15], (name, mode, actual)
        assert observations == [{"event": 0x21B}, {"input_mode_index": 1, "enabled": int(bool(mode))}, {"event": 0x5F}], observations
        results.append({"case": name,"buff_lists": lists,"exclude_instance": exclude,"native_buff_skills": actual,"native_mode_actions": mode,"notifications": list(observations)})

    # Native request construction and unbond gate; network and actor presentation are sinks.
    def sender():
        packet = n.arg(0)
        observations.append({"opcode": n.get(packet+4), "actor": n.get(packet+12), "doodad": n.get(packet+16)})
        n.ret(pop=4)
    n.hooks[0x39186810] = sender
    n.hooks[0x39035110] = lambda: n.ret(0, 4)
    def detach():
        observations.append({"detach_actor": PLAYER, "doodad": n.arg(0)})
        n.ret(pop=4)
    n.hooks[0x393566F0] = detach
    unbond = []
    for name, doodad in [("not_bonded",0),("bonded_piano",23456)]:
        n.put(PLAYER+0x3B94, doodad)
        observations.clear()
        n.call(0x3935A520, PLAYER)
        expected = [] if doodad == 0 else [{"opcode": 0xCD, "actor": 47, "doodad": doodad}, {"detach_actor": PLAYER, "doodad": doodad}]
        assert observations == expected, observations
        unbond.append({"case": name, "observations": list(observations)})

    # Execute the original item-use branch, not an inferred composer predicate.
    item, item_desc = 0x100E0000, 0x100E1000
    n.put(item, 12345, 0, 27902)
    impl = db.execute("SELECT impl_id FROM items WHERE id=27902").fetchone()[0]
    assert impl == 26
    n.put(item_desc+0x48, impl)  # Original dispatch table impl26 -> 394eb0e2.
    n.hooks[0x39428370] = lambda: (observations.append({"error":n.arg(0)}), n.ret())
    def event_args():
        assert n.arg(0) == 0x23F
        assert n.get(n.arg(1)) == 2 and n.get(n.arg(2)) == 5
        observations.append({"event":n.arg(0),"show":bool(n.u.mem_read(n.arg(1)+4,1)[0]),"item_id":n.string(n.get(n.arg(2)+4)).decode()})
        n.ret()
    n.hooks[0x39424260] = event_args
    def itoa():
        assert n.arg(4) == 10
        n.u.mem_write(n.arg(2),str(n.arg(0)+(n.arg(1)<<32)).encode()+b'\0')
        n.ret(0)
    n.hooks[0x3301BFD0] = itoa
    def lookup():
        observations.append({"lookup":n.arg(0)+(n.arg(1)<<32),"tooltip":n.arg(2)})
        n.ret(0)
    n.hooks[0x39082550] = lookup
    admission = []
    for buff in [None,5068,8079,6010,6176,6177,7011,8216]:
        for kind in range(3):
            n.u.mem_write(PROVIDER,bytes(0x2100))
            n.put(PROVIDER,0x399D1990)
            if buff is not None:
                count,start = LISTS[kind]
                n.put(PROVIDER+count,1)
                n.put(PROVIDER+start,17,buff)
            n.put(item+8,27902)
            observations.clear()
            n.call(0x394EAAC0,item,0,item_desc)
            expected = [{"event":0x23F,"show":True,"item_id":"12345"}] if buff in (None,5068,8079) else [{"error":702}]
            assert observations == expected,(buff,kind,observations)
            admission.append({"item":"music_paper","buff":buff,"list":kind,"observations":list(observations)})
            n.put(item+8,28051)
            n.u.mem_write(item+0x18,b'\x08'+struct.pack('<Q',98765))
            observations.clear()
            n.call(0x394EAAC0,item,0,item_desc)
            assert observations == [{"lookup":98765,"tooltip":0}],(buff,kind,observations)
            admission.append({"item":"written_sheet","buff":buff,"list":kind,"observations":list(observations)})
    for label,game,player,discriminator in [("no_game",False,False,8),("no_player",True,False,8),("written_sheet_wrong_discriminator",True,True,7)]:
        n.put(0x39B02294,WORLD if game else 0)
        n.put(WORLD+0x10C,PLAYER if player else 0)
        n.put(item+8,28051)
        n.u.mem_write(item+0x18,bytes([discriminator]))
        observations.clear()
        n.call(0x394EAAC0,item,0,item_desc)
        assert not observations,(label,observations)
        admission.append({"case":label,"observations":list(observations)})

    blocks = [(0x39389360,0x65),(0x393879C0,0x8A),(0x3938A1E0,0xE5),
              (0x393871A0,0x39),(0x392044A0,0x60),(0x392045F0,0x10D),
              (0x394C2860,0xD1),(0x394C2940,0x93),
              (0x391DB770,0x73),(0x391DB7F0,0x79),(0x393B2B90,0x7E),
              (0x393566C0,0x27),(0x3935A520,0xE2),(0x397AC640,0x11),
              (0x394E4960,0x3E),(0x3939E320,0x35),(0x394EAAC0,0x75A),
              (0x39405F73,0x25),(0x393B3D50,0x170),(0x394424C0,0xDD),(0x393B1DE0,0xC8)]
    cs = Cs(CS_ARCH_X86, CS_MODE_32)
    source = ["x2game_unpacked.dll sha256="+n.images[0][1]]
    for addr, length in blocks:
        raw = bytes(n.u.mem_read(addr,length))
        source.append(f"\nSOURCE {addr:08x} length={length} bytes_sha256={hashlib.sha256(raw).hexdigest()}")
        source.extend(f"{i.address:08x}: {i.mnemonic} {i.op_str}".rstrip() for i in cs.disasm(raw,addr))
    (OUT/"native-source.txt").write_text("\n".join(source)+"\n")
    result = {"dll_sha256":n.images[0][1],"runtime_dump_sha256":RUNTIME_SHA,"runtime_constants":constants,
              "mode_cases":results,"unbond_cases":unbond,"item_admission_cases":admission,
              "boundaries":["actual DB buff_skills rows supplied in controlled native descriptor storage", "native catalog lookup endpoint substituted; native provider traversal and callback, vector construction, mode append/rebuild, request constructor execute unchanged", "mount population, UI events/input-mode setter, network send and actor detach presentation are sinks", "item admission executes original full394eaac0, original394e4960/3939e320/provider; CRT decimal conversion, note lookup and UI event/error presentation are sinks"],
              "limits":["isolated native instruction execution only; no original live client or server I/O", "not a complete buff receipt or seat animation test"]}
    (OUT/"native-cases.json").write_text(json.dumps(result,indent=2)+"\n")
    print(f"Passed {len(results)} native mode-action cases, {len(unbond)} unbond cases, {len(admission)} item admission cases")


if __name__ == "__main__":
    main()
