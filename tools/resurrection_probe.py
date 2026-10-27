#!/usr/bin/env python3
"""Capture the original 1.2 client's PLAYER_RESURRECTION offer and the death window.

`FUN_391d39f0` (`OnNotifyResurrection`) sets the in-place offer flag at
`[[g_connection+0x40]+0xafc]` and raises event 92 (`PLAYER_RESURRECTION`) with
the caster's live-object id.  The death window (`x2ui/messageboximpl/
death_and_resurrection_window.alb`) handles the event with `ShowResurrectionMode
(casterId)` and names the caster through `X2Unit:GetUnitNameById`.

The probe drives one hidden original client (`a`) through the lane's
aaemu-clientdriver MCP session, observes the Lua event and the window, and reads
the native countdown fields with ReadProcessMemory (PROCESS_VM_READ only).  A
headless `aa-client` (`b`) is the other player: it kills `a` and casts the
resurrection skill (`/useskill`).  The replay mode swaps the roles so the Bevy
client is the dead one and the original is the caster.

Every path, account and lane is an argument (or an environment variable); the
script has no fork-local imports.  It refuses to read a client with an
anti-cheat module loaded and stops every client it starts, by pid, in a finally
block.

Examples:
    python tools/resurrection_probe.py --mode capture --account combat a ...
    python tools/resurrection_probe.py --mode replay ...

  python tools/resurrection_probe.py --mode capture \
      --driver D:/aaemu-target/combat-golden/live/aaemu-clientdriver.exe \
      --dll D:/aaemu-target/combat-golden/aadriver/aadriver.dll \
      --launcher D:/aaemu-live-combat-golden/bin/aaemu-launcher.exe \
      --password-file D:/aaemu-live-combat-golden/cap-pw.txt \
      --login 127.0.0.1:29237 --account combata --character CgCombata \
      --bevy D:/aaemu-target/bevy-map3/debug/aa-client.exe \
      --bevy-account combatb --bevy-character CgCombatb --bevy-login 127.0.0.1:29237 \
      --out tools/evidence/resurrection --raw data/extract/resurrection-20261027-r1
"""
from __future__ import annotations

import argparse
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
CLIENT_DEFAULT = "D:/AA1.2/bin32/archeage.exe"
ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect", "gameguard", "hackshield")

# Absolute virtual addresses in the 1.2 r208022 x2game.dll image dumped at
# base 0x38FF0000 (D:/aa-re/client12/capture/x2game.dll.a.mem).
DUMP_BASE = 0x38FF0000
G_CONNECTION = 0x39B02294


def log(*args: object) -> None:
    print("resurrection:", *args, flush=True)


class Mcp:
    """One long-lived aaemu-clientdriver MCP session (the lane's own driver)."""

    def __init__(self, driver: Path, dll: Path, client: Path, launcher: Path,
                 password_file: Path, login: str, user: str, artifact: Path):
        self.process = subprocess.Popen(
            [str(driver), "mcp", "--dll", str(dll), "--client", str(client),
             "--launcher", str(launcher), "--password-file", str(password_file),
             "--login", login, "--user", user, "--artifact-dir", str(artifact)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8",
        )
        self.sequence = 0
        self.call("initialize", {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "resurrection-probe", "version": "1"},
        })
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


def q(text: str) -> str:
    """A Lua string literal from an arbitrary Python string."""
    return json.dumps(text)


def lua(mcp: Mcp, pid: int, code: str, target: str = "ui") -> str:
    return mcp.text("client_lua", pid=pid, target=target, code=code)


def lua_json(mcp: Mcp, pid: int, code: str, target: str = "ui"):
    text = lua(mcp, pid, code, target).strip()
    if not text:
        return None
    return json.loads(text)


def screenshot(mcp: Mcp, driver: Path, pid: int, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run([str(driver), "screenshot", "--pid", str(pid), str(path)],
                            capture_output=True, timeout=120)
    if result.returncode != 0:
        raise RuntimeError("screenshot failed: " + result.stderr.decode("utf-8", "replace")[:300])


def wait_stage(mcp: Mcp, pid: int, stage: int, timeout: float) -> None:
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            last = lua(mcp, pid, "return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()").strip()
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
            state = lua(mcp, pid, "return X2LoginCharacter:GetCurrentStage(), X2Unit and X2Unit:UnitName('player')")
            stage, _, name = state.partition("\t")
            if stage == "0" and name not in ("", "nil"):
                return
            last = state
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError("player did not appear: " + last)


def launch_original(mcp: Mcp, user: str, password_file: Path) -> int:
    launch = mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                      user=user, password_file=str(password_file))
    log(launch.replace("\n", " | "))
    match = re.search(r"pid (\d+)", launch)
    if not match:
        raise RuntimeError("client_launch did not return a pid")
    pid = int(match.group(1))
    wait_stage(mcp, pid, 3, 240)
    lua(mcp, pid, "X2World:EnterWorld(1,0)")
    wait_stage(mcp, pid, 4, 240)
    lua(mcp, pid, "local b=characterSelectWindow.bottomPanel.startButton b:OnClick('LeftButton')")
    wait_player(mcp, pid, 240)
    time.sleep(3.0)
    return pid


RECORDER = r"""
probe = { resurrection = {}, death = nil, resurrected = nil, resurrected_at = nil }
local function pack(...)
  local t = { n = select('#', ...) }
  for i = 1, t.n do t[i] = select(i, ...) end
  return t
end
UIParent:SetEventHandler('UNIT_DEAD', function(...)
  probe.death = pack(...)
  probe.death_name = X2Unit:GetUnitNameById(select(1, ...))
end)
UIParent:SetEventHandler('PLAYER_RESURRECTION', function(...)
  local e = pack(...)
  e.name = X2Unit:GetUnitNameById(e[1])
  local flag, temple, wait = X2Player:GetResurrectionCountdown()
  e.countdown = { flag, temple, wait }
  table.insert(probe.resurrection, e)
end)
UIParent:SetEventHandler('PLAYER_RESURRECTED', function(...)
  probe.resurrected = pack(...)
end)
return 1
"""

STATE = r"""
local flag, temple, wait = X2Player:GetResurrectionCountdown()
local w = ADDON and ADDON.GetContent and ADDON:GetContent(UIC_DEATH_AND_RESURRECTION_WND) or nil
local window = nil
if w ~= nil then
  local function widget(name)
    local x = w[name]
    if x == nil then return nil end
    local ok, text = pcall(function() return x:GetText() end)
    local e = { visible = x:IsVisible(), enabled = x:IsEnabled() }
    if ok and text ~= nil then e.text = text end
    local ox, oy = x:GetEffectiveOffset()
    local ew, eh = x:GetEffectiveExtent()
    e.x, e.y, e.w, e.h = ox, oy, ew, eh
    return e
  end
  window = { visible = w:IsVisible(), textBox = widget('textBox'),
             penalty = widget('penalty_textbox'), resurrect = widget('btnResurrection'),
             graveyard = widget('btnGraveyard') }
end
return __probe_json({
  dead = X2Unit:UnitIsDead('player'),
  hp = X2Unit:UnitHealth('player'),
  flag = flag, temple = temple, wait = wait,
  death = probe.death, death_name = probe.death_name,
  resurrection = probe.resurrection, resurrected = probe.resurrected,
  window = window,
})
"""

PROBE_JSON = r"""
local function encode(v, depth)
  if depth > 8 then return 'null' end
  local t = type(v)
  if t == 'nil' then return 'null'
  elseif t == 'boolean' then return tostring(v)
  elseif t == 'number' then
    if v ~= v or v == math.huge or v == -math.huge then return 'null' end
    return string.format('%.9g', v)
  elseif t == 'string' then
    return '"' .. string.gsub(v, '[%z\1-\31\\"]', function(c)
      if c == '"' then return '\\"'
      elseif c == '\\' then return '\\\\'
      else return string.format('\\u%04x', string.byte(c)) end
    end) .. '"'
  elseif t == 'table' then
    local out = {}
    for k, x in pairs(v) do
      if type(x) ~= 'function' and type(x) ~= 'userdata' then
        out[#out + 1] = encode(tostring(k), depth + 1) .. ':' .. encode(x, depth + 1)
      end
    end
    table.sort(out)
    return '{' .. table.concat(out, ',') .. '}'
  end
  return 'null'
end
"""


def install_recorder(mcp: Mcp, pid: int) -> None:
    code = PROBE_JSON + "\nfunction __probe_json(v) return encode(v, 0) end\n" + RECORDER
    lua(mcp, pid, code)


def player_id(mcp: Mcp, pid: int) -> int:
    value = lua(mcp, pid, "return X2Unit:GetUnitId('player')").strip()
    if not value or value.lower() == "nil":
        raise RuntimeError("the original client did not report its unit id")
    return int(value, 16)


def state(mcp: Mcp, pid: int) -> dict:
    code = PROBE_JSON + "\nfunction __probe_json(v) return encode(v, 0) end\n" + STATE
    result = lua_json(mcp, pid, code)
    return result or {}


def native_state(mem: Mem, base: int) -> dict:
    g = mem.u32(base + (G_CONNECTION - DUMP_BASE))
    unit = mem.u32(g + 0x40) if g else 0
    unit_10c = mem.u32(g + 0x10C) if g else 0
    out = {"g_connection": g, "unit_0x40": unit, "unit_0x10c": unit_10c}
    for label, address in (("0x40", unit), ("0x10c", unit_10c)):
        if not address:
            continue
        out[label] = {
            "af4": mem.u32(address + 0xAF4),
            "af8": mem.u32(address + 0xAF8),
            "afc": mem.u8(address + 0xAFC),
        }
    return out


def window_after_offer(mcp: Mcp, pid: int) -> dict:
    code = PROBE_JSON + "\nfunction __probe_json(v) return encode(v, 0) end\n" + STATE
    return lua_json(mcp, pid, code) or {}


def click_window_resurrect(mcp: Mcp, pid: int, st: dict) -> bool:
    window = st.get("window") or {}
    button = window.get("resurrect") or {}
    if not button.get("visible"):
        return False
    x = int(button.get("x", 0) + button.get("w", 0) / 2)
    y = int(button.get("y", 0) + button.get("h", 0) / 2)
    if x <= 0 or y <= 0:
        return False
    mcp.text("client_pointer", pid=pid, action="click", x=x, y=y)
    return True


def bevy_script(mode: str, args: argparse.Namespace, raw: Path, target_id: int) -> str:
    if mode == "capture":
        return (
            f"wait {args.start_delay}\n"
            f"waitfor 30 unit id:{target_id}\n"
            f"target id:{target_id}\n"
            f"walk to id:{target_id}\n"
            "gm kill\n"
            f"wait {args.kill_wait}\n"
            f"target id:{target_id}\n"
            f"gm useskill {args.skill}\n"
            f"wait {args.caster_hold}\n"
            "quit\n"
        )
    if mode == "replay":
        return (
            f"wait 5\n"
            f"waitfor 30 unit id:{target_id}\n"
            f"target id:{target_id}\n"
            f"walk to id:{target_id}\n"
            "wait 1\n"
            "lua casterId = X2Unit:GetUnitId('target') or X2Unit:GetTargetUnitId(); resurrectEvent = nil; resurrectedSeen = false; "
            "UIParent:SetEventHandler('PLAYER_RESURRECTION', function(id) "
            "local f,t,w = X2Player:GetResurrectionCountdown(); "
            "resurrectEvent = { id, X2Unit:GetUnitNameById(id), f, t, w } end); "
            "UIParent:SetEventHandler('PLAYER_RESURRECTED', function() resurrectedSeen = true end); "
            "assert(casterId ~= nil, 'the caster is not the target')\n"
            f"await DeathDetails {args.death_timeout}\n"
            f"await ResurrectionOffered {args.offer_timeout}\n"
            "wait 1\n"
            f"lua assert(resurrectEvent ~= nil, 'no PLAYER_RESURRECTION'); "
            f"assert(resurrectEvent[1] == casterId, tostring(resurrectEvent[1]) .. ' ~= ' .. tostring(casterId)); "
            f"assert(resurrectEvent[2] == {args.character!r}, tostring(resurrectEvent[2])); "
            "local f,t,w = X2Player:GetResurrectionCountdown(); "
            "assert(f == true and t > 0); "
            "local win = ADDON:GetContent(UIC_DEATH_AND_RESURRECTION_WND); "
            "assert(win ~= nil and win:IsVisible(), 'the death window is not visible'); "
            "assert(win.btnResurrection:IsVisible(), 'the Accept button is not visible'); "
            "assert(win.btnResurrection:GetText() == 'Accept ', tostring(win.btnResurrection:GetText()))\n"
            "waitfor 30 lua select(3, X2Player:GetResurrectionCountdown()) == 0\n"
            "uiclickwidget ADDON:GetContent(UIC_DEATH_AND_RESURRECTION_WND).btnResurrection\n"
            "await Resurrected 20\n"
            "wait 1\n"
            "lua assert(resurrectedSeen, 'no PLAYER_RESURRECTED'); "
            "assert(X2Unit:UnitHealth('player') > 0, 'still dead')\n"
            "quit\n"
        )
    raise ValueError(mode)


def start_bevy(args: argparse.Namespace, script: str, log_path: Path, offscreen: bool = False) -> subprocess.Popen:
    log_path.parent.mkdir(parents=True, exist_ok=True)
    handle = open(log_path, "w", encoding="utf-8")
    command = [
        str(Path(args.bevy)), "--headless", "--mute",
        "--account", args.bevy_account, "--login", args.bevy_login,
        "--password-file", str(args.bevy_password_file or args.password_file),
        "--character", args.bevy_character, "--script", str(log_path.with_suffix(".txt")),
        "--script-enter-timeout", "240", "--script-timeout", "600",
    ]
    if offscreen:
        command += ["--offscreen-shot", str(log_path.with_name("bevy-initial.png")),
                    "--shot-after", str(args.bevy_shot_after)]
    log_path.with_suffix(".txt").write_text(script, encoding="utf-8")
    log("bevy:", " ".join(command))
    return subprocess.Popen(command, stdout=handle, stderr=subprocess.STDOUT, text=True)


def run_capture(args: argparse.Namespace, mcp: Mcp, driver: Path, raw: Path, out: Path) -> int:
    pid = args.pid
    if pid is None:
        pid = launch_original(mcp, args.account, args.password_file)
    else:
        wait_player(mcp, pid, 60)
    mem = Mem(pid)
    for name, _ in mem.modules():
        lower = name.lower()
        if any(bad in lower for bad in ANTI_CHEAT):
            raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
    base = mem.module_base("x2game.dll")
    if not base:
        raise RuntimeError("x2game.dll not loaded")
    install_recorder(mcp, pid)
    dead_id = player_id(mcp, pid)
    log(f"original dead character obj {dead_id:#x}")
    samples: list[dict] = []
    marks: dict[str, dict] = {}

    script = bevy_script("capture", args, raw, dead_id)
    bevy = start_bevy(args, script, raw / "bevy-caster.log")
    last = None
    owned = args.pid is None
    try:
        start = time.monotonic()
        death_at = offer_at = None
        clicked = False
        while time.monotonic() - start < args.timeout:
            now = time.monotonic()
            try:
                st = state(mcp, pid)
            except Exception as error:
                log("lua state failed:", error)
                break
            native = native_state(mem, base)
            row = {"t": round(now - start, 3), **st, "native": native}
            samples.append(row)
            if st.get("dead") and death_at is None:
                death_at = now
                path = raw / "capture-death.png"
                screenshot(mcp, driver, pid, path)
                marks["death"] = {"at": row["t"], "state": st, "native": native}
                log("death window", st.get("death"), "countdown", st.get("flag"), st.get("temple"), st.get("wait"))
            offers_raw = st.get("resurrection") or {}
            offers = list(offers_raw.values()) if isinstance(offers_raw, dict) else list(offers_raw)
            if offers and offer_at is None:
                offer_at = now
                screenshot(mcp, driver, pid, raw / "capture-offer.png")
                marks["offer"] = {"at": row["t"], "state": st, "native": native}
                log("offer", offers[-1])
            if offer_at is not None and not clicked and now - offer_at >= args.offer_hold:
                window = (st.get("window") or {}).get("resurrect") or {}
                if window.get("enabled"):
                    screenshot(mcp, driver, pid, raw / "capture-offer-settled.png")
                    marks["offer_settled"] = {"at": row["t"], "state": st, "native": native}
                    clicked = click_window_resurrect(mcp, pid, st)
                    if clicked:
                        log("clicked the in-place resurrection button")
                    marks["clicked"] = {"at": round(time.monotonic() - start, 3), "clicked": clicked,
                                        "button": window}
            if st.get("resurrected") is not None and offer_at is not None:
                screenshot(mcp, driver, pid, raw / "capture-resurrected.png")
                marks["resurrected"] = {"at": row["t"], "state": st, "native": native}
                log("PLAYER_RESURRECTED", st.get("resurrected"))
                break
            if death_at is not None and offer_at is None and now - death_at > args.offer_timeout:
                log("no resurrection offer before the timeout")
                break
            last = row
            time.sleep(args.interval)
        return 0 if "offer" in marks else 1
    finally:
        (raw / "capture-samples.json").write_text(json.dumps(samples, indent=1), encoding="utf-8")
        (raw / "capture-marks.json").write_text(json.dumps(marks, indent=1), encoding="utf-8")
        if bevy.poll() is None:
            bevy.terminate()
            try:
                bevy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                subprocess.run(["taskkill", "/PID", str(bevy.pid), "/F"], capture_output=True)
        if owned:
            try:
                mcp.tool("client_stop", pid=pid)
            except Exception:
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        if last is not None:
            log("last state", json.dumps(last, sort_keys=True)[:800])


def read_log(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return ""


def wait_log(path: Path, needle: str, timeout: float, process: subprocess.Popen | None = None) -> str:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        text = read_log(path)
        if needle in text:
            return text
        if process is not None and process.poll() is not None:
            tail = "\n".join(text.splitlines()[-12:])
            raise RuntimeError(f"the client exited before {needle!r}:\n{tail}")
        time.sleep(0.5)
    raise RuntimeError(f"the client log did not contain {needle!r}")


def run_replay(args: argparse.Namespace, mcp: Mcp, raw: Path, out: Path) -> int:
    pid = args.pid
    if pid is None:
        pid = launch_original(mcp, args.account, args.password_file)
    else:
        wait_player(mcp, pid, 60)
    mem = Mem(pid)
    for name, _ in mem.modules():
        lower = name.lower()
        if any(bad in lower for bad in ANTI_CHEAT):
            raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
    if not mem.module_base("x2game.dll"):
        raise RuntimeError("x2game.dll not loaded")
    owned = args.pid is None
    caster_id = player_id(mcp, pid)
    log(f"original caster character obj {caster_id:#x}")
    script = bevy_script("replay", args, raw, caster_id)
    log_path = raw / "bevy-dead.log"
    bevy = start_bevy(args, script, log_path, offscreen=True)
    code = 1
    try:
        text = wait_log(log_path, "> await DeathDetails", args.timeout, bevy)
        spawned = re.search(r"spawned (\S+) \(obj (\d+)\)", text)
        if not spawned:
            raise RuntimeError("the Bevy client did not log its spawned character")
        obj_id = int(spawned.group(2))
        log(f"bevy dead character {spawned.group(1)} obj {obj_id:#x}")
        time.sleep(1.0)
        for attempt in range(20):
            try:
                mcp.text("client_target", pid=pid, unit_id=f"{obj_id:x}", expected_name=args.bevy_character)
                break
            except Exception as error:
                if attempt == 19:
                    raise
                time.sleep(0.5)
        mcp.text("client_chat", pid=pid, message="/kill")
        time.sleep(args.kill_wait)
        for attempt in range(20):
            try:
                mcp.text("client_target", pid=pid, unit_id=f"{obj_id:x}", expected_name=args.bevy_character)
                break
            except Exception as error:
                if attempt == 19:
                    raise
                time.sleep(0.5)
        mcp.text("client_chat", pid=pid, message=f"/useskill {args.skill}")
        try:
            code = bevy.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            code = 124
        text = read_log(log_path)
        (raw / "replay-commands.txt").write_text(
            f"original {args.account}/{args.character} (pid {pid}) targeted Bevy {args.bevy_character} "
            f"obj {obj_id:#x}, then: /kill, /useskill {args.skill}\n", encoding="utf-8")
        (raw / "replay-bevy.log").write_text(text, encoding="utf-8")
        if "SCRIPT RESULT: PASS" not in text:
            log("the Bevy script did not PASS; see", raw / "replay-bevy.log")
        return 0 if code == 0 and "SCRIPT RESULT: PASS" in text else 1
    finally:
        if bevy.poll() is None:
            bevy.terminate()
            try:
                bevy.wait(timeout=10)
            except subprocess.TimeoutExpired:
                subprocess.run(["taskkill", "/PID", str(bevy.pid), "/F"], capture_output=True)
        if owned:
            try:
                mcp.tool("client_stop", pid=pid)
            except Exception:
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["capture", "replay"], default="capture")
    parser.add_argument("--client", default=os.environ.get("RES_CLIENT", CLIENT_DEFAULT))
    parser.add_argument("--driver", required=True)
    parser.add_argument("--dll", required=True)
    parser.add_argument("--launcher", required=True)
    parser.add_argument("--password-file", required=True, type=Path)
    parser.add_argument("--login", required=True)
    parser.add_argument("--account", required=True)
    parser.add_argument("--character", required=True)
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world original client")
    parser.add_argument("--skill", type=int, default=10546)
    parser.add_argument("--start-delay", type=float, default=15.0)
    parser.add_argument("--kill-wait", type=float, default=3.0)
    parser.add_argument("--offer-hold", type=float, default=2.0)
    parser.add_argument("--offer-timeout", type=float, default=45.0)
    parser.add_argument("--death-timeout", type=float, default=60.0)
    parser.add_argument("--caster-hold", type=float, default=40.0)
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--interval", type=float, default=0.2)
    parser.add_argument("--bevy", required=True)
    parser.add_argument("--bevy-account", required=True)
    parser.add_argument("--bevy-character", required=True)
    parser.add_argument("--bevy-login", required=True)
    parser.add_argument("--bevy-password-file", type=Path)
    parser.add_argument("--bevy-shot-after", type=float, default=24.0)
    parser.add_argument("--out", type=Path, default=ROOT / "tools" / "evidence" / "resurrection")
    parser.add_argument("--raw", type=Path, default=ROOT / "data" / "extract" /
                        f"resurrection-{time.strftime('%Y%m%d')}-r1")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    args.raw.mkdir(parents=True, exist_ok=True)
    mcp = Mcp(Path(args.driver), Path(args.dll), Path(args.client), Path(args.launcher),
              args.password_file, args.login, args.account, args.raw / "mcp")
    pid = args.pid
    try:
        if args.mode == "capture":
            return run_capture(args, mcp, Path(args.driver), args.raw, args.out)
        if args.mode == "replay":
            return run_replay(args, mcp, args.raw, args.out)
        raise SystemExit("unknown mode")
    finally:
        if pid is not None:
            try:
                mcp.tool("client_stop", pid=pid)
            except Exception:
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        mcp.close()


if __name__ == "__main__":
    raise SystemExit(main())
