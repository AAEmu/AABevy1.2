"""Record a few skills of one ability tree (the reel's stage, weapons and fake events) with a given exe.

    python one.py <ability 1..10> <skill id> [<skill id> ...] [--exe PATH] [--tag dev] [--mp4]

Writes shots/reel/<tag>/one_<ability>_sheet.png (a contact sheet) and keeps the frames in data/extract/reel/frames/<tag>/one/.
"""
import json
import sys
from pathlib import Path

import plan as P
import run_reel as R


def main():
    args = sys.argv[1:]
    exe = Path(R.opt(args, "--exe") or R.ROOT / "target/agents/anim-audit/aa-client.exe")
    tag = R.opt(args, "--tag") or "dev"
    nums = [int(a) for a in (args[: args.index(next(x for x in args if x.startswith('--')))] if any(x.startswith('--') for x in args) else args)]
    ability, skills = nums[0], nums[1:]
    c = P.connect()
    nm = P.names(c)
    t = P.FIRST_SKILL
    steps, captions = [], []
    for i, sid in enumerate(skills):
        row = c.execute("select id, casting_time, channeling_time, effect_delay, projectile_id, plot_id, target_type_id, ability_level, "
                        "target_relation_id from skills where id=?", (sid,)).fetchone()
        ev, length = P.skill_events(c, ability, row, 100 + i * 10, t)
        steps += ev
        steps.append(P.back_on_stage(t + length - 0.35))
        captions.append(dict(skill=sid, name=nm.get(sid, str(sid)), start=t, end=t + length))
        t += length
    env = {f"AA_DEV_{k}": str(v) for k, v in P.WEAPONS[ability].items()}
    env["AA_COMBAT_FAKE"] = ";".join(steps)
    env["AA_COMBAT_FAKE_TARGET"] = P.TARGET_ENV
    plan = dict(ability=ability, name=P.ABILITIES[ability], weapon=P.WEAPON_NAME[ability], env=env, captions=captions, file="one",
                record_from=P.RECORD_FROM, record_secs=round(t - P.RECORD_FROM + 1.0, 2), end=t)
    fps = int(R.opt(args, "--fps") or 10)  # disk is tight: a few frames a second, small
    d, origin, wall = R.record(tag, exe, plan, fps, R.opt(args, "--size") or "960x540", [])
    R.sheet(d, R.SHOTS / tag / f"one_{ability}_sheet.png", plan, origin, fps)
    (R.FRAMES / tag / "one.json").write_text(json.dumps(dict(origin=origin, captions=captions)))
    if "--mp4" in args:
        R.encode(d, R.SHOTS / tag / f"one_{ability}.mp4", plan, origin, fps, tag)
    print("frames:", d, "origin", origin, [(c["name"], round(c["start"] - origin, 2)) for c in captions])


if __name__ == "__main__":
    main()
