"""Skill movement videos: charges, pulls, knockbacks, leaps and blinks (shots/skill-motion/*.mp4).

    python tools/reel/run_motion.py setup                    # spawn the straw doll next to the character (server GM command), write stage
    data/extract/reel/venv/Scripts/python.exe tools/reel/run_motion.py record [scene ...] [--exe PATH]
    ... run_motion.py list

Same machinery as run_reel.py (fake server events, aa-dev `--record`, ffmpeg), with its own account `bevymotion` (character
Motioncaster, access level 100 in lane_bevy_game.accounts for the GM `/spawn`) and its own stage in data/extract/motion/stage.json.
Every scene starts the caster `dist` metres south of the doll, facing north. Besides the fake server events (AA_COMBAT_FAKE) a scene
may use AA_MOTION_FAKE (aa-client skill_motion.rs: a push, a blink, a blink behind the doll).
"""
import json
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import run_reel as rr  # noqa: E402

ROOT = rr.ROOT
WORK = ROOT / "data/extract/motion"
rr.WORK = WORK
rr.FRAMES = WORK / "frames"
rr.SHOTS = ROOT / "shots/skill-motion"
rr.ACCOUNT, rr.CHARACTER = "bevymotion", "Motioncaster"
EXE = ROOT / "target/agents/motion/aa-client.exe"

GREATSWORD, SWORD = "5660", "5569"

# name -> scene: dist (m from the caster to the doll), weapons, fake events, motion fake, captions (fake seconds), seconds of video
SCENES = {
    "charge": dict(
        title="Charge (Battlerage)", dist=10, cam=8, main=GREATSWORD, off="0",
        fake="15.5:plot:110:5100:11918;15.95:plot:110:5402:11918;16.0:hit:110:11918:121;16.4:plotend:110",
        captions=[("the caster runs to the doll", 15.4, 18.5)], secs=5.0),
    "lasso": dict(
        title="Lasso (Battlerage)", dist=12, cam=10, main=GREATSWORD, off="0",
        fake="15.5:plot:140:2656:12039:1000;16.5:plot:140:2657:12039;16.95:plot:140:2659:12039;17.2:plot:140:2658:12039;17.6:plotend:140",
        captions=[("the doll is pulled to the caster", 15.4, 19.5)], secs=6.0),
    "pull": dict(
        title="Vicious Implosion (Auramancy)", dist=9, cam=9, main="6102", off="0",
        fake="15.5:plot:130:4625:10710;16.0:plot:130:4628:10710;16.05:plot:130:4627:10710;16.1:hit:130:10710:121;16.5:plotend:130",
        captions=[("the doll is pulled in", 15.4, 19.0)], secs=5.5),
    "knockback": dict(
        title="Knockback", dist=4, cam=7, main=None, off=None,
        fake="15.5:fire:150:16210:700:0;16.3:hit:150:16210:150;16.4:end:150",
        motion="19.0:push:3000:35;22.0:push:6000:60",
        captions=[("Charged Bolt knocks the doll back 1.2 m", 15.4, 18.5), ("The player is knocked back 3 m", 18.9, 21.9), ("... and 6 m at 60 degrees", 21.9, 25.0)], secs=11.0),
    "overwhelm": dict(
        title="Overwhelm (Shadowplay)", dist=10, cam=8, main=SWORD, off=SWORD,
        fake="15.5:plot:110:7062:10648;15.95:plot:110:5108:10648;16.0:hit:110:10648:121;16.4:plotend:110",
        captions=[("a leap to the doll", 15.4, 18.5)], secs=5.0),
    "dropback": dict(
        title="Drop Back (Shadowplay)", dist=4, cam=8, main=SWORD, off=SWORD,
        fake="15.5:fire@self:120:12049:0:0;16.3:end@self:120",
        captions=[("a back step of 10 m", 15.4, 18.5)], secs=5.0),
    "shadowstep": dict(
        title="Shadow Step (Shadowplay)", dist=7, cam=8, main=SWORD, off=SWORD,
        fake="15.5:fire:200:12075:0:0;15.55:hit:200:12075:150;15.8:end:200",
        motion="15.52:behind:1300",
        captions=[("behind the doll", 15.4, 18.5)], secs=5.0),
    "teleport": dict(
        title="Teleportation (Auramancy)", dist=18, cam=12, main="6102", off="0",
        fake="15.5:fire@self:140:10152:0:0;15.8:end@self:140",
        motion="15.52:blink:15000",
        captions=[("15 m forward", 15.4, 18.5)], secs=5.0),
    # the same skills cast for real: the lane server answers the UseSkill with the plot steps / SCBlinkUnit (`{doll}` is the doll's
    # object id; `skilltest --user bevymotion --skills ...` teaches the skills first)
    "charge_live": dict(
        title="Charge (Battlerage), server", dist=10, cam=8, main=None, off=None,
        fake="15.5:use:11918:{doll}",
        captions=[("the lane server runs Charge", 15.4, 18.5)], secs=5.0),
    "lasso_live": dict(
        title="Lasso (Battlerage), server", dist=12, cam=10, main=None, off=None,
        fake="15.5:use:12039:{doll}",
        captions=[("the lane server runs Lasso: the doll is pulled by the server", 15.4, 19.5)], secs=6.5),
    "dropback_live": dict(
        title="Drop Back (Shadowplay), server", dist=4, cam=8, main=None, off=None,
        fake="15.5:use:12049:0",
        captions=[("the lane server runs Drop Back", 15.4, 18.5)], secs=5.0),
    "teleport_live": dict(
        title="Teleportation (Auramancy), server", dist=18, cam=12, main=None, off=None,
        fake="15.5:use:10152:0",
        captions=[("SCBlinkUnit: 15 m forward", 15.4, 18.5)], secs=5.0),
    "knock_live": dict(
        title="Charged Bolt (Archery), server", dist=6, cam=7, main=None, off=None,
        fake="15.5:use:16210:{doll}",
        captions=[("the server moves the doll 1.2 m: the client plays the flight", 15.4, 19.0)], secs=5.5),
    "shadowstep_live": dict(
        title="Shadow Step (Shadowplay), server", dist=7, cam=8, main=None, off=None,
        fake="15.5:use:12075:{doll}",
        captions=[("SCBlinkUnit: behind the doll", 15.4, 18.5)], secs=5.0),
}


HOME = "15530.0,15325.0"  # the open dirt patch near the spawn pond (the baseline reel's stage)


def cmd_home(exe):
    """Puts the character on the dirt patch on the server (a short client session that stands there), so every doll spawns there."""
    import time
    cmd = [str(exe), "--enter-world", "--user", rr.ACCOUNT, "--character", rr.CHARACTER, "--hour", "12", "--size", "640x360", "--no-hud",
           "--mute", "--pos", HOME, "--yaw", "0", "--exit-after", "6"]
    rr.wait_ram()
    subprocess.run(cmd, cwd=ROOT, capture_output=True, timeout=300)
    time.sleep(14)


def cmd_setup():
    wt = ROOT / "target/debug/examples/walkthrough.exe"
    base = [str(wt), "--user", rr.ACCOUNT, "--character", rr.CHARACTER, "--watch-secs", "2"]
    subprocess.run(base + ["--cmd", "/spawn npc 13013"], check=True, capture_output=True)
    out = subprocess.run(base, check=True, capture_output=True, text=True).stdout
    found = re.findall(r"template_id: 13013.*? at \(([0-9.]+), ([0-9.]+), ([0-9.]+)\) obj (\d+)", out)
    if not found:
        raise SystemExit("the target did not spawn (GM rights? access_level 100 in lane_bevy_game.accounts)")
    x, y, z, obj = found[-1]
    st = dict(dummy_obj=int(obj), dummy=[float(x), float(y), float(z)], yaw=0)
    WORK.mkdir(parents=True, exist_ok=True)
    (WORK / "stage.json").write_text(json.dumps(st))
    print("stage:", st)


def plan_of(name, s):
    st = json.loads((WORK / "stage.json").read_text())
    x, y, _ = st["dummy"]
    start = 15.4
    env = {"AA_COMBAT_FAKE": s["fake"].replace("{doll}", str(st["dummy_obj"])), "AA_COMBAT_FAKE_TARGET": str(st["dummy_obj"])}
    if s["main"] is not None:
        env["AA_DEV_MAINHAND"], env["AA_DEV_OFFHAND"] = s["main"], s["off"]
    if s.get("motion"):
        env["AA_MOTION_FAKE"] = s["motion"]
    env["AA_SKILL_MOTION_LOG"] = "1"
    plan = dict(name=s["title"], file=name, env=env, record_from=start - 1.0, record_secs=s["secs"] + 1.0,
                captions=[dict(skill=0, name=c[0], start=c[1], end=c[2]) for c in s["captions"]])
    pos = f"{x},{y - s['dist']}"
    return plan, pos


def watchdog(log, stop):
    """Kills our own client when its log stops growing for 150 s (it sometimes hangs under GPU contention on this machine)."""
    import threading
    import time

    def run():
        last, since = -1, time.time()
        while not stop.is_set():
            time.sleep(5)
            size = log.stat().st_size if log.exists() else 0
            if size != last:
                last, since = size, time.time()
            elif time.time() - since > 150:
                out = subprocess.run(["powershell", "-NoProfile", "-c",
                                      "Get-CimInstance Win32_Process -Filter \"name='aa-client.exe'\" | Where-Object { $_.ExecutablePath -like '*agents*motion**' } | ForEach-Object { $_.ProcessId }"],
                                     capture_output=True, text=True).stdout.split()
                for pid in out:
                    print(f"watchdog: the client {pid} hung, killing it", flush=True)
                    subprocess.run(["taskkill", "/F", "/PID", pid], capture_output=True)
                since = time.time()

    threading.Thread(target=run, daemon=True).start()


def record(names, exe, fresh=True):
    exe = Path(exe)
    for name in names:
        if fresh:
            # a fresh doll per scene (a live Lasso moves it; it is gone after a server restart); the server keeps the setup session 10 s
            import time
            cmd_home(exe)
            cmd_setup()
            time.sleep(14)
        s = SCENES[name]
        plan, pos = plan_of(name, s)
        rr.CAM[:] = ["--cam-dist", str(s["cam"]), "--cam-fov", "60"]
        rr.stage_args = lambda pos=pos: ["--pos", pos, "--yaw", "0"]
        import threading
        stop = threading.Event()
        watchdog(WORK / "logs" / "motion" / f"{name}.log", stop)
        try:
            d, origin, wall = rr.record("motion", exe, plan, 30, "1280x720", [])
        finally:
            stop.set()
        out = rr.SHOTS / f"{name}.mp4"
        # the captions carry the whole text already
        plan2 = dict(plan, name=s["title"].split(" (")[0] if name != "knockback" else "Knockback")
        rr.encode(d, out, plan2, origin, 30, f"skill motion {rr.commit_of(exe)}")
        print("wrote", out)


if __name__ == "__main__":
    args = sys.argv[1:]
    if not args or args[0] == "list":
        print("\n".join(SCENES))
    elif args[0] == "setup":
        cmd_setup()
    elif args[0] == "record":
        rest = args[1:]
        fresh = "--no-setup" not in rest
        rest = [a for a in rest if a != "--no-setup"]
        exe = rr.opt(rest, "--exe") if "--exe" in rest else EXE
        if "--exe" in rest:
            i = rest.index("--exe")
            rest = rest[:i] + rest[i + 2:]
        record(rest or list(SCENES), exe, fresh)
