#!/usr/bin/env python3
"""Replay the 1.2 rowboat deck standing in the Bevy client and compare with the capture.

Runs tools/evidence/ship_deck/replay.txt with `aa-client --headless --mute` against the
lane, polls the server's packet trace while it runs, and compares the per-case
`CSMoveUnit` 0x089 type-1 packets with the native deck capture
(tools/evidence/ship_deck/native-deck.json):

* the type-1 actor support block (gcFlags/gcPartId/gcId), header and actor flags,
* the ship-local primary position and the world auxiliary position,
* the relative facing (primary direction byte) and world facing (auxiliary rotation z),
* the send cadence, and
* the transition after leaving the deck (plain type 1 without the block).

The second half binds the hull as driver, sails it, leaves the helm and stands on the
coasting hull; the type-4 throttle and the 0x032 leave are checked on the way.

It writes bevy.json (packets and checks) and replay-report.txt (the before/after table)
to the evidence directory. The client is stopped by its pid on timeout or exit.

The game server must log the fields trace and the S->C trace structs:
`RUST_LOG=info,aaemu_game::net::fields=trace,aaemu_game::net=trace`.
The client needs the game pak (`--pak` or AA_PAK).

Usage:
    python tools/ship_deck_replay.py --client <aa-client.exe> \
        --login <host:port> --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        [--pak <game_pak>] [--evidence tools/evidence/ship_deck] [--fps 31.5]
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
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ship_steering_probe import Trace, decode_move  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CASES = ["still_idle", "still_turn", "still_walk", "still_jump", "off_walk",
         "off_after", "drive_forward", "drive_leave", "moving_idle", "moving_off",
         "after_off"]
NATIVE_REF = {
    "still_idle": "d_after",
    "still_turn": "d_after",
    "still_walk": "d_leave",
    "still_jump": "a_idle",
    "off_walk": "setup",
    "off_after": "setup",
    "drive_forward": "b_forward",
    "drive_leave": "d_leave",
    "moving_idle": "d_after",
    "moving_off": "setup",
    "after_off": "setup",
}
INTEREST_C2S = {0x031, 0x032, 0x089}
MARKER_RE = re.compile(r"case=([a-z0-9_.]+)")
UTC_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z")
SCRIPT_T_RE = re.compile(r"SCRIPT \[\s*([0-9.]+)s\]")
DIR_STEP = 2.0 * math.pi / 128.0


def log(*args: object) -> None:
    print("ship-deck-replay:", *args, flush=True)


def dir_byte(rad: float) -> int:
    value = round(rad / DIR_STEP) % 256
    return value - 256 if value > 127 else value


def dir_diff(a: int, b: int) -> int:
    value = (a - b) % 256
    return value - 256 if value > 127 else value


def moves_of(rows: list[dict], case: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet":
            continue
        if row.get("opcode") != "0x089":
            continue
        try:
            decoded = json.loads(row.get("decoded") or "{}")
        except json.JSONDecodeError:
            continue
        if decoded.get("type") != 1:
            continue
        decoded["_t"] = row["t_ms"]
        decoded["_len"] = row["len"]
        out.append(decoded)
    return out


def controls_of(rows: list[dict], case: str, opcode: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet" or row.get("opcode") != opcode:
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        decoded["_t"] = row["t_ms"]
        out.append(decoded)
    return out


def type4_of(rows: list[dict], case: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("event") != "packet" or row.get("opcode") != "0x089":
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        if decoded.get("type") != 4:
            continue
        decoded["_t"] = row["t_ms"]
        out.append(decoded)
    return out


def ship_samples(rows: list[dict], case: str) -> list[dict]:
    out = []
    for row in rows:
        if row.get("case") != case or row.get("name") != "SCOneUnitMovementPacket":
            continue
        decoded = json.loads(row.get("decoded") or "{}")
        if decoded.get("kind") != "Ship":
            continue
        if not all(key in decoded for key in ("x", "y", "z", "yaw_z")):
            continue
        out.append(decoded)
    return out


def median(values: list[float]) -> float | None:
    return round(statistics.median(values), 1) if values else None


def jsonable(value):
    if isinstance(value, dict):
        return {(key if isinstance(key, (str, int, float, bool)) or key is None else str(key)): jsonable(item)
                for key, item in value.items()}
    if isinstance(value, (list, tuple)):
        return [jsonable(item) for item in value]
    return value


def stats(moves: list[dict], ships: list[dict]) -> dict:
    support = [m for m in moves if m.get("actor_flags", 0) & 0x20]
    plain = [m for m in moves if not m.get("actor_flags", 0) & 0x20]
    header_only = [m for m in plain if m.get("flags", 0) & 0x40]
    times = [m["time_ms"] for m in support]
    gaps = [b - a for a, b in zip(times, times[1:])]
    local = [m.get("pos") for m in support if m.get("pos")]
    world = [m.get("pos2") for m in support if m.get("pos2")]
    ranges = {}
    if local:
        ranges["local"] = [round(max(p[i] for p in local) - min(p[i] for p in local), 3)
                           for i in range(3)]
    if world:
        ranges["world"] = [round(max(p[i] for p in world) - min(p[i] for p in world), 3)
                           for i in range(3)]
    last_support = support[-1]["_t"] if support else None
    after = [m for m in plain if last_support is not None and m["_t"] > last_support]
    residual = []
    distances = []
    for move in support:
        if not ships:
            continue
        ship = min(ships, key=lambda s: abs(s.get("time", 0) - move["time_ms"]))
        yaw = float(ship["yaw_z"])
        rel = move["rot"][2] * DIR_STEP
        residual.append(abs(dir_diff(dir_byte(yaw + rel), move["rot2"][2])))
        world_pos = move.get("pos2")
        if world_pos:
            distances.append(math.dist(world_pos, [ship["x"], ship["y"], ship["z"]]))
    return {
        "count": len(moves),
        "support": len(support),
        "plain": len(plain),
        "header_only": len(header_only),
        "flags": dict(Counter(m["flags"] for m in support)),
        "actors": dict(Counter(m["actor_flags"] for m in support)),
        "gc": dict(Counter((m.get("gc_flags"), m.get("gc_part_id"), m.get("gc_id"))
                           for m in support)),
        "cadence_median_ms": median(gaps),
        "cadence_min_ms": min(gaps) if gaps else None,
        "cadence_max_ms": max(gaps) if gaps else None,
        "ranges": ranges,
        "dirs": dict(Counter((m["rot"][2], m["rot2"][2]) for m in support)),
        "dir_residual_bytes": median(residual),
        "world_to_ship_m": median(distances),
        "plain_after": len(after),
        "plain_after_flags": dict(Counter(m["flags"] for m in after)),
        "plain_after_actors": dict(Counter(m["actor_flags"] for m in after)),
        "last_support_t": last_support,
        "tail": [{"t": m["_t"], "flags": m["flags"], "actor": m["actor_flags"],
                  "pos": m.get("pos")} for m in (support[-1:] + plain[:2])],
        "ship_ids": sorted({m["gc_id"] for m in support if m.get("gc_id")}),
    }


def native_summary(rows: list[dict]) -> dict:
    out = {}
    for ref in sorted(set(NATIVE_REF.values())):
        case_rows = [row for row in rows if row.get("case") == ref]
        moves = moves_of(case_rows, ref)
        out[ref] = stats(moves, [])
    return out


def a_idle_jump(rows: list[dict]) -> dict:
    moves = moves_of(rows, "a_idle")
    jump = [m for m in moves if m.get("actor_flags", 0) & 0x10]
    return {"n": len(jump),
            "flags": sorted({m["flags"] for m in jump}),
            "actors": sorted({m["actor_flags"] for m in jump})}


def set_text(values: dict) -> str:
    return "{" + ", ".join("0x%02x" % int(k) for k in sorted(values)) + "}"


def gc_text(gc: dict) -> str:
    keys = [(int(k[0]) if isinstance(k, str) else k) for k in gc]
    return str(keys)


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
    out = []
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
        at = packet.get("t_ms", 0) / 1000.0
        case = None
        for stamp, name in markers:
            if stamp <= at + 0.5:
                case = name
            else:
                break
        if case:
            out.setdefault(case, []).append(packet)
    return out


def bevy_rows(packets: list[dict]) -> list[dict]:
    rows = []
    for packet in packets:
        if packet["dir"] == "S->C":
            if packet["name"] not in ("SCOneUnitMovementPacket", "SCUnitAttachedPacket",
                                      "SCUnitDetachedPacket", "SCSlaveBoundPacket",
                                      "SCSlaveCreatedPacket", "SCMySlavePacket"):
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
        else:
            raw = bytes.fromhex(packet["body"].replace(" ", "") or "")
            decoded = {"tl_id": int.from_bytes(raw[:2], "little")} if len(raw) >= 2 else {}
        rows.append({"t_ms": packet["t_ms"], "event": "packet", "case": "",
                     "opcode": f"0x{packet['opcode']:03X}", "len": packet["len"],
                     "body": packet["body"], "decoded": json.dumps(decoded)})
    return rows


def case_text(s: dict) -> str:
    if not s["support"]:
        return "no 0x20 support block"
    flags = set_text(s["flags"])
    actors = set_text(s["actors"])
    dirs = sorted(s["dirs"], key=lambda k: -s["dirs"][k])[:3]
    return (f"{s['support']} x support, hdr {flags}, actor {actors}, "
            f"gc {gc_text(s['gc'])}, cadence {s['cadence_median_ms']} ms, "
            f"dir(rel,world) {dirs}")


def build(native: dict, bevy: dict, jump: dict) -> tuple[dict, str]:
    checks: dict[str, str] = {}
    details = {}
    bevy_rows_all = bevy["rows"]
    native_idle = native["d_after"]
    native_leave = native["d_leave"]
    for case in CASES:
        moves = moves_of(bevy_rows_all, case)
        ships = ship_samples(bevy_rows_all, case)
        s = stats(moves, ships)
        details[case] = s
        problems: list[str] = []
        ref = NATIVE_REF[case]
        if case in ("still_idle", "still_turn", "still_walk"):
            if s["support"] < 3:
                problems.append(f"support {s['support']} x (native {native_idle['support']})")
            if set(s["actors"]) != {0x24}:
                problems.append(f"actor {set_text(s['actors'])} vs {set_text(native_idle['actors'])}")
            if set(s["flags"]) != set(native_idle["flags"]):
                problems.append(f"header {set_text(s['flags'])} vs {set_text(native_idle['flags'])}")
            if s["cadence_median_ms"] is None or abs(s["cadence_median_ms"] - 128) > 25:
                problems.append(f"cadence {s['cadence_median_ms']} vs 128 ms")
            if case == "still_idle" and any(value > 0.2 for value in s["ranges"].get("local", [0, 0, 0])):
                problems.append(f"local moved {s['ranges'].get('local')}")
            if s["dir_residual_bytes"] is not None and s["dir_residual_bytes"] > 3:
                problems.append(f"direction frame residual {s['dir_residual_bytes']} B")
            if s["world_to_ship_m"] is not None and s["world_to_ship_m"] > 4.0:
                problems.append(f"aux is {s['world_to_ship_m']} m from the hull")
            if len(s["ship_ids"]) != 1:
                problems.append(f"support names {len(s['ship_ids'])} ship ids")
            if case == "still_turn" and len(s["dirs"]) < 2:
                problems.append("no turn: scripted A/D did not change the deck facing")
            if case == "still_walk" and max(s["ranges"].get("local", [0, 0])[:2]) < 0.25:
                problems.append(f"no walk: local range {s['ranges'].get('local')}")
        elif case == "still_jump":
            if s["support"] < 1:
                problems.append("no support block during the jump window")
            if jump["n"] and not any(m.get("actor_flags", 0) & 0x10 for m in moves):
                problems.append(f"no 0x44/0x34 jump: native {jump['n']} x")
            elif not jump["n"] and not any(m.get("actor_flags", 0) & 0x10 for m in moves):
                problems.append("no 0x34 packet")
        elif case in ("off_walk", "moving_off"):
            if s["support"] < 2:
                problems.append(f"support before leaving {s['support']}")
            if s["plain_after"] < 1:
                problems.append("no plain type 1 after the last support block")
            if not s["plain_after_actors"] or not set(s["plain_after_actors"]) <= {0, 4}:
                problems.append(f"plain actor {set_text(s['plain_after_actors'])}")
            if any(actor & 0x20 for actor in s["plain_after_actors"]):
                problems.append("support flag remained after leaving")
        elif case in ("off_after", "after_off"):
            if s["support"]:
                problems.append(f"{s['support']} support blocks after leaving")
        elif case == "drive_forward":
            t4 = type4_of(bevy_rows_all, case)
            if len(t4) < 2:
                problems.append(f"{len(t4)} type-4 requests")
            if not any(m.get("throttle") == 127 for m in t4):
                problems.append("no throttle 127")
            if ships:
                xs = [ship["x"] for ship in ships]
                if max(xs) - min(xs) < 0.02:
                    problems.append(f"hull moved x {round(max(xs) - min(xs), 2)} m")
            else:
                problems.append("no S->C type-3 ship samples")
        elif case == "drive_leave":
            disc = controls_of(bevy_rows_all, case, "0x032")
            if not disc:
                problems.append("no 0x032")
            elif [m for m in type4_of(bevy_rows_all, case) if m["_t"] > disc[0]["_t"]]:
                problems.append("type-4 after 0x032")
            moving_support = [m for m in moves if m["_t"] > (disc[0]["_t"] if disc else 0)
                              and m.get("actor_flags", 0) & 0x20]
            if len(moving_support) < 2:
                problems.append("no deck support after leaving the helm")
        elif case == "moving_idle":
            if s["support"] < 5:
                problems.append(f"support {s['support']}")
            if s["ranges"].get("local", [9, 9, 9])[0] > 0.2:
                problems.append(f"local x moved {s['ranges'].get('local')}")
            if max(s["ranges"].get("world", [0, 0, 0])[:2]) < 0.3:
                problems.append(f"world did not follow the hull {s['ranges'].get('world')}")
            if set(s["flags"]) != set(native_idle["flags"]):
                problems.append(f"header {set_text(s['flags'])} vs {set_text(native_idle['flags'])}")
            if set(s["actors"]) != {0x24}:
                problems.append(f"actor {set_text(s['actors'])} vs {set_text(native_idle['actors'])}")
            if s["cadence_median_ms"] is None or abs(s["cadence_median_ms"] - 128) > 25:
                problems.append(f"cadence {s['cadence_median_ms']} vs 128 ms")
            if s["world_to_ship_m"] is not None and s["world_to_ship_m"] > 4.0:
                problems.append(f"aux is {s['world_to_ship_m']} m from the hull")
            if len(s["ship_ids"]) != 1:
                problems.append(f"support names {len(s['ship_ids'])} ship ids")
        checks[case] = "MATCH" if not problems else "DIFFERS (" + "; ".join(problems) + ")"

    # The capture's off transition has two header-only 0x40/0x04 packets between the
    # last support block and the first plain world packet; note their presence/absence.
    headers = []
    for case in ("off_walk", "moving_off"):
        headers.append(details[case]["header_only"])

    before = {}
    for case in CASES:
        if case in ("drive_forward", "drive_leave"):
            before[case] = "type-4 helm only; the deck has no 0x20 block"
        elif case in ("off_after", "after_off"):
            before[case] = "plain world type 1, the hull slides away"
        else:
            before[case] = "no 0x20 block; plain world type 1 (0x02/0x00), the hull slides away"

    after_text = {case: case_text(details[case]) for case in CASES}
    t4 = type4_of(bevy_rows_all, "drive_forward")
    hull_x = [ship["x"] for ship in ship_samples(bevy_rows_all, "drive_forward")]
    after_text["drive_forward"] = (
        f"{len(t4)} x type 4, throttle {sorted({m['throttle'] for m in t4})}, "
        f"steering {sorted({m['steering'] for m in t4})}, hull x moved "
        f"{round(max(hull_x) - min(hull_x), 2) if hull_x else None} m")
    disc = controls_of(bevy_rows_all, "drive_leave", "0x032")
    leave = details["drive_leave"]
    after_text["drive_leave"] = (
        f"0x032 tl {disc[0].get('tl_id') if disc else None}; {leave['support']} support "
        f"after the get-off, hdr {set_text(leave['flags'])}, "
        f"world x range {leave['ranges'].get('world', [None])[0]} m")
    native_text = {
        "still_idle": case_text(native_idle),
        "still_turn": case_text(native_idle) + " (no A/D deck case in the capture)",
        "still_walk": case_text(native_leave),
        "still_jump": f"{jump['n']} x jump hdr {['0x%02x' % v for v in jump['flags']]}, "
                      f"actor {['0x%02x' % v for v in jump['actors']]}",
        "off_walk": "setup: 16 x support, then 2 x header-only 0x40/0x04, then plain 0x00/0x00",
        "off_after": "plain 0x00/0x00 after the support block ends",
        "drive_forward": "b_forward: type-4 throttle [0, 127], steering [0]",
        "drive_leave": "d_leave: 0x032, 15 x support across the leave, then no type-4",
        "moving_idle": case_text(native_idle),
        "moving_off": "setup: support ends, 2 x header-only 0x40/0x04, plain 0x00/0x00",
        "after_off": "plain 0x00/0x00",
    }

    ok = all(value == "MATCH" for value in checks.values())
    record = {
        "meta": bevy["meta"],
        "native": native,
        "native_jump": jump,
        "bevy": details,
        "checks": checks,
        "ok": ok,
        "rows": bevy["rows"],
    }
    lines = [
        "## Bevy replay",
        "",
        f"date: {bevy['meta']['date']}  account: <account> / <character>",
        f"client: <client>  (--headless --mute, --headless-fps {bevy['meta']['fps']})",
        f"script: <evidence>/replay.txt  exit: {bevy['meta']['script_exit']}",
        f"wall: {bevy['meta']['wall_s']} s",
        "",
        "| case | before (PR #39, no deck standing) | 1.2 capture | after (replay) | status |",
        "|---|---|---|---|---|",
    ]
    for case in CASES:
        lines.append(f"| {case} | {before[case]} | {native_text[case]} | "
                     f"{after_text[case]} | {checks[case]} |")
    lines += ["", "Checks:"]
    for case in CASES:
        s = details[case]
        ref = NATIVE_REF[case]
        n = native.get(ref, {})
        lines.append(
            f"- {case}: header {set_text(s['flags'])} vs {set_text(n.get('flags', {}))}; "
            f"actor {set_text(s['actors'])} vs {set_text(n.get('actors', {}))}; "
            f"gc {gc_text(s['gc'])} vs {gc_text(n.get('gc', {}))}; "
            f"cadence {s['cadence_median_ms']} vs {n.get('cadence_median_ms')} ms; "
            f"local range {s['ranges'].get('local')} vs {n.get('ranges', {}).get('local')}; "
            f"world range {s['ranges'].get('world')} vs {n.get('ranges', {}).get('world')}; "
            f"dir {sorted(s['dirs'].items(), key=lambda kv: -kv[1])[:2]} vs "
            f"{sorted(n.get('dirs', {}).items(), key=lambda kv: -kv[1])[:2]}; "
            f"dir residual {s['dir_residual_bytes']} B; world-to-ship {s['world_to_ship_m']} m.")
    idle = details["still_idle"]
    lines += [
        "",
        f"- The idle/standing deck packet is header {set_text(idle['flags'])} actor "
        f"{set_text(idle['actors'])} in the replay, the capture's 0x40/0x24.",
        f"- The jump window has {jump['n']} native jump packets (0x44/0x34); the replay's "
        f"jump packets are {set_text(details['still_jump']['flags'])}/"
        f"{set_text(details['still_jump']['actors'])}.",
        f"- The off transition has {headers[0]} header-only 0x40/0x04 packets before the "
        f"plain world type 1 in the replay; the capture has 2 (documented, not emitted).",
        "- Scripted W/A/S/D/J now reach the deck through the shared on-foot input sources.",
        "",
        "Notes:",
        "- The lane runs `aaemu_game::net=trace` (not `net=debug`) so the S->C type-3",
        "  Debug fields are in rs-game.log; the C->S bodies come from the fields trace.",
        "- Object ids and the absolute packet clock are normalised; the support block is",
        "  decoded as gcFlags, gcPartId, world position + rotation, gcId on both sides.",
        "- The moving run is one client: bind the rowboat as driver, W briefly, leave",
        "  the helm (0x032); the get-off lands on the coasting deck and is walked off.",
        "- `still_idle` waits 5 s; the shot line in replay.txt is skipped under --headless.",
        "",
        "Capture details:",
        "- The capture's only 0x44 support rows are the two jumps (a_idle, d_leave);",
        "  there is no 0x44/0x24 stop row. Idle and walking are 0x40/0x24.",
        "- The setup step-off has 2 header-only 0x40/0x04 rows between support and plain;",
        "  the client leaves the deck with support -> plain and does not send them.",
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
                                                                         "127.0.0.1:1237")))
    parser.add_argument("--account", default=os.environ.get("SHIP_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("SHIP_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("SHIP_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("SHIP_SERVER_LOG"))
    parser.add_argument("--pak", default=os.environ.get("AA_PAK"), help="the game pak (AA_PAK)")
    parser.add_argument("--fps", type=float, default=float(os.environ.get("SHIP_FPS", "31.5")))
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "ship_deck"))
    parser.add_argument("--raw", default=os.environ.get("SHIP_RAW")
                        or str(ROOT / "data" / "extract"
                               / f"ship-deck-replay-{time.strftime('%Y%m%d')}-r1"))
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
    native_rows = json.loads((evidence / "native-deck.json").read_text(encoding="utf-8"))["rows"]
    native = native_summary(native_rows)
    jump = a_idle_jump(native_rows)

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
        for row in bevy_rows(by_case.get(case, [])):
            row["case"] = case
            rows.append(row)
    bevy_record = {
        "meta": {
            "account": "<account>", "character": "<character>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "client": "<client>",
            "server_log": "<server-log>", "wall_s": round(time.monotonic() - started, 1),
            "fps": args.fps, "script_exit": process.returncode,
            "script_pass": process.returncode == 0,
        },
        "rows": rows,
    }
    record, report = build(native, bevy_record, jump)
    (evidence / "bevy.json").write_text(json.dumps(jsonable(record), indent=1), encoding="utf-8")
    (evidence / "replay-report.txt").write_text(report, encoding="utf-8")
    print(report)
    return 0 if record["ok"] and process.returncode == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
