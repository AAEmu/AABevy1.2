#!/usr/bin/env python3
"""Capture the original 1.2 client steering a rowboat (slave template 15), read-only.

The rowboat's helm is attach point 1 of a `ShipModel` slave: the server owns the
hull and the client sends `CSMoveUnit` 0x089 type 4 (throttle, steering) on the
100 ms non-actor pump, also while both bytes are 0. This probe:

* launches the hidden 1.2 client through aaemu-clientdriver (or attaches with
  `--pid`), moves to open sea, spawns the rowboat with the server's GM chat
  command `/slave spawn 15`, and boards it with the client's own right-click
  pick and F key (`CSStartInteraction` 0x068 -> `SCNpcInteractionSkillList` 0x07b
  skill 12076 -> `CSBindSlave` 0x031);
* holds the client's own W/S/A/D keys through `client_input` and records every
  C->S body from the server's field trace (`aaemu_game::net::fields=trace`) and
  the decoded S->C movement/attach fields (`aaemu_game::net=trace`);
* samples the player pose from client memory and the rowboat hull from the
  client's own `client_entities` (prefab_unit) every 100 ms, with the client's
  `X2SiegeWeapon` telemetry about once a second;
* leaves with `X2SiegeWeapon:DiscardSiegeWeapon` (`CSDiscardSlave` 0x032).

Cases (as the 2026-10-29 capture):
  a idle 5 s; b W 6 s + release and S 3 s; c A 3 s and D 3 s while moving;
  d leave the helm while moving, then 2.5 s after.

Writes native.csv / native.json / notes.txt / server-log.txt under `--out`
(default tools/evidence/ship_steering). The client is stopped by its pid when
this probe launched it; `--pid` clients are left running.

Usage:
    python tools/ship_steering_probe.py --client <archeage.exe> \
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \
        --launcher <aaemu-launcher.exe> --login <host:port> \
        --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        [--place 4318,4123,100.5] [--template 15] [--out tools/evidence/ship_steering]
    python tools/ship_steering_probe.py --pid 1234 ...        # already in world

The account needs GM level for `/slave spawn` and `/move`. No desktop mouse or
keyboard events are sent, and no game data is written.
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import os
import re
import struct
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import cart_boarding_probe as cb  # noqa: E402
import mount_reason_probe as base  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
ROWBOAT = 15
CASE_ACTIONS = {
    "a_idle": (5000, ()),
    "b_forward": (6500, ((0, "w", 6000),)),
    "b_coast": (1800, ()),
    "b_reverse": (3500, ((0, "s", 3000),)),
    "c_left": (3600, ((0, "w", 3500), (200, "a", 3000))),
    "c_left_coast": (1500, ()),
    "c_right": (3600, ((0, "w", 3500), (200, "d", 3000))),
    "c_right_coast": (1500, ()),
    "d_leave": (3500, ((0, "w", 1200),)),
    "d_after": (2500, ()),
}

C2S_RE = re.compile(r"C->S level=(\d+) opcode=0x([0-9A-Fa-f]{3}) (\w+) len=(\d+) body=(.*)$")
S2C_RE = re.compile(r"GamePacket: S->C type ([0-9A-Fa-f]{3}) (\w+) \((\d+) bytes\)")
LOG_RE = re.compile(r"^(\d\d:\d\d:\d\d) \[(\w+)\] (\w+) - (.*)$")
NUM = r"-?\d+(?:\.\d+)?(?:e[-+]?\d+)?"
S2C_MOVES = {"SCOneUnitMovementPacket", "SCUnitMovementsPacket"}
S2C_ATTACH = {"SCUnitAttachedPacket", "SCUnitDetachedPacket", "SCSlaveBoundPacket",
              "SCSlaveCreatedPacket", "SCMySlavePacket"}
S2C_KEEP = S2C_MOVES | S2C_ATTACH | {"SCUnitStatePacket", "SCSlaveDespawnPacket",
                                     "SCSlaveRemovedPacket", "SCErrorMsgPacket"}
OPCODES = {"SCOneUnitMovementPacket": 0x06C, "SCUnitMovementsPacket": 0x06B,
           "SCUnitAttachedPacket": 0x076, "SCUnitDetachedPacket": 0x077,
           "SCSlaveBoundPacket": 0x064, "SCSlaveCreatedPacket": 0x061,
           "SCMySlavePacket": 0x066, "SCUnitStatePacket": 0x069,
           "SCSlaveDespawnPacket": 0x062, "SCSlaveRemovedPacket": 0x063,
           "SCErrorMsgPacket": 0x0A0}
SHOT_PATH_RE = re.compile(r"shots[/\\][^\"\\/]+[/\\]")


def log(*args: object) -> None:
    print("ship-steering:", *args, flush=True)


def now_ms() -> int:
    return int(time.time() * 1000)


def pairs(body: str) -> dict:
    out: dict = {}
    for part in body.split(", "):
        key, _, value = part.partition(": ")
        if not _:
            continue
        try:
            out[key] = int(value)
        except ValueError:
            try:
                out[key] = float(value)
            except ValueError:
                out[key] = {"true": True, "false": False}.get(value, value)
    return out


# ------------------------------------------------------------------ log trace

class Trace:
    """Incremental rs-game.log reader with ms timestamps and S->C field decoding."""

    def __init__(self, path: Path) -> None:
        self.path = Path(path)
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw: list[str] = []
        self.pending: list[dict] = []

    def reset(self) -> None:
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw = []

    def poll(self) -> list[dict]:
        if not self.path.exists():
            return []
        data = self.path.read_bytes()[self.offset:].decode("utf-8", "replace")
        if not data:
            return []
        complete = data.rsplit("\n", 1)[0] if not data.endswith("\n") else data
        self.offset += len(complete.encode("utf-8"))
        stamp = now_ms()
        out: list[dict] = []
        for line in complete.splitlines():
            self.raw.append(line)
            parsed = self._line(line, stamp)
            if parsed:
                out.append(parsed)
        return out

    def _line(self, line: str, stamp: int) -> dict | None:
        text = line.strip()
        log_time = text[:8]
        match = C2S_RE.search(text)
        if match:
            return {"t_ms": stamp, "log_time": log_time, "dir": "C->S",
                    "opcode": int(match.group(2), 16), "name": match.group(3),
                    "len": int(match.group(4)), "body": match.group(5).strip(),
                    "line": text}
        match = S2C_RE.search(text)
        if match:
            packet = {"t_ms": stamp, "log_time": log_time, "dir": "S->C",
                      "opcode": int(match.group(1), 16), "name": match.group(2),
                      "len": int(match.group(3)), "body": "", "line": text}
            self.pending = (self.pending + [packet])[-4:]
            return packet if packet["name"] in S2C_KEEP else None
        match = LOG_RE.match(text)
        if match and match.group(2) == "TRACE":
            message = match.group(4)
            for name in S2C_KEEP:
                if message.startswith(name + " {"):
                    meta = next((p for p in reversed(self.pending) if p["name"] == name), {})
                    packet = {"t_ms": stamp, "log_time": log_time, "dir": "S->C",
                              "opcode": meta.get("opcode", OPCODES.get(name, -1)),
                              "name": name, "len": meta.get("len", 0), "body": "",
                              "line": text, "decoded": decode_s2c(name, message),
                              "trace": True}
                    return packet
        return None


def decode_s2c(name: str, message: str) -> dict:
    """Decoded fields of one S->C trace Debug line."""
    out: dict = {}
    try:
        if name == "SCOneUnitMovementPacket":
            match = re.search(
                r"obj_id: (\d+), move_type: MoveType \{ type_raw: (\d+), time: (\d+), "
                r"flags: MoveTypeFlags\((\d+)\), sc_type: (\d+), phase: (\d+), "
                r"kind: (\w+)\((\w+) \{ (.*?) \}\) \}", message)
            if not match:
                return {"unparsed": message}
            out.update(obj_id=int(match.group(1)), type_raw=int(match.group(2)),
                       time=int(match.group(3)), flags=int(match.group(4)),
                       sc_type=int(match.group(5)), phase=int(match.group(6)),
                       kind=match.group(7))
            fields = pairs(match.group(9))
            out.update(fields)
            if match.group(7) == "Ship":
                vx, vy, vz = (float(fields.get(k, 0)) for k in ("vel_x", "vel_y", "vel_z"))
                speed = math.sqrt(vx * vx + vy * vy + vz * vz)
                out["wire_speed_3d"] = round(speed, 3)
                out["mps_1092"] = round(speed / (32767.0 / 30.0), 4)
                qx = float(fields.get("rotation_x", 0)) * 0.00003052
                qy = float(fields.get("rotation_y", 0)) * 0.00003052
                qz = float(fields.get("rotation_z", 0)) * 0.00003052
                qw = math.sqrt(max(0.0, 1.0 - qx * qx - qy * qy - qz * qz))
                out["quat"] = [round(qx, 6), round(qy, 6), round(qz, 6), round(qw, 6)]
                out["yaw_z"] = round(math.atan2(
                    2.0 * (qw * qz + qx * qy), 1.0 - 2.0 * (qy * qy + qz * qz)), 5)
        elif name == "SCUnitAttachedPacket":
            match = re.search(
                r"child_obj_id: (\d+), point: (\d+), reason: (\d+), obj_id: (\d+)", message)
            if match:
                out.update(child_obj_id=int(match.group(1)), point=int(match.group(2)),
                           reason=int(match.group(3)), obj_id=int(match.group(4)))
        elif name == "SCUnitDetachedPacket":
            match = re.search(r"obj_id: (\d+), reason: (\d+)", message)
            if match:
                out.update(obj_id=int(match.group(1)), reason=int(match.group(2)))
        elif name == "SCSlaveBoundPacket":
            match = re.search(r"master_id: (\d+), slave_obj_id: (\d+)", message)
            if match:
                out.update(master_id=int(match.group(1)), slave_obj_id=int(match.group(2)))
        elif name == "SCSlaveCreatedPacket":
            match = re.search(
                r"owner_obj_id: (\d+), tl_id: (\d+), slave_obj_id: (\d+), "
                r"hide_spawn_effect: (\w+), unk_id: (-?\d+), creator_name: \"(.*)\"", message)
            if match:
                out.update(owner_obj_id=int(match.group(1)), tl_id=int(match.group(2)),
                           slave_obj_id=int(match.group(3)),
                           hide_spawn_effect=match.group(4) == "true",
                           unk_id=int(match.group(5)))
        elif name == "SCMySlavePacket":
            match = re.search(
                r"obj_id: (\d+), tl_id: (\d+), name: \"(.*)\", template_id: (\d+), "
                r"hp: (\d+), max_hp: (\d+), x: (" + NUM + r"), y: (" + NUM + r"), "
                r"z: (" + NUM + r")", message)
            if match:
                out.update(obj_id=int(match.group(1)), tl_id=int(match.group(2)),
                           name=match.group(3), template_id=int(match.group(4)),
                           hp=int(match.group(5)), max_hp=int(match.group(6)),
                           x=float(match.group(7)), y=float(match.group(8)),
                           z=float(match.group(9)))
        else:
            out["unparsed"] = message[:400]
    except Exception as error:  # malformed trace line
        out["error"] = f"{type(error).__name__}: {error}"
        out["unparsed"] = message[:400]
    return out


# ------------------------------------------------------------------ CSMoveUnit decoding

def decode_position(raw: bytes) -> tuple[float, float, float]:
    """9-byte packed position: x/y 24-bit fixed point 1/4096 m, z 22-bit over 4196 m."""

    def raw24(a: int, b: int, c: int) -> int:
        return 8 * (a | b << 8 | c << 16)

    x = raw24(raw[0], raw[1], raw[2])
    y = raw24(raw[3], raw[4], raw[5])
    if raw[8] & 0x80:
        x = -x
    if raw[8] & 0x40:
        y = -y
    z_raw = raw[6] | raw[7] << 8 | (raw[8] & 0x3F) << 16
    z = z_raw * 2.3841858e-07 * 4196.0 - 100.0
    return x / 4096.0, y / 4096.0, round(z, 4)


def short_quat(raw: bytes, offset: int) -> list[float]:
    x, y, z = struct.unpack_from("<3h", raw, offset)
    x, y, z = x / 32767.0, y / 32767.0, z / 32767.0
    norm = x * x + y * y + z * z
    w = math.sqrt(1.0 - norm) if norm < 0.9975 else 0.0
    return [x, y, z, w]


def quat_yaw(quat: list[float]) -> float:
    x, y, z, w = quat
    return math.atan2(2.0 * (w * z + x * y), 1.0 - 2.0 * (y * y + z * z))


def decode_move(body_hex: str) -> dict:
    """CSMoveUnit 0x089 body: header + the 1.2 type 1 / 3 / 4 fields."""
    raw = bytes.fromhex(body_hex.replace(" ", ""))
    if len(raw) < 4:
        return {"error": "short"}
    obj = raw[0] | raw[1] << 8 | raw[2] << 16
    mtype = raw[3]
    out: dict = {"obj_id": obj, "type": mtype}
    pos = 4
    try:
        out["time_ms"] = struct.unpack_from("<I", raw, pos)[0]
        pos += 4
        flags = raw[pos]
        pos += 1
        out["flags"] = flags
        if flags & 0x10:
            out["sc_type"] = struct.unpack_from("<I", raw, pos)[0]
            out["phase"] = raw[pos + 4]
            pos += 5
        if mtype == 4:
            out["throttle"] = struct.unpack_from("<b", raw, pos)[0]
            out["steering"] = struct.unpack_from("<b", raw, pos + 1)[0]
            pos += 2
        elif mtype in (0, 1):
            out["pos"] = [round(v, 4) for v in decode_position(raw[pos:pos + 9])]
            pos += 9
            out["vel"] = list(struct.unpack_from("<3h", raw, pos))
            pos += 6
            out["rot"] = list(struct.unpack_from("<3b", raw, pos))
            pos += 3
            out["delta"] = list(struct.unpack_from("<3b", raw, pos))
            pos += 3
            out["stance"] = raw[pos]
            out["alertness"] = raw[pos + 1]
            actor_flags = raw[pos + 2]
            out["actor_flags"] = actor_flags
            pos += 3
            if actor_flags & 0x80:
                out["fall_vel"] = struct.unpack_from("<H", raw, pos)[0]
                pos += 2
            if actor_flags & 0x20:
                out["gc_flags"] = raw[pos]
                out["gc_part_id"] = struct.unpack_from("<H", raw, pos + 1)[0]
                pos += 3
                out["pos2"] = [round(v, 4) for v in decode_position(raw[pos:pos + 9])]
                pos += 9
                out["rot2"] = list(struct.unpack_from("<3b", raw, pos))
                pos += 3
            if actor_flags & 0x60:
                out["gc_id"] = struct.unpack_from("<I", raw, pos)[0]
                pos += 4
            if actor_flags & 0x40:
                out["climb_data"] = struct.unpack_from("<I", raw, pos)[0]
                pos += 4
        elif mtype in (2, 3):
            out["pos"] = [round(v, 4) for v in decode_position(raw[pos:pos + 9])]
            pos += 9
            out["vel"] = list(struct.unpack_from("<3h", raw, pos))
            pos += 6
            quat = short_quat(raw, pos)
            out["quat"] = [round(v, 6) for v in quat]
            out["yaw_z"] = round(quat_yaw(quat), 5)
            pos += 6
            out["ang_vel"] = [round(v, 6) for v in struct.unpack_from("<3f", raw, pos)]
            pos += 12
            if mtype == 2:
                out["steering"] = round(struct.unpack_from("<f", raw, pos)[0], 6)
                pos += 4
                count = raw[pos]
                pos += 1
                out["wheel_ang_vel"] = [
                    round(struct.unpack_from("<f", raw, pos + 4 * i)[0], 6)
                    for i in range(count)]
                pos += 4 * count
            else:
                out["steering"] = struct.unpack_from("<b", raw, pos)[0]
                out["throttle"] = struct.unpack_from("<b", raw, pos + 1)[0]
                out["zone_id"] = struct.unpack_from("<H", raw, pos + 2)[0]
                out["stuck"] = raw[pos + 4]
                pos += 5
        out["consumed"] = pos
        out["tail"] = raw[pos:].hex()
    except Exception as error:
        out["error"] = f"{type(error).__name__}: {error}"
        out["raw"] = raw.hex()
    return out


def decode_c2s(name: str, body_hex: str) -> dict:
    if name == "CSMoveUnitPacket":
        return decode_move(body_hex)
    raw = bytes.fromhex(body_hex.replace(" ", "") or "")
    if name in ("CSBindSlavePacket", "CSDiscardSlavePacket", "CSDestroySlavePacket"):
        return {"tl_id": struct.unpack_from("<H", raw, 0)[0]} if len(raw) >= 2 else {"error": "short"}
    return {"raw": raw.hex()[:200]}


# ------------------------------------------------------------------ evidence

class Slice:
    """One capture: rows -> native.csv / native.json, plus notes and log excerpt."""

    def __init__(self, outdir: Path, name: str, trace: Trace) -> None:
        self.out = Path(outdir)
        self.name = name
        self.trace = trace
        self.rows: list[dict] = []
        self.notes: list[str] = []
        self.started = time.strftime("%Y-%m-%dT%H:%M:%S")

    def note(self, text: str) -> None:
        self.notes.append(text)
        log("note:", text)

    def state(self, case: str, event: str, **fields: object) -> None:
        self.rows.append({
            "t_ms": fields.pop("t_ms", now_ms()), "case": case, "event": event,
            "dir": "", "opcode": "", "name": "", "len": "", "body": "", "decoded": "",
            "client_state": json.dumps(fields, ensure_ascii=False, default=str), "text": ""})

    def add(self, case: str, packets: list[dict]) -> None:
        for packet in packets:
            decoded = packet.get("decoded")
            if decoded is None:
                decoded = decode_c2s(packet["name"], packet["body"]) \
                    if packet["dir"] == "C->S" else {}
            self.rows.append({
                "t_ms": packet["t_ms"], "case": case, "event": "packet",
                "dir": packet["dir"],
                "opcode": f"0x{packet['opcode']:03X}" if packet["opcode"] >= 0 else "",
                "name": packet["name"], "len": packet["len"], "body": packet["body"],
                "decoded": json.dumps(decoded, ensure_ascii=False, default=str),
                "client_state": "", "text": packet.get("log_time", "")})

    def write(self) -> None:
        self.out.mkdir(parents=True, exist_ok=True)
        fields = ["t_ms", "case", "event", "dir", "opcode", "name", "len", "body",
                  "decoded", "client_state", "text"]
        with open(self.out / "native.csv", "w", encoding="utf-8", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=fields)
            writer.writeheader()
            writer.writerows(self.rows)
        meta = {"slice": self.name, "started": self.started,
                "written": time.strftime("%Y-%m-%dT%H:%M:%S"),
                "client": "original 1.2 r208022 archeage.exe (hidden, one launch)",
                "driver": "aaemu-clientdriver (Lua/pointer/client key path, no OS input)",
                "server_log": "<server-log>", "rows": len(self.rows),
                "clock": "t_ms = host wall clock, epoch milliseconds; text = rs-game.log HH:MM:SS"}
        (self.out / "native.json").write_text(
            json.dumps({"meta": meta, "notes": self.notes, "rows": self.rows},
                       indent=1, ensure_ascii=False), encoding="utf-8")
        excerpt = [line for line in self.trace.raw
                   if C2S_RE.search(line) or S2C_RE.search(line) or "TRACE] " in line]
        (self.out / "server-log.txt").write_text(
            "# rs-game.log lines for the ship steering capture (C->S fields trace, "
            "S->C opcode lines, S->C trace structs)\n" + "\n".join(excerpt) + "\n",
            encoding="utf-8")
        (self.out / "notes.txt").write_text("\n".join(self.notes) + "\n", encoding="utf-8")


# ------------------------------------------------------------------ client access

def vehicle(mcp: base.Mcp, pid: int, action: str = "status") -> dict:
    try:
        return json.loads(mcp.text("client_vehicle", pid=pid, action=action))
    except Exception as error:
        return {"error": f"{type(error).__name__}: {error}"}


def entities(mcp: base.Mcp, pid: int, cls: str = "prefab_unit", limit: int = 200) -> dict:
    try:
        payload = json.loads(mcp.text("client_entities", pid=pid, **{"class": cls, "limit": limit}))
        return payload.get("entities") or {}
    except Exception:
        return {}


def ent_row(ent: dict | None) -> dict | None:
    if not ent:
        return None
    pos = ent.get("position") or {}
    angles = ent.get("angles") or {}
    vel = ent.get("velocity") or {}
    return {"name": ent.get("name"), "class": ent.get("class"),
            "pos": [round(float(pos.get(k, 0.0)), 4) for k in ("x", "y", "z")],
            "angles": {k: round(float(angles.get(k, 0.0)), 4) for k in ("x", "y", "z")}
            if isinstance(angles, dict) else angles,
            "vel": [round(float(vel.get(k, 0.0)), 4) for k in ("x", "y", "z")]
            if isinstance(vel, dict) else vel}


def screen_point(mcp: base.Mcp, pid: int, world: list[float]) -> tuple[float, float]:
    text = cb.lua_text(
        mcp, pid, "return X2Util:ConvertWorldToScreen(%.6f,%.6f,%.6f)" % tuple(world))
    parts = [part for part in text.replace("\t", " ").split() if part]
    return float(parts[0]), float(parts[1])


def dist3(a, b) -> float:
    return math.sqrt(sum((float(a[i]) - float(b[i])) ** 2 for i in range(3)))


class Ctx:
    def __init__(self, mcp: base.Mcp, pid: int, mem: base.Mem, addr: int, trace: Trace) -> None:
        self.mcp, self.pid, self.mem, self.addr = mcp, pid, mem, addr
        self.trace = trace
        self.player = mem.u32(mem.u32(addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)

    def ui(self, code: str) -> str:
        return cb.lua_text(self.mcp, self.pid, code)

    def chat(self, message: str) -> None:
        self.mcp.text("client_chat", pid=self.pid, message=message)

    def units(self) -> dict[int, int]:
        return dict(base.walk_units(self.mem, self.addr)[1])

    def pos(self) -> list[float]:
        pose = self.pose()
        if not pose:
            raise RuntimeError("player position lost")
        return pose["pos"]

    def pose(self) -> dict | None:
        if not self.player:
            return None
        rot, pos = base.unit_state(self.mem, self.player)
        if not pos or not rot:
            return None
        return {"pos": [round(v, 4) for v in pos], "rot": [round(v, 6) for v in rot],
                "yaw": round(base.quat_yaw(rot), 5)}

    def is_driver(self) -> bool:
        return bool(vehicle(self.mcp, self.pid).get("driver"))

    def settle(self, timeout: float = 3.0) -> None:
        base.settle(self.mem, self.player, self.player, timeout=timeout)

    def face(self, target: list[float]) -> None:
        pos = self.pos()
        base.set_heading(self.mcp, self.pid, self.mem, self.player,
                         math.atan2(-(target[0] - pos[0]), target[1] - pos[1]))

    def poll(self, seconds: float) -> list[dict]:
        packets: list[dict] = []
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            packets += self.trace.poll()
            time.sleep(0.05)
        return packets


def measure_shift(ctx: Ctx, sl: Slice, place: list[float], character: str) -> list[float]:
    ctx.chat("/move %s %.2f %.2f %.2f" % (character, *place))
    time.sleep(2.0)
    ctx.settle()
    pos = ctx.pos()
    shift = [place[i] - pos[i] for i in range(3)]
    sl.state("setup", "shift", server=place, client=[round(v, 3) for v in pos],
             shift=[round(v, 3) for v in shift])
    return shift


def place_client(ctx: Ctx, sl: Slice, want: list[float], shift: list[float],
                 character: str, z: float | None = None) -> list[float]:
    server = [want[0] + shift[0], want[1] + shift[1], (z if z is not None else want[2]) + shift[2]]
    pos = ctx.pos()
    for attempt in range(1, 6):
        ctx.chat("/move %s %.2f %.2f %.2f" % (character, *server))
        time.sleep(1.2)
        ctx.settle()
        pos = ctx.pos()
        err = (want[0] - pos[0], want[1] - pos[1])
        sl.state("setup", "place", attempt=attempt, want=[round(v, 3) for v in want],
                 server=[round(v, 2) for v in server], client=[round(v, 3) for v in pos],
                 err=round(math.hypot(*err), 3))
        if math.hypot(*err) < 0.3:
            break
        server[0] += err[0]
        server[1] += err[1]
    return pos


def new_entity(ctx: Ctx, before: set[str], timeout: float = 25.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        for cls in ("prefab_unit", ""):
            fresh = [(eid, ent) for eid, ent in entities(ctx.mcp, ctx.pid, cls).items()
                     if eid not in before]
            if fresh:
                me = ctx.pos()
                fresh.sort(key=lambda item: dist3(
                    [float((item[1].get("position") or {}).get(k, 0)) for k in ("x", "y", "z")],
                    me))
                return fresh[0]
        time.sleep(0.5)
    return None, None


def spawn_ship(ctx: Ctx, sl: Slice, template: int, shift: list[float]) -> dict | None:
    before = set(entities(ctx.mcp, ctx.pid)) | set(entities(ctx.mcp, ctx.pid, ""))
    before_units = ctx.units()
    ctx.trace.reset()
    ctx.chat(f"/slave spawn {template}")
    packets = ctx.poll(2.0)
    sl.add("setup", packets)
    engine_id, ent = new_entity(ctx, before)
    packets += ctx.poll(1.5)
    sl.add("setup", packets)
    created = [p for p in packets if p.get("decoded") and p["name"] == "SCSlaveCreatedPacket"]
    tl = created[-1]["decoded"].get("tl_id") if created else None
    obj = created[-1]["decoded"].get("slave_obj_id") if created else None
    candidates = []
    for key, unit in ctx.units().items():
        if key in before_units:
            continue
        pose = ctx.pose() if unit == ctx.player else None
        rot, pos = base.unit_state(ctx.mem, unit)
        if pos:
            candidates.append((key, unit, [round(v, 3) for v in pos]))
    if ent:
        pos = [float((ent.get("position") or {}).get(k, 0.0)) for k in ("x", "y", "z")]
        match = next((c for c in candidates if obj is not None and c[0] == obj), None)
        if match is None and candidates:
            match = min(candidates, key=lambda c: dist3(c[2], pos))
        key, unit = (match[0], match[1]) if match else (0, 0)
    elif candidates:
        key, unit, pos = candidates[0]
    else:
        sl.state("setup", "spawn_failed", template=template, vehicle=vehicle(ctx.mcp, ctx.pid))
        return None
    ship = {"template": template, "engine_id": engine_id,
            "name": (ent or {}).get("name"), "unit": unit, "key": key, "pos": pos,
            "tl_id": tl, "obj": obj, "shift": list(shift)}
    sl.state("setup", "spawn", **ship)
    return ship


def try_pick(ctx: Ctx, sl: Slice, ship: dict, name: str) -> bool:
    width, height = 1280, 720
    try:
        values = [int(float(v)) for v in ctx.ui(
            "return UIParent:GetScreenWidth(), UIParent:GetScreenHeight()"
        ).replace("\t", " ").split()[:2]]
        if len(values) == 2 and values[0] > 100:
            width, height = values
    except Exception:
        pass
    try:
        sx, sy = screen_point(ctx.mcp, ctx.pid, ship["pos"])
    except Exception as error:
        sx, sy = width / 2.0, height / 2.0
        sl.state("setup", "project_failed", error=str(error))
    points = [(int(sx), int(sy))]
    for dy in (0, 40, -40, 80, -80, 130, -130):
        for dx in (0, -50, 50, -100, 100):
            points.append((int(sx + dx), int(sy + dy)))
    seen: set[tuple[int, int]] = set()
    for x, y in points:
        if (x, y) in seen or not (0 <= x < width and 0 <= y < height):
            continue
        seen.add((x, y))
        ctx.ui("X2Unit:ReleaseTarget() return 1")
        ctx.mcp.text("client_pointer", pid=ctx.pid, action="right_click", x=x, y=y)
        time.sleep(0.35)
        ctx.mcp.text("client_input", pid=ctx.pid, key="f", duration_ms=160)
        time.sleep(0.55)
        driver = ctx.is_driver()
        target = ctx.ui("return tostring(X2Unit:UnitName('target'))").strip()
        if driver:
            sl.state("setup", "pick", point=[x, y], tries=len(seen), driver=driver,
                     target=target, vehicle=vehicle(ctx.mcp, ctx.pid))
            return True
        if target == name:
            sl.state("setup", "targeted", point=[x, y], tries=len(seen), target=target)
    sl.state("setup", "pick_failed", tries=len(seen), vehicle=vehicle(ctx.mcp, ctx.pid),
             target=ctx.ui("return tostring(X2Unit:UnitName('target'))").strip())
    return False


def adopt_driver(ctx: Ctx, sl: Slice) -> dict | None:
    info = vehicle(ctx.mcp, ctx.pid)
    if not info.get("driver") or not (info.get("info") or {}).get("unitId"):
        return None
    obj = int(str(info["info"]["unitId"]), 16)
    unit = ctx.units().get(obj, 0)
    tl = ctx.mem.u32(unit + 0x41C) if unit else None
    engine_id, ent = None, None
    for eid, candidate in entities(ctx.mcp, ctx.pid).items():
        name = str(candidate.get("name") or "")
        if name == "Rowboat" or name.lower() in str(info["info"].get("desc_name") or "").lower():
            engine_id, ent = eid, candidate
            break
    if not ent:
        sl.state("setup", "adopt_failed", vehicle=info)
        return None
    pos = [float((ent.get("position") or {}).get(k, 0.0)) for k in ("x", "y", "z")]
    ship = {"template": None, "engine_id": engine_id, "name": ent.get("name"),
            "unit": unit, "key": obj, "pos": pos, "tl_id": tl, "obj": obj,
            "shift": [0.0, 0.0, 0.0], "adopted": True}
    sl.state("setup", "adopt", ship=ship, vehicle=info)
    return ship


class Cap:
    def __init__(self, ctx: Ctx, sl: Slice, ship: dict) -> None:
        self.ctx, self.sl = ctx, sl
        self.ship = ship
        self.ship_unit = int(ship.get("unit") or 0)
        self.ship_engine = ship.get("engine_id")
        self.ship_name = ship.get("name")
        self.case = "s"
        self.samples: dict[str, list[dict]] = {}
        self.t0 = 0.0

    def grid(self) -> dict:
        return entities(self.ctx.mcp, self.ctx.pid)

    def entity_pose(self) -> dict | None:
        grid = self.grid()
        ent = grid.get(str(self.ship_engine)) if self.ship_engine is not None else None
        if ent is None:
            for eid, candidate in grid.items():
                if self.ship_name and (candidate.get("name") or "") == self.ship_name:
                    ent, self.ship_engine = candidate, eid
                    break
        if not ent:
            return None
        pos = [float((ent.get("position") or {}).get(k, 0.0)) for k in ("x", "y", "z")]
        angles = ent.get("angles") or {}
        yaw = float(angles.get("z") or 0.0) if isinstance(angles, dict) else 0.0
        return {"pos": [round(v, 4) for v in pos], "rot": None, "yaw": yaw, "source": "entity"}

    def ship_pose(self) -> dict | None:
        pose = None
        if self.ship_unit:
            rot, pos = base.unit_state(self.ctx.mem, self.ship_unit)
            if pos and rot:
                pose = {"pos": [round(v, 4) for v in pos], "rot": [round(v, 6) for v in rot],
                        "yaw": round(base.quat_yaw(rot), 5), "source": "memory"}
        ent = self.entity_pose()
        if pose and ent:
            pose["yaw"] = ent["yaw"]
            pose["source"] = "memory"
            return pose
        return pose or ent

    def sample(self, use_entities: bool) -> dict:
        row = {"t_ms": now_ms(), "ship": self.ship_pose(), "player": self.ctx.pose()}
        if row["ship"] and row["player"]:
            yaw = row["ship"]["yaw"]
            fwd = (-math.sin(yaw), math.cos(yaw))
            right = (math.cos(yaw), math.sin(yaw))
            dx = row["player"]["pos"][0] - row["ship"]["pos"][0]
            dy = row["player"]["pos"][1] - row["ship"]["pos"][1]
            row["player_local"] = {"forward": round(dx * fwd[0] + dy * fwd[1], 4),
                                   "right": round(dx * right[0] + dy * right[1], 4),
                                   "up": round(row["player"]["pos"][2] - row["ship"]["pos"][2], 4)}
        row["speed"] = None
        prev = self.samples.get(self.case) or []
        if prev and row["ship"] and prev[-1].get("ship"):
            dt = (row["t_ms"] - prev[-1]["t_ms"]) / 1000.0
            if dt > 0.05:
                a, b = prev[-1]["ship"]["pos"], row["ship"]["pos"]
                row["speed"] = round(math.hypot(b[0] - a[0], b[1] - a[1]) / dt, 4)
        if use_entities:
            grid = self.grid()
            row["ship_ent"] = ent_row(grid.get(str(self.ship_engine)) if self.ship_engine else None)
            row["vehicle"] = vehicle(self.ctx.mcp, self.ctx.pid)
        self.samples.setdefault(self.case, []).append(dict(row))
        stamp = row.pop("t_ms")
        self.sl.state(self.case, "sample", t_ms=stamp, **row)
        return {"t_ms": stamp, **row}

    def poll(self) -> list[dict]:
        packets = self.ctx.trace.poll()
        if packets:
            self.sl.add(self.case, packets)
        return packets

    def input(self, key: str, ms: int) -> None:
        self.sl.state(self.case, "input", t_ms=now_ms(), key=key, ms=ms)
        self.ctx.mcp.text("client_input", pid=self.ctx.pid, key=key, duration_ms=ms)

    def run(self, case: str, duration_ms: int, actions=()) -> None:
        self.case = case
        self.sl.state(case, "start", ship=self.ship_pose(), player=self.ctx.pose(),
                      vehicle=vehicle(self.ctx.mcp, self.ctx.pid),
                      ship_info={k: self.ship.get(k) for k in
                                 ("template", "name", "tl_id", "obj", "unit", "engine_id")})
        self.samples.setdefault(case, [])
        actions = sorted(actions, key=lambda action: action[0])
        index = 0
        t0 = time.monotonic()
        next_sample = t0
        next_entity = t0
        end = t0 + duration_ms / 1000.0
        while time.monotonic() < end:
            now = time.monotonic()
            while index < len(actions) and (now - t0) * 1000.0 >= actions[index][0]:
                _, key, ms = actions[index]
                index += 1
                self.input(key, ms)
                now = time.monotonic()
            if now >= next_sample:
                use_entities = now >= next_entity
                self.sample(use_entities)
                next_sample += 0.1
                if use_entities:
                    next_entity += 1.0
            self.poll()
            time.sleep(0.01)
        self.poll()
        self.sl.state(case, "end", ship=self.ship_pose(), player=self.ctx.pose(),
                      vehicle=vehicle(self.ctx.mcp, self.ctx.pid))


def capture(ctx: Ctx, sl: Slice, ship: dict) -> None:
    cap = Cap(ctx, sl, ship)
    sl.note("S9 ship steering live capture of the original 1.2 client (r208022), hidden, "
            "one launch; account <account>, character from the argument; GM 100.")
    sl.note("Input is the client's own key path through aaemu-clientdriver client_input "
            "(no OS input): W/S throttle and A/D steering; boarding is the client's "
            "right-click pick (CSStartInteraction 0x068 -> skill 12076 -> CSBindSlave 0x031); "
            "leaving the helm is X2SiegeWeapon:DiscardSiegeWeapon (CSDiscardSlave 0x032).")
    sl.note("The lane must run aaemu_game::net::fields=trace plus aaemu_game::net=trace: "
            "the C->S field trace carries every CSMoveUnit 0x089 body and the S->C trace "
            "structs carry the type-3 velocity/rotation fields.")
    sl.note("The player pose is read from client memory every 100 ms; the rowboat hull has "
            "no unit model entity through that path, so its pose comes from client_entities "
            "(prefab_unit) and the S->C type-3 fields, with X2SiegeWeapon telemetry about "
            "once a second.")
    for case, (duration, actions) in CASE_ACTIONS.items():
        if case == "d_leave":
            cap.run(case, duration, actions)
            sl.state(case, "verb", vehicle=vehicle(ctx.mcp, ctx.pid))
            ctx.ui("X2SiegeWeapon:DiscardSiegeWeapon() return 1")
            time.sleep(0.3)
            sl.state(case, "verb_result", vehicle=vehicle(ctx.mcp, ctx.pid))
            cap.run("d_after", 2500)
        elif case == "d_after":
            continue
        else:
            cap.run(case, duration, actions)


def main() -> int:
    parser = argparse.ArgumentParser(
        epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("SHIP_CLIENT", "archeage.exe"))
    parser.add_argument("--driver", default=os.environ.get("SHIP_DRIVER",
                                                          "aaemu-clientdriver.exe"))
    parser.add_argument("--driver-dll", default=os.environ.get("SHIP_DRIVER_DLL", "aadriver.dll"))
    parser.add_argument("--launcher", default=os.environ.get("SHIP_LAUNCHER",
                                                             "aaemu-launcher.exe"))
    parser.add_argument("--login", default=os.environ.get("SHIP_LOGIN",
                                                          os.environ.get("AA_LOGIN",
                                                                         "127.0.0.1:1237")))
    parser.add_argument("--account", default=os.environ.get("SHIP_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("SHIP_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("SHIP_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("SHIP_SERVER_LOG"))
    parser.add_argument("--place", default=os.environ.get("SHIP_PLACE", "4318,4123,100.5"),
                        help="open-sea server position x,y,z")
    parser.add_argument("--template", type=int, default=int(os.environ.get("SHIP_TEMPLATE", ROWBOAT)))
    parser.add_argument("--pid", type=int,
                        help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "ship_steering"))
    parser.add_argument("--raw", default=os.environ.get("SHIP_RAW")
                        or str(ROOT / "data" / "extract"
                               / f"ship-steering-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set SHIP_{name.upper()})")
    try:
        place = [float(part) for part in args.place.replace(" ", ",").split(",") if part]
        if len(place) != 3:
            raise ValueError
    except ValueError:
        parser.error("--place must be x,y,z")
    client = Path(args.client)
    driver = Path(args.driver)
    dll = Path(args.driver_dll)
    launcher = Path(args.launcher)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    if not driver.exists() or not dll.exists() or not launcher.exists() or not password_file.exists():
        raise SystemExit("missing driver, driver dll, launcher or password file")
    if not server_log.exists():
        raise SystemExit(f"missing server log: {server_log}")
    out = Path(args.out)
    raw = Path(args.raw)
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)

    log(f"account {args.account}: {args.character} template {args.template} at {place}")
    mcp = base.Mcp(driver, dll, launcher, client, args.login, password_file,
                   args.account, raw / "mcp")
    pid = args.pid
    started_pid = None
    try:
        if pid is None:
            launch = mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                              user=args.account, password_file=str(password_file))
            log(launch.replace("\n", " | "))
            match = re.search(r"pid (\d+)", launch)
            if not match:
                raise RuntimeError("client_launch did not return a pid")
            pid = int(match.group(1))
            started_pid = pid
            cb.select_character(mcp, pid, args.character)
        else:
            base.wait_player(mcp, pid, 30)
        mem, addr = cb.attach(mcp, pid)
        trace = Trace(server_log)
        trace.reset()
        ctx = Ctx(mcp, pid, mem, addr, trace)
        sl = Slice(out, "s910", trace)
        failed = False
        try:
            base.wait_player(mcp, pid, 60)
            sl.state("setup", "in_world", player_unit=hex(ctx.player),
                     player=ctx.pose(), vehicle=vehicle(mcp, pid))
            ship = adopt_driver(ctx, sl)
            if ship is None:
                shift = measure_shift(ctx, sl, place, args.character)
                ctx.chat("/time set 13 30")
                time.sleep(1.5)
                ship = spawn_ship(ctx, sl, args.template, shift)
                if not ship:
                    raise RuntimeError(f"ship template {args.template} did not spawn")
                for distance in (5.0, 3.5, 2.0, 2.8):
                    pos = ctx.pos()
                    dx, dy = pos[0] - ship["pos"][0], pos[1] - ship["pos"][1]
                    length = math.hypot(dx, dy) or 1.0
                    want = [ship["pos"][0] + dx / length * distance,
                            ship["pos"][1] + dy / length * distance, ship["pos"][2]]
                    place_client(ctx, sl, want, shift, args.character)
                    ctx.face(ship["pos"])
                    ctx.settle()
                    sl.state("setup", "approach", distance=distance,
                             measured=round(dist3(ctx.pos(), ship["pos"]), 3))
                    if try_pick(ctx, sl, ship, ship.get("name") or ""):
                        break
                    time.sleep(0.5)
                sl.state("setup", "final", driver=ctx.is_driver(), vehicle=vehicle(mcp, pid),
                         ship=ship, player=ctx.pos())
                if not ctx.is_driver():
                    raise RuntimeError("the right-click board did not take")
            capture(ctx, sl, ship)
        except Exception as error:
            import traceback
            traceback.print_exc()
            sl.note(f"CAPTURE FAILED: {type(error).__name__}: {error}")
            sl.state("s", "failed", error=f"{type(error).__name__}: {error}")
            failed = True
        finally:
            sl.state("s", "end", ship=Cap(ctx, sl, ship).ship_pose() if ship else None,
                     player=ctx.pose(), vehicle=vehicle(mcp, pid), rows=len(sl.rows))
            sl.write()
            log("wrote", sl.out, "rows", len(sl.rows))
        if failed:
            return 1
    finally:
        if started_pid is not None:
            try:
                mcp.text("client_stop", pid=started_pid)
            except Exception:
                subprocess.run(["taskkill", "/F", "/PID", str(started_pid)],
                               capture_output=True)
            log("stopped client pid", started_pid)
        mcp.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
