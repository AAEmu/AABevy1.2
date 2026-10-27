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
DB = "file:<server>/Data/compact.sqlite3?mode=ro"

# ability_id -> real 1.2 name (ui_texts 1111..1120)
ABILITIES = {
    1: "Battlerage", 2: "Witchcraft", 3: "Defense", 4: "Auramancy", 5: "Occultism",
    6: "Archery", 7: "Sorcery", 8: "Shadowplay", 9: "Songcraft", 10: "Vitalism",
}
MOTION = 11  # the extra reel: ground skills with the reticle, then the skills that move units (not one of the ten trees)
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
    11: {},  # the character's own sword and shield
}
WEAPON_NAME = {11: "sword and shield", 1: "two-handed sword", 2: "staff", 3: "sword and shield", 4: "staff", 5: "staff", 6: "bow",
               7: "staff", 8: "dual swords", 9: "lute", 10: "staff"}
# class of the wielded weapon for plot conditions (kind 6: 1 one hand, 2 two hand, 3 dual wield)
WIELD = {11: 1, 1: 2, 2: 2, 3: 1, 4: 2, 5: 2, 6: 1, 7: 2, 8: 3, 9: 1, 10: 2}

RECORD_FROM = 14.0   # fake seconds before the first frame (the world is still streaming in before)
FIRST_SKILL = 15.5   # fake time of the first skill
MIN_SLOT = 2.5       # seconds per skill, at least
TAIL = 1.0           # after the cast, the channel and the flight
CHANNEL_CAP = 8.0    # long channels (Play Dead 60 s, Twilight 60 s) end after this: the channel buff shows for as long
BUFF_SHOW = 3.5      # seconds a (non channel) buff stays on its bearer in the reel, at most; a shorter buff goes when it ends
SHOT_RANGE = 10.0    # metres a projectile skill is timed for when the doll is nearer: the real shots of a bow or a staff fly 8 to 17 m
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
            buffs = []
            for kind, actual in effects:
                if kind == "SpecialEffect":
                    t = c.execute("select special_effect_type_id from special_effects where id=?", (actual,)).fetchone()
                    if t and t[0] in VISUAL_TYPES:
                        visual = True
                elif kind == "DamageEffect":
                    damage = True
            for buff_id, target in c.execute(
                    "select be.buff_id, pe.target_id from plot_effects pe join buff_effects be on be.id=pe.actual_id "
                    "where pe.event_id=? and pe.actual_type='BuffEffect' order by pe.position", (eid,)):
                buffs.append((buff_id, target == 1))  # plot target 1 is the caster, the others are what the step hit
            conds = [r[0] for r in c.execute(
                "select condition_id from plot_event_conditions where event_id=? order by position", (eid,))]
            nexts = c.execute(
                "select next_event_id, casting, delay, channeling, fail, add_anim_cs_time from plot_next_events where event_id=? order by position",
                (eid,)).fetchall()
            self.events[eid] = dict(pos=pos, visual=visual, damage=damage, buffs=buffs, conds=conds, nexts=nexts)
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
            if ok and (e["visual"] or e["damage"] or e["buffs"]):
                out.append((t, eid, casting, e["damage"]))
                last[0] = max(last[0], t)

        go(self.start, 0.0)
        out.sort(key=lambda x: (x[0], self.events[x[1]]["pos"]))
        return out, last[0]


# ---- buffs and flights ---------------------------------------------------------------------------------------------

_look = {}
_index = [5000]


def buff_look(c, buff_id):
    """(shows something, duration in seconds or 0) of a buff: an aura (effect group), a start pose, a freeze or a ragdoll."""
    r = _look.get(buff_id)
    if r is None:
        b = c.execute("select duration, fx_group_id, anim_start_id, framehold, ragdoll, idle_anim from buffs where id=?", (buff_id,)).fetchone()
        r = _look[buff_id] = (False, 0.0) if b is None else (bool(b[1] or b[2] or b[3] == "t" or b[4] == "t" or b[5]), (b[0] or 0) / 1000.0)
    return r


def skill_buffs(c, sid, aimed_at_caster, hostile):
    """[(buff, on the caster, channel buff)] a skill's cast applies, the way the server's SCBuffCreated would carry them.

    `skill_effects` -> BuffEffect rows of the lowest level they apply at (rows that need a buff tag on someone are left out), and the
    channeling / toggle buff of the skill. A buff lands on the target of the skill (the doll); on the caster when the skill aims at the caster,
    or when the row is for friends only and the skill is a hostile one (a buff of the source)."""
    rows = c.execute(
        "select se.start_level, se.end_level, se.friendly, se.non_friendly, be.buff_id from skill_effects se "
        "join effects e on e.id=se.effect_id and e.actual_type='BuffEffect' join buff_effects be on be.id=e.actual_id "
        "where se.skill_id=? and se.target_buff_tag_id is null and se.source_buff_tag_id is null order by se.id", (sid,)).fetchall()
    out, seen = [], set()
    if rows:
        # the level of the first row that shows something (a song's visible buffs start at its own level, 10 or 18, not at 1)
        level = min([r[0] for r in rows if buff_look(c, r[4])[0]] or [r[0] for r in rows])
        for start, end, friendly, non_friendly, buff in rows:
            if not start <= level <= end or buff in seen:
                continue
            seen.add(buff)
            out.append((buff, aimed_at_caster or (hostile and friendly == "t" and non_friendly == "f"), False))
    for col in ("channeling_buff_id", "toggle_buff_id"):
        r = c.execute(f"select {col} from skills where id=?", (sid,)).fetchone()
        if r and r[0] and r[0] not in seen:
            seen.add(r[0])
            out.append((r[0], True, col == "channeling_buff_id"))
    return out


def stage_distance():
    """Metres from the caster to the doll (stage.json), 4 when there is no stage yet."""
    try:
        import math
        st = json.loads((WORK / "stage.json").read_text())
        x, y = (float(v) for v in st["pos"].split(","))
        return math.hypot(st["dummy"][0] - x, st["dummy"][1] - y)
    except (OSError, KeyError, ValueError):
        return 4.0


def buff_steps(c, at, buff, caster, channel):
    """The `buff` / `unbuff` steps of one buff that arrives at `at`; (steps, time it is gone), or ([], at) when it shows nothing."""
    visual, duration = buff_look(c, buff)
    if not visual:
        return [], at
    show = CHANNEL_CAP if channel else BUFF_SHOW
    if duration:
        show = min(show, max(duration, 0.8))
    _index[0] += 1
    suffix = "@self" if caster else ""
    return [f"{at:.3f}:buff{suffix}:{_index[0]}:{buff}:0", f"{at + show:.3f}:unbuff{suffix}:{_index[0]}"], at + show


# ---- one skill ---------------------------------------------------------------------------------------------------

def skill_events(c, ability, row, tl, t0):
    """Fake steps ["T:op:args", ...] of one skill started at t0, and its length in seconds."""
    sid, cast, chan, delay, proj, plot, ttype, _lvl, relation = row
    cast = cast or 0
    chan = chan or 0
    delay = delay or 0
    me = "@ground" if ttype == 6 else "@self" if target_self(ability, ttype) and relation != 4 else ""
    steps = []
    if sid == DROP_BACK:
        # the stage's own 10 m back is a thicket (the camera ends in the leaves): this one is cast from an open spot
        steps.append(f"{t0 - 0.2:.3f}:tp:{DROP_BACK_SPOT}")
    # does it hurt someone (the dummy takes the hit effects even when the skill is centred on the caster)?
    hit = relation == 4 or ttype in (4, 6) or (ttype == 5 and ability != 10)

    def add(t, op, *args):
        steps.append(f"{t:.3f}:{op}{'' if op == 'hit' else me}:" + ":".join(str(a) for a in args))

    gone = [t0]  # when the last buff is off again: the slot has to last that long

    def buffs_at(at, wanted):
        for buff, caster, channel in wanted:
            ev, off = buff_steps(c, at, buff, caster, channel)
            steps.extend(ev)
            gone[0] = max(gone[0], off)

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
                buffs_at(at + 0.05, [(b, caster, False) for b, caster in p.events[eid]["buffs"]])
                if damage and hit and hits < 3 and round(at, 1) not in hit_times:
                    add(at + 0.05, "hit", tlx, sid, 120 + stage)
                    hit_times.add(round(at, 1))
                    hits += 1
            add(t + span + 0.4, "plotend", tlx)
            end = max(end, t + span + 0.4)
            t += max(span, 0.0) + STAGE_GAP
        # the skill's own buff rows and its channel / toggle buff, when the steps of the plot do not name them
        named = {x[0] for ev in p.events.values() for x in ev["buffs"]}
        buffs_at(t0 + 0.1, [b for b in skill_buffs(c, sid, me == "@self", hit) if b[2] or b[0] not in named])
        length = max(MIN_SLOT, max(end + TAIL, gone[0] + 0.3) - t0)
        return steps, length

    t = t0
    ch = min(chan / 1000.0, CHANNEL_CAP)
    if cast:
        add(t, "start", tl, sid, cast)
        t += cast / 1000.0
    # the time the effects take to land, as the server computes it: the launch moment of the clip plus the flight (distance / speed)
    speed = (c.execute("select effect_speed from skills where id=?", (sid,)).fetchone() or (0,))[0] or 0
    metres = max(stage_distance(), SHOT_RANGE) if proj else stage_distance()
    flight = int(round(metres / speed * 1000)) if speed and (proj or hit) else (500 - delay if proj and delay < 500 else 0)
    land = delay + flight
    add(t, "fire", tl, sid, land, chan)
    if hit:
        add(t + (land / 1000.0 if proj else 0.0) + 0.05, "hit", tl, sid, 150)
    buffs_at(t + land / 1000.0 + 0.05, skill_buffs(c, sid, me == "@self", hit))
    end_at = t + land / 1000.0 + ch + 0.1
    add(end_at, "end", tl)
    length = max(MIN_SLOT, max(end_at + TAIL, gone[0] + 0.3) - t0)
    return steps, length


def back_on_stage(t):
    """The `tp` step that puts the caster back on the stage spot (skills that move the caster would carry it off, into the bushes)."""
    st = json.loads((WORK / "stage.json").read_text())
    x, y = (int(round(float(v))) for v in st["pos"].split(","))
    return f"{t:.3f}:tp:{x}:{y}:{(st['yaw'] + 360) % 360}"


def make_plan(c, ability, nm):
    t = FIRST_SKILL
    steps = []
    captions = []
    for i, row in enumerate(tree_skills(c, ability)):
        sid = row[0]
        ev, length = skill_events(c, ability, row, 100 + i * 10, t)
        # the plot steps of a skill do not all come at its start: the caption is from the start to the next skill
        steps += ev
        steps.append(back_on_stage(t + length - 0.35))
        captions.append(dict(skill=sid, name=nm.get(sid, str(sid)), start=t, end=t + length))
        t += length
    env = {f"AA_DEV_{k}": str(v) for k, v in WEAPONS[ability].items()}
    env["AA_COMBAT_FAKE"] = ";".join(steps)
    env["AA_COMBAT_FAKE_TARGET"] = TARGET_ENV
    return dict(
        ability=ability, name=ABILITIES[ability], weapon=WEAPON_NAME[ability], env=env, captions=captions,
        record_from=RECORD_FROM, record_secs=round(t - RECORD_FROM + 1.0, 2), end=t,
    )


# (skill, ability) of the motion reel; the two ground skills come first (the caster has not moved yet), with the reticle shown first
MOTION_SKILLS = [(10664, 7), (13281, 6), (11918, 1), (12039, 1), (10648, 8), (12075, 8), (10152, 4), (12049, 8)]
GROUND_SKILLS = {10664, 13281}
DROP_BACK = 12049
DROP_BACK_SPOT = "15530:15322:0"  # x:y:yaw of an open dirt patch (a back step of 10 m stays on open ground)
RETICLE_LEAD = 2.8       # seconds of aiming before a ground skill is cast
AIM_PIXEL = (140, 545)  # where the doll stands on the 1280x720 screen (camera on the caster's right): the reticle is aimed at it


def make_motion_plan(c, nm):
    t = FIRST_SKILL
    steps, captions, args = [], [], []
    for i, (sid, ability) in enumerate(MOTION_SKILLS):
        row = c.execute(
            "select id, casting_time, channeling_time, effect_delay, projectile_id, plot_id, target_type_id, ability_level, "
            "target_relation_id from skills where id=?", (sid,)).fetchone()
        start = t
        if sid in GROUND_SKILLS:
            # the key, the cursor on the doll, a click (Meteor Strike; Missile Rain's range starts at 10 m, so its reticle stays
            # red and the aim is dropped), then the cast the server would send
            args += ["--ground-press", str(sid), f"{t:.2f}", "--aim", str(AIM_PIXEL[0]), str(AIM_PIXEL[1]), f"{t + 0.4:.2f}"]
            args += (["--click", f"{t + RETICLE_LEAD - 0.2:.2f}"] if sid == 10664 else ["--ground-cancel", f"{t + RETICLE_LEAD - 0.2:.2f}"])
            start = t + RETICLE_LEAD
        ev, length = skill_events(c, ability, row, 100 + i * 10, start)
        steps += ev
        length += start - t
        if sid not in GROUND_SKILLS:
            # the caster is put back on the spot before the next skill (the moves of the skills would carry it off)
            steps.append(back_on_stage(t + length - 0.35))
        captions.append(dict(skill=sid, name=nm.get(sid, str(sid)), start=t, end=t + length))
        t += length
    env = {"AA_COMBAT_FAKE": ";".join(steps), "AA_COMBAT_FAKE_TARGET": TARGET_ENV}
    return dict(ability=MOTION, name="Motion", weapon=WEAPON_NAME[MOTION], env=env, captions=captions, args=args, hud=False,
                record_from=RECORD_FROM, record_secs=round(t - RECORD_FROM + 1.0, 2), end=t)


# the ground-target reels: the real cast through the ground reticle (`--ground-press`, `--aim-world`, `--click`), the server's own
# answer drives the effects (no fake events but the `tp` that opens the record gate). The camera is behind the caster so the reticle
# in front of it is in view. id -> (skill, ability, metres from the caster along its facing or None = on the doll, camera args)
GROUND_REELS = {12: (13281, 6, 11.5), 13: (10664, 7, None)}
# the camera looks along the caster's facing from above and behind (found by trying: yaw 0, pitch -35), so the ring ahead is in the middle
GROUND_CAM = ["--cam-dist", "8", "--cam-fov", "65", "--cam-yaw", "0", "--cam-pitch", "-35"]


def make_ground_plan(c, nm, rid):
    import math
    sid, ability, metres = GROUND_REELS[rid]
    st = json.loads((WORK / "stage.json").read_text())
    x, y = (float(v) for v in st["pos"].split(","))
    yaw = math.radians(st["yaw"])
    if metres is None:
        ax, ay = st["dummy"][0], st["dummy"][1]
    else:
        ax, ay = x - math.sin(yaw) * metres, y + math.cos(yaw) * metres
    cast, chan, delay = c.execute("select casting_time, channeling_time, effect_delay from skills where id=?", (sid,)).fetchone()
    t = FIRST_SKILL
    aim_for = 3.4  # the ring is drawn from about a second after the press
    end = t + 0.4 + aim_for + (cast or 0) / 1000.0 + (delay or 0) / 1000.0 + min((chan or 0) / 1000.0, CHANNEL_CAP) + 4.0
    args = ["--ground-press", str(sid), f"{t:.2f}", "--aim-world", f"{ax:.2f}", f"{ay:.2f}", f"{t + 0.4:.2f}", "--click", f"{t + 0.4 + aim_for:.2f}"]
    env = {f"AA_DEV_{k}": str(v) for k, v in WEAPONS[ability].items()}
    env["AA_COMBAT_FAKE"] = f"{FIRST_SKILL - 1:.3f}:tp:{int(round(x))}:{int(round(y))}:{(st['yaw'] + 360) % 360}"
    env["AA_CAMERA_NO_CLIP"] = "1"
    where = f"{metres:g} m ahead" if metres else "on the doll"
    caption = dict(skill=sid, name=f"{nm.get(sid, str(sid))} ({where})", start=t, end=end)
    return dict(ability=rid, name="Ground", weapon=WEAPON_NAME[ability], env=env, captions=[caption], args=args, cam=GROUND_CAM, cam_rel=None,
                hud=False, record_from=RECORD_FOR_GROUND, record_secs=round(end - RECORD_FOR_GROUND + 1.0, 2), end=end)


RECORD_FOR_GROUND = RECORD_FROM


def main():
    want = [int(a) for a in sys.argv[1:]] or list(ABILITIES) + [MOTION] + list(GROUND_REELS)
    c = connect()
    nm = names(c)
    out = WORK / "plans"
    out.mkdir(exist_ok=True)
    for a in want:
        plan = make_motion_plan(c, nm) if a == MOTION else make_ground_plan(c, nm, a) if a in GROUND_REELS else make_plan(c, a, nm)
        label = ABILITIES.get(a) or ("motion" if a == MOTION else f"ground_{nm[GROUND_REELS[a][0]].lower().replace(' ', '_')}")
        (out / f"{a:02d}_{label.lower()}.json").write_text(json.dumps(plan, indent=1), encoding="utf-8")
        print(f"{a:2d} {(ABILITIES.get(a) or plan['name']):11s} {len(plan['captions']):2d} skills, {plan['record_secs']:6.1f} s, weapon {plan['weapon']}")


if __name__ == "__main__":
    main()
