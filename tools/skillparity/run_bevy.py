#!/usr/bin/env python3
"""Run the skill parity plan in aa-client: write an `--script` file, run the client with `AA_SKILL_TRACE`, record the video.

    python tools/skillparity/run_bevy.py --plan plan.json --exe <aa-client.exe> --account NAME --login HOST:PORT \\
        --password-file P --character C --capture-root <capture-root> --run-id 2026-11-03-a \\
        [--trees 1,2] [--move "15530 15323 131"] [--video --ffmpeg FFMPEG --fps 30] [--ground-click 960,540] [--dry-run]

Writes `<capture-root>/<run-id>/bevy/{plan.txt, client.log, trace.jsonl (the client's), meta.json, video.mp4, video.json}` and
runs `aa-client.exe --account A --login L --password-file P --character C --script plan.txt --require-wall-clock`
with `AA_SKILL_TRACE=<...>/bevy/trace.jsonl` (and `AA_PAK`, `AA_DB` from your environment or `--pak` / `--db`).
`--dry-run` only writes the script (read it, run it yourself). Never edit a script that other captures used: it is part of the run.

The script (docs/aa-client-script.md): `gm`, `tp`, `target`, `key slotN`, `wait`, `lua`, `echo`, `stopcast`, `uiclick`, and the marks
`trace TEXT` (a script command that writes a `mark` row with the text as its label; being added by the trace work: with
`--mark-command echo` the marks are `echo` lines instead, which end up in client.log only, and `diff.py` falls back to the
press rows). Each skill is: select the doll (or yourself), `trace skill:<id>:start`, press the slot's key, wait the plan's time
(stop a channel past its cap), `trace skill:<id>:end`. The action bar of each batch is set with the Lua API the original UI
uses (`X2Action:SetActionSpell`), assumed to exist in aa-client's Lua like the rest of the X2 API.

Video (`--video`): aa-client films itself, `aa-client --video <run>/bevy/video.mp4 --video-fps FPS --ffmpeg FFMPEG --video-gate`
(docs/aa-dev.md, "Video"). The frames are screenshots taken inside the client, so a locked desktop does not matter, and they sit
on a fixed grid of the wall clock: frame k of the file shows wall time `t0_wall_us + k / fps`, `t0_wall_us` being the Unix time
the client read at the first frame of the capture (a measurement, not an assumption; what a frame shows is a few app frames older
than its tick, `image_age_ms_mean` in the sidecar says how much). The capture starts at the script's `video-start` mark (`trace
video-start`, `echo video-start` with `--mark-command echo`; the script then waits `--video-grace` seconds for ffmpeg to start) and
ends when the client exits. The client writes `video.mp4.json` and `video.json` (`t0_wall_us`, `fps`, `width`, `height`, `frames`,
`repeated`, `skipped`, `dropped`, `ffmpeg_exit`, ...); `video.json` goes into meta.json, and a video with no frames, an ffmpeg
that failed or a capture that stopped early makes the run not ok. Needs a windowed client (not `--headless`) built with `--video`.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import common  # noqa: E402
import plan as planlib  # noqa: E402

GRACE_AFTER_START = 0.3          # seconds of `wait` after selecting a target, before the press
SETTLE = 0.6                     # between skills
RETICLE_S = 1.2                  # a ground skill's reticle shows this long before the click
PRE_PICK_WAIT = 2.0              # after spawning the doll
VIDEO_START = "video-start"      # the script mark that starts the client's `--video-gate` capture (aa_dev::GATE_MARK)


# ---- the script ---------------------------------------------------------------------------------------------------------

def lua_set_bar(batch: dict, slots_per_page: int) -> str:
    """One `lua` line that clears the bar and puts the batch on it (no `#`: that starts a comment in the script)."""
    # aa-client's own bar path (the drag-and-drop's `HUD.drag_action_set`, which sends CSUpdateActionSlot): aa-client implements
    # X2Action:SetActionSpell only as a stub, as only the original UI's debug bar calls it. No pcall: a failure fails the script.
    sets = " ".join(f"HUD.drag_action_set({int(s)}, 'skill', {int(i)})" for s, i in sorted(batch["slots"].items(), key=lambda kv: int(kv[0])))
    return f"lua for s = 1, {slots_per_page} do HUD.drag_action_set(s) end {sets}"


def mark_line(mark_command: str, text: str) -> str:
    """`trace TEXT` (a mark row with the text as label) or its `echo TEXT` fallback."""
    return f"{mark_command} {text}"


def fmt_s(ms: float) -> str:
    return f"{ms / 1000.0:.3f}".rstrip("0").rstrip(".") or "0"


def build_script(plan: dict, batches: list[dict], *, run_id: str, setup: bool = True, move: str | None = None,
                 stage: str | None = None, stage_settle: float = 1.5,
                 spawn_doll: bool = True, mark_command: str = "trace", ground_click: tuple[int, int] | None = None,
                 camera: list[str] | None = None, video_grace: float = 0.0, extra_gm: list[str] | None = None) -> tuple[str, list[dict]]:
    """The aa-client script text for the batches, and the skills left out (ground skills without a click position)."""
    dummy = plan.get("dummy", {})
    template = dummy.get("template", planlib.DUMMY_TEMPLATE)
    slots = plan.get("slots_per_page", 10)
    lines = [f"# skillparity script for run {run_id}: generated by tools/skillparity/run_bevy.py, do not edit",
             f"echo skillparity run {run_id}"]
    skipped: list[dict] = []
    if setup:
        lines += [f"gm {c.lstrip('/')}" for c in plan.get("setup", {}).get("gm", ["/level 50", "/ignoreskillcds false"])]
    for c in extra_gm or []:
        lines.append(f"gm {c.lstrip('/')}")
    if move:
        x, y, z = re.split(r"[ ,]+", move.strip())[:3]
        lines.append(f"tp {x} {y} {z}")
        lines.append("wait 2")
    if spawn_doll:
        lines += [f"gm despawn npc {template} 30", "wait 0.8", f"gm spawn npc {template}", f"wait {PRE_PICK_WAIT:g}"]
    lines += list(camera or [])
    if video_grace > 0:
        lines += [mark_line(mark_command, VIDEO_START), f"wait {video_grace:g}"]
    for batch in batches:
        lines += ["", f"# batch {batch['batch']}: {batch['tree_name']} ({batch['weapon'].get('name')})",
                  lua_set_bar(batch, slots), "wait 0.5"]
        for skill in batch["skills"]:
            sid, n = skill["id"], skill["slot"]
            if skill.get("target") == "ground" and ground_click is None:
                skipped.append({"id": sid, "reason": "ground skill needs --ground-click"})
                lines.append(f"# skill {sid} {skill.get('name')}: ground skill, left out (pass --ground-click X,Y)")
                continue
            lines.append(f"# skill {sid} {skill.get('name')} (slot {n}, {skill['target']})")
            if stage:                                   # every skill from the same spot, as run_original.py --stage does
                sx, sy, sz = re.split(r"[ ,]+", stage.strip())[:3]
                # as run_original.py: targeting oneself ends the auto-attack on the doll, then no combat state, then the spot
                lines += ["target self", "gm clearcombat", "gm rcd", "wait 0.8", f"tp {sx} {sy} {sz}", f"wait {stage_settle:g}"]
                lines += [c for c in camera or [] if not c.split()[0] == "zoom"]   # a teleport can turn the camera; zoom is relative
            lines.append("target self" if skill.get("select") == "self" else f"target template:{template}")
            lines.append(f"wait {GRACE_AFTER_START:g}")
            lines.append(mark_line(mark_command, f"skill:{sid}:start"))
            lines.append(f"key slot{n}")
            waited = 0
            if skill.get("target") == "ground":
                lines += [f"wait {RETICLE_S:g}", f"uiclick {ground_click[0]} {ground_click[1]}"]
                waited = int(RETICLE_S * 1000)
            stop_after = skill.get("stop_after_ms")
            if stop_after:
                lines += [f"wait {fmt_s(max(stop_after - waited, 0))}", "stopcast", f"wait {fmt_s(max(skill['wait_ms'] - stop_after, 0))}"]
            else:
                lines.append(f"wait {fmt_s(max(skill['wait_ms'] - waited, 0))}")
            lines.append(mark_line(mark_command, f"skill:{sid}:end"))
            lines.append(f"wait {SETTLE:g}")
    lines += ["", "echo skillparity done"]
    return "\n".join(lines) + "\n", skipped


# ---- the video ----------------------------------------------------------------------------------------------------------

def video_client_args(video: Path, fps: int, ffmpeg: str) -> list[str]:
    """The aa-client flags that make it film itself from the script's `video-start` mark (aa-dev's `--video`)."""
    return ["--video", str(video), "--video-fps", str(fps), "--ffmpeg", ffmpeg, "--video-gate"]


def read_video_info(out: Path) -> dict:
    """The sidecar aa-client wrote next to the video (`video.mp4.json`; `video.json` is the same), or why there is none."""
    path = out / "video.mp4.json"
    try:
        info = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        return {"error": f"no usable video sidecar {path.name}: {error}"}
    return info if isinstance(info, dict) else {"error": f"{path.name} is not a JSON object"}


def video_problem(info: dict) -> str | None:
    """Why this video cannot be used for an aligned comparison (None when it can)."""
    if info.get("error"):
        return str(info["error"])
    if info.get("stopped"):
        return f"the capture stopped early: {info['stopped']}"
    if not info.get("frames"):
        return "the video has no frames"
    if info.get("ffmpeg_exit") != 0:
        return f"ffmpeg exited with {info.get('ffmpeg_exit')}"
    if not info.get("t0_wall_us"):
        return "the video has no start time"
    return None


# ---- running the client -------------------------------------------------------------------------------------------------

def client_command(args: argparse.Namespace, script: Path, video: list[str] | None = None) -> list[str]:
    cmd = [str(args.exe), "--account", args.account, "--login", args.login]
    if args.password_file:
        cmd += ["--password-file", str(args.password_file)]
    if args.character:
        cmd += ["--character", args.character]
    cmd += ["--script", str(script), "--require-wall-clock"]
    if args.headless:
        cmd += ["--headless"]
    return cmd + list(video or []) + list(args.client_arg or [])


def parse_pair(text: str | None) -> tuple[int, int] | None:
    if not text:
        return None
    x, y = (int(v) for v in re.split(r"[ ,]+", text.strip())[:2])
    return x, y


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--plan", type=Path, required=True)
    ap.add_argument("--exe", type=Path, default=Path(os.environ["AA_CLIENT_EXE"]) if os.environ.get("AA_CLIENT_EXE") else None,
                    help="aa-client.exe (default: AA_CLIENT_EXE)")
    ap.add_argument("--account", default=os.environ.get("AA_ACCOUNT", ""))
    ap.add_argument("--login", default=os.environ.get("AA_LOGIN", "127.0.0.1:1237"))
    ap.add_argument("--password-file", type=Path, default=Path(os.environ["AA_PASSWORD_FILE"]) if os.environ.get("AA_PASSWORD_FILE") else None)
    ap.add_argument("--character", default=os.environ.get("AA_CHARACTER", ""))
    ap.add_argument("--pak", default=os.environ.get("AA_PAK"), help="AA_PAK for the client (default: the variable)")
    ap.add_argument("--db", default=os.environ.get("AA_DB"), help="AA_DB for the client (default: the variable)")
    ap.add_argument("--trees", default="", help="only these ability trees (0 = the default skills)")
    ap.add_argument("--skills", default="", help="only these skill ids")
    ap.add_argument("--capture-root", type=Path, default=None, help="default: AA_PARITY_ROOT")
    ap.add_argument("--run-id", default=None)
    ap.add_argument("--force", action="store_true", help="overwrite an existing bevy/ directory of the run")
    ap.add_argument("--no-setup", action="store_true", help="no /level and /ignoreskillcds lines")
    ap.add_argument("--gm", action="append", default=[], help="an extra GM command line (repeatable), e.g. '/ignoreskillcds true'")
    ap.add_argument("--move", default=None, help='"x y z": a `tp` line before the doll is spawned')
    ap.add_argument("--stage", default=None, help='"x y z": a `tp` line before every skill (the same spot as run_original.py --stage)')
    ap.add_argument("--stage-settle", type=float, default=2.0, help="seconds to wait after the --stage teleport")
    ap.add_argument("--no-spawn-doll", action="store_true", help="the doll is already there")
    ap.add_argument("--camera-line", action="append", default=[], help="a script line (orbit/zoom/pitch) before the skills, repeatable")
    ap.add_argument("--mark-command", choices=("trace", "echo"), default="trace")
    ap.add_argument("--ground-click", default=None, help="X,Y pixel for the click of a ground skill (else those are left out)")
    ap.add_argument("--headless", action="store_true", help="no window (no video possible)")
    ap.add_argument("--client-arg", action="append", default=[], help="an extra aa-client argument (repeatable)")
    ap.add_argument("--timeout", type=float, default=0.0, help="seconds before the client is stopped (default: the plan's length + 10 min)")
    ap.add_argument("--video", action="store_true", help="aa-client films itself (video.mp4, video.json) from the script's video-start mark")
    ap.add_argument("--ffmpeg", default=None, help="ffmpeg for the client's video (default: FFMPEG, then PATH)")
    ap.add_argument("--fps", type=int, default=30, help="frame rate of the video")
    ap.add_argument("--video-grace", type=float, default=1.0, help="seconds the script waits after the video-start mark (ffmpeg's start-up)")
    ap.add_argument("--dry-run", action="store_true", help="write the script and meta, run nothing")
    args = ap.parse_args(argv)

    plan = planlib.load_plan(args.plan)
    batches = planlib.select_batches(plan, planlib.parse_int_list(args.trees) or None, planlib.parse_int_list(args.skills) or None)
    if not batches:
        print("run_bevy: the plan has nothing for --trees / --skills", file=sys.stderr)
        return 2
    root = args.capture_root or common.env_path("AA_PARITY_ROOT")
    if root is None:
        print("run_bevy: --capture-root (or AA_PARITY_ROOT) is required: captures never go into the repository", file=sys.stderr)
        return 2
    run_id = args.run_id or datetime.now().strftime("%Y%m%d-%H%M%S")
    out = Path(root) / run_id / "bevy"
    if (out / "trace.jsonl").exists() and not args.force and not args.dry_run:
        print(f"run_bevy: {out / 'trace.jsonl'} exists: pick another --run-id or --force", file=sys.stderr)
        return 2
    if args.video and args.headless:
        print("run_bevy: --video needs a window: drop --headless", file=sys.stderr)
        return 2
    out.mkdir(parents=True, exist_ok=True)

    script_text, skipped = build_script(
        plan, batches, run_id=run_id, setup=not args.no_setup, move=args.move, stage=args.stage, stage_settle=args.stage_settle,
        spawn_doll=not args.no_spawn_doll,
        mark_command=args.mark_command, ground_click=parse_pair(args.ground_click), camera=args.camera_line,
        video_grace=args.video_grace if args.video else 0.0, extra_gm=args.gm)
    script = out / "plan.txt"
    script.write_text(script_text, encoding="utf-8", newline="\n")
    trace = out / "trace.jsonl"
    meta: dict[str, Any] = {
        "schema": common.SCHEMA, "tool": "skillparity/run_bevy", "run_id": run_id, "src": "bevy", "script": script.name,
        "plan": {"schema": plan.get("schema"), "batches": batches, "skills": [s for b in batches for s in b["skills"]],
                 "dummy": plan.get("dummy"), "slots_per_page": plan.get("slots_per_page")},
        "skipped": skipped, "mark_command": args.mark_command, "character": args.character, "ok": None,
    }
    total_s = sum(s["wait_ms"] for b in batches for s in b["skills"]) / 1000.0
    print(f"run_bevy: script {script} ({sum(len(b['skills']) for b in batches)} skills, about {total_s / 60:.1f} min)")
    if args.dry_run:
        (out / "meta.json").write_text(json.dumps(meta, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        return 0
    if args.exe is None or not Path(args.exe).is_file():
        print("run_bevy: pass --exe <aa-client.exe> (or AA_CLIENT_EXE)", file=sys.stderr)
        return 2
    if not args.account:
        print("run_bevy: --account is required", file=sys.stderr)
        return 2
    if args.force:
        trace.unlink(missing_ok=True)

    env = dict(os.environ)
    env["AA_SKILL_TRACE"] = str(trace)
    for name, value in (("AA_PAK", args.pak), ("AA_DB", args.db)):
        if value:
            env[name] = str(value)
    video_args: list[str] = []
    if args.video:
        ffmpeg = args.ffmpeg or os.environ.get("FFMPEG") or shutil.which("ffmpeg")
        if not ffmpeg:
            print("run_bevy: --video needs ffmpeg: pass --ffmpeg, set FFMPEG or put it on PATH", file=sys.stderr)
            return 2
        video_args = video_client_args(out / "video.mp4", args.fps, ffmpeg)
        for stale in ("video.mp4", "video.mp4.json", "video.json"):     # a --force run must not read the last run's sidecar
            (out / stale).unlink(missing_ok=True)
    cmd = client_command(args, script, video_args)

    started = common.now_us()
    log_path = out / "client.log"
    timeout = args.timeout or (total_s + 600.0)
    code: int | None = None
    video_info: dict | None = None
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, env=env)
        meta["pid"] = proc.pid
        deadline = time.monotonic() + timeout
        try:
            while proc.poll() is None and time.monotonic() < deadline:
                time.sleep(0.25)
            if proc.poll() is None:
                print(f"run_bevy: timeout after {timeout:.0f} s: stopping the client this tool started (pid {proc.pid})")
                proc.terminate()
                try:
                    proc.wait(timeout=15)
                except subprocess.TimeoutExpired:
                    proc.kill()
        finally:
            code = proc.poll()
    rows, bad = common.read_jsonl(trace)
    video_ok = True
    if args.video:
        video_info = read_video_info(out)           # written by the client at its exit (docs/aa-dev.md, "Video")
        problem = video_problem(video_info)
        if problem:
            video_ok = False
            meta["video_problem"] = problem
            print(f"run_bevy: warning: video: {problem}")
    meta.update(started_wall_us=started, ended_wall_us=common.now_us(), exit_code=code, command=[c for c in cmd],
                trace_rows=len(rows), trace_kinds=common.kinds_in(rows), video=video_info,
                env={"AA_SKILL_TRACE": str(trace)}, ok=code == 0 and bool(rows) and video_ok)
    if not rows:
        meta["warning"] = "the client wrote no trace rows: does this build support AA_SKILL_TRACE?"
    (out / "meta.json").write_text(json.dumps(meta, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"run_bevy: client exit {code}, {len(rows)} trace rows -> {out}")
    return 0 if meta["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
