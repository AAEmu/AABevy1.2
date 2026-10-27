#!/usr/bin/env python3
"""Housing design ids that collide with NPC template ids in a compact database.

A house unit's `SCUnitState` template id is its housing design id
(`housings.id`), not an `npcs.id`. The client's HUD looks rows up by template
id, so a design whose id is also an NPC id can be shown with that NPC's
name/faction/name-tag flags. This tool lists every such collision from the
read-only game database; it does not talk to a client or a server.

    python tools/house_plate_collisions.py --db <compact.sqlite3> \
        --csv tools/evidence/house_plate/collisions.csv \
        --json tools/evidence/house_plate/collisions.json
"""
from __future__ import annotations

import argparse
import csv
import json
import sqlite3
from pathlib import Path

QUERY = """
SELECT h.id, h.name, h.main_model_id, h.hp,
       n.name, n.faction_id, n.show_name_tag, n.npc_kind_id, n.npc_grade_id
  FROM housings h
  JOIN npcs n ON n.id = h.id
 ORDER BY h.id
"""


def id_spaces(conn: sqlite3.Connection) -> dict:
    spaces = {}
    for table in ("housings", "slaves", "transfers", "shipyards"):
        rows = conn.execute(f'SELECT count(*) FROM "{table}"').fetchone()[0]
        hits = conn.execute(f'SELECT count(*) FROM "{table}" x JOIN npcs n ON n.id = x.id').fetchone()[0]
        spaces[table] = {"rows": rows, "id_also_in_npcs": hits}

    mate_npcs = conn.execute("SELECT count(*) FROM npcs WHERE coalesce(mate_kind_id, 0) <> 0").fetchone()[0]
    summon_rows = conn.execute("SELECT count(*) FROM item_summon_mates").fetchone()[0]
    summon_distinct = conn.execute("SELECT count(DISTINCT npc_id) FROM item_summon_mates").fetchone()[0]
    summon_missing = conn.execute(
        "SELECT count(*) FROM item_summon_mates s LEFT JOIN npcs n ON n.id = s.npc_id WHERE n.id IS NULL"
    ).fetchone()[0]
    summon_no_kind = conn.execute(
        "SELECT count(*) FROM item_summon_mates s JOIN npcs n ON n.id = s.npc_id "
        "WHERE coalesce(n.mate_kind_id, 0) = 0"
    ).fetchone()[0]
    horse = conn.execute(
        "SELECT n.id, n.name, l.en_us, n.mate_kind_id, n.faction_id "
        "FROM npcs n JOIN localized_texts l ON l.tbl_name='npcs' AND l.idx=n.id "
        "WHERE l.en_us='Brown Lilyut Horse' AND coalesce(n.mate_kind_id, 0) <> 0 LIMIT 1"
    ).fetchone()
    spaces["mates"] = {
        "npc_rows_with_mate_kind": mate_npcs,
        "summon_mate_rows": summon_rows,
        "summon_mate_distinct_npc_ids": summon_distinct,
        "summon_mate_npc_ids_missing_from_npcs": summon_missing,
        "summon_mate_npc_rows_without_mate_kind": summon_no_kind,
        "brown_lilyut_horse": {
            "id": horse[0],
            "name": horse[1],
            "name_en": horse[2],
            "mate_kind_id": horse[3],
            "faction_id": horse[4],
        } if horse else None,
    }
    spaces["client_reads_npcs_for"] = ["npc", "mate"]
    spaces["client_keeps_out_of_npcs"] = ["housing", "slave", "transfer", "shipyard"]
    return spaces


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=Path, help="compact.sqlite3 (read-only)")
    parser.add_argument("--csv", required=True, type=Path)
    parser.add_argument("--json", required=True, type=Path)
    args = parser.parse_args()

    conn = sqlite3.connect(f"file:{args.db.as_posix()}?mode=ro", uri=True)
    try:
        rows = conn.execute(QUERY).fetchall()
        designs = conn.execute("SELECT count(*) FROM housings").fetchone()[0]
        npcs = conn.execute("SELECT count(*) FROM npcs").fetchone()[0]
        spaces = id_spaces(conn)
    finally:
        conn.close()

    args.csv.parent.mkdir(parents=True, exist_ok=True)
    with args.csv.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow([
            "design_id", "design_name", "design_model_id", "design_hp",
            "npc_name", "npc_faction_id", "npc_show_name_tag",
            "npc_kind_id", "npc_grade_id",
        ])
        for r in rows:
            w.writerow(r)

    document = {
        "source": args.db.name,
        "housing_designs": designs,
        "npc_templates": npcs,
        "collisions": len(rows),
        "id_spaces": spaces,
        "rows": [
            {
                "design_id": r[0],
                "design_name": r[1],
                "design_model_id": r[2],
                "design_hp": r[3],
                "npc_name": r[4],
                "npc_faction_id": r[5],
                "npc_show_name_tag": bool(r[6]),
                "npc_kind_id": r[7],
                "npc_grade_id": r[8],
            }
            for r in rows
        ],
    }
    args.json.write_text(json.dumps(document, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{len(rows)} housing designs of {designs} collide with an NPC id ({npcs} npcs)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
