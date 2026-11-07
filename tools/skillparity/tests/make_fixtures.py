"""Builds the small hand-made capture in `fixtures/run1/` (two clients, three skills, plus one skill only the original has).

    python tools/skillparity/tests/make_fixtures.py        # rewrites fixtures/run1/

Everything is invented data (no game files). The story, per skill, original vs aa-client (times after the press):

* 100 "Test Slash"   same clips, effects and sounds within a few ms                      -> PASS
* 200 "Test Bolt"    a staff cast: the cast clip starts 66 ms later, cast bar 65 ms later  -> timing differences
* 300 "Test Dash"    clip speed 1.0 vs 1.25, a clip and an effect missing in aa-client,
                     an extra sound in aa-client, a 4 m dash that is 3 m in aa-client      -> missing / extra / value
* 400 "Test Only"    recorded by the original only                                          -> NO-DATA
* 500 "Test Dust"    the same effect (`fx.skill.dust`) missing as in 300: one root cause    -> a repeated difference

The original's anim rows carry an `inst` but no `role` (the role rule must find the player); aa-client's are labelled.
The original has no `net` rows of its own: the server rows (`server.jsonl`) serve both runs, told apart by time.
aa-client has no `pos` rows for the target in skill 300's segment... it has none at all for the target, so that is NO-DATA.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
T0 = 1_793_000_000_000_000          # a Unix time in microseconds, 2026
ORIG_BASE = T0
BEVY_BASE = T0 + 600_000_000        # the aa-client run is ten minutes later, same server
SKILLS = (100, 200, 300, 400, 500)
SEGMENT_LENGTH = 3.0

PLAYER_ORIG, TARGET_ORIG = "0x12a0", "0x4f10"
PLAYER_BEVY, TARGET_BEVY = "e1v7", "e1v9"


def ms(base: float, press_s: float, after_ms: float) -> int:
    return int(round(base + press_s * 1e6 + after_ms * 1000))


class Run:
    def __init__(self, src: str, base: int, frame_ms: float):
        self.src, self.base, self.frame_us = src, base, frame_ms * 1000.0
        self.rows: list[dict] = []

    def add(self, kind: str, wall: int, **fields) -> None:
        row = {"src": self.src, "wall_us": wall, "frame": int((wall - self.base) / self.frame_us), "kind": kind}
        row.update(fields)
        self.rows.append(row)

    def skill(self, sid: int, press_s: float):
        """Marks and the press of one skill; returns a function t(after_ms) -> wall_us on the press clock."""
        p = self.base + int(press_s * 1e6)
        self.add("mark", p - 50_000, label=f"skill:{sid}:start", skill_id=sid)
        self.add("press", p, key=str(1 + SKILLS.index(sid)), slot=1 + SKILLS.index(sid))
        self.add("mark", p - 50_000 + int(SEGMENT_LENGTH * 1e6), label=f"skill:{sid}:end", skill_id=sid)
        return lambda after: p + int(round(after * 1000))


def clip(run: Run, t, after, inst, role, name, layer=0, speed=1.0, blend=0.1, loop=False):
    extra = {"role": role} if role else {}
    run.add("anim", t(after), inst=inst, clip=name, file=f"animations/{name}.caf",
            layer=layer, speed=speed, blend=blend, loop=loop, **extra)


def build_orig() -> list[dict]:
    r = Run("orig", ORIG_BASE, 33.3)
    for i in range(12):
        r.add("frame", ORIG_BASE + 500_000 + i * 33_300, dt_ms=33.3)
    # the player stands still: one position row before the first press (rows are change events)
    r.add("pos", ORIG_BASE + 500_000, unit="player", x=100.0, y=200.0, z=10.0, yaw=90.0)
    r.add("pos", ORIG_BASE + 500_000, unit="target", x=100.0, y=204.0, z=10.0, yaw=270.0)

    t = r.skill(100, 1.05)
    clip(r, t, 10, PLAYER_ORIG, None, "sword_idle_01", speed=1.0, blend=0.2, loop=True)     # locomotion: not the role anchor
    clip(r, t, 33, PLAYER_ORIG, None, "sword_attack_01")
    clip(r, t, 150, TARGET_ORIG, None, "scarecrow_hit_01")
    clip(r, t, 900, PLAYER_ORIG, None, "sword_idle_01", blend=0.2, loop=True)
    r.add("fx", t(120), name="fx.combat.slash_hit", phase="spawn", pos=[100.0, 204.0, 11.0])
    r.add("fx", t(400), name="fx.combat.slash_hit", phase="spawn", pos=[100.0, 204.0, 11.0])
    r.add("sound", t(115), name="combat:sword_slash", phase="create")
    r.add("sound", t(120), name="combat:sword_slash", phase="play")
    r.add("cooldown", t(40), slot=1, remain_ms=1000, total_ms=1000, gcd=True)
    r.add("cooldown", t(1040), slot=1, remain_ms=0, total_ms=1000, gcd=True)

    t = r.skill(200, 4.55)
    clip(r, t, 33, PLAYER_ORIG, None, "staff_cast_spell_01", blend=0.15)
    r.add("castbar", t(40), unit="player", state="start", name="Test Bolt", total_ms=1500, elapsed_ms=0, channel=False)
    r.add("fx", t(50), name="fx.magic.charge", phase="spawn")
    r.add("sound", t(50), name="magic:cast_start", phase="play")
    r.add("cooldown", t(60), slot=2, remain_ms=15000, total_ms=15000, gcd=False)
    clip(r, t, 1540, PLAYER_ORIG, None, "staff_cast_fire_01", blend=0.05)
    r.add("castbar", t(1560), unit="player", state="stop", name="Test Bolt", total_ms=1500, elapsed_ms=1500, channel=False)
    r.add("fx", t(1550), name="fx.magic.bolt", phase="spawn")
    clip(r, t, 1700, TARGET_ORIG, None, "scarecrow_hit_01")

    t = r.skill(300, 8.05)
    clip(r, t, 20, PLAYER_ORIG, None, "sword_dash_01", speed=1.0, blend=0.1)
    clip(r, t, 400, PLAYER_ORIG, None, "sword_dash_land_01", speed=1.0, blend=0.1)
    r.add("fx", t(20), name="fx.skill.dash_trail", phase="spawn")
    r.add("fx", t(420), name="fx.skill.dust", phase="spawn")
    r.add("sound", t(25), name="skill:dash", phase="play")
    for i, after in enumerate((80, 160, 240, 320, 380)):          # 4 m north in 300 ms
        r.add("pos", t(after), unit="player", x=100.0, y=round(200.0 + 4.0 * (i + 1) / 5, 3), z=10.0, yaw=90.0)

    t = r.skill(400, 11.55)
    clip(r, t, 30, PLAYER_ORIG, None, "sword_attack_02")

    t = r.skill(500, 15.05)
    clip(r, t, 20, PLAYER_ORIG, None, "sword_dash_01")
    r.add("fx", t(420), name="fx.skill.dust", phase="spawn")
    return r.rows


def build_bevy() -> list[dict]:
    r = Run("bevy", BEVY_BASE, 16.7)
    for i in range(12):
        r.add("frame", BEVY_BASE + 500_000 + i * 16_700, dt_ms=16.7)
    r.add("pos", BEVY_BASE + 500_000, unit="player", x=100.0, y=200.0, z=10.0, yaw=90.0)

    t = r.skill(100, 1.05)
    clip(r, t, 12, PLAYER_BEVY, "player", "sword_idle_01", blend=0.2, loop=True)
    clip(r, t, 40, PLAYER_BEVY, "player", "sword_attack_01")
    clip(r, t, 155, TARGET_BEVY, "target", "scarecrow_hit_01")
    clip(r, t, 905, PLAYER_BEVY, "player", "sword_idle_01", blend=0.2, loop=True)
    r.add("fx", t(125), name="fx.combat.slash_hit", phase="spawn", pos=[100.0, 204.0, 11.0])
    r.add("fx", t(405), name="fx.combat.slash_hit", phase="spawn", pos=[100.0, 204.0, 11.0])
    r.add("sound", t(122), name="combat:sword_slash", phase="play")
    r.add("cooldown", t(45), slot=1, remain_ms=1000, total_ms=1000, gcd=True)
    r.add("cooldown", t(1045), slot=1, remain_ms=0, total_ms=1000, gcd=True)

    t = r.skill(200, 4.55)
    clip(r, t, 99, PLAYER_BEVY, "player", "staff_cast_spell_01", blend=0.15)
    r.add("castbar", t(105), unit="player", state="start", name="Test Bolt", total_ms=1500, elapsed_ms=0, channel=False)
    r.add("fx", t(112), name="fx.magic.charge", phase="spawn")
    r.add("sound", t(110), name="magic:cast_start", phase="play")
    r.add("cooldown", t(120), slot=2, remain_ms=15000, total_ms=15000, gcd=False)
    clip(r, t, 1600, PLAYER_BEVY, "player", "staff_cast_fire_01", blend=0.05)
    r.add("castbar", t(1625), unit="player", state="stop", name="Test Bolt", total_ms=1500, elapsed_ms=1500, channel=False)
    r.add("fx", t(1610), name="fx.magic.bolt", phase="spawn")
    clip(r, t, 1760, TARGET_BEVY, "target", "scarecrow_hit_01")

    t = r.skill(300, 8.05)
    clip(r, t, 25, PLAYER_BEVY, "player", "sword_dash_01", speed=1.25, blend=0.1)
    r.add("fx", t(25), name="fx.skill.dash_trail", phase="spawn")
    r.add("sound", t(30), name="skill:dash", phase="play")
    r.add("sound", t(900), name="ui:error_beep", phase="play")
    for i, after in enumerate((90, 170, 250, 330, 390)):          # 3 m north
        r.add("pos", t(after), unit="player", x=100.0, y=round(200.0 + 3.0 * (i + 1) / 5, 3), z=10.0, yaw=90.0)

    t = r.skill(500, 15.05)
    clip(r, t, 22, PLAYER_BEVY, "player", "sword_dash_01")
    return r.rows


def build_server() -> list[dict]:
    """Net rows of both runs: the same answers, a little after each press (CSStartSkill 30 ms, answers right behind it)."""
    rows: list[dict] = []
    for base, jitter in ((ORIG_BASE, 0), (BEVY_BASE, 4)):
        for sid, press_s in ((100, 1.05), (200, 4.55), (300, 8.05), (400, 11.55), (500, 15.05)):
            if base == BEVY_BASE and sid == 400:
                continue
            p = base + int(press_s * 1e6)
            tl = sid // 100

            def net(after, d, op, **kw):
                rows.append({"src": "server", "kind": "net", "wall_us": p + int((after + jitter) * 1000), "dir": d, "op": op,
                             "skill_id": sid, **kw})

            net(30, "out", "CSStartSkill", obj=26891)
            if sid == 200:
                net(35, "in", "SCSkillStarted", tl=tl, obj=26891)
                net(1535, "in", "SCSkillFired", tl=tl, obj=26891)
            else:
                net(35, "in", "SCSkillFired", tl=tl, obj=26891)
            if sid == 100:
                net(36, "in", "SCPlotEvent", tl=tl)
                net(300, "in", "SCPlotEvent", tl=tl)
            rows.append({"src": "server", "kind": "net", "wall_us": p + int((2000 + jitter) * 1000), "dir": "in",
                         "op": "SCSkillEnded", "tl": tl})
    rows.sort(key=lambda r: r["wall_us"])
    return rows


def build_plan() -> dict:
    skills = []
    names = {100: "Test Slash", 200: "Test Bolt", 300: "Test Dash", 400: "Test Only", 500: "Test Dust"}
    for i, (sid, name, cast) in enumerate((s, names[s], 1500 if s == 200 else 0) for s in SKILLS):
        skills.append({"id": sid, "name": name, "slot": i + 1, "key": str(i + 1), "batch": 1, "target": "unit", "cast_ms": cast,
                       "wait_ms": 3000, "cooldown_ms": 15000 if sid == 200 else 0})
    return {"schema": "skillparity/plan/v1", "skills": skills,
            "batches": [{"batch": 1, "tree": 1, "slots": {str(s["slot"]): s["id"] for s in skills}, "skills": skills}]}


def write(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="\n") as handle:
        for row in rows:
            handle.write(json.dumps(row, separators=(",", ":")) + "\n")


def build_all() -> dict[str, object]:
    return {"orig/trace.jsonl": build_orig(), "bevy/trace.jsonl": build_bevy(), "server.jsonl": build_server(), "plan.json": build_plan()}


def main() -> int:
    out = HERE / "fixtures" / "run1"
    for name, content in build_all().items():
        if name.endswith(".json"):
            (out / name).parent.mkdir(parents=True, exist_ok=True)
            (out / name).write_text(json.dumps(content, indent=1) + "\n", encoding="utf-8", newline="\n")
        else:
            write(out / name, content)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
