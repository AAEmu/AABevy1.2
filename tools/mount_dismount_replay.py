#!/usr/bin/env python3
"""Replay the 1.2 S2 dismount placement in the Bevy client.

Runs tools/evidence/mount_dismount/replay.txt with `aa-client --headless
--mute` against the same lane and reads the client's `AA_DISMOUNT_TRACE=1`
line, which samples the local rider's position every 100 ms for 1.5 s after
the detach together with the mount it left and the mount's facing.  The
samples are expressed in the mount frame (forward / left) and compared with
the native c_dismount capture in tools/evidence/mount_animations/native.json:
the last `CSMoveUnit` before the `CSUnMountMate` is the mount reference, the
`dismounted` row holds the rider's 100 ms samples.  The replay repeats the
dismount with the mount turned 90 degrees: the world offset rotates, the
mount-frame offset does not.

The client also logs `got off the mount ... reason 9: Left`, the side
`FUN_3909b1a0` picked, and the server trace must show one CSUnMountMate
0x0a8 body whose reason byte is 9 (the mate tl id is server-assigned and is
normalised to 00 00).

Before/after: before this slice (`mount-animations`, base 0619ad6a) the
local rider was unseated in place at the server's position and the get-off
clips played; there was no side placement and no hop.  The before column
below uses that behaviour.

Usage:
    python tools/mount_dismount_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --evidence tools/evidence/mount_dismount
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mount_animations_probe import Trace

ROOT = Path(__file__).resolve().parents[1]
CASES = [("dismount_yaw0", 0.0), ("dismount_yaw90", 90.0)]
CASE_RE = re.compile(r"CASE:([A-Za-z0-9_]+)")
TRACE_RE = re.compile(
    r"dismount-trace: t ([0-9.]+) pos \(([-0-9.]+), ([-0-9.]+), ([-0-9.]+)\) "
    r"mount \(([-0-9.]+), ([-0-9.]+), ([-0-9.]+)\) yaw ([-0-9.]+)")
SIDE_RE = re.compile(r"got off the mount at \(.*reason (\d+): (Left|Right|Mount)")
TOL_HALF = 0.25
TOL_END = 0.25
TOL_HOP = 0.15
BEFORE = ("rider unseated in place at the server position: 0.00 fwd / 0.00 left "
          "at 0.5 s and at the end, no hop, no side")
NATIVE_CASE = "c_dismount"
TAU = math.tau


def hex_bytes(body: str) -> bytes:
    return bytes.fromhex(body.replace(" ", ""))


def decode_position(v: bytes) -> tuple[float, float, float]:
    raw = lambda a, b, c: 8 * (a | (b << 8) | (c << 16))
    x = raw(v[0], v[1], v[2]) << 32
    y = raw(v[3], v[4], v[5]) << 32
    if v[8] & 0x80:
        x = -x
    if v[8] & 0x40:
        y = -y
    zr = v[6] | (v[7] << 8) | ((v[8] & 0x3F) << 16)
    z = zr * 0.00000023841858 * 4196.0 - 100.0
    return ((x >> 32) / 4096.0, (y >> 32) / 4096.0, z)


def decode_move(body: str) -> dict:
    raw = hex_bytes(body)
    obj = raw[0] | (raw[1] << 8) | (raw[2] << 16)
    offset = 9
    if raw[8] & 0x10:
        offset += 5
    pos = decode_position(raw[offset:offset + 9])
    direction = struct.unpack_from("<b", raw, offset + 9 + 6 + 2)[0]
    return {"obj_id": obj, "pos": pos, "direction": direction,
            "yaw": direction * TAU / 128.0}


def frame(mount: tuple[float, float, float], yaw: float, pos: tuple[float, float, float]) -> tuple[float, float]:
    forward = (-math.sin(yaw), math.cos(yaw))
    left = (-forward[1], forward[0])
    delta = (pos[0] - mount[0], pos[1] - mount[1])
    return (delta[0] * forward[0] + delta[1] * forward[1],
            delta[0] * left[0] + delta[1] * left[1])


def side_name(left: float) -> str:
    if abs(left) < 0.05:
        return "mount"
    return "left" if left > 0 else "right"


def native_record(native: dict) -> dict:
    rows = [r for r in native["rows"] if r.get("case") == NATIVE_CASE]
    detach = next(i for i, r in enumerate(rows)
                  if r.get("dir") == "C->S" and r.get("opcode") == "0x0A8")
    mount_row = next(r for r in reversed(rows[:detach])
                     if r.get("dir") == "C->S" and r.get("opcode") == "0x089" and r.get("body"))
    mount = decode_move(mount_row["body"])
    samples = json.loads(next(r for r in rows if r.get("event") == "dismounted")["client_state"])["samples"]
    points = []
    for sample in samples:
        pos = tuple(sample["pos"])
        fwd, left = frame(mount["pos"], mount["yaw"], pos)
        points.append({"t_ms": sample["since_detached_ms"], "pos": pos,
                       "forward": fwd, "left": left})
    start = points[0]
    end = points[-1]
    half = min(points, key=lambda p: abs(p["t_ms"] - 500.0))
    return {
        "body": rows[detach].get("body", "").strip(),
        "mount": {"pos": mount["pos"], "direction": mount["direction"], "yaw_deg": math.degrees(mount["yaw"])},
        "samples": points,
        "half": half,
        "end": end,
        "hop": max(p["pos"][2] for p in points) - start["pos"][2],
        "side": side_name(end["left"]),
    }


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("MOUNT_BEVY_CLIENT", "aa-client.exe"))
    parser.add_argument("--login", default=os.environ.get("MOUNT_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")))
    parser.add_argument("--account", default=os.environ.get("MOUNT_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("MOUNT_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("MOUNT_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("MOUNT_SERVER_LOG"))
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "mount_dismount"))
    parser.add_argument("--native", default=str(ROOT / "tools" / "evidence" / "mount_animations" / "native.json"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-dismount-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or MOUNT_{name.upper()})")
    client = Path(args.client)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    evidence = Path(args.evidence)
    native_path = Path(args.native)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    script = evidence / "replay.txt"
    for path in (client, password_file, server_log, script, native_path):
        if not path.exists():
            raise SystemExit(f"missing: {path}")
    native = native_record(json.loads(native_path.read_text(encoding="utf-8")))

    trace = Trace(server_log)
    trace.reset()
    packets: list[dict] = []
    stop = threading.Event()

    def poll() -> None:
        while not stop.wait(0.02):
            packets.extend(trace.poll())

    poller = threading.Thread(target=poll, daemon=True)
    poller.start()

    env = dict(os.environ)
    env["RUST_LOG"] = "info,aa_client::ingame::mount=debug"
    env["AA_DISMOUNT_TRACE"] = "1"
    command = [
        str(client), "--headless", "--mute", "--account", args.account, "--login", args.login,
        "--password-file", str(password_file), "--character", args.character, "--script", str(script),
    ]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, bufsize=1, env=env)
    assert process.stdout is not None
    lines: list[str] = []
    markers: list[tuple[float, str]] = []
    samples: list[tuple[float, dict]] = []
    sides: list[tuple[float, dict]] = []

    def read_output() -> None:
        for line in process.stdout:
            line = line.rstrip("\n")
            lines.append(line)
            marker = CASE_RE.search(line)
            if marker:
                markers.append((time.monotonic(), marker.group(1)))
            sample = TRACE_RE.search(line)
            if sample:
                samples.append((time.monotonic(), {
                    "t": float(sample.group(1)),
                    "pos": tuple(float(sample.group(i)) for i in (2, 3, 4)),
                    "mount": tuple(float(sample.group(i)) for i in (5, 6, 7)),
                    "yaw": float(sample.group(8)),
                }))
            side = SIDE_RE.search(line)
            if side:
                sides.append((time.monotonic(), {"reason": int(side.group(1)), "side": side.group(2)}))

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    try:
        process.wait(timeout=600)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        lines.append("replay: client timed out")
    reader.join(timeout=5.0)
    stop.set()
    poller.join(timeout=2.0)
    packets.extend(trace.poll())
    wall = round(time.monotonic() - started, 1)
    (raw / "bevy-replay.log").write_text(
        " ".join(command) + f"\n\nAA_DISMOUNT_TRACE=1 RUST_LOG={env['RUST_LOG']}\n\n"
        + "\n".join(lines) + "\n", encoding="utf-8")

    markers.sort()
    windows: dict[str, tuple[float, float | None]] = {}
    for i, (at, case) in enumerate(markers):
        windows[case] = (at, markers[i + 1][0] if i + 1 < len(markers) else None)
    detaches = [p for p in packets if p.get("dir") == "C->S" and p.get("opcode") == 0x0A8]

    cases: dict[str, dict] = {}
    checks_ok = process.returncode == 0
    for case, yaw_deg in CASES:
        window = windows.get(case)
        if window is None:
            cases[case] = {"result": "DIFFERS", "why": "case marker missing"}
            checks_ok = False
            continue
        begin, end = window
        inside = [s for t, s in samples if t >= begin and (end is None or t < end)]
        got = next((s for t, s in sides if t >= begin and (end is None or t < end)), None)
        wire = [p for p in detaches if begin <= p["t"] and (end is None or p["t"] < end)]
        record: dict = {"yaw_deg": yaw_deg, "samples": inside, "side_log": got,
                        "packets": [p.get("body", "").strip() for p in wire]}
        if len(inside) < 2:
            record.update({"result": "DIFFERS", "why": f"{len(inside)} trace samples"})
            checks_ok = False
            cases[case] = record
            continue
        mount = inside[0]["mount"]
        yaw = inside[0]["yaw"]
        for s in inside:
            s["forward"], s["left"] = frame(mount, yaw, s["pos"])
        half = min(inside, key=lambda s: abs(s["t"] - 0.5))
        last = inside[-1]
        hop = max(s["pos"][2] for s in inside) - inside[0]["pos"][2]
        side = side_name(last["left"])
        record.update({
            "mount": mount, "yaw": yaw,
            "half": half, "end": last, "hop": hop, "side": side,
            "world_end": (last["pos"][0] - mount[0], last["pos"][1] - mount[1]),
        })
        native_half = native["half"]
        native_end = native["end"]
        diffs = {
            "half": math.hypot(half["forward"] - native_half["forward"], half["left"] - native_half["left"]),
            "end": math.hypot(last["forward"] - native_end["forward"], last["left"] - native_end["left"]),
            "hop": abs(hop - native["hop"]),
            "yaw": abs((math.degrees(yaw) - yaw_deg + 180.0) % 360.0 - 180.0),
        }
        record["diffs"] = diffs
        reasons = []
        if diffs["yaw"] > 5.0:
            reasons.append(f"mount yaw {math.degrees(yaw):.1f} deg, wanted {yaw_deg}")
        if diffs["half"] > TOL_HALF:
            reasons.append(f"0.5 s offset {diffs['half']:.3f} m > {TOL_HALF}")
        if diffs["end"] > TOL_END:
            reasons.append(f"end offset {diffs['end']:.3f} m > {TOL_END}")
        if diffs["hop"] > TOL_HOP:
            reasons.append(f"hop {hop:.3f} vs {native['hop']:.3f} m > {TOL_HOP}")
        if side != native["side"]:
            reasons.append(f"side {side}, capture {native['side']}")
        if len(wire) != 1:
            reasons.append(f"{len(wire)} CSUnMountMate, wanted 1")
        elif hex_bytes(wire[0]["body"].strip())[3] != 9:
            reasons.append(f"CSUnMountMate reason {hex_bytes(wire[0]['body'].strip())[3]}, wanted 9")
        record["result"] = "DIFFERS" if reasons else "MATCH"
        record["why"] = "; ".join(reasons)
        if reasons:
            checks_ok = False
        cases[case] = record

    checks = {
        "script_exit": process.returncode,
        "script_pass": process.returncode == 0,
        "bevy_pid": process.pid,
        "cases": len(cases),
        "matched": sum(1 for c in cases.values() if c.get("result") == "MATCH"),
        "ok": checks_ok,
        "tolerance_m": {"half": TOL_HALF, "end": TOL_END, "hop": TOL_HOP},
    }
    record = {
        "meta": {
            "account": "<account>", "character": "<character>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "client": "<client>",
            "server_log": "<server-log>", "wall_s": wall, "bevy_pid": process.pid,
            "rust_log": env["RUST_LOG"], "aa_dismount_trace": True,
            "native": "<native>",
        },
        "checks": checks,
        "native": native,
        "cases": cases,
    }
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")

    def metrics_text(record: dict) -> str:
        half = record["half"]
        end = record["end"]
        return (f"{half['forward']:.2f} fwd / {half['left']:.2f} left at 0.5 s; "
                f"{end['forward']:.2f} fwd / {end['left']:.2f} left at end; "
                f"hop +{record['hop']:.3f} m; {record['side']}")

    native_text = (f"{native['half']['forward']:.2f} fwd / {native['half']['left']:.2f} left at 0.5 s; "
                   f"{native['end']['forward']:.2f} fwd / {native['end']['left']:.2f} left at end; "
                   f"hop +{native['hop']:.3f} m; {native['side']}")

    out = [
        "## Bevy dismount replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  account: <account> / <character>",
        f"client: <client>  (--headless --mute)  pid: {process.pid} (started and stopped by the replay)",
        f"script: <evidence>/{script.name}  exit: {process.returncode}  wall: {wall} s",
        "",
        "The 1.2 reference is the c_dismount capture (tools/evidence/mount_animations/native.json):",
        f"the mount's last CSMoveUnit before the CSUnMountMate is at "
        f"({native['mount']['pos'][0]:.3f}, {native['mount']['pos'][1]:.3f}, {native['mount']['pos'][2]:.3f})",
        f"direction byte {native['mount']['direction']} ({native['mount']['yaw_deg']:.2f} deg); the rider's "
        "100 ms samples run 0-1405 ms.",
        "",
        "Frame: forward = (-sin yaw, cos yaw), left = (-forward.y, forward.x); offsets are the rider minus",
        "the mount at the detach. Bevy samples the local rider every 100 ms for 1.5 s after the detach",
        "(AA_DISMOUNT_TRACE=1; the headless loop runs at 30 Hz). The comparison tolerance is",
        f"{TOL_HALF:.2f} m at 0.5 s and the end, {TOL_HOP:.2f} m for the hop, and the same side.",
        "",
        "| case | mount yaw | before (mount-animations) | 1.2 c_dismount | Bevy replay | result |",
        "|---|---|---|---|---|---|",
    ]
    for case, yaw_deg in CASES:
        c = cases[case]
        if "half" in c:
            out.append(f"| {case} | {yaw_deg:.0f} deg | {BEFORE} | {native_text} | {metrics_text(c)} | "
                       f"{c['result']} |")
        else:
            out.append(f"| {case} | {yaw_deg:.0f} deg | {BEFORE} | {native_text} | - | {c['result']} |")
    out += [
        "",
        "Mount-frame differences (Bevy minus 1.2):",
        "",
        "| case | 0.5 s | end | hop | mount yaw | world end (dx, dy) | side picked |",
        "|---|---|---|---|---|---|---|",
    ]
    for case, yaw_deg in CASES:
        c = cases[case]
        if "half" not in c:
            out.append(f"| {case} | - | - | - | - | - | - |")
            continue
        side_log = c.get("side_log")
        picked = f"{side_log['side']} (reason {side_log['reason']})" if side_log else "no log line"
        out.append(
            f"| {case} | {c['diffs']['half']:.3f} m | {c['diffs']['end']:.3f} m | "
            f"{c['hop']:.3f} vs {native['hop']:.3f} m | {math.degrees(c['yaw']):.2f} vs {yaw_deg:.0f} deg | "
            f"({c['world_end'][0]:+.2f}, {c['world_end'][1]:+.2f}) | {picked} |")
    out += [
        "",
        "The second case turns the mount 90 deg before the detach: the world end delta rotates with it",
        "(yaw 0: forward is +y and left is -x; yaw 90: forward is -x and left is -y) while the",
        "mount-frame offsets stay the same, so the placement is in the mount frame rather than in world axes.",
        "",
        "Wire: one CSUnMountMate 0x0a8 per case, reason byte 9 (native body " + native["body"] +
        ", mate tl id server-assigned): " +
        "; ".join(f"{case} {c['packets'][0]}" for case, _ in CASES if c.get("packets")),
        "",
        "Before (mount-animations, base 0619ad6a): `track_local_mount` unseated the local rider in place",
        "and `pose_riders` skipped a dismounting rider, so the character stayed at the server's detach",
        "position while the get-off clips played; no side probe, no hop. The S3 replay recorded that",
        "behaviour as `unseated at once and walked; no get-off chain`.",
        "",
        "The 1.2 capture's z drops from the seat (100.166) to 97.968 at 0.8 s and settles at 98.432 by",
        "1.3 s; its peak is 100.541 (+0.375 over the seat sample). The Bevy controller starts at the",
        "seat too (the last seat lift), adds the 4.0 m/s type-10 z impulse under gravity 25 and lands",
        "on the terrain, then applies the 0.5-0.8 s root motion; the hop and end offsets above are the",
        "measured comparison.",
        "",
        f"checks: {checks}",
        "",
    ]
    for case, _ in CASES:
        c = cases[case]
        if c.get("why"):
            out.append(f"{case}: {c['why']}")
    section = "\n".join(out) + ("\n" if out[-1] else "")
    (evidence / "replay-report.txt").write_text(section, encoding="utf-8")
    print(section)
    return 0 if checks["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
