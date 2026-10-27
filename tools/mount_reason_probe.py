#!/usr/bin/env python3
"""Capture CSMountMate reason 1..4 from the running 1.2 client, read-only.

The original client computes the mount approach reason in `x2game.dll`
`FUN_3909ae90` (decompile line 134731) from the mate's facing and the vector
from the player to the mate, and sends it in `CSMountMatePacket` (opcode
0x0A7).  This probe:

* drives the hidden original through aaemu-clientdriver (no OS input, no
  injection beyond the normal client driver),
* reads the player and mate unit positions and the mate's entity quaternion
  with ReadProcessMemory (PROCESS_VM_READ only),
* reads the eight constants the function uses from the running x2game.dll,
* walks the character to the front, back, left, right and diagonal sides of
  the summoned horse and presses the mount action there,
* records the reason byte from the server's `CSMountMatePacket fields` trace,
* compares every captured case with the arithmetic of the decompiled function.

Bulky raw output belongs under `--raw` (default `data/extract/mount-reason-
<date>-r1`); the committed CSV and report.txt are written to `--out`
(default `tools/evidence/mount_reason`).  The script stops every client it
starts, by pid, in a finally block.

Usage:
    python tools/mount_reason_probe.py --client <archeage.exe> \
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \
        --launcher <aaemu-launcher.exe> --login <host:port> \
        --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        --out tools/evidence/mount_reason
    python tools/mount_reason_probe.py --pid 1234 ...  # already in world

The driver is NickMesser/AAEmu's `aaemu-clientdriver`
(`rust/crates/aaemu-clientdriver`): build it with
`cargo build -p aaemu-clientdriver` and build the x86 `aadriver.dll` with the
driver's `native/build.bat` (or use the DLL from the same AAEmu build).  Pass
the driver, DLL and launcher paths with `--driver`, `--driver-dll` and
`--launcher`; the defaults are the bare file names.

The game server must log the CSMountMate fields:
`RUST_LOG=info,aaemu_game::net::fields=trace` (the `aaemu_game::net::fields`
target).  The script reads that log at `--server-log`.

The 1.2 client must be hidden (`client_launch ... hidden=True`); this script
does that.  It refuses to run if an anti-cheat module is loaded.
"""
from __future__ import annotations

import argparse
import csv
import ctypes
import json
import math
import os
import re
import struct
import subprocess
import sys
import threading
import time
from ctypes import wintypes
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

ITEM = 8159  # Brown Lilyut Horse
HORSE_NAME = "Brown Lilyut Horse"
ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect", "gameguard", "hackshield")

# Absolute virtual addresses in the 1.2 r208022 x2game.dll image dumped at
# base 0x38FF0000.  At run time
# add (address - DUMP_BASE) to the module base from the process.
DUMP_BASE = 0x38FF0000
G_CONNECTION = 0x39B02294       # net::g_connection global in FUN_3909b070
UNIT_MAP = 0x3A169998           # tree object used by FUN_39360020
CONSTANTS = {
    "DAT_3999cffc": (0x3999CFFC, 0x40000000, 2.0),
    "_DAT_3999c488": (0x3999C488, 0x3F800000, 1.0),
    "_DAT_3999cc18": (0x3999CC18, 0x00800000, 1.1754943508222875e-38),
    "DAT_3999c478": (0x3999C478, 0x3FC00000, 1.5),
    "DAT_3999c47c": (0x3999C47C, 0x3F000000, 0.5),
    "_DAT_399a2d28": (0x399A2D28, 0x3F75C28F, 0.9599999785423279),
    "_DAT_399a2d24": (0x399A2D24, 0xBF75C28F, -0.9599999785423279),
    "DAT_3999c484": (0x3999C484, 0x00000000, 0.0),
}

SIDES = [
    (0, "front"),
    (45, "front-right"),
    (90, "right"),
    (135, "back-right"),
    (180, "back"),
    (225, "back-left"),
    (270, "left"),
    (315, "front-left"),
]


def log(*a: object) -> None:
    print("mount-reason:", *a, flush=True)


class Mcp:
    def __init__(self, driver: Path, dll: Path, launcher: Path, client: Path, login: str,
                 password_file: Path, user: str, artifact: Path):
        self.process = subprocess.Popen(
            [str(driver), "mcp", "--dll", str(dll), "--client", str(client), "--launcher", str(launcher),
             "--password-file", str(password_file), "--login", login, "--user", user,
             "--artifact-dir", str(artifact)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8",
        )
        self.sequence = 0
        self.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "mount-reason-probe", "version": "1"}})
        assert self.process.stdin and self.process.stdout
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def call(self, method: str, params: object = None) -> object:
        assert self.process.stdin and self.process.stdout
        self.sequence += 1
        request: dict[str, object] = {"jsonrpc": "2.0", "id": self.sequence, "method": method}
        if params is not None:
            request["params"] = params
        self.process.stdin.write(json.dumps(request) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("driver MCP closed stdout")
        response = json.loads(line)
        if response.get("id") != self.sequence:
            raise RuntimeError(f"driver MCP id mismatch: {response}")
        if "error" in response:
            raise RuntimeError(str(response["error"]))
        return response["result"]

    def tool(self, name: str, **arguments: object) -> dict:
        result = self.call("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError("\n".join(c.get("text", "") for c in result.get("content", [])))
        return result

    def text(self, name: str, **arguments: object) -> str:
        return "\n".join(c["text"] for c in self.tool(name, **arguments)["content"] if c.get("type") == "text")

    def close(self) -> None:
        try:
            assert self.process.stdin
            self.process.stdin.close()
            self.process.wait(timeout=15)
        except Exception:
            self.process.kill()


class Mem:
    def __init__(self, pid: int):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.handle = self.kernel.OpenProcess(0x0410, False, pid)  # QUERY_INFORMATION | VM_READ
        if not self.handle:
            raise RuntimeError("OpenProcess failed")
        self.pid = pid

    def read(self, address: int, size: int) -> bytes | None:
        buf = (ctypes.c_char * size)()
        got = ctypes.c_size_t()
        if not self.kernel.ReadProcessMemory(self.handle, ctypes.c_void_p(address), buf, size, ctypes.byref(got)):
            return None
        return bytes(buf[: got.value])

    def u32(self, address: int) -> int:
        data = self.read(address, 4)
        return struct.unpack("<I", data)[0] if data and len(data) == 4 else 0

    def u8(self, address: int) -> int:
        data = self.read(address, 1)
        return data[0] if data else 0

    def f32s(self, address: int, count: int) -> list[float] | None:
        data = self.read(address, 4 * count)
        return list(struct.unpack("<%df" % count, data)) if data and len(data) == 4 * count else None

    def modules(self) -> list[tuple[str, int]]:
        class ME(ctypes.Structure):
            _fields_ = [("dwSize", wintypes.DWORD), ("th32ModuleID", wintypes.DWORD),
                        ("th32ProcessID", wintypes.DWORD), ("GlblcntUsage", wintypes.DWORD),
                        ("ProccntUsage", wintypes.DWORD), ("modBaseAddr", ctypes.c_void_p),
                        ("modBaseSize", wintypes.DWORD), ("hModule", wintypes.HMODULE),
                        ("szModule", ctypes.c_wchar * 256), ("szExePath", ctypes.c_wchar * 260)]
        snap = self.kernel.CreateToolhelp32Snapshot(0x18, self.pid)
        out: list[tuple[str, int]] = []
        me = ME()
        me.dwSize = ctypes.sizeof(ME)
        ok = self.kernel.Module32FirstW(snap, ctypes.byref(me))
        while ok:
            out.append((me.szModule, int(me.modBaseAddr or 0)))
            ok = self.kernel.Module32NextW(snap, ctypes.byref(me))
        self.kernel.CloseHandle(snap)
        return out

    def module_base(self, name: str) -> int:
        for module, base in self.modules():
            if module.lower() == name.lower():
                return base
        return 0


class Sampler(threading.Thread):
    def __init__(self, mem: Mem, player_unit: int, mate_unit: int):
        super().__init__(daemon=True)
        self.mem = mem
        self.player_unit = player_unit
        self.mate_unit = mate_unit
        self.rows: list[tuple[float, list[float], list[float], list[float], list[float]]] = []
        self.stop = False

    def run(self) -> None:
        while not self.stop:
            stamp = time.time()
            _, player_entity = entity_of(self.mem, self.player_unit)
            _, mate_entity = entity_of(self.mem, self.mate_unit)
            if player_entity and mate_entity:
                player_rot, player_pos = ent_state(self.mem, player_entity)
                mate_rot, mate_pos = ent_state(self.mem, mate_entity)
                if player_pos and mate_pos and player_rot and mate_rot:
                    self.rows.append((stamp, player_pos, player_rot, mate_pos, mate_rot))
            time.sleep(0.001)


def entity_of(mem: Mem, unit: int) -> tuple[int, int | None]:
    """The entity behind a unit: unit+0x3be8 model, [model+0x1f0]+0xc object, +0x80 transform."""
    model = mem.u32(unit + 0x3BE8)
    if not model:
        return 0, None
    holder = mem.u32(model + 0x1F0)
    if not holder:
        return model, None
    obj = mem.u32(holder + 0x0C)
    return model, (obj + 0x80 if obj else None)


def ent_state(mem: Mem, entity: int) -> tuple[list[float] | None, list[float] | None]:
    """Entity quaternion [x,y,z,w] at +0 and position [x,y,z] at +0x10."""
    return mem.f32s(entity, 4), mem.f32s(entity + 0x10, 3)


def walk_units(mem: Mem, base: int) -> tuple[int, list[tuple[int, int]]]:
    """Walk the unit tree used by FUN_39360020 (circular RB tree, node +0xc key, +0x10 unit)."""
    header = mem.u32(base + (UNIT_MAP - DUMP_BASE) + 4)
    out: list[tuple[int, int]] = []
    seen: set[int] = set()

    def walk(node: int, depth: int = 0) -> None:
        if not node or node in seen or depth > 60 or node == header:
            return
        seen.add(node)
        left = mem.u32(node)
        right = mem.u32(node + 8)
        key = mem.u32(node + 0x0C)
        value = mem.u32(node + 0x10)
        out.append((key, value))
        walk(left, depth + 1)
        walk(right, depth + 1)

    root = mem.u32(header + 4) if header else 0
    walk(root)
    return header, out


def unit_states(mem: Mem, base: int) -> tuple[int, dict[int, tuple[int, int, list[float], list[float]]]]:
    player_unit = mem.u32(mem.u32(base + (G_CONNECTION - DUMP_BASE)) + 0x10C)
    _, units = walk_units(mem, base)
    result: dict[int, tuple[int, int, list[float], list[float]]] = {}
    for key, unit in units:
        model, entity = entity_of(mem, unit)
        if entity:
            rot, pos = ent_state(mem, entity)
            if pos and rot:
                result[unit] = (key, entity, rot, pos)
    return player_unit, result


def identify(mem: Mem, base: int, target: list[float]) -> tuple[float, int, int, int, list[float], list[float]] | None:
    _, units = unit_states(mem, base)
    best = None
    for unit, (key, entity, rot, pos) in units.items():
        dist = (pos[0] - target[0]) ** 2 + (pos[1] - target[1]) ** 2 + (pos[2] - target[2]) ** 2
        if dist < 0.01 and (best is None or dist < best[0]):
            best = (dist, unit, key, entity, rot, pos)
    return best


def gates(mem: Mem, mate_unit: int) -> tuple[int | None, int | None]:
    """FUN_390c9f40(model, 1) and (model, 2): circular list at model+0x120, node key +0xc, value +8."""
    model = mem.u32(mate_unit + 0x3BE8)
    if not model:
        return None, None
    head = mem.u32(model + 0x120)
    out: dict[int, int] = {}
    node = mem.u32(head) if head else 0
    for _ in range(256):
        if not node or node == head:
            break
        out.setdefault(mem.u32(node + 0x0C), mem.u32(node + 8))
        node = mem.u32(node)
    return out.get(1), out.get(2)


def read_constants(mem: Mem, base: int) -> dict[str, tuple[int, int, float | None]]:
    out: dict[str, tuple[int, int, float | None]] = {}
    for name, (va, expected_bits, expected) in CONSTANTS.items():
        bits = mem.u32(base + (va - DUMP_BASE))
        value = struct.unpack("<f", struct.pack("<I", bits))[0]
        out[name] = (bits, expected_bits, value if value == value else None)
        if bits != expected_bits:
            raise RuntimeError(f"{name} at {va:#x}: 0x{bits:08x}, expected 0x{expected_bits:08x} ({expected})")
    return out


def quat_g(q: list[float]) -> tuple[float, float, float]:
    """The three components FUN_3909ae90 builds: gx=(w*w+y*y)*C-D, gy=(x*y-z*w)*C, gz=(w*x+z*y)*C."""
    x, y, z, w = q
    gx = (w * w + y * y) * 2.0 - 1.0
    gy = (x * y - z * w) * 2.0
    gz = (w * x + z * y) * 2.0
    return gx, gy, gz


def predict(player: list[float], mate: list[float], rot: list[float], gate1: int = 0, gate2: int = 0) -> tuple[int, float, float]:
    """Python mirror of mount_reason for report.txt (float32 rounding is not emulated)."""
    gx, gy, gz = quat_g(rot)
    dx = mate[0] - player[0]
    dy = mate[1] - player[1]
    dz = mate[2] - player[2]
    length = math.sqrt(dy * dy + dx * dx + dz * dz + 1.1754943508222875e-38)
    inv = 1.0 / length
    dot = (dy * inv * gx + dx * inv * gy) + dz * inv * gz
    lateral = dy * inv * gy - dx * inv * gx
    if dot >= 0.9599999785423279:
        reason = 1 if gate2 else 3
    elif dot < -0.9599999785423279:
        reason = 4
        if gate1:
            reason = 1
    else:
        reason = 2 if lateral >= 0.0 else 1
    return reason, dot, lateral


def last_reason(path: Path, offset: int) -> tuple[int, int, int] | None:
    if not path.exists():
        return None
    data = path.read_bytes()[offset:].decode("utf-8", "replace")
    found = None
    for line in data.splitlines():
        match = re.search(r"CSMountMatePacket fields: CSMountMatePacket \{ tl_id: (\d+), ap: (\d+), reason: (\d+) \}", line)
        if match:
            found = tuple(int(v) for v in match.groups())
    return found


def lua_pos(mcp: Mcp, pid: int, name: str) -> list[float] | None:
    code = """
local out=''
for _,e in pairs(System.GetEntities()or{}) do
  if e:GetName()==NAME then local p=e:GetWorldPos() out=string.format('%.9g %.9g %.9g',p.x,p.y,p.z) end
end
return out
""".replace("NAME", json.dumps(name))
    values = mcp.text("client_lua", pid=pid, target="system", code=code).split()
    return [float(v) for v in values] if len(values) == 3 else None


def lua_yaw(mcp: Mcp, pid: int, name: str) -> float | None:
    code = """
local out=''
for _,e in pairs(System.GetEntities()or{}) do
  if e:GetName()==NAME then out=string.format('%.9g',e:GetAngles().z) end
end
return out
""".replace("NAME", json.dumps(name))
    value = mcp.text("client_lua", pid=pid, target="system", code=code).strip()
    try:
        return float(value)
    except ValueError:
        return None


def wrap(angle: float) -> float:
    return (angle + math.pi) % (2.0 * math.pi) - math.pi


def unit_state(mem: Mem, unit: int) -> tuple[list[float] | None, list[float] | None]:
    _, entity = entity_of(mem, unit)
    if not entity:
        return None, None
    return ent_state(mem, entity)


def quat_yaw(q: list[float]) -> float:
    x, y, z, w = q
    return math.atan2(2.0 * (w * z + x * y), 1.0 - 2.0 * (y * y + z * z))


def set_heading(mcp: Mcp, pid: int, mem: Mem, player_unit: int, radians: float) -> bool:
    for _ in range(24):
        rot, _ = unit_state(mem, player_unit)
        if not rot:
            time.sleep(0.02)
            continue
        delta = wrap(radians - quat_yaw(rot))
        if abs(delta) < 0.06:
            return True
        millis = int(min(700.0, max(45.0, abs(math.degrees(delta)) / 90.0 * 1000.0)))
        mcp.text("client_input", pid=pid, key="a" if delta > 0 else "d", duration_ms=millis)
        time.sleep(millis / 1000.0 + 0.05)
    return False


def goto(mcp: Mcp, pid: int, mem: Mem, player_unit: int, target: list[float], tolerance: float = 0.45,
         max_steps: int = 10) -> bool:
    for _ in range(max_steps):
        _, pos = unit_state(mem, player_unit)
        if not pos:
            return False
        dx, dy = target[0] - pos[0], target[1] - pos[1]
        distance = math.hypot(dx, dy)
        if distance < tolerance:
            return True
        if not set_heading(mcp, pid, mem, player_unit, math.atan2(-dx, dy)):
            continue
        millis = int(min(400.0, max(70.0, distance / 4.5 * 1000.0)))
        mcp.text("client_input", pid=pid, key="w", duration_ms=millis)
        time.sleep(millis / 1000.0 + 0.05)
    return False


def approach(mcp: Mcp, pid: int, mem: Mem, player_unit: int, mate_unit: int, stop: float = 3.0,
             max_steps: int = 10) -> bool:
    for _ in range(max_steps):
        _, player_pos = unit_state(mem, player_unit)
        _, mate_pos = unit_state(mem, mate_unit)
        if not player_pos or not mate_pos:
            return False
        dx, dy = mate_pos[0] - player_pos[0], mate_pos[1] - player_pos[1]
        distance = math.hypot(dx, dy)
        if distance <= stop:
            return True
        if not set_heading(mcp, pid, mem, player_unit, math.atan2(-dx, dy)):
            continue
        millis = int(min(240.0, max(60.0, (distance - stop) / 4.5 * 1000.0)))
        mcp.text("client_input", pid=pid, key="w", duration_ms=millis)
        time.sleep(millis / 1000.0 + 0.05)
    return False


def orbit_to(mcp: Mcp, pid: int, mem: Mem, player_unit: int, mate_unit: int, target_deg: float,
             radius: float = 4.2) -> bool:
    """Walk around the horse to the requested relative side without crossing it."""
    target = math.radians(target_deg)
    for _ in range(32):
        player_rot, player_pos = unit_state(mem, player_unit)
        mate_rot, mate_pos = unit_state(mem, mate_unit)
        if not player_pos or not mate_pos or not mate_rot:
            return False
        gx, gy, _ = quat_g(mate_rot)
        norm = math.hypot(gx, gy) or 1.0
        fx, fy = gy / norm, gx / norm
        rx, ry = fy, -fx
        dx, dy = player_pos[0] - mate_pos[0], player_pos[1] - mate_pos[1]
        distance = math.hypot(dx, dy)
        relative = math.atan2(dx * rx + dy * ry, dx * fx + dy * fy)
        diff = wrap(target - relative)
        if abs(diff) < math.radians(10.0) and 2.8 <= distance <= 5.5:
            return True
        step = max(-math.radians(45.0), min(math.radians(45.0), diff))
        next_angle = relative + step
        point = [mate_pos[0] + radius * (math.cos(next_angle) * fx + math.sin(next_angle) * rx),
                 mate_pos[1] + radius * (math.cos(next_angle) * fy + math.sin(next_angle) * ry)]
        goto(mcp, pid, mem, player_unit, point, tolerance=0.7, max_steps=6)
    return False


def settle(mem: Mem, player_unit: int, mate_unit: int, timeout: float = 2.5, stable: float = 0.15) -> bool:
    """Wait until the player and horse stops moving, so the call's state is the sampled state."""
    last = None
    since = None
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        player_rot, player_pos = unit_state(mem, player_unit)
        mate_rot, mate_pos = unit_state(mem, mate_unit)
        if not player_pos or not mate_pos or not player_rot or not mate_rot:
            return False
        state = (mate_pos, mate_rot, player_pos)
        if last is not None:
            dpos = max(abs(state[0][i] - last[0][i]) for i in range(3))
            dquat = max(abs(state[1][i] - last[1][i]) for i in range(4))
            dplayer = max(abs(state[2][i] - last[2][i]) for i in range(3))
            if dpos < 1e-4 and dquat < 1e-5 and dplayer < 1e-3:
                if since is None:
                    since = time.monotonic()
                elif time.monotonic() - since >= stable:
                    return True
            else:
                since = None
        last = state
        time.sleep(0.02)
    return True


def mounted(mcp: Mcp, pid: int) -> bool:
    return mcp.text("client_lua", pid=pid, target="ui", code="return tostring(X2Mate:IsPlayerMounting())").strip() == "true"


def pet_exists(mcp: Mcp, pid: int) -> bool:
    return mcp.text("client_lua", pid=pid, target="ui", code="return tostring(X2Mate:IsPlayerPetExists())").strip() == "true"


def dismount(mcp: Mcp, pid: int) -> bool:
    if mounted(mcp, pid):
        mcp.text("client_lua", pid=pid, target="ui", code="X2Mate:UnMountMate() return 1")
        end = time.monotonic() + 6.0
        while time.monotonic() < end and mounted(mcp, pid):
            time.sleep(0.2)
        time.sleep(1.0)
    return not mounted(mcp, pid)


def wait_stage(mcp: Mcp, pid: int, stage: int, timeout: float) -> None:
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            last = mcp.text("client_lua", pid=pid, target="ui",
                            code="return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()").strip()
            if last == str(stage):
                return
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError(f"stage {stage} not reached: {last}")


def wait_player(mcp: Mcp, pid: int, timeout: float) -> None:
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            state = mcp.text("client_lua", pid=pid, target="ui",
                             code="return X2LoginCharacter:GetCurrentStage(), X2Unit and X2Unit:UnitName('player')")
            stage, _, name = state.partition("\t")
            if stage == "0" and name not in ("", "nil"):
                return
            last = state
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError("player did not appear: " + last)


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
                        help="game server log with the CSMountMate field trace")
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--reps", type=int, default=3, help="attempts per requested side (8 sides)")
    parser.add_argument("--angles", help="comma-separated requested angles instead of all eight sides")
    parser.add_argument("--append", action="store_true", help="append the cases to an existing native.json capture")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "mount_reason"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-reason-{time.strftime('%Y%m%d')}-r1"))
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
    json_path = out / "native.json"
    csv_path = out / "native.csv"
    if args.append and json_path.exists():
        previous = json.loads(json_path.read_text(encoding="utf-8"))
        rows = previous.get("cases", [])
        log(f"appending to {len(rows)} previous cases")
    else:
        rows = []

    log(f"account {args.account}: {args.character}")
    mcp = Mcp(driver, dll, launcher, client, args.login, password_file, args.account, raw / "mcp")
    pid = args.pid
    constants: dict[str, tuple[int, int, float | None]] = {}
    try:
        if pid is None:
            launch = mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                              user=args.account, password_file=str(password_file))
            log(launch.replace("\n", " | "))
            match = re.search(r"pid (\d+)", launch)
            if not match:
                raise RuntimeError("client_launch did not return a pid")
            pid = int(match.group(1))
            wait_stage(mcp, pid, 3, 240)
            mcp.text("client_lua", pid=pid, target="ui", code="X2World:EnterWorld(1,0)")
            wait_stage(mcp, pid, 4, 240)
            mcp.text("client_lua", pid=pid, target="ui",
                     code="local b=characterSelectWindow.bottomPanel.startButton b:OnClick('LeftButton')")
            wait_player(mcp, pid, 240)
            time.sleep(3.0)
        else:
            wait_player(mcp, pid, 30)

        mem = Mem(pid)
        modules = mem.modules()
        for name, _ in modules:
            lower = name.lower()
            if any(bad in lower for bad in ANTI_CHEAT):
                raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
        base = mem.module_base("x2game.dll")
        if not base:
            raise RuntimeError("x2game.dll not loaded")
        constants = read_constants(mem, base)
        log("x2game base", hex(base), "constants", {k: repr(v[2]) for k, v in constants.items()})

        mcp.text("client_chat", pid=pid, message="/move 10512 15585 232")
        time.sleep(2.5)
        if not pet_exists(mcp, pid):
            mcp.text("client_use_item", pid=pid, item_template_id=ITEM)
            end = time.monotonic() + 25
            while time.monotonic() < end and not pet_exists(mcp, pid):
                time.sleep(0.3)
        if not pet_exists(mcp, pid):
            raise RuntimeError("the horse could not be summoned")
        time.sleep(2.0)
        horse = lua_pos(mcp, pid, HORSE_NAME)
        found = identify(mem, base, horse) if horse else None
        if not found:
            raise RuntimeError("the summoned horse has no unit")
        mate_unit = found[1]
        log("horse summoned", hex(mate_unit))

        wanted = SIDES
        if args.angles:
            requested = {int(value) for value in args.angles.split(",") if value.strip()}
            wanted = [(angle, label) for angle, label in SIDES if angle in requested]
        attempts = [(rep, angle, label) for rep in range(args.reps) for angle, label in wanted]
        for index, (rep, angle, label) in enumerate(attempts):
            if not dismount(mcp, pid):
                raise RuntimeError("dismount did not finish")
            if not pet_exists(mcp, pid):
                raise RuntimeError(f"pet vanished before attempt {index + 1}")
            player_unit = mem.u32(mem.u32(base + (G_CONNECTION - DUMP_BASE)) + 0x10C)
            if not unit_state(mem, player_unit)[1]:
                player_unit, _ = unit_states(mem, base)
            if not unit_state(mem, mate_unit)[1]:
                horse = lua_pos(mcp, pid, HORSE_NAME)
                found = identify(mem, base, horse) if horse else None
                if not found:
                    log(f"attempt {index + 1} ({label}): horse unit lost")
                    continue
                mate_unit = found[1]
            if not orbit_to(mcp, pid, mem, player_unit, mate_unit, angle):
                log(f"attempt {index + 1} ({label}): orbit failed")
            if not approach(mcp, pid, mem, player_unit, mate_unit):
                log(f"attempt {index + 1} ({label}): approach failed")
            settle(mem, player_unit, mate_unit)
            player_rot, player_pos = unit_state(mem, player_unit)
            mate_rot, mate_pos = unit_state(mem, mate_unit)
            if not player_pos or not mate_pos:
                log(f"attempt {index + 1} ({label}): unit state lost")
                continue
            gate1, gate2 = gates(mem, mate_unit)
            if not pet_exists(mcp, pid):
                raise RuntimeError(f"pet vanished before mount {index + 1}")
            offset = server_log.stat().st_size if server_log.exists() else 0
            sampler = Sampler(mem, player_unit, mate_unit)
            sampler.start()
            started = time.time()
            mcp.text("client_lua", pid=pid, target="ui", code="X2Mate:MountMate() return 1")
            reason = None
            end = time.monotonic() + 3.0
            while time.monotonic() < end:
                reason = last_reason(server_log, offset)
                if reason:
                    break
                time.sleep(0.05)
            sampler.stop = True
            sampler.join(timeout=2.0)
            is_mounted = mounted(mcp, pid)
            time.sleep(0.2)
            if not reason:
                # One retry without moving: a just-finished dismount can still be settling.
                mcp.text("client_lua", pid=pid, target="ui", code="X2Mate:MountMate() return 1")
                end = time.monotonic() + 2.0
                while time.monotonic() < end and not reason:
                    reason = last_reason(server_log, offset)
                    time.sleep(0.05)
                is_mounted = mounted(mcp, pid)
            after_start = [row for row in sampler.rows if row[0] >= started]
            best = after_start[0] if after_start else (sampler.rows[-1] if sampler.rows else None)
            if not best:
                log(f"attempt {index + 1} ({label}): no memory samples")
                continue
            _, player_pos, player_rot, mate_pos, mate_rot = best
            gx, gy, gz = quat_g(mate_rot)
            norm = math.hypot(gx, gy) or 1.0
            fx, fy = gy / norm, gx / norm
            rx, ry = fy, -fx
            dx, dy = player_pos[0] - mate_pos[0], player_pos[1] - mate_pos[1]
            relative = math.degrees(math.atan2(dx * rx + dy * ry, dx * fx + dy * fy))
            sector = int(round(((relative % 360.0) + 360.0) % 360.0 / 45.0)) % 8
            actual_label = SIDES[sector][1]
            predicted, dot, lateral = predict(player_pos, mate_pos, mate_rot, gate1 or 0, gate2 or 0)
            row = {
                "attempt": len(rows) + 1,
                "rep": rep + 1,
                "requested_deg": angle,
                "side": label,
                "actual_side": actual_label,
                "relative_deg": round(relative, 2),
                "player_x": player_pos[0], "player_y": player_pos[1], "player_z": player_pos[2],
                "mate_x": mate_pos[0], "mate_y": mate_pos[1], "mate_z": mate_pos[2],
                "qx": mate_rot[0], "qy": mate_rot[1], "qz": mate_rot[2], "qw": mate_rot[3],
                "player_qx": player_rot[0], "player_qy": player_rot[1],
                "player_qz": player_rot[2], "player_qw": player_rot[3],
                "dot": round(dot, 6), "lateral": round(lateral, 6),
                "gate1": gate1, "gate2": gate2,
                "native_reason": reason[2] if reason else None,
                "predicted": predicted,
                "match": bool(reason and reason[2] == predicted),
                "mounted": is_mounted,
                "tl_id": reason[0] if reason else None,
                "ap": reason[1] if reason else None,
            }
            rows.append(row)
            with open(csv_path, "w", encoding="utf-8", newline="") as handle:
                writer = csv.DictWriter(handle, fieldnames=list(row))
                writer.writeheader()
                writer.writerows(rows)
            with open(json_path, "w", encoding="utf-8") as handle:
                json.dump({"meta": {"account": "<account>", "character": args.character,
                                    "pid": pid, "date": time.strftime("%Y-%m-%dT%H:%M:%S"),
                                    "x2game_base": base, "constants": constants},
                           "cases": rows}, handle, indent=1)
            log(f"{row['attempt']:2d}/{len(attempts)} {label:11s} requested {angle:3d} actual {relative:7.1f} "
                f"({actual_label}) reason {row['native_reason']} predicted {predicted} dot {dot:+.4f} "
                f"{'OK' if row['match'] else 'MISMATCH'}")
            if is_mounted:
                dismount(mcp, pid)
            else:
                time.sleep(0.5)

        matched = sum(1 for row in rows if row["match"])
        baseline = sum(1 for row in rows if row["native_reason"] == 1)
        reason_counts = {reason: sum(1 for row in rows if row["native_reason"] == reason) for reason in (1, 2, 3, 4)}
        side_counts = {label: sum(1 for row in rows if row["actual_side"] == label and row["native_reason"]) for _, label in SIDES}
        margins = [min(abs(row["dot"] - 0.96), abs(row["dot"] + 0.96)) for row in rows]
        report = [
            "How to reproduce:",
            "  server: RUST_LOG=info,aaemu_game::net::fields=trace (the CSMountMate field trace)",
            "  client: the original 1.2 client (r208022), character next to a summoned Brown Lilyut Horse (item 8159)",
            "  probe: python tools/mount_reason_probe.py --client <client> --driver <driver> --driver-dll <driver-dll> --launcher <launcher> --login <host:port> --account <account> --character <character> --password-file <password-file> --server-log <server-log> --out tools/evidence/mount_reason",
            "  replay: python tools/mount_reason_replay.py --client <aa-client> --login <host:port> --account <account> --character <character> --password-file <password-file> --server-log <server-log> --evidence tools/evidence/mount_reason",
            "",
            "mount approach reason: native 1.2 capture vs FUN_3909ae90 arithmetic",
            f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
            f"account: {args.account} / {args.character}",
            f"client: {client} (hidden)  pid: {pid}  x2game base: {base:#x}",
            f"server log: {server_log}",
            f"source commit: {subprocess.run(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], capture_output=True, text=True).stdout.strip()}",
            "",
            "constants read from the running x2game.dll (address, bits, value):",
        ]
        for name, (bits, expected, value) in constants.items():
            report.append(f"  {name:16s} {bits:#010x}  {value!r}")
        report += [
            "",
            f"cases: {len(rows)} attempts, {matched} predicted matches, {len(rows) - matched} mismatches",
            f"pre-fix constant reason 1 would match {baseline}/{len(rows)}; {len(rows) - baseline} captured approaches differ",
            f"native reasons: {reason_counts}",
            f"requested sides: {side_counts}",
            f"smallest distance of a captured dot from the +/-0.96 thresholds: {min(margins):.6f}" if margins else "",
            "",
            "The prediction mirrors FUN_3909ae90: d = mate - player, q = mate entity quaternion; "
            "gx=(w*w+y*y)*2-1, gy=(x*y-z*w)*2, gz=(w*x+z*y)*2; dot = dy*gx + dx*gy + dz*gz; "
            "dot >= 0.96 -> 3, dot < -0.96 -> 4, otherwise 2 if lateral >= 0 else 1 "
            "(lateral = dy*gy - dx*gx).  The fast rsqrtss is replaced by 1/sqrt in this report; "
            "the captured margins show whether that could change a reason.",
            "",
            "case table:",
            "  #  requested    actual       req  actual     dot   lateral  native  predicted  match",
        ]
        for row in rows:
            report.append(
                f"  {row['attempt']:2d}  {row['side']:11s}  {row['actual_side']:11s} {row['requested_deg']:3d}  {row['relative_deg']:7.1f}  "
                f"{row['dot']:+.4f}  {row['lateral']:+.4f}  {str(row['native_reason']):>6s}  {row['predicted']:>9d}  "
                f"{'yes' if row['match'] else 'NO'}"
            )
        report += [
            "",
            "Raw server log excerpt:",
        ]
        if server_log.exists():
            text = server_log.read_text(encoding="utf-8", errors="replace")
            for line in text.splitlines():
                if "CSMountMatePacket" in line:
                    report.append("  " + line)
        report += [
            "",
            "Files:",
            f"  native.csv   {csv_path}",
            f"  native.json  {json_path}",
            f"  raw run dir  {raw}",
        ]
        (out / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
        (raw / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
        (raw / "ACTUAL_RUN_REPORT.md").write_text(
            "\n".join([
                "# mount approach reason native run",
                "",
                f"Command: `python tools/mount_reason_probe.py --client {args.client} --driver {args.driver} --driver-dll {args.driver_dll} --launcher {args.launcher} --login {args.login} --account {args.account} --character {args.character} --password-file {args.password_file} --server-log {args.server_log} --reps {args.reps}`",
                f"Date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
                f"Client: {client} (pid {pid}, hidden), driver {driver}, DLL {dll}",
                f"Account: {args.account} / {args.character}",
                f"Server log: {server_log}",
                f"Exit: {0 if matched == len(rows) else 1} after {len(rows)} attempts; native reasons {reason_counts}; "
                f"{matched} predicted matches, {len(rows) - matched} mismatches.",
                "",
                "The bulky driver artifacts are in this directory; the committed evidence is "
                "tools/evidence/mount_reason/native.json and report.txt.",
            ]) + "\n", encoding="utf-8")
        log(f"done: {matched}/{len(rows)} matches, reasons {reason_counts}; report {out / 'report.txt'}")
        return 0 if matched == len(rows) and not (len(rows) - matched) else 1
    finally:
        if pid:
            try:
                log("stopping client", pid)
                mcp.text("client_stop", pid=pid)
            except Exception as error:
                log("client_stop failed:", error)
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        mcp.close()


if __name__ == "__main__":
    raise SystemExit(main())
