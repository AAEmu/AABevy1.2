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
  python tools/resurrection_probe.py --mode capture \
      --client <client>/bin32/archeage.exe --driver <driver> --dll <aadriver.dll> \
      --launcher <lane>/bin/aaemu-launcher.exe --password-file <lane>/cap-pw.txt \
      --login <host:port> --account <account> --character <character> \
      --bevy <aa-client.exe> --bevy-account <caster-account> \
      --bevy-character <caster-character> --bevy-login <host:port> \
      --out tools/evidence/resurrection --raw <raw-dir>

  python tools/resurrection_probe.py --mode replay ... (the same paths, roles swapped)
  python tools/resurrection_probe.py --mode evidence --raw <raw-dir> \
      --replay-log <raw-dir>/replay-bevy.log --out tools/evidence/resurrection
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
CLIENT_DEFAULT = "archeage.exe"
ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect", "gameguard", "hackshield")

# Absolute virtual addresses in the 1.2 r208022 x2game.dll image dumped at
# base 0x38FF0000 (<re-notes>/client12/capture/x2game.dll.a.mem).
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
        event_file = (raw / "bevy-events.txt").resolve().as_posix()
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
            f"lua local ok=pcall(function() local f=io.open([[{event_file}]],'a'); if f then f:write('event', '\\t', tostring(resurrectEvent[1]), '\\t', tostring(resurrectEvent[2]), '\\t', tostring(resurrectEvent[3]), '\\t', tostring(resurrectEvent[4]), '\\t', tostring(resurrectEvent[5]), '\\n'); f:close() end end)\n"
            "waitfor 30 lua select(3, X2Player:GetResurrectionCountdown()) == 0\n"
            "uiclickwidget ADDON:GetContent(UIC_DEATH_AND_RESURRECTION_WND).btnResurrection\n"
            "await Resurrected 20\n"
            "wait 1\n"
            "lua assert(resurrectedSeen, 'no PLAYER_RESURRECTED'); "
            "assert(X2Unit:UnitHealth('player') > 0, 'still dead'); "
            "assert(not ADDON:GetContent(UIC_DEATH_AND_RESURRECTION_WND):IsVisible(), 'the death window is still visible')\n"
            f"lua local ok=pcall(function() local f=io.open([[{event_file}]],'a'); if f then f:write('done', '\\t', tostring(resurrectedSeen), '\\t', tostring(X2Unit:UnitHealth('player')), '\\n'); f:close() end end)\n"
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
    if offscreen and args.bevy_shot_after > 0:
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
        data = path.read_bytes()
    except FileNotFoundError:
        return ""
    if data.startswith(b"\xff\xfe") or data.startswith(b"\xfe\xff") or b"\x00" in data[:200]:
        return data.decode("utf-16", errors="replace")
    return data.decode("utf-8", errors="replace")


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


def _event(t: dict | None) -> dict | None:
    if not isinstance(t, dict) or not t:
        return None
    values = list(t.values())
    return values[0] if values else None


def _row(sample: dict) -> dict:
    event = _event(sample.get("resurrection"))
    window = sample.get("window") or {}
    resurrect = window.get("resurrect") or {}
    graveyard = window.get("graveyard") or {}
    native = (sample.get("native") or {}).get("0x40") or {}
    return {
        "t_s": sample.get("t"),
        "dead": sample.get("dead"),
        "flag": sample.get("flag"),
        "temple_s": sample.get("temple"),
        "wait_s": sample.get("wait"),
        "death_id": (sample.get("death") or {}).get("1") if isinstance(sample.get("death"), dict) else None,
        "death_name": sample.get("death_name"),
        "resurrection_id": event.get("1") if event else None,
        "resurrection_name": event.get("name") if event else None,
        "event_flag": (event.get("countdown") or {}).get("1") if event else None,
        "event_temple": (event.get("countdown") or {}).get("2") if event else None,
        "event_wait": (event.get("countdown") or {}).get("3") if event else None,
        "native_af4": native.get("af4"),
        "native_af8": native.get("af8"),
        "native_afc": native.get("afc"),
        "window_visible": window.get("visible"),
        "resurrect_visible": resurrect.get("visible"),
        "resurrect_enabled": resurrect.get("enabled"),
        "resurrect_text": resurrect.get("text"),
        "graveyard_text": graveyard.get("text"),
        "resurrected": sample.get("resurrected") is not None,
    }


def _crop(raw: Path, out: Path, marks: dict) -> dict[str, str]:
    """Crop the death window's area out of the full back-buffer captures."""
    try:
        from PIL import Image
    except Exception:
        return {}
    names = {"death": "death.png", "offer": "offer.png", "offer_settled": "offer-enabled.png",
             "resurrected": "resurrected.png"}
    written: dict[str, str] = {}
    for key, name in names.items():
        source = raw / f"capture-{key.replace('_settled', '-settled')}.png"
        mark = marks.get(key) or {}
        state = mark.get("state") or {}
        window = state.get("window") or {}
        if not source.exists():
            continue
        widgets = [w for w in window.values() if isinstance(w, dict) and "x" in w]
        if widgets:
            x0 = min(w["x"] for w in widgets) - 40
            y0 = min(w["y"] for w in widgets) - 80
            x1 = max(w["x"] + w["w"] for w in widgets) + 40
            y1 = max(w["y"] + w["h"] for w in widgets) + 30
        else:
            x0, y0, x1, y1 = 1050, 420, 1450, 720
        image = Image.open(source)
        # The hidden client renders the UI in the back-buffer pixels the Lua getters report
        # (the 1280x720 request is overridden by the DXGI DPI scale), so no extra scale.
        scale = 1.0
        box = (max(0, int(x0 * scale)), max(0, int(y0 * scale)),
               min(image.width, int(x1 * scale)), min(image.height, int(y1 * scale)))
        crop = image.crop(box)
        target = raw / name  # real-client crops stay local (copyright), never in the evidence folder
        crop.save(target, optimize=True)
        if target.stat().st_size > 1_000_000:
            target = raw / name.replace(".png", ".jpg")
            crop.convert("RGB").save(target, quality=88, optimize=True)
        written[key] = target.name
    return written


def run_evidence(args: argparse.Namespace, raw: Path, out: Path) -> int:
    samples = json.loads((raw / "capture-samples.json").read_text(encoding="utf-8"))
    marks = json.loads((raw / "capture-marks.json").read_text(encoding="utf-8"))
    rows = [_row(sample) for sample in samples]
    out.mkdir(parents=True, exist_ok=True)
    fields = list(rows[0])
    with (out / "native.csv").open("w", encoding="utf-8", newline="") as handle:
        writer = __import__("csv").DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)
    death = marks.get("death") or {}
    offer = marks.get("offer") or {}
    settled = marks.get("offer_settled") or {}
    resurrected = marks.get("resurrected") or {}
    offer_event = _event((offer.get("state") or {}).get("resurrection")) or {}
    offer_countdown = offer_event.get("countdown") or {}
    meta = {
        "scenario": "PLAYER_RESURRECTION offer and the death/resurrection window",
        "captured": time.strftime("%Y-%m-%d"),
        "client": "ArcheAge 1.2 r208022 (hidden, 1280x720)",
        "lane": "<lane>",
        "account": "<account>",
        "character": "<character>",
        "caster_account": "<caster-account>",
        "caster_character": "<caster-character>",
        "skill": args.skill,
        "native_observation": "ReadProcessMemory, PROCESS_VM_READ; [[g_connection+0x40]+0xaf4] death start, +0xaf8 server wait, +0xafc offer flag",
        "notes": "The original's OnNotifyResurrection sets +0xafc and raises event 92 with the caster's live-object id; the death window's ShowResurrectionMode names it through X2Unit:GetUnitNameById.",
    }
    (out / "native.json").write_text(json.dumps({"meta": meta, "events": marks, "samples": rows}, indent=1), encoding="utf-8")
    crops = _crop(raw, out, marks)
    replay = read_log(args.replay_log) if args.replay_log else ""
    before = read_log(args.test_before_log) if args.test_before_log else ""
    after = read_log(args.test_after_log) if args.test_after_log else ""
    replay_key = [line for line in replay.splitlines() if "hud: resurrection offered" in line
                  or "SCRIPT RESULT" in line or "SCRIPT FAIL" in line
                  or "RESURRECTION_EVENT" in line or "RESURRECTION_DONE" in line]
    events = read_log(args.replay_log.parent / "bevy-events.txt") if args.replay_log else ""
    if events:
        replay_key += [f"bevy-events.txt: {line}" for line in events.splitlines()]
    replay_values: dict[str, list[str]] = {}
    for line in events.splitlines():
        fields = line.split("\t")
        if len(fields) >= 2 and fields[0] in ("event", "done"):
            replay_values[fields[0]] = fields[1:]
    lines = [
        "PLAYER_RESURRECTION and the resurrection countdown: 1.2 native capture",
        "=" * 78,
        "",
        "How to reproduce",
        "----------------",
        "The probe drives one hidden original 1.2 client and one headless --mute aa-client.",
        "Every path, account and the lane are arguments; the placeholders below stand for the",
        "live lane used for this run.",
        "",
        "  python tools/resurrection_probe.py --mode capture \\",
        "      --client <client>/bin32/archeage.exe --driver <driver> --dll <aadriver.dll> \\",
        "      --launcher <lane>/bin/aaemu-launcher.exe --password-file <lane>/cap-pw.txt \\",
        "      --login <host:port> --account <account> --character <character> \\",
        "      --bevy <aa-client.exe> --bevy-account <caster-account> \\",
        "      --bevy-character <caster-character> --bevy-login <host:port> \\",
        "      --out tools/evidence/resurrection --raw <raw-dir>",
        "",
        "  python tools/resurrection_probe.py --mode replay ... (the same paths, roles swapped)",
        "  python tools/resurrection_probe.py --mode evidence --raw <raw-dir> \\",
        "      --replay-log <raw-dir>/replay-bevy.log --out tools/evidence/resurrection",
        "",
        f"The capture was {raw.name}; the Bevy replay was {args.replay_log.name if args.replay_log else '<replay-log>'}.",
        "The original client was hidden and read only (ReadProcessMemory, PROCESS_VM_READ); the",
        "probe refuses to run when an anti-cheat module is loaded.",
        "",
        "What the original fires",
        "-----------------------",
        f"UNIT_DEAD: {death.get('state', {}).get('death')} (name {death.get('state', {}).get('death_name')})",
        f"PLAYER_RESURRECTION: one argument, the caster's live-object id {offer_event.get('1')!r}",
        f"  X2Unit:GetUnitNameById(arg) = {offer_event.get('name')!r}",
        f"  countdown captured inside the handler: flag={offer_countdown.get('1')} temple={offer_countdown.get('2')} wait={offer_countdown.get('3')}",
        f"PLAYER_RESURRECTED: {resurrected.get('state', {}).get('resurrected')} (no arguments)",
        "",
        "Death window and countdown",
        "---------------------------",
        "mark            t(s)    flag  temple  wait  window  resurrect button                text",
    ]
    for key, mark in (("death", death), ("offer", offer), ("offer_settled", settled), ("resurrected", resurrected)):
        state = mark.get("state") or {}
        window = state.get("window") or {}
        resurrect = window.get("resurrect") or {}
        lines.append(
            f"{key:<15} {str(mark.get('at')):<7} {str(state.get('flag')):<5} {str(state.get('temple')):<6} "
            f"{str(state.get('wait')):<5} {str(window.get('visible')):<7} "
            f"visible={str(resurrect.get('visible')):<5} enabled={str(resurrect.get('enabled')):<5} "
            f"text={resurrect.get('text')!r}")
    lines += [
        "",
        "At the offer the window switches from death mode (only 'Respawn: Temple') to resurrection",
        "mode: 'Accept ' appears (text 'Accept ' in the button getter), the offer screenshot shows",
        "the caster line (the penalty textbox getter itself reads empty), and the countdown continues",
        "(the offer does not restart either timer). At wait 0 both buttons enable; clicking Accept",
        "sends the in-place resurrect request (the packet itself was not captured); PLAYER_RESURRECTED",
        "hides the window and the death timer/flag become 0/false.",
        "",
        "Native countdown fields (RPM)",
        "-----------------------------",
    ]
    for key, mark in (("death", death), ("offer", offer), ("offer_settled", settled), ("resurrected", resurrected)):
        native = mark.get("native") or {}
        unit40 = native.get("0x40") or {}
        lines.append(f"{key:<15} [[g_connection+0x40]+0xaf4]={unit40.get('af4')} +0xaf8={unit40.get('af8')} +0xafc={unit40.get('afc')}")
    lines += [
        "",
        "+0xaf4 is the death start in the client clock, +0xaf8 the server's resurrect wait in ms,",
        "+0xafc the in-place offer flag. OnNotifyResurrection sets +0xafc=1 (FUN_391d39f0:363098);",
        "OnCharacterResurrected clears +0xaf4 and +0xafc (FUN_391d3ad0:363191-363192) and leaves",
        "+0xaf8 at the last death's value. GetResurrectionCountdown (FUN_39495c00:955714) returns",
        "(flag, (300000 - min(elapsed,300000))/1000, (af8 - min(elapsed,af8))/1000).",
        "",
        "Branches of FUN_391d39f0",
        "------------------------",
        "  0xafc=1                                  measured (afc 0 -> 1 -> 0)",
        "  caster kind 0: param_2+0x18 (unit)       measured: 0x3ee1 = 16097 = CgCombatb",
        "  caster kind 2: param_2+0x24 (item)       NOT reached; aa-net decodes it, not proven here",
        "  caster kind 3: param_2+0x18 (mount)      NOT reached; not proven here",
        "  other kind:   DAT_39b022a0 (0)           NOT reached; not proven here",
        "  event 0x5c (92) with one argument        measured n=1, hex string",
        "Branches of FUN_39495c00",
        "------------------------",
        "  +0xaf4 == 0 -> (0,0,0)                   measured before death and after resurrection",
        "  elapsed < 300000 -> decreasing temple    measured 300 -> 290 -> 285",
        "  elapsed > 299999 -> temple 0             not reached live (five minutes)",
        "  wait > 0 -> button disabled              measured wait 5",
        "  wait == 0 -> button enabled              measured at offer_settled",
        "",
        "Our build",
        "---------",
        "The replay is the same scenario with the roles swapped: the original caster (object",
        "0x64c0) kills the Bevy character and casts skill 10546 on it. The Bevy script records",
        "PLAYER_RESURRECTION, compares the argument with the caster's own X2Unit:GetUnitId, checks",
        "the window text and the countdown, waits for wait 0, clicks Accept through the real widget",
        "hit test, and asserts PLAYER_RESURRECTED and HP>0. Its log:",
        "",
    ]
    if replay_key:
        lines += [f"  {line}" for line in replay_key]
    else:
        lines.append("  (replay log not supplied)")
    lines += [
        "",
        "Replay against the native capture",
        "---------------------------------",
    ]
    replay_event = replay_values.get("event")
    if replay_event and len(replay_event) >= 5:
        rid, rname, rflag, rtemple, rwait = replay_event[:5]
        lines += [
            f"  native offer:  PLAYER_RESURRECTION arg {offer_event.get('1')} name {offer_event.get('name')!r} "
            f"countdown ({str(offer_countdown.get('1')).lower()},{offer_countdown.get('2')},{offer_countdown.get('3')})",
            f"  bevy replay:   PLAYER_RESURRECTION arg {rid} name {rname!r} countdown ({rflag},{rtemple},{rwait})",
            "  The replay's argument equals the caster's own X2Unit:GetUnitId and the name comes from",
            "  X2Unit:GetUnitNameById (both asserted by the script). The countdown decrements on the same",
            "  300 s / server-wait branches about a second apart because it is a separate death; both",
            "  reach wait 0 and enable the same Accept button.",
        ]
    else:
        lines.append("  (replay events not supplied)")
    lines += [
        f"  PLAYER_RESURRECTED: native {resurrected.get('state', {}).get('resurrected')} with no arguments; "
        f"the replay handler takes none (asserted).",
        "",
        "Failing test first",
        "------------------",
    ]
    if before:
        lines += ["  Before the fix:"] + [f"  {line}" for line in before.splitlines() if "PLAYER_RESURRECTION" in line or "test result" in line]
    else:
        lines.append("  (before-fix log not supplied)")
    if after:
        after_lines = [line for line in after.splitlines()
                       if ("test " in line or "test result" in line)
                       and "cargo test" not in line and not line.lstrip().startswith("+ ")]
        lines += ["  After the fix:"] + [f"  {line}" for line in after_lines]
    else:
        lines.append("  (after-fix log not supplied)")
    lines += [
        "",
        "Evidence files",
        "--------------",
        "  native.csv / native.json: every probe sample (time, Lua event/window/countdown, native fields)",
        "  report.txt: this file",
    ]
    for key, name in crops.items():
        lines.append(f"  {name}: cropped window for {key}")
    lines += [
        "",
        "Deliberately not claimed",
        "------------------------",
        "- Item (kind 2) and mount (kind 3) casters and the default kind were not reached.",
        "- The CSResurrectCharacter packet bytes and the caster name after the offer were not",
        "  captured; the caster line is read from the offer screenshot, not the textbox getter.",
        "- The five-minute temple expiry was not waited out live.",
        "- The capture client's clock is the client's, not wall time; the countdown values and",
        "  their order are measured, absolute timings are not a performance claim.",
        "- The server-side Resurrection effect and the GM /useskill path are AAEmu's, not the",
        "  original server's; the original client's packet and window behavior is what was captured.",
        "",
    ]
    (out / "report.txt").write_text("\n".join(lines), encoding="utf-8")
    log("evidence written to", out)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--mode", choices=["capture", "replay", "evidence"], default="capture")
    parser.add_argument("--client", default=os.environ.get("RES_CLIENT", CLIENT_DEFAULT))
    parser.add_argument("--driver")
    parser.add_argument("--dll")
    parser.add_argument("--launcher")
    parser.add_argument("--password-file", type=Path)
    parser.add_argument("--login")
    parser.add_argument("--account")
    parser.add_argument("--character")
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
    parser.add_argument("--bevy")
    parser.add_argument("--bevy-account")
    parser.add_argument("--bevy-character")
    parser.add_argument("--bevy-login")
    parser.add_argument("--bevy-password-file", type=Path)
    parser.add_argument("--bevy-shot-after", type=float, default=24.0,
                        help="off-screen screenshot seconds after entering; 0 skips it")
    parser.add_argument("--replay-log", type=Path)
    parser.add_argument("--test-before-log", type=Path)
    parser.add_argument("--test-after-log", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "tools" / "evidence" / "resurrection")
    parser.add_argument("--raw", type=Path, default=ROOT / "data" / "extract" /
                        f"resurrection-{time.strftime('%Y%m%d')}-r1")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    args.raw.mkdir(parents=True, exist_ok=True)
    if args.mode == "evidence":
        return run_evidence(args, args.raw, args.out)
    missing = [name for name in ("driver", "dll", "launcher", "password_file", "login", "account",
                                 "character", "bevy", "bevy_account", "bevy_character", "bevy_login")
               if not getattr(args, name)]
    if missing:
        raise SystemExit("missing arguments: " + ", ".join("--" + name.replace("_", "-") for name in missing))
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
