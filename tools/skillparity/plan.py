#!/usr/bin/env python3
"""Skill parity plan: which skills to cast, in which order, on which action bar slot, with which weapon, how long to wait.

    python tools/skillparity/plan.py --db <compact.sqlite3> --trees 1,2 --out plan.json
    AA_DB=<compact.sqlite3> python tools/skillparity/plan.py            # all ten trees + the default skills

Reads the server's game database (`AA_DB` or `--db`, plain SQLite, opened read-only): `skills`, `skill_effects` (not needed
for the plan itself), `plot_events` / `plot_next_events` (how long a plot runs) and `localized_texts` (English names).
The ten trees are `skills.ability_id` 1..10; the shown skills are the rows with `SHOW = 't'` (11 per tree, 110 in all),
the same selection as `tools/reel/plan.py` and `crates/aa-net/examples/skilltest.rs`.

The plan is JSON (`skillparity/plan/v1`): a list of `batches` that both runners walk in order. A batch is one action bar
setup: `slots` maps slot number to skill id, and its skills follow with `slot`, `key`, `target` (`unit` / `self` /
`ground`), `select` (what to have selected before the press: `dummy`, `self`), and `wait_ms` (how long to record after the
press). The action bar has `--slots-per-page` slots (default 10: the keys `1`..`0`; use 12 to also use `-` and `=`). A tree
has 11 shown skills, so with 10 slots a tree is two batches (10 + 1) and the runners re-slot the bar between them
(`X2Action:SetActionSpell`), which is cheaper and surer than switching bar pages. The default skills (the melee attack and
a few more) are their own batch of tree 0.

Wait after the press = cast + channel (capped, `--channel-cap`, default 6 s) + flight (effect delay + distance / effect
speed) + the longest plot path when the skill has a plot + `--tail` (default 2.5 s), at least `--min-wait`. A channel that
runs past the cap is stopped at the cap (`stop_after_ms`).
"""
from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

PLAN_SCHEMA = "skillparity/plan/v1"
TREES = {1: "Battlerage", 2: "Witchcraft", 3: "Defense", 4: "Auramancy", 5: "Occultism",
         6: "Archery", 7: "Sorcery", 8: "Shadowplay", 9: "Songcraft", 10: "Vitalism"}
SLOT_KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "="]   # README press rows: key "1".."0","-","=" = slot 1..12

DUMMY_TEMPLATE = 13013          # "Forward Scarecrow", a level 50 training doll (the doll of the skill reel and of skilltest.rs)
DUMMY_NAME = "Forward Scarecrow"
# the default skills of skilltest.rs: melee attack, kick, Shoot Arrow, Dash first; the rest only with --defaults all
DEFAULTS_BASIC = [2, 14495, 16064, 16287]
DEFAULTS_ALL = DEFAULTS_BASIC + [14900, 14901, 14902, 14152, 15229, 14451, 10594]
AUTO_ATTACK = {2, 16064}          # the repeating basic attacks (melee attack, Shoot Arrow): pressed once, they keep swinging
GROUND_TYPES = {6, 11, 15, 18}    # skilltest.rs: target types answered with a ground position (`CSStartSkill` position target)
SELF_TYPES = {0, 1, 5}            # skilltest.rs: cast without a target

# What the caster holds per tree: item ids from tools/reel/plan.py (WEAPONS: the first-steps civilian gear, items.id). Names are
# read from the database when it has them. `wield` is the plot condition class (1 one hand, 2 two hand, 3 dual wield).
# `own` = the character's own gear (sword 5569 + shield 6152 + bow 6127 + lute 6177); the bow and lute come out by themselves.
GREATSWORD, STAFF, SWORD, SHIELD, BOW, LUTE = 5660, 6102, 5569, 6152, 6127, 6177
WEAPONS = {
    0: dict(name="own gear (sword, shield, bow)", mainhand=SWORD, offhand=SHIELD, ranged=BOW, wield=1),
    1: dict(name="two-handed sword", mainhand=GREATSWORD, offhand=0, wield=2),
    2: dict(name="staff", mainhand=STAFF, offhand=0, wield=2),
    3: dict(name="sword and shield", mainhand=SWORD, offhand=SHIELD, wield=1),
    4: dict(name="staff", mainhand=STAFF, offhand=0, wield=2),
    5: dict(name="staff", mainhand=STAFF, offhand=0, wield=2),
    6: dict(name="bow", mainhand=0, offhand=0, ranged=BOW, wield=1),
    7: dict(name="staff", mainhand=STAFF, offhand=0, wield=2),
    8: dict(name="dual swords", mainhand=SWORD, offhand=SWORD, wield=3),
    9: dict(name="lute", mainhand=0, offhand=0, musical=LUTE, wield=1),
    10: dict(name="staff", mainhand=STAFF, offhand=0, wield=2),
}

CLIP_SYNC_MS = 450     # tools/reel/plan.py CLIP_SYNC: an `add_anim_cs_time` edge adds about this much (the clip's own sync moment)


def connect(db: str | os.PathLike) -> sqlite3.Connection:
    """Open the game database read-only."""
    path = Path(db)
    if not path.is_file():
        raise FileNotFoundError(f"game database not found: {path} (set AA_DB or pass --db)")
    return sqlite3.connect(f"file:{path.resolve().as_posix()}?mode=ro", uri=True)


def english_names(conn: sqlite3.Connection, table: str) -> dict[int, str]:
    rows = conn.execute("select idx, en_us from localized_texts where tbl_name=? and tbl_column_name='name' and en_us<>''", (table,))
    return {int(i): n for i, n in rows}


def target_mode(target_type_id: int | None) -> str:
    """`unit`, `self` or `ground`: how the cast is aimed (skilltest.rs: ground types 6/11/15/18, self types 0/1/5)."""
    t = target_type_id if target_type_id is not None else 0
    if t in GROUND_TYPES:
        return "ground"
    if t in SELF_TYPES:
        return "self"
    return "unit"


def plot_span_ms(conn: sqlite3.Connection, plot_id: int | None, cap_ms: int = 8000) -> int:
    """Longest chain of `plot_next_events.delay` (ms) from a plot's first event, ignoring the `fail` branches."""
    if not plot_id:
        return 0
    events = conn.execute("select id from plot_events where plot_id=? order by position", (plot_id,)).fetchall()
    if not events:
        return 0
    edges: dict[int, list[tuple[int, int]]] = {}
    ids = [e[0] for e in events]
    marks = ",".join("?" * len(ids))
    for event_id, nxt, delay, fail, cs in conn.execute(
            f"select event_id, next_event_id, delay, fail, add_anim_cs_time from plot_next_events where event_id in ({marks})", ids):
        if fail == "t":
            continue
        edges.setdefault(event_id, []).append((nxt, int(delay or 0) + (CLIP_SYNC_MS if cs == "t" else 0)))
    memo: dict[int, int] = {}

    def longest(eid: int, seen: frozenset) -> int:
        if eid in memo:
            return memo[eid]
        best = 0
        for nxt, delay in edges.get(eid, []):
            if nxt in seen:
                continue
            best = max(best, delay + longest(nxt, seen | {eid}))
            if best >= cap_ms:
                best = cap_ms
                break
        memo[eid] = best
        return best

    return min(longest(ids[0], frozenset()), cap_ms)


SKILL_COLUMNS = ("id, name, ability_id, ability_level, target_type_id, target_selection_id, target_relation_id, casting_time, "
                 "channeling_time, effect_delay, effect_speed, projectile_id, plot_id, cooldown_time, default_gcd, custom_gcd, "
                 "min_range, max_range, active_weapon_id, start_autoattack, mana_cost")


def tree_skill_ids(conn: sqlite3.Connection, tree: int) -> list[int]:
    """The shown skills of a tree in skill book order (level, id)."""
    rows = conn.execute("select id from skills where ability_id=? and \"SHOW\"='t' order by ability_level, id", (tree,))
    return [r[0] for r in rows]


def skill_entry(conn: sqlite3.Connection, names: dict[int, str], sid: int, *, distance_m: float, channel_cap_ms: int,
                tail_ms: int, min_wait_ms: int, autoattack_ms: int) -> dict | None:
    row = conn.execute(f"select {SKILL_COLUMNS} from skills where id=?", (sid,)).fetchone()
    if row is None:
        return None
    (_, name_db, ability, level, ttype, _sel, relation, cast, chan, delay, speed, projectile, plot, cooldown, default_gcd, custom_gcd,
     min_range, max_range, active_weapon, start_auto, mana) = row
    cast, chan, delay, cooldown = (int(v or 0) for v in (cast, chan, delay, cooldown))
    speed = float(speed or 0.0)
    mode = target_mode(ttype)
    autoattack = sid in AUTO_ATTACK
    # flight: the effect delay plus the travel time to the doll; a projectile flies at least the typical 10 m (tools/reel/plan.py)
    flight = delay
    if speed > 0 and (projectile or mode == "unit"):
        flight += round(max(distance_m, 10.0 if projectile else 0.0) / speed * 1000.0)
    plot_ms = plot_span_ms(conn, plot)
    chan_capped = min(chan, channel_cap_ms)
    stop_after = None
    if autoattack:
        stop_after = autoattack_ms
        wait = autoattack_ms + tail_ms
    else:
        busy = cast + chan_capped + flight
        if plot_ms:
            busy = max(busy, cast + plot_ms)
        if chan > channel_cap_ms:
            stop_after = cast + channel_cap_ms
            wait = stop_after + tail_ms
        else:
            wait = busy + tail_ms
    wait = max(wait, min_wait_ms)
    entry = {
        "id": sid,
        "name": names.get(sid) or name_db,
        "tree": ability,
        "level": level,
        "target": mode,
        "target_type_id": ttype,
        "relation_id": relation,
        "select": "self" if mode == "self" else "dummy",
        "needs_target": mode == "unit",
        "needs_click": mode == "ground",
        "cast_ms": cast,
        "channel_ms": chan,
        "channel_capped_ms": chan_capped,
        "flight_ms": int(flight),
        "plot": bool(plot),
        "plot_span_ms": plot_ms,
        "wait_ms": int(wait),
        "cooldown_ms": cooldown,
        "gcd": default_gcd == "t" or bool(custom_gcd),
        "gcd_ms": int(custom_gcd or 0),
        "min_range": min_range,
        "max_range": max_range,
        "active_weapon_id": active_weapon,
        "autoattack": autoattack,
    }
    if stop_after is not None:
        entry["stop_after_ms"] = int(stop_after)
    return entry


def build_plan(conn: sqlite3.Connection, trees: list[int], defaults: str = "basic", slots_per_page: int = 10,
               distance_m: float = 4.0, channel_cap_ms: int = 6000, tail_ms: int = 2500, min_wait_ms: int = 3000,
               autoattack_ms: int = 5000, only_skills: list[int] | None = None, weapons: dict | None = None) -> dict:
    """The whole plan (see the module doc). `weapons` overrides the WEAPONS table (tree -> weapon dict)."""
    if not 1 <= slots_per_page <= len(SLOT_KEYS):
        raise ValueError(f"--slots-per-page must be 1..{len(SLOT_KEYS)}")
    names = english_names(conn, "skills")
    item_names = english_names(conn, "items")
    table = {**WEAPONS, **(weapons or {})}
    kw = dict(distance_m=distance_m, channel_cap_ms=channel_cap_ms, tail_ms=tail_ms, min_wait_ms=min_wait_ms,
              autoattack_ms=autoattack_ms)

    groups: list[tuple[int, str, list[int]]] = []
    for tree in trees:
        ids = tree_skill_ids(conn, tree)
        if only_skills:
            ids = [i for i in ids if i in only_skills]
        if ids:
            groups.append((tree, TREES.get(tree, f"Tree {tree}"), ids))
    default_ids = {"none": [], "basic": DEFAULTS_BASIC, "all": DEFAULTS_ALL}[defaults]
    if only_skills:
        extra = [i for i in only_skills if i in DEFAULTS_ALL and defaults != "none"]
        default_ids = [i for i in default_ids if i in only_skills] or extra
    if default_ids:
        groups.append((0, "Defaults", list(default_ids)))

    batches: list[dict] = []
    skills_flat: list[dict] = []
    for tree, tree_name, ids in groups:
        weapon = dict(table.get(tree) or table[0])
        for key in ("mainhand", "offhand", "ranged", "musical"):
            if weapon.get(key):
                weapon.setdefault("names", {})[key] = item_names.get(weapon[key], str(weapon[key]))
        for start in range(0, len(ids), slots_per_page):
            chunk = ids[start:start + slots_per_page]
            entries = []
            for pos, sid in enumerate(chunk):
                e = skill_entry(conn, names, sid, **kw)
                if e is None:
                    continue
                e.update(slot=pos + 1, key=SLOT_KEYS[pos], batch=len(batches) + 1)
                entries.append(e)
            if not entries:
                continue
            batch = {"batch": len(batches) + 1, "tree": tree, "tree_name": tree_name, "weapon": weapon,
                     "slots": {str(e["slot"]): e["id"] for e in entries}, "skills": entries}
            batches.append(batch)
            skills_flat.extend(entries)

    return {
        "schema": PLAN_SCHEMA,
        "generated": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "trace_schema": common.SCHEMA,
        "slots_per_page": slots_per_page,
        "slot_keys": SLOT_KEYS[:slots_per_page],
        "timing": {"channel_cap_ms": channel_cap_ms, "tail_ms": tail_ms, "min_wait_ms": min_wait_ms,
                   "autoattack_ms": autoattack_ms, "distance_m": distance_m},
        "dummy": {"template": DUMMY_TEMPLATE, "name": names_of_npc(conn) or DUMMY_NAME,
                  "note": "level 50 training doll; spawn it 3-5 m in front of the caster at a fixed spot"},
        "setup": {"level": 50, "gm": ["/level 50", "/ignoreskillcds true"]},
        "batches": batches,
        "skills": skills_flat,
        "counts": {"batches": len(batches), "skills": len(skills_flat),
                   "trees": sorted({b["tree"] for b in batches})},
    }


def names_of_npc(conn: sqlite3.Connection) -> str | None:
    try:
        row = conn.execute("select en_us from localized_texts where tbl_name='npcs' and tbl_column_name='name' and idx=?",
                           (DUMMY_TEMPLATE,)).fetchone()
        return row[0] if row and row[0] else None
    except sqlite3.Error:
        return None


def load_plan(path: str | os.PathLike) -> dict:
    plan = json.loads(Path(path).read_text(encoding="utf-8"))
    if plan.get("schema") != PLAN_SCHEMA:
        raise ValueError(f"{path} is not a {PLAN_SCHEMA} plan")
    return plan


def select_batches(plan: dict, trees: list[int] | None = None, skills: list[int] | None = None) -> list[dict]:
    """The plan's batches reduced to the wanted trees / skill ids (a copy; slots keep their numbers)."""
    out = []
    for batch in plan["batches"]:
        if trees and batch["tree"] not in trees:
            continue
        entries = [s for s in batch["skills"] if not skills or s["id"] in skills]
        if not entries:
            continue
        out.append({**batch, "skills": entries, "slots": {str(s["slot"]): s["id"] for s in entries}})
    return out


def parse_int_list(text: str | None) -> list[int]:
    return [int(x) for x in text.replace(" ", "").split(",") if x] if text else []


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", default=os.environ.get("AA_DB"), help="the server's compact.sqlite3 (default: AA_DB)")
    ap.add_argument("--trees", default=",".join(str(t) for t in TREES), help="ability trees, e.g. 1,2 (default: all ten)")
    ap.add_argument("--skills", default="", help="only these skill ids (of the chosen trees / defaults)")
    ap.add_argument("--defaults", choices=("none", "basic", "all"), default="basic",
                    help="the default skills batch: basic = melee attack, kick, Shoot Arrow, Dash; all = + instruments, dances, Recall")
    ap.add_argument("--slots-per-page", type=int, default=10, help="action bar slots to use per batch (10 = keys 1..0; 12 adds - and =)")
    ap.add_argument("--distance", type=float, default=4.0, help="metres from the caster to the doll (flight time)")
    ap.add_argument("--channel-cap", type=float, default=6.0, help="seconds a channel runs before it is stopped")
    ap.add_argument("--tail", type=float, default=2.5, help="seconds recorded after the cast, flight and plot")
    ap.add_argument("--min-wait", type=float, default=3.0, help="least seconds recorded per skill")
    ap.add_argument("--weapons", type=Path, default=None, help='JSON {"tree": {"name":..,"mainhand":item,"offhand":item}} overriding the table')
    ap.add_argument("--out", type=Path, default=None, help="plan.json to write (default: print)")
    args = ap.parse_args(argv)
    if not args.db:
        print("plan: no game database: pass --db or set AA_DB", file=sys.stderr)
        return 2
    try:
        conn = connect(args.db)
    except (FileNotFoundError, sqlite3.Error) as error:
        print(f"plan: {error}", file=sys.stderr)
        return 2
    weapons = None
    if args.weapons:
        weapons = {int(k): v for k, v in json.loads(args.weapons.read_text(encoding="utf-8")).items()}
    try:
        plan = build_plan(conn, parse_int_list(args.trees), args.defaults, args.slots_per_page, args.distance,
                          int(args.channel_cap * 1000), int(args.tail * 1000), int(args.min_wait * 1000),
                          only_skills=parse_int_list(args.skills) or None, weapons=weapons)
    finally:
        conn.close()
    text = json.dumps(plan, indent=1, ensure_ascii=False)
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(text + "\n", encoding="utf-8")
        total = sum(s["wait_ms"] for s in plan["skills"]) / 1000.0
        print(f"plan: {plan['counts']['skills']} skills in {plan['counts']['batches']} batches, "
              f"about {total / 60:.1f} min of recording -> {args.out}")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
