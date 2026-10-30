#!/usr/bin/env python3
"""Replay the 1.2 cart terrain-following cases (C8a) in the Bevy client.

Runs tools/evidence/cart_terrain/replay.txt with `aa-client --headless --mute`
against the lane, polls the server's C->S field trace and compares the type-2
CSMoveUnit bodies with tools/evidence/cart_terrain/native.json (the 2026-10-29
s67 capture, cases b_forward/e_slope):

* pitch and roll from the wire quaternion (Rz*Ry*Rx: pitch = asin(forward.z),
  roll = asin(-right.z)),
* z and z - terrain (read at run time from the user's AA_PAK through aa-pak),
* vel.z at 30/32767 m/s (negative downhill),
* the full quaternion (x, y and z all carried) and the matched-point error
  against the capture along the driven distance.

Writes bevy.json and replay-report.txt (before/after table) to the evidence
directory. The client is stopped by its pid on timeout or exit. Screenshots are
not taken here (local-only shots belong under the handoff shots directory).

Usage:
    python tools/cart_terrain_replay.py --client <aa-client.exe> \
        --login <host:port> --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        [--pak <game_pak>] [--aa-pak-bin <aa-pak.exe>] [--evidence tools/evidence/cart_terrain] [--fps 21]
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
CASES = ["flat", "e_slope"]
NATIVE_CASE = {"flat": "b_forward", "e_slope": "e_slope"}
INTEREST_C2S = {0x031, 0x032, 0x089}
MARKER_RE = re.compile(r"case=([a-z0-9_.]+)")
UTC_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z")
SCRIPT_T_RE = re.compile(r"SCRIPT \[\s*([0-9.]+)s\]")
SCALE = 30.0 / 32767.0
GRAVITY = 9.81


def log(*args: object) -> None:
    print("cart-terrain:", *args, flush=True)


def fmt(value: float | None, spec: str = "+.3f") -> str:
    return "n/a" if value is None else format(value, spec)


class CellHeights:
    """The finest terrain triangle of one cell, from its `heightmap.dat`."""

    GRID = 513
    CELL = 512

    def __init__(self, heights: list[float]) -> None:
        self.heights = heights

    @classmethod
    def parse(cls, data: bytes) -> "CellHeights":
        import struct

        pos = 0

        def take(fmt: str) -> tuple:
            nonlocal pos
            size = struct.calcsize(fmt)
            value = struct.unpack_from(fmt, data, pos)
            pos += size
            return value

        def u8() -> int:
            return take("<B")[0]

        def i32() -> int:
            return take("<i")[0]

        def f32() -> float:
            return take("<f")[0]

        def u16() -> int:
            return take("<H")[0]

        version = u8()
        pos += 3
        chunk = i32()
        _units = i32()
        unit_m = i32()
        sector_m = i32()
        _sectors = i32()
        _z_ratio = f32()
        _ocean = f32()
        if unit_m != 2 or sector_m != 64:
            raise SystemExit(f"heightmap: unit {unit_m} m, sector {sector_m} m")
        if version >= 24:
            pos += 128
        nodes = []
        while pos != chunk:
            v = u8()
            pos += 3
            minimum = [f32(), f32(), f32()]
            pos += 12  # box max
            _holes = u8()
            offset = f32()
            value_range = f32()
            size = i32()
            unk = i32()
            samples = [u16() for _ in range(size * size)]
            pos += 4 + 16
            if unk >= 1:
                pos += min(unk, len(data) - pos)
            pos += min(36, len(data) - pos)
            nodes.append({
                "version": v, "min": minimum, "offset": offset,
                "range": value_range, "size": size, "data": samples,
            })
        if not nodes:
            raise SystemExit("heightmap: no nodes")
        origin_x, origin_y = nodes[0]["min"][0], nodes[0]["min"][1]
        heights = [0.0] * (cls.GRID * cls.GRID)

        def node_height(node: dict, raw: int) -> float:
            if node["version"] < 7:
                return node["offset"] + (raw & 0xFFE0) * node["range"]
            f_max = node["offset"] + 0xFFF0 * node["range"]
            i_offset = int(node["offset"] * 20.0)
            i_range = int((f_max - node["offset"]) * 20.0)
            step = (i_range + 4095 - 1) // 4095 if i_range > 0 else 1
            return 0.05 * i_offset + ((raw >> 4) * step) * 0.05

        def sample_node(node: dict, x: int, y: int) -> float:
            if node["size"] == 33:
                return node_height(node, node["data"][x * 33 + y])
            s = (node["size"] - 1) / 32.0
            fx, fy = x * s, y * s
            ix = min(int(fx), node["size"] - 2)
            iy = min(int(fy), node["size"] - 2)
            tx, ty = fx - ix, fy - iy
            h = lambda a, b: node_height(node, node["data"][a * node["size"] + b])
            top = h(ix, iy) * (1 - tx) + h(ix + 1, iy) * tx
            bottom = h(ix, iy + 1) * (1 - tx) + h(ix + 1, iy + 1) * tx
            return top * (1 - ty) + bottom * ty

        for node in nodes:
            if node["size"] <= 0:
                continue
            sx = round((node["min"][0] - origin_x) / 2.0)
            sy = round((node["min"][1] - origin_y) / 2.0)
            if sx < 0 or sy < 0 or sx + 32 > cls.CELL or sy + 32 > cls.CELL:
                continue
            for y in range(33):
                for x in range(33):
                    heights[(sy + y) * cls.GRID + sx + x] = sample_node(node, x, y)
        return cls(heights)

    def at(self, ix: int, iy: int) -> float:
        return self.heights[min(iy, self.GRID - 1) * self.GRID + min(ix, self.GRID - 1)]

    def mesh_height(self, x: float, y: float) -> float:
        import math as _math

        fx = max(0.0, min(x / 2.0, self.CELL))
        fy = max(0.0, min(y / 2.0, self.CELL))
        ix = min(int(_math.floor(fx)), self.CELL - 1)
        iy = min(int(_math.floor(fy)), self.CELL - 1)
        tx, ty = fx - ix, fy - iy
        a, b = self.at(ix, iy), self.at(ix + 1, iy)
        c, d = self.at(ix + 1, iy + 1), self.at(ix, iy + 1)
        if tx + ty <= 1.0:
            return a + (b - a) * tx + (d - a) * ty
        return c + (d - c) * (1.0 - tx) + (b - c) * (1.0 - ty)


class PakTerrain:
    """The route's terrain, read at run time from the user's `AA_PAK`."""

    def __init__(self, pak_bin: list[str], env: dict[str, str]) -> None:
        self.pak_bin = pak_bin
        self.env = env
        self.cells: dict[tuple[int, int], CellHeights] = {}

    def cell(self, cx: int, cy: int) -> CellHeights | None:
        if cx < 0 or cy < 0:
            return None
        if (cx, cy) not in self.cells:
            path = f"worlds/main_world/cells/{cx:03}_{cy:03}/client/terrain/heightmap.dat"
            result = subprocess.run(
                self.pak_bin + ["cat", path], stdout=subprocess.PIPE,
                stderr=subprocess.PIPE, env=self.env, cwd=str(ROOT))
            if result.returncode != 0:
                raise SystemExit(
                    f"cannot read {path} from {self.env.get('AA_PAK')}: "
                    f"{result.stderr.decode('utf-8', 'replace').strip()}")
            self.cells[(cx, cy)] = CellHeights.parse(result.stdout)
        return self.cells[(cx, cy)]

    def at(self, x: float, y: float) -> float | None:
        if not (math.isfinite(x) and math.isfinite(y)) or x < 0 or y < 0:
            return None
        cx, cy = math.floor(x / 1024), math.floor(y / 1024)
        cell = self.cell(cx, cy)
        return None if cell is None else cell.mesh_height(x - cx * 1024, y - cy * 1024)


def quat_rotate(q: list[float], v: tuple[float, float, float]) -> list[float]:
    x, y, z, w = q
    vx, vy, vz = v
    tx = 2.0 * (y * vz - z * vy)
    ty = 2.0 * (z * vx - x * vz)
    tz = 2.0 * (x * vy - y * vx)
    return [vx + w * tx + y * tz - z * ty,
            vy + w * ty + z * tx - x * tz,
            vz + w * tz + x * ty - y * tx]


def pose(move: dict, terrain: PakTerrain) -> dict:
    q = move["quat"]
    forward = quat_rotate(q, (0.0, 1.0, 0.0))
    right = quat_rotate(q, (1.0, 0.0, 0.0))
    pitch = math.asin(max(-1.0, min(1.0, forward[2])))
    roll = math.asin(max(-1.0, min(1.0, -right[2])))
    x, y, z = move["pos"]
    ground = terrain.at(x, y)
    return {
        "x": x, "y": y, "z": z, "pitch": pitch, "roll": roll,
        "z_terrain": None if ground is None else z - ground,
        "vel_z": move["vel"][2] * SCALE,
        "quat": q,
    }


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


def cumulative(points: list[dict]) -> list[float]:
    out = [0.0]
    for a, b in zip(points, points[1:]):
        out.append(out[-1] + math.hypot(b["x"] - a["x"], b["y"] - a["y"]))
    return out


def summary(points: list[dict]) -> dict:
    if not points:
        return {"count": 0}
    def rng(key: str) -> list[float]:
        vals = [p[key] for p in points if p[key] is not None]
        return [round(min(vals), 4), round(max(vals), 4)] if vals else []
    def mean(key: str) -> float | None:
        vals = [p[key] for p in points if p[key] is not None]
        return round(statistics.mean(vals), 4) if vals else None
    zs = [p["z"] for p in points]
    return {
        "count": len(points),
        "pitch_range": rng("pitch"), "pitch_mean": mean("pitch"),
        "roll_range": rng("roll"), "roll_mean": mean("roll"),
        "z_range": [round(min(zs), 3), round(max(zs), 3)],
        "z_drop": round(zs[0] - zs[-1], 3),
        "z_terrain_range": rng("z_terrain"), "z_terrain_mean": mean("z_terrain"),
        "vel_z_range": rng("vel_z"), "vel_z_mean": mean("vel_z"),
        "quat_xy": [round(max(abs(p["quat"][0]) for p in points), 5),
                    round(max(abs(p["quat"][1]) for p in points), 5)],
    }


def matched(native: list[dict], bevy: list[dict]) -> dict:
    """Compare the Bevy samples to the native curve at the same driven distance."""
    nd, bd = cumulative(native), cumulative(bevy)
    errors = []
    for point, distance in zip(bevy, bd):
        if distance > nd[-1]:
            break
        j = next((i for i, d in enumerate(nd) if d >= distance), len(nd) - 1)
        if j == 0:
            reference = native[0]
        else:
            span = nd[j] - nd[j - 1]
            t = 0.0 if span <= 0 else (distance - nd[j - 1]) / span
            reference = {}
            for key in ("pitch", "roll", "z", "z_terrain", "vel_z"):
                a, b = native[j - 1][key], native[j][key]
                reference[key] = None if a is None or b is None else a + (b - a) * t
        errors.append({
            "pitch": abs(point["pitch"] - reference["pitch"]),
            "roll": abs(point["roll"] - reference["roll"]),
            "z_terrain": (None if point["z_terrain"] is None or reference["z_terrain"] is None
                          else abs(point["z_terrain"] - reference["z_terrain"])),
        })
    out = {"samples": len(errors)}
    for key in ("pitch", "roll", "z_terrain"):
        vals = [e[key] for e in errors if e[key] is not None]
        out[key] = {
            "mean": round(statistics.mean(vals), 4) if vals else None,
            "max": round(max(vals), 4) if vals else None,
        }
    return out


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
            if stamp <= at:
                case = name
            else:
                break
        if case:
            out.setdefault(case, []).append(packet)
    return out


def build(native: dict, bevy: dict, terrain: PakTerrain) -> tuple[dict, str]:
    native_rows = native["rows"]
    bevy_rows = bevy["rows"]
    result: dict = {"meta": bevy["meta"], "cases": {}, "checks": {}}
    lines = [
        "## Cart terrain following replay (C8a)",
        "",
        f"date: {bevy['meta']['date']}  account: <account> / <character>",
        f"client: <client> (--headless --mute, --headless-fps {bevy['meta']['fps']})",
        f"script: <evidence>/replay.txt  exit: {bevy['meta']['script_exit']}",
        f"wall: {bevy['meta']['wall_s']} s",
        "",
        "Before = the cart controller before C8a: pitch/roll 0, spawn z kept, vel.z 0.",
        "",
        "| case | before (no terrain following) | 1.2 capture | Bevy replay | matched mean |diff| | status |",
        "|---|---|---|---|---|---|",
    ]
    ok = True
    for case in CASES:
        native_case = NATIVE_CASE[case]
        native_points = [pose(m, terrain) for m in moves_of(native_rows, native_case)]
        bevy_points = [pose(m, terrain) for m in moves_of(bevy_rows, case)]
        ns, bs = summary(native_points), summary(bevy_points)
        if not bevy_points:
            bs = {"count": 0, "pitch_mean": 0.0, "pitch_range": [0.0, 0.0],
                  "roll_mean": 0.0, "roll_range": [0.0, 0.0],
                  "z_terrain_mean": None, "vel_z_mean": 0.0, "z_drop": 0.0}
        diff = matched(native_points, bevy_points) if native_points and bevy_points else {}
        result["cases"][case] = {"native": ns, "bevy": bs, "matched": diff}
        problems = []
        if not bevy_points:
            problems.append("no type-2 sends")
        else:
            if case == "flat":
                if abs(bs["pitch_mean"] or 0) > 0.02:
                    problems.append(f"pitch {bs['pitch_mean']}")
                if bs["z_terrain_mean"] is None or abs(bs["z_terrain_mean"] - 0.024) > 0.06:
                    problems.append(f"z-terrain {bs['z_terrain_mean']}")
                if abs(bs["vel_z_mean"] or 0) > 0.05:
                    problems.append(f"vel.z {bs['vel_z_mean']}")
            else:
                if (bs["pitch_mean"] or 0) > -0.03:
                    problems.append(f"pitch {bs['pitch_mean']}")
                if (bs["roll_mean"] or 0) < 0.03:
                    problems.append(f"roll {bs['roll_mean']}")
                if (bs["vel_z_mean"] or 0) > -0.05:
                    problems.append(f"vel.z {bs['vel_z_mean']}")
                if not 0.5 <= bs["z_drop"] <= 1.6:
                    problems.append(f"z drop {bs['z_drop']}")
            if diff.get("pitch", {}).get("mean") is None:
                problems.append("no matched samples")
            else:
                if diff["pitch"]["mean"] > 0.08:
                    problems.append(f"pitch diff {diff['pitch']['mean']}")
                if diff["roll"]["mean"] > 0.08:
                    problems.append(f"roll diff {diff['roll']['mean']}")
                if diff["z_terrain"]["mean"] is not None and diff["z_terrain"]["mean"] > 0.12:
                    problems.append(f"z-terrain diff {diff['z_terrain']['mean']}")
        status = "MATCH" if not problems else "DIFFERS (" + "; ".join(problems) + ")"
        result["checks"][case] = status
        ok = ok and not problems
        before = "pitch/roll 0, z kept at spawn, vel.z 0"
        native_text = (f"pitch {fmt(ns['pitch_mean'])} ({fmt(ns['pitch_range'][0])}..{fmt(ns['pitch_range'][1])}), "
                       f"roll {fmt(ns['roll_mean'])}, z-terrain {fmt(ns['z_terrain_mean'])} m, "
                       f"z drop {ns['z_drop']:.2f} m, vel.z {fmt(ns['vel_z_mean'])} m/s")
        bevy_text = (f"pitch {fmt(bs['pitch_mean'])} ({fmt(bs['pitch_range'][0])}..{fmt(bs['pitch_range'][1])}), "
                     f"roll {fmt(bs['roll_mean'])}, z-terrain {fmt(bs['z_terrain_mean'])} m, "
                     f"z drop {bs['z_drop']:.2f} m, vel.z {fmt(bs['vel_z_mean'])} m/s")
        diff_text = (f"pitch {fmt(diff.get('pitch', {}).get('mean'))}, "
                     f"roll {fmt(diff.get('roll', {}).get('mean'))}, "
                     f"z-terrain {fmt(diff.get('z_terrain', {}).get('mean'))}")
        lines.append(f"| {case} | {before} | {native_text} | {bevy_text} | {diff_text} | {status} |")
    lines += [
        "",
        "Notes:",
        "- Pitch/roll are decoded from the type-2 quaternion (Rz*Ry*Rx): pitch = asin(forward.z),",
        "  roll = asin(-right.z). z-terrain is read at run time from AA_PAK (nothing is stored).",
        "- The native capture ran at 22.5 FPS; the replay uses --headless-fps 21 so the 100 ms",
        "  send gate fires on the same 143 ms cadence.",
        "- The native e_slope case starts with the tilt left by the previous d_reverse slope, so",
        "  its first samples are not a settled plane; the replay spawns and settles first.",
        "- Out of scope: collisions with rocks/bushes (g3), full wheeled physics (cars), and",
        "  remote carts' tilt (S8).",
        "",
    ]
    result["ok"] = ok
    return result, "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(
        epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("CART_BEVY_CLIENT", "aa-client.exe"))
    parser.add_argument("--login", default=os.environ.get("AA_LOGIN", "127.0.0.1:1237"))
    parser.add_argument("--account", default=os.environ.get("CART_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("CART_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("CART_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("CART_SERVER_LOG"))
    parser.add_argument("--pak", default=os.environ.get("AA_PAK"))
    parser.add_argument("--aa-pak-bin", default=os.environ.get("AA_PAK_BIN"),
                        help="the aa-pak CLI (else tools/aa-pak.exe or `cargo run -p aa-pak`)")
    parser.add_argument("--fps", type=float, default=float(os.environ.get("CART_FPS", "21")))
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "cart_terrain"))
    parser.add_argument("--raw", default=os.environ.get("CART_RAW")
                        or str(ROOT / "data" / "extract"
                               / f"cart-terrain-replay-{time.strftime('%Y%m%d')}-r1"))
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
    native = json.loads((evidence / "native.json").read_text(encoding="utf-8"))
    env = os.environ.copy()
    if args.pak:
        env["AA_PAK"] = args.pak
    if args.aa_pak_bin:
        pak_bin = [args.aa_pak_bin]
    elif (ROOT / "tools" / "aa-pak.exe").exists():
        pak_bin = [str(ROOT / "tools" / "aa-pak.exe")]
    else:
        pak_bin = ["cargo", "run", "--quiet", "--release", "-p", "aa-pak", "--bin", "aa-pak", "--"]
    terrain = PakTerrain(pak_bin, env)
    if not args.pak:
        raise SystemExit("--pak / AA_PAK is required to read the terrain at run time")

    trace = Trace(server_log)
    trace.reset()
    packets: list[dict] = []
    stopping = threading.Event()

    def poll() -> None:
        while not stopping.wait(0.02):
            packets.extend(trace.poll())

    thread = threading.Thread(target=poll, daemon=True)
    thread.start()
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
    log("client pid", process.pid, "exit", process.returncode)

    markers = [(stamp, match.group(1)) for stamp, line in timed(reader.lines)
               if stamp is not None and (match := MARKER_RE.search(line))]
    by_case = assign(packets, markers)
    rows: list[dict] = []
    for case in CASES:
        for packet in by_case.get(case, []):
            if packet["opcode"] == 0x089 and packet["dir"] == "C->S":
                decoded = decode_move(packet["body"])
                decoded["_t"] = packet["t_ms"]
                rows.append({"t_ms": packet["t_ms"], "event": "packet", "case": case,
                             "opcode": "0x089", "len": packet["len"],
                             "body": packet["body"], "decoded": json.dumps(decoded)})
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
    record, report = build(native, bevy_record, terrain)
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    (evidence / "replay-report.txt").write_text(report, encoding="utf-8")
    print(report)
    return 0 if record["ok"] and process.returncode == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())


