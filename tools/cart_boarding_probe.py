#!/usr/bin/env python3
"""Capture the original 1.2 client's cart (slave) boarding and leaving path, read-only.

The 1.2 client answers a right click on a slave with `CSChangeTarget` 0x02c and
`CSStartInteraction` 0x068 (`FUN_390f3780` -> `FUN_393be030`); the server's
`SCNpcInteractionSkillList` 0x07b is acted on at once (`FUN_393be850` ->
`FUN_393be280`), and skill 12076 ("occupy") is the bind path (`FUN_393bf310` ->
`FUN_393464a0` -> `CSBindSlave` 0x031). Leaving is
`X2SiegeWeapon:DiscardSiegeWeapon` (`FUN_394aa090` -> `FUN_39345aa0` ->
`CSDiscardSlave` 0x032), refused above 5 m/s with 0x67.

This probe drives the hidden original through aaemu-clientdriver (Lua, the
client's own pointer/action/key APIs, no OS input), reads player and cart
positions with ReadProcessMemory (PROCESS_VM_READ only), and records the
server's C->S field trace plus the S->C opcode lines (type, name, size and the
log time) in `native.csv` / `native.json` / `notes.txt` / `server-log.txt`.

Cases (one client, one cart, `/slave spawn 60`):
  a. right click at ~2 m: 0x02c, 0x068, 0x07b [12076], 0x031 body, 0x064/0x076.
  b. leave with `X2SiegeWeapon:DiscardSiegeWeapon`: 0x032 body and replies.
  c. right click at ~3.5, ~4.5, ~3.1 and ~2.9 m: which sends 0x068, whether
     the client auto-walks after 0x07b (positions every 100 ms for 3 s), then
     0x031 or a system message; each bound case is left again.
  d. board, drive forward with the client's own W key for 2.5 s, then leave
     while moving: 0x032 or 0x67 with the speed at that moment.
  e. right click again while bound: message/packet.
  f. a non-owner/passenger case needs a second client and is skipped.

Usage:
    python tools/cart_boarding_probe.py --client <archeage.exe> \\
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \\
        --launcher <aaemu-launcher.exe> --login <host:port> \\
        --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --out tools/evidence/cart_boarding
    python tools/cart_boarding_probe.py --pid 1234 ...   # already in world

The game server must log the fields trace and the S->C opcode lines:
`RUST_LOG=info,aaemu_game::net::fields=trace,aaemu_game::net=debug`.
The client is launched hidden; the script stops the client it started by pid.
The account needs GM level for `/slave spawn`. An anti-cheat module makes the
probe refuse to read the process.
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

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mount_reason_probe as base

ROOT = Path(__file__).resolve().parents[1]
CART = 60
SCREEN_W, SCREEN_H = 2554, 1411
BIND = 0x031
UNBIND = 0x032
START_INTERACTION = 0x068
CHANGE_TARGET = 0x02C
S2C_OF_INTEREST = {0x061, 0x064, 0x066, 0x069, 0x076, 0x077, 0x07B, 0x10A}

CAPTURE_INSTALL = r"""
_CAP = {}
local function _cap_add(name, ...)
  local parts = {}
  for i = 1, select('#', ...) do
    local value = select(i, ...)
    local kind = type(value)
    if kind == 'string' or kind == 'number' or kind == 'boolean' then
      parts[#parts + 1] = tostring(value)
    end
  end
  _CAP[#_CAP + 1] = name .. ': ' .. table.concat(parts, ' | ')
end
for _, name in ipairs({'AddMsgToChatWin', 'AddMessageToSysMsgWindow', 'OnCreateChatMessage',
                       'DispatchChatMessage', 'AddMessageTo', 'ShowEffectMsgMessage'}) do
  local original = _G[name]
  if type(original) == 'function' then
    _G[name] = function(...) _cap_add(name, ...); return original(...) end
  end
end
return 'ok'
"""

CAPTURE_CLEAR = "_CAP = {} return 'ok'"
CAPTURE_READ = "return table.concat(_CAP or {}, '\\n')"

C2S_RE = re.compile(r"C->S level=(\d+) opcode=0x([0-9A-Fa-f]{3}) (\w+) len=(\d+) body=(.*)$")
S2C_RE = re.compile(r"GamePacket: S->C type ([0-9A-Fa-f]{3}) (\w+) \((\d+) bytes\)")
SI_RE = re.compile(
    r"StartInteraction, NpcObjId: (\d+), objId: (\d+), extraInfo: (-?\d+), "
    r"pickId: (-?\d+), mouse: (\d+), mods: (-?\d+)")
LOG_TIME_RE = re.compile(r"^(\d\d:\d\d:\d\d(?:\.\d+)?)")


def log(*args: object) -> None:
    print("cart-boarding:", *args, flush=True)


def lua_text(mcp: base.Mcp, pid: int, code: str, target: str = "ui") -> str:
    for _ in range(20):
        try:
            return mcp.text("client_lua", pid=pid, target=target, code=code)
        except Exception:
            time.sleep(0.1)
    return ""


def u24(raw: bytes, offset: int = 0) -> int:
    return int.from_bytes(raw[offset:offset + 3], "little")


def decode_c2s(name: str, body: str) -> dict:
    """Decoded fields for the packets this capture cares about (byte exact)."""
    try:
        raw = bytes.fromhex(body.replace(" ", ""))
    except ValueError:
        return {"error": "bad hex"}
    out: dict = {"bytes": len(raw)}
    try:
        if name == "CSBindSlavePacket":
            out["tl_id"] = struct.unpack_from("<H", raw, 0)[0]
        elif name == "CSDiscardSlavePacket":
            out["tl_id"] = struct.unpack_from("<H", raw, 0)[0]
        elif name == "CSStartInteractionPacket":
            out["npc_obj_id"] = u24(raw, 0)
            out["obj_id"] = u24(raw, 3)
            out["extra_info"] = struct.unpack_from("<i", raw, 6)[0]
            out["pick_id"] = struct.unpack_from("<i", raw, 10)[0]
            out["mouse_button"] = raw[14]
            out["modifier_keys"] = struct.unpack_from("<i", raw, 15)[0]
        elif name == "CSChangeTargetPacket":
            out["obj_id"] = u24(raw, 0)
        elif name == "CSMoveUnitPacket":
            out["obj_id"] = u24(raw, 0)
            out["head"] = raw[:10].hex()
        else:
            out["raw"] = raw.hex()
    except Exception as error:  # malformed body
        out["error"] = f"{type(error).__name__}: {error}"
    return out


class Trace:
    """Incremental reader of the game server log (C->S bodies, S->C opcode lines)."""

    def __init__(self, path: Path) -> None:
        self.path = Path(path)
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw: list[str] = []

    def reset(self) -> None:
        self.offset = self.path.stat().st_size if self.path.exists() else 0

    def poll(self) -> list[dict]:
        if not self.path.exists():
            return []
        raw = self.path.read_bytes()[self.offset:]
        newline = raw.rfind(b"\n")
        if newline < 0:
            # no complete line yet: keep the fragment for the next poll
            return []
        complete = raw[:newline + 1]
        self.offset += len(complete)
        now = time.time()
        out: list[dict] = []
        for line in complete.decode("utf-8", "replace").splitlines():
            self.raw.append(line)
            stamp = LOG_TIME_RE.match(line)
            log_time = stamp.group(1) if stamp else ""
            match = C2S_RE.search(line)
            if match:
                out.append({"t": now, "dir": "C->S", "opcode": int(match.group(2), 16),
                            "name": match.group(3), "len": int(match.group(4)),
                            "body": match.group(5).strip(), "log_time": log_time,
                            "line": line.strip()})
                continue
            match = S2C_RE.search(line)
            if match:
                out.append({"t": now, "dir": "S->C", "opcode": int(match.group(1), 16),
                            "name": match.group(2), "len": int(match.group(3)),
                            "body": "", "log_time": log_time, "line": line.strip()})
                continue
            match = SI_RE.search(line)
            if match:
                out.append({"t": now, "dir": "server", "opcode": -1,
                            "name": "StartInteraction", "len": 0,
                            "body": json.dumps({"npc_obj_id": int(match.group(1)),
                                                "obj_id": int(match.group(2)),
                                                "extra_info": int(match.group(3)),
                                                "pick_id": int(match.group(4)),
                                                "mouse": int(match.group(5)),
                                                "mods": int(match.group(6))}),
                            "log_time": log_time, "line": line.strip()})
        return out


def has(packets: list[dict], direction: str, opcode: int) -> bool:
    return any(p["dir"] == direction and p["opcode"] == opcode for p in packets)


def of(packets: list[dict], direction: str, opcode: int) -> list[dict]:
    return [p for p in packets if p["dir"] == direction and p["opcode"] == opcode]


def dist3(a, b) -> float:
    return math.sqrt(sum((float(a[i]) - float(b[i])) ** 2 for i in range(3)))


def dist2(a, b) -> float:
    return math.hypot(float(a[0]) - float(b[0]), float(a[1]) - float(b[1]))


class Slice:
    """One capture: rows -> native.csv / native.json, plus notes and log excerpt."""

    def __init__(self, outdir: Path, name: str, trace: Trace) -> None:
        self.out = Path(outdir)
        self.name = name
        self.trace = trace
        self.rows: list[dict] = []
        self.notes: list[str] = []
        self.packets: list[dict] = []
        self.started = time.strftime("%Y-%m-%dT%H:%M:%S")

    def note(self, text: str) -> None:
        self.notes.append(text)
        log("note:", text)

    def state(self, case: str, event: str, **fields: object) -> None:
        self.rows.append({
            "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "case": case, "event": event,
            "dir": "", "opcode": "", "name": "", "len": "", "body": "",
            "decoded": "", "client_state": json.dumps(fields, ensure_ascii=False, default=str),
            "text": fields.get("text", ""),
        })

    def add(self, case: str, packets: list[dict]) -> None:
        self.packets.extend(packets)
        for p in packets:
            fields: dict = {"log_time": p.get("log_time", "")}
            if p["dir"] == "C->S":
                fields.update(decode_c2s(p["name"], p["body"]))
            elif p["dir"] == "server":
                fields["fields"] = p["body"]
            self.rows.append({
                "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "case": case,
                "event": "packet" if p["dir"] != "server" else "server-log",
                "dir": p["dir"], "opcode": f"0x{p['opcode']:03X}" if p["opcode"] >= 0 else "",
                "name": p["name"], "len": p["len"], "body": p["body"],
                "decoded": json.dumps(fields, ensure_ascii=False, default=str),
                "client_state": "", "text": "",
            })

    def write(self) -> None:
        self.out.mkdir(parents=True, exist_ok=True)
        fields = ["time", "case", "event", "dir", "opcode", "name", "len", "body",
                  "decoded", "client_state", "text"]
        with open(self.out / "native.csv", "w", encoding="utf-8", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=fields)
            writer.writeheader()
            writer.writerows(self.rows)
        meta = {
            "slice": self.name,
            "started": self.started,
            "written": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "client": "original 1.2 r208022 archeage.exe (hidden, one launch)",
            "driver": "aaemu-clientdriver (Lua/pointer/client actions, no OS input)",
            "rows": len(self.rows),
            "packets": len(self.packets),
        }
        (self.out / "native.json").write_text(
            json.dumps({"meta": meta, "rows": self.rows}, indent=1, ensure_ascii=False),
            encoding="utf-8")
        excerpt = []
        for line in self.trace.raw:
            match = C2S_RE.search(line)
            if match:
                excerpt.append(line)
                continue
            match = S2C_RE.search(line)
            if match and int(match.group(1), 16) in S2C_OF_INTEREST:
                excerpt.append(line)
                continue
            if SI_RE.search(line):
                excerpt.append(line)
        (self.out / "server-log.txt").write_text(
            f"# rs-game.log lines for {self.name}: every C->S field trace, "
            f"SCNpcInteractionSkillList 0x07B / SCSlaveBound 0x064 / SCUnitAttached 0x076 / "
            f"SCUnitDetached 0x077 / SCErrorMsg 0x10A and the slave lifecycle lines, "
            f"with type, name, size and log time\n"
            + "\n".join(excerpt) + "\n", encoding="utf-8")
        (self.out / "notes.txt").write_text("\n".join(self.notes) + "\n", encoding="utf-8")


class Ctx:
    def __init__(self, mcp: base.Mcp, pid: int, mem: base.Mem, base_addr: int,
                 trace: Trace) -> None:
        self.mcp = mcp
        self.pid = pid
        self.mem = mem
        self.base_addr = base_addr
        self.trace = trace
        self.cart_unit: int | None = None
        self.cart_obj: int | None = None

    def ui(self, code: str) -> str:
        return lua_text(self.mcp, self.pid, code)

    def num(self, code: str, default: float = 0.0) -> float:
        try:
            return float(self.ui(code).strip())
        except ValueError:
            return default

    def player_unit(self) -> int:
        return self.mem.u32(
            self.mem.u32(self.base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)

    def player_pos(self) -> list[float] | None:
        return base.unit_state(self.mem, self.player_unit())[1]

    def unit_pos(self, unit: int | None) -> list[float] | None:
        return base.unit_state(self.mem, unit)[1] if unit else None

    def units(self) -> dict[int, int]:
        return dict(base.walk_units(self.mem, self.base_addr)[1])

    def face(self, target: list[float]) -> None:
        pos = self.player_pos()
        if not pos:
            return
        base.set_heading(self.mcp, self.pid, self.mem, self.player_unit(),
                         math.atan2(-(target[0] - pos[0]), target[1] - pos[1]))

    def settle(self, timeout: float = 2.5) -> None:
        unit = self.player_unit()
        base.settle(self.mem, unit, unit, timeout=timeout)

    def chat(self, message: str) -> None:
        self.mcp.text("client_chat", pid=self.pid, message=message)

    def collect(self, seconds: float, stop=None) -> list[dict]:
        packets: list[dict] = []
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            packets += self.trace.poll()
            if stop is not None and stop(packets):
                break
            time.sleep(0.02)
        return packets

    def capture_text(self) -> str:
        return lua_text(self.mcp, self.pid, CAPTURE_READ).strip()

    def capture_clear(self) -> None:
        lua_text(self.mcp, self.pid, CAPTURE_CLEAR)

    def cart_status(self) -> dict:
        code = ("local r={has=X2Player:HasSlaveUnit() and 1 or 0,"
                " mounted=X2SiegeWeapon:IsMountedSiegeWeapon() and 1 or 0,"
                " driver=X2SiegeWeapon:IsDriver() and 1 or 0,"
                " speed=X2SiegeWeapon:GetSiegeWeaponSpeed() or 0,"
                " turn=X2SiegeWeapon:GetSiegeWeaponTurnSpeed() or 0, v=''}"
                " local i=X2SiegeWeapon:GetMountedSiegeWeaponInfo()"
                " if type(i)=='table' then r.v=tostring(i.name)..'/'..tostring(i.tl)..'/'..tostring(i.objId) end"
                " return table.concat({r.has,r.mounted,r.driver,tostring(r.speed),tostring(r.turn),r.v},'|')")
        parts = self.ui(code).strip().split("|")
        out = dict(zip(["has_vehicle", "mounted", "driver", "speed", "turn_speed", "vehicle"], parts))
        for key in ("has_vehicle", "mounted", "driver"):
            out[key] = out.get(key) == "1"
        return out

    def target_name(self) -> str:
        return self.ui("return tostring(X2Unit:UnitName('target'))").strip()

    def release_target(self) -> None:
        self.ui("X2Unit:ReleaseTarget() return 1")


def attach(mcp: base.Mcp, pid: int) -> tuple[base.Mem, int]:
    base.wait_player(mcp, pid, 60)
    mem = base.Mem(pid)
    for name, _ in mem.modules():
        if any(bad in name.lower() for bad in base.ANTI_CHEAT):
            raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
    address = mem.module_base("x2game.dll")
    if not address:
        raise RuntimeError("x2game.dll not loaded")
    log("attached", pid, "x2game base", hex(address))
    return mem, address


def select_character(mcp: base.Mcp, pid: int, name: str) -> None:
    base.wait_stage(mcp, pid, 3, 240)
    lua_text(mcp, pid, "X2World:EnterWorld(1,0)")
    for _ in range(240):
        if lua_text(mcp, pid, "return X2LoginCharacter:GetCurrentStage()").strip() in ("4", "5"):
            break
        time.sleep(0.5)
    names = [lua_text(mcp, pid, f"return X2LoginCharacter:GetLoginCharacterName({i})").strip()
             for i in range(1, 5)]
    names = [item for item in names if item and item != "nil"]
    if name not in names:
        raise RuntimeError(f"character {name!r} not in {names}")
    lua_text(mcp, pid, f"X2LoginCharacter:SelectCharacter({names.index(name) + 1})")
    lua_text(mcp, pid, "characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')")
    base.wait_player(mcp, pid, 240)
    time.sleep(3.0)


def entities(mcp: base.Mcp, pid: int, **filters: object) -> list[dict]:
    args = {"pid": pid, "limit": 200}
    args.update(filters)
    try:
        payload = json.loads(mcp.text("client_entities", **args))
    except Exception:
        return []
    rows = []
    for key, value in (payload.get("entities") or {}).items():
        row = dict(value)
        row["engine_id"] = key
        rows.append(row)
    return rows


def spawn_cart(ctx: Ctx, sl: Slice, case: str) -> tuple[int, list[float], str]:
    before = ctx.units()
    ctx.trace.reset()
    ctx.chat(f"/slave spawn {CART}")
    deadline = time.monotonic() + 15.0
    found = None
    while time.monotonic() < deadline:
        new = [(key, value) for key, value in ctx.units().items() if key not in before]
        if new:
            found = new[-1]
            break
        time.sleep(0.3)
    if not found:
        raise RuntimeError("no new slave unit after /slave spawn")
    key, unit = found
    time.sleep(1.2)
    entries = [e for e in entities(ctx.mcp, ctx.pid) if e.get("class") == "prefab_unit"]
    pos = ctx.unit_pos(unit)
    entity = None
    for candidate in entries:
        p = candidate.get("position") or {}
        if pos and dist3([p.get("x", 0), p.get("y", 0), p.get("z", 0)], pos) < 2.0:
            entity = candidate
    if entity is None and entries:
        entity = entries[-1]
    if entity is None:
        raise RuntimeError("the cart entity did not appear")
    position = entity["position"]
    ctx.cart_unit = unit
    ctx.cart_obj = key
    sl.add(case, ctx.collect(0.6))
    sl.state(case, "spawn", cart_name=entity["name"], cart_obj=key, cart_hex=f"{key:X}",
             cart_pos=[position["x"], position["y"], position["z"]],
             unit_kind=ctx.mem.u32(unit + 0x410), unit_tl=ctx.mem.u32(unit + 0x41C),
             player_pos=ctx.player_pos())
    return key, [position["x"], position["y"], position["z"]], entity["name"]


def place_at(ctx: Ctx, sl: Slice, case: str, cart: list[float], distance: float) -> float:
    """Teleport to the cart + distance along the spawn axis, then correct the measured 3D gap."""
    character = ctx.ui("return X2Unit:UnitName('player')").strip()
    requested = (cart[0], cart[1] - distance, cart[2])
    measured = 99.0
    for attempt in range(4):
        if attempt == 0:
            target = requested
        else:
            pos = ctx.player_pos() or list(requested)
            dx, dy, dz = pos[0] - cart[0], pos[1] - cart[1], pos[2] - cart[2]
            horizontal = math.hypot(dx, dy) or 1.0
            wanted = math.sqrt(max(distance * distance - dz * dz, 0.04))
            target = (cart[0] + dx / horizontal * wanted, cart[1] + dy / horizontal * wanted, cart[2])
        ctx.chat(f"/move {character} {target[0]:.2f} {target[1]:.2f} {target[2]:.2f}")
        time.sleep(1.8)
        ctx.face(cart)
        ctx.settle()
        pos = ctx.player_pos()
        measured = dist3(pos, cart) if pos else 99.0
        sl.state(case, "placed", requested=distance, measured=round(measured, 4),
                 attempt=attempt + 1, player_pos=pos, cart=cart)
        if abs(measured - distance) < 0.08:
            return measured
    return measured


def interaction_npc(packets: list[dict]) -> int | None:
    for packet in of(packets, "C->S", START_INTERACTION):
        value = decode_c2s(packet["name"], packet["body"]).get("npc_obj_id")
        if value is not None:
            return value
    return None


def pick_and_follow(ctx: Ctx, sl: Slice, case: str, cart: list[float], name: str,
                    after_skill: float = 3.0, pick_window: float = 0.7):
    """Right-click until the cart is picked, then sample every 100 ms for `after_skill` s.

    A right-click that hits the cart is the case click: the client answers the server's
    0x07b itself, so a second click would act from the new (mounted) state. Returns
    (packets, samples, point) or None.
    """
    try:
        text = ctx.ui("return X2Util:ConvertWorldToScreen(%.6f,%.6f,%.6f)" % tuple(cart)).replace("\t", " ")
        parts = [item for item in text.split() if item]
        if len(parts) >= 2:
            first = (int(round(float(parts[0]))), int(round(float(parts[1]))))
        else:
            first = (SCREEN_W // 2, SCREEN_H // 2)
    except Exception:
        first = (SCREEN_W // 2, SCREEN_H // 2)
    attempts = [first]
    for dy in (-40, 40, -80, 80, -120, 120, -160, 160):
        attempts.append((first[0], first[1] + dy))
    for y in range(700, SCREEN_H, 40):
        attempts.append((SCREEN_W // 2, y))
    for y in range(40, 700, 40):
        attempts.append((SCREEN_W // 2, y))
    seen: set[tuple[int, int]] = set()
    for x, y in attempts:
        if (x, y) in seen:
            continue
        seen.add((x, y))
        ctx.release_target()
        ctx.capture_clear()
        ctx.trace.reset()
        started = time.monotonic()
        ctx.mcp.text("client_pointer", pid=ctx.pid, action="right_click", x=x, y=y)
        packets: list[dict] = []
        samples: list[dict] = []
        skill_at = None
        next_sample = started
        aborted = False
        while True:
            packets += ctx.trace.poll()
            now = time.monotonic()
            if skill_at is None and has(packets, "S->C", 0x07B):
                if interaction_npc(packets) != ctx.cart_obj:
                    break
                skill_at = now
            if skill_at is not None:
                if now >= next_sample:
                    player = ctx.player_pos()
                    cart_pos = ctx.unit_pos(ctx.cart_unit)
                    samples.append({
                        "ms": round((now - skill_at) * 1000.0, 1),
                        "player": [round(v, 3) for v in player] if player else None,
                        "cart": [round(v, 3) for v in cart_pos] if cart_pos else None,
                        "dist": round(dist3(player, cart_pos), 3) if player and cart_pos else None,
                    })
                    next_sample = now + 0.1
                if now - skill_at >= after_skill:
                    break
            elif now - started >= pick_window:
                break
            time.sleep(0.02)
        packets += ctx.trace.poll()
        target = ctx.target_name()
        picked = interaction_npc(packets) == ctx.cart_obj
        opcodes = sorted({p["opcode"] for p in packets if p["dir"] == "C->S" and p["opcode"] in
                          (CHANGE_TARGET, START_INTERACTION, BIND)})
        sl.state(case, "sweep", point=[x, y], try_index=len(seen), target=target,
                 sent=[f"0x{code:03X}" for code in opcodes],
                 interact_npc=interaction_npc(packets),
                 s2c_07b=len(of(packets, "S->C", 0x07B)))
        if picked:
            sl.add(case, packets)
            walked = 0.0
            if samples and samples[0]["player"] and samples[-1]["player"]:
                walked = dist3(samples[0]["player"], samples[-1]["player"])
            sl.state(case, "result",
                     point=[x, y],
                     send_02c=has(packets, "C->S", CHANGE_TARGET),
                     send_068=True,
                     bind=has(packets, "C->S", BIND),
                     bind_body=of(packets, "C->S", BIND)[0]["body"] if has(packets, "C->S", BIND) else "",
                     s2c_07b=[{"len": p["len"], "log_time": p.get("log_time")}
                              for p in of(packets, "S->C", 0x07B)],
                     s2c_064=[{"len": p["len"], "log_time": p.get("log_time")}
                              for p in of(packets, "S->C", 0x064)],
                     s2c_076=[{"len": p["len"], "log_time": p.get("log_time")}
                              for p in of(packets, "S->C", 0x076)],
                     s2c_077=[{"len": p["len"], "log_time": p.get("log_time")}
                              for p in of(packets, "S->C", 0x077)],
                     s2c_10a=[{"len": p["len"], "log_time": p.get("log_time")}
                              for p in of(packets, "S->C", 0x10A)],
                     samples=samples, walked_m=round(walked, 3),
                     cart=ctx.cart_status(), target=target, text=ctx.capture_text())
            return packets, samples, (x, y)
        time.sleep(0.1)
    sl.state(case, "pick_failed", tries=len(seen))
    return None


def leave(ctx: Ctx, sl: Slice, case: str, seconds: float = 3.0) -> list[dict]:
    ctx.capture_clear()
    ctx.trace.reset()
    before = ctx.cart_status()
    packets: list[dict] = []
    attempts = 0
    for attempt in range(2):
        attempts = attempt + 1
        ctx.ui("X2SiegeWeapon:DiscardSiegeWeapon() return 1")
        packets += ctx.collect(seconds, stop=lambda p: has(p, "S->C", 0x077))
        if has(packets, "C->S", UNBIND):
            break
        time.sleep(1.5)
    packets += ctx.collect(0.5)
    sl.add(case, packets)
    sl.state(case, "leave",
             attempts=attempts,
             send_032=has(packets, "C->S", UNBIND),
             body_032=of(packets, "C->S", UNBIND)[0]["body"] if has(packets, "C->S", UNBIND) else "",
             s2c_077=[{"len": p["len"], "log_time": p.get("log_time")} for p in of(packets, "S->C", 0x077)],
             s2c_10a=[{"len": p["len"], "log_time": p.get("log_time")} for p in of(packets, "S->C", 0x10A)],
             before=before, cart=ctx.cart_status(), text=ctx.capture_text())
    return packets


def ensure_unbound(ctx: Ctx, sl: Slice, case: str) -> None:
    status = ctx.cart_status()
    if status.get("mounted") or status.get("driver"):
        leave(ctx, sl, case + "_auto_leave")
        time.sleep(0.8)


def summary(ctx: Ctx, sl: Slice, case: str, measured: float, hit) -> bool:
    if not hit:
        sl.state(case, "summary", measured=round(measured, 3), pick="not found")
        return False
    packets, samples, point = hit
    walked = 0.0
    if samples and samples[0]["player"] and samples[-1]["player"]:
        walked = dist3(samples[0]["player"], samples[-1]["player"])
    sl.state(case, "summary", measured=round(measured, 3), point=list(point),
             send_02c=has(packets, "C->S", CHANGE_TARGET),
             send_068=has(packets, "C->S", START_INTERACTION),
             send_031=has(packets, "C->S", BIND),
             bind_body=of(packets, "C->S", BIND)[0]["body"] if has(packets, "C->S", BIND) else "",
             s2c_07b=len(of(packets, "S->C", 0x07B)),
             s2c_064=len(of(packets, "S->C", 0x064)),
             s2c_076=len(of(packets, "S->C", 0x076)),
             s2c_10a=len(of(packets, "S->C", 0x10A)),
             walked_m=round(walked, 3), samples=len(samples),
             cart=ctx.cart_status(), target=ctx.target_name(), text=ctx.capture_text())
    return has(packets, "C->S", BIND)


def slice_s5(ctx: Ctx, sl: Slice) -> None:
    character = ctx.ui("return X2Unit:UnitName('player')").strip()
    sl.note("S5 cart boarding, original 1.2 client (capture 2). Cart = slave template 60 "
            "(Farm Wagon) spawned with the server's GM chat command /slave spawn 60; the pick is "
            "the client's own right click through the driver's pointer API; positions come from "
            "ReadProcessMemory; the client's visible system text is captured by wrapping its own "
            "message functions.")
    sl.note("The pointer space is the 2554x1411 back buffer; X2Util:ConvertWorldToScreen does "
            "not agree with it on this client, so the pick point is found with a short right-click "
            "sweep (target name + CSStartInteraction 0x068 both required). The click that hits the "
            "cart is the case click: the server now answers it with 0x07b, so the probe never "
            "clicks a second time from the resulting state. Every attempt is a `sweep` row.")
    sl.note("Placement: /move to the cart + requested distance on the spawn axis, then up to "
            "three radial corrections so the 3D centre distance is within 0.08 m (z is terrain "
            "height). The measured 3D distance is in each `placed` row. After every bound case "
            "the client leaves with X2SiegeWeapon:DiscardSiegeWeapon before the next placement.")
    ctx.chat(f"/move {character} 4318.00 4118.00 98.00")
    time.sleep(2.5)
    ensure_unbound(ctx, sl, "start")

    cart_obj, cart, name = spawn_cart(ctx, sl, "a_board_2m")

    # (a) right click from ~2 m
    measured = place_at(ctx, sl, "a_board_2m", cart, 2.0)
    hit = pick_and_follow(ctx, sl, "a_board_2m", cart, name, after_skill=3.0)
    bound_a = summary(ctx, sl, "a_board_2m", measured, hit)

    # (e) right click a second time while bound
    if bound_a:
        time.sleep(0.8)
        sl.state("e_click_while_bound", "before", cart=ctx.cart_status())
        hit_e = pick_and_follow(ctx, sl, "e_click_while_bound", cart, name,
                                after_skill=1.5, pick_window=0.6)
        summary(ctx, sl, "e_click_while_bound", 0.0, hit_e)
        time.sleep(0.5)

        # (b) leave with the client's own X2SiegeWeapon:DiscardSiegeWeapon
        leave(ctx, sl, "b_discard")
        time.sleep(0.8)

    # (c) reach sweep
    for distance in (3.5, 4.5, 3.1, 2.9):
        case = f"c_reach_{distance:g}"
        ensure_unbound(ctx, sl, case)
        measured = place_at(ctx, sl, case, cart, distance)
        hit = pick_and_follow(ctx, sl, case, cart, name, after_skill=3.0)
        if summary(ctx, sl, case, measured, hit):
            time.sleep(0.5)
            leave(ctx, sl, case + "_leave")
            time.sleep(0.8)

    # (d) board, drive forward, leave while moving
    case = "d_drive_moving"
    ensure_unbound(ctx, sl, case)
    measured = place_at(ctx, sl, case, cart, 2.0)
    hit = pick_and_follow(ctx, sl, case, cart, name, after_skill=0.8)
    if hit and has(hit[0], "C->S", BIND):
        time.sleep(1.0)
        ctx.capture_clear()
        ctx.trace.reset()
        started = time.monotonic()
        ctx.mcp.text("client_input", pid=ctx.pid, key="w", duration_ms=2600)
        samples: list[dict] = []
        speeds: list[float] = []
        compare = None
        sent_at = None
        speed_at = None
        next_sample = started
        while time.monotonic() - started < 3.0:
            now = time.monotonic()
            if now >= next_sample:
                player = ctx.player_pos()
                cart_pos = ctx.unit_pos(ctx.cart_unit)
                if compare and player:
                    dt = now - compare[0]
                    if dt > 0:
                        speeds.append(dist3(player, compare[1]) / dt)
                compare = (now, player) if player else None
                status = ctx.cart_status()
                samples.append({"t": round(now - started, 3),
                                "player": [round(v, 3) for v in player] if player else None,
                                "cart": [round(v, 3) for v in cart_pos] if cart_pos else None,
                                "client_speed": status.get("speed"),
                                "mounted": status.get("mounted"), "driver": status.get("driver")})
                next_sample = now + 0.1
            if sent_at is None and now - started >= 1.2:
                speed_at = ctx.cart_status()
                ctx.ui("X2SiegeWeapon:DiscardSiegeWeapon() return 1")
                sent_at = time.monotonic()
            time.sleep(0.02)
        packets = ctx.collect(1.5)
        sl.add(case, packets)
        recent = [item for item in speeds[-4:]]
        sl.state(case, "drive",
                 measured=round(measured, 3),
                 samples=samples,
                 speed_before_client=speed_at,
                 speed_measured_max=round(max(speeds), 3) if speeds else 0.0,
                 speed_measured_at_leave=round(sum(recent) / len(recent), 3) if recent else 0.0,
                 send_032=has(packets, "C->S", UNBIND),
                 body_032=of(packets, "C->S", UNBIND)[0]["body"] if has(packets, "C->S", UNBIND) else "",
                 s2c_077=[{"len": p["len"], "log_time": p.get("log_time")} for p in of(packets, "S->C", 0x077)],
                 s2c_10a=[{"len": p["len"], "log_time": p.get("log_time")} for p in of(packets, "S->C", 0x10A)],
                 move_packets=len(of(packets, "C->S", 0x089)),
                 cart=ctx.cart_status(), text=ctx.capture_text())
        if not has(packets, "C->S", UNBIND):
            time.sleep(2.5)
            leave(ctx, sl, case + "_stopped")

    sl.note("(f) a non-owner passenger / second player case is SKIPPED: the machine rule allows "
            "one original client for this task.")
    sl.note("S->C bodies: the lane logs SCNpcInteractionSkillList 0x07B, SCSlaveBound 0x064, "
            "SCUnitAttached 0x076, SCUnitDetached 0x077 and SCErrorMsg 0x10A as type/name/size "
            "lines at the configured aaemu_game::net=debug level; the packet bodies are not in "
            "this trace, so the rows carry the size and the log time, and the body fields are "
            "compared through the C->S trace and the server-side StartInteraction line.")


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
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
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "cart_boarding"))
    parser.add_argument("--raw", default=os.environ.get("CART_RAW")
                        or str(ROOT / "data" / "extract" / f"cart-boarding-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set CART_{name.upper()})")
    client = Path(args.client)
    driver = Path(args.driver)
    dll = Path(args.driver_dll)
    launcher = Path(args.launcher)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    if not driver.exists() or not dll.exists() or not launcher.exists() or not password_file.exists():
        raise SystemExit(f"missing driver/dll/launcher/password file: {driver} {dll} {launcher} {password_file}")
    if not server_log.exists():
        raise SystemExit(f"missing server log: {server_log}")
    out = Path(args.out)
    raw = Path(args.raw)
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)

    log(f"account {args.account}: {args.character}")
    mcp = base.Mcp(driver, dll, launcher, client, args.login, password_file, args.account, raw / "mcp")
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
            select_character(mcp, pid, args.character)
        else:
            base.wait_player(mcp, pid, 30)
        mem, address = attach(mcp, pid)
        install = lua_text(mcp, pid, CAPTURE_INSTALL).strip()
        if install != "ok":
            log("text capture install:", install)
        trace = Trace(server_log)
        trace.reset()
        ctx = Ctx(mcp, pid, mem, address, trace)
        sl = Slice(out, "cart_boarding", trace)
        failed = False
        try:
            slice_s5(ctx, sl)
        except Exception as error:
            import traceback
            traceback.print_exc()
            sl.note(f"CAPTURE FAILED: {type(error).__name__}: {error}")
            failed = True
        finally:
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
