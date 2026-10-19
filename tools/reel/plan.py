"""Skill reel plan: for each ability tree, the AA_COMBAT_FAKE script that casts every shown skill in turn.

    python plan.py                 # prints a summary and writes plans/<n>_<ability>.json
    python plan.py 7               # only tree 7

A plan has the client environment (weapons, fake events), the caption schedule and the video length.
All times are seconds on the fake clock (0 = the moment the player appeared); the recording starts at
RECORD_FROM on that clock, so video time = fake time - RECORD_FROM (the exact origin is logged by aa-dev:
`aa-dev: record start at gate+X s`, the run script corrects the captions with it).
"""
import csv
import json
import sqlite3
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent          # tools/reel (scripts, tracked)
ROOT = HERE.parents[1]
WORK = ROOT / "data/extract/reel"                 # plans, frames, logs, venv (git-ignored)
DB = "file:D:/aaemu-live/rs-game/Data/compact.sqlite3?mode=ro"

# ability_id -> real 1.2 name (ui_texts 1111..1120)
ABILITIES = {
    1: "Battlerage", 2: "Witchcraft", 3: "Defense", 4: "Auramancy", 5: "Occultism",
    6: "Archery", 7: "Sorcery", 8: "Shadowplay", 9: "Songcraft", 10: "Vitalism",
}
STAFF = 6102      # "first steps magic staff", two handed
GREATSWORD = 5660  # first two handed sword
SWORD = 5569      # the test character's own sword
# what the caster holds per tree (AA_DEV_MAINHAND / OFFHAND / MUSICAL; 0 = nothing; unset = the character's own: sword 5569,
# shield 6152, bow 6127, lute 6177). The bow and the lute come out by themselves for bow_ / music_ clips.
WEAPONS = {
    1: {"MAINHAND": GREATSWORD, "OFFHAND": 0},
    2: {"MAINHAND": STAFF, "OFFHAND": 0},
    3: {},  # sword and shield
    4: {"MAINHAND": STAFF, "OFFHAND": 0},
    5: {"MAINHAND": STAFF, "OFFHAND": 0},
    6: {},  # bow (own ranged slot)
    7: {"MAINHAND": STAFF, "OFFHAND": 0},
    8: {"MAINHAND": SWORD, "OFFHAND": SWORD},  # dual wield
    9: {},  # lute
    10: {"MAINHAND": STAFF, "OFFHAND": 0},
}
WEAPON_NAME = {1: "two-handed sword", 2: "staff", 3: "sword and shield", 4: "staff", 5: "staff", 6: "bow",
               7: "staff", 8: "dual swords", 9: "lute", 10: "staff"}
# class of the wielded weapon for plot conditions (kind 6: 1 one hand, 2 two hand, 3 dual wield)
WIELD = {1: 2, 2: 2, 3: 1, 4: 2, 5: 2, 6: 1, 7: 2, 8: 3, 9: 1, 10: 2}

RECORD_FROM = 14.0   # fake seconds before the first frame (the world is still streaming in before)
FIRST_SKILL = 15.5   # fake time of the first skill
MIN_SLOT = 2.5       # seconds per skill, at least
TAIL = 1.0           # after the cast, the channel and the flight
CHANNEL_CAP = 4.0    # long channels (Play Dead 60 s) end after this
CLIP_SYNC = 0.45     # seconds an `add_anim_cs_time` edge adds
COMBO = {13282}      # skills whose three stages are three casts (plot conditions of kind 17)
STAGE_GAP = 1.3      # between the casts of a combo skill
TARGET_ENV = "model=1290"  # AA_COMBAT_FAKE_TARGET: the training dummy (npc 7512, model 1290)


def connect():
    return sqlite3.connect(DB, uri=True)


def names(c):
    return {r[0]: r[1] for r in c.execute(
        "select idx,en_us from localized_texts where tbl_name='skills' and tbl_column_name='name' and en_us<>''")}


def tree_skills(c, ability):
    """Shown skills of a tree, in the order of the skill book (level, id)."""
    return c.execute(
        "select id, casting_time, channeling_time, effect_delay, projectile_id, plot_id, target_type_id, ability_level, target_relation_id "
        "from skills where ability_id=? and \"SHOW\"='t' order by ability_level, id", (ability,)).fetchall()


def target_self(ability, target_type):
    """Does the skill land on the caster (buffs, self centred area skills, friendly spells)?"""
    if target_type in (0, 1):
        return True
    if target_type == 5:
        return ability == 10
    return False


# ---- plots -------------------------------------------------------------------------------------------------------

VISUAL_TYPES = {34, 35, 36, 37, 38}  # Anim, FxGroup, FxGroupAnim, Projectile, ProjectileAnim


class Plot:
    def __init__(self, c, plot_id):
        self.c = c
        self.events = {}
        for eid, pos in c.execute("select id, position from plot_events where plot_id=? order by position", (plot_id,)):
            effects = c.execute("select actual_type, actual_id from plot_effects where event_id=?", (eid,)).fetchall()
            visual = False
            damage = False
            for kind, actual in effects:
                if kind == "SpecialEffect":
                    t = c.execute("select special_effect_type_id from special_effects where id=?", (actual,)).fetchone()
                    if t and t[0] in VISUAL_TYPES:
                        visual = True
                elif kind == "DamageEffect":
                    damage = True
            conds = [r[0] for r in c.execute(
                "select condition_id from plot_event_conditions where event_id=? order by position", (eid,))]
            nexts = c.execute(
                "select next_event_id, casting, delay, channeling, fail, add_anim_cs_time from plot_next_events where event_id=? order by position",
                (eid,)).fetchall()
            self.events[eid] = dict(pos=pos, visual=visual, damage=damage, conds=conds, nexts=nexts)
        self.start = min(self.events, key=lambda e: self.events[e]["pos"]) if self.events else None
        self.cond_cache = {}

    def cond(self, cid, wield, stage):
        r = self.cond_cache.get(cid)
        if r is None:
            r = self.cond_cache[cid] = self.c.execute(
                "select not_condition, kind_id, param1, param2, param3 from plot_conditions where id=?", (cid,)).fetchone()
        if r is None:
            return True
        neg, kind, p1, p2, p3 = r
        if kind == 6:        # wielded weapon class
            ok = p1 == wield
        elif kind == 17:     # combo stage by the time since the last cast: ranges 1.., 25.., 40..
            ok = stage == (1 if p2 < 25 else 2 if p2 < 40 else 3)
        else:
            return True      # hit checks, distances, buffs: assume they pass
        return ok != (neg == "t")

    def walk(self, wield, stage):
        """[(t, event, casting_ms, damage)] of one cast, t in seconds after the first event; and the end time."""
        out = []
        seen = set()
        last = [0.0]

        def go(eid, t):
            if eid in seen or eid not in self.events:
                return
            seen.add(eid)
            e = self.events[eid]
            ok = all(self.cond(cid, wield, stage) for cid in e["conds"])
            casting = 0
            for nxt, cast, delay, chan, fail, plus_clip in e["nexts"]:
                if (fail == "t") == (not ok):
                    if cast == "t" and delay and ok:
                        casting = max(casting, delay)
                    # `add_anim_cs_time`: the delay comes on top of the clip's own sync moment (about 0.45 s into a swing)
                    go(nxt, t + delay / 1000.0 + (CLIP_SYNC if plus_clip == "t" else 0.0))
            if ok and (e["visual"] or e["damage"]):
                out.append((t, eid, casting, e["damage"]))
                last[0] = max(last[0], t)

        go(self.start, 0.0)
        out.sort(key=lambda x: (x[0], self.events[x[1]]["pos"]))
        return out, last[0]


# ---- one skill ---------------------------------------------------------------------------------------------------

def skill_events(c, ability, row, tl, t0):
    """Fake steps ["T:op:args", ...] of one skill started at t0, and its length in seconds."""
    sid, cast, chan, delay, proj, plot, ttype, _lvl, relation = row
    cast = cast or 0
    chan = chan or 0
    delay = delay or 0
    me = "@ground" if ttype == 6 else "@self" if target_self(ability, ttype) else ""
    steps = []
    # does it hurt someone (the dummy takes the hit effects even when the skill is centred on the caster)?
    hit = relation == 4 or ttype in (4, 6) or (ttype == 5 and ability != 10)

    def add(t, op, *args):
        steps.append(f"{t:.3f}:{op}{'' if op == 'hit' else me}:" + ":".join(str(a) for a in args))

    if plot:
        p = Plot(c, plot)
        stages = 3 if sid in COMBO else 1
        t = t0
        end = t0
        for stage in range(1, stages + 1):
            walked, span = p.walk(WIELD[ability], stage)
            tlx = tl + (stage - 1) * 100
            hits = 0
            hit_times = set()
            for dt, eid, casting, damage in walked:
                at = t + dt
                if eid is not None and p.events[eid]["visual"]:
                    add(at, "plot", tlx, eid, sid, *([casting] if casting else []))
                if damage and hit and hits < 3 and round(at, 1) not in hit_times:
                    add(at + 0.05, "hit", tlx, sid, 120 + stage)
                    hit_times.add(round(at, 1))
                    hits += 1
            add(t + span + 0.4, "plotend", tlx)
            end = max(end, t + span + 0.4)
            t += max(span, 0.0) + STAGE_GAP
        length = max(MIN_SLOT, end - t0 + TAIL)
        return steps, length

    t = t0
    ch = min(chan / 1000.0, CHANNEL_CAP)
    if cast:
        add(t, "start", tl, sid, cast)
        t += cast / 1000.0
    add(t, "fire", tl, sid, delay if delay else (500 if proj else 0), chan)
    if hit:
        add(t + 0.05, "hit", tl, sid, 150)
    end_at = t + max(delay, 0) / 1000.0 + ch + 0.1
    add(end_at, "end", tl)
    length = max(MIN_SLOT, end_at - t0 + TAIL)
    return steps, length


def make_plan(c, ability, nm):
    t = FIRST_SKILL
    steps = []
    captions = []
    for i, row in enumerate(tree_skills(c, ability)):
        sid = row[0]
        ev, length = skill_events(c, ability, row, 100 + i * 10, t)
        # the plot steps of a skill do not all come at its start: the caption is from the start to the next skill
        steps += ev
        captions.append(dict(skill=sid, name=nm.get(sid, str(sid)), start=t, end=t + length))
        t += length
    env = {f"AA_DEV_{k}": str(v) for k, v in WEAPONS[ability].items()}
    env["AA_COMBAT_FAKE"] = ";".join(steps)
    env["AA_COMBAT_FAKE_TARGET"] = TARGET_ENV
    return dict(
        ability=ability, name=ABILITIES[ability], weapon=WEAPON_NAME[ability], env=env, captions=captions,
        record_from=RECORD_FROM, record_secs=round(t - RECORD_FROM + 1.0, 2), end=t,
    )


def main():
    want = [int(a) for a in sys.argv[1:]] or list(ABILITIES)
    c = connect()
    nm = names(c)
    out = WORK / "plans"
    out.mkdir(exist_ok=True)
    for a in want:
        plan = make_plan(c, a, nm)
        (out / f"{a:02d}_{ABILITIES[a].lower()}.json").write_text(json.dumps(plan, indent=1), encoding="utf-8")
        print(f"{a:2d} {ABILITIES[a]:11s} {len(plan['captions']):2d} skills, {plan['record_secs']:6.1f} s, weapon {plan['weapon']}")


if __name__ == "__main__":
    main()
