"""Record the skill reel: one video per ability tree, captions, contact sheets, and the all-skills video.

    python run_reel.py baseline [abilities...]     # exe = target/agents/reel/baseline/aa-client.exe
    python run_reel.py after    [abilities...]     # exe = target/agents/reel/after/aa-client.exe (copy of the HEAD build)
    python run_reel.py compare  [abilities...]     # side by side before | after (needs both)
    python run_reel.py setup                       # spawn the training dummy once (server GM command, via the walkthrough example)

Options: --exe PATH, --keep-frames, --fps N (30), --size WxH (1280x720), --from-frames (re-encode what is on disk).
Output: shots/reel/<tag>/<NN>_<ability>.mp4, <NN>_<ability>_sheet.png, all_skills.mp4. Frames (PNG, about 1.5 MB each)
live in data/extract/reel/frames/<tag>/<ability>/ only until encoded.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent          # tools/reel (scripts, tracked)
ROOT = HERE.parents[1]
WORK = ROOT / "data/extract/reel"                 # plans, frames, logs, venv, font (git-ignored)
FFMPEG = next((WORK / "venv/Lib/site-packages/imageio_ffmpeg/binaries").glob("ffmpeg-*.exe"), None)
FONT = WORK / "arialbd.ttf"
SHOTS = ROOT / "shots/reel"
FRAMES = WORK / "frames"
ACCOUNT, CHARACTER = "bevyreel9", "Reelcasterx"  # access_level 100 in lane_bevy_game.accounts (the GM command /spawn needs it)
# the target (npc 13013, a straw training doll) is spawned by `setup` 3 m in front of the character; its obj id and the caster position (4 m south of it, facing it; yaw: degrees
# counter-clockwise from north) are in stage.json (a server restart loses the doll: run `setup` again).
def stage_args():
    st = json.loads((WORK / "stage.json").read_text())
    return ["--pos", st["pos"], "--yaw", str(st["yaw"])]
CAM = ["--cam-dist", "5", "--cam-fov", "60"]
MIN_FREE_KB = 6 * 1024 * 1024
STALL_SECS = 150


class Stalled(Exception):
    pass


def free_kb():
    out = subprocess.run(["powershell", "-NoProfile", "-c", "(Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory"],
                         capture_output=True, text=True).stdout.strip()
    return int(out or 0)


def wait_ram():
    while free_kb() < MIN_FREE_KB:
        print(f"free RAM {free_kb() / 1048576:.1f} GB < 6 GB, waiting 30 s", flush=True)
        time.sleep(30)


def commit_of(exe):
    f = Path(exe).with_name("commit.txt")
    return f.read_text().strip()[:7] if f.exists() else "?"


def plans(which):
    out = []
    for f in sorted((WORK / "plans").glob("*.json")):
        p = json.loads(f.read_text(encoding="utf-8"))
        if p["ability"] in which or (not which and p["ability"] != 11):  # the motion reel (11) only when asked for
            p["file"] = f.stem
            out.append(p)
    return out


# ---- recording ------------------------------------------------------------------------------------------------------

def record(tag, exe, plan, fps, size, extra_args):
    d = FRAMES / tag / plan["file"]
    shutil.rmtree(d, ignore_errors=True)
    d.parent.mkdir(parents=True, exist_ok=True)
    cmd = [str(exe), "--enter-world", "--user", ACCOUNT, "--character", CHARACTER, "--hour", "12", "--size", size,
           *([] if plan.get("hud") else ["--no-hud"]), "--mute", *CAM, *stage_args(), "--record", str(d), "--record-fps", str(fps), "--record-gate",
           "--record-from", str(plan["record_from"]), "--record-secs", str(plan["record_secs"]), *plan.get("args", []), *extra_args]
    env = dict(os.environ)
    env.update(plan["env"])
    env["AA_COMBAT_FAKE_TARGET"] = str(json.loads((WORK / "stage.json").read_text())["dummy_obj"])
    env["AA_COMBAT_CAM_REL"] = "-90"
    env["AA_COMBAT_LOG"] = "1"
    wait_ram()
    print(f"[{tag}] {plan['name']}: {len(plan['captions'])} skills, {plan['record_secs']} s of video", flush=True)
    t0 = time.time()
    log = WORK / "logs" / tag
    log.mkdir(parents=True, exist_ok=True)
    with open(log / f"{plan['file']}.log", "w", encoding="utf-8", errors="replace") as lf:
        p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=lf, stderr=subprocess.STDOUT)
        # watchdog: kill our own client when neither the log nor the frame directory grew for STALL_SECS
        last, last_t = (-1, -1), time.time()
        while p.poll() is None:
            time.sleep(5)
            lf.flush()
            now = (os.path.getsize(lf.name), len(os.listdir(d)) if d.exists() else 0)
            if now != last:
                last, last_t = now, time.time()
            elif time.time() - last_t > STALL_SECS:
                p.kill()  # our own child only
                p.wait()
                raise Stalled(f"{plan['name']}: no new frames or log output for {STALL_SECS} s, client killed")
            if time.time() - t0 > 45 * 60:
                p.kill()
                raise Stalled(f"{plan['name']}: over 45 minutes, client killed")
    wall = time.time() - t0
    text = (log / f"{plan['file']}.log").read_text(encoding="utf-8", errors="replace")
    text = re.sub(r"\x1b\[[0-9;]*m", "", text)
    m = re.search(r"record start at gate\+([0-9.]+) s", text)
    if not m or not list(d.glob("f_*.png")):
        raise SystemExit(f"no recording for {plan['name']}; see {log / (plan['file'] + '.log')}")
    if "not in view" in text:
        print("  WARNING: the target dummy was not in view (events aimed at the player); run `setup`", flush=True)
    fpsl = re.findall(r"aa-dev: fps=([0-9.]+)", text)
    frames = len(list(d.glob("f_*.png")))
    print(f"  {frames} frames in {wall:.0f} s wall ({frames / wall:.1f} frames/s rendered), origin gate+{m.group(1)}"
          f"{', log fps ' + fpsl[-1] if fpsl else ''}", flush=True)
    return d, float(m.group(1)), wall


# ---- encoding -------------------------------------------------------------------------------------------------------

def drawtext(textfile, start, end, y, size, color="white", x="(w-text_w)/2"):
    return (f"drawtext=fontfile={FONT.name}:textfile={textfile}:x={x}:y={y}:fontsize={size}:fontcolor={color}:"
            f"box=1:boxcolor=black@0.55:boxborderw=10:enable='between(t,{start:.3f},{end:.3f})'")


def encode(d, out, plan, origin, fps, label):
    """frames -> captioned mp4 (H.264, yuv420p, crf 20)."""
    work = d.parent / (d.name + "_txt")
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir()
    # the window is sometimes maximised by the desktop (2560x1351): always cut to 16:9 and 1280x720
    filters = ["scale=1280:720:force_original_aspect_ratio=increase,crop=1280:720"]
    n = len(list(d.glob("f_*.png")))
    for i, c in enumerate(plan["captions"]):
        f = work / f"c{i}.txt"
        f.write_text(f"{plan['name']}: {c['name']}", encoding="utf-8")
        filters.append(drawtext(f"{work.name}/{f.name}", c["start"] - 0.1 - origin, c["end"] - 0.1 - origin, "h-70", 34))
    f = work / "tag.txt"
    f.write_text(label, encoding="utf-8")
    filters.append(drawtext(f"{work.name}/{f.name}", 0, n / fps + 1, 12, 20, "yellow", "w-text_w-12"))
    script = work / "filter.txt"
    script.write_text(",\n".join(filters), encoding="utf-8")
    shutil.copy(FONT, d.parent / FONT.name) if not (d.parent / FONT.name).exists() else None
    out.parent.mkdir(parents=True, exist_ok=True)
    cmd = [str(FFMPEG), "-y", "-hide_banner", "-loglevel", "error", "-framerate", str(fps), "-i", f"{d.name}/f_%06d.png",
           "-filter_script:v", f"{work.name}/{script.name}", "-c:v", "libx264", "-crf", "20", "-preset", "medium",
           "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(out)]
    subprocess.run(cmd, cwd=d.parent, check=True)
    shutil.rmtree(work, ignore_errors=True)


def sheet(d, out, plan, origin, fps):
    """4x4 grid of key frames: every skill at 0.8 s after its start, then a second frame of the longest ones."""
    from PIL import Image, ImageDraw, ImageFont, ImageOps
    n = len(list(d.glob("f_*.png")))
    picks = []
    caps = plan["captions"]
    for c in caps:
        picks.append((c["start"] + 0.8, c))
    for c in sorted(caps, key=lambda c: c["start"] - c["end"])[: 16 - len(picks)]:
        picks.append((c["start"] + 0.55 * (c["end"] - c["start"]), c))
    picks = sorted(picks, key=lambda p: p[0])[:16]
    tw, th = 480, 270
    img = Image.new("RGB", (tw * 4, th * 4), "black")
    font = ImageFont.truetype(str(FONT), 16)
    for i, (t, c) in enumerate(picks):
        k = min(n, max(1, round((t - origin) * fps)))
        fr = Image.open(d / f"f_{k:06d}.png").convert("RGB"); fr = ImageOps.fit(fr, (tw, th), Image.LANCZOS)
        dr = ImageDraw.Draw(fr)
        dr.rectangle([0, th - 24, tw, th], fill=(0, 0, 0))
        dr.text((6, th - 21), f"{c['name']}  (t={t - origin:.1f}s)", fill="white", font=font)
        img.paste(fr, ((i % 4) * tw, (i // 4) * th))
    out.parent.mkdir(parents=True, exist_ok=True)
    img.save(out)


def concat(videos, out):
    lst = out.with_suffix(".txt")
    lst.write_text("".join(f"file '{v.as_posix()}'\n" for v in videos), encoding="utf-8")
    subprocess.run([str(FFMPEG), "-y", "-hide_banner", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(lst),
                    "-c", "copy", str(out)], check=True)
    lst.unlink()


# ---- commands -------------------------------------------------------------------------------------------------------

def cmd_record(tag, args):
    exe = Path(opt(args, "--exe") or ROOT / f"target/agents/reel/{tag}/aa-client.exe")
    fps = int(opt(args, "--fps") or 30)
    size = opt(args, "--size") or "1280x720"
    which = [int(a) for a in args if a.isdigit()]
    label = f"{tag} {commit_of(exe)}"
    outs = []
    stats = []
    for plan in plans(which):
        out = SHOTS / tag / f"{plan['file']}.mp4"
        if "--from-frames" in args:
            d = FRAMES / tag / plan["file"]
            origin = float(json.loads((FRAMES / tag / (plan["file"] + ".json")).read_text())["origin"])
            wall = 0
        else:
            for attempt in (1, 2):  # a stalled client is killed by the watchdog and the ability is recorded once more
                try:
                    d, origin, wall = record(tag, exe, plan, fps, size, [])
                    break
                except Stalled as e:
                    print(f"  STALL ({attempt}/2): {e}", flush=True)
                    if attempt == 2:
                        raise SystemExit("stalled twice")
            (FRAMES / tag / (plan["file"] + ".json")).write_text(json.dumps(dict(origin=origin, wall=wall)))
        sheet(d, SHOTS / tag / f"{plan['file']}_sheet.png", plan, origin, fps)
        encode(d, out, plan, origin, fps, label)
        if "--keep-frames" not in args:
            shutil.rmtree(d)
        outs.append(out)
        stats.append((plan["name"], plan["record_secs"], wall))
        print(f"  -> {out}", flush=True)
    if len(plans(which)) == len(plans([])):
        concat(outs, SHOTS / tag / "all_skills.mp4")
        print(f"  -> {SHOTS / tag / 'all_skills.mp4'}")
    for n, secs, wall in stats:
        print(f"{n:11s} video {secs:6.1f} s, wall {wall:6.0f} s")


def cmd_compare(args):
    which = [int(a) for a in args if a.isdigit()]
    outs = []
    for plan in plans(which):
        a, b = SHOTS / "baseline" / f"{plan['file']}.mp4", SHOTS / "after" / f"{plan['file']}.mp4"
        out = SHOTS / "compare" / f"{plan['file']}_before_after.mp4"
        out.parent.mkdir(parents=True, exist_ok=True)
        lab = lambda s, x: f"drawtext=fontfile={FONT.name}:text='{s}':x={x}:y=12:fontsize=26:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=8"
        subprocess.run([str(FFMPEG), "-y", "-hide_banner", "-loglevel", "error", "-i", str(a), "-i", str(b), "-filter_complex",
                        f"[0:v]{lab('BEFORE', '12')}[l];[1:v]{lab('AFTER', '12')}[r];[l][r]hstack=inputs=2[v]", "-map", "[v]",
                        "-c:v", "libx264", "-crf", "20", "-pix_fmt", "yuv420p", str(out)], check=True, cwd=WORK)
        outs.append(out)
        print(f"  -> {out}")
    if len(outs) == len(plans([])):
        concat(outs, SHOTS / "compare" / "all_skills_before_after.mp4")


SPAWN_AT = (15540.0, 15330.0)  # the caster is parked here facing east (-90 degrees): the doll appears 3 m ahead on the open dirt path


def cmd_setup():
    """Park the character at SPAWN_AT facing east, spawn the target (npc 13013, a straw training doll with a real actor model; the
    dummy 7512 is a prefab and is not drawn) 3 m ahead, find it, and write stage.json (obj id, caster position 4 m west of it)."""
    exe = ROOT / "target/agents/reel/after/aa-client.exe"
    if not exe.exists():
        exe = ROOT / "target/agents/reel/baseline/aa-client.exe"
    subprocess.run([str(exe), "--enter-world", "--user", ACCOUNT, "--character", CHARACTER, "--size", "640x360", "--no-hud", "--mute",
                    "--pos", f"{SPAWN_AT[0]},{SPAWN_AT[1]}", "--yaw", "-90", "--exit-after", "22"], cwd=ROOT, capture_output=True)
    wt = ROOT / "target/debug/examples/walkthrough.exe"
    base = [str(wt), "--user", ACCOUNT, "--character", CHARACTER, "--watch-secs", "2"]
    subprocess.run(base + ["--cmd", "/spawn npc 13013"], check=True, capture_output=True)
    out = subprocess.run(base, check=True, capture_output=True, text=True).stdout
    found = re.findall(r"template_id: 13013.*? at \(([0-9.]+), ([0-9.]+), ([0-9.]+)\) obj (\d+)", out)
    found = [f for f in found if abs(float(f[0]) - SPAWN_AT[0] - 3) < 1.5 and abs(float(f[1]) - SPAWN_AT[1]) < 1.5]
    if not found:
        raise SystemExit("the target did not spawn 3 m east of the caster (GM rights? access_level 100 in lane_bevy_game.accounts)")
    x, y, z, obj = max(found, key=lambda f: int(f[3]))  # the newest doll is ours
    st = dict(dummy_obj=int(obj), dummy=[float(x), float(y), float(z)], pos=f"{float(x) - 4.0},{float(y)}", yaw=-90)
    (WORK / "stage.json").write_text(json.dumps(st))
    print("stage:", st)


def opt(args, name):
    return args[args.index(name) + 1] if name in args else None


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    what, rest = sys.argv[1], sys.argv[2:]
    if not FFMPEG:
        raise SystemExit("no ffmpeg: python -m venv data/extract/reel/venv; data/extract/reel/venv/Scripts/pip install imageio-ffmpeg pillow")
    if what == "setup":
        cmd_setup()
    elif what == "all":
        concat([SHOTS / rest[0] / f"{p['file']}.mp4" for p in plans([])], SHOTS / rest[0] / "all_skills.mp4")
    elif what == "compare":
        cmd_compare(rest)
    else:
        cmd_record(what, rest)
