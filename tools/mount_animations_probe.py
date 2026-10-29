#!/usr/bin/env python3
"""Capture the 1.2 client's S3 rider placement and dismount cases.

The original client seats a rider on a mate's seat with `SCUnitAttached`
(0x076) after `CSMountMate` (0x0a7); the reason byte of the request is the side
the player came from (`FUN_3909ae90`), 1 left / 2 right / 3 behind / 4 front.
`X2Mate:UnMountMate` sends `CSUnMountMate` (0x0a8) with reason 9 and the server
answers `SCUnitDetached` (0x077).  This probe drives the hidden original through
aaemu-clientdriver (Lua and the client's own actions, no OS input), walks to
each side of a summoned Heir's Horse with the driver's own key input, mounts and
dismounts, samples the rider position/quaternion, and records the gaits and the
dismount reply latency.  Positions come from ReadProcessMemory (PROCESS_VM_READ
only); the server's log supplies the C->S field trace and the S->C opcode lines.

The capture logs the S->C direction as opcode + byte count only, so the
`SCUnitAttached` reason byte is not captured; the `CSMountMate` reason 1..4 and
the detach reply are (the 2063 ms in the old evidence was measured with the
earlier timing code and is an upper bound).  A GM /move teleport is not used: it leaves the summoned
mate behind and the mate despawns; the player walks (`base.orbit_to`).

Usage:
    python tools/mount_animations_probe.py --client <archeage.exe> \\
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \\
        --launcher <aaemu-launcher.exe> --login <host:port> \\
        --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --out tools/evidence/mount_animations
    python tools/mount_animations_probe.py --pid 1234 ...  # already in world

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace`.  The 1.2 client must be hidden
(`client_launch ... hidden`), which this script does.  It refuses to run if an
anti-cheat module is loaded.

The account may be new: with AutoAccount the first login creates it.  A missing
character is created through the original creation UI when `--create-character`
is passed.
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mount_reason_probe as base

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ITEM = 27264  # Heir's Horse, npc 13022, mate level 40

C2S_RE = re.compile(r"C->S level=(\d+) opcode=0x([0-9A-Fa-f]{3}) (\w+) len=(\d+) body=(.*)$")
S2C_RE = re.compile(r"GamePacket: S->C type ([0-9A-Fa-f]{3}) (\w+) \((\d+) bytes\)")

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


def log(*args: object) -> None:
    print("mount-animations:", *args, flush=True)


def lua_text(mcp: base.Mcp, pid: int, code: str) -> str:
    for _ in range(20):
        try:
            return mcp.text("client_lua", pid=pid, target="ui", code=code)
        except Exception:
            time.sleep(0.1)
    return ""


def capture_text(mcp: base.Mcp, pid: int) -> str:
    return lua_text(mcp, pid, CAPTURE_READ).strip()


def capture_clear(mcp: base.Mcp, pid: int) -> None:
    lua_text(mcp, pid, CAPTURE_CLEAR)


def u24(body: str, offset: int = 0) -> int:
    raw = bytes.fromhex(body.replace(" ", ""))
    return int.from_bytes(raw[offset:offset + 3], "little")


def decode_c2s(name: str, body: str) -> dict:
    """Decoded fields for the C->S packets this capture cares about."""
    try:
        raw = bytes.fromhex(body.replace(" ", ""))
    except ValueError:
        return {"error": "bad hex"}
    out: dict = {"bytes": len(raw)}
    try:
        if name in ("CSMountMatePacket", "CSUnMountMatePacket"):
            out["tl_id"], out["ap"], out["reason"] = int.from_bytes(raw[0:2], "little"), raw[2], raw[3]
        elif name == "CSMoveUnitPacket":
            # CSMoveUnit starts with the sender's 3-byte object id (the bc) at offset 0
            out["obj_id"] = u24(body, 0) if len(raw) >= 7 else None
            out["head"] = raw[:10].hex()
        else:
            out["raw"] = raw.hex()
    except Exception as error:  # malformed body
        out["error"] = f"{type(error).__name__}: {error}"
    return out


class Trace:
    """Incremental reader of the game server log (C->S trace + S->C opcode lines)."""

    def __init__(self, path: Path):
        self.path = Path(path)
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw: list[str] = []

    def reset(self) -> None:
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw = []

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
        now = time.monotonic()
        out: list[dict] = []
        for line in complete.decode("utf-8", "replace").splitlines():
            self.raw.append(line)
            m = C2S_RE.search(line)
            if m:
                out.append({"t": now, "dir": "C->S", "opcode": int(m.group(2), 16),
                            "name": m.group(3), "len": int(m.group(4)),
                            "body": m.group(5).strip(), "line": line.strip()})
                continue
            m = S2C_RE.search(line)
            if m:
                out.append({"t": now, "dir": "S->C", "opcode": int(m.group(1), 16),
                            "name": m.group(2), "len": int(m.group(3)),
                            "body": "", "line": line.strip()})
        return out


def collect(trace: Trace, seconds: float, stop=None) -> list[dict]:
    packets: list[dict] = []
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        packets += trace.poll()
        if stop is not None and stop(packets):
            break
        time.sleep(0.02)
    return packets


def has(packets: list[dict], direction: str, opcode: int) -> bool:
    return any(p["dir"] == direction and p["opcode"] == opcode for p in packets)


class Slice:
    """The S3 rows -> native.csv / native.json."""

    def __init__(self, outdir: Path, name: str):
        self.out = Path(outdir)
        self.name = name
        self.rows: list[dict] = []
        self.notes: list[str] = []
        self.packets: list[dict] = []
        self.started = time.strftime("%Y-%m-%dT%H:%M:%S")

    def note(self, text: str) -> None:
        self.notes.append(text)
        log(self.name, "note:", text)

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
            fields = {}
            if p["dir"] == "C->S":
                fields = decode_c2s(p["name"], p["body"])
            self.rows.append({
                "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "case": case,
                "event": "packet", "dir": p["dir"],
                "opcode": f"0x{p['opcode']:03X}" if p["opcode"] >= 0 else "",
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
            "server_log": "<server-log>",
            "rows": len(self.rows),
            "packets": len(self.packets),
        }
        (self.out / "native.json").write_text(
            json.dumps({"meta": meta, "rows": self.rows}, indent=1, ensure_ascii=False),
            encoding="utf-8")


class Ctx:
    def __init__(self, mcp: base.Mcp, pid: int, mem: base.Mem, base_addr: int, trace: Trace, out: Path):
        self.mcp = mcp
        self.pid = pid
        self.mem = mem
        self.base_addr = base_addr
        self.trace = trace
        self.out = out
        self.slices: dict[str, Slice] = {}

    def ui(self, code: str) -> str:
        return lua_text(self.mcp, self.pid, code)

    def player_unit(self) -> int:
        return self.mem.u32(self.mem.u32(self.base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)

    def player_pos(self) -> list[float]:
        _rot, pos = base.unit_state(self.mem, self.player_unit())
        if not pos:
            raise RuntimeError("player position lost")
        return pos

    def unit_pos(self, unit: int) -> list[float] | None:
        return base.unit_state(self.mem, unit)[1]

    def units(self) -> dict[int, int]:
        return dict(base.walk_units(self.mem, self.base_addr)[1])

    def collect(self, seconds: float, stop=None) -> list[dict]:
        return collect(self.trace, seconds, stop)

    def chat(self, message: str) -> None:
        self.mcp.text("client_chat", pid=self.pid, message=message)

    def pet_exists(self) -> bool:
        return base.pet_exists(self.mcp, self.pid)

    def mounted(self) -> bool:
        return base.mounted(self.mcp, self.pid)

    def mount_status(self) -> dict:
        code = ("local function s(v) if v==nil then return 'nil' end return tostring(v) end "
                "return table.concat({s(X2Mate:IsPlayerPetExists()),s(X2Mate:IsPlayerMounting()),"
                "s(X2Mate:CanMount()),s(X2Mate:PetState()),s(X2Mate:IsMountablePet())},'|')")
        parts = self.ui(code).strip().split("|")
        keys = ["pet_exists", "mounting", "can_mount", "pet_state", "mountable"]
        out = dict(zip(keys, [p == "true" for p in parts[:3]] + parts[3:]))
        out["pet_name"] = self.ui("return tostring(X2Unit:UnitName('playerpet'))").strip()
        return out

    def slice(self, name: str) -> Slice:
        if name not in self.slices:
            self.slices[name] = Slice(self.out, name)
        return self.slices[name]


def create_character(mcp: base.Mcp, pid: int, name: str, allowed: bool) -> None:
    stage = lua_text(mcp, pid, "return X2LoginCharacter:GetCurrentStage()").strip()
    count = int(lua_text(mcp, pid, "return X2LoginCharacter:GetNumLoginCharacters()").strip() or 0) if stage == "4" else 0
    if count == 0:
        if not allowed:
            raise RuntimeError("the account has no character; pass --create-character")
        if stage == "4":
            lua_text(mcp, pid, "X2LoginCharacter:CreateCharacter()")
        for _ in range(240):
            if lua_text(mcp, pid, "return characterCreate and characterCreate.state").strip() == "Step1":
                break
            time.sleep(0.25)
        time.sleep(1.0)
        lua_text(mcp, pid, "characterCreate.OnNextStep()")
        for _ in range(240):
            if lua_text(mcp, pid, "return characterCreate.state").strip() == "Step2":
                break
            time.sleep(0.25)
        time.sleep(1.0)
        lua_text(mcp, pid, "characterCreate.OnNextStep()")
        for _ in range(240):
            if lua_text(mcp, pid, "return characterCreate.state").strip() == "Step3":
                break
            time.sleep(0.25)
        lua_text(
            mcp,
            pid,
            f"SelectAbility(1) step3.content.nameWnd.characterName:SetText({json.dumps(name)}) CreateCharacter(false)",
        )
        base.wait_stage(mcp, pid, 4, 120)
    names = [lua_text(mcp, pid, f"return X2LoginCharacter:GetLoginCharacterName({i})").strip() for i in range(1, 5)]
    names = [n for n in names if n and n != "nil"]
    if name not in names:
        raise RuntimeError(f"character {name!r} not in {names}")
    lua_text(mcp, pid, f"X2LoginCharacter:SelectCharacter({names.index(name) + 1})")
    lua_text(mcp, pid, "characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')")
    base.wait_player(mcp, pid, 240)


def reset_pet(ctx: Ctx) -> None:
    base.dismount(ctx.mcp, ctx.pid)
    if ctx.pet_exists():
        for _ in range(2):
            ctx.ui("X2Mate:DismissPet() return 1")
            end = time.monotonic() + 6.0
            while time.monotonic() < end and ctx.pet_exists():
                time.sleep(0.2)
            if not ctx.pet_exists():
                break
        time.sleep(2.2)


def summon(ctx: Ctx, item: int, sl: Slice, case: str) -> bool:
    reset_pet(ctx)
    ok = False
    for attempt in range(1, 4):
        ctx.chat(f"/item add self {item} 1")
        time.sleep(2.0)
        try:
            ctx.mcp.text("client_use_item", pid=ctx.pid, item_template_id=item)
        except Exception as error:
            sl.state(case, "use_item_failed", attempt=attempt, error=str(error))
        end = time.monotonic() + 10.0
        while time.monotonic() < end and not ctx.pet_exists():
            time.sleep(0.3)
        ok = ctx.pet_exists()
        sl.state(case, "summon", attempt=attempt, item=item, pet_exists=ok, mount=ctx.mount_status())
        if ok:
            break
        time.sleep(3.0)
    time.sleep(2.0)
    return ok


def mount_unit(ctx: Ctx) -> tuple[int | None, list[float] | None]:
    name = ctx.ui("return tostring(X2Unit:UnitName('playerpet'))").strip()
    pos = base.lua_pos(ctx.mcp, ctx.pid, name)
    if not pos:
        return None, None
    found = base.identify(ctx.mem, ctx.base_addr, pos)
    return (found[1] if found else None), pos


def slice_s3(ctx: Ctx, item: int) -> None:
    sl = ctx.slice("s3")
    sl.note("S3 rider placement / dismount: mount item "
            f"{item} (Heir's Horse 27264, npc 13022), mount and dismount through "
            "X2Mate:MountMate / UnMountMate (the client's own calls). The player walks to the side; "
            "a GM /move teleport leaves the mate behind and is not used. The get-on side is the "
            "CSMountMate reason byte (1..4 -> Enter_l/Enter_r/Enter_b/Enter_f), the reply is "
            "SCUnitAttached 0x076 (logged by opcode and byte count only, so its reason byte is NOT "
            "captured). The driver exposes no animation-graph or ActionController reader (probed at "
            "the end), so the animation evidence is the reason byte, the attach reply and 100 ms "
            "position/quaternion sampling.")

    if not summon(ctx, item, sl, "setup"):
        sl.note(f"NOT CAPTURED: mount item {item} could not be summoned.")
        return
    time.sleep(1.5)
    unit, horse = mount_unit(ctx)
    sl.state("setup", "mount_unit", unit=hex(unit) if unit else None, horse=horse,
             name=ctx.ui("return tostring(X2Unit:UnitName('playerpet'))").strip(), mount=ctx.mount_status())
    if not unit:
        sl.note("NOT CAPTURED: the mate unit was not found.")
        return

    player_unit = ctx.player_unit()
    for side, angle in (("left", 270), ("right", 90), ("behind", 180), ("front", 0)):
        case = f"a_mount_{side}"
        ok = base.orbit_to(ctx.mcp, ctx.pid, ctx.mem, player_unit, unit, angle)
        horse = base.lua_pos(ctx.mcp, ctx.pid,
                             ctx.ui("return tostring(X2Unit:UnitName('playerpet'))").strip()) or horse
        base.settle(ctx.mem, player_unit, unit)
        sl.state(case, "placed", side=side, orbit_ok=ok,
                 measured_2d=round(math.hypot(ctx.player_pos()[0] - horse[0], ctx.player_pos()[1] - horse[1]), 3),
                 player_pos=ctx.player_pos(), horse=horse)
        ctx.trace.reset()
        capture_clear(ctx.mcp, ctx.pid)
        ctx.ui("X2Mate:MountMate() return 1")
        packets = ctx.collect(6.0, stop=lambda p: has(p, "S->C", 0x076))
        time.sleep(0.3)
        samples = []
        for _ in range(8):
            rot, pos = base.unit_state(ctx.mem, ctx.player_unit())
            samples.append({"t": round(time.time(), 3), "pos": pos, "quat": rot})
            time.sleep(0.1)
        sl.add(case, packets)
        sl.state(case, "mounted", mount_sent=has(packets, "C->S", 0x0A7),
                 attached=has(packets, "S->C", 0x076), mount=ctx.mount_status(),
                 samples=samples, text=capture_text(ctx.mcp, ctx.pid))
        time.sleep(2.2)
        ctx.trace.reset()
        ctx.ui("X2Mate:UnMountMate() return 1")
        ctx.collect(4.0, stop=lambda p: has(p, "S->C", 0x077))
        time.sleep(2.4)

    ok = base.orbit_to(ctx.mcp, ctx.pid, ctx.mem, player_unit, unit, 270)
    base.settle(ctx.mem, player_unit, unit)
    ctx.trace.reset()
    ctx.ui("X2Mate:MountMate() return 1")
    ctx.collect(6.0, stop=lambda p: has(p, "S->C", 0x076))
    time.sleep(1.5)
    sl.state("b_gaits", "idle", samples=[{"t": round(time.time(), 3), "player": ctx.player_pos()}],
             mount=ctx.mount_status(), orbit_ok=ok)
    for label, millis in (("walk", 900), ("run", 2600)):
        ctx.trace.reset()
        capture_clear(ctx.mcp, ctx.pid)
        ctx.mcp.text("client_input", pid=ctx.pid, key="w", duration_ms=millis)
        samples = []
        end = time.monotonic() + millis / 1000.0 + 0.4
        while time.monotonic() < end:
            samples.append({"t": round(time.time(), 3), "player": ctx.player_pos()})
            time.sleep(0.12)
        packets = ctx.collect(0.5)
        speeds = []
        for a, b in zip(samples, samples[1:]):
            dt = b["t"] - a["t"]
            if dt > 0:
                speeds.append(math.dist(a["player"], b["player"]) / dt)
        sl.add("b_gaits", [p for p in packets if p["dir"] == "C->S" and p["opcode"] == 0x089][:3])
        sl.state("b_gaits", label, ms=millis, samples=samples,
                 speed_mean=round(sum(speeds) / len(speeds), 3) if speeds else None,
                 speed_max=round(max(speeds), 3) if speeds else None,
                 move_packets=sum(1 for p in packets if p["dir"] == "C->S" and p["opcode"] == 0x089),
                 mount=ctx.mount_status(), text=capture_text(ctx.mcp, ctx.pid))
        time.sleep(1.0)

    ctx.trace.reset()
    capture_clear(ctx.mcp, ctx.pid)
    t0 = time.monotonic()
    ctx.ui("X2Mate:UnMountMate() return 1")
    packets = ctx.collect(4.0, stop=lambda p: has(p, "S->C", 0x077))
    detached_at = None
    if has(packets, "S->C", 0x077):
        detached_at = min(p["t"] for p in packets if p["dir"] == "S->C" and p["opcode"] == 0x077)
    samples = []
    end = time.monotonic() + 1.5
    while time.monotonic() < end:
        now = time.monotonic()
        rot, pos = base.unit_state(ctx.mem, ctx.player_unit())
        samples.append({"t": round(now, 3),
                        "since_detached_ms": round((now - detached_at) * 1000.0, 1) if detached_at else None,
                        "pos": pos, "quat": rot})
        time.sleep(0.1)
    packets += ctx.collect(0.5)
    sl.add("c_dismount", packets)
    sl.state("c_dismount", "dismounted", unbind_sent=has(packets, "C->S", 0x0A8),
             detached=has(packets, "S->C", 0x077),
             # request -> SCUnitDetached only; the sampling window is not part of the reply latency
             total_ms=round((detached_at - t0) * 1000.0, 1) if detached_at else None,
             samples=samples, mount=ctx.mount_status(), text=capture_text(ctx.mcp, ctx.pid))

    sl.note("(d) remote rider: SKIPPED - the machine rule allows one original client for this task.")
    probe = ctx.ui(
        "local e=System.GetEntityByName and System.GetEntityByName(X2Unit:UnitName('player'))"
        " local out={tostring(e~=nil)}"
        " if e then out[#out+1]=type(e.GetAnimationGraphState)~='nil' and 'animgraph' or 'no-animgraph'"
        " out[#out+1]=type(e.GetCurrentAnimation)~='nil' and 'curanim' or 'no-curanim' end"
        " return table.concat(out,',')").strip()
    sl.note("Animation graph/ActionController probe: '" + probe
            + "' ; animation state/clip names are NOT readable by this driver. Recorded instead: the "
              "CSMountMate reason byte (get-on side), SCUnitAttached, and position/quaternion samples.")


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("MOUNT_CLIENT", "archeage.exe"),
                        help="path to the original 1.2 archeage.exe")
    parser.add_argument("--driver", default=os.environ.get("MOUNT_DRIVER", "aaemu-clientdriver.exe"),
                        help="path to AAEmu's aaemu-clientdriver.exe")
    parser.add_argument("--driver-dll", default=os.environ.get("MOUNT_DRIVER_DLL", "aadriver.dll"),
                        help="path to the x86 aadriver.dll")
    parser.add_argument("--launcher", default=os.environ.get("MOUNT_LAUNCHER", "aaemu-launcher.exe"),
                        help="path to AAEmu's aaemu-launcher.exe")
    parser.add_argument("--login", default=os.environ.get("MOUNT_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("MOUNT_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("MOUNT_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("MOUNT_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("MOUNT_SERVER_LOG"),
                        help="game server log with the fields trace")
    parser.add_argument("--item", type=int, default=DEFAULT_ITEM,
                        help=f"summon item template (default {DEFAULT_ITEM}, Heir's Horse)")
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--create-character", action="store_true",
                        help="create the character through the original creation UI when the account has none")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "mount_animations"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-animations-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set MOUNT_{name.upper()})")
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
    launched = False
    try:
        if pid is None:
            launch = mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                              user=args.account, password_file=str(password_file))
            log(launch.replace("\n", " | "))
            match = re.search(r"pid (\d+)", launch)
            if not match:
                raise RuntimeError("client_launch did not return a pid")
            pid = int(match.group(1))
            launched = True
            base.wait_stage(mcp, pid, 3, 240)
            lua_text(mcp, pid, "X2World:EnterWorld(1,0)")
            for _ in range(240):
                if lua_text(mcp, pid, "return X2LoginCharacter:GetCurrentStage()").strip() in ("4", "5"):
                    break
                time.sleep(0.5)
            create_character(mcp, pid, args.character, args.create_character)
            time.sleep(5.0)
        else:
            base.wait_player(mcp, pid, 30)

        install = lua_text(mcp, pid, CAPTURE_INSTALL).strip()
        if install != "ok":
            log("text capture install:", install)

        mem = base.Mem(pid)
        for name, _ in mem.modules():
            if any(bad in name.lower() for bad in base.ANTI_CHEAT):
                raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
        base_addr = mem.module_base("x2game.dll")
        if not base_addr:
            raise RuntimeError("x2game.dll not loaded")
        log("x2game base", hex(base_addr))

        trace = Trace(server_log)
        ctx = Ctx(mcp, pid, mem, base_addr, trace, out)
        sl = ctx.slice("s3")
        try:
            slice_s3(ctx, args.item)
        finally:
            sl.write()
            report = [
                "How measured:",
                "  server: RUST_LOG=info,aaemu_game::net::fields=trace (the C->S field trace); the S->C packets come from",
                "          the server's GamePacket lines as opcode + byte count only",
                "  client: the original 1.2 client (r208022), hidden, one launch, driven by aaemu-clientdriver",
                f"  lane:   {args.login}; account {args.account} / {args.character}",
                f"  date:   {time.strftime('%Y-%m-%d')}",
                "  probe:  python tools/mount_animations_probe.py --client <client> --driver <driver> "
                "--driver-dll <driver-dll> --launcher <launcher> --login <host:port> --account <account> "
                "--character <character> --password-file <password-file> --server-log <server-log> "
                "--create-character --out tools/evidence/mount_animations",
                "",
            ] + sl.notes + [
                "",
                "Files:",
                "  native.csv   tools/evidence/mount_animations/native.csv",
                "  native.json  tools/evidence/mount_animations/native.json",
            ]
            (out / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
            log("done; report", out / "report.txt")
        return 0
    finally:
        # only a client this run launched may be stopped: --pid attaches to the user's own client
        if launched and pid is not None:
            try:
                log("stopping client", pid)
                mcp.text("client_stop", pid=pid)
            except Exception as error:
                log("client_stop failed:", error)
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        elif pid is not None:
            log("leaving attached client", pid, "running")
        mcp.close()


if __name__ == "__main__":
    raise SystemExit(main())
