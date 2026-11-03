"""Shared pieces of the skill parity tools: trace file IO, name rules, tolerances and the role rule.

Everything here is plain Python (standard library only) and works on the `skillparity/v1` rows described in
README.md. `learn_player_inst` / `assign_roles` implement the README's role rule for the original client; both
`diff.py` and `run_original.py` use them.
"""
from __future__ import annotations

import json
import os
import re
import statistics
import time
from pathlib import Path
from typing import Any, Iterable

SCHEMA = "skillparity/v1"
CLIENTS = ("orig", "bevy")
CLIENT_LABEL = {"orig": "1.2 client", "bevy": "aa-client"}

# README tolerances: timings 1 frame of the slower client, playback speed 2 %, blend 0.05 s, positions 0.1 m.
# The facing tolerance is not in the README: 2 degrees is a choice of these tools.
SPEED_REL_TOL = 0.02
BLEND_TOL_S = 0.05
POS_TOL_M = 0.1
YAW_TOL_DEG = 2.0
DEFAULT_FRAME_MS = 1000.0 / 30.0   # used when a trace has no way to tell its frame time

KINDS = ("mark", "press", "anim", "fx", "sound", "net", "castbar", "cooldown", "pos", "frame")


# ---- time and files -----------------------------------------------------------------------------------------------

def now_us() -> int:
    """Unix time in microseconds from the OS wall clock (the clock every source of a capture shares)."""
    return time.time_ns() // 1000


def read_jsonl(path: str | os.PathLike) -> tuple[list[dict], int]:
    """(rows, number of unreadable lines). A missing file gives ([], 0). Rows are sorted by `wall_us` (stable)."""
    p = Path(path)
    if not p.is_file():
        return [], 0
    rows: list[dict] = []
    bad = 0
    with p.open("r", encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except ValueError:
                bad += 1
                continue
            if isinstance(row, dict):
                rows.append(row)
            else:
                bad += 1
    rows.sort(key=lambda r: r.get("wall_us") if isinstance(r.get("wall_us"), (int, float)) else float("inf"))
    return rows, bad


def write_jsonl(path: str | os.PathLike, rows: Iterable[dict]) -> int:
    """Write rows one JSON object per line (UTF-8, compact); returns the row count."""
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    count = 0
    with p.open("w", encoding="utf-8", newline="\n") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
            count += 1
    return count


class TraceWriter:
    """Append rows to a trace file, flushing every row (a crash loses nothing). Thread safety is not needed: one runner."""

    def __init__(self, path: str | os.PathLike, src: str):
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.src = src
        self.handle = self.path.open("a", encoding="utf-8", newline="\n")
        self.count = 0

    def write(self, row: dict) -> None:
        row.setdefault("src", self.src)
        self.handle.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
        self.handle.flush()
        self.count += 1

    def mark(self, label: str, skill_id: int | None = None, wall_us: int | None = None, frame: int | None = None) -> dict:
        """A `mark` row with the runner's own wall clock (README: the runner writes its own marks)."""
        row: dict[str, Any] = {"src": self.src, "kind": "mark", "wall_us": wall_us if wall_us is not None else now_us(),
                               "label": label}
        if frame is not None:
            row["frame"] = frame
        if skill_id is not None:
            row["skill_id"] = skill_id
        self.write(row)
        return row

    def close(self) -> None:
        try:
            self.handle.close()
        except OSError:
            pass


def norm_name(value: Any) -> str:
    """README rule: lowercase, `\\` turned into `/`, no `game/` prefix on paths."""
    text = str(value if value is not None else "").strip().lower().replace("\\", "/")
    if text.startswith("game/"):
        text = text[5:]
    return text


# ---- frame time ---------------------------------------------------------------------------------------------------

def frame_ms(rows: list[dict]) -> float | None:
    """Median frame time of a trace in ms: from `frame` rows' `dt_ms`, else from `frame` counter and `wall_us` of any rows."""
    dts = [r["dt_ms"] for r in rows if r.get("kind") == "frame" and isinstance(r.get("dt_ms"), (int, float)) and r["dt_ms"] > 0]
    if len(dts) >= 3:
        return float(statistics.median(dts))
    pairs: list[tuple[int, int]] = []
    last: tuple[int, int] | None = None
    for r in rows:
        f, w = r.get("frame"), r.get("wall_us")
        if isinstance(f, int) and isinstance(w, (int, float)):
            if last is not None and f > last[0] and w > last[1]:
                pairs.append((f - last[0], int(w) - last[1]))
            last = (f, int(w))
    per_frame = [w / 1000.0 / df for df, w in pairs if df > 0]
    # a long gap between rows would say little about the frame time: keep only plausible values
    per_frame = [v for v in per_frame if 1.0 <= v <= 250.0]
    if len(per_frame) >= 3:
        return float(statistics.median(per_frame))
    return None


# ---- roles: the player's skeleton in the original's trace ---------------------------------------------------------

# A clip is idle / locomotion when one of its name tokens is one of these and none of the skill words is present.
# CAL clip names look like `sword_combat_idle_01`, `all_run_f`, `staff_cast_spell_01`; the split is on `_`, `/`, `.`.
LOCOMOTION_TOKENS = {
    "idle", "stand", "run", "walk", "jog", "sprint", "swim", "fly", "glide", "jump", "fall", "land", "turn", "strafe",
    "move", "breath", "wait", "sit", "ride", "rest", "loco", "locomotion", "crouch", "climb", "float", "tread",
}
SKILL_TOKENS = {
    "attack", "cast", "spell", "skill", "shot", "shoot", "slash", "strike", "charge", "combo", "fire", "magic",
    "throw", "stab", "swing", "smash", "bash", "dash", "kick", "punch", "buff", "heal", "channel", "dance",
}
# Sounds and effects that are not the skill's: the world's ambience and music, footsteps (other units walk past in both runs at
# their own times), the UI's clicks. They differ run by run in both clients, so the skill comparison leaves them out (`ambient`).
AMBIENT_RE = re.compile(r"(^|/)sounds/x2(ambience|music|interface|monster_footstep|footstep)[:/]|^collisions\.footsteps\.")


def is_ambient(name: str) -> bool:
    return bool(AMBIENT_RE.search(norm_name(name)))


# Hit reactions: what the target (and only the target) plays after the player's skill.
HIT_TOKENS = {"hit", "damage", "dmg", "stagger", "knock", "knockback", "down", "react", "reaction", "flinch", "stun", "hurt"}


def clip_tokens(clip: Any) -> set[str]:
    return {t for t in re.split(r"[_/.\-\s]+", norm_name(clip)) if t}


def is_locomotion(clip: Any) -> bool:
    """True for idle and locomotion clips (the ones that say nothing about a skill)."""
    tokens = clip_tokens(clip)
    return bool(tokens & LOCOMOTION_TOKENS) and not (tokens & SKILL_TOKENS)


def is_hit_reaction(clip: Any) -> bool:
    return bool(clip_tokens(clip) & HIT_TOKENS)


def _timed(rows: list[dict]) -> list[dict]:
    """The rows with a numeric `wall_us`, in time order (the role rules walk a trace forward from the first press)."""
    return sorted((r for r in rows if isinstance(r.get("wall_us"), (int, float))), key=lambda r: r["wall_us"])


def learn_player_inst(rows: list[dict], window_us: int = 1_000_000) -> dict | None:
    """The player's skeleton `inst` in a trace whose `anim` rows carry no `role`.

    README rule: the inst of the first anim starting within 1 s after the first `press` of the run whose clip is not an
    idle / locomotion clip (see `is_locomotion`). Returns {"inst", "clip", "dt_ms", "press_us"} or None when the trace
    has no press, no such anim, or no `inst`.
    """
    rows = _timed(rows)
    press = next((r for r in rows if r.get("kind") == "press"), None)
    if press is None:
        return None
    t0 = int(press["wall_us"])
    for r in rows:
        if r.get("kind") != "anim":
            continue
        dt = int(r["wall_us"]) - t0
        if dt < 0:
            continue
        if dt > window_us:
            break
        if r.get("inst") in (None, "") or not r.get("clip") or is_locomotion(r["clip"]):
            continue
        return {"inst": str(r["inst"]), "clip": norm_name(r["clip"]), "dt_ms": dt / 1000.0, "press_us": t0}
    return None


def learn_target_inst(rows: list[dict], player_inst: str, window_us: int = 3_000_000) -> str | None:
    """README: the target's skeleton is learned from its hit reactions: the first other inst that plays a hit clip
    within 3 s after the first press."""
    rows = _timed(rows)
    press = next((r for r in rows if r.get("kind") == "press"), None)
    if press is None:
        return None
    t0 = int(press["wall_us"])
    for r in rows:
        if r.get("kind") != "anim" or int(r["wall_us"]) < t0:
            continue
        if int(r["wall_us"]) - t0 > window_us:
            break
        if r.get("inst") in (None, "") or str(r["inst"]) == player_inst:
            continue
        if is_hit_reaction(r.get("clip")):
            return str(r["inst"])
    return None


def assign_roles(rows: list[dict], force: bool = False) -> dict:
    """Fill `role` (`player` / `target` / `other`) in place on the rows that have an `inst` but no `role`.

    The player's inst comes from `learn_player_inst`, the target's from `learn_target_inst`. A row that already has a
    `role` is left alone unless `force`. Returns a summary {"player_inst", "target_inst", "set", "learned_from"}.
    """
    # a role the trace already gave for some inst is trusted for that inst
    known_player = next((str(r["inst"]) for r in rows if r.get("role") == "player" and r.get("inst") not in (None, "")), None)
    learned = None if known_player else learn_player_inst(rows)
    player = known_player or (learned["inst"] if learned else None)
    known_target = next((str(r["inst"]) for r in rows if r.get("role") == "target" and r.get("inst") not in (None, "")), None)
    target = known_target or (learn_target_inst(rows, player) if player else None)
    changed = 0
    for r in rows:
        if r.get("kind") not in ("anim", "fx", "sound") or r.get("inst") in (None, ""):
            continue
        if r.get("role") and not force:
            continue
        inst = str(r["inst"])
        role = "player" if inst == player else "target" if inst == target else "other"
        if player is None:
            continue   # nothing learned: leave the rows without a role rather than call everything `other`
        r["role"] = role
        changed += 1
    return {"player_inst": player, "target_inst": target, "set": changed,
            "learned_from": (learned or {}).get("clip") if learned else ("trace roles" if known_player else None)}


def has_player_role(rows: list[dict]) -> bool:
    return any(r.get("kind") == "anim" and r.get("role") == "player" for r in rows)


def ensure_roles(rows: list[dict]) -> dict:
    """Apply the role rule when a trace's anim rows are not labelled (original client); no-op for labelled traces."""
    if has_player_role(rows):
        return {"player_inst": next((str(r["inst"]) for r in rows if r.get("role") == "player" and r.get("inst")), None),
                "target_inst": None, "set": 0, "learned_from": "trace roles"}
    return assign_roles(rows)


# ---- small helpers ------------------------------------------------------------------------------------------------

def kinds_in(rows: Iterable[dict]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for r in rows:
        k = r.get("kind")
        if isinstance(k, str):
            counts[k] = counts.get(k, 0) + 1
    return counts


def parse_skill_label(label: Any) -> tuple[str, str] | None:
    """`skill:14376:start` -> ("14376", "start"); anything else -> None."""
    m = re.fullmatch(r"\s*skill:([0-9A-Za-z_\-]+):(start|end)\s*", str(label or ""))
    return (m.group(1), m.group(2)) if m else None


def as_skill_id(text: Any) -> int | str:
    try:
        return int(text)
    except (TypeError, ValueError):
        return str(text)


def env_path(name: str) -> Path | None:
    value = os.environ.get(name)
    return Path(value) if value else None
