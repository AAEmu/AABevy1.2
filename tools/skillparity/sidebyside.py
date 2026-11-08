#!/usr/bin/env python3
"""Aligned side-by-side videos, one per skill (original left, aa-client right), plus one `all.mp4`.

    python tools/skillparity/sidebyside.py <capture-root>/<run-id> [--ffmpeg PATH] [--skills 18132,11918] [--height 540]

Uses `orig/video.mp4` + `orig/video.json` and `bevy/video.mp4` + `bevy/video.json` (`{"t0_wall_us": .., "fps": 30}`: frame k
shows wall time t0 + k / fps) and the two traces' skill segments (`diff.find_segments`): for each skill both sides are cut from
0.5 s before their own press to the end of the longer segment (at most `--max-seconds` after the press), so the two presses fall
on the same output time. Both are scaled to the same height, stacked left / right, captioned with the skill, the client names
and a running counter of milliseconds from the press (ffmpeg `drawtext`; without a usable font the videos are made without text),
and written to `<run>/sidebyside/<skill>.mp4` and `<run>/sidebyside/all.mp4` (all cuts, in the order of the plan or the original
trace). Pure ffmpeg through `subprocess`: `--ffmpeg PATH` or `ffmpeg` on PATH or the `FFMPEG` variable.

A cut whose press lies before the start of a video is padded with black; a press before the video started is skipped (said in
`sidebyside.json`, next to the videos, with every cut's numbers).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
import diff  # noqa: E402

FONT_CANDIDATES = [
    "{WINDIR}/Fonts/arial.ttf", "{WINDIR}/Fonts/segoeui.ttf", "{WINDIR}/Fonts/consola.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    "/System/Library/Fonts/Supplemental/Arial.ttf", "/Library/Fonts/Arial.ttf",
]
DEFAULT_HEIGHT = 540


# ---- finding things -----------------------------------------------------------------------------------------------

def find_ffmpeg(given: str | None = None) -> str | None:
    """`--ffmpeg`, else the FFMPEG variable, else `ffmpeg` on PATH; None when there is none."""
    for candidate in (given, os.environ.get("FFMPEG")):
        if candidate and Path(candidate).is_file():
            return str(candidate)
    return shutil.which("ffmpeg")


def find_font(given: str | None = None) -> str | None:
    for candidate in [given, os.environ.get("SKILLPARITY_FONT")] + [c.replace("{WINDIR}", os.environ.get("WINDIR", "C:/Windows")) for c in FONT_CANDIDATES]:
        if candidate and Path(candidate).is_file():
            return str(candidate)
    return None


def load_video_info(side_dir: Path) -> dict:
    """`video.json` of a side; the video file is `video.mp4` next to it."""
    info_path, video = side_dir / "video.json", side_dir / "video.mp4"
    if not video.is_file():
        raise FileNotFoundError(f"missing {video}")
    if not info_path.is_file():
        raise FileNotFoundError(f"missing {info_path}: the video's wall-clock origin is needed to align it")
    info = json.loads(info_path.read_text(encoding="utf-8"))
    if "t0_wall_us" not in info:
        raise ValueError(f"{info_path} has no t0_wall_us")
    info.setdefault("fps", 30)
    info["path"] = str(video)
    return info


# ---- the cuts -----------------------------------------------------------------------------------------------------

def cut_for(video: dict, press_us: int, pre_s: float, length_s: float) -> dict | None:
    """Where to cut one side: input start `ss`, black padding `pad` before it, or None when the press precedes the video."""
    t0 = float(video["t0_wall_us"])
    press_t = (press_us - t0) / 1e6                  # seconds into the video file
    start = press_t - pre_s
    if press_t < 0:
        return None
    pad = max(0.0, -start)
    return {"ss": round(max(start, 0.0), 3), "pad": round(pad, 3), "press_in_video": round(press_t, 3), "length": round(length_s - pad, 3)}


def plan_cuts(orig: diff.Trace, bevy: diff.Trace, orig_video: dict, bevy_video: dict, plan_skills: dict, pre_s: float,
              max_s: float, only: list | None = None) -> list[dict]:
    """One cut description per skill that both traces have with a press (warnings for the others)."""
    seg_o = {s.key: s for s in diff.find_segments(orig, plan_skills)}
    seg_b = {s.key: s for s in diff.find_segments(bevy, plan_skills)}
    order = [(sid, 0) for sid in plan_skills if (sid, 0) in seg_o or (sid, 0) in seg_b]       # the plan's order first
    for k in sorted(seg_o, key=lambda k: seg_o[k].start_us) + sorted(seg_b, key=lambda k: seg_b[k].start_us):
        if k not in order:
            order.append(k)
    if only:
        wanted = {common.as_skill_id(x) for x in only}
        order = [k for k in order if k[0] in wanted]
    cuts = []
    for key in order:
        so, sb = seg_o.get(key), seg_b.get(key)
        cut: dict[str, Any] = {"skill_id": key[0], "occurrence": key[1], "name": (plan_skills.get(key[0]) or {}).get("name")}
        if so is None or sb is None:
            cut["skipped"] = "no segment in " + ("the original" if so is None else "aa-client")
        elif so.press_us is None or sb.press_us is None:
            cut["skipped"] = "no press row in " + ("the original" if so.press_us is None else "aa-client")
        else:
            after = min(max_s, max((so.end_us - so.press_us), (sb.end_us - sb.press_us)) / 1e6)
            length = pre_s + after
            left, right = cut_for(orig_video, so.press_us, pre_s, length), cut_for(bevy_video, sb.press_us, pre_s, length)
            if left is None or right is None:
                cut["skipped"] = "the press is before the start of the " + ("original" if left is None else "aa-client") + " video"
            else:
                cut.update(length=round(length, 3), pre=pre_s, left=left, right=right,
                           press_us={"orig": so.press_us, "bevy": sb.press_us})
        cuts.append(cut)
    return cuts



# ---- ffmpeg -------------------------------------------------------------------------------------------------------

def safe_text(text: Any) -> str:
    """Text for drawtext: no characters that need quoting inside a filtergraph."""
    return re.sub(r"[^A-Za-z0-9 _\-./()#+]", " ", str(text)).strip()


def ffmpeg_font_path(font: str) -> str:
    """A font path as a filter option value: forward slashes, `:` escaped (`C\\:/Windows/Fonts/arial.ttf`)."""
    return font.replace("\\", "/").replace(":", "\\:")


def build_filter(cut: dict, title: str, font: str | None, height: int, fps: int, text: bool = True) -> str:
    """The filter_complex string for one skill: scale both to `height`, hstack, optional captions and counter."""
    ff = f"fontfile='{ffmpeg_font_path(font)}'" if font else ""
    style = "fontcolor=white:fontsize=h/24:box=1:boxcolor=black@0.6:boxborderw=6"
    pre = cut["pre"]

    def side(index: int, label: str, name: str, spec: dict) -> str:
        chain = f"[{index}:v]fps={fps},scale=-2:{height},setsar=1"
        if spec["pad"] > 0:
            chain += f",tpad=start_duration={spec['pad']}:start_mode=add:color=black"
        chain += ",setpts=PTS-STARTPTS"
        if text and font:
            # the client's name top left, the running counter top right of its own half (the halves can differ in width);
            # `t` is the output time of the cut, the press is at `pre`
            chain += (f",drawtext={ff}:text='{name}':x=12:y=10:{style}"
                      f",drawtext={ff}:text='%{{eif\\:(t-{pre})*1000\\:d}} ms':x=w-text_w-12:y=10:{style}")
        return f"{chain}[{label}]"

    graph = [side(0, "l", "1.2 client", cut["left"]), side(1, "r", "aa-client", cut["right"])]
    if text and font:
        graph.append(f"[l][r]hstack=inputs=2,drawtext={ff}:text='{safe_text(title)}':x=(w-text_w)/2:y=h-text_h-14:{style}[o]")
    else:
        graph.append("[l][r]hstack=inputs=2[o]")
    return ";".join(graph)


def build_command(ffmpeg: str, orig_video: dict, bevy_video: dict, cut: dict, out: Path, title: str, font: str | None,
                  height: int, fps: int, crf: int, text: bool = True) -> list[str]:
    return [ffmpeg, "-hide_banner", "-loglevel", "error", "-y",
            "-ss", str(cut["left"]["ss"]), "-t", str(cut["left"]["length"]), "-i", orig_video["path"],
            "-ss", str(cut["right"]["ss"]), "-t", str(cut["right"]["length"]), "-i", bevy_video["path"],
            "-filter_complex", build_filter(cut, title, font, height, fps, text),
            "-map", "[o]", "-an", "-t", str(cut["length"]), "-c:v", "libx264", "-preset", "veryfast", "-crf", str(crf),
            "-pix_fmt", "yuv420p", str(out)]


def concat_command(ffmpeg: str, list_file: Path, out: Path) -> list[str]:
    return [ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i", str(list_file), "-c", "copy", str(out)]


def concat_list_text(files: list[Path]) -> str:
    return "".join("file '" + f.resolve().as_posix().replace("'", "'\\''") + "'\n" for f in files)


def slug(text: Any) -> str:
    return re.sub(r"[^A-Za-z0-9]+", "_", str(text)).strip("_").lower() or "skill"


def run_cut(cmd: list[str], plain_cmd: list[str]) -> tuple[bool, str, bool]:
    """Run an ffmpeg command; a failure that names drawtext or a font is retried with the plain command.
    Returns (ok, stderr, text_dropped)."""
    proc = subprocess.run(cmd, capture_output=True, text=True, errors="replace")
    if proc.returncode == 0:
        return True, "", False
    err = proc.stderr or ""
    if re.search(r"drawtext|font", err, re.I):
        proc2 = subprocess.run(plain_cmd, capture_output=True, text=True, errors="replace")
        if proc2.returncode == 0:
            return True, err, True
        err = proc2.stderr or err
    return False, err, False


# ---- command line -------------------------------------------------------------------------------------------------

def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", type=Path, help="capture directory <capture-root>/<run-id>")
    ap.add_argument("--ffmpeg", default=None, help="path of ffmpeg (default: FFMPEG, then PATH)")
    ap.add_argument("--plan", type=Path, default=None)
    ap.add_argument("--skills", default="", help="only these skill ids")
    ap.add_argument("--pre", type=float, default=0.5, help="seconds before the press (default 0.5)")
    ap.add_argument("--max-seconds", type=float, default=15.0, help="most seconds after the press (default 15)")
    ap.add_argument("--height", type=int, default=0, help=f"output height of each side (default: the smaller video, else {DEFAULT_HEIGHT})")
    ap.add_argument("--fps", type=int, default=0, help="output frame rate (default: the higher of the two videos)")
    ap.add_argument("--crf", type=int, default=20)
    ap.add_argument("--font", default=None, help="a .ttf for the captions (default: SKILLPARITY_FONT or a system font)")
    ap.add_argument("--no-text", action="store_true", help="no captions and no counter")
    ap.add_argument("--no-all", action="store_true", help="do not write all.mp4")
    ap.add_argument("--out", type=Path, default=None, help="output directory (default: <run>/sidebyside)")
    ap.add_argument("--dry-run", action="store_true", help="print the ffmpeg commands and write the manifest only")
    args = ap.parse_args(argv)

    run = args.run
    ffmpeg = find_ffmpeg(args.ffmpeg)
    if ffmpeg is None and not args.dry_run:
        print("sidebyside: no ffmpeg: pass --ffmpeg PATH, set FFMPEG, or put ffmpeg on PATH", file=sys.stderr)
        return 2
    ffmpeg = ffmpeg or "ffmpeg"
    try:
        vo, vb = load_video_info(run / "orig"), load_video_info(run / "bevy")
    except (FileNotFoundError, ValueError) as error:
        print(f"sidebyside: {error}", file=sys.stderr)
        return 2
    orig_rows, _ = common.read_jsonl(run / "orig" / "trace.jsonl")
    bevy_rows, _ = common.read_jsonl(run / "bevy" / "trace.jsonl")
    if not orig_rows or not bevy_rows:
        print(f"sidebyside: need both traces under {run}", file=sys.stderr)
        return 2
    plan = diff.find_plan(run, args.plan)
    plan_skills = diff.plan_index(plan)
    orig, bevy = diff.load_trace(orig_rows, "orig"), diff.load_trace(bevy_rows, "bevy")
    only = [x for x in args.skills.split(",") if x.strip()] or None
    cuts = plan_cuts(orig, bevy, vo, vb, plan_skills, args.pre, args.max_seconds, only)

    heights = [v.get("height") for v in (vo, vb) if isinstance(v.get("height"), int) and v["height"] > 0]
    height = args.height or (min(heights) if heights else DEFAULT_HEIGHT)
    height -= height % 2
    fps = args.fps or int(round(max(float(vo["fps"]), float(vb["fps"]))))
    font = None if args.no_text else find_font(args.font)
    if not args.no_text and font is None:
        print("sidebyside: no font found (--font, SKILLPARITY_FONT): making the videos without captions")
    out_dir = args.out or run / "sidebyside"
    out_dir.mkdir(parents=True, exist_ok=True)

    made: list[Path] = []
    warnings: list[str] = []
    for cut in cuts:
        title = f"{cut['skill_id']} {cut['name']}" if cut.get("name") else str(cut["skill_id"])
        if cut.get("skipped"):
            warnings.append(f"{title}: skipped, {cut['skipped']}")
            print(f"sidebyside: {title}: skipped, {cut['skipped']}")
            continue
        suffix = f"_{cut['occurrence'] + 1}" if cut["occurrence"] else ""
        out = out_dir / (f"{cut['skill_id']}{suffix}" + (f"_{slug(cut['name'])}" if cut.get("name") else "") + ".mp4")
        cmd = build_command(ffmpeg, vo, vb, cut, out, title, font, height, fps, args.crf, text=font is not None)
        plain = build_command(ffmpeg, vo, vb, cut, out, title, None, height, fps, args.crf, text=False)
        cut["file"] = out.name
        if args.dry_run:
            print(" ".join(f'"{c}"' if " " in c or ";" in c else c for c in cmd))
            continue
        ok, err, dropped = run_cut(cmd, plain)
        if not ok:
            warnings.append(f"{title}: ffmpeg failed: {err.strip()[-300:]}")
            print(f"sidebyside: {title}: ffmpeg failed\n{err.strip()[-600:]}")
            continue
        if dropped:
            warnings.append(f"{title}: captions dropped (drawtext failed)")
        made.append(out)
        print(f"sidebyside: {out.name}")

    if made and not args.no_all and not args.dry_run:
        list_file = out_dir / "all.txt"
        list_file.write_text(concat_list_text(made), encoding="utf-8")
        ok, err, _ = run_cut(concat_command(ffmpeg, list_file, out_dir / "all.mp4"), concat_command(ffmpeg, list_file, out_dir / "all.mp4"))
        list_file.unlink(missing_ok=True)
        print("sidebyside: all.mp4" if ok else f"sidebyside: all.mp4 failed: {err.strip()[-300:]}")
        if not ok:
            warnings.append("all.mp4: " + err.strip()[-300:])
    manifest = {"run": run.name, "height": height, "fps": fps, "font": bool(font), "pre_s": args.pre,
                "videos": {"orig": {k: vo[k] for k in vo if k != "path"}, "bevy": {k: vb[k] for k in vb if k != "path"}},
                "cuts": cuts, "warnings": warnings}
    (out_dir / "sidebyside.json").write_text(json.dumps(manifest, indent=1) + "\n", encoding="utf-8")
    return 0 if (made or args.dry_run) else 1


if __name__ == "__main__":
    raise SystemExit(main())
