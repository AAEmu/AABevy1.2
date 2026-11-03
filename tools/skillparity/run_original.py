#!/usr/bin/env python3
"""Drive the original 1.2 client through aaemu-clientdriver (MCP) and record `<capture-root>/<run-id>/orig/trace.jsonl`.

The client is one you (or the lane's `live-stack.sh client`) already started: attach with `--pid`. This tool never starts or
stops a client, never sends `LeaveWorld` or quits, never sends desktop input (keys go through the driver's `client_input`).

    # once: the GM character and the training doll, then the action bar of a batch
    python tools/skillparity/run_original.py setup --pid 1234 --driver D --dll L --launcher R --client C --login H:P \\
        --plan plan.json --move "15530 15323 131" --spawn-doll
    python tools/skillparity/run_original.py bar --pid 1234 ... --plan plan.json --batch 1

    # the recording: casts the plan's skills (re-slotting the bar for each batch), traces, optional video
    python tools/skillparity/run_original.py run --pid 1234 ... --plan plan.json --trees 1,2 \\
        --capture-root <capture-root> --run-id 2026-11-03-a [--video --ffmpeg FFMPEG] [--ground-click 960,540]

    # the README role rule on a finished trace (no driver needed)
    python tools/skillparity/run_original.py roles <capture-root>/<run-id>/orig/trace.jsonl

Driver tools used (the existing ones: client_attach, client_status, client_lua, client_input, client_chat, client_target,
client_unit_state, client_pointer; and the two new ones, checked up front with `tools/list`, a clear error when missing):

* `client_trace {pid, action: start|drain|stop|status, kinds?}`  drain = JSONL rows (`src:"orig"`, `wall_us`, `frame`) since
  the last drain; the driver writes the `press` rows of keys sent through `client_input` while tracing (a press row missing
  after a key is added by this tool, `source:"runner"`).
* `client_video {pid, action: start|stop|status, path, fps, ffmpeg?}`  writes `path` and `path.json` (`{t0_wall_us, fps, ...}`);
  copied to `video.json` (README name) after the stop.

What the runner adds itself (README: the runner writes its own marks): `mark skill:<id>:start` / `skill:<id>:end` rows with its
own `wall_us`, and `castbar`, `cooldown` and `pos` change rows from a per-frame Lua timer inside the client:

* castbar: `X2Unit:UnitCastingInfo('player')` (name, total ms, ms so far); `channel` is left out (the fourth value's meaning is not
  known) unless the plan says the skill is a channel with no cast time.
* cooldown: `X2Action:GetActionCooldownEx(slot)` of the pressed slot = (combined total, combined remain, skill total, skill remain)
  in ms (real values in tools/evidence/combat_core/native.json); a row for the skill's own cooldown, and one for the global
  cooldown when the combined pair is longer than the skill's own.
* pos: entity world position and angle from the system Lua state (as record_original.py's SYSTEM_SAMPLE) plus
  `X2Util:GetSeamlessOrigin()`; a row when a unit moved more than 1 cm or turned 0.1 degrees. `yaw` is the entity's `GetAngles().z`
  in degrees (0 = north, counter-clockwise: the engine's convention), left out when the call is not there.

Client timestamps of these rows are converted to the wall clock from bracketed clock reads (the sample with the smallest round trip
wins); the uncertainty (half the round trip plus the UI clock's frame-start stamping) is written to meta.json.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import math
import os
import re
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import common  # noqa: E402
import plan as planlib  # noqa: E402
from mcp_driver import DriverError, DriverGone, Mcp, MissingToolError, add_driver_args  # noqa: E402

REPO = HERE.parents[1]
RECORD_ORIGINAL = REPO / "tools" / "combat-side" / "record_original.py"
TRACE_KINDS = ["anim", "fx", "sound", "frame", "press"]
POLL_S = 0.2
RETICLE_S = 1.2           # seconds the ground reticle gets before the click
SETTLE_S = 0.6            # between skills
UI_PRESS_MS = 80


def log(*a: object) -> None:
    print("run_original:", *a, flush=True)


# ---- record_original.py, loaded by path (its directory name has a hyphen) -------------------------------------------------

_record_original = None


def record_original():
    """The combat recorder module: its `find_target`, `self_target`, `CHAT_HOOK` / `CHAT_UNHOOK` are reused as they are."""
    global _record_original
    if _record_original is None:
        if not RECORD_ORIGINAL.is_file():
            raise RuntimeError(f"cannot find {RECORD_ORIGINAL}: run from a full checkout")
        spec = importlib.util.spec_from_file_location("record_original", RECORD_ORIGINAL)
        module = importlib.util.module_from_spec(spec)
        assert spec.loader is not None
        sys.modules.setdefault("record_original", module)
        spec.loader.exec_module(module)
        _record_original = module
    return _record_original


class NullWriter:
    """record_original's functions write chat rows to a channel writer; this run does not keep them."""

    def write(self, *args: Any, **kwargs: Any) -> None:
        return None


# ---- Lua ----------------------------------------------------------------------------------------------------------------

def lua_str(text: str) -> str:
    """A Lua double-quoted string literal."""
    return '"' + text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\r", "") + '"'


TIMER_TEMPLATE = r"""
_G.SP = _G.SP or {}
local SP = _G.SP
SP.errors = SP.errors or {}
SP[KEY .. '_rows'] = {}
SP[KEY .. '_stop'] = false
local function tick()
  if _G.SP == nil or SP[KEY .. '_stop'] then return end
  local ok, row = pcall(function()
SAMPLE
  end)
  if not ok then
    SP.errors[KEY] = tostring(row)
  elseif type(row) == 'string' and row ~= '' then
    local rows = SP[KEY .. '_rows']
    if rows and #rows < 200000 then rows[#rows + 1] = row end
  end
  Script.SetTimer(INTERVAL, tick)
end
Script.SetTimer(INTERVAL, tick)
return 'installed'
"""


def lua_timer(key: str, interval_ms: int, sample: str) -> str:
    """Lua that runs `sample` (a function body returning a string or '') every `interval_ms` (1 = every frame) into a buffer."""
    return TIMER_TEMPLATE.replace("KEY", lua_str(key)).replace("INTERVAL", str(int(interval_ms))).replace("SAMPLE", sample)


TAKE_ROWS = ("local c=_G.SP if not c then return '' end local t=c[%s..'_rows'] if not t then return '' end "
             "c[%s..'_rows']={} return table.concat(t,'\\n')")
STOP_TIMER = "if _G.SP then _G.SP[%s..'_stop']=true end return 1"
READ_ERRORS = ("local out={} if _G.SP and _G.SP.errors then for k,v in pairs(_G.SP.errors) do out[#out+1]=k..': '..v end end "
               "return table.concat(out,'; ')")
CLEANUP = "_G.SP=nil return 1"

# the per-frame sampler of the UI state: the cast bar and the cooldown of the pressed slot
UI_SAMPLE = r"""
local SP = _G.SP
local now = X2Time:GetUiMsec()
local out = {}
local n, total, cur, extra = X2Unit:UnitCastingInfo('player')
local key = ''
if n ~= nil then key = tostring(n) .. '/' .. tostring(total) end
if key ~= (SP.cast_key or '') then
  if (SP.cast_key or '') ~= '' then
    out[#out + 1] = 'C|' .. now .. '|stop|' .. (SP.cast_name or '') .. '|' .. (SP.cast_total or '') .. '|' .. tostring(cur or '') .. '|'
  end
  if key ~= '' then
    out[#out + 1] = 'C|' .. now .. '|start|' .. tostring(n) .. '|' .. tostring(total) .. '|' .. tostring(cur or '') .. '|' .. tostring(extra or '')
  end
  SP.cast_key = key
  SP.cast_name = tostring(n)
  SP.cast_total = tostring(total)
end
local slot = SP.slot
if slot and slot > 0 then
  local ok, a, b, c, d = pcall(X2Action.GetActionCooldownEx, X2Action, slot)
  if ok then
    a = tonumber(a) or 0 b = tonumber(b) or 0 c = tonumber(c) or 0 d = tonumber(d) or 0
    local sig = ((b > 0 or d > 0) and 1 or 0) .. ':' .. a .. ':' .. c
    if slot ~= SP.cd_slot then SP.cd_slot = slot SP.cd_sig = '0:' .. a .. ':' .. c end
    if sig ~= SP.cd_sig then
      out[#out + 1] = 'K|' .. now .. '|' .. slot .. '|' .. a .. '|' .. b .. '|' .. c .. '|' .. d .. '|' .. ((b > 0 or d > 0) and 'start' or 'stop')
      SP.cd_sig = sig
    end
  end
end
return table.concat(out, '\n')
"""

# the per-frame sampler of the system state: the player's and the target's world position and angle (changes only)
SYS_SAMPLE = r"""
local SP = _G.SP
local now = System.GetCurrTime() * 1000.0
SP.tick = (SP.tick or 0) + 1
if SP.tick % 30 == 1 or not SP.me then
  SP.me, SP.tgt = nil, nil
  local best
  for _, x in pairs(System.GetEntities() or {}) do
    local ok, name = pcall(function() return x:GetName() end)
    if ok and name == CHAR_NAME then SP.me = x end
  end
  if SP.me then
    local okq, q = pcall(function() return SP.me:GetWorldPos() end)
    for _, x in pairs(System.GetEntities() or {}) do
      local ok, name = pcall(function() return x:GetName() end)
      if ok and name == TARGET_NAME and okq and q then
        local okp, p = pcall(function() return x:GetWorldPos() end)
        if okp and p then
          local d = (p.x - q.x) ^ 2 + (p.y - q.y) ^ 2 + (p.z - q.z) ^ 2
          if best == nil or d < best then best = d SP.tgt = x end
        end
      end
    end
  end
end
SP.last = SP.last or {}
local out = {}
local function emit(unit, e)
  if not e then return end
  local ok, p = pcall(function() return e:GetWorldPos() end)
  if not ok or not p then if unit == 'player' then SP.me = nil else SP.tgt = nil end return end
  local yaw = ''
  local oka, ang = pcall(function() return e:GetAngles() end)
  if oka and type(ang) == 'table' and ang.z then yaw = string.format('%.2f', ang.z * 180 / math.pi) end
  local l = SP.last[unit]
  local moved = (l == nil) or ((p.x - l.x) ^ 2 + (p.y - l.y) ^ 2 + (p.z - l.z) ^ 2 > 0.0001)
  if not moved and yaw ~= '' and l.yaw ~= '' then
    local dy = math.abs(tonumber(yaw) - tonumber(l.yaw))
    if dy > 180 then dy = 360 - dy end
    moved = dy > 0.1
  end
  if moved then
    out[#out + 1] = string.format('P|%.1f|%s|%.4f|%.4f|%.4f|%s', now, unit, p.x, p.y, p.z, yaw)
    SP.last[unit] = { x = p.x, y = p.y, z = p.z, yaw = yaw }
  end
end
emit('player', SP.me)
emit('target', SP.tgt)
return table.concat(out, '\n')
"""

ORIGIN_LUA = r"""
local ok, a, b, c = pcall(function() return X2Util:GetSeamlessOrigin() end)
if not ok or a == nil then return '' end
if type(a) == 'table' then return tostring(a.x or a[1] or 0) .. ',' .. tostring(a.y or a[2] or 0) .. ',' .. tostring(a.z or a[3] or 0) end
return tostring(a) .. ',' .. tostring(b or 0) .. ',' .. tostring(c or 0)
"""

CLOCK_UI = "local ok,t=pcall(function() return X2Time:GetUiMsec() end) if ok and t then return t end return ''"
CLOCK_SYS = "return System.GetCurrTime()"

SET_BAR_LUA = r"""
pcall(function() X2Action:SetActionBarPage(1) end)
for slot = 1, CLEAR do pcall(function() X2Action:ClearAction(slot) end) end
local map = {MAP}
local out = {}
for slot, id in pairs(map) do
  local ok = pcall(function() X2Action:SetActionSpell(slot, id) end)
  local ok2, t = pcall(X2Action.GetActionTooltipText, X2Action, slot)
  local have, name = '', ''
  if ok2 and type(t) == 'table' then
    if t.type ~= nil then have = tostring(t.type) end
    if t.name ~= nil then name = tostring(t.name) end
  end
  out[#out + 1] = slot .. ':' .. tostring(ok) .. ':' .. have .. ':' .. name
end
table.sort(out)
return table.concat(out, '|')
"""


# ---- clock mapping ------------------------------------------------------------------------------------------------------

class ClockMap:
    """Client clock (ms) -> wall clock (us) from bracketed reads; the sample with the smallest round trip near the row wins."""

    DRIFT = 1e-4          # seconds of uncertainty per second between a row and a sample (100 ppm)

    def __init__(self, name: str):
        self.name = name
        self.samples: list[tuple[float, float, float]] = []   # (client_ms, wall_mid_us, rtt_us)

    def add(self, client_ms: float, before_us: int, after_us: int) -> None:
        self.samples.append((float(client_ms), (before_us + after_us) / 2.0, float(max(after_us - before_us, 0))))

    def wall_us(self, client_ms: float) -> int | None:
        if not self.samples:
            return None
        best = min(self.samples, key=lambda s: s[2] / 2.0 + abs(client_ms - s[0]) * 1000.0 * self.DRIFT)
        return int(round(best[1] + (client_ms - best[0]) * 1000.0))

    def report(self) -> dict:
        if not self.samples:
            return {"samples": 0}
        rtts = sorted(s[2] for s in self.samples)
        return {"samples": len(self.samples), "min_rtt_ms": round(rtts[0] / 1000.0, 3), "median_rtt_ms": round(rtts[len(rtts) // 2] / 1000.0, 3),
                "uncertainty_ms": round(rtts[0] / 2000.0, 3),
                "note": "half the smallest round trip; the UI clock is also stamped at frame start (about half a frame more)"}


# ---- sampler rows -> trace rows -----------------------------------------------------------------------------------------

def num(text: str) -> float | None:
    try:
        return float(text)
    except (TypeError, ValueError):
        return None


def convert_ui_rows(lines: list[str], clock: ClockMap, slot_skill: dict | None = None) -> list[dict]:
    """`C|ms|start|name|total|cur|extra` and `K|ms|slot|a|b|c|d|start` lines -> `castbar` and `cooldown` rows."""
    rows: list[dict] = []
    for line in lines:
        f = line.split("|")
        if len(f) < 3 or f[0] not in ("C", "K"):
            continue
        ms = num(f[1])
        wall = clock.wall_us(ms) if ms is not None else None
        if wall is None:
            continue
        if f[0] == "C" and len(f) >= 6:
            row: dict[str, Any] = {"src": "orig", "kind": "castbar", "wall_us": wall, "unit": "player", "state": f[2],
                                   "name": f[3]}
            total, cur = num(f[4]), num(f[5])
            if total is not None:
                row["total_ms"] = int(total)
            if cur is not None:
                row["elapsed_ms"] = int(cur)
            if slot_skill and slot_skill.get("channel_ms") and not slot_skill.get("cast_ms"):
                row["channel"] = True
            elif slot_skill and slot_skill.get("channel_ms") == 0:
                row["channel"] = False
            rows.append(row)
        elif f[0] == "K" and len(f) >= 8:
            # (a, b) = the combined pair (total, remain) the bar shows, (c, d) = the skill's own cooldown. The skill's own row
            # is c/d; the global cooldown only shows as its own row when the combined pair is longer than the skill's.
            slot = int(num(f[2]) or 0)
            a, b, c, d = (num(x) or 0.0 for x in f[3:7])
            started = f[7] == "start"

            def cd(gcd: bool, total: float, remain: float) -> dict:
                return {"src": "orig", "kind": "cooldown", "wall_us": wall, "slot": slot, "remain_ms": int(remain),
                        "total_ms": int(total), "gcd": gcd}

            if started:
                if c > 0:
                    rows.append(cd(False, c, d))
                if a > 0 and a != c:
                    rows.append(cd(True, a, b))
            else:                                  # both ended: the zero rows say when
                rows.append(cd(False, c, 0))
                if a > 0 and a != c:
                    rows.append(cd(True, a, 0))
    return rows


def parse_origin(text: str) -> tuple[float, float, float] | None:
    parts = [num(p) for p in text.strip().split(",")]
    if len(parts) >= 3 and None not in parts[:3]:
        return parts[0], parts[1], parts[2]
    return None


def convert_sys_rows(lines: list[str], clock: ClockMap, origin: tuple[float, float, float] | None) -> list[dict]:
    """`P|ms|unit|x|y|z|yaw` lines -> `pos` rows in game coordinates (the seamless origin added; yaw 0 = north, counter-clockwise)."""
    ox, oy, oz = origin or (0.0, 0.0, 0.0)
    rows: list[dict] = []
    for line in lines:
        f = line.split("|")
        if len(f) < 6 or f[0] != "P":
            continue
        ms, x, y, z = num(f[1]), num(f[3]), num(f[4]), num(f[5])
        wall = clock.wall_us(ms) if ms is not None else None
        if wall is None or None in (x, y, z):
            continue
        row: dict[str, Any] = {"src": "orig", "kind": "pos", "wall_us": wall, "unit": f[2], "x": round(x + ox, 4),
                               "y": round(y + oy, 4), "z": round(z + oz, 4)}
        yaw = num(f[6]) if len(f) > 6 else None
        if yaw is not None:
            row["yaw"] = round(yaw % 360.0, 2)
        rows.append(row)
    return rows


# ---- the session --------------------------------------------------------------------------------------------------------

class Session:
    """One attached client: hooks, samplers, clocks, the trace writer. All driver calls go through `lua` / `mcp.text`."""

    def __init__(self, mcp: Mcp, pid: int, out_dir: Path | None, args: argparse.Namespace):
        self.mcp, self.pid, self.args = mcp, pid, args
        self.out_dir = out_dir
        self.writer = common.TraceWriter(out_dir / "trace.jsonl", "orig") if out_dir else None
        self.clock_ui, self.clock_sys = ClockMap("ui"), ClockMap("system")
        self.origin: tuple[float, float, float] | None = None
        self.character = ""
        self.target_name = args.target_name or planlib.DUMMY_NAME
        self.counts: dict[str, int] = {}
        self.warnings: list[str] = []
        self.skipped: list[dict] = []
        self.slot_skill: dict = {}
        self.timers: list[tuple[str, str]] = []
        self.tracing = False
        self.video_path: Path | None = None

    # -- plumbing
    def mark(self, label: str, skill_id: int | None = None) -> None:
        if self.writer:
            self.writer.mark(label, skill_id)
            self.counts["mark"] = self.counts.get("mark", 0) + 1

    def lua(self, code: str, target: str = "ui") -> str:
        return self.mcp.text("client_lua", pid=self.pid, target=target, code=code)

    def warn(self, text: str) -> None:
        log("warning:", text)
        self.warnings.append(text)

    def emit(self, row: dict) -> None:
        self.counts[row.get("kind", "?")] = self.counts.get(row.get("kind", "?"), 0) + 1
        if self.writer:
            self.writer.write(row)

    def sample_clock(self) -> None:
        for which, code, clock, scale in (("ui", CLOCK_UI, self.clock_ui, 1.0), ("system", CLOCK_SYS, self.clock_sys, 1000.0)):
            before = common.now_us()
            try:
                text = self.lua(code, target=which).strip()
            except DriverError:
                continue
            after = common.now_us()
            value = num(text)
            if value is not None:
                clock.add(value * scale, before, after)

    # -- start / stop
    def wait_player(self, timeout: float = 120.0) -> str:
        end = time.monotonic() + timeout
        last = ""
        while time.monotonic() < end:
            try:
                state = self.lua("return X2LoginCharacter:GetCurrentStage(), X2Unit and X2Unit:UnitName('player')")
                stage, _, name = state.partition("\t")
                if stage.strip() == "0" and name.strip() not in ("", "nil"):
                    return name.strip()
                last = state
            except DriverError as error:
                last = str(error)
            time.sleep(0.5)
        raise RuntimeError("the player is not in the world: " + last)

    def attach(self) -> str:
        self.mcp.text("client_attach", pid=self.pid)
        status = self.mcp.text("client_status", pid=self.pid)
        self.character = self.wait_player()
        log("attached to pid", self.pid, "player", self.character)
        return status

    def install_hooks(self) -> None:
        ro = record_original()
        self.lua(ro.CHAT_HOOK)
        self.lua("local o=assert(_G.__combat_recorder) o.chat={} return 1")

    def remove_hooks(self) -> None:
        try:
            self.lua(record_original().CHAT_UNHOOK)
        except Exception as error:   # the client may be gone: nothing more to clean
            log("hook cleanup:", error)

    def install_samplers(self) -> None:
        self.sample_clock()
        sys_code = SYS_SAMPLE.replace("CHAR_NAME", lua_str(self.character)).replace("TARGET_NAME", lua_str(self.target_name))
        for target, key, sample in (("ui", "sp_ui", UI_SAMPLE), ("system", "sp_sys", sys_code)):
            reply = self.lua(lua_timer(key, 1, sample), target=target)
            if "installed" not in reply:
                self.warn(f"sampler {key}: unexpected reply {reply[:80]!r}")
            self.timers.append((target, key))
        self.refresh_origin()

    def refresh_origin(self) -> None:
        try:
            origin = parse_origin(self.lua(ORIGIN_LUA))
        except DriverError:
            origin = None
        if origin is None and self.origin is None:
            self.warn("X2Util:GetSeamlessOrigin() gave nothing: pos rows are in the client's local coordinates")
        self.origin = origin or self.origin

    def stop_samplers(self) -> None:
        for target, key in self.timers:
            try:
                self.lua(STOP_TIMER % lua_str(key), target=target)
            except Exception:
                pass
        try:
            errors = self.lua(READ_ERRORS, target="ui") + " " + self.lua(READ_ERRORS, target="system")
            if errors.strip(" ;"):
                self.warn("sampler errors in the client: " + errors.strip())
        except Exception:
            pass
        for target in ("ui", "system"):
            try:
                self.lua(CLEANUP, target=target)
            except Exception:
                pass

    def start_trace(self) -> None:
        reply = self.mcp.text("client_trace", pid=self.pid, action="start", kinds=TRACE_KINDS)
        self.tracing = True
        log("client_trace start:", reply.strip()[:120])

    def stop_trace(self) -> dict:
        info: dict = {}
        if not self.tracing:
            return info
        try:
            self.drain()
            status = self.mcp.text("client_trace", pid=self.pid, action="status")
            info = json.loads(status) if status.strip().startswith("{") else {"raw": status.strip()}
            self.mcp.text("client_trace", pid=self.pid, action="stop")
        except Exception as error:
            self.warn(f"client_trace stop: {error}")
        self.tracing = False
        if info.get("dropped"):
            self.warn(f"the driver dropped {info['dropped']} trace rows: the run is not complete")
        return info

    def start_video(self, path: Path, fps: int, ffmpeg: str | None) -> None:
        self.mcp.require("client_video")
        params: dict[str, Any] = {"pid": self.pid, "action": "start", "path": str(path), "fps": fps}
        if ffmpeg:
            params["ffmpeg"] = ffmpeg
        self.mcp.text("client_video", **params)
        self.video_path = path

    def stop_video(self) -> dict | None:
        if self.video_path is None:
            return None
        info = None
        try:
            self.mcp.text("client_video", pid=self.pid, action="stop")
        except Exception as error:
            self.warn(f"client_video stop: {error}")
        sidecar = Path(str(self.video_path) + ".json")
        for _ in range(50):                      # the driver finishes the file after the stop call returns
            if sidecar.is_file():
                break
            time.sleep(0.2)
        if sidecar.is_file():
            info = json.loads(sidecar.read_text(encoding="utf-8"))
            (self.video_path.parent / "video.json").write_text(json.dumps(info, indent=1) + "\n", encoding="utf-8")
            if info.get("dropped"):
                self.warn(f"the video dropped {info['dropped']} frames")
        else:
            self.warn("no video sidecar was written")
        return info

    # -- draining
    def drain(self) -> list[dict]:
        """Move everything the driver and the samplers have into the trace; returns the driver's rows."""
        rows: list[dict] = []
        if self.tracing:
            text = self.mcp.text("client_trace", pid=self.pid, action="drain")
            for line in text.splitlines():
                line = line.strip()
                if not line:
                    continue
                try:
                    row = json.loads(line)
                except ValueError:
                    self.counts["unreadable"] = self.counts.get("unreadable", 0) + 1
                    continue
                if isinstance(row, dict) and isinstance(row.get("wall_us"), (int, float)):
                    row.setdefault("src", "orig")
                    rows.append(row)
                    self.emit(row)
        ui = self.lua(TAKE_ROWS % (lua_str("sp_ui"), lua_str("sp_ui")), "ui")
        for row in convert_ui_rows(ui.splitlines(), self.clock_ui, self.slot_skill):
            self.emit(row)
        sys_rows = self.lua(TAKE_ROWS % (lua_str("sp_sys"), lua_str("sp_sys")), "system")
        for row in convert_sys_rows(sys_rows.splitlines(), self.clock_sys, self.origin):
            self.emit(row)
        return rows

    # -- target and bar
    def unit_state(self, which: str) -> dict:
        try:
            return json.loads(self.mcp.text("client_unit_state", pid=self.pid, target=which))
        except (DriverError, ValueError):
            return {}

    def ensure_dummy(self, template: int) -> None:
        state = self.unit_state("target")
        if state.get("present") and state.get("name") == self.target_name and int(state.get("health") or 0) > 0:
            return
        ro = record_original()
        ro.find_target(self.mcp, self.pid, NullWriter(), template, self.target_name)

    def select(self, skill: dict, template: int) -> None:
        if skill.get("select") == "self":
            record_original().self_target(self.mcp, self.pid)
        else:
            self.ensure_dummy(template)

    def set_bar(self, batch: dict, slots_per_page: int) -> list[str]:
        mapping = ",".join(f"[{int(s)}]={int(i)}" for s, i in sorted(batch["slots"].items(), key=lambda kv: int(kv[0])))
        text = self.lua(SET_BAR_LUA.replace("CLEAR", str(slots_per_page)).replace("MAP", mapping))
        problems = []
        for entry in text.split("|"):
            parts = entry.split(":", 3)
            if len(parts) < 3:
                continue
            slot, ok, have = parts[0], parts[1], parts[2]
            want = str(batch["slots"].get(slot, ""))
            if ok != "true" or (have and want and have != want):
                problems.append(f"slot {slot}: wanted {want}, bar says {have or '?'} (SetActionSpell ok={ok})")
        if not text.strip():
            problems.append("the bar setup returned nothing")
        for p in problems:
            self.warn("action bar: " + p)
        return problems

    # -- one skill
    def press(self, key: str, slot: int) -> tuple[int, int]:
        before = common.now_us()
        reply = self.mcp.text("client_input", pid=self.pid, key=key, duration_ms=UI_PRESS_MS)
        after = common.now_us()
        rows = self.drain()
        seen = any(r.get("kind") == "press" and before - 20_000 <= r["wall_us"] <= after + 200_000 for r in rows)
        if not seen:
            self.emit({"src": "orig", "kind": "press", "wall_us": before, "key": key, "slot": slot, "source": "runner",
                       "reply": reply.strip()[:80]})
        return before, after

    def click(self, x: int, y: int) -> None:
        self.mcp.text("client_pointer", pid=self.pid, action="click", x=x, y=y)

    def cast(self, skill: dict, template: int, ground_click: tuple[int, int] | None, stop_key: str) -> None:
        sid, slot = skill["id"], skill["slot"]
        if skill.get("target") == "ground" and ground_click is None:
            self.skipped.append({"id": sid, "reason": "ground skill needs --ground-click"})
            self.warn(f"skill {sid} ({skill.get('name')}) is a ground skill: pass --ground-click X,Y to cast it; skipped")
            return
        self.slot_skill = skill
        self.lua(f"_G.SP.slot={int(slot)} return 1")
        self.select(skill, template)
        self.sample_clock()
        self.refresh_origin()
        self.mark(f"skill:{sid}:start", sid)
        try:
            self.run_skill(skill, ground_click, stop_key)
        finally:
            self.sample_clock()
            self.mark(f"skill:{sid}:end", sid)
        time.sleep(SETTLE_S)

    def run_skill(self, skill: dict, ground_click: tuple[int, int] | None, stop_key: str) -> None:
        self.press(skill["key"], skill["slot"])
        t_press = time.monotonic()
        deadline = t_press + skill["wait_ms"] / 1000.0
        stop_at = t_press + skill["stop_after_ms"] / 1000.0 if skill.get("stop_after_ms") else None
        click_at = t_press + RETICLE_S if skill.get("target") == "ground" else None
        while time.monotonic() < deadline:
            started = time.monotonic()
            if click_at and started >= click_at and ground_click:
                self.click(*ground_click)
                click_at = None
            if stop_at and started >= stop_at:
                self.mcp.text("client_input", pid=self.pid, key=stop_key, duration_ms=UI_PRESS_MS)
                stop_at = None
            self.drain()
            time.sleep(max(0.0, POLL_S - (time.monotonic() - started)))
        self.drain()


# ---- finishing a trace --------------------------------------------------------------------------------------------------

def finalize_trace(path: Path) -> dict:
    """Sort the trace by `wall_us`, apply the README role rule, rewrite the file. Returns the role summary."""
    rows, bad = common.read_jsonl(path)
    summary = common.ensure_roles(rows)
    common.write_jsonl(path, rows)
    summary["rows"], summary["unreadable_lines"] = len(rows), bad
    return summary


# ---- commands -----------------------------------------------------------------------------------------------------------

def parse_pair(text: str | None) -> tuple[int, int] | None:
    if not text:
        return None
    x, y = (int(v) for v in re.split(r"[ ,]+", text.strip())[:2])
    return x, y


def gm(session: Session, command: str, wait: float = 0.8) -> None:
    session.mcp.text("client_chat", pid=session.pid, message=command)
    log("gm", command)
    time.sleep(wait)


def open_driver(args: argparse.Namespace, artifact: Path | None) -> Mcp:
    """Start the driver process for the given arguments (a missing exe, dll, launcher or client path is an error up front)."""
    if args.pid is None:
        raise SystemExit("--pid is required: attach to a client you started (the lane's live-stack.sh client)")
    for flag in ("driver", "dll", "launcher", "client"):
        path = getattr(args, flag)
        if path is not None and not Path(path).exists():
            raise SystemExit(f"missing --{flag} {path}")
    # the driver's scratch directory: next to the capture, else under the git-ignored data/local/ of the checkout
    return Mcp.from_args(args, artifact or REPO / "data" / "local" / "skillparity-mcp")


def connect(args: argparse.Namespace, out_dir: Path | None) -> tuple[Mcp, Session]:
    mcp = open_driver(args, (out_dir / "mcp") if out_dir else None)
    return mcp, Session(mcp, args.pid, out_dir, args)


# The follow camera of r208022 (tools/camera_probe.py): camera::g_camera = x2game+0xb0e2b8 -> +0x20 the wheel's wanted distance (m),
# +0x84 the yaw offset from the heading (radians). There is no Lua or cvar for either (cl_tpvYaw is not read by this camera), so
# the runner writes the two floats; the camera keeps them until the wheel or a mouse drag changes them.
CAMERA_GLOBAL_RVA = 0xB0E2B8


def set_camera(pid: int, yaw_deg: float, distance: float) -> None:
    import ctypes
    import math
    import struct

    k = ctypes.WinDLL("kernel32", use_last_error=True)
    sys.path.insert(0, str(HERE.parent / "combat-side"))
    from record_original import Mem  # noqa: E402

    mem = Mem(pid)
    camera = mem.u32(mem.module_base("x2game.dll") + CAMERA_GLOBAL_RVA)
    if not camera:
        raise RuntimeError("no follow camera")
    handle = k.OpenProcess(0x0008 | 0x0020, False, pid)   # PROCESS_VM_OPERATION | PROCESS_VM_WRITE
    try:
        for offset, value in ((0x20, distance), (0x84, math.radians(yaw_deg))):
            written = ctypes.c_size_t()
            if not k.WriteProcessMemory(handle, ctypes.c_void_p(camera + offset), struct.pack("<f", value), 4, ctypes.byref(written)):
                raise RuntimeError(f"camera write failed ({ctypes.get_last_error()})")
    finally:
        k.CloseHandle(handle)


def cmd_setup(args: argparse.Namespace) -> int:
    plan = planlib.load_plan(args.plan) if args.plan else None
    if not args.target_name and plan:
        args.target_name = plan.get("dummy", {}).get("name", "")
    mcp, session = connect(args, None)
    try:
        session.attach()
        commands = list(args.gm)
        if not args.no_default_gm:
            commands = list((plan or {}).get("setup", {}).get("gm", ["/level 50", "/ignoreskillcds true"])) + commands
        for command in commands:
            gm(session, command)
        if args.move:
            gm(session, "/move " + args.move.replace(",", " "), wait=2.0)
        template = (plan or {}).get("dummy", {}).get("template", planlib.DUMMY_TEMPLATE)
        if args.spawn_doll:
            gm(session, f"/despawn npc {template} 30")
            gm(session, f"/spawn npc {template}", wait=2.0)
        if args.spawn_doll or args.find_doll:
            session.install_hooks()
            try:
                record_original().find_target(mcp, args.pid, NullWriter(), template, session.target_name)
                log("the doll is targeted:", session.unit_state("target"))
            finally:
                session.remove_hooks()
        return 0
    finally:
        mcp.close()


def cmd_bar(args: argparse.Namespace) -> int:
    plan = planlib.load_plan(args.plan)
    batches = [b for b in plan["batches"] if b["batch"] == args.batch] if args.batch else planlib.select_batches(plan, planlib.parse_int_list(args.trees))[:1]
    if not batches:
        raise SystemExit("no such batch")
    mcp, session = connect(args, None)
    try:
        session.attach()
        problems = session.set_bar(batches[0], plan.get("slots_per_page", 10))
        log("batch", batches[0]["batch"], "tree", batches[0]["tree_name"], "slots", batches[0]["slots"],
            "OK" if not problems else f"{len(problems)} problem(s)")
        return 0 if not problems else 1
    finally:
        mcp.close()


def cmd_roles(args: argparse.Namespace) -> int:
    summary = finalize_trace(args.trace)
    print(json.dumps(summary, indent=1))
    return 0 if summary.get("player_inst") else 1


def cmd_run(args: argparse.Namespace) -> int:
    plan = planlib.load_plan(args.plan)
    batches = planlib.select_batches(plan, planlib.parse_int_list(args.trees) or None, planlib.parse_int_list(args.skills) or None)
    if not batches:
        raise SystemExit("the plan has nothing for --trees / --skills")
    root = args.capture_root or common.env_path("AA_PARITY_ROOT")
    if root is None:
        raise SystemExit("--capture-root (or AA_PARITY_ROOT) is required: captures never go into the repository")
    run_id = args.run_id or datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir = Path(root) / run_id / "orig"
    if (out_dir / "trace.jsonl").exists() and not args.force:
        raise SystemExit(f"{out_dir / 'trace.jsonl'} exists: pick another --run-id, or --force to append")
    ground = parse_pair(args.ground_click)
    template = plan.get("dummy", {}).get("template", planlib.DUMMY_TEMPLATE)
    if not args.target_name:
        args.target_name = plan.get("dummy", {}).get("name", planlib.DUMMY_NAME)

    mcp = open_driver(args, out_dir / "mcp")
    try:
        mcp.require("client_trace", *(["client_video"] if args.video else []))   # before anything is written: a clear error
    except MissingToolError:
        mcp.close()
        raise
    out_dir.mkdir(parents=True, exist_ok=True)
    session = Session(mcp, args.pid, out_dir, args)
    started = common.now_us()
    failure: BaseException | None = None
    status = ""
    video_info = None
    trace_info: dict = {}
    try:
        status = session.attach()
        session.install_hooks()
        session.install_samplers()
        session.start_trace()
        if args.camera:
            set_camera(args.pid, *args.camera)
        if args.video:
            session.start_video(out_dir / "video.mp4", args.fps, args.ffmpeg)
        for batch in batches:
            log("batch", batch["batch"], batch["tree_name"], "weapon:", batch["weapon"].get("name"), "slots", batch["slots"])
            problems = session.set_bar(batch, plan.get("slots_per_page", 10))
            if problems and args.strict_bar:
                raise RuntimeError("the action bar is not as planned: " + "; ".join(problems))
            for skill in batch["skills"]:
                log("skill", skill["id"], skill.get("name"), "slot", skill["slot"], "wait", skill["wait_ms"], "ms")
                try:
                    if args.stage:                      # every skill from the same spot, as run_bevy.py --stage does
                        gm(session, "/clearcombat")         # no auto-attack or combat state carried into the next skill
                        gm(session, "/move " + args.stage.replace(",", " "), wait=args.stage_settle)
                    if args.camera:
                        set_camera(args.pid, *args.camera)
                    session.cast(skill, template, ground, args.stop_key)
                except DriverGone:
                    raise
                except RuntimeError as error:          # one skill failing (no live doll, a refused key) does not end the run
                    session.skipped.append({"id": skill["id"], "reason": str(error)[:300]})
                    session.warn(f"skill {skill['id']} failed: {str(error)[:200]}")
    except BaseException as error:
        failure = error
        log("run failed:", type(error).__name__, error)
    finally:
        try:
            session.sample_clock()
            session.drain()
        except Exception as error:
            log("final drain:", error)
        session.stop_samplers()
        trace_info = session.stop_trace()
        video_info = session.stop_video()
        session.remove_hooks()
        if session.writer:
            session.writer.close()
        mcp.close()
    roles = finalize_trace(out_dir / "trace.jsonl")
    meta = {
        "schema": common.SCHEMA, "tool": "skillparity/run_original", "run_id": run_id, "src": "orig",
        "started_wall_us": started, "ended_wall_us": common.now_us(), "pid": args.pid, "character": session.character,
        "client_status": status.strip()[:2000], "lane": args.lane, "server_commit": args.server_commit,
        "plan": {"schema": plan.get("schema"), "batches": batches,
                 "skills": [s for b in batches for s in b["skills"]], "dummy": plan.get("dummy"),
                 "slots_per_page": plan.get("slots_per_page")},
        "trace": trace_info, "video": video_info, "roles": roles, "rows": session.counts,
        "clock": {"ui": session.clock_ui.report(), "system": session.clock_sys.report()},
        "origin": session.origin, "skipped": session.skipped, "warnings": session.warnings,
        "ok": failure is None and not session.warnings,
        "error": f"{type(failure).__name__}: {failure}" if failure else None,
    }
    (out_dir / "meta.json").write_text(json.dumps(meta, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    log("rows", json.dumps(session.counts, sort_keys=True), "->", out_dir)
    if isinstance(failure, (KeyboardInterrupt, SystemExit)):
        raise failure
    return 0 if failure is None else 1


def build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)

    def with_driver(name: str, help_text: str) -> argparse.ArgumentParser:
        p = sub.add_parser(name, help=help_text)
        add_driver_args(p)
        p.add_argument("--target-name", default="", help="the doll's client name (default: the plan's dummy name)")
        return p

    p = with_driver("setup", "GM commands, move, spawn the training doll")
    p.add_argument("--plan", type=Path, default=None)
    p.add_argument("--gm", action="append", default=[], help="a GM command to send (repeatable), e.g. '/level 50'")
    p.add_argument("--no-default-gm", action="store_true", help="skip the plan's setup commands (/level 50, /ignoreskillcds true)")
    p.add_argument("--move", default=None, help='"x y z": /move there first')
    p.add_argument("--spawn-doll", action="store_true", help="/despawn then /spawn npc <doll> and target it")
    p.add_argument("--find-doll", action="store_true", help="target the nearest doll")
    p.set_defaults(func=cmd_setup)

    p = with_driver("bar", "put a batch of the plan on the action bar")
    p.add_argument("--plan", type=Path, required=True)
    p.add_argument("--batch", type=int, default=0)
    p.add_argument("--trees", default="", help="the first batch of these trees")
    p.set_defaults(func=cmd_bar)

    p = with_driver("run", "cast the plan and record the trace")
    p.add_argument("--plan", type=Path, required=True)
    p.add_argument("--trees", default="", help="only these ability trees (0 = the default skills)")
    p.add_argument("--skills", default="", help="only these skill ids")
    p.add_argument("--capture-root", type=Path, default=None, help="default: AA_PARITY_ROOT")
    p.add_argument("--run-id", default=None)
    p.add_argument("--force", action="store_true", help="append to an existing trace.jsonl")
    p.add_argument("--video", action="store_true", help="record orig/video.mp4 with the driver's client_video")
    p.add_argument("--stage", default=None, help='"x y z": /move here before every skill (the same spot as run_bevy.py --stage)')
    p.add_argument("--stage-settle", type=float, default=1.5, help="seconds to wait after the --stage move")
    p.add_argument("--camera", type=lambda v: tuple(float(x) for x in v.split(",")), default=None,
                   help="YAW_DEG,DISTANCE: the follow camera's yaw offset from the heading and wanted distance, e.g. 90,6.0835 "
                        "(aa-client: orbit 90, zoom -3)")
    p.add_argument("--fps", type=int, default=30)
    p.add_argument("--ffmpeg", default=None, help="ffmpeg for client_video (the driver's own default when unset)")
    p.add_argument("--ground-click", default=None, help="X,Y screen pixel clicked after a ground skill's reticle shows (else ground skills are skipped)")
    p.add_argument("--stop-key", default="escape", help="key that stops a channel past the plan's cap and the repeating basic attacks")
    p.add_argument("--strict-bar", action="store_true", help="stop when the bar does not hold what the plan says")
    p.add_argument("--lane", default=None, help="free text for meta.json: the lane")
    p.add_argument("--server-commit", default=None, help="free text for meta.json: the server commit")
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("roles", help="apply the README role rule to a trace file (in place)")
    p.add_argument("trace", type=Path)
    p.set_defaults(func=cmd_roles)
    return ap


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except MissingToolError as error:
        print(f"run_original: {error}", file=sys.stderr)
        return 3
    except DriverError as error:
        print(f"run_original: driver error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
