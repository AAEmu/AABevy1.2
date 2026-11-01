#!/usr/bin/env python3
"""Record the original 1.2 client into per-channel JSONL for combat parity work.

The recorder attaches to an already running, hidden client (the lane stack owns
the process) and samples the client itself, never the server's replies:

* ``clock_sync``      the client's UI and game clocks beside the recorder clock;
* ``anim_state``      caster and victim actor animation state / clip time / pose;
* ``anim_controller`` the caster and victim controller lists (A/B scheduler);
* ``gcd``             the cooldown manager's global pair at unit+0x2c48 (+8/+0xc)
                       plus cast slots, stance, weapon and combat bytes;
* ``castbar``         ``X2Unit:UnitCastingInfo`` for the player and the target;
* ``buffs``           ``UnitBuff`` / ``UnitDeBuff`` for the player and target,
                       sampled once per second;
* ``unit_state``      ``client_unit_state`` for the player and target (HP/MP);
* ``chat``            the client's own chat/system-message dispatch, including
                       refusal texts;
* ``inputs``          every driver key press with client-clock stamps.

Every row carries ``client_ms`` from a client clock (``X2Time:GetUiMsec`` for UI
rows, ``System.GetCurrTime`` for system rows, and the last synced offset for
ReadProcessMemory samples) and a ``wall_ms`` recorder stamp.  See README.md in
this directory for the row schema and the decompile citations for the GCD pair.

The client is hidden before any sampling.  The tool never sends desktop input,
never calls ``X2World:LeaveWorld``, and stops the attached client by pid unless
``--keep-client`` is given.

Typical use against a lane stack (the lane's ``live-stack.sh client`` already
started the client):

    python tools/combat-side/record_original.py \\
        --pid 1234 --run-dir D:/aaemu-client/handoff/2026-11-01-combat-recorder/run \\
        --driver D:/aaemu-target/combatcap/live/aaemu-clientdriver.exe \\
        --dll D:/aaemu-target/combatcap/aadriver/aadriver.dll \\
        --launcher D:/aaemu-target/combatcap/live/aaemu-launcher.exe \\
        --client D:/AA1.2/bin32/archeage.exe --login 127.0.0.1:35237 \\
        --password-file D:/aaemu-live/cfg/client-test-password.txt \\
        --character CombatT1 --character-ability 2 \\
        --spawn-npc 6409 --npc-name "rat_brown " --fight-keys 1,1,2,1,3,2,1

The account must already exist and have GM (access_level 100) in the lane's
game database; ``prepare_account.py`` does that for a lane.  The character is
created through the original creation UI when it is missing.
"""
from __future__ import annotations

import argparse
import ctypes
import ctypes.wintypes as wt
import json
import math
import os
import re
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path

SCHEMA = "aabevy.combat-recorder/v2"
DEFAULT_UPDATER = "aabevy.combat-recorder"

DUMP_BASE = 0x38FF0000
G_CONNECTION = 0x39B02294       # local player global (docs/research/port-combat/structs.md)
UNIT_MAP = 0x3A169998           # tree used by FUN_39360020
UNIT_CD = 0x2C48                # CooldownManager, inline in Unit
GCD_REMAINING = UNIT_CD + 0x08  # FUN_3989a290 reads manager+8 / +0xc
GCD_TOTAL = UNIT_CD + 0x0C
UNIT_CAST = 0x3AC0
UNIT_STANCE = 0x3BBA
UNIT_WEAPON = 0x3BA8
UNIT_WEAPON_SLOTS = 0x3BB0
UNIT_VISUAL_BYTE = 0x3C30
UNIT_COMBAT_DEADLINE = 0x5FC0
UNIT_COMBAT_IN = 0x5FC4
UNIT_COMBAT_LATCH_A = 0x5FC5
UNIT_COMBAT_LATCH_B = 0x5FC6
UNIT_ANIM_MODEL = 0x3BE8

ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect",
              "gameguard", "hackshield")
REACTION_ANIMS = {14, 15, 17, 417, 418}

REFUSAL_HINTS = (
    "not ready", "cooldown", "cannot", "can't", "too far", "too close",
    "not enough", "no target", "invalid", "failed", "unable", "already",
    "사용할 수", "쿨다운", "너무", "부족", "없", "실패", "불가",
)


def log(*a: object) -> None:
    print("recorder:", *a, flush=True)


def ms_now() -> float:
    return time.time() * 1000.0


def clean(value: object) -> str:
    text = "" if value is None else str(value)
    return text.replace("\t", " ").replace("\r", " ").replace("\n", " ").replace("|", " ")


def s32(value: int) -> int:
    return value - 0x100000000 if value & 0x80000000 else value


class Mcp:
    """Minimal serialised newline JSON-RPC client for ``aaemu-clientdriver mcp``."""

    def __init__(self, driver: Path, dll: Path, launcher: Path, client: Path,
                 password_file: Path | None, login: str, user: str, artifact: Path):
        args = [str(driver), "mcp", "--dll", str(dll), "--client", str(client),
                "--launcher", str(launcher), "--login", login, "--user", user,
                "--artifact-dir", str(artifact)]
        if password_file is not None:
            args += ["--password-file", str(password_file)]
        self.process = subprocess.Popen(args, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE,
                                        stderr=subprocess.DEVNULL, text=True,
                                        encoding="utf-8")
        self.sequence = 0
        self.lock = threading.Lock()
        self.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "combat-recorder", "version": "2"}})
        assert self.process.stdin and self.process.stdout
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0",
                                             "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def call(self, method: str, params: object = None) -> object:
        with self.lock:
            assert self.process.stdin and self.process.stdout
            self.sequence += 1
            request: dict[str, object] = {"jsonrpc": "2.0", "id": self.sequence,
                                          "method": method}
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
            raise RuntimeError("\n".join(c.get("text", "")
                                         for c in result.get("content", [])))
        return result

    def text(self, name: str, **arguments: object) -> str:
        return "\n".join(c["text"] for c in self.tool(name, **arguments)["content"]
                         if c.get("type") == "text")

    def close(self) -> None:
        try:
            assert self.process.stdin
            self.process.stdin.close()
            self.process.wait(timeout=15)
        except Exception:
            self.process.kill()


class Mem:
    PROCESS_QUERY_INFORMATION = 0x0400
    PROCESS_VM_READ = 0x0010

    def __init__(self, pid: int):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.handle = self.kernel.OpenProcess(
            self.PROCESS_QUERY_INFORMATION | self.PROCESS_VM_READ, False, pid)
        if not self.handle:
            raise OSError(ctypes.get_last_error())
        self.pid = pid

    def read(self, address: int, size: int) -> bytes | None:
        buf = ctypes.create_string_buffer(size)
        got = ctypes.c_size_t()
        if not self.kernel.ReadProcessMemory(
                self.handle, ctypes.c_void_p(address), buf, size, ctypes.byref(got)):
            return None
        return buf.raw[:got.value]

    def u32(self, address: int) -> int:
        data = self.read(address, 4)
        return struct.unpack("<I", data)[0] if data and len(data) == 4 else 0

    def u8(self, address: int) -> int:
        data = self.read(address, 1)
        return data[0] if data else 0

    def f32(self, address: int) -> float:
        data = self.read(address, 4)
        return struct.unpack("<f", data)[0] if data and len(data) == 4 else 0.0

    def u32s(self, address: int, count: int) -> list[int]:
        data = self.read(address, 4 * count)
        return list(struct.unpack("<%dI" % count, data)) if data and len(data) == 4 * count else [0] * count

    def modules(self) -> list[tuple[str, int]]:
        class ME(ctypes.Structure):
            _fields_ = [("dwSize", wt.DWORD), ("th32ModuleID", wt.DWORD),
                        ("th32ProcessID", wt.DWORD), ("GlblcntUsage", wt.DWORD),
                        ("ProccntUsage", wt.DWORD), ("modBaseAddr", ctypes.c_void_p),
                        ("modBaseSize", wt.DWORD), ("hModule", wt.HMODULE),
                        ("szModule", ctypes.c_wchar * 256),
                        ("szExePath", ctypes.c_wchar * 260)]
        snap = self.kernel.CreateToolhelp32Snapshot(0x18, self.pid)
        out: list[tuple[str, int]] = []
        entry = ME()
        entry.dwSize = ctypes.sizeof(ME)
        ok = self.kernel.Module32FirstW(snap, ctypes.byref(entry))
        while ok:
            out.append((entry.szModule, int(entry.modBaseAddr or 0)))
            ok = self.kernel.Module32NextW(snap, ctypes.byref(entry))
        self.kernel.CloseHandle(snap)
        return out

    def module_base(self, name: str) -> int:
        for module, base in self.modules():
            if module.lower() == name.lower():
                return base
        return 0


class Clock:
    """Last known client wall clock beside the recorder wall clock."""

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.system_offset: float | None = None
        self.ui_offset: float | None = None
        self.start_client_ms: float | None = None

    def sample(self, system_ms: float | None, ui_ms: float | None,
               wall_ms: float) -> None:
        with self.lock:
            if system_ms is not None:
                self.system_offset = system_ms - wall_ms
            if ui_ms is not None:
                self.ui_offset = ui_ms - wall_ms
            start = self.ui_offset if self.ui_offset is not None else self.system_offset
            if self.start_client_ms is None and start is not None:
                self.start_client_ms = wall_ms + start

    def estimate(self, wall_ms: float | None = None) -> int:
        wall = ms_now() if wall_ms is None else wall_ms
        with self.lock:
            offset = self.ui_offset if self.ui_offset is not None else self.system_offset
        return int(round(wall + (offset or 0.0)))

    def wall_for_ui(self, ui_ms: float) -> float:
        with self.lock:
            offset = self.ui_offset
        return float(ui_ms) - (offset or 0.0)

    def wall_for_system(self, system_ms: float) -> float:
        with self.lock:
            offset = self.system_offset
        return float(system_ms) - (offset or 0.0)

    def ui_for_system(self, system_ms: float) -> int:
        with self.lock:
            system = self.system_offset
            ui = self.ui_offset
        if system is None or ui is None:
            return int(round(system_ms))
        return int(round(system_ms + (ui - system)))

    def relative(self, client_ms: float | int | None) -> int | None:
        with self.lock:
            start = self.start_client_ms
        if client_ms is None or start is None:
            return None
        return int(round(float(client_ms) - start))


class ChannelWriter:
    def __init__(self, run: Path, clock: Clock):
        self.run = run
        self.clock = clock
        self.lock = threading.Lock()
        self.handles: dict[str, object] = {}
        self.counts: dict[str, int] = {}
        self.sequences: dict[str, int] = {}

    def write(self, channel: str, *, client_ms: int | float | None = None,
              wall_ms: float | None = None, clock_source: str = "estimate",
              **fields: object) -> None:
        native = None
        if client_ms is not None:
            native = int(round(float(client_ms)))
            if clock_source == "ui":
                wall = self.clock.wall_for_ui(float(client_ms)) if wall_ms is None else wall_ms
                client = native
            elif clock_source == "system":
                wall = self.clock.wall_for_system(float(client_ms)) if wall_ms is None else wall_ms
                client = self.clock.ui_for_system(float(client_ms))
            else:
                wall = ms_now() if wall_ms is None else wall_ms
                client = native
        else:
            wall = ms_now() if wall_ms is None else wall_ms
            client = self.clock.estimate(wall)
        row = {
            "schema": SCHEMA,
            "channel": channel,
            "seq": self.sequences.get(channel, 0),
            "wall_ms": round(wall, 1),
            "client_ms": client,
            "t_ms": self.clock.relative(client),
            "clock": clock_source if client_ms is not None else "estimate",
        }
        if native is not None:
            row["native_ms"] = native
        for key, value in fields.items():
            if isinstance(value, float) and not math.isfinite(value):
                value = None
            row[key] = value
        with self.lock:
            handle = self.handles.get(channel)
            if handle is None:
                handle = (self.run / f"{channel}.jsonl").open("a", encoding="utf-8")
                self.handles[channel] = handle
            handle.write(json.dumps(row, ensure_ascii=False, separators=(",", ":")) + "\n")
            handle.flush()
            self.sequences[channel] = row["seq"] + 1
            self.counts[channel] = self.counts.get(channel, 0) + 1

    def close(self) -> None:
        with self.lock:
            for handle in self.handles.values():
                try:
                    handle.close()
                except Exception:
                    pass
            self.handles.clear()


class ClockSync(threading.Thread):
    def __init__(self, mcp: Mcp, pid: int, clock: Clock, writer: ChannelWriter,
                 interval: float = 1.0):
        super().__init__(daemon=True)
        self.mcp = mcp
        self.pid = pid
        self.clock = clock
        self.writer = writer
        self.interval = interval
        self.stop_event = threading.Event()

    def run(self) -> None:
        while not self.stop_event.is_set():
            wall = ms_now()
            system = ui = None
            try:
                text = self.mcp.text("client_lua", pid=self.pid, target="system",
                                     code="return System.GetCurrTime()")
                system = float(text.strip()) * 1000.0
            except Exception as error:
                self.writer.write("clock_sync", wall_ms=wall, clock_source="external",
                                  system_ms=None, ui_ms=None, error=str(error))
                self.stop_event.wait(self.interval)
                continue
            try:
                text = self.mcp.text("client_lua", pid=self.pid, target="ui",
                                     code="local ok,t=pcall(function() return X2Time:GetUiMsec() end) if ok and t then return t end return ''")
                ui = float(text.strip()) if text.strip() else None
            except Exception:
                ui = None
            self.clock.sample(system, ui, wall)
            self.writer.write("clock_sync", wall_ms=wall, clock_source="external",
                              system_ms=round(system, 1) if system is not None else None,
                              ui_ms=round(ui, 1) if ui is not None else None)
            self.stop_event.wait(self.interval)

    def stop(self) -> None:
        self.stop_event.set()
        self.join(timeout=5)


def lua_timer(name: str, interval_ms: int, sample: str) -> str:
    return r"""
_G.CS = _G.CS or {}
local key = %s
_G.CS[key..'_rows'] = {}
_G.CS[key..'_stop'] = false
local function tick()
  if _G.CS[key..'_stop'] then return end
  local ok, row = pcall(function()
%s
  end)
  if ok and type(row) == 'string' and row ~= '' then
    local rows = _G.CS[key..'_rows']
    if #rows < 500000 then rows[#rows + 1] = row end
  end
  Script.SetTimer(%d, tick)
end
Script.SetTimer(%d, tick)
return 'installed'
""" % (json.dumps(name), sample, interval_ms, interval_ms)


def fetch_rows(mcp: Mcp, pid: int, target: str, name: str) -> list[str]:
    table = "_G.CS[%s..'_rows']" % json.dumps(name)
    total = int(mcp.text("client_lua", pid=pid, target=target,
                         code="return #%s" % table).strip() or 0)
    rows: list[str] = []
    for start in range(1, total + 1, 400):
        end = min(total, start + 399)
        text = mcp.text("client_lua", pid=pid, target=target,
                        code="return table.concat(%s, '\\n', %d, %d)" % (table, start, end))
        if text:
            rows.extend(text.splitlines())
    return rows


def stop_timer(mcp: Mcp, pid: int, target: str, name: str) -> None:
    mcp.text("client_lua", pid=pid, target=target,
             code="_G.CS[%s..'_stop']=true return 1" % json.dumps(name))


SYSTEM_SAMPLE = r"""
local now = System.GetCurrTime()
local function q(v)
  if v == nil then return '' end
  return (tostring(v):gsub('[|\r\n]', ' '))
end
local me, target, best = nil, nil, nil
for _, x in pairs(System.GetEntities() or {}) do
  local ok, name = pcall(function() return x:GetName() end)
  if ok and name == CHAR_NAME then me = x end
  if ok and name == TARGET_NAME and me ~= nil then
    local okp, p = pcall(function() return x:GetWorldPos() end)
    local okq, q = pcall(function() return me:GetWorldPos() end)
    if okp and okq and p and q then
      local dx, dy, dz = p.x - q.x, p.y - q.y, p.z - q.z
      local d = dx * dx + dy * dy + dz * dz
      if best == nil or d < best then best = d; target = x end
    end
  end
end
if me == nil then return '' end
local function anim(x)
  if x == nil or x.actor == nil then return '', '', '' end
  local state, clip, pointer = '?', '', ''
  pcall(function() state = x.actor:GetCurrentAnimationState() end)
  pcall(function() clip = string.format('%.4f', x.actor:GetAnimationTime(0, 0)) end)
  local ok, value = pcall(function() return tostring(x.actor.__this) end)
  if ok and value and value ~= '' then pointer = value end
  if pointer == '' then
    pcall(function() pointer = tostring(x.actor) end)
  end
  return q(state), q(clip), q(pointer)
end
local function pos(x)
  if x == nil then return '' end
  local ok, p = pcall(function() return x:GetWorldPos() end)
  if ok and p then return string.format('%.4f,%.4f,%.4f', p.x or 0, p.y or 0, p.z or 0) end
  return ''
end
local ps, pt, pp = anim(me)
local vs, vt, vp = anim(target)
return table.concat({
  q(math.floor(now * 1000.0 + 0.5)), q(System.GetFrameID()),
  ps, pt, pp, vs, vt, vp, pos(me), pos(target),
}, '|')
"""


UI_CAST_SAMPLE = r"""
local now = 0
pcall(function() now = X2Time:GetUiMsec() or 0 end)
local function q(v)
  if v == nil then return '' end
  return (tostring(v):gsub('[|\r\n]', ' '))
end
local function cast(unit)
  local ok, a, b, c, d = pcall(X2Unit.UnitCastingInfo, X2Unit, unit)
  if ok then return q(a), q(b), q(c), q(d) end
  return '', '', '', ''
end
local pn, pt, pc, pd = cast('player')
local tn, tt, tc, td = cast('target')
local function get(fn, unit)
  local ok, v = pcall(fn, X2Unit, unit)
  if ok then return v end
  return nil
end
return table.concat({
  q(now), pn, pt, pc, pd, tn, tt, tc, td,
  q(get(X2Unit.UnitHealth, 'player')), q(get(X2Unit.UnitMaxHealth, 'player')),
  q(get(X2Unit.UnitMana, 'player')), q(get(X2Unit.UnitMaxMana, 'player')),
  q(get(X2Unit.UnitHealth, 'target')), q(get(X2Unit.UnitMaxHealth, 'target')),
  q(get(X2Unit.UnitIsDead, 'player')), q(get(X2Unit.UnitCombatState, 'player')),
}, '|')
"""


UI_BUFF_SAMPLE = r"""
local now = 0
pcall(function() now = X2Time:GetUiMsec() or 0 end)
local function q(v)
  if v == nil then return '' end
  return (tostring(v):gsub('[;|~\r\n]', ' '))
end
local function field(t, key)
  if type(t) ~= 'table' then return '' end
  local v = t[key]
  if v == nil then return '' end
  return q(v)
end
local function value(v)
  if type(v) == 'table' then
    return field(v, 'buff_id') .. ',' .. field(v, 'path') .. ',' ..
           field(v, 'timeLeft') .. ',' .. field(v, 'stack') .. ',' ..
           field(v, 'duration') .. ',' .. field(v, 'name')
  end
  return q(v)
end
local rows = { q(now) }
local function list(tag, unit, counter, getter)
  local ok, count = pcall(counter, X2Unit, unit)
  count = ok and tonumber(count) or 0
  rows[#rows + 1] = tag .. '~count~' .. tostring(count)
  for i = 1, math.min(count or 0, 32) do
    local ok2, a, b, c, d = pcall(getter, X2Unit, unit, i)
    if ok2 then
      local parts = { tag, tostring(i) }
      if a ~= nil then parts[#parts + 1] = value(a) end
      if b ~= nil then parts[#parts + 1] = value(b) end
      if c ~= nil then parts[#parts + 1] = value(c) end
      if d ~= nil then parts[#parts + 1] = value(d) end
      rows[#rows + 1] = table.concat(parts, '~')
    end
  end
end
list('player_buff', 'player', X2Unit.UnitBuffCount, X2Unit.UnitBuff)
list('player_debuff', 'player', X2Unit.UnitDeBuffCount, X2Unit.UnitDeBuff)
list('target_buff', 'target', X2Unit.UnitBuffCount, X2Unit.UnitBuff)
list('target_debuff', 'target', X2Unit.UnitDeBuffCount, X2Unit.UnitDeBuff)
return table.concat(rows, '|')
"""


CHAT_HOOK = r"""
_G.__combat_recorder = _G.__combat_recorder or { chat = {} }
local o = _G.__combat_recorder
local function esc(v)
  if v == nil then return '' end
  return (tostring(v):gsub('[\t\r\n|]', ' '))
end
local function stamp()
  local ok, t = pcall(function() return X2Time:GetUiMsec() end)
  if ok and t then return tostring(t) end
  return ''
end
local function add(name, ...)
  if #o.chat >= 8192 then return end
  local parts = {}
  for i = 1, select('#', ...) do parts[i] = esc((select(i, ...))) end
  o.chat[#o.chat + 1] = stamp() .. '\t' .. name .. '\t' .. table.concat(parts, '\t')
end
if type(X2Chat) == 'table' and type(X2Chat.DispatchChatMessage) == 'function' then
  if not o.method_hooked then
    local original = X2Chat.DispatchChatMessage
    X2Chat.DispatchChatMessage = function(self, ...)
      add('X2Chat.DispatchChatMessage', ...)
      return original(self, ...)
    end
    o.method_hooked = true
  end
end
for _, name in ipairs({'AddMsgToChatWin', 'AddMessageToSysMsgWindow',
                       'OnCreateChatMessage', 'AddMessageTo', 'ShowEffectMsgMessage'}) do
  if not o['global_' .. name] and type(_G[name]) == 'function' then
    local original = _G[name]
    _G[name] = function(...)
      add(name, ...)
      return original(...)
    end
    o['global_' .. name] = true
  end
end
return 'hooked'
"""


def actor_pointers(mcp: Mcp, pid: int, character: str, target: str) -> dict[str, int]:
    code = r"""
local me, target = CHAR_NAME, TARGET_NAME
local out = {}
for _, x in pairs(System.GetEntities() or {}) do
  local ok, name = pcall(function() return x:GetName() end)
  if ok and (name == me or name == target) then
    local pointer = ''
    local okp, p = pcall(function() return tostring(x.actor.__this) end)
    if okp and p and p ~= '' then pointer = p end
    if pointer == '' then pcall(function() pointer = tostring(x.actor) end) end
    out[#out + 1] = name .. '\t' .. pointer
  end
end
return table.concat(out, '\n')
""".replace("CHAR_NAME", json.dumps(character)).replace("TARGET_NAME", json.dumps(target))
    result: dict[str, int] = {}
    for line in mcp.text("client_lua", pid=pid, target="system", code=code).splitlines():
        name, _, pointer = line.partition("\t")
        match = re.search(r"0x([0-9a-fA-F]+)", pointer)
        if match:
            result[name] = int(match.group(1), 16)
    return result


def looks_like_controller(mem: Mem, ctrl: int) -> bool:
    if not ctrl:
        return False
    for offset in (0x24, 0x30):
        head = mem.u32(ctrl + offset)
        if not head:
            continue
        first = mem.u32(head)
        if not first or first == head:
            continue
        data = mem.read(first + 8, 0x44)
        if not data or len(data) < 0x44:
            continue
        anim = s32(struct.unpack_from("<I", data, 8)[0])
        start = struct.unpack_from("<f", data, 0x28)[0]
        duration = struct.unpack_from("<f", data, 0x2C)[0]
        if 0 <= anim < 1_000_000 and 0.0 <= start < 1e9 and 0.0 <= duration < 3600.0:
            return True
    return False


def resolve_controller(mem: Mem, unit: int, lua_pointer: int | None,
                       label: str) -> tuple[int, list[str]]:
    candidates: list[tuple[str, int]] = []
    if lua_pointer:
        candidates.append(("lua_actor", lua_pointer))
    if unit:
        model = mem.u32(unit + UNIT_ANIM_MODEL)
        if model:
            candidates.append(("unit_model", model))
            holder = mem.u32(model + 0x1F0)
            if holder:
                candidates.append(("model_holder", holder))
                obj = mem.u32(holder + 0x0C)
                if obj:
                    candidates.append(("model_object", obj))
        candidates.append(("unit_plus_model", unit + UNIT_ANIM_MODEL))
    tried: list[str] = []
    for name, address in candidates:
        if not address:
            continue
        tried.append(f"{name}=0x{address:08x}")
        if looks_like_controller(mem, address):
            log(label, "controller", name, hex(address))
            return address, tried
    log(label, "controller unresolved; tried", ", ".join(tried) or "none")
    return 0, tried


def read_controller_entry(mem: Mem, ctrl: int, offset: int,
                          name: str) -> dict[str, object] | None:
    head = mem.u32(ctrl + offset)
    if not head:
        return None
    first = mem.u32(head)
    if not first or first == head:
        return None
    data = mem.read(first + 8, 0x44)
    if not data or len(data) < 0x44:
        return None
    words = struct.unpack("<17I", data)
    start = struct.unpack_from("<f", data, 0x28)[0]
    last = mem.f32(ctrl + 0x70)
    return {
        "list": name,
        "id": words[0],
        "weapon": s32(words[1]),
        "anim_id": s32(words[2]),
        "data": words[8],
        "start": round(start, 6),
        "duration": round(struct.unpack_from("<f", data, 0x2C)[0], 6),
        "fire": round(struct.unpack_from("<f", data, 0x30)[0], 6),
        "last_time": round(last, 6),
        "clip_time": round(max(0.0, last - start), 6) if start > 0 else 0.0,
    }


class MemorySampler(threading.Thread):
    def __init__(self, mem: Mem, clock: Clock, writer: ChannelWriter, base: int,
                 actor: int, victim_unit: int, caster_ctrl: int, victim_ctrl: int,
                 interval: float = 0.005):
        super().__init__(daemon=True)
        self.mem = mem
        self.clock = clock
        self.writer = writer
        self.base = base
        self.actor = actor
        self.victim_unit = victim_unit
        self.caster_ctrl = caster_ctrl
        self.victim_ctrl = victim_ctrl
        self.interval = interval
        self.stop_event = threading.Event()
        self.gcd_samples = 0
        self.gcd_nonzero = 0
        self.anim_samples = 0

    def sample(self) -> None:
        wall = ms_now()
        client = self.clock.estimate(wall)
        actor = self.actor
        if not actor:
            return
        gcd = self.mem.u32s(actor + GCD_REMAINING, 2)
        cast = self.mem.u32s(actor + UNIT_CAST + 4, 4)
        stance = self.mem.u8(actor + UNIT_STANCE)
        deadline = self.mem.u32(actor + UNIT_COMBAT_DEADLINE)
        combat_in = self.mem.u8(actor + UNIT_COMBAT_IN)
        latch_a = self.mem.u8(actor + UNIT_COMBAT_LATCH_A)
        latch_b = self.mem.u8(actor + UNIT_COMBAT_LATCH_B)
        weapon = self.mem.u32(actor + UNIT_WEAPON)
        slots = self.mem.u32s(actor + UNIT_WEAPON_SLOTS, 6)
        visual = self.mem.u8(actor + UNIT_VISUAL_BYTE)
        self.writer.write(
            "gcd", wall_ms=wall, client_ms=client, clock_source="estimate",
            player_unit=actor, caster_ctrl=self.caster_ctrl, victim_ctrl=self.victim_ctrl,
            gcd_remaining=gcd[0], gcd_total=gcd[1],
            cast_normal=cast[0], cast_delay=cast[1], cast_channel=cast[2],
            cast_channel_delay=cast[3], stance_perm=stance, combat_deadline=deadline,
            combat_in=combat_in, latch_a=latch_a, latch_b=latch_b,
            weapon_active=weapon, weapon_slots=slots, visual_byte=visual)
        self.gcd_samples += 1
        if gcd[0] or gcd[1]:
            self.gcd_nonzero += 1
        for unit, ctrl, label in ((actor, self.caster_ctrl, "caster"),
                                  (self.victim_unit, self.victim_ctrl, "victim")):
            if not ctrl:
                continue
            for offset, name in ((0x24, "A"), (0x30, "B")):
                entry = read_controller_entry(self.mem, ctrl, offset, name)
                if entry is None:
                    continue
                self.writer.write("anim_controller", wall_ms=wall, client_ms=client,
                                  clock_source="estimate", unit=label, unit_ptr=unit,
                                  ctrl=ctrl, **entry)
                self.anim_samples += 1

    def run(self) -> None:
        while not self.stop_event.is_set():
            try:
                self.sample()
            except Exception as error:
                self.writer.write("gcd", clock_source="external", error=str(error))
            self.stop_event.wait(self.interval)

    def stop(self) -> None:
        self.stop_event.set()
        self.join(timeout=5)


class UnitStateSampler(threading.Thread):
    def __init__(self, mcp: Mcp, pid: int, clock: Clock, writer: ChannelWriter,
                 interval: float = 1.0):
        super().__init__(daemon=True)
        self.mcp = mcp
        self.pid = pid
        self.clock = clock
        self.writer = writer
        self.interval = interval
        self.stop_event = threading.Event()

    def run(self) -> None:
        while not self.stop_event.is_set():
            for target in ("player", "target"):
                wall = ms_now()
                client = self.clock.estimate(wall)
                try:
                    raw = self.mcp.text("client_unit_state", pid=self.pid, target=target)
                    state = json.loads(raw)
                except Exception as error:
                    self.writer.write("unit_state", wall_ms=wall, client_ms=client,
                                      clock_source="estimate", target=target,
                                      present=False, error=str(error))
                    continue
                self.writer.write("unit_state", wall_ms=wall, client_ms=client,
                                  clock_source="estimate", target=target,
                                  present=state.get("present", False),
                                  name=state.get("name"),
                                  unit_id=state.get("unit_id"),
                                  distance=state.get("distance"),
                                  health=state.get("health"),
                                  max_health=state.get("max_health"),
                                  mana=state.get("mana"),
                                  max_mana=state.get("max_mana"),
                                  buff_count=state.get("buff_count"),
                                  debuff_count=state.get("debuff_count"),
                                  buffs=state.get("buffs"),
                                  debuffs=state.get("debuffs"))
            self.stop_event.wait(self.interval)

    def stop(self) -> None:
        self.stop_event.set()
        self.join(timeout=5)


def walk_units(mem: Mem, base: int) -> list[tuple[int, int]]:
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

    walk(mem.u32(header + 4) if header else 0)
    return out


def wait_stage(mcp: Mcp, pid: int, stage: int | tuple[int, ...], timeout: float) -> str:
    wanted = {str(stage)} if isinstance(stage, int) else {str(value) for value in stage}
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            last = mcp.text("client_lua", pid=pid, target="ui",
                            code="return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()").strip()
            if last in wanted:
                return last
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError(f"stage {'/'.join(sorted(wanted))} not reached: {last}")


def wait_player(mcp: Mcp, pid: int, timeout: float) -> str:
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            state = mcp.text(
                "client_lua", pid=pid, target="ui",
                code="return X2LoginCharacter:GetCurrentStage(), X2Unit and X2Unit:UnitName('player')")
            stage, _, name = state.partition("\t")
            if stage.strip() == "0" and name.strip() not in ("", "nil"):
                return name.strip()
            last = state
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError("player did not appear: " + last)


def create_character(mcp: Mcp, pid: int, name: str, ability: int,
                     open_ui: bool = True) -> None:
    lua = lambda code: mcp.text("client_lua", pid=pid, target="ui", code=code)
    if open_ui:
        lua("X2LoginCharacter:CreateCharacter()")
    for _ in range(240):
        if lua("return characterCreate and characterCreate.state").strip() == "Step1":
            break
        time.sleep(0.25)
    time.sleep(1.0)
    lua("characterCreate.OnNextStep()")
    for _ in range(240):
        if lua("return characterCreate.state").strip() == "Step2":
            break
        time.sleep(0.25)
    time.sleep(1.0)
    lua("characterCreate.OnNextStep()")
    for _ in range(240):
        if lua("return characterCreate.state").strip() == "Step3":
            break
        time.sleep(0.25)
    lua("SelectAbility(%d) step3.content.nameWnd.characterName:SetText(%s) CreateCharacter(false)"
        % (ability, json.dumps(name)))
    wait_stage(mcp, pid, 4, 180)


def select_character(mcp: Mcp, pid: int, name: str) -> None:
    lua = lambda code: mcp.text("client_lua", pid=pid, target="ui", code=code)
    names = []
    for index in range(1, 6):
        value = lua("return X2LoginCharacter:GetLoginCharacterName(%d)" % index).strip()
        if value and value != "nil":
            names.append(value)
    if name not in names:
        raise RuntimeError(f"character {name!r} not in {names}")
    lua("X2LoginCharacter:SelectCharacter(%d)" % (names.index(name) + 1))
    lua("characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')")


def attach_and_hide(mcp: Mcp, pid: int) -> None:
    mcp.text("client_attach", pid=pid)
    mcp.text("client_window", pid=pid, visible=False)
    status = mcp.text("client_status", pid=pid)
    if "hidden=1" not in status:
        subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        raise RuntimeError("client could not be hidden; stopped pid %d: %s" % (pid, status))
    log("attached and hidden", pid)


def current_stage(mcp: Mcp, pid: int, timeout: float = 240.0) -> str:
    end = time.monotonic() + timeout
    value = ""
    while time.monotonic() < end:
        try:
            value = mcp.text("client_lua", pid=pid, target="ui",
                             code="return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()").strip()
        except Exception:
            value = ""
        if value in ("0", "3", "4"):
            return value
        time.sleep(0.5)
    raise RuntimeError("login stage did not become readable: " + value)


def ensure_world(mcp: Mcp, pid: int, character: str, ability: int,
                 create: bool, timeout: float = 240.0) -> str:
    stage = current_stage(mcp, pid, timeout)
    if stage == "3":
        mcp.text("client_lua", pid=pid, target="ui", code="X2World:EnterWorld(1,0) return 1")
        # With no characters the client goes straight to stage 5 (create); with
        # characters it goes to stage 4 (select).
        stage = wait_stage(mcp, pid, (4, 5), timeout)
    if stage == "5":
        if not create:
            raise RuntimeError("the account has no character; pass --create-character")
        create_character(mcp, pid, character, ability, open_ui=False)
        stage = "4"
    if stage == "4":
        lua = lambda code: mcp.text("client_lua", pid=pid, target="ui", code=code)
        names = []
        for index in range(1, 6):
            value = lua("return X2LoginCharacter:GetLoginCharacterName(%d)" % index).strip()
            if value and value != "nil":
                names.append(value)
        if character not in names:
            if not create:
                raise RuntimeError(f"character {character!r} not in {names}; pass --create-character")
            create_character(mcp, pid, character, ability, open_ui=True)
            names = []
            for index in range(1, 6):
                value = lua("return X2LoginCharacter:GetLoginCharacterName(%d)" % index).strip()
                if value and value != "nil":
                    names.append(value)
            if character not in names:
                raise RuntimeError(f"created character {character!r} not in {names}")
        select_character(mcp, pid, character)
    return wait_player(mcp, pid, timeout)


def server_slice(path: Path, offset: int) -> tuple[bytes, int]:
    if not path.exists():
        return b"", offset
    end = path.stat().st_size
    if end < offset:
        offset = 0
    with path.open("rb") as handle:
        handle.seek(offset)
        return handle.read(end - offset), end


def parse_lua_rows(rows: list[str]) -> list[list[str]]:
    return [line.split("|") for line in rows if line]


def self_check(writer: ChannelWriter, gcd_rows: list[dict], inputs: list[dict],
               caster_ctrl: int, victim_ctrl: int, ctrl_tried: dict[str, list[str]],
               required: list[str]) -> dict:
    counts = dict(writer.counts)
    missing = [name for name in required if counts.get(name, 0) == 0]
    changes = []
    previous = None
    for row in gcd_rows:
        current = (row.get("gcd_remaining"), row.get("gcd_total"))
        if previous is not None and current != previous:
            changes.append({"wall_ms": row.get("wall_ms"), "client_ms": row.get("client_ms"),
                            "from": list(previous), "to": list(current)})
        previous = current
    presses = []
    for event in inputs:
        before = event.get("before_wall_ms")
        for change in changes:
            if before is not None and 0 <= change["wall_ms"] - before <= 3000:
                presses.append({"key": event.get("key"), "before_wall_ms": before,
                                "change": change})
                break
    press_linked = bool(presses)
    if not press_linked and changes and inputs:
        first_press = min(event.get("before_wall_ms", 0.0) for event in inputs)
        press_linked = any(change["wall_ms"] >= first_press for change in changes)
    gcd_changed = press_linked
    result = {
        "ok": not missing and gcd_changed,
        "channels": counts,
        "required": required,
        "missing": missing,
        "gcd": {
            "changed": gcd_changed,
            "pressed_changes": presses,
            "changes": changes[:200],
            "samples": len(gcd_rows),
        },
        "animations": {
            "caster_ctrl": caster_ctrl,
            "victim_ctrl": victim_ctrl,
            "tried": ctrl_tried,
        },
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--driver", type=Path,
                        default=Path(os.environ.get("AA_DRIVER", "aaemu-clientdriver.exe")))
    parser.add_argument("--dll", type=Path,
                        default=Path(os.environ.get("AA_DRIVER_DLL", "aadriver.dll")))
    parser.add_argument("--launcher", type=Path,
                        default=Path(os.environ.get("AA_LAUNCHER", "aaemu-launcher.exe")))
    parser.add_argument("--client", type=Path,
                        default=Path(os.environ.get("AA_CLIENT", r"D:/AA1.2/bin32/archeage.exe")))
    parser.add_argument("--login", default=os.environ.get("AA_LOGIN", "127.0.0.1:1237"))
    parser.add_argument("--user", default=os.environ.get("AA_USER", "clienttest"))
    parser.add_argument("--password-file", type=Path, default=None)
    parser.add_argument("--character", default=os.environ.get("AA_CHARACTER", "CombatT1"))
    parser.add_argument("--character-ability", type=int, default=2)
    parser.add_argument("--create-character", action="store_true",
                        help="create the character through the original creation UI when missing")
    parser.add_argument("--pid", type=int, default=None,
                        help="attach to an already running client (started by the lane)")
    parser.add_argument("--spawn-npc", type=int, default=None,
                        help="GM-spawn this NPC template before the fight")
    parser.add_argument("--npc-name", default="rat_brown ",
                        help="client name of the spawned NPC (exact, including spaces)")
    parser.add_argument("--find-npc", type=int, default=None,
                        help="template id understood by /findobject (defaults to --spawn-npc)")
    parser.add_argument("--move", default=None, help="GM /move x y z before spawning")
    parser.add_argument("--fight-keys", default="1,1,2,1,3,2,1,1")
    parser.add_argument("--fight-interval", type=float, default=1.1)
    parser.add_argument("--fight-seconds", type=float, default=0.0,
                        help="record this long after the last key (0 = a little over one interval)")
    parser.add_argument("--server-log", type=Path, default=None)
    parser.add_argument("--keep-client", action="store_true")
    parser.add_argument("--no-self-check", action="store_true")
    parser.add_argument("--required", default="clock_sync,anim_state,anim_controller,gcd,castbar,unit_state,chat,inputs",
                        help="comma-separated channels that must have rows")
    args = parser.parse_args()

    if args.password_file is None:
        password = os.environ.get("AA_PASSWORD_FILE")
        if password:
            args.password_file = Path(password)

    args.run_dir.mkdir(parents=True, exist_ok=True)
    for path in (args.driver, args.dll, args.launcher, args.client):
        if not path.exists():
            raise SystemExit(f"missing {path}")
    clock = Clock()
    writer = ChannelWriter(args.run_dir, clock)
    mcp = None
    pid = args.pid
    sampler = None
    unit_sampler = None
    sync = None
    server_offset = 0
    inputs: list[dict] = []
    gcd_rows: list[dict] = []
    player_id = victim_id = 0
    caster_ctrl = victim_ctrl = 0
    ctrl_tried: dict[str, list[str]] = {}
    try:
        mcp = Mcp(args.driver, args.dll, args.launcher, args.client,
                  args.password_file, args.login, args.user, args.run_dir / "mcp")
        if pid is None:
            raise SystemExit("--pid is required: start the client with the lane's live-stack.sh client")
        attach_and_hide(mcp, pid)
        log("waiting for world")
        player_name = ensure_world(mcp, pid, args.character, args.character_ability,
                                   args.create_character)
        log("player", player_name)

        # The first clock sample also fixes the relative-time origin for every row.
        wall = ms_now()
        system = float(mcp.text("client_lua", pid=pid, target="system",
                                code="return System.GetCurrTime()").strip()) * 1000.0
        ui_text = mcp.text("client_lua", pid=pid, target="ui",
                           code="local ok,t=pcall(function() return X2Time:GetUiMsec() end) if ok and t then return t end return ''").strip()
        ui = float(ui_text) if ui_text else None
        clock.sample(system, ui, wall)
        writer.write("clock_sync", wall_ms=wall, clock_source="external",
                     system_ms=round(system, 1), ui_ms=round(ui, 1) if ui is not None else None)

        log("installing chat hook", mcp.text("client_lua", pid=pid, target="ui", code=CHAT_HOOK))
        if args.server_log:
            server_offset = args.server_log.stat().st_size if args.server_log.exists() else 0

        mem = Mem(pid)
        for name, _ in mem.modules():
            if any(bad in name.lower() for bad in ANTI_CHEAT):
                raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
        base = mem.module_base("x2game.dll")
        if not base:
            raise RuntimeError("x2game.dll not loaded")
        local = mem.u32(base + (G_CONNECTION - DUMP_BASE))
        actor = mem.u32(local + 0x10C) if local else 0
        if not actor:
            raise RuntimeError("the client is not in the world (local actor is null)")
        log("local unit", hex(actor), "id", mem.u32(actor + 8))

        if args.move:
            mcp.text("client_chat", pid=pid, message="/move " + args.move)
            time.sleep(2.0)
        if args.spawn_npc:
            template = args.find_npc or args.spawn_npc
            mcp.text("client_chat", pid=pid, message=f"/despawn npc {template} 30")
            time.sleep(0.8)
            mcp.text("client_chat", pid=pid, message=f"/spawn npc {args.spawn_npc}")
            time.sleep(2.0)
        find_template = args.find_npc or args.spawn_npc
        if find_template:
            mcp.text("client_chat", pid=pid, message=f"/findobject npc {find_template} 5")
            time.sleep(1.2)
            chat_text = mcp.text(
                "client_lua", pid=pid, target="ui",
                code="local o=assert(_G.__combat_recorder) return table.concat(o.chat, '\\n')")
            ids = re.findall(r"BcId:\s*(\d+)", chat_text)
            if not ids:
                raise RuntimeError("spawned NPC not found in chat:\n" + chat_text[-4000:])
            victim_id = int(ids[0])
            target = mcp.text("client_target", pid=pid, unit_id=format(victim_id, "X"),
                              expected_name=args.npc_name)
            log("target", target)
            state = json.loads(mcp.text("client_unit_state", pid=pid, target="target"))
            if not state.get("present"):
                raise RuntimeError("target is not present")
            distance = float(state.get("distance", 999))
            log("target", args.npc_name, "distance", distance)
            if distance > 4.0 and args.move is None:
                raise RuntimeError("target is out of melee range: %s (pass --move)" % distance)

        player = json.loads(mcp.text("client_unit_state", pid=pid, target="player"))
        player_id = int(player["unit_id"], 16) if player.get("unit_id") else mem.u32(actor + 8)

        # The caster's unit is the local actor.  The victim's unit comes from the
        # unit tree used by FUN_39360020; the key is the object id.
        victim_unit = 0
        if victim_id:
            for key, unit in walk_units(mem, base):
                if key == victim_id:
                    victim_unit = unit
                    break
            log("victim unit", hex(victim_unit) if victim_unit else "unresolved",
                "id", victim_id)

        pointers = actor_pointers(mcp, pid, player_name, args.npc_name)
        caster_ctrl, tried = resolve_controller(mem, actor, pointers.get(player_name), "caster")
        ctrl_tried["caster"] = tried
        if victim_unit:
            victim_ctrl, tried = resolve_controller(
                mem, victim_unit, pointers.get(args.npc_name), "victim")
            ctrl_tried["victim"] = tried

        # Timers: 16 ms animation state, 50 ms cast bar, 1000 ms buffs.
        system_code = (SYSTEM_SAMPLE.replace("CHAR_NAME", json.dumps(player_name))
                       .replace("TARGET_NAME", json.dumps(args.npc_name)))
        log("system timer", mcp.text("client_lua", pid=pid, target="system",
                                     code=lua_timer("anim", 16, system_code)))
        log("cast timer", mcp.text("client_lua", pid=pid, target="ui",
                                   code=lua_timer("cast", 50, UI_CAST_SAMPLE)))
        log("buff timer", mcp.text("client_lua", pid=pid, target="ui",
                                   code=lua_timer("buff", 1000, UI_BUFF_SAMPLE)))

        sync = ClockSync(mcp, pid, clock, writer)
        sync.start()
        unit_sampler = UnitStateSampler(mcp, pid, clock, writer)
        unit_sampler.start()
        sampler = MemorySampler(mem, clock, writer, base, actor, victim_unit,
                                caster_ctrl, victim_ctrl)
        sampler.start()

        keys = [key.strip() for key in args.fight_keys.split(",") if key.strip()]
        wall0 = time.monotonic()
        for index, key in enumerate(keys):
            target_time = wall0 + index * args.fight_interval
            delay = target_time - time.monotonic()
            if delay > 0:
                time.sleep(delay)
            before_wall = ms_now()
            before_client = float(mcp.text("client_lua", pid=pid, target="system",
                                           code="return System.GetCurrTime()").strip()) * 1000.0
            try:
                result = mcp.text("client_input", pid=pid, key=key, duration_ms=80)
            except Exception as error:
                result = "error: " + str(error)
            after_client = float(mcp.text("client_lua", pid=pid, target="system",
                                          code="return System.GetCurrTime()").strip()) * 1000.0
            after_wall = ms_now()
            event = {"key": key, "before_wall_ms": before_wall,
                     "before_client_ms": before_client, "after_client_ms": after_client,
                     "after_wall_ms": after_wall, "result": result}
            inputs.append(event)
            writer.write("inputs", client_ms=before_client, clock_source="system",
                         wall_ms=before_wall, key=key, duration_ms=80,
                         before_client_ms=before_client, after_client_ms=after_client,
                         after_wall_ms=after_wall, result=result)
            log("key", key, "at", round(time.monotonic() - wall0, 2), result)
        tail = args.fight_seconds or (args.fight_interval + 1.0)
        time.sleep(tail)

        stop_timer(mcp, pid, "system", "anim")
        stop_timer(mcp, pid, "ui", "cast")
        stop_timer(mcp, pid, "ui", "buff")
        time.sleep(0.3)
        if sync is not None:
            sync.stop()
        if unit_sampler is not None:
            unit_sampler.stop()
        if sampler is not None:
            sampler.stop()

        # Animation state rows.
        for parts in parse_lua_rows(fetch_rows(mcp, pid, "system", "anim")):
            if len(parts) < 10:
                continue
            writer.write("anim_state", client_ms=parts[0], clock_source="system",
                         frame=parts[1], player_state=parts[2], player_clip=parts[3],
                         player_ptr=parts[4], victim_state=parts[5], victim_clip=parts[6],
                         victim_ptr=parts[7], player_pos=parts[8], victim_pos=parts[9])
        # Cast-bar rows.
        for parts in parse_lua_rows(fetch_rows(mcp, pid, "ui", "cast")):
            if len(parts) < 17:
                continue
            writer.write("castbar", client_ms=parts[0], clock_source="ui",
                         player_name=parts[1], player_total=parts[2], player_current=parts[3],
                         player_show=parts[4], target_name=parts[5], target_total=parts[6],
                         target_current=parts[7], target_show=parts[8],
                         health=parts[9], max_health=parts[10], mana=parts[11],
                         max_mana=parts[12], target_health=parts[13],
                         target_max_health=parts[14], player_dead=parts[15],
                         player_combat=parts[16])
        # Buff rows: one row per list sample and one per visible entry.
        for parts in parse_lua_rows(fetch_rows(mcp, pid, "ui", "buff")):
            if not parts:
                continue
            client = parts[0]
            for entry in parts[1:]:
                fields = entry.split("~")
                if len(fields) < 2:
                    continue
                tag = fields[0]
                index = fields[1]
                if index == "count":
                    writer.write("buffs", client_ms=client, clock_source="ui",
                                 list=tag, count=int(fields[2]) if len(fields) > 2 and fields[2].isdigit() else 0)
                else:
                    writer.write("buffs", client_ms=client, clock_source="ui",
                                 list=tag, index=int(index) if index.isdigit() else index,
                                 values=fields[2:])
        # Chat rows from the client's own dispatch.
        chat_text = mcp.text(
            "client_lua", pid=pid, target="ui",
            code="local o=assert(_G.__combat_recorder) return table.concat(o.chat, '\\n')")
        for line in chat_text.splitlines():
            fields = line.split("\t")
            if len(fields) < 2:
                continue
            client = fields[0]
            function = fields[1]
            text = " ".join(fields[2:])
            lowered = text.lower()
            writer.write("chat", client_ms=client if client else None, clock_source="ui",
                         function=function, args=fields[2:], text=text,
                         likely_refusal=any(hint in lowered for hint in REFUSAL_HINTS))

        if args.server_log and server_offset:
            data, server_offset = server_slice(args.server_log, server_offset)
            (args.run_dir / "server_slice.log").write_bytes(data)

        gcd_rows = []
        gcd_path = args.run_dir / "gcd.jsonl"
        if gcd_path.exists():
            gcd_rows = [json.loads(line) for line in gcd_path.read_text(encoding="utf-8").splitlines() if line]

        required = [name.strip() for name in args.required.split(",") if name.strip()]
        result = self_check(writer, gcd_rows, inputs, caster_ctrl, victim_ctrl,
                            ctrl_tried, required)
        result.update({
            "pid": pid,
            "character": player_name,
            "player_id": player_id,
            "victim_id": victim_id,
            "npc": args.spawn_npc,
            "npc_name": args.npc_name,
            "inputs": inputs,
            "controller_tried": ctrl_tried,
            "start_client_ms": clock.start_client_ms,
        })
        (args.run_dir / "self_check.json").write_text(
            json.dumps(result, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        writer.write("meta", clock_source="estimate", kind="end", **{
            "character": player_name, "player_id": player_id, "victim_id": victim_id,
            "npc": args.spawn_npc, "npc_name": args.npc_name,
            "channels": writer.counts, "self_check_ok": result["ok"],
        })
        log("channels", json.dumps(writer.counts, sort_keys=True))
        log("self-check", "ok" if result["ok"] else "FAILED",
            "missing", result["missing"], "gcd_changed", result["gcd"]["changed"])
        return 0 if result["ok"] or args.no_self_check else 2
    finally:
        if sampler is not None:
            try:
                sampler.stop()
            except Exception:
                pass
        if unit_sampler is not None:
            try:
                unit_sampler.stop()
            except Exception:
                pass
        if sync is not None:
            try:
                sync.stop()
            except Exception:
                pass
        writer.close()
        if mcp is not None:
            if pid and not args.keep_client:
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
                log("stopped client", pid)
            mcp.close()


if __name__ == "__main__":
    raise SystemExit(main())
