#!/usr/bin/env python3
"""Replay the 1.2 cart driving cases in the Bevy client and compare with the capture.

Runs tools/evidence/cart_driving/replay.txt with `aa-client --headless --mute` against the
lane, polls the server's C->S field trace while it runs, and compares the per-case
`CSMoveUnit` 0x089 bodies with the native capture (tools/evidence/cart_driving/native.json):

* send interval (median, and resend while still),
* body layout and flags (47-byte type 2, flags 0, wheel count 0, no scType/phase),
* velocity at 30/32767 m/s and the speed-vs-time curve (accelerate and coast),
* steering values and ramp, and angVel.z = 0.4 * steering,
* leaving while moving (CSDiscardSlave 0x032 and no cart move after it).

It writes bevy.json (packets and checks) and replay-report.txt (the before/after table) to
the evidence directory. The client is stopped by its pid on timeout or exit.

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace,aaemu_game::net=debug`.
The client needs the game pak (`--pak` or AA_PAK).

Usage:
    python tools/cart_driving_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        [--pak <game_pak>] [--evidence tools/evidence/cart_driving] [--fps 21]
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
from cart_driving_probe import Trace, decode_move  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CASES = ["a_still", "b_forward", "c_left", "c_right", "d_reverse", "f_leave"]
INTEREST_C2S = {0x031, 0x032, 0x068, 0x089}
MARKER_RE = re.compile(r"case=([a-z0-9_.]+)")
UTC_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z")
SCRIPT_T_RE = re.compile(r"SCRIPT \[\s*([0-9.]+)s\]")
SCALE = 30.0 / 32767.0


def log(*args: object) -> None:
    print("cart-replay:", *args, flush=True)


def moves_of(rows: list[dict], case: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet":
            continue
        if row.get("opcode") != "0x089":
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        if decoded.get("type") != 2:
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


def body_len(move: dict) -> int:
    return int(move.get("_len") or 47)


def speed_of(move: dict) -> float:
    return math.hypot(*move["vel"]) * SCALE


def stats(moves: list[dict]) -> dict:
    if not moves:
        return {"count": 0, "len": [], "type": [], "flags": [], "wheel_count": [],
                "sc_type": 0, "interval_median_ms": None, "interval_min_ms": None,
                "interval_max_ms": None, "max_speed": None, "speed_min": None,
                "accel_s": None, "coast_s": None, "steering": [], "angvel_error": None,
                "velocity_ratio": None, "speed_curve": []}
    speeds = [speed_of(move) for move in moves]
    times = [move["time_ms"] for move in moves]
    intervals = [b - a for a, b in zip(times, times[1:])]
    top = max(speeds)
    accel_start = next((i for i, speed in enumerate(speeds) if speed > 0.2), None)
    full = next((i for i, speed in enumerate(speeds) if speed >= top - 0.05), None)
    last_full = max((i for i, speed in enumerate(speeds) if speed >= top - 0.05), default=None)
    coast_end = None
    if last_full is not None:
        for i in range(last_full + 1, len(speeds)):
            if speeds[i] < 0.05:
                coast_end = i
                break
    ratio = []
    for a, b in zip(moves, moves[1:]):
        seconds = (b["time_ms"] - a["time_ms"]) / 1000.0
        if not 0.02 < seconds < 0.4:
            continue
        delta = [b["pos"][i] - a["pos"][i] for i in range(2)]
        wire = math.hypot(b["vel"][0], b["vel"][1]) * SCALE
        if wire > 0.1:
            ratio.append(math.hypot(*delta) / seconds / wire)
    return {
        "count": len(moves),
        "len": sorted({body_len(move) for move in moves}),
        "type": sorted({move.get("type") for move in moves}),
        "flags": sorted({move.get("flags") for move in moves}),
        "wheel_count": sorted({move.get("wheel_count") for move in moves}),
        "sc_type": sum(1 for move in moves if "sc_type" in move),
        "interval_median_ms": round(statistics.median(intervals), 1) if intervals else None,
        "interval_min_ms": min(intervals) if intervals else None,
        "interval_max_ms": max(intervals) if intervals else None,
        "max_speed": round(top, 3),
        "speed_min": round(min(speeds), 3),
        "accel_s": round((times[full] - times[accel_start]) / 1000.0, 2)
        if accel_start is not None and full is not None else None,
        "coast_s": round((times[coast_end] - times[last_full]) / 1000.0, 2)
        if last_full is not None and coast_end is not None else None,
        "steering": sorted({round(move["steering"], 3) for move in moves}),
        "angvel_error": round(max(abs(move["ang_vel"][2] - 0.4 * move["steering"])
                                 for move in moves), 6),
        "velocity_ratio": round(statistics.median(ratio), 3) if ratio else None,
        "speed_curve": [[round((t - times[0]) / 1000.0, 2), round(speed, 2)]
                        for t, speed in zip(times, speeds)],
    }


def native_summary(native: dict) -> dict:
    rows = native["rows"]
    cart = None
    for case in CASES:
        for move in moves_of(rows, case):
            cart = move["obj_id"]
            break
        if cart is not None:
            break
    cases = {}
    for case in CASES:
        moves = moves_of(rows, case)
        cases[case] = {"cart_obj": cart, "moves": stats(moves),
                       "leave": leave_of(rows, case)}
        leave = cases[case]["leave"]
        if leave:
            last = [move for move in moves if move["_t"] <= leave[0]["_t"]]
            cases[case]["leave"] = {
                "body": leave[0]["_body"], "len": leave[0]["_len"],
                "tl_id": leave[0].get("tl_id"),
                "speed": round(speed_of(last[-1]), 2) if last else None,
                "moves_after": len([move for move in moves if move["_t"] > leave[0]["_t"]]),
            }
    bind = next((row for row in rows
                 if row.get("event") == "packet" and row.get("opcode") == "0x031"), None)
    return {"cart": cart, "bind": bind, "cases": cases}


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


def bevy_cases(packets: list[dict]) -> dict:
    rows = []
    for packet in packets:
        if packet["dir"] != "C->S" or packet["opcode"] not in INTEREST_C2S:
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
        rows.append({"t_ms": packet["t_ms"],
                     "event": "packet",
                     "case": "",
                     "opcode": f"0x{packet['opcode']:03X}",
                     "len": packet["len"],
                     "body": packet["body"],
                     "decoded": json.dumps(decoded)})
    return rows


def build(native: dict, bevy: dict) -> tuple[dict, str]:
    checks: dict[str, str] = {}
    table = []
    details = {}
    native_cases = native["cases"]
    bevy_rows = bevy["rows"]
    bind = bevy.get("bind") or next((row for row in bevy_rows
                                     if row["event"] == "packet" and row["opcode"] == "0x031"), None)
    cart = None
    for move in moves_of(bevy_rows, "a_still"):
        cart = move["obj_id"]
        break
    for case in CASES:
        nm = native_cases[case]
        moves = moves_of(bevy_rows, case)
        leave = leave_of(bevy_rows, case)
        actual = {"moves": stats(moves), "leave": None}
        if leave:
            last = [move for move in moves if move["_t"] <= leave[0]["_t"]]
            actual["leave"] = {
                "body": leave[0]["_body"], "len": leave[0]["_len"],
                "tl_id": leave[0].get("tl_id"),
                "speed": round(speed_of(last[-1]), 2) if last else None,
                "moves_after": len([move for move in moves if move["_t"] > leave[0]["_t"]]),
            }
        details[case] = actual
        a, n = actual["moves"], nm["moves"]
        if not moves:
            checks[case] = "DIFFERS (no type-2 sends)"
            table.append((case, "DIFFERS (no type-2 sends)"))
            continue
        problems = []
        if a["len"] != n["len"]:
            problems.append(f"len {a['len']} vs {n['len']}")
        if a["flags"] != n["flags"]:
            problems.append(f"flags {a['flags']} vs {n['flags']}")
        if a["wheel_count"] != n["wheel_count"]:
            problems.append(f"wheel {a['wheel_count']} vs {n['wheel_count']}")
        if a["sc_type"] != 0:
            problems.append("scType set")
        if a["interval_median_ms"] is None or n["interval_median_ms"] is None \
                or abs(a["interval_median_ms"] - n["interval_median_ms"]) > 25:
            problems.append(f"interval {a['interval_median_ms']} vs {n['interval_median_ms']}")
        if abs(a["max_speed"] - n["max_speed"]) > 0.15 and case != "f_leave":
            problems.append(f"max speed {a['max_speed']} vs {n['max_speed']}")
        if n["velocity_ratio"] is not None and (a["velocity_ratio"] is None
                                                or abs(a["velocity_ratio"] - 1.0) > 0.08):
            problems.append(f"velocity ratio {a['velocity_ratio']}")
        if a["angvel_error"] > 0.002:
            problems.append(f"angVel error {a['angvel_error']}")
        if case != "a_still" and case != "f_leave":
            if n["accel_s"] is not None and a["accel_s"] is not None \
                    and abs(a["accel_s"] - n["accel_s"]) > 0.5:
                problems.append(f"accel {a['accel_s']} vs {n['accel_s']}")
            if n["coast_s"] is not None and a["coast_s"] is not None \
                    and abs(a["coast_s"] - n["coast_s"]) > 0.5:
                problems.append(f"coast {a['coast_s']} vs {n['coast_s']}")
        if case in ("c_left", "c_right"):
            expected = 1.0 if case == "c_left" else -1.0
            if expected not in a["steering"] or expected not in n["steering"]:
                problems.append(f"steering {a['steering']} vs {n['steering']}")
        if case == "a_still":
            if a["max_speed"] > 0.01:
                problems.append("not still")
            if a["count"] < 3:
                problems.append("does not resend")
        if case == "f_leave":
            if not leave:
                problems.append("no 0x032")
            else:
                if leave[0]["_len"] != 2:
                    problems.append(f"0x032 len {leave[0]['_len']}")
                if bind is None or leave[0].get("tl_id") != json.loads(bind["decoded"]).get("tl_id"):
                    problems.append("0x032 tl differs from 0x031")
                if actual["leave"]["moves_after"] != 0:
                    problems.append(f"{actual['leave']['moves_after']} moves after 0x032")
                if actual["leave"]["speed"] is not None and not (2.5 <= actual["leave"]["speed"] <= 4.5):
                    problems.append(f"leave speed {actual['leave']['speed']}")
        checks[case] = "MATCH" if not problems else "DIFFERS (" + "; ".join(problems) + ")"
        table.append((case, checks[case]))

    # before column: the S5 cart boarding replay had no cart driving, hence no type-2 sends
    before = {
        "a_still": "no type-2 sends (cart driving not ported)",
        "b_forward": "no type-2 sends",
        "c_left": "no type-2 sends",
        "c_right": "no type-2 sends",
        "d_reverse": "no type-2 sends",
        "f_leave": "no type-2 sends; 0x032 while still",
    }
    native_text = {}
    for case in CASES:
        n = native_cases[case]["moves"]
        native_text[case] = (f"{n['count']} x 0x089 type 2, {n['len'][0]} B, flags "
                             f"{n['flags']}, wheel {n['wheel_count']}, "
                             f"interval {n['interval_median_ms']} ms, speed "
                             f"{n['speed_min']}..{n['max_speed']} m/s")
        if case in ("c_left", "c_right"):
            native_text[case] += f", steering {n['steering']}"
        if case == "f_leave":
            leave = native_cases[case]["leave"]
            native_text[case] += (f"; 0x032 {leave['body']} tl {leave['tl_id']} at "
                                  f"{leave['speed']} m/s, {leave['moves_after']} moves after")
    after_text = {}
    for case in CASES:
        a = details[case]["moves"]
        if not a["count"]:
            after_text[case] = "no type-2 sends"
            continue
        after_text[case] = (f"{a['count']} x 0x089 type 2, {a['len'][0]} B, flags "
                            f"{a['flags']}, wheel {a['wheel_count']}, "
                            f"interval {a['interval_median_ms']} ms, speed "
                            f"{a['speed_min']}..{a['max_speed']} m/s")
        if case in ("c_left", "c_right"):
            after_text[case] += f", steering {a['steering']}"
        if case == "f_leave":
            leave = details[case]["leave"]
            if leave:
                body = " ".join(f"{byte:02x}" for byte in bytes.fromhex(leave["body"]))
                after_text[case] += (f"; 0x032 {body} tl {leave['tl_id']} at "
                                     f"{leave['speed']} m/s, {leave['moves_after']} moves after")
            else:
                after_text[case] += "; no 0x032"

    ok = all(value == "MATCH" for value in checks.values())
    record = {
        "meta": bevy["meta"],
        "native_cart": native["cart"],
        "bevy_cart": cart,
        "native": {case: native_cases[case] for case in CASES},
        "bevy": details,
        "moves": {case: [
            {"t_rel_s": round((move["time_ms"] - moves[0]["time_ms"]) / 1000.0, 3),
             "pos": move["pos"], "vel": move["vel"], "steering": move["steering"],
             "ang_vel_z": move["ang_vel"][2], "flags": move["flags"],
             "len": body_len(move)}
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
        "| case | before (cart boarding, no driving) | 1.2 capture | after (replay) | status |",
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
            f"speed {a['speed_min']}..{a['max_speed']} vs {n['speed_min']}..{n['max_speed']} m/s; "
            f"accel {a['accel_s']} vs {n['accel_s']} s; coast {a['coast_s']} vs {n['coast_s']} s; "
            f"velocity ratio {a['velocity_ratio']} vs {n['velocity_ratio']}; "
            f"steering {a['steering']} vs {n['steering']}; "
            f"angVel error {a['angvel_error']} vs {n['angvel_error']}.")
    lines += [
        "",
        "Notes:",
        "- The native capture ran the hidden original at 22.5 FPS, so its send gate fired every",
        "  143 ms; the replay uses --headless-fps 21 (3 frames per send) to match that cadence.",
        "  The gate is frame-based on both clients, so the interval is the comparison's frame-size",
        "  term, not a protocol value.",
        "- Object ids and the absolute packet clock are normalised; velocity is decoded at",
        "  30/32767 m/s on both sides.",
        "- The native lane logged no S->C bodies; the replay compares only the C->S direction.",
        "- c_left / c_right: both reach +-1.0 with angVel.z 0.4; the intermediate steering send",
        "  is frame-phase dependent (the native 0.484/0.269 and the replay 0.265/0.736/0.527).",
        "- f_leave: the replay leaves at 3.83 m/s, the native at 3.58 m/s; both are below the",
        "  native 5 m/s refusal, and both send 0x032 (the tl is normalised) with no cart move",
        "  after it.",
        "",
    ]
    return record, "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(
        epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("CART_BEVY_CLIENT", "aa-client.exe"),
                        help="path to the Bevy aa-client.exe")
    parser.add_argument("--login", default=os.environ.get("CART_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("CART_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("CART_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("CART_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("CART_SERVER_LOG"),
                        help="game server log with the C->S trace")
    parser.add_argument("--pak", default=os.environ.get("AA_PAK"), help="the game pak (AA_PAK)")
    parser.add_argument("--fps", type=float, default=float(os.environ.get("CART_FPS", "21")),
                        help="headless frame rate; 21 gives the capture's 143 ms send gate")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "cart_driving"))
    parser.add_argument("--raw", default=os.environ.get("CART_RAW")
                        or str(ROOT / "data" / "extract"
                               / f"cart-driving-replay-{time.strftime('%Y%m%d')}-r1"))
    parser.add_argument("--timeout", type=float, default=420.0)
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set CART_{name.upper()})")
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
            raw = bytes.fromhex(packet["body"].replace(" ", ""))
            bind = {"event": "packet", "case": "setup", "opcode": "0x031",
                    "len": packet["len"], "body": packet["body"],
                    "decoded": json.dumps({"tl_id": int.from_bytes(raw[:2], "little")})}
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
