#!/usr/bin/env python3
"""Replay the 1.2 rowboat steering cases in the Bevy client and compare with the capture.

Runs tools/evidence/ship_steering/replay.txt with `aa-client --headless --mute` against
the lane, polls the server's packet trace while it runs, and compares the per-case
`CSMoveUnit` 0x089 type-4 bodies with the native capture
(tools/evidence/ship_steering/native.json):

* send interval (median, and resend 0/0 while still),
* body layout and flags (11-byte type 4, flags 0),
* W/S throttle levels and the A/D steering ramp and release decay,
* leaving the helm (CSDiscardSlave 0x032 and no type-4 after it),
* the server's type-3 velocity shorts against the measured hull speed.

It writes bevy.json (packets and checks) and replay-report.txt (the before/after table)
to the evidence directory. The client is stopped by its pid on timeout or exit.

The game server must log the fields trace and the S->C trace structs:
`RUST_LOG=info,aaemu_game::net::fields=trace,aaemu_game::net=trace`.
The client needs the game pak (`--pak` or AA_PAK).

Usage:
    python tools/ship_steering_replay.py --client <aa-client.exe> \
        --login <host:port> --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        [--pak <game_pak>] [--evidence tools/evidence/ship_steering] [--fps 31.5]
"""
from __future__ import annotations

import argparse
import datetime
import json
import math
import os
import re
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ship_steering_probe import Trace, decode_move  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CASES = ["a_idle", "b_forward", "b_coast", "b_reverse", "c_left", "c_left_coast",
         "c_right", "c_right_coast", "d_leave", "d_after"]
INTEREST_C2S = {0x031, 0x032, 0x068, 0x089}
MARKER_RE = re.compile(r"case=([a-z0-9_.]+)")
UTC_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z")
SCRIPT_T_RE = re.compile(r"SCRIPT \[\s*([0-9.]+)s\]")
SCALE = 30.0 / 32767.0
WIRE_PER_MPS = 32767.0 / 30.0


def log(*args: object) -> None:
    print("ship-replay:", *args, flush=True)


def moves_of(rows: list[dict], case: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet":
            continue
        if row.get("opcode") != "0x089":
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        if decoded.get("type") != 4:
            continue
        decoded["_t"] = row["t_ms"]
        decoded["_len"] = row["len"]
        out.append(decoded)
    return out


def leave_of(rows: list[dict], case: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet" or row.get("opcode") != "0x032":
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        decoded["_t"] = row["t_ms"]
        decoded["_body"] = row["body"]
        decoded["_len"] = row["len"]
        out.append(decoded)
    return out


def type1_after(rows: list[dict], case: str, at: float) -> int:
    count = 0
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet" or row.get("opcode") != "0x089":
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        if decoded.get("type") == 1 and row["t_ms"] > at:
            count += 1
    return count


def body_len(move: dict) -> int:
    return int(move.get("_len") or 11)


def sequence(moves: list[dict]) -> list[list[float]]:
    out: list[list[float]] = []
    if not moves:
        return out
    base = moves[0]["time_ms"]
    last = None
    for move in moves:
        value = (move["throttle"], move["steering"])
        if value != last:
            out.append([round((move["time_ms"] - base) / 1000.0, 2), value[0], value[1]])
            last = value
    return out


def stats(moves: list[dict]) -> dict:
    if not moves:
        return {"count": 0, "len": [], "flags": [], "interval_median_ms": None,
                "interval_min_ms": None, "interval_max_ms": None, "throttle": [],
                "steering": [], "sequence": [], "resend_zero": False,
                "full_lock": False, "release_decay": None, "release_rate": None}
    times = [move["time_ms"] for move in moves]
    intervals = [b - a for a, b in zip(times, times[1:])]
    seq = sequence(moves)
    steering = [move["steering"] for move in moves]
    full = max(abs(value) for value in steering) >= 127
    decay = None
    zeros = [i for i, value in enumerate(steering) if value == 0]
    if full and zeros:
        before_zero = [value for value in steering[:zeros[-1]] if value != 0]
        if before_zero:
            decay = abs(before_zero[-1])
    rate = None
    for first, second in zip(moves, moves[1:]):
        before, after = abs(first["steering"]), abs(second["steering"])
        if before == 0 or after >= before:
            continue
        seconds = (second["time_ms"] - first["time_ms"]) / 1000.0
        if seconds > 0:
            rate = max(rate or 0.0, (before - after) / seconds)
    return {
        "count": len(moves),
        "len": sorted({body_len(move) for move in moves}),
        "flags": sorted({int(move.get("flags", 0)) for move in moves}),
        "interval_median_ms": round(statistics.median(intervals), 1) if intervals else None,
        "interval_min_ms": min(intervals) if intervals else None,
        "interval_max_ms": max(intervals) if intervals else None,
        "throttle": list(dict.fromkeys(move["throttle"] for move in moves)),
        "steering": list(dict.fromkeys(move["steering"] for move in moves)),
        "sequence": seq,
        "resend_zero": all(move["throttle"] == 0 and move["steering"] == 0 for move in moves),
        "full_lock": full,
        "release_decay": decay,
        "release_rate": round(rate, 1) if rate is not None else None,
    }


def type3_pairs(rows: list[dict], case: str | None = None) -> dict:
    """Wire velocity shorts per measured m/s from the S->C type-3 Ship structs."""
    points: list[tuple[float, float, float, float, float, float, float]] = []
    for row in rows:
        if row.get("name") != "SCOneUnitMovementPacket" or not row.get("decoded"):
            continue
        if case is not None and row.get("case") != case:
            continue
        decoded = json.loads(row["decoded"])
        if decoded.get("kind") != "Ship":
            continue
        if not all(key in decoded for key in ("x", "y", "z", "time")):
            continue
        points.append((float(decoded["time"]), float(decoded["x"]), float(decoded["y"]),
                       float(decoded["z"]), float(decoded.get("vel_x", 0)),
                       float(decoded.get("vel_y", 0)), float(decoded.get("vel_z", 0))))
    points.sort()
    ratios: list[float] = []
    steady: list[float] = []
    for first, second in zip(points, points[1:]):
        dt = (second[0] - first[0]) / 1000.0
        if not 0.03 < dt < 0.5:
            continue
        moved = math.dist(first[1:4], second[1:4])
        measured = moved / dt
        wire = math.hypot(second[4], second[5], second[6])
        if measured < 0.03 or wire <= 0:
            continue
        ratios.append(wire / measured)
        wire_first = math.hypot(first[4], first[5], first[6])
        if abs(wire - wire_first) <= 0.04 * max(wire, 1.0):
            steady.append(wire / measured)
    return {"points": len(points),
            "pairs": len(ratios),
            "median_wire_per_mps": round(statistics.median(ratios), 1) if ratios else None,
            "steady_n": len(steady),
            "steady_wire_per_mps": round(statistics.median(steady), 1) if steady else None}


def native_summary(native: dict) -> dict:
    rows = native["rows"]
    ship = None
    for row in rows:
        if row.get("name") == "SCMySlavePacket" and row.get("decoded"):
            decoded = json.loads(row["decoded"])
            if decoded.get("obj_id"):
                ship = decoded["obj_id"]
                break
    cases = {}
    for case in CASES:
        moves = moves_of(rows, case)
        cases[case] = {"moves": stats(moves), "leave": leave_of(rows, case)}
        leave = cases[case]["leave"]
        if leave:
            last = [move for move in moves if move["_t"] <= leave[0]["_t"]]
            cases[case]["leave"] = {
                "body": leave[0]["_body"], "len": leave[0]["_len"],
                "tl_id": leave[0].get("tl_id"),
                "speed": None,
                "moves_after": len([move for move in moves if move["_t"] > leave[0]["_t"]]),
                "type1_after": type1_after(rows, case, leave[0]["_t"]),
            }
    bind = next((row for row in rows
                 if row.get("event") == "packet" and row.get("opcode") == "0x031"), None)
    return {"ship": ship, "bind": bind, "cases": cases,
            "type3": type3_pairs(rows)}


class Lines(threading.Thread):
    def __init__(self, pipe) -> None:
        super().__init__(daemon=True)
        self.pipe = pipe
        self.lines: list[str] = []

    def run(self) -> None:
        for line in self.pipe:
            self.lines.append(line.rstrip())


def timed(lines: list[str]) -> list[tuple[float | None, str]]:
    anchor = None
    out: list[tuple[float | None, str]] = []
    for line in lines:
        stamp = None
        match = UTC_RE.match(line)
        if match:
            stamp = datetime.datetime.fromisoformat(match.group(1) + "+00:00").timestamp()
            if anchor is None:
                anchor = stamp
        else:
            match = SCRIPT_T_RE.search(line)
            if match and anchor is not None:
                stamp = anchor + float(match.group(1))
        out.append((stamp, line))
    return out


def assign(packets: list[dict], markers: list[tuple[float, str]]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for packet in packets:
        at = packet.get("t", packet.get("t_ms", 0) / 1000.0)
        case = None
        for stamp, name in markers:
            if stamp <= at:
                case = name
            else:
                break
        if case:
            out.setdefault(case, []).append(packet)
    return out


def bevy_cases(packets: list[dict]) -> list[dict]:
    rows = []
    for packet in packets:
        if packet["dir"] == "S->C":
            if packet["name"] not in ("SCOneUnitMovementPacket", "SCUnitMovementsPacket",
                                      "SCUnitAttachedPacket", "SCUnitDetachedPacket",
                                      "SCSlaveBoundPacket", "SCSlaveCreatedPacket",
                                      "SCMySlavePacket"):
                continue
            rows.append({"t_ms": packet["t_ms"], "event": "packet", "case": "",
                         "opcode": f"0x{packet['opcode']:03X}", "name": packet["name"],
                         "len": packet["len"], "body": "",
                         "decoded": json.dumps(packet.get("decoded") or {})})
            continue
        if packet["opcode"] not in INTEREST_C2S:
            continue
        if packet["opcode"] == 0x089:
            decoded = decode_move(packet["body"])
        elif packet["opcode"] in (0x031, 0x032):
            raw = bytes.fromhex(packet["body"].replace(" ", ""))
            decoded = {"tl_id": int.from_bytes(raw[:2], "little")}
        elif packet["opcode"] == 0x068:
            raw = bytes.fromhex(packet["body"].replace(" ", ""))
            decoded = {"npc_obj_id": int.from_bytes(raw[:3], "little")}
        else:
            decoded = {}
        rows.append({"t_ms": packet["t_ms"], "event": "packet", "case": "",
                     "opcode": f"0x{packet['opcode']:03X}", "len": packet["len"],
                     "body": packet["body"], "decoded": json.dumps(decoded)})
    return rows


def build(native: dict, bevy: dict) -> tuple[dict, str]:
    checks: dict[str, str] = {}
    table = []
    details = {}
    native_cases = native["cases"]
    bevy_rows = bevy["rows"]
    bind = bevy.get("bind") or next((row for row in bevy_rows
                                     if row["event"] == "packet" and row["opcode"] == "0x031"), None)
    ship = None
    for move in moves_of(bevy_rows, "a_idle"):
        ship = move["obj_id"]
        break
    for case in CASES:
        n = native_cases[case]["moves"]
        moves = moves_of(bevy_rows, case)
        leave = leave_of(bevy_rows, case)
        actual = {"moves": stats(moves), "leave": None}
        if leave:
            last = [move for move in moves if move["_t"] <= leave[0]["_t"]]
            actual["leave"] = {
                "body": leave[0]["_body"], "len": leave[0]["_len"],
                "tl_id": leave[0].get("tl_id"), "speed": None,
                "moves_after": len([move for move in moves if move["_t"] > leave[0]["_t"]]),
                "type1_after": type1_after(bevy_rows, case, leave[0]["_t"]),
            }
        details[case] = actual
        a = actual["moves"]
        if not moves:
            checks[case] = "MATCH" if case == "d_after" else "DIFFERS (no type-4 sends)"
            table.append((case, checks[case]))
            continue
        problems = []
        if a["len"] != n["len"]:
            problems.append(f"len {a['len']} vs {n['len']}")
        if a["flags"] != n["flags"]:
            problems.append(f"flags {a['flags']} vs {n['flags']}")
        if a["interval_median_ms"] is None or n["interval_median_ms"] is None \
                or abs(a["interval_median_ms"] - n["interval_median_ms"]) > 25:
            problems.append(f"interval {a['interval_median_ms']} vs {n['interval_median_ms']}")
        if case == "a_idle":
            if not a["resend_zero"]:
                problems.append("idle sends a nonzero axis")
            if a["count"] < 3:
                problems.append("does not resend")
        if case == "b_forward" and (127 not in a["throttle"] or moves[-1]["throttle"] != 0):
            problems.append(f"W throttle {a['throttle']} vs {n['throttle']}")
        if case == "b_reverse" and (-127 not in a["throttle"] or moves[-1]["throttle"] != 0):
            problems.append(f"S throttle {a['throttle']} vs {n['throttle']}")
        if case in ("c_left", "c_right"):
            sign = -1 if case == "c_left" else 1
            if 127 * sign not in a["steering"]:
                problems.append(f"steering {a['steering']} vs {n['steering']}")
            intermediate = any(0 < abs(value) < 127 and value * sign > 0
                               for value in a["steering"])
            if not intermediate:
                problems.append(f"no steering ramp sample {a['steering']} vs {n['steering']}")
            if a["release_decay"] is None or moves[-1]["steering"] != 0:
                problems.append("release does not decay to 0")
            if a["release_rate"] is None or not 250 <= a["release_rate"] <= 900:
                problems.append(f"release rate {a['release_rate']} vs {n['release_rate']}")
            if 127 not in a["throttle"]:
                problems.append(f"throttle {a['throttle']} vs {n['throttle']}")
        if case.endswith("_coast") and not a["resend_zero"]:
            problems.append("coast sends a nonzero axis")
        if case == "d_leave":
            if not leave:
                problems.append("no 0x032")
            else:
                if leave[0]["_len"] != 2:
                    problems.append(f"0x032 len {leave[0]['_len']}")
                if bind is None or leave[0].get("tl_id") != json.loads(bind["decoded"]).get("tl_id"):
                    problems.append("0x032 tl differs from 0x031")
                if actual["leave"]["moves_after"] != 0:
                    problems.append(f"{actual['leave']['moves_after']} type-4 after 0x032")
                if actual["leave"]["type1_after"] == 0:
                    problems.append("no type-1 after leaving")
        checks[case] = "MATCH" if not problems else "DIFFERS (" + "; ".join(problems) + ")"
        table.append((case, checks[case]))

    bevy_type3 = type3_pairs(bevy_rows)
    native_type3 = native["type3"]
    scale_ok = (bevy_type3["steady_wire_per_mps"] is not None
                and abs(bevy_type3["steady_wire_per_mps"] / 1104.5 - 1.0) <= 0.05)
    checks["type3_scale"] = "MATCH" if scale_ok else "DIFFERS"

    before = {case: "no type-4 sends (cart driving has no ship control)" for case in CASES}
    before["d_leave"] = "no type-4 sends; 0x032 while still"
    before["d_after"] = "no type-4 sends; type 1 resumes"
    native_text = {}
    for case in CASES:
        n = native_cases[case]["moves"]
        if not n["count"]:
            native_text[case] = "no type-4 sends; type 1 resumes"
            continue
        native_text[case] = (f"{n['count']} x type 4, {n['len'][0]} B, flags {n['flags']}, "
                             f"interval {n['interval_median_ms']} ms, "
                             f"throttle {n['throttle']}, steering {n['steering']}")
        if case == "d_leave":
            leave = native_cases[case]["leave"]
            native_text[case] += (f"; 0x032 tl {leave['tl_id']} at {leave['moves_after']} "
                                  f"moves after, then type 1")
    after_text = {}
    for case in CASES:
        a = details[case]["moves"]
        if not a["count"]:
            after_text[case] = "no type-4 sends" + (" (type 1 resumes)" if case == "d_after" else "")
            continue
        after_text[case] = (f"{a['count']} x type 4, {a['len'][0]} B, flags {a['flags']}, "
                            f"interval {a['interval_median_ms']} ms, "
                            f"throttle {a['throttle']}, steering {a['steering']}")
        if case == "d_leave":
            leave = details[case]["leave"]
            if leave:
                after_text[case] += (f"; 0x032 tl {leave['tl_id']} at "
                                     f"{leave['moves_after']} type-4 after, then "
                                     f"{leave['type1_after']} type 1")
            else:
                after_text[case] += "; no 0x032"

    ok = all(value == "MATCH" for value in checks.values())
    record = {
        "meta": bevy["meta"],
        "native_ship": native["ship"],
        "bevy_ship": ship,
        "native": {case: native_cases[case] for case in CASES},
        "bevy": details,
        "type3": {"native": native_type3, "bevy": bevy_type3},
        "moves": {case: [
            {"t_rel_s": round((move["time_ms"] - moves[0]["time_ms"]) / 1000.0, 3),
             "throttle": move["throttle"], "steering": move["steering"],
             "flags": move["flags"], "len": body_len(move)}
            for move in moves]
            for case, moves in ((case, moves_of(bevy_rows, case)) for case in CASES)},
        "checks": checks,
        "ok": ok,
    }
    lines = [
        "## Bevy replay",
        "",
        f"date: {bevy['meta']['date']}  account: <account> / <character>",
        f"client: <client>  (--headless --mute, --headless-fps {bevy['meta']['fps']})",
        f"script: <evidence>/replay.txt  exit: {bevy['meta']['script_exit']}",
        f"wall: {bevy['meta']['wall_s']} s",
        "",
        "| case | before (cart driving, no ship control) | 1.2 capture | after (replay) | status |",
        "|---|---|---|---|---|",
    ]
    for case in CASES:
        lines.append(f"| {case} | {before[case]} | {native_text[case]} | "
                     f"{after_text[case]} | {checks[case]} |")
    lines += ["", "Checks:"]
    for case in CASES:
        n = native_cases[case]["moves"]
        a = details[case]["moves"]
        lines.append(
            f"- {case}: interval {a['interval_median_ms']} vs {n['interval_median_ms']} ms; "
            f"throttle {a['throttle']} vs {n['throttle']}; "
            f"steering {a['steering']} vs {n['steering']}; "
            f"release sample {a['release_decay']} vs {n['release_decay']} "
            f"(slope {a['release_rate']} vs {n['release_rate']} wire/s); flags "
            f"{a['flags']} vs {n['flags']}.")
    lines += [
        "",
        f"- type 3: native steady {native_type3['steady_wire_per_mps']} wire units per m/s "
        f"(n={native_type3['steady_n']}, {native_type3['pairs']} pairs); replay steady "
        f"{bevy_type3['steady_wire_per_mps']} (n={bevy_type3['steady_n']}, "
        f"{bevy_type3['pairs']} pairs). 30/32767 m/s = {WIRE_PER_MPS:.1f} wire units per m/s; "
        f"2048 would read 2048. {checks['type3_scale']}.",
        "",
        "Notes:",
        "- The lane runs `aaemu_game::net=trace` (not `net=debug`) so the S->C type-3",
        "  Debug fields are in rs-game.log; the C->S bodies come from the fields trace.",
        "- The native capture ran the hidden original at 22.5 FPS; the replay uses",
        "  --headless-fps 31.5 so the 100 ms gate fires every 4 frames, about 127 ms,",
        "  the native ship cadence.",
        "- Object ids and the absolute packet clock are normalised; the type-4 body is",
        "  decoded as time, flags, i8 throttle, i8 steering on both sides.",
        "- The release decay is one frame sample of the 6.0 axis/s ramp; its value depends",
        "  on where the release lands between sends, as in the native capture.",
        "- d_leave: the native sends a final 0/0 and stops type 4 at 0x032, resuming type 1;",
        "  the replay must do the same.",
        "",
    ]
    return record, "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(
        epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("SHIP_BEVY_CLIENT", "aa-client.exe"),
                        help="path to the Bevy aa-client.exe")
    parser.add_argument("--login", default=os.environ.get("SHIP_LOGIN",
                                                          os.environ.get("AA_LOGIN",
                                                                         "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("SHIP_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("SHIP_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("SHIP_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("SHIP_SERVER_LOG"),
                        help="game server log with the packet trace")
    parser.add_argument("--pak", default=os.environ.get("AA_PAK"), help="the game pak (AA_PAK)")
    parser.add_argument("--fps", type=float, default=float(os.environ.get("SHIP_FPS", "31.5")),
                        help="headless frame rate; 31.5 gives the capture's 127 ms gate")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "ship_steering"))
    parser.add_argument("--raw", default=os.environ.get("SHIP_RAW")
                        or str(ROOT / "data" / "extract"
                               / f"ship-steering-replay-{time.strftime('%Y%m%d')}-r1"))
    parser.add_argument("--timeout", type=float, default=420.0)
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set SHIP_{name.upper()})")
    client = Path(args.client)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    evidence = Path(args.evidence)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    if not client.exists() or not password_file.exists() or not server_log.exists():
        raise SystemExit("missing client, password file or server log")
    script = evidence / "replay.txt"
    if not script.exists():
        raise SystemExit(f"missing replay script: {script}")
    native = native_summary(json.loads(
        (evidence / "native.json").read_text(encoding="utf-8")))

    trace = Trace(server_log)
    trace.reset()
    packets: list[dict] = []
    stopping = threading.Event()

    def poll() -> None:
        while not stopping.wait(0.02):
            packets.extend(trace.poll())

    thread = threading.Thread(target=poll, daemon=True)
    thread.start()
    env = os.environ.copy()
    if args.pak:
        env["AA_PAK"] = args.pak
    command = [
        str(client), "--headless", "--mute", "--headless-fps", f"{args.fps:g}",
        "--account", args.account, "--login", args.login,
        "--password-file", str(password_file), "--character", args.character,
        "--script", str(script),
    ]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, encoding="utf-8", errors="replace", env=env, bufsize=1)
    reader = Lines(process.stdout)
    reader.start()
    try:
        process.wait(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    stopping.set()
    thread.join(timeout=2.0)
    packets.extend(trace.poll())
    reader.join(timeout=2.0)
    (raw / "bevy-replay.log").write_text(
        " ".join(command) + "\n\n" + "\n".join(reader.lines), encoding="utf-8")

    timed_lines = timed(reader.lines)
    markers = [(stamp, match.group(1)) for stamp, line in timed_lines
               if stamp is not None and (match := MARKER_RE.search(line))]
    by_case = assign(packets, markers)
    rows: list[dict] = []
    for case in CASES:
        for row in bevy_cases(by_case.get(case, [])):
            row["case"] = case
            rows.append(row)
    bind = None
    for packet in packets:
        if packet["dir"] == "C->S" and packet["opcode"] == 0x031:
            raw_body = bytes.fromhex(packet["body"].replace(" ", ""))
            bind = {"event": "packet", "case": "setup", "opcode": "0x031",
                    "len": packet["len"], "body": packet["body"],
                    "decoded": json.dumps({"tl_id": int.from_bytes(raw_body[:2], "little")})}
            break
    bevy_record = {
        "meta": {
            "account": "<account>", "character": "<character>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "client": "<client>",
            "server_log": "<server-log>", "wall_s": round(time.monotonic() - started, 1),
            "fps": args.fps, "script_exit": process.returncode,
            "script_pass": process.returncode == 0,
        },
        "bind": bind,
        "rows": rows,
    }
    record, report = build(native, bevy_record)
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    (evidence / "replay-report.txt").write_text(report, encoding="utf-8")
    print(report)
    return 0 if record["ok"] and process.returncode == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
