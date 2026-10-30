#!/usr/bin/env python3
"""Draft replay/checker for a bundled tree-cutdown capture.

The capture is the evidence plan in tools/evidence/cutdown/capture-plan.txt: the server
packets around one cutdown (0x10f phase change, 0x081 SCVegetationCutdowning, 0x10d
removed), the cut tree's root transform over time, the cutter position, and screenshots.
This script decodes the packets, measures the fall from the transforms, and compares it
with the client's current approximation (away from the cutter, 0.1 s native start delay,
2.2 s quadratic fall; crates/aa-client/src/ingame/cutdown.rs).

Usage:
    python tools/cutdown_replay.py --capture tools/evidence/cutdown/capture.json
    python tools/cutdown_replay.py --self-test
    python tools/cutdown_replay.py --print-schema

No game files or live server are needed. Exit code 1 means the capture is malformed or
contradicts the packet layout; timing/direction differences are reported, not failures,
because the native fall is still unverified.
"""
from __future__ import annotations

import argparse
import json
import math
import struct
import sys
from pathlib import Path

OP_CUTDOWN = 0x081
OP_PHASE = 0x10F
OP_REMOVED = 0x10D

# The current client approximation (ingame/cutdown.rs).
CLIENT_START_DELAY_S = 0.1
CLIENT_FALL_S = 2.2

SCHEMA = {
    "meta": {
        "source": "1.2 client capture name",
        "skill_id": 16469,
        "cutter_obj_id": 4096,
        "cutter_pos": [0.0, 0.0, 0.0],
        "tree_template": 1234,
        "note": "positions are the client's cry/game metres (x, y, z)",
    },
    "phase_models": {"3151": "vegetation://objects/env/05_nature/03_temp/temp_tree_oak03.cgf", "3169": ""},
    "packets": [
        {"t_ms": 0, "dir": "S->C", "opcode": "0x10f", "body": "12 34 56 ... 19 bytes"},
        {"t_ms": 5, "dir": "S->C", "opcode": "0x081", "body": "00 01 00 00 02 00"},
        {"t_ms": 5305, "dir": "S->C", "opcode": "0x10d", "body": "00 02 00"},
    ],
    "transforms": [
        {"t_ms": 0, "obj_id": 512, "pos": [10.0, 0.0, 0.0], "rot": [0.0, 0.0, 0.0, 1.0]},
        {"t_ms": 3000, "obj_id": 512, "pos": [10.0, 0.0, 0.0], "rot": [0.0, 0.7071068, 0.0, 0.7071068]},
    ],
    "screenshots": ["cutdown_before.png", "cutdown_fall.png", "cutdown_after.png"],
}


def _bytes(text: str) -> bytes:
    return bytes.fromhex(text.replace(" ", "").replace("\n", ""))


def _bc(body: bytes, offset: int) -> tuple[int, int]:
    if len(body) < offset + 3:
        raise ValueError(f"short bc at {offset}: {body.hex(' ')}")
    return int.from_bytes(body[offset:offset + 3], "little"), offset + 3


def decode_cutdown(body: str) -> tuple[int, int]:
    raw = _bytes(body)
    if len(raw) != 6:
        raise ValueError(f"SCVegetationCutdowning must be 6 bytes, got {len(raw)}: {raw.hex(' ')}")
    unit, off = _bc(raw, 0)
    doodad, _ = _bc(raw, off)
    return unit, doodad


def decode_phase(body: str) -> dict:
    raw = _bytes(body)
    if len(raw) != 19:
        raise ValueError(f"SCDoodadPhaseChanged must be 19 bytes, got {len(raw)}: {raw.hex(' ')}")
    obj_id, off = _bc(raw, 0)
    group, time_left, puzzle, item = struct.unpack_from("<IIiI", raw, off)
    return {"obj_id": obj_id, "func_group_id": group, "time_left": time_left,
            "puzzle_group": puzzle, "item_template_id": item}


def decode_removed(body: str) -> list[int]:
    raw = _bytes(body)
    obj_id, _ = _bc(raw, 0)
    return [obj_id]


def quat_rotate(q: list[float], v: tuple[float, float, float]) -> tuple[float, float, float]:
    x, y, z, w = q
    vx, vy, vz = v
    # q * v * q^-1, expanded to avoid a dependency.
    tx = 2.0 * (y * vz - z * vy)
    ty = 2.0 * (z * vx - x * vz)
    tz = 2.0 * (x * vy - y * vx)
    return (vx + w * tx + y * tz - z * ty,
            vy + w * ty + z * tx - x * tz,
            vz + w * tz + x * ty - y * tx)


def tilt_deg(q: list[float]) -> float:
    up = quat_rotate(q, (0.0, 0.0, 1.0))
    return math.degrees(math.acos(max(-1.0, min(1.0, up[2]))))


def horizontal(q: list[float]) -> tuple[float, float]:
    up = quat_rotate(q, (0.0, 0.0, 1.0))
    length = math.hypot(up[0], up[1])
    if length < 1e-6:
        return (0.0, 0.0)
    return (up[0] / length, up[1] / length)


def analyse(capture: dict) -> dict:
    meta = capture.get("meta", {})
    packets = capture.get("packets", [])
    cuts = {}
    selected = None
    phases = []
    removals = []
    for p in packets:
        if p.get("dir", "S->C") != "S->C":
            continue
        op = int(str(p.get("opcode", "0")), 16)
        if op == OP_CUTDOWN:
            unit, doodad = decode_cutdown(p["body"])
            # the client ignores a repeated cut of a tree that is already falling (ingame/cutdown.rs), so
            # a tree stays cut at its first packet (a later one would drop the earlier fall samples);
            # the last packet's tree is the one measured
            cuts.setdefault(doodad, {"t_ms": p.get("t_ms", 0), "unit": unit, "doodad": doodad})
            selected = doodad
        elif op == OP_PHASE:
            phases.append({"t_ms": p.get("t_ms", 0), **decode_phase(p["body"])})
        elif op == OP_REMOVED:
            removals += [{"t_ms": p.get("t_ms", 0), "obj_id": obj} for obj in decode_removed(p["body"])]
    if selected is None:
        raise ValueError("capture has no SCVegetationCutdowning (0x081) packet")
    cutdown = cuts[selected]

    tree_frames = sorted(
        (f for f in capture.get("transforms", []) if f.get("obj_id") == cutdown["doodad"]),
        key=lambda f: f.get("t_ms", 0),
    )
    at = tree_frames[0] if tree_frames else None
    cutter = meta.get("cutter_pos")
    expected = None
    if at is not None and cutter is not None:
        dx = at["pos"][0] - cutter[0]
        dy = at["pos"][1] - cutter[1]
        length = math.hypot(dx, dy)
        expected = (dx / length, dy / length) if length > 1e-6 else None

    first_motion = None
    horizontal_at = None
    max_tilt = 0.0
    max_dir = None
    for frame in tree_frames:
        if frame["t_ms"] < cutdown["t_ms"]:
            continue
        q = frame["rot"]
        tilt = tilt_deg(q)
        if first_motion is None and tilt > 0.5:
            first_motion = frame["t_ms"]
        if horizontal_at is None and tilt >= 85.0:
            horizontal_at = frame["t_ms"]
        if tilt > max_tilt:
            max_tilt = tilt
            max_dir = horizontal(q)

    tree_phase_before = [p for p in phases if p["obj_id"] == cutdown["doodad"] and p["t_ms"] <= cutdown["t_ms"]]
    tree_phases_after = [p for p in phases if p["obj_id"] == cutdown["doodad"] and p["t_ms"] > cutdown["t_ms"]]
    removed = [r for r in removals if r["obj_id"] == cutdown["doodad"]]
    dot = None
    if max_dir is not None and expected is not None:
        dot = max_dir[0] * expected[0] + max_dir[1] * expected[1]

    return {
        "cutdown": cutdown,
        "expected_direction": expected,
        "first_motion_ms": first_motion,
        "horizontal_ms": horizontal_at,
        "max_tilt_deg": max_tilt,
        "direction_dot": dot,
        "phase_before_cutdown": bool(tree_phase_before),
        "phase_after_cutdown": tree_phases_after,
        "removed_ms": removed[0]["t_ms"] if removed else None,
        "frames": len([f for f in tree_frames if f["t_ms"] >= cutdown["t_ms"]]),
    }


def report(result: dict) -> str:
    cut = result["cutdown"]
    lines = [
        f"SCVegetationCutdowning: unit {cut['unit']} (0x{cut['unit']:x}) cut doodad "
        f"{cut['doodad']} (0x{cut['doodad']:x}) at t={cut['t_ms']} ms",
        f"phase change before the cutdown: {result['phase_before_cutdown']}",
        f"tree frames after the cut: {result['frames']}",
        f"fall direction dot(expected cutter->tree): {result['direction_dot']}",
        f"max tilt: {result['max_tilt_deg']:.1f} deg",
        f"first motion >0.5 deg: {result['first_motion_ms']} ms; horizontal (>=85 deg): "
        f"{result['horizontal_ms']} ms",
        f"removed: {result['removed_ms']} ms",
    ]
    if result["horizontal_ms"] is not None and result["first_motion_ms"] is not None:
        seconds = (result["horizontal_ms"] - result["first_motion_ms"]) / 1000.0
        lines.append(f"observed fall {seconds:.2f} s (client approximation {CLIENT_FALL_S:.2f} s, "
                     f"start delay {CLIENT_START_DELAY_S:.2f} s)")
    if result["direction_dot"] is not None and result["direction_dot"] < 0.5:
        lines.append("WARNING: the tree falls toward the cutter or sideways; the client falls away.")
    if result["frames"] == 0:
        lines.append("WARNING: no transforms for the cut doodad after the cut; record the root transform.")
    return "\n".join(lines)


def self_test_capture() -> dict:
    # A synthetic tree at (10, 0, 0) cut from the origin, 50 Hz frames, a phase change to an
    # empty group first and the packet 5 ms later, removal after the fall. The fall uses the
    # client's curve so the checker can exercise the measurement path.
    frames = []
    for t in range(0, 3200, 50):
        elapsed = t / 1000.0
        progress = max(0.0, min(1.0, (elapsed - CLIENT_START_DELAY_S) / CLIENT_FALL_S))
        angle = math.pi / 2.0 * progress * progress
        frames.append({"t_ms": t, "obj_id": 0x200, "pos": [10.0, 0.0, 0.0],
                       "rot": [0.0, math.sin(angle / 2.0), 0.0, math.cos(angle / 2.0)]})
    phase = struct.pack("<I", 3169) + struct.pack("<I", 3000) + struct.pack("<i", -1) + struct.pack("<I", 0)
    return {
        "meta": {"source": "self-test", "cutter_obj_id": 0x100, "cutter_pos": [0.0, 0.0, 0.0]},
        "phase_models": {"3169": ""},
        "packets": [
            {"t_ms": 0, "dir": "S->C", "opcode": "0x10f", "body": (b"\x00\x02\x00" + phase).hex(" ")},
            {"t_ms": 5, "dir": "S->C", "opcode": "0x081", "body": "00 01 00 00 02 00"},
            {"t_ms": 5000, "dir": "S->C", "opcode": "0x10d", "body": "00 02 00"},
        ],
        "transforms": frames,
        "screenshots": [],
    }


def self_test() -> int:
    result = analyse(self_test_capture())
    print(report(result))
    assert result["cutdown"] == {"t_ms": 5, "unit": 0x100, "doodad": 0x200}
    assert result["phase_before_cutdown"]
    assert result["direction_dot"] is not None and result["direction_dot"] > 0.99, result["direction_dot"]
    assert result["max_tilt_deg"] > 89.0, result["max_tilt_deg"]
    assert result["horizontal_ms"] is not None and result["removed_ms"] == 5000
    fall = (result["horizontal_ms"] - result["first_motion_ms"]) / 1000.0
    assert 1.5 < fall < 3.0, fall
    # a repeated broadcast for the same tree is ignored, like the client does: the cut stays the
    # first packet, so the fall samples between the two packets still count
    repeated = self_test_capture()
    repeated["packets"].insert(2, {"t_ms": 1000, "dir": "S->C", "opcode": "0x081", "body": "00 01 00 00 02 00"})
    again = analyse(repeated)
    assert again["cutdown"] == result["cutdown"], again["cutdown"]
    assert again == result, (again, result)
    # malformed bodies are rejected
    for bad in ("", "00 01 00", "00 01 00 00 02", "00 01 00 00 02 00 00"):
        try:
            decode_cutdown(bad)
        except ValueError:
            pass
        else:
            raise AssertionError(f"accepted a malformed cutdown body: {bad}")
    print("self-test: ok")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture", type=Path, help="capture JSON (see --print-schema)")
    parser.add_argument("--self-test", action="store_true", help="check the decoder/measurement path")
    parser.add_argument("--print-schema", action="store_true", help="print the expected capture shape")
    args = parser.parse_args(argv)
    if args.print_schema:
        print(json.dumps(SCHEMA, indent=2))
        return 0
    if args.self_test:
        return self_test()
    if not args.capture:
        parser.error("--capture, --self-test or --print-schema is required")
    try:
        capture = json.loads(args.capture.read_text(encoding="utf-8"))
        result = analyse(capture)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"cutdown replay: {error}", file=sys.stderr)
        return 1
    print(report(result))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
