#!/usr/bin/env python3
"""Run the skill parity plan in aa-client: write an `--script` file, run the client with `AA_SKILL_TRACE`, record the video.

    python tools/skillparity/run_bevy.py --plan plan.json --exe <aa-client.exe> --account NAME --login HOST:PORT \\
        --password-file P --character C --capture-root <capture-root> --run-id 2026-11-03-a \\
        [--trees 1,2] [--move "15530 15323 131"] [--video --ffmpeg FFMPEG] [--ground-click 960,540] [--dry-run]

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

Video (`--video`): ffmpeg `gdigrab` of the aa-client window (`--window-title`, default `AAEMU 1.2`), started when the script's
`skillparity:record-start` mark appears (the script waits `--video-grace` seconds after it), stopped when the client exits.
The video's wall-clock origin `t0_wall_us` (video.json) is measured, not assumed: ffmpeg runs with
`-use_wallclock_as_timestamps 1 -debug_ts` and the first captured frame's timestamp is read from its log (the OS clock at the moment
gdigrab delivered the frame, in us). The uncertainty is about one frame period plus the GDI grab time (tens of ms); when the
timestamp cannot be read, t0 falls back to the time the recorder started and `video.json` says so (`method: "spawn"`, 700 ms
uncertainty). Needs ffmpeg 5.1 or newer (`-fps_mode cfr`: frame k = t0 + k / fps) and a windowed client (not `--headless`).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import common  # noqa: E402
import plan as planlib  # noqa: E402

DEFAULT_TITLE = "AAEMU 1.2"
GRACE_AFTER_START = 0.3          # seconds of `wait` after selecting a target, before the press
SETTLE = 0.6                     # between skills
RETICLE_S = 1.2                  # a ground skill's reticle shows this long before the click
PRE_PICK_WAIT = 2.0              # after spawning the doll
RECORD_START = "skillparity:record-start"   # the mark the video recorder waits for


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
        lines += [f"gm {c.lstrip('/')}" for c in plan.get("setup", {}).get("gm", ["/level 50", "/ignoreskillcds true"])]
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
        lines += [mark_line(mark_command, RECORD_START), f"wait {video_grace:g}"]
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
                lines += ["target self", "gm clearcombat", "wait 0.8", f"tp {sx} {sy} {sz}", f"wait {stage_settle:g}"]
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


# ---- the window video ---------------------------------------------------------------------------------------------------

def window_exists(title: str) -> bool:
    """Is there a top-level window with exactly this title (Windows only; elsewhere True so nothing waits for it)?"""
    if os.name != "nt":
        return True
    try:
        import ctypes
        return bool(ctypes.windll.user32.FindWindowW(None, title))
    except Exception:
        return True


PTS_RE = re.compile(r"type:video\b.*?\bpkt_pts:(\d+)")
SIZE_RE = re.compile(r"Stream #0:0.*?Video:.*?,\s*(\d{2,5})x(\d{2,5})")


def ffmpeg_args(ffmpeg: str, title: str, out: Path, fps: int, grab: str = "gdigrab") -> list[str]:
    """ffmpeg grabbing one window; wall-clock timestamps so the first frame's time can be read; constant frame rate output."""
    if grab == "gdigrab":
        source = ["-f", "gdigrab", "-framerate", str(fps), "-use_wallclock_as_timestamps", "1", "-draw_mouse", "0", "-i", f"title={title}"]
    else:
        raise ValueError(f"unknown grab method {grab!r}")
    return [ffmpeg, "-hide_banner", "-y", "-debug_ts", *source, "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
            "-fps_mode", "cfr", "-r", str(fps), "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p", str(out)]


class WindowRecorder:
    """ffmpeg process capturing the client's window; measures when its first frame was taken (see the module doc)."""

    def __init__(self, ffmpeg: str, title: str, out: Path, fps: int = 30):
        self.ffmpeg, self.title, self.out, self.fps = ffmpeg, title, out, fps
        self.process: subprocess.Popen | None = None
        self.spawned_us = 0
        self.first_us: int | None = None
        self.size: tuple[int, int] | None = None
        self.tail: list[str] = []
        self.first_event = threading.Event()

    def start(self) -> None:
        self.spawned_us = common.now_us()
        self.process = subprocess.Popen(ffmpeg_args(self.ffmpeg, self.title, self.out, self.fps), stdin=subprocess.PIPE,
                                        stderr=subprocess.PIPE, stdout=subprocess.DEVNULL, text=True, errors="replace")
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self) -> None:
        assert self.process and self.process.stderr
        for line in self.process.stderr:             # -debug_ts is chatty: keep only the first frame time and a short tail
            if self.first_us is None:
                m = PTS_RE.search(line)
                if m and int(m.group(1)) > 10 ** 15:  # epoch microseconds (use_wallclock_as_timestamps)
                    self.first_us = int(m.group(1))
                    self.first_event.set()
            if self.size is None:
                m = SIZE_RE.search(line)
                if m:
                    self.size = (int(m.group(1)), int(m.group(2)))
            if "pkt_pts" not in line and "next_dts" not in line:
                self.tail = (self.tail + [line.rstrip()])[-12:]

    def wait_first_frame(self, timeout: float = 15.0) -> bool:
        return self.first_event.wait(timeout)

    def alive(self) -> bool:
        return self.process is not None and self.process.poll() is None

    def stop(self) -> dict:
        info: dict[str, Any] = {"fps": self.fps, "title": self.title}
        proc = self.process
        if proc is None:
            return info
        try:
            if proc.poll() is None and proc.stdin:
                proc.stdin.write("q\n")
                proc.stdin.flush()
            proc.wait(timeout=30)
        except Exception:
            proc.kill()
        if self.first_us is not None:
            info.update(t0_wall_us=self.first_us, method="gdigrab -use_wallclock_as_timestamps -debug_ts: pkt_pts of the first frame",
                        t0_uncertainty_ms=round(1000.0 / self.fps + 20.0, 1))
        else:
            info.update(t0_wall_us=self.spawned_us, method="spawn", t0_uncertainty_ms=700,
                        warning="the first frame time could not be read from ffmpeg: t0 is the time ffmpeg was started")
        if self.size:
            info["width"], info["height"] = self.size
        info["ffmpeg_exit"] = proc.returncode
        if proc.returncode not in (0, None):
            info["ffmpeg_tail"] = self.tail
        return info


def trace_has_mark(trace: Path, text: str) -> bool:
    """Does the client's trace file hold a mark row whose label is `text` (read from the end of the file it is being written to)?"""
    try:
        with trace.open("rb") as handle:
            size = trace.stat().st_size
            handle.seek(max(0, size - 65536))
            tail = handle.read().decode("utf-8", "replace")
    except OSError:
        return False
    for line in tail.splitlines():
        if text in line and '"mark"' in line:
            return True
    return False


# ---- running the client -------------------------------------------------------------------------------------------------

def client_command(args: argparse.Namespace, script: Path) -> list[str]:
    cmd = [str(args.exe), "--account", args.account, "--login", args.login]
    if args.password_file:
        cmd += ["--password-file", str(args.password_file)]
    if args.character:
        cmd += ["--character", args.character]
    cmd += ["--script", str(script), "--require-wall-clock"]
    if args.headless:
        cmd += ["--headless"]
    return cmd + list(args.client_arg or [])


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
    ap.add_argument("--video", action="store_true")
    ap.add_argument("--ffmpeg", default=None)
    ap.add_argument("--fps", type=int, default=30)
    ap.add_argument("--window-title", default=DEFAULT_TITLE)
    ap.add_argument("--video-grace", type=float, default=3.0, help="seconds the script waits after record-start (ffmpeg's start-up)")
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
    cmd = client_command(args, script)
    recorder: WindowRecorder | None = None
    if args.video:
        ffmpeg = args.ffmpeg or shutil.which("ffmpeg")
        if not ffmpeg:
            print("run_bevy: --video needs ffmpeg: pass --ffmpeg or put it on PATH", file=sys.stderr)
            return 2
        recorder = WindowRecorder(ffmpeg, args.window_title, out / "video.mp4", args.fps)

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
            if recorder:
                begun = False
                while proc.poll() is None and time.monotonic() < deadline and not begun:
                    ready = trace_has_mark(trace, RECORD_START) if args.mark_command == "trace" else window_exists(args.window_title)
                    if ready and window_exists(args.window_title):
                        recorder.start()
                        begun = True
                        if not recorder.wait_first_frame():
                            print("run_bevy: warning: no first frame time from ffmpeg (see video.json)")
                    else:
                        time.sleep(0.05)
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
            if recorder:
                video_info = recorder.stop()
                (out / "video.json").write_text(json.dumps(video_info, indent=1) + "\n", encoding="utf-8")
    rows, bad = common.read_jsonl(trace)
    meta.update(started_wall_us=started, ended_wall_us=common.now_us(), exit_code=code, command=[c for c in cmd],
                trace_rows=len(rows), trace_kinds=common.kinds_in(rows), video=video_info,
                env={"AA_SKILL_TRACE": str(trace)}, ok=code == 0 and bool(rows))
    if not rows:
        meta["warning"] = "the client wrote no trace rows: does this build support AA_SKILL_TRACE?"
    (out / "meta.json").write_text(json.dumps(meta, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"run_bevy: client exit {code}, {len(rows)} trace rows -> {out}")
    return 0 if meta["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
