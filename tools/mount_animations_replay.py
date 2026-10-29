#!/usr/bin/env python3
"""Replay the native S3 rider placement/dismount cases in the Bevy client.

Runs tools/evidence/mount_animations/replay.txt with `aa-client --headless
--mute` against the same lane, polls the server's `C->S` trace while it runs,
and compares every `CSMountMate` 0x0a7 body (left/right/behind/front) and the
`CSUnMountMate` 0x0a8 body (reason 9) byte for byte with the original 1.2
capture in tools/evidence/mount_animations/native.json.  The mate tl id is a
server-assigned runtime id (80 in the 1.2 capture, 78 on this lane), so it is
normalised to 00 00; the attach point and reason bytes are compared exactly.

The client's `aa_client::ingame::mount` debug line names the rider unit, phase,
clip and bound `.caf` on every change; the replay assigns those lines to the
case windows and records the get-on, gait and get-off sequences next to the
`player_full_body.xml` states and lines from docs/aa-client-units.md.  The walk
gait is not headlessly drivable (the script runner holds W at the run speed);
the report says so instead of inventing a walk sample.

Usage:
    python tools/mount_animations_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --evidence tools/evidence/mount_animations
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mount_animations_probe import Trace, decode_c2s

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    ("a_mount_left", 0x0A7),
    ("a_mount_right", 0x0A7),
    ("a_mount_behind", 0x0A7),
    ("a_mount_front", 0x0A7),
    ("c_dismount", 0x0A8),
]
GAIT_CASES = ["b_stand", "b_walk", "b_run"]
CASES_ALL = [c for c, _ in CASES] + GAIT_CASES
MARKER_RE = re.compile(r"CASE:([A-Za-z0-9_]+)")
RIDER_RE = re.compile(r"aa_client::ingame::mount: .*(rides|riding|seat of|got off)", re.IGNORECASE)
RIDER_CLIP_RE = re.compile(
    r"aa_client::ingame::mount: rider (\d+) phase (\w+) clip (\w+) file (\S+)")
MOVE_RE = re.compile(r"move: \(.*speed ([0-9.]+)")
# The 1.2 player graph (player_full_body.xml) as docs/aa-client-units.md names it.
GRAPH = {
    "a_mount_left": "Horse_Enter_l :1517 -> fist_ac_horse_l_geton; Horse_DriverIdle :1464",
    "a_mount_right": "Horse_Enter_r :1521 -> fist_ac_horse_r_geton; Horse_DriverIdle :1464",
    "a_mount_behind": "Horse_Enter_b :1509 -> fist_ac_horse_b_geton; Horse_DriverIdle :1464",
    "a_mount_front": "Horse_Enter_f :1513 -> fist_ac_horse_f_geton; Horse_DriverIdle :1464",
    "b_stand": "Horse_DriverIdle :1464 -> horse_ba_relaxed_idle",
    "b_walk": ("walk Horse_DriverWalk :1490 (not driven); the short W leg starts Horse_DriverIdleToRun "
               ":1473 -> _FIST_MO_RIDER_HORSE_IDLETORUN :1474 and falls back to Horse_DriverIdle :1464"),
    "b_run": ("Horse_DriverIdleToRun :1473 -> _FIST_MO_RIDER_HORSE_IDLETORUN :1474; Horse_DriverRun "
              ":1479 -> _FIST_MO_RIDER_HORSE_RUN -> horse_mo_normal_run_f; on stop Horse_DriverRunToIdle "
              ":1487 -> horse_mo_relaxed_runtoidle_f"),
    "c_dismount": ("jump_horse_start :11742 -> fist_ac_horse_l_getoff_start; "
                   "jump_horse_loop :11736; jump_horse_end :11733"),
}
EXPECTED = {
    "a_mount_left": "fist_ac_horse_l_geton.caf",
    "a_mount_right": "fist_ac_horse_r_geton.caf",
    "a_mount_behind": "fist_ac_horse_b_geton.caf",
    "a_mount_front": "fist_ac_horse_f_geton.caf",
    "b_stand": "horse_ba_relaxed_idle.caf",
    "b_run": "horse_mo_normal_run_f.caf",
}
GET_OFF = ["fist_ac_horse_l_getoff_start.caf", "fist_ac_horse_l_getoff_loop.caf",
           "fist_ac_horse_l_getoff_end.caf"]
BEFORE = {
    "a_mount_left": "rider sat in the speed-band loops only; no get-on clip, reason not kept",
    "a_mount_right": "rider sat in the speed-band loops only; no get-on clip, reason not kept",
    "a_mount_behind": "rider sat in the speed-band loops only; no get-on clip, reason not kept",
    "a_mount_front": "rider sat in the speed-band loops only; no get-on clip, reason not kept",
    "b_stand": "idle loop (speed < 0.3 m/s)",
    "b_walk": "walk loop below 3 m/s; no idle-to-run one-shot",
    "b_run": "run loop above 3 m/s; no idle-to-run one-shot",
    "c_dismount": "unseated at once and walked; no get-off chain",
}
NATIVE = {
    "a_mount_left": "CSMountMate 50 00 01 01; SCUnitAttached 14 bytes (reason byte not logged)",
    "a_mount_right": "CSMountMate 50 00 01 02; SCUnitAttached 14 bytes (reason byte not logged)",
    "a_mount_behind": "CSMountMate 50 00 01 03; SCUnitAttached 14 bytes (reason byte not logged)",
    "a_mount_front": "CSMountMate 50 00 01 04; SCUnitAttached 14 bytes (reason byte not logged)",
    "b_stand": "idle: 100.22 z, seat height",
    "b_walk": "W 0.9 s: 15 CSMoveUnit; mean 1.25 m/s (max 2.61)",
    "b_run": "W 2.6 s: 28 CSMoveUnit; mean 1.42 m/s (max 2.50)",
    "c_dismount": "CSUnMountMate 50 00 01 09; SCUnitDetached 10 bytes, 2063 ms (old timing, upper bound)",
}


def hex_bytes(body: str) -> bytes:
    return bytes.fromhex(body.replace(" ", ""))


def normalize(body: str) -> bytes:
    raw = bytearray(hex_bytes(body))
    raw[0:2] = b"\x00\x00"
    return bytes(raw)


def norm_hex(body: str) -> str:
    return normalize(body).hex(" ")


def fields(body: str, opcode: int) -> dict:
    name = "CSMountMatePacket" if opcode == 0x0A7 else "CSUnMountMatePacket"
    return decode_c2s(name, body)


def describe(body: str, opcode: int) -> str:
    f = fields(body, opcode)
    return f"{name(opcode)} tl {f.get('tl_id')} ap {f.get('ap')} reason {f.get('reason')}"


def name(opcode: int) -> str:
    return "CSMountMate" if opcode == 0x0A7 else "CSUnMountMate"


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("MOUNT_BEVY_CLIENT", "aa-client.exe"),
                        help="path to the Bevy aa-client.exe")
    parser.add_argument("--login", default=os.environ.get("MOUNT_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("MOUNT_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("MOUNT_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("MOUNT_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("MOUNT_SERVER_LOG"),
                        help="game server log with the C->S trace")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "mount_animations"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-animations-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for field in ("account", "character", "password_file", "server_log"):
        if not getattr(args, field):
            parser.error(f"--{field.replace('_', '-')} is required (or set MOUNT_{field.upper()})")
    client = Path(args.client)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    evidence = Path(args.evidence)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    if not client.exists() or not password_file.exists() or not server_log.exists():
        raise SystemExit(f"missing: {client} {password_file} {server_log}")
    script = evidence / "replay.txt"
    if not script.exists():
        raise SystemExit(f"missing replay script: {script}")
    native = json.loads((evidence / "native.json").read_text(encoding="utf-8"))
    native_cases: dict[str, str] = {}
    for row in native["rows"]:
        for case, opcode in CASES:
            if (row.get("case") == case and row.get("dir") == "C->S"
                    and row.get("opcode") == f"0x{opcode:03X}" and row.get("body")):
                native_cases[case] = row["body"].strip()

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
    env["AA_MOVE_TRACE"] = "1"
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
    clips: list[tuple[float, dict]] = []

    def read_output() -> None:
        for line in process.stdout:
            line = line.rstrip("\n")
            lines.append(line)
            match = MARKER_RE.search(line)
            if match:
                markers.append((time.monotonic(), match.group(1)))
            clip = RIDER_CLIP_RE.search(line)
            if clip:
                clips.append((time.monotonic(), {
                    "unit": int(clip.group(1)), "phase": clip.group(2),
                    "clip": clip.group(3), "file": clip.group(4), "line": line,
                }))

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
        " ".join(command) + "\n\n" + "\n".join(lines) + "\n", encoding="utf-8")

    markers.sort()
    windows: dict[str, tuple[float, float | None]] = {}
    for i, (at, case) in enumerate(markers):
        windows[case] = (at, markers[i + 1][0] if i + 1 < len(markers) else None)
    wire = [p for p in packets if p.get("dir") == "C->S" and p.get("opcode") in (0x0A7, 0x0A8)]

    results: dict[str, dict] = {}
    for case, opcode in CASES:
        native_body = native_cases.get(case)
        window = windows.get(case)
        if window is None:
            bevy: list[dict] = []
            why = "case marker missing from the replay output"
        else:
            begin, end = window
            bevy = [p for p in wire
                    if p["opcode"] == opcode and p["t"] >= begin and (end is None or p["t"] < end)]
            why = ""
        record: dict = {
            "opcode": f"0x{opcode:03X}",
            "native": None,
            "bevy": [
                {"body": p["body"].strip(), "fields": fields(p["body"], opcode), "line": p.get("line", "")}
                for p in bevy
            ],
            "result": "MATCH",
            "why": why,
        }
        if native_body is not None:
            record["native"] = {"body": native_body, "fields": fields(native_body, opcode),
                                "normalized": norm_hex(native_body)}
        if why:
            record["result"] = "DIFFERS"
        elif native_body is None:
            if bevy:
                record["result"] = "DIFFERS"
                record["why"] = f"native sent no 0x{opcode:03x}, Bevy sent {len(bevy)}"
        elif len(bevy) != 1:
            record["result"] = "DIFFERS"
            record["why"] = f"native sent one 0x{opcode:03x}, Bevy sent {len(bevy)}"
        elif normalize(native_body) != normalize(bevy[0]["body"]):
            record["result"] = "DIFFERS"
            record["why"] = f"normalised native {norm_hex(native_body)} != Bevy {norm_hex(bevy[0]['body'])}"
        results[case] = record
        if record["bevy"]:
            record["normalized"] = norm_hex(bevy[0]["body"])

    base_t = markers[0][0] if markers else started
    idle_file = "horse_ba_relaxed_idle.caf"
    idle_to_run_file = "horse_mo_relaxed_idletorun_f.caf"
    run_file = "horse_mo_normal_run_f.caf"

    def at_seconds(t: float) -> float:
        return round(t - base_t, 2)

    def describe_clip(c: dict) -> str:
        return f"{c['phase']} {c['clip']} {c['file']}"

    rider_clips: dict[str, dict] = {}
    for case in CASES_ALL:
        window = windows.get(case)
        if window is None:
            rider_clips[case] = {"sequence": [], "expected": GRAPH[case],
                                 "result": "N/A", "why": "case marker missing"}
            continue
        begin, end = window
        inside = [(t, c) for t, c in clips
                  if t >= begin and (end is None or t < end)]
        before = [c for t, c in clips if t < begin]
        seq = [dict(c, at_s=at_seconds(t)) for t, c in inside]
        current = seq[-1] if seq else (before[-1] if before else None)
        unit = next((c["unit"] for c in seq), None)
        record = {
            "unit": unit,
            "sequence": seq,
            "state_at_start": (dict(before[-1], at_s=at_seconds(begin)) if before else None),
            "expected": GRAPH[case],
            "result": "MATCH",
            "why": "",
        }
        if case.startswith("a_mount_"):
            get_on = next((c for c in seq if c["clip"] == "GetOn"), None)
            idle = next((c for c in seq if c["clip"] == "Idle"), None)
            off = [c["file"] for c in seq if c["clip"].startswith("GetOff")]
            record["get_on"] = get_on
            record["get_off"] = off
            if get_on is None or get_on["file"] != EXPECTED[case]:
                record["result"] = "DIFFERS"
                record["why"] = (f"get-on {get_on['file'] if get_on else 'missing'}, "
                                 f"graph {EXPECTED[case]}")
            elif idle is None or idle["file"] != idle_file:
                record["result"] = "DIFFERS"
                record["why"] = f"idle {idle['file'] if idle else 'missing'}, graph {idle_file}"
            elif case != "a_mount_front" and off != GET_OFF:
                record["result"] = "DIFFERS"
                record["why"] = f"get-off {off}, graph {GET_OFF}"
        elif case == "c_dismount":
            off = [c["file"] for c in seq if c["clip"].startswith("GetOff")]
            record["get_off"] = off
            if off != GET_OFF:
                record["result"] = "DIFFERS"
                record["why"] = f"get-off {off}, graph {GET_OFF}"
        elif case == "b_stand":
            if not current or current["file"] != idle_file:
                record["result"] = "DIFFERS"
                record["why"] = f"idle {current['file'] if current else 'missing'}, graph {idle_file}"
        elif case == "b_run":
            run_to_idle_file = "horse_mo_relaxed_runtoidle_f.caf"
            idletorun = next((c for c in seq if c["clip"] == "IdleToRun"), None)
            run = next((c for c in seq if c["clip"] == "Run"), None)
            run_to_idle = next((c for c in seq if c["clip"] == "RunToIdle"), None)
            record["idle_to_run"] = idletorun
            record["run"] = run
            record["run_to_idle"] = run_to_idle
            # the transitions are the feature under test: all three must play, in this order
            order = [c["clip"] for c in seq if c["clip"] in ("IdleToRun", "Run", "RunToIdle")]
            wanted = ["IdleToRun", "Run", "RunToIdle"]
            if order[:len(wanted)] != wanted:
                record["result"] = "DIFFERS"
                record["why"] = (f"expected {', '.join(wanted)} in that order; "
                                 f"saw {', '.join(order) or 'none'}")
            elif idletorun["file"] != idle_to_run_file:
                record["result"] = "DIFFERS"
                record["why"] = (f"idle-to-run {idletorun['file']}, graph {idle_to_run_file}")
            elif run["file"] != run_file:
                record["result"] = "DIFFERS"
                record["why"] = f"run {run['file']}, graph {run_file}"
            elif run_to_idle["file"] != run_to_idle_file:
                record["result"] = "DIFFERS"
                record["why"] = (f"run-to-idle {run_to_idle['file']}, graph {run_to_idle_file}")
        elif case == "b_walk":
            idletorun = next((c for c in seq if c["clip"] == "IdleToRun"), None)
            record["result"] = "N/A"
            if idletorun is None:
                record["why"] = "walk gait not driven; the idle-to-run one-shot was not observed"
            elif idletorun["file"] != idle_to_run_file:
                record["result"] = "DIFFERS"
                record["why"] = (f"idle-to-run {idletorun['file']}, graph {idle_to_run_file}")
            else:
                record["why"] = ("walk gait not driven; the short W leg shows the idle-to-run "
                                 "one-shot then idle")
        rider_clips[case] = record

    clip_ok = all(
        r["result"] == "MATCH" or (case == "b_walk" and r["result"] == "N/A")
        for case, r in rider_clips.items()
    )

    bevy_tl = None
    for case, opcode in CASES:
        if results[case]["bevy"]:
            bevy_tl = results[case]["bevy"][0]["fields"].get("tl_id")
            break
    matched = sum(1 for r in results.values() if r["result"] == "MATCH")
    checks = {
        "script_exit": process.returncode,
        "script_pass": process.returncode == 0,
        "cases": len(CASES),
        "matched": matched,
        "ok": process.returncode == 0 and matched == len(CASES) and clip_ok,
        "bevy_pid": process.pid,
        "mate_tl_id": bevy_tl,
        "comparison": (f"mate tl id {bevy_tl} (bytes 0..2, server-assigned) -> 00 00; "
                       "attach point and reason bytes compared exactly"),
        "rider_clips_ok": clip_ok,
        "rider_clips": {case: r["result"] for case, r in rider_clips.items()},
    }
    speeds = [float(m.group(1)) for line in lines if (m := MOVE_RE.search(line))]
    max_speed = max(speeds) if speeds else None
    rider_lines = [line for line in lines if RIDER_RE.search(line)]
    clip_lines = [line for line in lines if RIDER_CLIP_RE.search(line)]
    move_lines = [line for line in lines if MOVE_RE.search(line)]
    rider = {
        "clip_names_logged": bool(clip_lines),
        "note": ("aa-client/src/ingame/mount.rs logs 'rider <unit> phase <phase> clip <clip> file <caf>' "
                 "once per phase/clip change; the sequences below are compared with player_full_body.xml "
                 "as docs/aa-client-units.md names it. No 1.2 clip names were captured (the driver had no "
                 "animation accessor)."),
        "attach_lines": rider_lines,
        "clip_lines": clip_lines,
        "rider_clips": rider_clips,
        "move_trace": move_lines,
        "max_move_speed": max_speed,
    }
    record = {
        "meta": {
            "account": "<account>", "character": "<character>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "client": "<client>",
            "server_log": "<server-log>", "wall_s": wall,
            "bevy_pid": process.pid, "mate_tl_id": bevy_tl,
            "rust_log": env["RUST_LOG"], "aa_move_trace": True,
        },
        "checks": checks,
        "cases": results,
        "rider_animation": rider,
        "packets": [
            {key: p.get(key) for key in ("dir", "opcode", "name", "len", "body", "line")}
            for p in packets if p.get("dir") == "C->S" and p.get("opcode") in (0x0A7, 0x0A8)
        ],
    }
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")

    def clip_file(case: str, clip: str) -> str | None:
        for c in rider_clips[case]["sequence"]:
            if c["clip"] == clip:
                return c["file"]
        return None

    def after(case: str) -> str:
        r = results.get(case)
        if case == "b_stand":
            file = clip_file(case, "Idle") or (rider_clips[case]["state_at_start"] or {}).get("file")
            return f"idle {file} (log)" if file else "idle not observed"
        if case == "b_walk":
            trans = clip_file(case, "IdleToRun")
            return (f"walk not driven; short W leg {trans} -> idle"
                    if trans else "walk not driven; idle-to-run not observed")
        if case == "b_run":
            file = clip_file(case, "Run") or (rider_clips[case]["state_at_start"] or {}).get("file")
            return f"run {file} (log)" if file else "run not observed"
        if case.startswith("a_mount_"):
            if r and r["bevy"]:
                f = r["bevy"][0]["fields"]
                off = rider_clips[case].get("get_off", [])
                tail = ("stays mounted" if case == "a_mount_front" else f"get-off {len(off)} clips")
                return (f"reason {f.get('reason')} matches; get-on {clip_file(case, 'GetOn')}; "
                        f"idle {clip_file(case, 'Idle')}; {tail}")
            return "no packet"
        if case == "c_dismount":
            if r and r["bevy"]:
                return ("reason 9 matches; get-off " +
                        " -> ".join(rider_clips[case].get("get_off", [])))
            return "no packet"
        return "no packet"

    lines_out = [
        "## Bevy replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  account: <account> / <character>",
        f"client: <client>  (--headless --mute)  pid: {process.pid} (started and stopped by the replay)",
        f"script: <evidence>/{script.name}  exit: {process.returncode}",
        f"wall: {wall} s",
        "",
        "Wire comparison: CSMountMate 0x0a7 and CSUnMountMate 0x0a8 with native.json. The mate tl id",
        f"is a server-assigned runtime id (80 in the 1.2 capture, {bevy_tl} here), so bytes 0..2 are",
        "normalised to 00 00 and the remaining bytes (attach point, reason) are compared exactly.",
        "",
        "case                   1.2 native body                       Bevy body                             result",
    ]
    for case, opcode in CASES:
        r = results[case]
        native_text = r["native"]["body"] if r["native"] else "no packet"
        bevy_text = r["bevy"][0]["body"] if r["bevy"] else "no packet"
        lines_out.append(f"{case:23s} {native_text:37s} {bevy_text:37s} {r['result']}")
        if r["result"] != "MATCH":
            lines_out.append(f"    {r['why']}")
    lines_out += [
        "",
        "Before/after (before = base 45f80700; 1.2 = capture; after = this replay):",
        "",
        "| case | before 45f80700 | 1.2 capture | after (replay) |",
        "|---|---|---|---|",
    ]
    for case in [c for c, _ in CASES] + ["b_stand", "b_walk", "b_run"]:
        lines_out.append(f"| {case} | {BEFORE[case]} | {NATIVE[case]} | {after(case)} |")
    def sequence_text(case: str) -> str:
        seq = rider_clips[case]["sequence"]
        if seq:
            return "; ".join(describe_clip(c) for c in seq)
        state = rider_clips[case]["state_at_start"]
        return f"(unchanged) {describe_clip(state)}" if state else "no clip line"

    lines_out += [
        "",
        "Rider clips (new debug! line; RUST_LOG=info,aa_client::ingame::mount=debug; one line per",
        "phase/clip change, never per frame):",
        "",
        "| case | Bevy clip sequence | 1.2 graph prescribes (docs/aa-client-units.md) | result |",
        "|---|---|---|---|",
    ]
    for case in CASES_ALL:
        record = rider_clips[case]
        result = record["result"]
        if record["why"] and result != "MATCH":
            result += f" ({record['why']})"
        lines_out.append(f"| {case} | {sequence_text(case)} | {GRAPH[case]} | {result} |")
    lines_out += [
        "",
        "Walk: N/A. The script runner's `walk` holds W at the mount's run speed; the walk state",
        "Horse_DriverWalk :1490 was not exercised headlessly. The b_walk window is the short W leg:",
        "it reaches Horse_DriverIdleToRun :1473 only, then falls back to idle; the b_run window",
        "reaches Horse_DriverRun :1479. The 1.2 capture itself has no clip names (the driver had no",
        "animation accessor); this section compares Bevy's implementation with the graph the 1.2",
        "client plays.",
        "",
    ]
    if rider_lines:
        lines_out.append(f"Observed attach line: {rider_lines[0]}")
    lines_out += [
        f"Observed mounted move speed (AA_MOVE_TRACE): max {max_speed} m/s (mount run 9.9 m/s).",
        "",
        "Approaches are ~2 m from the horse; the 1.2 capture walked to 4.1-4.5 m (the get-on reason is",
        "distance-invariant). The remote-rider case stays skipped (1.2 could not capture it with one",
        "client).",
        "",
        f"checks: {checks}",
        "",
    ]
    section = "\n".join(lines_out)
    (evidence / "replay-report.txt").write_text(section, encoding="utf-8")
    print(section)
    return 0 if checks["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
