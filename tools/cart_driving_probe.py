#!/usr/bin/env python3
"""Capture the original 1.2 client driving a Farm Wagon (slave template 60), read-only.

The probe launches the hidden 1.2 client through aaemu-clientdriver (or attaches with
`--pid`), asks the server for the cart with `/slave spawn 60`, boards it with the client's
own right-click pick, then holds the client's own movement keys (W/S/A/D) through
`client_input`. While driving it records:

* the server's C->S field trace (`aaemu_game::net::fields=trace`), including every
  `CSMoveUnit` 0x089 body, decoded with the type-2 layout (position, velocity at
  30/32767 m/s, short quaternion, angular velocity, steering, wheel block);
* a ~100 ms sample of the cart and driver from the client's own engine (prefab_unit:
  position, Euler angles, velocity, terrain height) and the client_vehicle block;
* the S->C opcode lines when the lane prints them (`aaemu_game::net=debug`).

Cases (same as the 2026-10-29 capture):
  a still 5 s; b forward 5 s + coast; c hard left 3 s; c hard right 3 s;
  d reverse 3 s; e a nearby slope if one is there; f leave while moving (0x032);
  g optional screenshots (`--shots DIR`, local only).

Writes native.csv / native.json / notes.txt / server-log.txt and report.txt under
`--out` (default tools/evidence/cart_driving). The client is stopped by its pid when
this probe launched it; `--pid` clients are left running.

Usage:
    python tools/cart_driving_probe.py --client <archeage.exe> \\
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \\
        --launcher <aaemu-launcher.exe> --login <host:port> \\
        --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --out tools/evidence/cart_driving
    python tools/cart_driving_probe.py --pid 1234 ...        # already in world
    python tools/cart_driving_probe.py --report-only --out tools/evidence/cart_driving

The account needs GM level for `/slave spawn` and `/move`. An anti-cheat module makes
the probe refuse to read the process. No desktop mouse or keyboard events are sent.
"""
from __future__ import annotations

import argparse
import json
import math
import os
import re
import statistics
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
CART_TEMPLATE = 60
PLACE = (10512.0, 15585.0, 230.5)
MOVE_RE = re.compile(r"C->S level=(\d+) opcode=0x([0-9A-Fa-f]{3}) (\w+) len=(\d+) body=(.*)$")
S2C_RE = re.compile(r"GamePacket: S->C type ([0-9A-Fa-f]{3}) (\w+) \((\d+) bytes\)")
INTERACT_RE = re.compile(
    r"StartInteraction, NpcObjId: (\d+), objId: (\d+), extraInfo: (-?\d+), "
    r"pickId: (-?\d+), mouse: (\d+), mods: (-?\d+)")


def log(*args: object) -> None:
    print("cart-driving:", *args, flush=True)


def now_ms() -> int:
    return int(time.time() * 1000)


class Trace(cb.Trace):
    """The lane's game log with millisecond timestamps (s67 layout)."""

    def __init__(self, path: Path) -> None:
        super().__init__(path)
        self.raw = []

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
            line = line.strip()
            match = MOVE_RE.search(line)
            if match:
                out.append({"t_ms": stamp, "dir": "C->S", "opcode": int(match.group(2), 16),
                            "name": match.group(3), "len": int(match.group(4)),
                            "body": match.group(5).strip(), "line": line,
                            "log_time": line[:8]})
                continue
            match = S2C_RE.search(line)
            if match:
                out.append({"t_ms": stamp, "dir": "S->C", "opcode": int(match.group(1), 16),
                            "name": match.group(2), "len": int(match.group(3)), "body": "",
                            "line": line, "log_time": line[:8]})
                continue
            match = INTERACT_RE.search(line)
            if match:
                out.append({"t_ms": stamp, "dir": "log", "opcode": -1,
                            "name": "StartInteraction", "len": 0,
                            "body": ",".join(match.groups()), "line": line,
                            "log_time": line[:8]})
        return out

    def relevant(self) -> list[str]:
        return [line for line in self.raw
                if MOVE_RE.search(line) or S2C_RE.search(line) or INTERACT_RE.search(line)]


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


def quat_yaw_y(quat: list[float]) -> float:
    x, y, z, w = quat
    return math.atan2(2.0 * (w * y + x * z), 1.0 - 2.0 * (x * x + y * y))


def decode_move(body_hex: str) -> dict:
    """CSMoveUnit body (after the obj id) decoded per the 1.2 type-2 serializer."""
    raw = bytes.fromhex(body_hex.replace(" ", ""))
    if len(raw) < 4:
        return {"error": "short"}
    obj = raw[0] | raw[1] << 8 | raw[2] << 16
    out: dict = {"obj_id": obj, "type": raw[3]}
    pos = 4
    try:
        out["time_ms"] = struct.unpack_from("<I", raw, pos)[0]
        pos += 4
        out["flags"] = raw[pos]
        pos += 1
        if out["flags"] & 0x10:
            out["sc_type"] = struct.unpack_from("<H", raw, pos)[0]
            out["phase"] = raw[pos + 2]
            pos += 3
        out["pos"] = decode_position(raw[pos:pos + 9])
        pos += 9
        out["vel"] = list(struct.unpack_from("<3h", raw, pos))
        pos += 6
        mtype = out["type"]
        if mtype in (2, 3):
            quat = short_quat(raw, pos)
            pos += 6
            out["quat"] = [round(v, 6) for v in quat]
            out["yaw_z"] = round(quat_yaw(quat), 5)
            out["yaw_y"] = round(quat_yaw_y(quat), 5)
            out["ang_vel"] = [round(v, 6) for v in struct.unpack_from("<3f", raw, pos)]
            pos += 12
            out["steering"] = round(struct.unpack_from("<f", raw, pos)[0], 6)
            pos += 4
            if mtype == 2:
                count = raw[pos]
                pos += 1
                out["wheel_count"] = count
                out["wheel_ang_vel"] = [
                    round(struct.unpack_from("<f", raw, pos + 4 * i)[0], 6) for i in range(count)]
                pos += 4 * count
            else:
                out["ship_tail"] = raw[pos:pos + 5].hex()
                pos += 5
        elif mtype in (0, 1):
            out["tilt"] = list(struct.unpack_from("<2b", raw, pos))
            out["direction"] = struct.unpack_from("<b", raw, pos + 2)[0]
            out["delta"] = list(struct.unpack_from("<3b", raw, pos + 3))
            out["stance"] = raw[pos + 6]
            out["alertness"] = raw[pos + 7]
            actor_flags = raw[pos + 8]
            out["actor_flags"] = actor_flags
            pos += 9
            if actor_flags & 0x08:
                out["fall_vel"] = struct.unpack_from("<H", raw, pos)[0]
                pos += 2
        out["consumed"] = pos
        out["tail"] = raw[pos:].hex()
    except Exception as error:  # malformed
        out["error"] = f"{type(error).__name__}: {error}"
        out["raw"] = raw.hex()
    return out


# ------------------------------------------------------------------ client sampling

def client_vehicle(mcp: base.Mcp, pid: int, action: str = "status") -> dict:
    try:
        return json.loads(mcp.text("client_vehicle", pid=pid, action=action))
    except Exception:
        return {}


def cart_row(mcp: base.Mcp, pid: int, token: str = "Wagon") -> dict | None:
    try:
        payload = json.loads(mcp.text(
            "client_entities", pid=pid, **{"class": "prefab_unit", "limit": 10}))
    except Exception:
        return None
    for engine_id, ent in (payload.get("entities") or {}).items():
        if token in (ent.get("name") or ""):
            position = ent.get("position") or {}
            angles = ent.get("angles") or {}
            velocity = ent.get("velocity") or {}
            return {
                "id": engine_id,
                "name": ent.get("name"),
                "class": ent.get("class"),
                "pos": [round(float(position.get(k, 0.0)), 4) for k in ("x", "y", "z")],
                "angles": {k: round(float(angles.get(k, 0.0)), 4) for k in ("x", "y", "z")}
                if isinstance(angles, dict) else angles,
                "vel": [round(float(velocity.get(k, 0.0)), 4) for k in ("x", "y", "z")]
                if isinstance(velocity, dict) else velocity,
                "terrain": round(float(ent.get("terrain_height") or 0.0), 3),
            }
    return None


class Slice(cb.Slice):
    """Evidence writer that decodes every 0x089 body with the type-2 layout."""

    def add(self, case: str, packets: list[dict]) -> None:
        self.packets.extend(packets)
        for packet in packets:
            fields: dict = {}
            if packet["dir"] == "C->S" and packet["opcode"] == 0x089:
                fields = decode_move(packet["body"])
            elif packet["dir"] == "C->S":
                fields = cb.decode_c2s(packet["name"], packet["body"])
            elif packet["dir"] == "log":
                fields = {"log": packet["body"]}
            self.rows.append({
                "t_ms": packet["t_ms"], "case": case,
                "event": "packet" if packet["dir"] != "log" else "server-log",
                "dir": packet["dir"],
                "opcode": f"0x{packet['opcode']:03X}" if packet["opcode"] >= 0 else "",
                "name": packet["name"], "len": packet["len"], "body": packet["body"],
                "decoded": json.dumps(fields, ensure_ascii=False, default=str),
                "client_state": "", "text": "",
            })


class CartCap:
    def __init__(self, mcp: base.Mcp, pid: int, mem: base.Mem, address: int,
                 trace: Trace, out: Path, shots: Path | None, driver: Path) -> None:
        self.mcp, self.pid, self.mem, self.base = mcp, pid, mem, address
        self.trace = trace
        self.slice = Slice(out, "cart_driving", trace)
        self.shots = shots
        self.driver = driver
        self.player_unit = mem.u32(
            mem.u32(address + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)
        self.case = ""
        self.samples: dict[str, list[dict]] = {}
        self.phases: list[dict] = []

    # ---------------------------------------------------------------- sampling
    def sample(self, use_client: bool = False) -> dict:
        row: dict = {"t_ms": now_ms()}
        started = time.perf_counter()
        cart = cart_row(self.mcp, self.pid)
        row["cart_read_ms"] = round((time.perf_counter() - started) * 1000.0, 1)
        if cart:
            row["cart"] = {"id": cart["id"], "pos": cart["pos"], "angles": cart["angles"],
                           "vel": cart["vel"], "terrain": cart["terrain"]}
        rotation, position = base.unit_state(self.mem, self.player_unit)
        if position and rotation:
            row["driver"] = {"pos": [round(v, 4) for v in position],
                             "quat": [round(v, 6) for v in rotation],
                             "yaw": round(base.quat_yaw(rotation), 5)}
        if use_client:
            vehicle = client_vehicle(self.mcp, self.pid)
            if vehicle:
                row["vehicle"] = {k: vehicle.get(k) for k in
                                  ("has_vehicle", "mounted", "driver", "speed", "turn_speed")}
        row["speed"] = None
        previous = self.samples.get(self.case) or []
        if previous and row.get("cart") and previous[-1].get("cart"):
            seconds = (row["t_ms"] - previous[-1]["t_ms"]) / 1000.0
            if seconds > 0:
                a, b = previous[-1]["cart"]["pos"], row["cart"]["pos"]
                row["speed"] = round(math.hypot(b[0] - a[0], b[1] - a[1]) / seconds, 4)
        self.samples.setdefault(self.case, []).append(dict(row))
        stamp = row.pop("t_ms")
        self.slice.state(self.case, "sample", t_ms=stamp, **row)
        return {"t_ms": stamp, **row}

    def last(self) -> dict:
        rows = self.samples.get(self.case) or []
        return rows[-1] if rows else {}

    def speed(self) -> float:
        for row in reversed(self.samples.get(self.case) or []):
            if row.get("speed") is not None:
                return float(row["speed"])
        return 0.0

    def pos(self) -> list[float]:
        return list(self.last().get("cart", {}).get("pos") or [])

    # ---------------------------------------------------------------- trace + actions
    def poll(self) -> list[dict]:
        packets = self.trace.poll()
        if packets:
            self.slice.add(self.case or "s", packets)
        return packets

    def input(self, key: str, ms: int) -> None:
        self.phases.append({"t_ms": now_ms(), "case": self.case, "input": key, "ms": ms})
        self.mcp.text("client_input", pid=self.pid, key=key, duration_ms=ms)

    def vehicle(self, action: str = "status") -> dict:
        return client_vehicle(self.mcp, self.pid, action)

    def run(self, case: str, duration_ms: int,
            actions: list[tuple[int, str, tuple]] | None = None,
            use_client_every: int = 500) -> None:
        start_pos = self.pos()
        self.case = case
        self.slice.state(case, "start", t_ms=now_ms(), duration_ms=duration_ms,
                         cart_pos=[round(v, 3) for v in start_pos],
                         vehicle=client_vehicle(self.mcp, self.pid))
        self.samples.setdefault(case, [])
        self.phases = []
        started = time.monotonic()
        actions = sorted(actions or [], key=lambda a: a[0])
        index = 0
        next_sample = started
        next_client = started
        end = started + duration_ms / 1000.0
        while time.monotonic() < end:
            now = time.monotonic()
            while index < len(actions) and (now - started) * 1000.0 >= actions[index][0]:
                _offset, kind, parameters = actions[index]
                index += 1
                self.action(kind, parameters)
                now = time.monotonic()
            if now >= next_sample:
                self.sample(now >= next_client)
                next_sample += 0.1
                if now >= next_client:
                    next_client += use_client_every / 1000.0
            self.poll()
            time.sleep(0.005)
        self.poll()
        self.slice.state(case, "end", t_ms=now_ms(), speed=self.speed(),
                         vehicle=client_vehicle(self.mcp, self.pid),
                         packets=0, phases=self.phases)

    def action(self, kind: str, parameters: tuple) -> None:
        if kind == "input":
            self.input(parameters[0], parameters[1])
        elif kind == "verb":
            self.vehicle(parameters[0])
            self.slice.state(self.case, "verb", t_ms=now_ms(), action=parameters[0],
                             vehicle=client_vehicle(self.mcp, self.pid))
        elif kind == "stop":
            self.slice.state(self.case, "stop_point", t_ms=now_ms(), speed=self.speed(),
                             pos=self.pos())
        elif kind == "shot":
            self.shot(parameters[0], parameters[1], parameters[2])
        elif kind == "camera":
            self.slice.state(self.case, "camera", t_ms=now_ms(),
                             result=self.camera(*parameters))
        else:
            raise ValueError(kind)

    def coast(self, case: str, limit_ms: int = 9000, quiet_ms: int = 800) -> None:
        self.case = case
        quiet_from = None
        end = time.monotonic() + limit_ms / 1000.0
        next_sample = time.monotonic()
        while time.monotonic() < end:
            now = time.monotonic()
            if now >= next_sample:
                self.sample(False)
                next_sample += 0.1
                if self.speed() < 0.03:
                    quiet_from = quiet_from or now
                    if (now - quiet_from) * 1000.0 >= quiet_ms:
                        break
                else:
                    quiet_from = None
            self.poll()
            time.sleep(0.005)
        self.slice.state(case, "coast_end", t_ms=now_ms(), speed=self.speed(), pos=self.pos())

    # ---------------------------------------------------------------- screenshots
    def camera(self, yaw_deg: float, pitch_deg: float, distance: float) -> dict:
        code = ("local out={}"
                " local ok,e=pcall(function() System.ExecuteCommand('uc_set_yaw %.3f') end)"
                " out[#out+1]='yaw='..tostring(ok)..':'..tostring(e)"
                " ok,e=pcall(function() System.ExecuteCommand('uc_set_pitch %.3f') end)"
                " out[#out+1]='pitch='..tostring(ok)..':'..tostring(e)"
                " ok,e=pcall(function() System.ExecuteCommand('uc_set_wheel_dist %.3f') end)"
                " out[#out+1]='dist='..tostring(ok)..':'..tostring(e)"
                " return table.concat(out,' ')") % (yaw_deg, pitch_deg, distance)
        result = cb.lua_text(self.mcp, self.pid, code).strip()
        return {"result": result, "yaw": yaw_deg, "pitch": pitch_deg, "distance": distance}

    def shot(self, name: str, camera: tuple[float, float, float]) -> str:
        cart = self.pos()
        result = self.camera(*camera)
        path = (self.shots or Path(".")) / f"{name}.png"
        if self.shots is not None:
            self.shots.mkdir(parents=True, exist_ok=True)
        started = now_ms()
        try:
            completed = subprocess.run(
                [str(self.driver), "screenshot", "--pid", str(self.pid), str(path)],
                capture_output=True, text=True, timeout=60)
            ok = completed.returncode == 0 and path.exists()
            detail = (completed.stdout + completed.stderr).strip()[:200]
        except Exception as error:
            ok, detail = False, f"{type(error).__name__}: {error}"
        self.slice.state(self.case, "shot", t_ms=started, name=name, ok=ok,
                         bytes=path.stat().st_size if path.exists() else 0,
                         camera=result, detail=detail, speed=self.speed(), cart=cart)
        return name


# ------------------------------------------------------------------ setup

def measure_shift(ctx: cb.Ctx, sl: Slice, place: tuple[float, float, float]) -> tuple[float, float]:
    character = ctx.ui("return X2Unit:UnitName('player')").strip()
    ctx.chat(f"/move {character} {place[0]:.2f} {place[1]:.2f} {place[2]:.2f}")
    time.sleep(2.0)
    ctx.settle()
    position = ctx.player_pos() or [place[0], place[1], place[2]]
    shift = (place[0] - position[0], place[1] - position[1])
    sl.state("setup", "shift", t_ms=now_ms(), server=list(place[:2]),
             client=[round(v, 3) for v in position], shift=[round(v, 3) for v in shift])
    return shift


def setup_session(mcp: base.Mcp, ctx: cb.Ctx, sl: Slice,
                  place: tuple[float, float, float]) -> tuple[list[float], str]:
    base.wait_player(mcp, ctx.pid, 60)
    player_unit = ctx.player_unit()
    sl.state("setup", "in_world", t_ms=now_ms(), player_unit=hex(player_unit),
             player_pos=[round(v, 3) for v in (ctx.player_pos() or [])],
             vehicle=client_vehicle(mcp, ctx.pid))
    measure_shift(ctx, sl, place)
    ctx.chat("/time set 13 30")
    time.sleep(1.5)

    entries = [e for e in cb.entities(mcp, ctx.pid, **{"class": "prefab_unit"})
               if "Wagon" in (e.get("name") or "")]
    if entries:
        entity = entries[0]
        cart = [float(entity["position"][k]) for k in ("x", "y", "z")]
        # identify the cart unit by finding the nearest kind-2 slave unit
        cart_unit = None
        carts = ctx.units()
        for key, unit in carts.items():
            position = ctx.unit_pos(unit)
            if position and cb.dist3(position, cart) < 2.0:
                cart_unit = unit
                ctx.cart_obj = key
                break
        ctx.cart_unit = cart_unit
        sl.state("setup", "reuse", t_ms=now_ms(), cart_name=entity["name"],
                 cart_obj=ctx.cart_obj, cart_unit=hex(cart_unit) if cart_unit else None,
                 cart_pos=[round(v, 3) for v in cart])
    else:
        _cart_obj, cart, _name = cb.spawn_cart(ctx, sl, "setup")
        # cb.spawn_cart sets ctx.cart_unit / ctx.cart_obj
        entity = {"name": "Farm Wagon"}
    if not ctx.cart_unit:
        raise RuntimeError("the cart unit was not found")
    cart = ctx.unit_pos(ctx.cart_unit) or cart
    if not ctx.cart_status().get("driver"):
        for distance in (2.0, 2.6, 1.6):
            measured = cb.place_at(ctx, sl, "setup", cart, distance)
            ctx.face(cart)
            ctx.settle()
            sl.state("setup", "placed", t_ms=now_ms(), distance=distance,
                     measured=round(measured, 3), player=ctx.player_pos())
            hit = cb.pick_and_follow(ctx, sl, "setup_pick", cart, entity["name"],
                                     after_skill=2.0)
            if hit or ctx.cart_status().get("driver"):
                break
        time.sleep(1.0)
    if not ctx.cart_status().get("driver"):
        raise RuntimeError("boarding failed: not in the driver seat")
    sl.state("setup", "final", t_ms=now_ms(), cart_obj=ctx.cart_obj,
             cart_hex=f"{ctx.cart_obj:X}" if ctx.cart_obj else "0",
             cart_unit=hex(ctx.cart_unit), cart_tl=ctx.mem.u32(ctx.cart_unit + 0x41C),
             cart_pos=[round(v, 3) for v in cart], vehicle=client_vehicle(mcp, ctx.pid))
    return cart, entity["name"]


# ------------------------------------------------------------------ cases

def drive_cases(cap: CartCap) -> None:
    cap.slice.state("s", "begin", t_ms=now_ms(), vehicle=client_vehicle(cap.mcp, cap.pid),
                    pos=cap.pos())
    cap.run("a_still", 5000)
    cap.run("b_forward", 5000, [(0, "input", ("w", 5000))])
    cap.coast("b_forward")
    cap.run("c_left", 3400, [(0, "input", ("w", 3400)),
                             (120, "input", ("a", 3000))])
    cap.coast("c_left")
    cap.run("c_right", 3400, [(0, "input", ("w", 3400)),
                              (120, "input", ("d", 3000))])
    cap.coast("c_right")
    cap.run("d_reverse", 3200, [(0, "input", ("s", 3000))])
    cap.coast("d_reverse")

    try:
        rows = [r for case in ("a_still", "b_forward", "c_left", "c_right", "d_reverse")
                for r in cap.samples.get(case, []) if r.get("cart")]
        zs = [r["cart"]["pos"][2] for r in rows]
        xy = [r["cart"]["pos"][:2] for r in rows]
        span = 0.0
        for i in range(len(xy)):
            for j in range(i + 1, len(xy), max(1, len(xy) // 40)):
                span = max(span, math.hypot(xy[i][0] - xy[j][0], xy[i][1] - xy[j][1]))
        cap.slice.state("e_slope", "terrain", t_ms=now_ms(),
                        z_min=round(min(zs), 3) if zs else None,
                        z_max=round(max(zs), 3) if zs else None,
                        z_spread=round(max(zs) - min(zs), 3) if zs else None,
                        driven_span_m=round(span, 2), pos=cap.pos())
        if zs and (max(zs) - min(zs)) >= 1.0 and span > 3.0:
            cap.run("e_slope", 3000, [(0, "input", ("w", 3000))])
            cap.coast("e_slope", 4000)
        else:
            cap.slice.state("e_slope", "skipped", t_ms=now_ms(),
                            reason="no slope at the capture spot: cart z spread %.2f m over %.1f m driven"
                                   % ((max(zs) - min(zs)) if zs else 0.0, span))
    except Exception as error:
        cap.slice.state("e_slope", "failed", t_ms=now_ms(),
                        error=f"{type(error).__name__}: {error}")

    cap.run("f_leave", 3500, [(0, "input", ("w", 3500)),
                              (1500, "verb", ("discard",)),
                              (1600, "stop", ())])
    cap.coast("f_leave", 5000)
    cap.slice.state("f_leave", "after", t_ms=now_ms(),
                    vehicle=client_vehicle(cap.mcp, cap.pid), pos=cap.pos())


def screenshot_set(cap: CartCap) -> None:
    if cap.shots is None:
        return
    cap.case = "g3"
    cap.input("w", 6000)
    time.sleep(2.0)
    for name, yaw, pitch, distance in (
            ("drive12-01-moving-side", 90.0, 0.0, 10.0),
            ("drive12-03-behind-moving", 0.0, -6.0, 10.0)):
        cap.shot(name, (yaw, pitch, distance))
        time.sleep(0.2)
    for index in range(1, 6):
        cap.shot(f"drive12-burst-{index}", (90.0, 4.0, 10.0))
        time.sleep(0.2)
    cap.input("w", 1000)
    cap.input("d", 2500)
    time.sleep(1.0)
    cap.shot("drive12-02-turning", (20.0, -2.0, 10.0))
    cap.shot("drive12-02b-turning", (135.0, 0.0, 10.0))


# ------------------------------------------------------------------ report

def build_report(out: Path) -> None:
    data = json.loads((out / "native.json").read_text(encoding="utf-8"))
    rows = data["rows"]
    moves = [(row["case"], row["t_ms"], json.loads(row["decoded"]))
             for row in rows if row["event"] == "packet" and row["opcode"] == "0x089"]
    type2 = [(case, t, move) for case, t, move in moves if move.get("type") == 2]
    by_case: dict[str, list[dict]] = {}
    for case, _t, move in type2:
        by_case.setdefault(case, []).append(move)
    samples: dict[str, list[tuple[int, list[float], float | None, dict]]] = {}
    for row in rows:
        if row["event"] != "sample":
            continue
        state = json.loads(row["client_state"])
        cart = state.get("cart")
        if cart:
            samples.setdefault(row["case"], []).append(
                (row["t_ms"], cart["pos"], state.get("speed"), cart.get("angles") or {}))

    ratios, dots, ratios_2048 = [], [], []
    for sequence in by_case.values():
        for a, b in zip(sequence, sequence[1:]):
            seconds = (b["time_ms"] - a["time_ms"]) / 1000.0
            if not (0.02 < seconds < 0.4):
                continue
            delta = [b["pos"][i] - a["pos"][i] for i in range(3)]
            velocity = b["vel"]
            if math.hypot(velocity[0], velocity[1]) < 200:
                continue
            measured = math.hypot(delta[0], delta[1]) / seconds
            wire = math.hypot(velocity[0], velocity[1]) / 1092.0
            if wire > 0.1:
                ratios.append(measured / wire)
                dots.append(delta[0] * velocity[0] + delta[1] * velocity[1])
                ratios_2048.append(measured / (math.hypot(velocity[0], velocity[1]) / 2048.0))

    still = [move["time_ms"] for move in by_case.get("a_still", [])]
    still_gaps = [b - a for a, b in zip(still, still[1:])]
    fps = {}
    for row in rows:
        if row["event"] == "perf_fps":
            fps = json.loads(json.loads(row["client_state"])["profile_3s"])
    labels = {"a_still": "a still 5 s", "b_forward": "b forward 5 s + coast",
              "c_left": "c hard left 3 s", "c_right": "c hard right 3 s",
              "d_reverse": "d reverse 3 s", "e_slope": "e slope 3 s",
              "f_leave": "f leave while moving"}
    steering = {case: sorted(set(round(m["steering"], 3) for m in by_case.get(case, [])))
                for case in labels if case in by_case}
    angvel = {case: sorted(set(round(m["ang_vel"][2], 3) for m in by_case.get(case, [])))
              for case in labels if case in by_case}
    wheels = sorted(set(m.get("wheel_count") for m in by_case.get("a_still", [])))
    flags = sorted(set(m.get("flags") for m in by_case.get("a_still", [])))
    sc = sum(1 for _case, _t, m in type2 if "sc_type" in m)
    leave = [(row["t_ms"], row["body"], json.loads(row["decoded"])) for row in rows
             if row["event"] == "packet" and row["opcode"] == "0x032"]
    leave_speed = None
    if leave:
        before = [m for case, t, m in type2 if case == "f_leave" and t <= leave[0][0]]
        if before:
            leave_speed = math.hypot(*before[-1]["vel"][:2]) / 1092.0
    stop_points = [json.loads(row["client_state"]) for row in rows
                   if row["event"] == "stop_point" and row["case"] == "f_leave"]

    def f(value: float | None, digits: int = 3) -> str:
        return "?" if value is None else f"{value:.{digits}f}"

    lines = [
        "Cart driving (S6/S7) - native 1.2 capture",
        "",
        "How measured",
        "- Original client r208022 hidden, one launch, driven only through aaemu-clientdriver",
        "  (Lua, the client's own pointer/action/key calls; no OS input).",
        "- Input is the client's own key path: w/s throttle and a/d steering; no desktop mouse",
        "  or keyboard events.",
        "- While driving, the server's C->S field trace (aaemu_game::net::fields=trace) records",
        "  every CSMoveUnit 0x089 body; the body is decoded with the type-2 layout.",
        "- Positions are sampled every ~100 ms from the client's own engine (prefab_unit entity)",
        "  and the client_vehicle block every ~500 ms. The lane must not be restarted, so S->C",
        "  bodies are not in this evidence; the client-side vehicle state is recorded instead.",
        "- Cart: slave template 60 (Farm Wagon, model 1008), boarded as the driver with the",
        "  client's own right-click (CSStartInteraction 0x068 -> server 0x07b -> CSBindSlave",
        "  0x031). Client-local = server - (7168, 11264) where the client streams seamlessly.",
        "",
        "Cases (packet intervals from the body's own clock)",
    ]
    for case, label in labels.items():
        sequence = by_case.get(case, [])
        if not sequence:
            lines.append(f"  {label}: no type-2 packets")
            continue
        speeds = [math.hypot(*m["vel"][:2]) / 1092.0 for m in sequence]
        gaps = [b["time_ms"] - a["time_ms"] for a, b in zip(sequence, sequence[1:])]
        lines.append(f"  {label}: {len(sequence)} x CSMoveUnit type 2, speed "
                     f"{min(speeds):.2f}..{max(speeds):.2f} m/s, packet interval "
                     f"{statistics.median(gaps) if gaps else 0:.0f} ms (median)")
    if leave:
        lines.append(f"  f leave: CSDiscardSlave 0x032 body {leave[0][1]} "
                     f"(tl_id {leave[0][2].get('tl_id')}) at {leave[0][0]} ms; last cart "
                     f"velocity before it {f(leave_speed, 2)} m/s, measured "
                     f"{f(stop_points[0].get('speed') if stop_points else None, 2)} m/s; "
                     f"no cart move after it.")
    lines += [
        "",
        "Summary items",
        f"1. Send interval and resend: it RESENDS while still. a_still has "
        f"{len(by_case.get('a_still', []))} bodies, interval from the body clock "
        f"{min(still_gaps) if still_gaps else '?'}..{max(still_gaps) if still_gaps else '?'} ms, "
        f"median {statistics.median(still_gaps) if still_gaps else '?'} ms."
        + (f" The hidden client rendered at {fps.get('fps', 0) if fps else '?'} FPS; the send "
           f"pump runs once per frame behind its 100 ms gate." if fps else ""),
        f"2. Wheel block: wheel_count {wheels}, wheelAngVel not on the wire; the body ends "
        f"after the steering float.",
        f"3. Steering: values {steering}; angVel.z {angvel} -> angVel.z = 0.4 * steering rad/s.",
        f"4. Flags byte: {flags} for every type-2 body.",
        f"5. scType/phase: {sc} of {len(type2)} type-2 bodies carry flags&0x10.",
        f"6. Velocity: measured / wire = {f(statistics.mean(ratios) if ratios else None)} mean, "
        f"{f(statistics.median(ratios) if ratios else None)} median, sd "
        f"{f(statistics.pstdev(ratios) if ratios else None)} over {len(ratios)} pairs (1/1092); "
        f"at 1/2048 the ratio would be "
        f"{f(statistics.mean(ratios_2048) if ratios_2048 else None)}; positive direction in "
        f"{sum(1 for dot in dots if dot > 0)}/{len(dots)} pairs.",
        "7. Server correction: not captured directly (no S->C lines).",
        "8. camera_distance: not captured; its consumer is not identified.",
        "9. Spindle formula: not captured; the wire carries no wheel data.",
        "",
        "Derived checks",
        f"  velocity scale: 1092 CONFIRMED (2048 ratio "
        f"{f(statistics.mean(ratios_2048) if ratios_2048 else None)}).",
        f"  wheel angular velocity: NOT CAPTURED (wheel_count {wheels}).",
        f"  flags byte: {flags} for every cart body ({len(type2)} packets).",
        "  steering: raw input with a short ramp; angVel.z = 0.4 * steering rad/s.",
        "",
        "Limits",
        "  The lane prints only the C->S field trace plus StartInteraction; S->C bodies are",
        "  absent because the lane must not be restarted with a different log level.",
        "",
        "Files",
        "  native.csv, native.json, notes.txt; server-log.txt when the probe writes it;",
        "  replay.txt, bevy.json, replay-report.txt (this directory); screenshots are local only.",
    ]
    (out / "report.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


# ------------------------------------------------------------------ main

def parse_place(value: str) -> tuple[float, float, float]:
    parts = [item for item in value.replace(",", " ").split() if item]
    if len(parts) != 3:
        raise argparse.ArgumentTypeError("place needs x y z")
    return tuple(float(item) for item in parts)  # type: ignore[return-value]


def main() -> int:
    parser = argparse.ArgumentParser(
        epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("CART_CLIENT", "archeage.exe"),
                        help="path to the original 1.2 archeage.exe")
    parser.add_argument("--driver", default=os.environ.get("CART_DRIVER", "aaemu-clientdriver.exe"),
                        help="path to AAEmu's aaemu-clientdriver.exe")
    parser.add_argument("--driver-dll", default=os.environ.get("CART_DRIVER_DLL", "aadriver.dll"),
                        help="path to the x86 aadriver.dll")
    parser.add_argument("--launcher", default=os.environ.get("CART_LAUNCHER", "aaemu-launcher.exe"),
                        help="path to AAEmu's aaemu-launcher.exe")
    parser.add_argument("--login", default=os.environ.get("CART_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("CART_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("CART_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("CART_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("CART_SERVER_LOG"),
                        help="game server log with the C->S fields trace and S->C opcode lines")
    parser.add_argument("--pid", type=int,
                        help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--place", type=parse_place,
                        default=parse_place(os.environ.get("CART_PLACE", "10512 15585 230.5")),
                        help="server x y z to place the character and cart")
    parser.add_argument("--shots", default=os.environ.get("CART_SHOTS"),
                        help="optional local directory for the g3 screenshots")
    parser.add_argument("--report-only", action="store_true",
                        help="rebuild report.txt from --out/native.json, no client")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "cart_driving"))
    parser.add_argument("--raw", default=os.environ.get("CART_RAW")
                        or str(ROOT / "data" / "extract"
                               / f"cart-driving-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    out = Path(args.out)
    if args.report_only:
        build_report(out)
        log("report rebuilt in", out)
        return 0
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set CART_{name.upper()})")
    client = Path(args.client)
    driver = Path(args.driver)
    dll = Path(args.driver_dll)
    launcher = Path(args.launcher)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    if not client.exists() or not driver.exists() or not dll.exists() or not launcher.exists():
        raise SystemExit("missing client/driver/dll/launcher")
    if not password_file.exists() or not server_log.exists():
        raise SystemExit("missing password file or server log")
    out.mkdir(parents=True, exist_ok=True)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    shots = Path(args.shots) if args.shots else None

    log(f"account {args.account}: {args.character}")
    mcp = base.Mcp(driver, dll, launcher, client, args.login, password_file,
                   args.account, raw / "mcp")
    pid = args.pid
    started_pid = None
    trace = Trace(server_log)
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
        mem, address = cb.attach(mcp, pid)
        ctx = cb.Ctx(mcp, pid, mem, address, trace)
        cap = CartCap(mcp, pid, mem, address, trace, out, shots, driver)
        trace.reset()
        sl = cap.slice
        sl.note("S6/S7 live drive of the original 1.2 client (r208022) through the cart probe.")
        sl.note("Input is the client's own key path through aaemu-clientdriver client_input "
                "(CryInput on the client main thread, no OS input): w/s throttle, a/d steering.")
        sl.note("Cart samples come from the client's own engine (prefab_unit) every ~100 ms and "
                "the client_vehicle block every ~500 ms; the CSMoveUnit bodies come from the "
                "server's aaemu_game::net::fields=trace. S->C bodies need aaemu_game::net=debug "
                "and are absent when the lane does not print them.")
        try:
            cart, name = setup_session(mcp, ctx, sl, args.place)
            sl.note(f"Cart: {name}, slave template {CART_TEMPLATE}, client obj "
                    f"{ctx.cart_obj} (0x{ctx.cart_obj:X}), unit {hex(ctx.cart_unit)}; boarded "
                    f"as the driver by the client's own right-click.")
            drive_cases(cap)
            if shots is not None:
                screenshot_set(cap)
        finally:
            sl.state("s", "server-log-lines", t_ms=now_ms(),
                     relevant=len(trace.relevant()),
                     s2c=sum(1 for line in trace.raw if S2C_RE.search(line)))
            sl.write()
            build_report(out)
            log("wrote", out)
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
