"""Execute original music skill/effect SQL row loaders and skill-vector attachment.

Only SQLite endpoints, capacity allocation and catalog indexing are fixtures. Input
rows are the actual four music SpecialEffect rows from the read-only compact DB.
Original loaders choose their SQL columns, layouts and effect pointers; the actual
39732e50 attachment loop appends those rows to SkillDesc+1e8 without method filtering.
Requires the same RE-only packages/DLLs as user_music_lifecycle_oracle.py.
"""
import hashlib
import json
import sqlite3
import subprocess
from pathlib import Path

from user_music_lifecycle_oracle import Native, OUT, UC_X86_REG_EBP, UC_X86_REG_EBX, UC_X86_REG_ESI


def server_source():
    """Read retained revisions only; never fetch, checkout, run or modify a server."""
    checkout = "<aaemu-src>"
    path = "rust/crates/aaemu-game/src/systems/services/music.rs"
    def git(*args):
        return subprocess.check_output(["git", "-C", checkout, *args], text=True, encoding="utf-8")
    def implementation(source):
        begin = source.index("impl ClientPacket for CSRequestMusicNotesPacket")
        brace = source.index("{", begin)
        end, depth = brace+1, 1
        while depth:
            depth += (source[end] == "{") - (source[end] == "}")
            end += 1
        return source[begin:end]
    snapshots = []
    # Fixed local revisions, not moving branch tips or claims of current live state.
    for revision in ("3e0f74498929695d325353b76ffb644be2958a40",
                     "1e359beb36a222868d705fa1f92742c4a5775d1f"):
        value = git("show", revision+":"+path)
        snapshots.append({"revision": revision, "path": path,
                          "source_utf8_sha256": hashlib.sha256(value.encode()).hexdigest(),
                          "request_impl": implementation(value)})
    original = Path("<home>/Documents/GitHub/AAEmu/AAEmu.Game/Core/Packets/C2G/CSRequestMusicNotesPacket.cs")
    enum = original.parents[3]/"Models/Game/Skills/Effects/Enums/SkillEffectApplicationMethod.cs"
    snapshots.extend({"path": str(p), "sha256": hashlib.sha256(p.read_bytes()).hexdigest(),
                       "source": p.read_text(encoding="utf-8")} for p in (original, enum))
    (OUT/"server-source.json").write_text(json.dumps({
        "scope": "Read-only source snapshots. No handler/server execution, no runtime deployment claim.",
        "background_pr": "https://github.com/NickMesser/AAEmu/pull/44",
        "snapshots": snapshots}, indent=2)+"\n", encoding="utf-8")


def instrument_cases(db):
    """Original items column9 -> ItemDesc+18 -> lookup -> wrapper fallback."""
    n = Native()
    game_db, item_desc, key_range, head, node, midi_ptr = (
        0x100A0000, 0x100B0000, 0x100B1000, 0x100B2000, 0x100B2100, 0x100B3000)
    n.put(0x39ACB0E8, game_db)
    n.put(game_db+0x386C, key_range, key_range+12)
    n.put(game_db+0xDF4, head)
    n.put(game_db+0xDF8, 0)
    rows = []
    source_sql = """SELECT i.id,i.category_id,w.holdable_id,m.midi
                    FROM items i JOIN item_weapons w ON w.item_id=i.id
                    LEFT JOIN instrument_sounds m ON m.item_id=i.id
                    WHERE i.id IN (34598,34599,6183,14047,6177,6202) ORDER BY i.id"""
    actual = db.execute(source_sql).fetchall()
    def compile_sink():
        n.calls.append({"endpoint": "390838a0_compile_fixture", "program": n.arg(3),
                        "bank": n.arg(4)})
        n.ret(6)
    def error():
        n.calls.append({"endpoint": "native_error", "id": n.arg(0)})
        n.ret()
    n.hooks[0x390838A0] = compile_sink
    n.hooks[0x39428370] = error
    n.u.mem_write(0x10030000, b"CDE\0")
    def at_end():
        n.u.emu_stop()
    n.hooks[0x396CB5BE] = at_end
    n.hooks[0x396BA778] = at_end
    # Real rows, plus zero-MIDI rows to discriminate fallback from override.
    fixtures = [(r, r[3], "actual_DB") for r in actual]
    fixtures.extend((r, 0, "zero_MIDI_fixture") for r in actual if r[1] in (80,81))
    for row, midi, label in fixtures:
        template, category, holdable, original_midi = row
        n.u.mem_write(item_desc, bytes(0x100))
        frame = 0x201E8000
        n.u.mem_write(frame-0x140, bytes(0x140))
        n.put(frame-0x30, 1)
        n.put(frame-0x140, template)
        def column9():
            assert n.arg(1) == 9
            n.ret(category)
        n.hooks[0x3953A390] = column9
        n.u.reg_write(UC_X86_REG_EBP, frame)
        n.call(0x396CB5A7, end=0x396CB5BE)
        assert n.get(frame-0x128) == category
        n.call(0x396BA730, item_desc, frame-0x140, end=0x396BA778)
        assert n.get(item_desc) == template and n.get(item_desc+24) == category
        n.put(key_range, template, template, key_range+16)
        n.put(key_range+16, item_desc)
        n.put(head, head, head, head)
        n.u.mem_write(head+20, b"\x01\x01")
        if midi is not None:
            n.put(head, node, node, node)
            n.put(node, head, head, head, template, midi_ptr)
            n.u.mem_write(node+20, b"\x01\x00")
            n.put(midi_ptr, midi)
        from user_music_lifecycle_oracle import PLAYER
        n.put(PLAYER+0x1240, template)
        n.calls.clear()
        # Parameter0 is the actual bypass ID. Registry/catalog getters and the
        # UTF32 converter execute original instructions; only final compiler is a sink.
        result = n.call(0x39083960, 0, 0x10040000, 32768, 0x10030000, 0)
        compiled = [x for x in n.calls if "program" in x]
        expected = midi if midi else 106 if category == 80 else 73 if category == 81 else None
        assert bool(compiled) == (expected is not None), (row, midi, n.calls)
        if compiled:
            assert compiled == [{"endpoint": "390838a0_compile_fixture", "program": expected, "bank": 0}]
            assert result == 6
        else:
            assert n.calls == [{"endpoint": "native_error", "id": 1}]
            assert result == 0
        rows.append({"input_kind": label, "template": template, "category": category,
                     "holdable_id": holdable, "DB_midi": original_midi, "supplied_midi": midi,
                     "column9_to_ItemDesc_18": True, "calls": list(n.calls), "result": result})
    counts_sql = """SELECT w.holdable_id,i.category_id,COUNT(*) FROM item_weapons w
                     JOIN items i ON i.id=w.item_id WHERE w.holdable_id IN (21,23)
                     GROUP BY w.holdable_id,i.category_id ORDER BY w.holdable_id,i.category_id"""
    (OUT/"native-instruments.json").write_text(json.dumps({
        "dll_sha256": n.images[0][1], "source_sql": source_sql, "cases": rows,
        "native_items_query_39a00d90": n.string(0x39A00D90).decode(),
        "native_instrument_query_39a08240": n.string(0x39A08240).decode(),
        "counts_sql": counts_sql, "category_counts": db.execute(counts_sql).fetchall(),
        "fallback_table": {f"{a:08x}": n.get(a) for a in (0x399A1E94,0x399A1E98,0x399A1E9C,0x399A1EA0)},
        "limitations": "Column9 SQL endpoint supplies actual DB category; original column/store/copy/getters/wrapper/converter run. Compiler body and error presentation are sinks."},
        indent=2)+"\n", encoding="utf-8")
    print(f"{len(rows)} native instrument/category/zero-MIDI cases passed")


def main():
    n = Native()
    db = sqlite3.connect("file:<server>/Data/compact.sqlite3?mode=ro", uri=True)
    mapping_sql = """SELECT s.id,s.skill_id,s.application_method_id,s.effect_id,
                      e.actual_type,e.actual_id,x.special_effect_type_id
                      FROM skill_effects s JOIN effects e ON e.id=s.effect_id
                      JOIN special_effects x ON x.id=e.actual_id
                      WHERE e.actual_type='SpecialEffect' AND x.special_effect_type_id IN (93,94,97)
                      ORDER BY s.id"""
    mapping = db.execute(mapping_sql).fetchall()
    assert len(mapping) == 4
    ids = {"skill_effects": {r[0] for r in mapping}, "effects": {r[3] for r in mapping},
           "special_effects": {r[5] for r in mapping}}
    game_db, stmt = 0x100A0000, 0x100B1000
    payload_ids, payloads = 0x100C0000, 0x100C1000
    effect_ids, effects = 0x100C2000, 0x100C3000
    skill_ids, skill_rows = 0x100C4000, 0x100C5000
    n.put(game_db+0x167C, payload_ids, payloads)
    n.put(game_db+0x251C, effect_ids, effects)
    n.put(game_db+0x2584, skill_ids, skill_rows)
    queries = []
    rows, current = [], -1

    def prepare():
        nonlocal rows, current
        sql = n.string(n.arg(1)).decode()
        table = sql.split(" FROM ")[1]
        rows = [r for r in db.execute(sql).fetchall() if r[0] in ids[table]]
        current = -1
        n.put(n.arg(3), stmt)
        queries.append({"sql": sql, "selected_fixture_ids": sorted(ids[table]), "rows": rows})
        n.ret()

    def step():
        nonlocal current
        current += 1
        n.ret(100 if current < len(rows) else 101)

    def column_int():
        n.ret(int(rows[current][n.arg(1)] or 0))

    def column_text():
        value = str(rows[current][n.arg(1)] or "")
        n.u.mem_write(0x100B2000, value.encode()+b"\0")
        n.ret(0x100B2000)

    def finalizer():
        n.calls.append({"endpoint": "catalog_index_fixture", "count": n.arg(0)})
        n.ret(pop=4)

    def strcmp():
        a, b = n.string(n.arg(0)), n.string(n.arg(1))
        # Used only by the effects loader's type-name dispatch.
        n.ret((a > b)-(a < b))

    def equal_string():
        n.ret(int(n.string(n.arg(0)) == n.string(n.arg(1))))

    n.hooks.update({0x39715230: lambda: n.ret(1), 0x39718610: lambda: n.ret(1),
                    0x39718780: lambda: n.ret(1), 0x3955B3D0: prepare,
                    0x3955FB10: step, 0x3953A390: column_int,
                    0x3953A480: column_text, 0x395519B0: lambda: n.ret(),
                    0x39721C10: finalizer, 0x397255F0: finalizer,
                    0x39725790: finalizer, 0x39005B60: equal_string,
                    0x30000400: strcmp})
    # The first two effect-name comparisons use the imported strcmp equivalent.
    import pefile
    from user_music_lifecycle_oracle import GAME
    pe = pefile.PE(str(GAME))
    for d in pe.DIRECTORY_ENTRY_IMPORT:
        for x in d.imports:
            if x.name and b'XlCompareString@@' in x.name:
                n.put(x.address, 0x30000400)

    n.call(0x3972C390, 0, 1, game_db)
    payload_by_id = {n.get(payload_ids+i*4): payloads+i*20 for i in range(4)}
    for row in mapping:
        assert n.get(payload_by_id[row[5]]) == row[6]
    # Actual range lookup396e8d40 expects base+4/+8 triples.
    def ranges(base, records, ptr):
        low, high = min(records), max(records)
        n.put(base+4, ptr, ptr+12)
        n.put(ptr, low, high, ptr+16)
        for key, value in records.items():
            n.put(ptr+16+4*(key-low), value)
    ranges(game_db+0x161C, payload_by_id, 0x100D0000)
    n.call(0x39730620, 0, 1, game_db)
    effects_by_id = {n.get(effect_ids+i*4): effects+i*12 for i in range(4)}
    for row in mapping:
        record = effects_by_id[row[3]]
        assert n.get(record+4) == 24 and n.get(record+8) == payload_by_id[row[5]]
    ranges(game_db+0x24BC, effects_by_id, 0x100D3000)
    n.call(0x39730EE0, 0, 1, game_db)
    skill_by_id = {}
    loaded = []
    for i in range(4):
        record = skill_rows+i*0x54
        row_id = n.get(skill_ids+i*4)
        row = next(r for r in mapping if r[0] == row_id)
        assert n.get(record+4) == row[2]
        assert n.get(record+0x30) == row[1]
        assert n.get(record+0x1C) == effects_by_id[row[3]]
        skill_by_id[row[1]] = 0x100E0000+i*0x300
        vector = 0x100E2000+i*0x100
        n.put(skill_by_id[row[1]]+0x1E8, vector, vector, vector+0x100)
        loaded.append({"skill_effect_id": row_id, "skill_id": n.get(record+0x30),
                       "application_method": n.get(record+4), "effect_category": 24,
                       "special_action": n.get(n.get(n.get(record+0x1C)+8))})
    ranges(game_db+0x26D4, skill_by_id, 0x100E3000)
    n.put(game_db+0x2578, 4)
    # Execute the original attachment loop using real row/index/vector storage.
    # Vector capacity is preallocated, so no vector-growth fixture is required.
    frame = 0x201F8000
    n.put(frame-0x40, game_db)
    n.put(frame-0x3C, 0)
    n.u.reg_write(UC_X86_REG_EBP, frame)
    n.u.reg_write(UC_X86_REG_ESI, game_db)
    n.u.reg_write(UC_X86_REG_EBX, 0)
    n.hooks[0x39732F12] = lambda: n.u.emu_stop()
    n.call(0x39732E50, end=0x39732F12)
    for i, row in enumerate(loaded):
        skill = skill_by_id[row["skill_id"]]
        start, end = n.get(skill+0x1E8), n.get(skill+0x1EC)
        assert end-start == 4
        row["attached_original_row"] = n.get(start) == skill_rows+i*0x54
        assert row["attached_original_row"]
    result = {"dll_sha256": n.images[0][1], "mapping_sql": mapping_sql,
              "score_memorized_buff_skills": db.execute(
                  "SELECT id,buff_id,skill_id FROM buff_skills WHERE buff_id=6010 ORDER BY id").fetchall(),
              "input": "actual compact.sqlite3 rows, restricted to four music SpecialEffect rows",
              "limitations": "SQLite/capacity/index sinks are fixtures. Original column reads, row stores, effect resolution and attachment loop execute.",
              "native_queries": queries, "loaded_and_attached": loaded}
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT/"native-catalog.json").write_text(json.dumps(result, indent=2)+"\n", encoding="utf-8")
    server_source()
    instrument_cases(db)
    print("4 native special payloads -> 4 native effects -> 4 native skill-effect rows -> 4 attached vectors passed")


if __name__ == "__main__":
    main()
