#!/usr/bin/env python3
"""Compare the original client's trace with aa-client's, skill by skill: report.json + report.md.

    python tools/skillparity/diff.py <capture-root>/<run-id> [--plan plan.json] [--skills 18132,11918] [--tol-ms 34] [--out DIR]

Inputs (README, "A capture is one directory"): `orig/trace.jsonl`, `bevy/trace.jsonl`, optional `server.jsonl` (net rows
from `server_log.py`, shared by both runs: a client's segment takes the server rows of its own time window) and the plan
(`--plan`, else `<run>/plan.json`, else the `plan` of `orig/meta.json`).

Each trace is cut into segments by the `mark` rows `skill:<id>:start` / `skill:<id>:end` (without skill marks: by the
`press` rows). Everything in a segment is timed from its `press` (t = 0). Per skill and client the tool measures

* net: press -> first `net out CSStartSkill`, and the server's answers (`SCSkillStarted`, `SCSkillFired`, `SCPlotEvent`,
  `SCSkillEnded`, ...) with their first times and counts (server rows when `server.jsonl` exists, the client's own net rows too)
* anim: every player clip in order (start, layer, speed, blend, loop), first clip times
* fx and sound: the set of names, first time and count of each
* castbar (start, stop, total, channel), cooldown (skill and GCD start and total)
* pos: the player's and the target's displacement over the segment, when it starts and ends moving, the turn

then compares orig with bevy under the README tolerances and classifies each difference: `missing` (orig has it, bevy not),
`extra` (bevy only), `timing` (a time differs by more than a frame of the slower client) and `value` (speed, blend, loop,
layer, counts, totals, positions). A metric a client never records (no `pos` rows in its whole trace, say) is NO-DATA for
that pair, never PASS. Verdicts: DIFF (any difference), NO-DATA (a segment or its press is missing, nothing recorded, or the
anim comparison itself was impossible), else PASS.

Roles: an original trace whose `anim` rows have no `role` gets them from the README rule (`common.assign_roles`).
"""
from __future__ import annotations

import argparse
import bisect
import json
import math
import statistics
import sys
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402
from common import CLIENT_LABEL, norm_name  # noqa: E402

REPORT_SCHEMA = "skillparity/report/v1"
CATEGORIES = ("missing", "extra", "timing", "value")
PRE_US = 5_000                 # rows up to 5 ms before the press still belong to a skill (clock jitter between sources)
FALLBACK_SEGMENT_US = 15_000_000
ANSWER_OPS = ("SCSkillStarted", "SCSkillFired", "SCPlotEvent", "SCSkillEnded", "SCSkillStopped", "SCCastingStopped",
              "SCPlotEnded", "SCPlotStopped")
MOVE_START_M = POS_NOISE_M = 0.05     # a position row counts as movement beyond this distance from where the skill started


# ---- tolerances ---------------------------------------------------------------------------------------------------

def tolerances(orig: "Trace", bevy: "Trace", tol_ms: float | None = None) -> dict:
    """README tolerances; the timing one is one frame of the slower client unless `tol_ms` is given."""
    frames = [f for f in (orig.frame_ms, bevy.frame_ms) if f]
    return {
        "timing_ms": round(float(tol_ms) if tol_ms else max(frames, default=common.DEFAULT_FRAME_MS), 3),
        "timing_source": "--tol-ms" if tol_ms else ("frame time of the slower client" if frames else "default 30 fps (no frame data)"),
        "speed_rel": common.SPEED_REL_TOL,
        "blend_s": common.BLEND_TOL_S,
        "pos_m": common.POS_TOL_M,
        "yaw_deg": common.YAW_TOL_DEG,
        "frame_ms": {"orig": orig.frame_ms, "bevy": bevy.frame_ms},
    }


# ---- one trace, indexed ---------------------------------------------------------------------------------------------

class Trace:
    """A client's rows indexed by kind and time."""

    def __init__(self, rows: list[dict], src: str):
        self.src = src
        self.rows = [r for r in rows if isinstance(r.get("wall_us"), (int, float))]
        self.dropped = len(rows) - len(self.rows)
        self.rows.sort(key=lambda r: r["wall_us"])
        self.kinds = common.kinds_in(self.rows)
        self.by_kind: dict[str, list[dict]] = {}
        for r in self.rows:
            self.by_kind.setdefault(r.get("kind"), []).append(r)
        self.times = {k: [r["wall_us"] for r in v] for k, v in self.by_kind.items()}
        self.frame_ms = common.frame_ms(self.rows)
        self.last_us = self.rows[-1]["wall_us"] if self.rows else 0
        self.roles = {"player_inst": None, "target_inst": None, "set": 0, "learned_from": None}

    def window(self, kind: str, lo: float, hi: float) -> list[dict]:
        rows = self.by_kind.get(kind)
        if not rows:
            return []
        t = self.times[kind]
        return rows[bisect.bisect_left(t, lo):bisect.bisect_right(t, hi)]

    def last_before(self, kind: str, t: float, pred=None) -> dict | None:
        rows = self.by_kind.get(kind)
        if not rows:
            return None
        i = bisect.bisect_left(self.times[kind], t)
        while i > 0:
            i -= 1
            if pred is None or pred(rows[i]):
                return rows[i]
        return None

    def has(self, kind: str) -> bool:
        return self.kinds.get(kind, 0) > 0


def load_trace(rows: list[dict], src: str) -> Trace:
    """Index rows; an original trace gets its anim roles from the README rule first."""
    common.ensure_roles(rows)
    trace = Trace(rows, src)
    trace.roles = {"player_inst": next((str(r["inst"]) for r in trace.by_kind.get("anim", []) if r.get("role") == "player" and r.get("inst")), None),
                   "target_inst": next((str(r["inst"]) for r in trace.by_kind.get("anim", []) if r.get("role") == "target" and r.get("inst")), None)}
    return trace


# ---- segments -----------------------------------------------------------------------------------------------------

@dataclass
class Segment:
    skill_id: Any
    occurrence: int
    start_us: int
    end_us: int
    press_us: int | None
    slot: int | None
    source: str          # "marks" | "press"

    @property
    def key(self) -> tuple:
        return (self.skill_id, self.occurrence)


def _first_press(trace: Trace, lo: int, hi: int) -> dict | None:
    rows = trace.window("press", lo, hi)
    return rows[0] if rows else None


def find_segments(trace: Trace, plan_skills: dict[Any, dict] | None = None, server: Trace | None = None) -> list[Segment]:
    """Cut a trace at its skill marks; without any skill mark, at its presses (skill from CSStartSkill or the plan's slot)."""
    marks = [(r["wall_us"], common.parse_skill_label(r.get("label")), r) for r in trace.by_kind.get("mark", [])]
    marks = [(t, p, r) for t, p, r in marks if p]
    segs: list[Segment] = []
    if marks:
        open_at: dict[str, int] = {}
        spans: list[tuple[str, int, int | None]] = []
        for t, (sid, what), _ in marks:
            if what == "start":
                if sid in open_at:                       # a second start without an end: the first one ends here
                    spans.append((sid, open_at[sid], t))
                open_at[sid] = t
            else:
                start = open_at.pop(sid, None)
                if start is not None:
                    spans.append((sid, start, t))
        for sid, start in open_at.items():
            later = [t for t, (_, what), _ in marks if what == "start" and t > start]
            spans.append((sid, start, min(later) if later else trace.last_us))
        spans.sort(key=lambda s: s[1])
        seen: Counter = Counter()
        for sid, start, end in spans:
            skill = common.as_skill_id(sid)
            press = _first_press(trace, start - 50_000, end)
            slot = press.get("slot") if press else None
            segs.append(Segment(skill, seen[skill], int(start), int(end), int(press["wall_us"]) if press else None,
                                slot if isinstance(slot, int) else None, "marks"))
            seen[skill] += 1
        return segs

    presses = trace.by_kind.get("press", [])
    in_order = list(plan_skills or {})
    sequential = len(presses) == len(in_order) and all(
        presses[i].get("slot") in (None, plan_skills[sid].get("slot")) for i, sid in enumerate(in_order))
    seen = Counter()
    for i, p in enumerate(presses):
        t = int(p["wall_us"])
        end = int(presses[i + 1]["wall_us"]) - 1 if i + 1 < len(presses) else min(trace.last_us, t + FALLBACK_SEGMENT_US)
        end = max(end, t)
        sid: Any = None
        for source in (trace, server):
            if source is None or sid is not None:
                continue
            for r in source.window("net", t, t + 1_000_000):
                if r.get("op") == "CSStartSkill" and r.get("skill_id") is not None:
                    sid = r["skill_id"]
                    break
        slot = p.get("slot") if isinstance(p.get("slot"), int) else None
        if sid is None and sequential:
            sid = in_order[i]                    # as many presses as plan skills, each on its plan slot: in plan order
        if sid is None:
            sid = f"slot{slot if slot is not None else i}"
        skill = common.as_skill_id(sid)
        segs.append(Segment(skill, seen[skill], t, end, t, slot, "press"))
        seen[skill] += 1
    return segs


# ---- numbers ------------------------------------------------------------------------------------------------------

def num(value: Any) -> float | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, (int, float)) and math.isfinite(value):
        return float(value)
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def ms_from(t: float, press: float) -> float:
    return round((t - press) / 1000.0, 3)


def r3(x: float | None) -> float | None:
    return None if x is None else round(x, 3)


def wrap_deg(a: float) -> float:
    a = (a + 180.0) % 360.0 - 180.0
    return a


# ---- metrics of one client in one segment ---------------------------------------------------------------------------

def net_metrics(rows: list[dict], skill: Any, press: float, source: str) -> dict:
    """press -> CSStartSkill and the server's answers among `net` rows of the window (skill filtered)."""
    skill_n = num(skill)
    mine = [r for r in rows if skill_n is None or r.get("skill_id") is None or num(r.get("skill_id")) == skill_n]
    tls = {r["tl"] for r in mine if r.get("skill_id") is not None and r.get("tl") is not None}
    # a packet with the skill id of another skill is dropped above; one with only a `tl` (SCSkillEnded) must belong to our casts
    selected = [r for r in mine if r.get("skill_id") is not None or r.get("tl") is None or r["tl"] in tls]
    out: dict[str, Any] = {"source": source, "rows": len(selected), "start_skill_ms": None, "answers": {}}
    for r in selected:
        op, t = r.get("op"), ms_from(r["wall_us"], press)
        if r.get("dir") == "out" and op == "CSStartSkill" and out["start_skill_ms"] is None:
            out["start_skill_ms"] = t
        elif r.get("dir") == "in" or (op or "").startswith("SC"):
            a = out["answers"].setdefault(op, {"first_ms": t, "count": 0, "last_ms": t})
            a["count"] += 1
            a["last_ms"] = t
    return out


def anim_metrics(trace: Trace, lo: float, hi: float, press: float) -> dict:
    clips = []
    for r in trace.window("anim", lo, hi):
        if r.get("role") != "player":
            continue
        clip = norm_name(r.get("clip"))
        clips.append({"clip": clip, "t_ms": ms_from(r["wall_us"], press), "layer": r.get("layer"), "speed": num(r.get("speed")),
                      "blend": num(r.get("blend")), "loop": r.get("loop"), "file": norm_name(r["file"]) if r.get("file") else None,
                      "flags": r.get("flags"), "locomotion": common.is_locomotion(clip)})
    return {"clips": clips, "first_ms": clips[0]["t_ms"] if clips else None,
            "first_skill_ms": next((c["t_ms"] for c in clips if not c["locomotion"]), None)}


def name_set_metrics(rows: list[dict], press: float) -> dict:
    out: dict[str, dict] = {}
    for r in rows:
        name = norm_name(r.get("name"))
        if not name:
            continue
        item = out.setdefault(name, {"first_ms": ms_from(r["wall_us"], press), "count": 0, "pos": None})
        item["count"] += 1
        if item["pos"] is None and isinstance(r.get("pos"), (list, tuple)) and len(r["pos"]) >= 3:
            item["pos"] = [num(v) for v in r["pos"][:3]]
    return out


def fx_metrics(trace: Trace, lo: float, hi: float, press: float) -> dict:
    return name_set_metrics([r for r in trace.window("fx", lo, hi) if r.get("phase") in (None, "spawn")], press)


def sound_metrics(trace: Trace, lo: float, hi: float, press: float) -> dict:
    # the original logs `create` and `play`, aa-client one voice start: compare `play` where a trace has it, else everything
    use_play = any(r.get("phase") == "play" for r in trace.by_kind.get("sound", []))
    rows = [r for r in trace.window("sound", lo, hi) if not use_play or r.get("phase") == "play"]
    return name_set_metrics(rows, press)


def castbar_metrics(trace: Trace, lo: float, hi: float, press: float) -> dict:
    rows = [r for r in trace.window("castbar", lo, hi) if r.get("unit") in (None, "player")]
    casts: list[dict] = []
    cur: dict | None = None
    for r in rows:
        t = ms_from(r["wall_us"], press)
        if r.get("state") == "start":
            if cur is not None:
                casts.append(cur)
            cur = {"start_ms": t, "stop_ms": None, "name": r.get("name"), "total_ms": num(r.get("total_ms")),
                   "channel": r.get("channel") if isinstance(r.get("channel"), bool) else None}
        elif r.get("state") == "stop" and cur is not None:
            cur["stop_ms"] = t
            casts.append(cur)
            cur = None
    if cur is not None:
        casts.append(cur)
    return {"casts": casts, "count": len(casts)}


def cooldown_metrics(trace: Trace, lo: float, hi: float, press: float, slot: int | None) -> dict:
    out: dict[str, Any] = {"skill": None, "gcd": None}
    for r in trace.window("cooldown", lo, hi):
        remain = num(r.get("remain_ms"))
        if remain is None or remain <= 0:
            continue
        is_gcd = bool(r.get("gcd"))
        if not is_gcd and slot is not None and isinstance(r.get("slot"), int) and r["slot"] != slot:
            continue
        key = "gcd" if is_gcd else "skill"
        if out[key] is None:
            out[key] = {"start_ms": ms_from(r["wall_us"], press), "total_ms": num(r.get("total_ms")), "slot": r.get("slot")}
    return out


def pos_metrics(trace: Trace, unit: str, lo: float, hi: float, press: float) -> dict:
    """Displacement of a unit over the segment from `pos` change rows (state at the press = the last row before it)."""
    def is_unit(r: dict) -> bool:
        return r.get("unit") == unit

    rows = [r for r in trace.window("pos", lo, hi) if is_unit(r)]
    base = trace.last_before("pos", press, is_unit)
    pts = ([base] if base else []) + rows
    pts = [r for r in pts if num(r.get("x")) is not None and num(r.get("y")) is not None]
    if not pts:
        return {"moved": None, "rows": 0}
    start = pts[0]
    sx, sy, sz = num(start["x"]), num(start["y"]), num(start.get("z")) or 0.0
    path = 0.0
    prev = (sx, sy, sz)
    move_start = move_end = None
    peak = 0.0
    last_t = None
    for r in pts[1:]:
        x, y, z = num(r["x"]), num(r["y"]), num(r.get("z")) or 0.0
        step = math.dist(prev, (x, y, z))
        path += step
        t = r["wall_us"]
        if last_t is not None and t > last_t and step > 0:
            peak = max(peak, step / ((t - last_t) / 1e6))
        last_t = t
        if math.dist((sx, sy), (x, y)) > MOVE_START_M or abs(z - sz) > MOVE_START_M:
            if move_start is None:
                move_start = ms_from(t, press)
        if step > POS_NOISE_M * 0.2:
            move_end = ms_from(t, press)
        prev = (x, y, z)
    end = pts[-1]
    ex, ey, ez = num(end["x"]), num(end["y"]), num(end.get("z")) or 0.0
    yaw0, yaw1 = num(start.get("yaw")), num(end.get("yaw"))
    return {
        "rows": len(rows),
        "start": [r3(sx), r3(sy), r3(sz)],
        "end": [r3(ex), r3(ey), r3(ez)],
        "delta": [r3(ex - sx), r3(ey - sy), r3(ez - sz)],
        "displacement_m": r3(math.dist((sx, sy), (ex, ey))),
        "path_m": r3(path),
        "peak_speed_mps": r3(peak),
        "moved": (math.dist((sx, sy), (ex, ey)) > MOVE_START_M) or abs(ez - sz) > MOVE_START_M,
        "move_start_ms": move_start,
        "move_end_ms": move_end if move_start is not None else None,
        "yaw_change_deg": r3(wrap_deg(yaw1 - yaw0)) if yaw0 is not None and yaw1 is not None else None,
    }


def client_metrics(trace: Trace, server: Trace | None, seg: Segment | None, plan_skill: dict | None) -> dict:
    """Everything measured for one client in one segment (all times in ms after the press)."""
    supported = {k: trace.has(k) for k in ("anim", "fx", "sound", "castbar", "cooldown", "pos")}
    if trace.has("anim") and not any(r.get("role") == "player" for r in trace.by_kind["anim"]):
        supported["anim"] = False        # clips exist but no player skeleton could be identified
    supported["net_client"] = trace.has("net")
    supported["net_server"] = server is not None
    out: dict[str, Any] = {"found": seg is not None, "supported": supported, "src": trace.src}
    if seg is None:
        return out
    out["source"] = seg.source
    out["press_us"] = seg.press_us
    out["segment_us"] = [seg.start_us, seg.end_us]
    if seg.press_us is None:
        out["reason"] = "no press row in the segment"
        return out
    press = float(seg.press_us)
    lo, hi = press - PRE_US, float(seg.end_us)
    out["length_ms"] = ms_from(hi, press)
    counts = {k: len(trace.window(k, lo, hi)) for k in ("anim", "fx", "sound", "net", "castbar", "cooldown", "pos")}
    out["rows"] = counts
    slot = seg.slot if seg.slot is not None else (plan_skill or {}).get("slot")
    out["slot"] = slot
    out["anim"] = anim_metrics(trace, lo, hi, press) if supported["anim"] else None
    out["fx"] = fx_metrics(trace, lo, hi, press) if supported["fx"] else None
    out["sound"] = sound_metrics(trace, lo, hi, press) if supported["sound"] else None
    out["castbar"] = castbar_metrics(trace, lo, hi, press) if supported["castbar"] else None
    out["cooldown"] = cooldown_metrics(trace, lo, hi, press, slot) if supported["cooldown"] else None
    out["pos_player"] = pos_metrics(trace, "player", lo, hi, press) if supported["pos"] else None
    out["pos_target"] = pos_metrics(trace, "target", lo, hi, press) if supported["pos"] else None
    out["net_client"] = net_metrics(trace.window("net", lo, hi), seg.skill_id, press, "client") if supported["net_client"] else None
    # the server's rows of this client's window (the log is shared by both runs, told apart by time)
    out["net_server"] = (net_metrics(server.window("net", lo, hi + 500_000), seg.skill_id, press, "server")
                         if server is not None else None)
    own_rows = sum(counts.values()) - counts["net"]
    net_rows = (out["net_client"] or {}).get("rows", 0) + (out["net_server"] or {}).get("rows", 0)
    out["empty"] = own_rows == 0 and net_rows == 0
    return out


# ---- the comparison -----------------------------------------------------------------------------------------------

CLIENT_NAMES = {"orig": CLIENT_LABEL["orig"], "bevy": CLIENT_LABEL["bevy"]}
UNITS = {"speed": "", "blend": " s", "total_ms": " ms", "displacement_m": " m", "delta_m": " m", "yaw_change_deg": " deg",
         "count": "", "layer": "", "loop": "", "channel": "", "path_m": " m", "file": "", "name": ""}


def _fmt(value: Any, unit: str = "") -> str:
    if value is None:
        return "none"
    if isinstance(value, float):
        value = round(value, 3)
        if value == int(value):
            value = int(value)
    return f"{value}{unit}"


class Differ:
    """Collects the differences of one skill and the data it used."""

    def __init__(self, skill: Any, tol: dict):
        self.skill = skill
        self.tol = tol
        self.diffs: list[dict] = []
        self.no_data: dict[str, str] = {}
        self.shifts: list[float] = []

    def add(self, category: str, metric: str, subject: str, field: str, orig: Any, bevy: Any, **extra: Any) -> None:
        d = {"skill_id": self.skill, "category": category, "metric": metric, "subject": subject, "field": field,
             "orig": orig, "bevy": bevy}
        d.update(extra)
        self.diffs.append(d)

    def time(self, metric: str, subject: str, field: str, o: float | None, b: float | None, **extra: Any) -> None:
        """Compare two times (ms after the press): missing/extra when only one client has it, else timing beyond the tolerance."""
        if o is None and b is None:
            return
        if o is None or b is None:
            self.add("missing" if b is None else "extra", metric, subject, field, o, b, **extra)
            return
        delta = round(b - o, 3)
        self.shifts.append(delta)
        if abs(delta) > self.tol["timing_ms"]:
            self.add("timing", metric, subject, field, o, b, delta_ms=delta, **extra)

    def value(self, metric: str, subject: str, field: str, o: Any, b: Any, **extra: Any) -> None:
        self.add("value", metric, subject, field, o, b, **extra)


def _occurrences(clips: list[dict]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for c in clips:
        out.setdefault(c["clip"], []).append(c)
    return out


def compare_anim(d: Differ, o: dict, b: dict) -> None:
    oc, bc = _occurrences(o["clips"]), _occurrences(b["clips"])
    matched: list[tuple[dict, dict]] = []
    for name in sorted(set(oc) | set(bc), key=lambda n: min([c["t_ms"] for c in oc.get(n, []) + bc.get(n, [])])):
        ol, bl = oc.get(name, []), bc.get(name, [])
        for i in range(max(len(ol), len(bl))):
            x, y = ol[i] if i < len(ol) else None, bl[i] if i < len(bl) else None
            label = name if i == 0 else f"{name} (#{i + 1})"
            if x is None:
                d.add("extra", "anim", label, "clip", None, y["t_ms"], layer=y["layer"], speed=y["speed"])
                continue
            if y is None:
                d.add("missing", "anim", label, "clip", x["t_ms"], None, layer=x["layer"], speed=x["speed"])
                continue
            matched.append((x, y))
            d.time("anim", label, "start_ms", x["t_ms"], y["t_ms"])
            if x["layer"] is not None and y["layer"] is not None and x["layer"] != y["layer"]:
                d.value("anim", label, "layer", x["layer"], y["layer"])
            if x["speed"] is not None and y["speed"] is not None:
                base = max(abs(x["speed"]), 1e-9)
                if abs(y["speed"] - x["speed"]) / base > d.tol["speed_rel"]:
                    d.value("anim", label, "speed", x["speed"], y["speed"], delta=r3(y["speed"] - x["speed"]))
            if x["blend"] is not None and y["blend"] is not None and abs(y["blend"] - x["blend"]) > d.tol["blend_s"]:
                d.value("anim", label, "blend", x["blend"], y["blend"], delta=r3(y["blend"] - x["blend"]))
            if isinstance(x["loop"], bool) and isinstance(y["loop"], bool) and x["loop"] != y["loop"]:
                d.value("anim", label, "loop", x["loop"], y["loop"])
            if x["file"] and y["file"] and x["file"] != y["file"]:
                d.value("anim", label, "file", x["file"], y["file"])
    # order of the clips both clients played
    by_orig = [x["clip"] for x, _ in sorted(matched, key=lambda p: p[0]["t_ms"])]
    by_bevy = [y["clip"] for _, y in sorted(matched, key=lambda p: p[1]["t_ms"])]
    if by_orig != by_bevy:
        d.value("anim", "clip order", "order", by_orig, by_bevy)


def compare_names(d: Differ, metric: str, o: dict, b: dict) -> None:
    for name in sorted(set(o) | set(b), key=lambda n: min(v["first_ms"] for v in (o.get(n), b.get(n)) if v)):
        x, y = o.get(name), b.get(name)
        if x is None:
            d.add("extra", metric, name, "name", None, y["first_ms"], count=y["count"])
            continue
        if y is None:
            d.add("missing", metric, name, "name", x["first_ms"], None, count=x["count"])
            continue
        d.time(metric, name, "first_ms", x["first_ms"], y["first_ms"])
        if x["count"] != y["count"]:
            d.value(metric, name, "count", x["count"], y["count"], delta=y["count"] - x["count"])
        if x["pos"] and y["pos"] and None not in x["pos"] + y["pos"]:
            dist = math.dist(x["pos"], y["pos"])
            if dist > d.tol["pos_m"]:
                d.value(metric, name, "pos", x["pos"], y["pos"], delta_m=r3(dist))


def compare_castbar(d: Differ, o: dict, b: dict) -> None:
    oc, bc = o["casts"], b["casts"]
    for i in range(max(len(oc), len(bc))):
        x, y = oc[i] if i < len(oc) else None, bc[i] if i < len(bc) else None
        label = "cast bar" if i == 0 else f"cast bar (#{i + 1})"
        if x is None:
            d.add("extra", "castbar", label, "cast", None, y["start_ms"], name=y["name"], total_ms=y["total_ms"])
            continue
        if y is None:
            d.add("missing", "castbar", label, "cast", x["start_ms"], None, name=x["name"], total_ms=x["total_ms"])
            continue
        d.time("castbar", label, "start_ms", x["start_ms"], y["start_ms"])
        if x["stop_ms"] is not None or y["stop_ms"] is not None:
            d.time("castbar", label, "stop_ms", x["stop_ms"], y["stop_ms"])
        if x["total_ms"] is not None and y["total_ms"] is not None and abs(x["total_ms"] - y["total_ms"]) > d.tol["timing_ms"]:
            d.value("castbar", label, "total_ms", x["total_ms"], y["total_ms"], delta_ms=r3(y["total_ms"] - x["total_ms"]))
        if x["channel"] is not None and y["channel"] is not None and x["channel"] != y["channel"]:
            d.value("castbar", label, "channel", x["channel"], y["channel"])
        if x["name"] and y["name"] and norm_name(x["name"]) != norm_name(y["name"]):
            d.value("castbar", label, "name", x["name"], y["name"])


def compare_cooldown(d: Differ, o: dict, b: dict) -> None:
    for key, label in (("skill", "skill cooldown"), ("gcd", "global cooldown")):
        x, y = o[key], b[key]
        if x is None and y is None:
            continue
        if x is None or y is None:
            d.add("missing" if y is None else "extra", "cooldown", label, "cooldown", x and x["start_ms"], y and y["start_ms"],
                  total_ms=(x or y)["total_ms"])
            continue
        d.time("cooldown", label, "start_ms", x["start_ms"], y["start_ms"])
        if x["total_ms"] is not None and y["total_ms"] is not None and abs(x["total_ms"] - y["total_ms"]) > d.tol["timing_ms"]:
            d.value("cooldown", label, "total_ms", x["total_ms"], y["total_ms"], delta_ms=r3(y["total_ms"] - x["total_ms"]))


def compare_pos(d: Differ, label: str, o: dict, b: dict) -> None:
    if o.get("moved") is None or b.get("moved") is None:
        who = " and ".join(CLIENT_NAMES[k] for k, m in (("orig", o), ("bevy", b)) if m.get("moved") is None)
        d.no_data[f"pos {label}"] = f"no position of the {label} from {who}"
        return
    metric = "pos"
    if o["moved"] != b["moved"]:
        x = o if o["moved"] else b
        d.add("missing" if o["moved"] else "extra", metric, label, "movement", o.get("displacement_m"), b.get("displacement_m"),
              moved_m=x["displacement_m"], moves_in="orig" if o["moved"] else "bevy")
        return
    if not o["moved"]:
        if o.get("yaw_change_deg") is not None and b.get("yaw_change_deg") is not None \
                and abs(wrap_deg(b["yaw_change_deg"] - o["yaw_change_deg"])) > d.tol["yaw_deg"]:
            d.value(metric, f"{label} facing", "yaw_change_deg", o["yaw_change_deg"], b["yaw_change_deg"])
        return
    vec = math.dist(o["delta"][:2], b["delta"][:2])
    if vec > d.tol["pos_m"] or abs(o["delta"][2] - b["delta"][2]) > d.tol["pos_m"]:
        d.value(metric, label, "displacement_m", o["displacement_m"], b["displacement_m"], delta_m=r3(vec),
                orig_delta=o["delta"], bevy_delta=b["delta"])
    d.time(metric, label, "move_start_ms", o["move_start_ms"], b["move_start_ms"])
    d.time(metric, label, "move_end_ms", o["move_end_ms"], b["move_end_ms"])
    if o.get("yaw_change_deg") is not None and b.get("yaw_change_deg") is not None \
            and abs(wrap_deg(b["yaw_change_deg"] - o["yaw_change_deg"])) > d.tol["yaw_deg"]:
        d.value(metric, f"{label} facing", "yaw_change_deg", o["yaw_change_deg"], b["yaw_change_deg"])


def compare_net(d: Differ, o: dict, b: dict, which: str) -> None:
    metric = f"net_{which}"
    if o["start_skill_ms"] is not None or b["start_skill_ms"] is not None:
        d.time(metric, "CSStartSkill", "start_ms", o["start_skill_ms"], b["start_skill_ms"])
    for op in sorted(set(o["answers"]) | set(b["answers"]), key=lambda n: (n not in ANSWER_OPS, n)):
        x, y = o["answers"].get(op), b["answers"].get(op)
        if x is None:
            d.add("extra", metric, op, "packet", None, y["first_ms"], count=y["count"])
        elif y is None:
            d.add("missing", metric, op, "packet", x["first_ms"], None, count=x["count"])
        else:
            d.time(metric, op, "first_ms", x["first_ms"], y["first_ms"])
            if x["count"] != y["count"]:
                d.value(metric, op, "count", x["count"], y["count"], delta=y["count"] - x["count"])


def compare_skill(skill: Any, occurrence: int, o: dict, b: dict, tol: dict, plan_skill: dict | None) -> dict:
    """The skill's record: verdict, differences, no-data metrics, and both clients' metrics."""
    d = Differ(skill, tol)
    rec: dict[str, Any] = {"skill_id": skill, "occurrence": occurrence, "name": (plan_skill or {}).get("name")}
    reason = None
    if not o["found"] or not b["found"]:
        reason = "no segment for the skill in " + " and ".join(
            CLIENT_NAMES[k] for k, m in (("orig", o), ("bevy", b)) if not m["found"])
    elif o.get("press_us") is None or b.get("press_us") is None:
        reason = "no press row in the segment of " + " and ".join(
            CLIENT_NAMES[k] for k, m in (("orig", o), ("bevy", b)) if m.get("press_us") is None)
    elif o.get("empty") and b.get("empty"):
        reason = "neither client recorded anything in the segment"
    if reason:
        rec.update(verdict="NO-DATA", reason=reason, differences=[], no_data={}, counts=dict.fromkeys(CATEGORIES, 0),
                   worst_delta_ms=None, median_shift_ms=None, orig=o, bevy=b)
        return rec

    def both(group: str, label: str) -> bool:
        if o.get(group) is None or b.get(group) is None:
            missing = [CLIENT_NAMES[k] for k, m in (("orig", o), ("bevy", b)) if m.get(group) is None]
            d.no_data[label] = f"no {label} data from " + " and ".join(missing)
            return False
        return True

    if both("anim", "anim"):
        compare_anim(d, o["anim"], b["anim"])
    if both("fx", "fx"):
        compare_names(d, "fx", o["fx"], b["fx"])
    if both("sound", "sound"):
        compare_names(d, "sound", o["sound"], b["sound"])
    if both("castbar", "castbar"):
        compare_castbar(d, o["castbar"], b["castbar"])
    if both("cooldown", "cooldown"):
        compare_cooldown(d, o["cooldown"], b["cooldown"])
    if both("pos_player", "pos"):
        compare_pos(d, "player", o["pos_player"], b["pos_player"])
        compare_pos(d, "target", o["pos_target"], b["pos_target"])
    net_done, net_reasons = False, {}
    for which in ("server", "client"):
        if both(f"net_{which}", f"net ({which})"):
            compare_net(d, o[f"net_{which}"], b[f"net_{which}"], which)
            net_done = True
        else:
            net_reasons[which] = d.no_data.pop(f"net ({which})")
    if not net_done:        # one net comparison is enough (usually the server log's); none at all is NO-DATA
        d.no_data["net"] = "no net data: " + "; ".join(net_reasons.values())

    for diff in d.diffs:
        diff["text"] = describe(diff)
    counts = Counter(x["category"] for x in d.diffs)
    deltas = [abs(x["delta_ms"]) for x in d.diffs if "delta_ms" in x]
    rec["counts"] = {c: counts.get(c, 0) for c in CATEGORIES}
    rec["worst_delta_ms"] = round(max(deltas), 3) if deltas else None
    rec["median_shift_ms"] = round(statistics.median(d.shifts), 3) if len(d.shifts) >= 3 else None
    rec["differences"] = d.diffs
    rec["no_data"] = d.no_data
    if d.diffs:
        rec["verdict"] = "DIFF"
    elif "anim" in d.no_data:
        rec["verdict"], rec["reason"] = "NO-DATA", "the animation comparison was not possible: " + d.no_data["anim"]
    else:
        rec["verdict"] = "PASS"
    rec["orig"], rec["bevy"] = o, b
    return rec


# ---- plain words --------------------------------------------------------------------------------------------------

TIME_VERBS = {
    ("anim", "start_ms"): "starts", ("fx", "first_ms"): "first spawns", ("sound", "first_ms"): "first plays",
    ("castbar", "start_ms"): "starts", ("castbar", "stop_ms"): "ends", ("cooldown", "start_ms"): "starts",
    ("pos", "move_start_ms"): "starts moving", ("pos", "move_end_ms"): "stops moving",
}
SUBJECT_KIND = {"anim": "clip", "fx": "effect", "sound": "sound", "castbar": "", "cooldown": "", "pos": "", "net_server": "server packet",
                "net_client": "client packet"}


def subject_text(diff: dict) -> str:
    kind, subject = SUBJECT_KIND.get(diff["metric"], ""), diff["subject"]
    if diff["metric"] in ("pos",) and diff["subject"] in ("player", "target"):
        return f"{subject} movement"
    return f"{kind} `{subject}`".strip() if kind else subject


def describe(diff: dict) -> str:
    """One sentence for a difference, naming the clients as README does (1.2 client, aa-client)."""
    cat, metric, field = diff["category"], diff["metric"], diff["field"]
    o, b = diff["orig"], diff["bevy"]
    who = subject_text(diff)
    if metric.startswith("net_") and field == "start_ms" and cat == "timing":
        who = "CSStartSkill"
    if cat == "timing":
        verb = TIME_VERBS.get((metric, field), "arrives" if metric.startswith("net_") else "happens")
        if metric == "net_client" and field == "start_ms":
            verb = "is sent"
        delta = diff["delta_ms"]
        return (f"{who}: {verb} {abs(delta):.0f} ms {'later' if delta > 0 else 'earlier'} in {CLIENT_NAMES['bevy']} "
                f"(orig {_fmt(o)} ms, bevy {_fmt(b)} ms)")
    if cat == "missing":
        if field in ("clip", "name", "packet", "cast", "cooldown"):
            extra = ""
            if "count" in diff:
                extra = f" ({diff['count']}x)"
            if field == "cooldown" or field == "cast":
                extra = f" (total {_fmt(diff.get('total_ms'))} ms)"
            when = f" at {_fmt(o)} ms" if o is not None else ""
            return f"{who}: missing in {CLIENT_NAMES['bevy']}; the {CLIENT_NAMES['orig']} has it{when}{extra}"
        if field == "movement":
            return f"{who}: the {CLIENT_NAMES['orig']} moves {_fmt(diff.get('moved_m'))} m, {CLIENT_NAMES['bevy']} does not move"
    if cat == "extra":
        if field in ("clip", "name", "packet", "cast", "cooldown"):
            when = f" at {_fmt(b)} ms" if b is not None else ""
            return f"{who}: only in {CLIENT_NAMES['bevy']}{when}; the {CLIENT_NAMES['orig']} does not have it"
        if field == "movement":
            return f"{who}: {CLIENT_NAMES['bevy']} moves {_fmt(diff.get('moved_m'))} m, the {CLIENT_NAMES['orig']} does not move"
    if cat == "missing" and field not in ("clip", "name", "packet", "cast", "cooldown", "movement"):
        return f"{who}: {field} is missing in {CLIENT_NAMES['bevy']} (orig {_fmt(o)})"
    if cat == "extra":
        return f"{who}: {field} appears only in {CLIENT_NAMES['bevy']} (bevy {_fmt(b)})"
    unit = UNITS.get(field, "")
    if field == "order":
        return f"{who}: the clips play in another order: {', '.join(o)} in the {CLIENT_NAMES['orig']}, {', '.join(b)} in {CLIENT_NAMES['bevy']}"
    if field == "count":
        return f"{who}: {_fmt(o)} in the {CLIENT_NAMES['orig']}, {_fmt(b)} in {CLIENT_NAMES['bevy']}"
    if field == "displacement_m":
        extra = f"; the end points are {diff['delta_m']:.2f} m apart" if "delta_m" in diff else ""
        return f"{who}: moves {_fmt(o)} m in the {CLIENT_NAMES['orig']}, {_fmt(b)} m in {CLIENT_NAMES['bevy']}{extra}"
    return f"{who}: {field.replace('_ms', '').replace('_', ' ')} is {_fmt(o, unit)} in the {CLIENT_NAMES['orig']}, {_fmt(b, unit)} in {CLIENT_NAMES['bevy']}"


# ---- the report ---------------------------------------------------------------------------------------------------

def common_key(diff: dict) -> tuple:
    """Same key = same root cause across skills: category, metric, the thing and (for values) the field."""
    return (diff["category"], diff["metric"], diff["subject"], diff["field"] if diff["category"] == "value" else "")


def common_differences(skills: list[dict], top: int = 40) -> list[dict]:
    groups: dict[tuple, dict] = {}
    for rec in skills:
        seen: set[tuple] = set()
        for diff in rec["differences"]:
            key = common_key(diff)
            g = groups.setdefault(key, {"category": key[0], "metric": key[1], "subject": key[2], "field": key[3], "skills": [],
                                        "occurrences": 0, "example": diff["text"]})
            g["occurrences"] += 1
            if key not in seen:
                g["skills"].append(rec["skill_id"])
                seen.add(key)
    ranked = sorted(groups.values(), key=lambda g: (-len(g["skills"]), -g["occurrences"], g["metric"], g["subject"]))
    for g in ranked:
        g["skill_count"] = len(g["skills"])
    return ranked[:top]


def plan_index(plan: dict | None) -> dict[Any, dict]:
    if not plan:
        return {}
    return {s["id"]: s for s in plan.get("skills", [])}


def build_report(orig_rows: list[dict], bevy_rows: list[dict], server_rows: list[dict] | None = None, plan: dict | None = None,
                 tol_ms: float | None = None, only: list[Any] | None = None, run_id: str | None = None,
                 include_unrecorded: bool = False) -> dict:
    """The whole comparison on rows in memory (the CLI loads the files and calls this)."""
    orig, bevy = load_trace(orig_rows, "orig"), load_trace(bevy_rows, "bevy")
    server = Trace(server_rows, "server") if server_rows else None
    tol = tolerances(orig, bevy, tol_ms)
    skills_plan = plan_index(plan)
    seg_o = find_segments(orig, skills_plan, server)
    seg_b = find_segments(bevy, skills_plan, server)
    map_o, map_b = {s.key: s for s in seg_o}, {s.key: s for s in seg_b}

    keys: list[tuple] = []
    for sid in skills_plan:                       # the plan's order first
        occs = sorted({k[1] for k in list(map_o) + list(map_b) if k[0] == sid})
        if not occs and include_unrecorded:
            occs = [0]
        keys.extend((sid, occ) for occ in occs)
    for k in [s.key for s in seg_o] + [s.key for s in seg_b]:
        if k not in keys:
            keys.append(k)
    if only:
        wanted = {common.as_skill_id(x) for x in only}
        keys = [k for k in keys if k[0] in wanted]

    records = []
    for sid, occ in keys:
        plan_skill = skills_plan.get(sid)
        o = client_metrics(orig, server, map_o.get((sid, occ)), plan_skill)
        b = client_metrics(bevy, server, map_b.get((sid, occ)), plan_skill)
        records.append(compare_skill(sid, occ, o, b, tol, plan_skill))

    verdicts = Counter(r["verdict"] for r in records)
    by_cat = Counter()
    for r in records:
        by_cat.update(r["counts"])
    return {
        "schema": REPORT_SCHEMA,
        "generated": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "capture": run_id,
        "tolerances": tol,
        "clients": {
            name: {"rows": len(t.rows), "unusable_rows": t.dropped, "kinds": t.kinds, "player_inst": t.roles["player_inst"],
                   "target_inst": t.roles["target_inst"], "segments": len(segs),
                   "segment_source": segs[0].source if segs else None}
            for name, t, segs in (("orig", orig, seg_o), ("bevy", bevy, seg_b))
        },
        "server": {"rows": len(server.rows)} if server else None,
        "summary": {"skills": len(records), "PASS": verdicts.get("PASS", 0), "DIFF": verdicts.get("DIFF", 0),
                    "NO-DATA": verdicts.get("NO-DATA", 0), "differences": sum(by_cat.values()),
                    "by_category": {c: by_cat.get(c, 0) for c in CATEGORIES}},
        "common_differences": common_differences(records),
        "skills": records,
        "notes": [
            "Times are ms after the skill's `press` row. The original's press row is the driver's key injection, aa-client's is "
            "the moment the HUD consumed the key, so a constant shift of every event (see `median_shift_ms`) is a press alignment "
            "offset more than a per-event difference.",
            "An event is a difference only beyond the timing tolerance (one frame of the slower client).",
        ],
    }


# ---- report.md ----------------------------------------------------------------------------------------------------

def md_cell(text: Any) -> str:
    return str(text).replace("|", "\\|").replace("\n", " ")


def render_markdown(report: dict) -> str:
    s, tol = report["summary"], report["tolerances"]
    lines = [f"# Skill parity report{': ' + report['capture'] if report.get('capture') else ''}", ""]
    lines += [f"Generated {report['generated']}. Original = the 1.2 client, bevy = aa-client. Times are ms after the press.", ""]
    lines += [f"* skills compared: {s['skills']} ({s['PASS']} PASS, {s['DIFF']} DIFF, {s['NO-DATA']} NO-DATA)",
              f"* differences: {s['differences']} ({', '.join(f'{c} {n}' for c, n in s['by_category'].items())})",
              f"* tolerances: timing {tol['timing_ms']} ms ({tol['timing_source']}), speed {tol['speed_rel'] * 100:g} %, "
              f"blend {tol['blend_s']} s, position {tol['pos_m']} m, facing {tol['yaw_deg']} deg",
              f"* rows: orig {report['clients']['orig']['rows']}, bevy {report['clients']['bevy']['rows']}"
              + (f", server {report['server']['rows']}" if report.get("server") else ", no server rows"), ""]
    lines += ["## Summary", "", "| skill | verdict | missing | extra | timing | value | worst timing diff (ms) | no data |",
              "|---|---|---:|---:|---:|---:|---:|---|"]
    for r in report["skills"]:
        title = f"{r['skill_id']} {r['name']}" if r.get("name") else str(r["skill_id"])
        if r.get("occurrence"):
            title += f" #{r['occurrence'] + 1}"
        c = r["counts"]
        worst = "" if r["worst_delta_ms"] is None else f"{r['worst_delta_ms']:.0f}"
        nd = ", ".join(r.get("no_data", {})) or (r.get("reason", "") if r["verdict"] == "NO-DATA" else "")
        lines.append(f"| {md_cell(title)} | {r['verdict']} | {c['missing']} | {c['extra']} | {c['timing']} | {c['value']} | {worst} | {md_cell(nd)} |")
    lines.append("")

    common_rows = [g for g in report["common_differences"] if g["skill_count"] >= 2]
    lines += ["## Most common differences", ""]
    if common_rows:
        lines += ["The same clip, effect or sound off in several skills is one root cause.", "",
                  "| skills | kind | what | example |", "|---:|---|---|---|"]
        for g in common_rows[:25]:
            what = f"{g['metric']} `{g['subject']}`" + (f" {g['field']}" if g["field"] else "")
            lines.append(f"| {g['skill_count']} | {g['category']} | {md_cell(what)} | {md_cell(g['example'])} |")
    else:
        lines.append("No difference repeats across skills.")
    lines.append("")

    lines += ["## Per skill", ""]
    for r in report["skills"]:
        title = f"{r['skill_id']} {r['name']}" if r.get("name") else str(r["skill_id"])
        lines += [f"### {title}: {r['verdict']}", ""]
        if r["verdict"] == "NO-DATA" and r.get("reason"):
            lines += [f"No comparison: {r['reason']}.", ""]
        for diff in r["differences"]:
            lines.append(f"* {diff['text']}")
        if r["differences"]:
            lines.append("")
        if r.get("no_data"):
            lines += ["No data for: " + "; ".join(r["no_data"].values()) + ".", ""]
        if r.get("median_shift_ms") is not None and abs(r["median_shift_ms"]) > tol["timing_ms"]:
            lines += [f"Every event is shifted by a median {r['median_shift_ms']:+.0f} ms: look at the press alignment first.", ""]
        o, b = r.get("orig") or {}, r.get("bevy") or {}
        if o.get("press_us") is not None and b.get("press_us") is not None:
            lines += _latency_table(o, b)
    return "\n".join(lines) + "\n"


def _first(m: dict, *path: str) -> Any:
    cur: Any = m
    for p in path:
        if cur is None:
            return None
        cur = cur.get(p) if isinstance(cur, dict) else None
    return cur


def _cast0(m: dict, key: str) -> Any:
    casts = (m.get("castbar") or {}).get("casts") or []
    return casts[0].get(key) if casts else None


def _latency_table(o: dict, b: dict) -> list[str]:
    rows = [
        ("press -> CSStartSkill (server log)", _first(o, "net_server", "start_skill_ms"), _first(b, "net_server", "start_skill_ms")),
        ("press -> CSStartSkill (client)", _first(o, "net_client", "start_skill_ms"), _first(b, "net_client", "start_skill_ms")),
        ("press -> first player clip", _first(o, "anim", "first_ms"), _first(b, "anim", "first_ms")),
        ("press -> first skill clip", _first(o, "anim", "first_skill_ms"), _first(b, "anim", "first_skill_ms")),
        ("press -> cast bar", _cast0(o, "start_ms"), _cast0(b, "start_ms")),
        ("cast bar total", _cast0(o, "total_ms"), _cast0(b, "total_ms")),
        ("press -> GCD", _first(o, "cooldown", "gcd", "start_ms"), _first(b, "cooldown", "gcd", "start_ms")),
        ("press -> skill cooldown", _first(o, "cooldown", "skill", "start_ms"), _first(b, "cooldown", "skill", "start_ms")),
        ("player displacement (m)", _first(o, "pos_player", "displacement_m"), _first(b, "pos_player", "displacement_m")),
        ("effects (spawned)", len(o["fx"]) if o.get("fx") is not None else None, len(b["fx"]) if b.get("fx") is not None else None),
        ("sounds", len(o["sound"]) if o.get("sound") is not None else None, len(b["sound"]) if b.get("sound") is not None else None),
        ("player clips", len(o["anim"]["clips"]) if o.get("anim") else None, len(b["anim"]["clips"]) if b.get("anim") else None),
    ]
    out = ["| measure | 1.2 client | aa-client |", "|---|---:|---:|"]
    shown = 0
    for label, x, y in rows:
        if x is None and y is None:
            continue
        out.append(f"| {label} | {_fmt(x)} | {_fmt(y)} |")
        shown += 1
    return (out + [""]) if shown else []


# ---- command line -------------------------------------------------------------------------------------------------

def find_plan(run: Path, given: Path | None) -> dict | None:
    candidates = [given] if given else [run / "plan.json"]
    for c in candidates:
        if c and c.is_file():
            return json.loads(c.read_text(encoding="utf-8"))
    if given:
        raise FileNotFoundError(f"plan not found: {given}")
    for side in ("orig", "bevy"):
        meta = run / side / "meta.json"
        if meta.is_file():
            try:
                plan = json.loads(meta.read_text(encoding="utf-8")).get("plan")
            except ValueError:
                plan = None
            if isinstance(plan, dict) and plan.get("skills"):
                return plan
    return None


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run", type=Path, help="capture directory <capture-root>/<run-id> with orig/ and bevy/")
    ap.add_argument("--plan", type=Path, default=None)
    ap.add_argument("--skills", default="", help="only these skill ids")
    ap.add_argument("--tol-ms", type=float, default=None, help="timing tolerance in ms (default: one frame of the slower client)")
    ap.add_argument("--server", type=Path, default=None, help="server.jsonl (default: <run>/server.jsonl)")
    ap.add_argument("--out", type=Path, default=None, help="directory for report.json / report.md (default: the run directory)")
    args = ap.parse_args(argv)
    run = args.run
    orig_rows, bad_o = common.read_jsonl(run / "orig" / "trace.jsonl")
    bevy_rows, bad_b = common.read_jsonl(run / "bevy" / "trace.jsonl")
    if not orig_rows or not bevy_rows:
        print(f"diff: need both orig/trace.jsonl and bevy/trace.jsonl under {run} "
              f"(orig {len(orig_rows)} rows, bevy {len(bevy_rows)} rows)", file=sys.stderr)
        return 2
    server_rows, _ = common.read_jsonl(args.server or run / "server.jsonl")
    plan = find_plan(run, args.plan)
    only = [common.as_skill_id(x) for x in args.skills.split(",") if x.strip()] or None
    report = build_report(orig_rows, bevy_rows, server_rows or None, plan, args.tol_ms, only, run.name)
    report["clients"]["orig"]["unreadable_lines"], report["clients"]["bevy"]["unreadable_lines"] = bad_o, bad_b
    out = args.out or run
    out.mkdir(parents=True, exist_ok=True)
    (out / "report.json").write_text(json.dumps(report, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    (out / "report.md").write_text(render_markdown(report), encoding="utf-8")
    s = report["summary"]
    print(f"diff: {s['skills']} skills: {s['PASS']} PASS, {s['DIFF']} DIFF, {s['NO-DATA']} NO-DATA; {s['differences']} differences "
          f"-> {out / 'report.md'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
