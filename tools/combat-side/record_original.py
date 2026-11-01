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
rows, ``System.GetCurrTime`` for system rows, and the fitted clock model for
ReadProcessMemory samples) and a ``wall_ms`` recorder stamp.  The first and
last rows of the ``meta`` channel bracket the run.  See README.md in this
directory for the row schema and the decompile citations for the GCD pair.

The client is hidden before any sampling.  The tool never sends desktop input,
never calls ``X2World:LeaveWorld``, and stops the attached client by pid unless
``--keep-client`` is given.

Typical use against a lane stack (the lane's ``live-stack.sh client`` already
started the client):

    python tools/combat-side/record_original.py \\
        --pid 1234 --run-dir <capture-root>/2026-11-01-combat-recorder/run \\
        --driver <lane-target>/live/aaemu-clientdriver.exe \\
        --dll <lane-target>/aadriver/aadriver.dll \\
        --launcher <lane-target>/live/aaemu-launcher.exe \\
        --client <client>/bin32/archeage.exe --login <login-host:port> \\
        --password-file <password-file> \\
        --character CombatT1 --character-ability 2 \\
        --spawn-npc 6409 --npc-name "rat_brown " --fight-keys 1,1,2,1,3,2,1,1

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
import statistics
import struct
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import NamedTuple

SCHEMA = "aabevy.combat-recorder/v2"
DRIFT_WARNING_POLICY = (
    "reported, not gated: the UI clock is stamped at frame start, so a long "
    "run can fit a slope beyond 200 ppm by chance")
PRESS_WINDOW_MS = 500.0
MEMORY_SHIFT_CHANNELS = ("gcd", "anim_controller")
MEMORY_UI_RESIDUAL_MS = 25.0
MEMORY_UI_NEAR_WINDOW_MS = 1000.0
MIN_CONFIRMING_PAIRS = 2
MEMORY_UI_ASSUMPTION = (
    "the GCD is written in the same frame the cast starts; the measured offset "
    "absorbs UI frame-start stamping (about half a 52 ms frame), the 5 ms memory "
    "cadence and any real 1.2 ordering, so after the shift the GCD-rise-vs-"
    "cast-start difference is zero by construction and is not 1.2 evidence")
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
    """Monotonic recorder milliseconds (never jumps with the system clock)."""
    return time.perf_counter_ns() / 1_000_000.0


def sleep_until(deadline: float) -> None:
    """Sleep to a perf_counter deadline; the last ~2 ms spin past timer slop."""
    while True:
        remaining = deadline - time.perf_counter()
        if remaining <= 0.0:
            return
        if remaining > 0.002:
            time.sleep(remaining - 0.001)
        else:
            while time.perf_counter() < deadline:
                pass
            return


def read_clock(mcp: Mcp, pid: int, target: str) -> tuple[float | None, float, float, str | None]:
    """Read one client clock inside a bracket taken under the MCP lock."""
    bracket: dict[str, float] = {}
    try:
        if target == "system":
            text = mcp.text("client_lua", pid=pid, target="system",
                            code="return System.GetCurrTime()", bracket=bracket)
            value: float | None = float(text.strip()) * 1000.0
        else:
            text = mcp.text("client_lua", pid=pid, target="ui",
                            code="local ok,t=pcall(function() return X2Time:GetUiMsec() end) if ok and t then return t end return ''",
                            bracket=bracket)
            value = float(text.strip()) if text.strip() else None
        return (value, bracket.get("before", ms_now()),
                bracket.get("after", ms_now()), None)
    except Exception as error:
        return (None, bracket.get("before", ms_now()),
                bracket.get("after", ms_now()), str(error))


def clean(value: object) -> str:
    text = "" if value is None else str(value)
    return text.replace("\t", " ").replace("\r", " ").replace("\n", " ").replace("|", " ")


def s32(value: int) -> int:
    return value - 0x100000000 if value & 0x80000000 else value


class Mcp:
    """Minimal serialised newline JSON-RPC client for ``aaemu-clientdriver mcp``.

    The driver runs in its own process group so a console Ctrl-C aimed at the
    recorder does not kill it.  A reply left unread by an interrupted call is
    skipped by id on the next call, and ``resync`` restarts a dead driver
    before cleanup so the tail of the run can still be drained.
    """

    def __init__(self, driver: Path, dll: Path, launcher: Path, client: Path,
                 password_file: Path | None, login: str, user: str, artifact: Path):
        self.args = [str(driver), "mcp", "--dll", str(dll), "--client", str(client),
                     "--launcher", str(launcher), "--login", login, "--user", user,
                     "--artifact-dir", str(artifact)]
        if password_file is not None:
            self.args += ["--password-file", str(password_file)]
        self.lock = threading.Lock()
        self.process: subprocess.Popen | None = None
        self.sequence = 0
        self.skipped = 0
        self.start()

    def start(self) -> None:
        self.process = subprocess.Popen(
            self.args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8",
            creationflags=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0))
        self.sequence = 0
        assert self.process.stdin and self.process.stdout
        self.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "combat-recorder", "version": "2"}})
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0",
                                             "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def _read_matching(self) -> dict:
        """Read until the reply to the current request; skip stale replies."""
        assert self.process and self.process.stdout
        for _ in range(32):
            line = self.process.stdout.readline()
            if not line:
                raise RuntimeError("driver MCP closed stdout")
            response = json.loads(line)
            if response.get("id") == self.sequence:
                return response
            self.skipped += 1
        raise RuntimeError(f"driver MCP sent no reply for id {self.sequence}")

    def call(self, method: str, params: object = None,
             bracket: dict[str, float] | None = None) -> object:
        with self.lock:
            assert self.process and self.process.stdin and self.process.stdout
            self.sequence += 1
            request: dict[str, object] = {"jsonrpc": "2.0", "id": self.sequence,
                                          "method": method}
            if params is not None:
                request["params"] = params
            if bracket is not None:
                bracket["before"] = ms_now()
            self.process.stdin.write(json.dumps(request) + "\n")
            self.process.stdin.flush()
            response = self._read_matching()
            if bracket is not None:
                bracket["after"] = ms_now()
            if "error" in response:
                raise RuntimeError(str(response["error"]))
            return response["result"]

    def tool(self, name: str, bracket: dict[str, float] | None = None,
             **arguments: object) -> dict:
        result = self.call("tools/call", {"name": name, "arguments": arguments},
                           bracket=bracket)
        if result.get("isError"):
            raise RuntimeError("\n".join(c.get("text", "")
                                         for c in result.get("content", [])))
        return result

    def text(self, name: str, bracket: dict[str, float] | None = None,
             **arguments: object) -> str:
        return "\n".join(c["text"] for c in self.tool(name, bracket=bracket)["content"]
                         if c.get("type") == "text")

    def resync(self) -> None:
        """Reconnect a driver that the interrupt killed; keep a live one."""
        if self.process is None or self.process.poll() is not None:
            self.close()
            self.start()

    def close(self) -> None:
        process = self.process
        self.process = None
        if process is None:
            return
        try:
            if process.stdin:
                process.stdin.close()
            process.wait(timeout=5)
        except Exception:
            try:
                process.kill()
            except Exception:
                pass


class MemoryReadError(RuntimeError):
    """ReadProcessMemory failed: the client is gone or the pointer is stale."""


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

    def read_exact(self, address: int, size: int) -> bytes:
        data = self.read(address, size)
        if data is None or len(data) != size:
            raise MemoryReadError("read %d bytes at 0x%08x failed" % (size, address))
        return data

    def u32_exact(self, address: int) -> int:
        return struct.unpack("<I", self.read_exact(address, 4))[0]

    def u8_exact(self, address: int) -> int:
        return self.read_exact(address, 1)[0]

    def u32s_exact(self, address: int, count: int) -> list[int]:
        return list(struct.unpack("<%dI" % count, self.read_exact(address, 4 * count)))

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


class ClockFit(NamedTuple):
    intercept: float       # client ms at the epoch wall
    slope: float           # client ms per wall ms (1.0 when drift is not applied)
    fitted_slope: float    # unconstrained weighted least-squares slope
    slope_stderr: float    # weighted standard error of fitted_slope
    span_ms: float         # span of the sample midpoints
    drift_applied: bool


class Clock:
    """Client clock model: offset fitted over bracketed samples.

    One sample brackets a single client clock read between two recorder stamps.
    The read happened somewhere inside the bracket, so its wall time is the
    midpoint and its uncertainty is half the round trip.  Each clock gets a
    weighted least-squares line through all of its samples, weighted by
    ``1 / rtt^2`` so the smallest round trips dominate.  The slope is held at
    1.0 unless the samples span more than ``DRIFT_SPAN_MS``; a shorter run
    cannot separate drift from the frame-quantised read noise, and the fitted
    slope is reported only as an unconstrained estimate with its standard
    error.

    Samples keep arriving while the run goes on, and every row is remapped
    through the final fit when the run ends.  The fit measures how well the
    wall-to-client mapping reproduces the clock reads; the per-run memory-to-UI
    offset is measured separately from matched GCD rises and cast starts.
    """

    DRIFT_SPAN_MS = 60_000.0

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.samples: dict[str, list[tuple[float, float, float]]] = {"system": [], "ui": []}
        self.fits: dict[str, ClockFit | None] = {"system": None, "ui": None}
        self.epoch: float | None = None

    def add(self, kind: str, client_ms: float, wall_before: float,
            wall_after: float) -> None:
        midpoint = (wall_before + wall_after) / 2.0
        rtt = max(0.0, wall_after - wall_before)
        with self.lock:
            self.samples[kind].append((float(client_ms), midpoint, rtt))
            self.fits[kind] = None
            if self.epoch is None:
                self.epoch = midpoint

    def _fit(self, kind: str) -> ClockFit | None:
        """Return the fit for one clock; the caller holds the lock."""
        samples = self.samples[kind]
        if not samples:
            return None
        cached = self.fits[kind]
        if cached is not None:
            return cached
        epoch = self.epoch if self.epoch is not None else samples[0][1]
        weighted = [(client, midpoint, 1.0 / max(rtt, 0.5) ** 2)
                    for client, midpoint, rtt in samples]
        sw = sum(weight for _, _, weight in weighted)
        swx = sum(weight * (midpoint - epoch)
                  for _, midpoint, weight in weighted)
        swy = sum(weight * client for client, _, weight in weighted)
        swxx = sum(weight * (midpoint - epoch) ** 2
                   for _, midpoint, weight in weighted)
        swxy = sum(weight * client * (midpoint - epoch)
                   for client, midpoint, weight in weighted)
        times = [midpoint for _, midpoint, _ in samples]
        span = max(times) - min(times)
        centered = swxx - swx * swx / sw
        if abs(centered) > 1e-12:
            fitted_slope = (sw * swxy - swx * swy) / (sw * centered)
        else:
            fitted_slope = 1.0
        fitted_intercept = (swy - fitted_slope * swx) / sw
        if len(samples) > 2 and centered > 1e-12:
            residual_sq = sum(
                weight * (client - (fitted_intercept + fitted_slope * (midpoint - epoch))) ** 2
                for client, midpoint, weight in weighted)
            slope_stderr = math.sqrt((residual_sq / (len(samples) - 2)) / centered)
        else:
            slope_stderr = math.inf
        if span >= self.DRIFT_SPAN_MS:
            fitted = ClockFit(fitted_intercept, fitted_slope, fitted_slope,
                              slope_stderr, span, True)
        else:
            # client = intercept + (midpoint - epoch); intercept is the
            # weighted mean of client - (midpoint - epoch).
            intercept = sum(weight * (client - (midpoint - epoch))
                            for client, midpoint, weight in weighted) / sw
            fitted = ClockFit(intercept, 1.0, fitted_slope, slope_stderr, span, False)
        self.fits[kind] = fitted
        return fitted

    def _best(self) -> tuple[ClockFit | None, float | None]:
        return (self._fit("ui") or self._fit("system"), self.epoch)

    def estimate(self, wall_ms: float | None = None) -> int:
        wall = ms_now() if wall_ms is None else wall_ms
        with self.lock:
            fit, epoch = self._best()
        if fit is None or epoch is None:
            return int(round(wall))
        return int(round(fit.intercept + fit.slope * (wall - epoch)))

    def wall_for_ui(self, ui_ms: float) -> float:
        with self.lock:
            fit, epoch = self._best()
        if fit is None or epoch is None or abs(fit.slope) < 1e-6:
            return float(ui_ms)
        return epoch + (float(ui_ms) - fit.intercept) / fit.slope

    def wall_for_system(self, system_ms: float) -> float:
        with self.lock:
            fit, epoch = self._fit("system"), self.epoch
        if fit is None or epoch is None or abs(fit.slope) < 1e-6:
            return float(system_ms)
        return epoch + (float(system_ms) - fit.intercept) / fit.slope

    def ui_for_system(self, system_ms: float) -> int:
        with self.lock:
            system_fit = self._fit("system")
            ui_fit = self._fit("ui")
            epoch = self.epoch
        if system_fit is None or ui_fit is None or epoch is None or abs(system_fit.slope) < 1e-6:
            return int(round(float(system_ms)))
        wall = epoch + (float(system_ms) - system_fit.intercept) / system_fit.slope
        return int(round(ui_fit.intercept + ui_fit.slope * (wall - epoch)))

    @property
    def start_client_ms(self) -> float | None:
        with self.lock:
            fit, _ = self._best()
        return float(fit.intercept) if fit is not None else None

    def relative(self, client_ms: float | int | None) -> int | None:
        start = self.start_client_ms
        if client_ms is None or start is None:
            return None
        return int(round(float(client_ms) - start))

    def report(self) -> dict:
        with self.lock:
            epoch = self.epoch
            start = self._best()[0]
            out: dict[str, object] = {
                "epoch_wall_ms": round(epoch, 1) if epoch is not None else None,
                "start_client_ms": round(start.intercept, 1) if start is not None else None,
                "wall_clock": "perf_counter",
                "drift_warning_policy": DRIFT_WARNING_POLICY,
            }
            for kind in ("system", "ui"):
                fit = self._fit(kind)
                samples = list(self.samples[kind])
                if fit is None or epoch is None:
                    out[kind] = None
                    continue
                residuals = [client - (fit.intercept + fit.slope * (mid - epoch))
                             for client, mid, _ in samples]
                absolute = sorted(abs(value) for value in residuals)
                rtts = sorted(rtt for _, _, rtt in samples)
                best_index = min(range(len(samples)), key=lambda i: samples[i][2])
                fitted_ppm = (fit.fitted_slope - 1.0) * 1e6
                out[kind] = {
                    "samples": len(samples),
                    "span_s": round(fit.span_ms / 1000.0, 1),
                    "offset_ms": round(fit.intercept - epoch, 3),
                    "drift_ppm": round((fit.slope - 1.0) * 1e6, 2),
                    "fitted_drift_ppm": round(fitted_ppm, 2),
                    "drift_stderr_ppm": round(fit.slope_stderr * 1e6, 2)
                    if math.isfinite(fit.slope_stderr) else None,
                    "drift_applied": fit.drift_applied,
                    "drift_warning": bool(fit.drift_applied and abs(fitted_ppm) > 200.0),
                    "residual_min_ms": round(absolute[0], 3),
                    "residual_median_ms": round(absolute[len(absolute) // 2], 3),
                    "residual_max_ms": round(absolute[-1], 3),
                    "residual_rms_ms": round(math.sqrt(sum(v * v for v in residuals) / len(residuals)), 3),
                    "residual_is_fit_quality": True,
                    "rtt_min_ms": round(rtts[0], 3),
                    "rtt_median_ms": round(rtts[len(rtts) // 2], 3),
                    "rtt_max_ms": round(rtts[-1], 3),
                    "best_sample_residual_ms": round(residuals[best_index], 3),
                    "best_sample_rtt_ms": round(samples[best_index][2], 3),
                }
        return out


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
            system = ui = None
            system_error = ui_error = None
            system_rtt = ui_rtt = None
            system_bracket: dict[str, float] = {}
            try:
                text = self.mcp.text("client_lua", pid=self.pid, target="system",
                                     code="return System.GetCurrTime()",
                                     bracket=system_bracket)
                system = float(text.strip()) * 1000.0
                before = system_bracket["before"]
                after = system_bracket["after"]
                self.clock.add("system", system, before, after)
                system_rtt = after - before
            except Exception as error:
                before = system_bracket.get("before", ms_now())
                after = system_bracket.get("after", ms_now())
                system_error = str(error)
            ui_bracket: dict[str, float] = {}
            try:
                text = self.mcp.text("client_lua", pid=self.pid, target="ui",
                                     code="local ok,t=pcall(function() return X2Time:GetUiMsec() end) if ok and t then return t end return ''",
                                     bracket=ui_bracket)
                ui_before = ui_bracket["before"]
                ui_after = ui_bracket["after"]
                ui = float(text.strip()) if text.strip() else None
                if ui is not None:
                    self.clock.add("ui", ui, ui_before, ui_after)
                ui_rtt = ui_after - ui_before
            except Exception as error:
                ui_before = ui_bracket.get("before", ms_now())
                ui_after = ui_bracket.get("after", ms_now())
                ui_error = str(error)
            wall = ((ui_before + ui_after) / 2.0 if ui is not None
                    else (before + after) / 2.0)
            self.writer.write("clock_sync", wall_ms=wall, clock_source="external",
                              system_ms=round(system, 1) if system is not None else None,
                              ui_ms=round(ui, 1) if ui is not None else None,
                              system_rtt_ms=round(system_rtt, 3) if system_rtt is not None else None,
                              ui_rtt_ms=round(ui_rtt, 3) if ui_rtt is not None else None,
                              system_error=system_error, ui_error=ui_error)
            if system is None and ui is None:
                log("clock sync failed:", system_error or ui_error)
            self.stop_event.wait(self.interval)

    def stop(self) -> None:
        self.stop_event.set()
        self.join(timeout=5)


def lua_timer(name: str, interval_ms: int, sample: str) -> str:
    return r"""
_G.CS = _G.CS or {}
local key = %s
_G.CS[key..'_rows'] = _G.CS[key..'_rows'] or {}
_G.CS[key..'_stop'] = false
local function tick()
  if _G.CS == nil or _G.CS[key..'_stop'] or _G.CS[key..'_rows'] == nil then return end
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


def take_rows(mcp: Mcp, pid: int, target: str, name: str) -> list[str]:
    """Atomically swap out one Lua timer buffer, so a flush loses nothing."""
    code = ("local c=_G.CS if not c then return '' end "
            "local t=c[%s..'_rows'] if not t then return '' end "
            "c[%s..'_rows']={} return table.concat(t,'\\n')"
            % (json.dumps(name), json.dumps(name)))
    text = mcp.text("client_lua", pid=pid, target=target, code=code)
    return text.splitlines() if text else []


def take_chat(mcp: Mcp, pid: int) -> list[str]:
    code = ("local o=_G.__combat_recorder if not o then return '' end "
            "local t=o.chat if not t then return '' end o.chat={} "
            "return table.concat(t,'\\n')")
    text = mcp.text("client_lua", pid=pid, target="ui", code=code)
    return text.splitlines() if text else []


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
local function flat(v)
  if type(v) == 'table' then
    local parts = {}
    for _, key in ipairs({'x', 'y', 'z'}) do
      if v[key] ~= nil then parts[#parts + 1] = tostring(v[key]) end
    end
    if #parts > 0 then return table.concat(parts, ',') end
    for _, w in ipairs(v) do parts[#parts + 1] = tostring(w) end
    return table.concat(parts, ',')
  end
  return tostring(v)
end
local function origin()
  local ok, a, b, c = pcall(function() return X2Util:GetSeamlessOrigin() end)
  if not ok or a == nil then return '' end
  local parts = { flat(a) }
  if b ~= nil then parts[#parts + 1] = flat(b) end
  if c ~= nil then parts[#parts + 1] = flat(c) end
  return q(table.concat(parts, ';'))
end
return table.concat({
  q(now), pn, pt, pc, pd, tn, tt, tc, td,
  q(get(X2Unit.UnitHealth, 'player')), q(get(X2Unit.UnitMaxHealth, 'player')),
  q(get(X2Unit.UnitMana, 'player')), q(get(X2Unit.UnitMaxMana, 'player')),
  q(get(X2Unit.UnitHealth, 'target')), q(get(X2Unit.UnitMaxHealth, 'target')),
  q(get(X2Unit.UnitIsDead, 'player')), q(get(X2Unit.UnitCombatState, 'player')),
  origin(),
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
    o.chat_original = original
    X2Chat.DispatchChatMessage = function(self, ...)
      add('X2Chat.DispatchChatMessage', ...)
      return original(self, ...)
    end
    o.method_hooked = true
  end
end
o.globals_original = o.globals_original or {}
for _, name in ipairs({'AddMsgToChatWin', 'AddMessageToSysMsgWindow',
                       'OnCreateChatMessage', 'AddMessageTo', 'ShowEffectMsgMessage'}) do
  if not o['global_' .. name] and type(_G[name]) == 'function' then
    local original = _G[name]
    o.globals_original[name] = original
    _G[name] = function(...)
      add(name, ...)
      return original(...)
    end
    o['global_' .. name] = true
  end
end
return 'hooked'
"""


CHAT_UNHOOK = r"""
local o = _G.__combat_recorder
if o then
  if o.method_hooked and o.chat_original ~= nil then
    X2Chat.DispatchChatMessage = o.chat_original
  end
  if o.globals_original then
    for name, original in pairs(o.globals_original) do _G[name] = original end
  end
  o.method_hooked = nil
  o.chat = nil
end
if _G.CS then
  for key in pairs(_G.CS) do
    if type(key) == 'string' and key:sub(-5) == '_stop' then _G.CS[key] = true end
  end
end
Script.SetTimer(250, function()
  _G.CS = nil
  _G.__combat_recorder = nil
end)
return 'removed'
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
                 victim_id: int = 0, controllers: bool = False,
                 interval: float = 0.005, recovery: float = 5.0,
                 refresh_interval: float = 1.0):
        super().__init__(daemon=True)
        self.mem = mem
        self.clock = clock
        self.writer = writer
        self.base = base
        self.actor = actor
        self.victim_unit = victim_unit
        self.victim_id = victim_id
        self.caster_ctrl = caster_ctrl
        self.victim_ctrl = victim_ctrl
        self.controllers = controllers
        self.interval = interval
        self.recovery = recovery
        self.refresh_interval = refresh_interval
        self.stop_event = threading.Event()
        self.gcd_samples = 0
        self.gcd_nonzero = 0
        self.anim_samples = 0
        self.error: str | None = None

    def sample(self) -> None:
        wall = ms_now()
        client = self.clock.estimate(wall)
        actor = self.actor
        if not actor:
            return
        gcd = self.mem.u32s_exact(actor + GCD_REMAINING, 2)
        cast = self.mem.u32s_exact(actor + UNIT_CAST + 4, 4)
        stance = self.mem.u8_exact(actor + UNIT_STANCE)
        deadline = self.mem.u32_exact(actor + UNIT_COMBAT_DEADLINE)
        combat_in = self.mem.u8_exact(actor + UNIT_COMBAT_IN)
        latch_a = self.mem.u8_exact(actor + UNIT_COMBAT_LATCH_A)
        latch_b = self.mem.u8_exact(actor + UNIT_COMBAT_LATCH_B)
        weapon = self.mem.u32_exact(actor + UNIT_WEAPON)
        slots = self.mem.u32s_exact(actor + UNIT_WEAPON_SLOTS, 6)
        visual = self.mem.u8_exact(actor + UNIT_VISUAL_BYTE)
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

    def refresh(self) -> bool:
        """Refresh the pointers; True when the actor address is readable."""
        local = self.mem.u32(self.base + (G_CONNECTION - DUMP_BASE))
        actor = self.mem.u32(local + 0x10C) if local else 0
        if actor and actor != self.actor:
            log("actor moved", hex(self.actor), "->", hex(actor))
            self.actor = actor
            self.caster_ctrl = 0
            if self.controllers:
                self.caster_ctrl, _ = resolve_controller(self.mem, actor, None, "caster")
        if self.victim_id:
            unit = 0
            for key, candidate in walk_units(self.mem, self.base):
                if key == self.victim_id:
                    unit = candidate
                    break
            if unit and unit != self.victim_unit:
                log("victim unit moved", hex(self.victim_unit or 0), "->", hex(unit))
                self.victim_unit = unit
                self.victim_ctrl = 0
                if self.controllers:
                    self.victim_ctrl, _ = resolve_controller(self.mem, unit, None, "victim")
        return bool(actor)

    def recover(self, error: MemoryReadError) -> bool:
        """Wait for the actor to come back; gap rows mark the outage."""
        deadline = time.perf_counter() + self.recovery
        next_mark = 0.0
        while not self.stop_event.is_set() and time.perf_counter() < deadline:
            try:
                valid = self.refresh()
            except Exception:
                valid = False
            if valid:
                try:
                    self.sample()
                    log("sampler resumed after a gap")
                    return True
                except MemoryReadError:
                    pass
            now = time.perf_counter()
            if now >= next_mark:
                self.writer.write("gcd", clock_source="estimate", gap=True,
                                  reason="actor-unavailable", last_error=str(error))
                next_mark = now + 0.25
            sleep_until(min(deadline, time.perf_counter() + 0.05))
        return False

    def run(self) -> None:
        next_time = time.perf_counter()
        next_refresh = next_time + self.refresh_interval
        while not self.stop_event.is_set():
            failure: MemoryReadError | None = None
            try:
                if time.perf_counter() >= next_refresh:
                    next_refresh += self.refresh_interval
                    if not self.refresh():
                        # The old actor address can still read while the
                        # connection global has no actor; never pass those
                        # stale bytes off as fresh data.
                        raise MemoryReadError("local actor is null")
                self.sample()
            except MemoryReadError as error:
                # A respawn or zone change frees the old actor; give the new
                # one a few seconds and mark the gap while waiting.
                if not self.recover(error):
                    if self.stop_event.is_set():
                        return
                    failure = error
            except Exception as error:
                self.error = str(error)
                self.writer.write("gcd", clock_source="external", error=str(error))
                log("sampler error; stopping:", error)
                return
            if failure is not None:
                self.error = str(failure)
                self.writer.write("gcd", clock_source="external", error=str(failure),
                                  read_failed=True)
                log("memory read failed; stopping:", failure)
                return
            next_time += self.interval
            now = time.perf_counter()
            if next_time < now:
                next_time = now
            sleep_until(next_time)

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


def origin_at(origins: list[tuple[int, str]], ui_ms: int) -> str:
    best = ""
    best_delta = None
    for client, value in origins:
        delta = abs(client - ui_ms)
        if best_delta is None or delta < best_delta:
            best_delta = delta
            best = value
    return best


def write_anim_rows(writer: ChannelWriter, rows: list[str], clock: Clock,
                    origins: list[tuple[int, str]]) -> None:
    for parts in parse_lua_rows(rows):
        if len(parts) < 10:
            continue
        try:
            ui = clock.ui_for_system(float(parts[0]))
        except (TypeError, ValueError):
            ui = None
        writer.write("anim_state", client_ms=parts[0], clock_source="system",
                     frame=parts[1], player_state=parts[2], player_clip=parts[3],
                     player_ptr=parts[4], victim_state=parts[5], victim_clip=parts[6],
                     victim_ptr=parts[7], player_pos=parts[8], victim_pos=parts[9],
                     seamless_origin=origin_at(origins, ui) if ui is not None else "")


def write_cast_rows(writer: ChannelWriter, rows: list[str]) -> None:
    for parts in parse_lua_rows(rows):
        if len(parts) < 17:
            continue
        writer.write("castbar", client_ms=parts[0], clock_source="ui",
                     player_name=parts[1], player_total=parts[2], player_current=parts[3],
                     player_show=parts[4], target_name=parts[5], target_total=parts[6],
                     target_current=parts[7], target_show=parts[8],
                     health=parts[9], max_health=parts[10], mana=parts[11],
                     max_mana=parts[12], target_health=parts[13],
                     target_max_health=parts[14], player_dead=parts[15],
                     player_combat=parts[16],
                     seamless_origin=parts[17] if len(parts) > 17 else "")


def write_buff_rows(writer: ChannelWriter, rows: list[str]) -> None:
    for parts in parse_lua_rows(rows):
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


def write_chat_rows(writer: ChannelWriter, rows: list[str]) -> None:
    for line in rows:
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


def drain_lua(mcp: Mcp, pid: int, clock: Clock, writer: ChannelWriter) -> None:
    """Move every buffered Lua channel into its JSONL file right now."""
    batches: dict[str, list[str]] = {}
    for name, target in (("anim", "system"), ("cast", "ui"), ("buff", "ui")):
        try:
            batches[name] = take_rows(mcp, pid, target, name)
        except Exception as error:
            log("flush", name, "failed:", error)
            batches[name] = []
    try:
        chat = take_chat(mcp, pid)
    except Exception as error:
        log("flush chat failed:", error)
        chat = []
    origins: list[tuple[int, str]] = []
    for parts in parse_lua_rows(batches["cast"]):
        if len(parts) > 17 and parts[17]:
            try:
                origins.append((int(float(parts[0])), parts[17]))
            except ValueError:
                pass
    write_anim_rows(writer, batches["anim"], clock, origins)
    write_cast_rows(writer, batches["cast"])
    write_buff_rows(writer, batches["buff"])
    write_chat_rows(writer, chat)


class LuaFlusher(threading.Thread):
    """Drain the Lua buffers every few seconds so a disconnect loses little."""

    def __init__(self, mcp: Mcp, pid: int, clock: Clock, writer: ChannelWriter,
                 interval: float = 5.0):
        super().__init__(daemon=True)
        self.mcp = mcp
        self.pid = pid
        self.clock = clock
        self.writer = writer
        self.interval = interval
        self.stop_event = threading.Event()

    def run(self) -> None:
        while not self.stop_event.wait(self.interval):
            drain_lua(self.mcp, self.pid, self.clock, self.writer)

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


def gcd_runs(gcd_rows: list[dict]) -> list[dict]:
    """Split the GCD rows into runs that start at a rise of ``remaining``."""
    runs: list[dict] = []
    previous = None
    for row in gcd_rows:
        if (row.get("read_failed") or row.get("gap")
                or row.get("gcd_remaining") is None):
            continue
        current = (row.get("gcd_remaining"), row.get("gcd_total"))
        if previous is not None and current[0] > previous[0]:
            if runs and runs[-1]["done"] is None:
                runs[-1]["superseded"] = True
            runs.append({"start": row, "from": list(previous), "to": list(current),
                         "peak": current[0], "total": current[1] or 0,
                         "done": None, "superseded": False})
        elif runs and runs[-1]["done"] is None:
            run = runs[-1]
            run["peak"] = max(run["peak"], current[0])
            if current[0] == 0:
                run["done"] = row
        previous = current
    return runs


def gcd_run_complete(run: dict, slack_ms: float = 250.0) -> bool:
    total = run["total"]
    if total <= 0 or run["done"] is None:
        return False
    tolerance = max(50.0, total * 0.05)
    if not total - tolerance <= run["peak"] <= total + tolerance:
        return False
    duration = run["done"].get("client_ms", 0) - run["start"].get("client_ms", 0)
    return 0 <= duration <= total + slack_ms


def gcd_run_press(run: dict, inputs: list[dict],
                  used: set[int] | None = None) -> tuple[int, dict] | None:
    """The latest press before this rise, within ``PRESS_WINDOW_MS``.

    A press starts at most one rise: ``used`` holds the input indexes already
    matched to an earlier run, so two rises cannot both claim one press.
    """
    start_wall = run["start"].get("wall_ms")
    if start_wall is None:
        return None
    best: tuple[int, dict] | None = None
    best_delta = None
    for index, event in enumerate(inputs):
        if used is not None and index in used:
            continue
        before = event.get("before_wall_ms", event.get("wall_ms"))
        if before is None:
            continue
        delta = start_wall - before
        if 0.0 <= delta <= PRESS_WINDOW_MS and (best_delta is None or delta < best_delta):
            best = (index, event)
            best_delta = delta
    return best


def self_check(writer: ChannelWriter, gcd_rows: list[dict], inputs: list[dict],
               caster_ctrl: int, victim_ctrl: int, ctrl_tried: dict[str, list[str]],
               required: list[str]) -> dict:
    counts = dict(writer.counts)
    missing = [name for name in required if counts.get(name, 0) == 0]
    changes = []
    previous = None
    for row in gcd_rows:
        if (row.get("read_failed") or row.get("gap")
                or row.get("gcd_remaining") is None):
            continue
        current = (row.get("gcd_remaining"), row.get("gcd_total"))
        if previous is not None and current != previous:
            changes.append({"wall_ms": row.get("wall_ms"), "client_ms": row.get("client_ms"),
                            "from": list(previous), "to": list(current)})
        previous = current
    press_ticks = 0
    for event in inputs:
        before = event.get("before_wall_ms", event.get("wall_ms"))
        for change in changes:
            if before is not None and 0.0 <= change["wall_ms"] - before <= PRESS_WINDOW_MS:
                if change["to"][0] <= change["from"][0]:
                    press_ticks += 1
                break
    runs = gcd_runs(gcd_rows)
    complete = [run for run in runs if gcd_run_complete(run)]
    linked: list[tuple[dict, dict]] = []
    used: set[int] = set()
    for run in complete:
        match = gcd_run_press(run, inputs, used)
        if match is not None:
            index, event = match
            used.add(index)
            linked.append((run, event))
    # "Most rises follow a press": at least one and more than half of the
    # complete rises must be press-linked (a tie is not a majority).
    gcd_changed = bool(linked) and len(linked) * 2 > len(complete)
    pressed_runs = []
    for run, event in linked:
        pressed_runs.append({
            "key": event.get("key"),
            "start_client_ms": run["start"].get("client_ms"),
            "end_client_ms": run["done"].get("client_ms"),
            "from": run["from"],
            "to": run["to"],
            "peak": run["peak"],
            "total": run["total"],
            "duration_ms": (run["done"].get("client_ms", 0)
                            - run["start"].get("client_ms", 0)),
        })
    result = {
        "ok": not missing and gcd_changed,
        "channels": counts,
        "required": required,
        "missing": missing,
        "gcd": {
            "changed": gcd_changed,
            "pressed_changes": pressed_runs,
            "pressed_runs": pressed_runs,
            "rises": len(runs),
            "complete_rises": len(complete),
            "linked_complete_rises": len(linked),
            "incomplete_rises": sum(1 for run in runs if run["done"] is None),
            "short_or_slow_rises": sum(1 for run in runs
                                       if run["done"] is not None
                                       and not gcd_run_complete(run)),
            "countdown_ticks_rejected": press_ticks,
            "changes": changes[:200],
            "samples": len(gcd_rows),
        },
        "animations": {
            "controller_lists": bool(caster_ctrl or victim_ctrl),
            "caster_ctrl": caster_ctrl,
            "victim_ctrl": victim_ctrl,
            "tried": ctrl_tried,
        },
    }
    return result
def read_channels(run: Path) -> dict[str, list[dict]]:
    channels: dict[str, list[dict]] = {}
    for path in sorted(run.glob("*.jsonl")):
        rows = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if line:
                rows.append(json.loads(line))
        channels[path.stem] = rows
    return channels


def monotonic_report(channels: dict[str, list[dict]]) -> dict:
    report = {}
    for channel, rows in channels.items():
        previous = None
        backsteps = []
        for row in rows:
            current = row.get("client_ms")
            if previous is not None and current is not None and current < previous:
                backsteps.append({"seq": row.get("seq"), "step_ms": current - previous})
            previous = current if current is not None else previous
        report[channel] = {
            "ok": not backsteps,
            "backsteps": len(backsteps),
            "max_backstep_ms": min((step["step_ms"] for step in backsteps), default=0),
            "first": backsteps[0] if backsteps else None,
        }
    return report


def rate_report(channels: dict[str, list[dict]]) -> dict:
    report = {}
    for channel, rows in channels.items():
        walls = [row.get("wall_ms") for row in rows if row.get("wall_ms") is not None]
        steps = sorted(b - a for a, b in zip(walls, walls[1:]) if b > a)
        entry: dict[str, object] = {"rows": len(rows)}
        if steps:
            entry.update({
                "span_ms": round(walls[-1] - walls[0], 1),
                "median_step_ms": round(steps[len(steps) // 2], 2),
                "p95_step_ms": round(steps[min(len(steps) - 1, int(len(steps) * 0.95))], 2),
                "max_step_ms": round(steps[-1], 2),
            })
        report[channel] = entry
    return report


def _as_float(value: object) -> float | None:
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def cast_start_rows(rows: list[dict]) -> list[dict]:
    """Rows where a player cast begins (name/show becomes active or resets)."""
    starts = []
    previous_active = False
    previous_name = None
    previous_current = 0.0
    for row in rows:
        active = (str(row.get("player_show")).lower() == "true"
                  and bool(row.get("player_name")))
        current = _as_float(row.get("player_current")) or 0.0
        name = row.get("player_name") if active else None
        if active and (not previous_active or name != previous_name
                       or (previous_current > 0 and current <= 0)):
            starts.append(row)
        previous_active = active
        previous_name = name
        previous_current = current if active else 0.0
    return starts


def measure_memory_ui_offset(channels: dict[str, list[dict]],
                             window_ms: float = 150.0,
                             negative_slack_ms: float = 25.0,
                             near_window_ms: float = MEMORY_UI_NEAR_WINDOW_MS) -> dict:
    """Compare each GCD rise with the cast start of the same press.

    Both sides are client-clock times: the cast start is the UI clock the
    client reports, the GCD rise is a memory row mapped through the fitted
    clock.  Their difference is the memory-to-UI offset for this run.  A small
    negative difference is allowed so the same pairs can be re-measured after
    the offset has been applied; the raw memory time (``client_ms_raw``) is
    preferred when present, so that re-measurement still reports the real
    difference instead of the zero the shift builds in.

    An unpaired rise is checked again against every unused cast start within
    ``near_window_ms``.  If one is that close, the ordering does not match the
    same-frame assumption (e.g. the GCD written 300 ms after its cast start)
    and the run is reported as ``unpaired_near_start`` and fails.  Only an
    instant skill with no cast start anywhere near it stays exempt.

    ``measurable`` needs at least one matched pair, but the offset is
    ``confirmed`` only with ``MIN_CONFIRMING_PAIRS`` pairs; one pair is
    reported as ``low_confidence``.  A run with no pairs and no near start
    reports ``not-measurable`` and does not fail.
    """
    gcd_rows = channels.get("gcd", [])
    cast_rows = channels.get("castbar", [])
    rises = []
    previous = None
    for row in gcd_rows:
        if (row.get("read_failed") or row.get("gap")
                or row.get("gcd_remaining") is None):
            continue
        current = row.get("gcd_remaining")
        if (previous is not None and current > previous
                and row.get("client_ms_raw", row.get("client_ms")) is not None):
            rises.append(row)
        previous = current
    starts = cast_start_rows(cast_rows)
    used: set[int] = set()
    paired: set[int] = set()
    pairs: list[dict] = []
    for rise_index, rise in enumerate(rises):
        rise_ms = rise.get("client_ms_raw", rise.get("client_ms"))
        best = None
        for index, start in enumerate(starts):
            if index in used:
                continue
            start_ms = start.get("native_ms")
            if start_ms is None:
                start_ms = start.get("client_ms")
            if start_ms is None:
                continue
            delta = rise_ms - start_ms
            if (-negative_slack_ms <= delta <= window_ms
                    and (best is None or abs(delta) < abs(best[0]))):
                best = (delta, index, start, start_ms)
        if best is not None:
            used.add(best[1])
            paired.add(rise_index)
            pairs.append({
                "cast_start_ms": int(round(best[3])),
                "gcd_rise_ms": int(rise_ms),
                "offset_ms": round(best[0], 1),
                "skill": best[2].get("player_name"),
            })
    unpaired_near_start: list[dict] = []
    for rise_index, rise in enumerate(rises):
        if rise_index in paired:
            continue
        rise_ms = rise.get("client_ms_raw", rise.get("client_ms"))
        best = None
        for index, start in enumerate(starts):
            if index in used:
                continue
            start_ms = start.get("native_ms")
            if start_ms is None:
                start_ms = start.get("client_ms")
            if start_ms is None:
                continue
            delta = rise_ms - start_ms
            if (abs(delta) <= near_window_ms
                    and (best is None or abs(delta) < abs(best[0]))):
                best = (delta, start, start_ms)
        if best is not None:
            unpaired_near_start.append({
                "cast_start_ms": int(round(best[2])),
                "gcd_rise_ms": int(rise_ms),
                "delta_ms": round(best[0], 1),
                "skill": best[1].get("player_name"),
            })
    offsets = [pair["offset_ms"] for pair in pairs]
    median = statistics.median(offsets) if offsets else 0.0
    residuals = sorted(abs(offset - median) for offset in offsets)
    measured = bool(pairs)
    measurable = measured
    confirmed = len(pairs) >= MIN_CONFIRMING_PAIRS
    ordering_mismatch = bool(unpaired_near_start)
    if measured:
        ok = residuals[-1] <= MEMORY_UI_RESIDUAL_MS and not ordering_mismatch
        reason = None
    else:
        ok = not ordering_mismatch
        if not rises and not starts:
            reason = "no-gcd-rises-or-cast-starts"
        elif not rises:
            reason = "no-gcd-rises"
        elif not starts:
            reason = "no-cast-starts"
        else:
            reason = "no-pairs"
    if confirmed:
        status = "confirmed"
    elif measured:
        status = "low-confidence"
    else:
        status = "not-measurable"
    return {
        "assumption": MEMORY_UI_ASSUMPTION,
        "raw_field": "client_ms_raw",
        "status": status,
        "measurable": measurable,
        "measured": measured,
        "confirmed": confirmed,
        "low_confidence": measured and not confirmed,
        "ok": ok,
        "reason": reason,
        "offset_ms": round(median, 2),
        "pairs": len(pairs),
        "gcd_rises": len(rises),
        "cast_starts": len(starts),
        "unpaired_near_start": unpaired_near_start,
        "ordering_mismatch": ordering_mismatch,
        "window_ms": window_ms,
        "negative_slack_ms": negative_slack_ms,
        "near_window_ms": near_window_ms,
        "spread_ms": round(max(offsets) - min(offsets), 2) if offsets else None,
        "stdev_ms": round(statistics.pstdev(offsets), 2) if len(offsets) > 1 else 0.0,
        "residual_after_median_ms": round(statistics.median(residuals), 2)
        if residuals else None,
        "residual_after_max_ms": round(residuals[-1], 2) if residuals else None,
        "samples": pairs,
    }


def shift_estimate_rows(run: Path, offset_ms: float) -> int:
    """Move the memory rows onto the UI clock; returns the row count.

    Only ``gcd`` and ``anim_controller`` come from ReadProcessMemory.  The
    wall-stamped estimate rows (``unit_state``, ``meta``) keep the fitted
    clock, and the unshifted memory time stays in ``client_ms_raw``.  The shift
    is idempotent: an existing ``client_ms_raw`` is never overwritten, so a
    second call with the same offset changes nothing and a call with a new
    offset re-shifts from the original time.
    """
    if not offset_ms:
        return 0
    shifted = 0
    for path in sorted(run.glob("*.jsonl")):
        lines = []
        changed = False
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line:
                continue
            row = json.loads(line)
            if not (row.get("channel") in MEMORY_SHIFT_CHANNELS
                    and row.get("clock") == "estimate"
                    and isinstance(row.get("client_ms"), int)):
                lines.append(json.dumps(row, ensure_ascii=False, separators=(",", ":")))
                continue
            raw = row.get("client_ms_raw")
            if not isinstance(raw, int):
                raw = row["client_ms"]
                row["client_ms_raw"] = raw
            target = int(round(raw - offset_ms))
            delta = target - row["client_ms"]
            if delta:
                row["client_ms"] = target
                if isinstance(row.get("t_ms"), int):
                    row["t_ms"] = int(round(row["t_ms"] + delta))
                shifted += 1
                changed = True
            lines.append(json.dumps(row, ensure_ascii=False, separators=(",", ":")))
        if changed:
            path.write_text(("\n".join(lines) + "\n") if lines else "",
                            encoding="utf-8")
    return shifted


def remap_channels(run: Path, clock: Clock) -> dict:
    """Rewrite every row through the final fit, then align memory to UI."""
    for path in sorted(run.glob("*.jsonl")):
        lines = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line:
                continue
            row = json.loads(line)
            source = row.get("clock")
            native = row.get("native_ms")
            wall = row.get("wall_ms")
            if source == "ui" and native is not None:
                client: int | None = int(round(float(native)))
                wall = clock.wall_for_ui(client)
            elif source == "system" and native is not None:
                client = clock.ui_for_system(float(native))
                wall = clock.wall_for_system(float(native))
            elif source in ("ui", "system"):
                client = row.get("client_ms")
            else:
                client = clock.estimate(wall) if wall is not None else row.get("client_ms")
            if client is not None:
                row["client_ms"] = int(client)
                row["t_ms"] = clock.relative(client)
            if wall is not None:
                row["wall_ms"] = round(float(wall), 1)
            # client_ms is computed afresh above, so any raw time from an
            # earlier shift is stale; shift_estimate_rows adds it again.
            row.pop("client_ms_raw", None)
            lines.append(json.dumps(row, ensure_ascii=False, separators=(",", ":")))
        path.write_text(("\n".join(lines) + "\n") if lines else "", encoding="utf-8")
    memory_ui = measure_memory_ui_offset(read_channels(run))
    memory_ui["applied_offset_ms"] = (memory_ui["offset_ms"]
                                      if memory_ui["measured"] else 0.0)
    memory_ui["applied_rows"] = shift_estimate_rows(run,
                                                    memory_ui["applied_offset_ms"])
    memory_ui["shifted_channels"] = list(MEMORY_SHIFT_CHANNELS)
    memory_ui["post_shift_note"] = (
        "the shifted memory rows report 0 ms against their cast starts by "
        "construction; use client_ms_raw and the spread/residual, never the "
        "post-shift GCD-rise-vs-cast difference, as 1.2 evidence")
    return memory_ui


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
                        default=Path(os.environ.get("AA_CLIENT", "archeage.exe")))
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
    parser.add_argument("--target-range", type=float, default=4.0,
                        help="refuse a target farther than this many metres (pass --move)")
    parser.add_argument("--fight-keys", default="1,1,2,1,3,2,1,1")
    parser.add_argument("--fight-interval", type=float, default=1.1)
    parser.add_argument("--fight-seconds", type=float, default=0.0,
                        help="record this long after the last key (0 = a little over one interval)")
    parser.add_argument("--server-log", type=Path, default=None)
    parser.add_argument("--controller-lists", action="store_true",
                        help="also sample the old controller scheduler lists; needs the "
                             "PlayAnim hook driver extension, off by default")
    parser.add_argument("--keep-client", action="store_true")
    parser.add_argument("--no-self-check", action="store_true")
    parser.add_argument("--required", default="clock_sync,anim_state,gcd,castbar,unit_state,chat,inputs",
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
    flusher = None
    sync = None
    server_offset = 0
    inputs: list[dict] = []
    gcd_rows: list[dict] = []
    player_id = victim_id = 0
    player_name = ""
    caster_ctrl = victim_ctrl = 0
    ctrl_tried: dict[str, list[str]] = {}
    failure: BaseException | None = None
    result: dict[str, object] = {"ok": False}
    memory_ui: dict = {}
    timer_resolution = False
    required = [name.strip() for name in args.required.split(",") if name.strip()]
    started = time.monotonic()
    try:
        winmm = ctypes.WinDLL("winmm")
        winmm.timeBeginPeriod(1)
        timer_resolution = True
    except Exception:
        pass

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

        # Bracket each clock read: the client value lies between the two stamps.
        system, s_before, s_after, s_error = read_clock(mcp, pid, "system")
        ui, u_before, u_after, u_error = read_clock(mcp, pid, "ui")
        if system is not None:
            clock.add("system", system, s_before, s_after)
        if ui is not None:
            clock.add("ui", ui, u_before, u_after)
        wall = ((u_before + u_after) / 2.0 if ui is not None
                else (s_before + s_after) / 2.0)
        writer.write("clock_sync", wall_ms=wall, clock_source="external",
                     system_ms=round(system, 1) if system is not None else None,
                     ui_ms=round(ui, 1) if ui is not None else None,
                     system_rtt_ms=round(s_after - s_before, 3),
                     ui_rtt_ms=round(u_after - u_before, 3),
                     system_error=s_error, ui_error=u_error)

        log("installing chat hook", mcp.text("client_lua", pid=pid, target="ui", code=CHAT_HOOK))
        mcp.text("client_lua", pid=pid, target="ui",
                 code="local o=assert(_G.__combat_recorder) o.chat={} return 1")
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
        victim_id = 0
        state: dict = {}
        if find_template:
            for attempt in range(3):
                mcp.text("client_chat", pid=pid, message=f"/findobject npc {find_template} 5")
                time.sleep(1.2)
                chat_text = mcp.text(
                    "client_lua", pid=pid, target="ui",
                    code="local o=assert(_G.__combat_recorder) return table.concat(o.chat, '\\n')")
                candidates = [int(value) for value in re.findall(r"BcId:\s*(\d+)", chat_text)]
                if not candidates:
                    raise RuntimeError("spawned NPC not found in chat:\n" + chat_text[-4000:])
                for candidate in candidates:
                    try:
                        target = mcp.text("client_target", pid=pid,
                                          unit_id=format(candidate, "X"),
                                          expected_name=args.npc_name)
                        candidate_state = json.loads(
                            mcp.text("client_unit_state", pid=pid, target="target"))
                    except Exception as error:
                        log("target candidate", candidate, "failed", error)
                        continue
                    if (candidate_state.get("present")
                            and candidate_state.get("name") == args.npc_name
                            and int(candidate_state.get("health") or 0) > 0):
                        victim_id = candidate
                        state = candidate_state
                        log("target", target)
                        break
                    log("target candidate", candidate, "not a live", args.npc_name,
                        candidate_state.get("health"))
                if victim_id:
                    break
                time.sleep(1.0)
            if not victim_id:
                raise RuntimeError("no live %r target after three findobject attempts" % args.npc_name)
            distance = float(state.get("distance", 999))
            log("target", args.npc_name, "distance", distance)
            if distance > args.target_range and args.move is None:
                raise RuntimeError("target is out of melee range: %s (pass --move or "
                                   "--target-range)" % distance)

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

        if args.controller_lists:
            pointers = actor_pointers(mcp, pid, player_name, args.npc_name)
            caster_ctrl, tried = resolve_controller(mem, actor, pointers.get(player_name), "caster")
            ctrl_tried["caster"] = tried
            if victim_unit:
                victim_ctrl, tried = resolve_controller(
                    mem, victim_unit, pointers.get(args.npc_name), "victim")
                ctrl_tried["victim"] = tried

        writer.write("meta", clock_source="estimate", kind="start", pid=pid,
                     character=player_name, player_id=player_id, victim_id=victim_id,
                     npc=args.spawn_npc, npc_name=args.npc_name)

        # Lua timers are frame-bound: 1 ms means "every frame" for the two fast
        # channels.  The Python sampler busy-waits for its 5 ms cadence.
        system_code = (SYSTEM_SAMPLE.replace("CHAR_NAME", json.dumps(player_name))
                       .replace("TARGET_NAME", json.dumps(args.npc_name)))
        log("system timer", mcp.text("client_lua", pid=pid, target="system",
                                     code=lua_timer("anim", 1, system_code)))
        log("cast timer", mcp.text("client_lua", pid=pid, target="ui",
                                   code=lua_timer("cast", 1, UI_CAST_SAMPLE)))
        log("buff timer", mcp.text("client_lua", pid=pid, target="ui",
                                   code=lua_timer("buff", 1000, UI_BUFF_SAMPLE)))

        sync = ClockSync(mcp, pid, clock, writer)
        sync.start()
        unit_sampler = UnitStateSampler(mcp, pid, clock, writer)
        unit_sampler.start()
        flusher = LuaFlusher(mcp, pid, clock, writer)
        flusher.start()
        sampler = MemorySampler(mem, clock, writer, base, actor, victim_unit,
                                caster_ctrl, victim_ctrl, victim_id,
                                controllers=args.controller_lists)
        sampler.start()

        keys = [key.strip() for key in args.fight_keys.split(",") if key.strip()]
        wall0 = time.monotonic()
        for index, key in enumerate(keys):
            target_time = wall0 + index * args.fight_interval
            delay = target_time - time.monotonic()
            if delay > 0:
                time.sleep(delay)
            before_client, before_wall, b_after, _ = read_clock(mcp, pid, "system")
            if before_client is not None:
                clock.add("system", before_client, before_wall, b_after)
            try:
                reply = mcp.text("client_input", pid=pid, key=key, duration_ms=80)
            except Exception as error:
                reply = "error: " + str(error)
            after_client, a_before, after_wall, _ = read_clock(mcp, pid, "system")
            if after_client is not None:
                clock.add("system", after_client, a_before, after_wall)
            event = {"key": key, "before_wall_ms": before_wall,
                     "before_client_ms": before_client, "after_client_ms": after_client,
                     "after_wall_ms": after_wall, "result": reply}
            inputs.append(event)
            if before_client is not None:
                writer.write("inputs", client_ms=before_client, clock_source="system",
                             wall_ms=before_wall, key=key, duration_ms=80,
                             before_client_ms=before_client, after_client_ms=after_client,
                             after_wall_ms=after_wall, result=reply)
            log("key", key, "at", round(time.monotonic() - wall0, 2), reply)
        tail = args.fight_seconds or (args.fight_interval + 1.0)
        time.sleep(tail)
    except BaseException as error:
        # KeyboardInterrupt and SystemExit must still run the cleanup and leave
        # a failed self_check; they are re-raised after the finally block.
        failure = error
        log("recording failed:", type(error).__name__, error)
    finally:
        for thread in (flusher, sync, unit_sampler, sampler):
            if thread is not None:
                try:
                    thread.stop()
                except Exception:
                    pass
        if mcp is not None:
            try:
                mcp.resync()
            except Exception as error:
                log("driver reconnect failed:", error)
            if pid:
                for target, name in (("system", "anim"), ("ui", "cast"), ("ui", "buff")):
                    try:
                        stop_timer(mcp, pid, target, name)
                    except Exception:
                        pass
            if clock.epoch is not None:
                try:
                    drain_lua(mcp, pid, clock, writer)
                except Exception as error:
                    log("final drain failed:", error)
            try:
                if sampler is not None and sampler.error and failure is None:
                    failure = RuntimeError("memory sampler: " + sampler.error)
            except Exception:
                pass
            for target in ("ui", "system"):
                try:
                    mcp.text("client_lua", pid=pid, target=target, code=CHAT_UNHOOK)
                except Exception as error:
                    log("client cleanup failed on", target, ":", error)
        writer.close()
        try:
            memory_ui = remap_channels(args.run_dir, clock)
        except Exception as error:
            log("clock remap failed:", error)
            if failure is None:
                failure = error
        if args.server_log and server_offset:
            try:
                data, server_offset = server_slice(args.server_log, server_offset)
                (args.run_dir / "server_slice.log").write_bytes(data)
            except Exception as error:
                log("server slice failed:", error)
        try:
            channels = read_channels(args.run_dir)
            gcd_rows = channels.get("gcd", [])
            quality = {"monotonic": monotonic_report(channels),
                       "rates": rate_report(channels)}
            result = self_check(writer, gcd_rows, inputs, caster_ctrl, victim_ctrl,
                                ctrl_tried, required)
            result.update({
                "pid": pid,
                "character": player_name or None,
                "player_id": player_id,
                "victim_id": victim_id,
                "npc": args.spawn_npc,
                "npc_name": args.npc_name,
                "inputs": inputs,
                "controller_tried": ctrl_tried,
                "start_client_ms": clock.start_client_ms,
                "duration_s": round(time.monotonic() - started, 1),
                "clock": clock.report(),
                "quality": quality,
                "memory_ui": memory_ui,
            })
            if not all(entry["ok"] for entry in quality["monotonic"].values()):
                result["ok"] = False
            if not memory_ui.get("ok", True):
                result["ok"] = False
            if failure is not None:
                result["ok"] = False
                result["error"] = f"{type(failure).__name__}: {failure}"
            end_wall = ms_now()
            # Not a memory row: the same fitted clock as the start row and
            # unit_state, not the shifted memory clock.
            end_client = clock.estimate(end_wall)
            final_counts = dict(writer.counts)
            final_counts["meta"] = final_counts.get("meta", 0) + 1
            end_row = {
                "schema": SCHEMA, "channel": "meta",
                "seq": writer.sequences.get("meta", 0),
                "wall_ms": round(end_wall, 1), "client_ms": end_client,
                "t_ms": clock.relative(end_client), "clock": "estimate",
                "kind": "end", "character": player_name or None,
                "player_id": player_id, "victim_id": victim_id,
                "npc": args.spawn_npc, "npc_name": args.npc_name,
                "channels": final_counts, "self_check_ok": bool(result.get("ok")),
            }
            with (args.run_dir / "meta.jsonl").open("a", encoding="utf-8") as handle:
                handle.write(json.dumps(end_row, ensure_ascii=False,
                                        separators=(",", ":")) + "\n")
            result["channels"] = final_counts
            (args.run_dir / "self_check.json").write_text(
                json.dumps(result, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        except Exception as error:
            log("self-check failed:", error)
            failure = failure or error
            try:
                (args.run_dir / "self_check.json").write_text(
                    json.dumps({"ok": False,
                                "error": f"{type(failure).__name__}: {failure}",
                                "channels": writer.counts}, indent=1) + "\n",
                    encoding="utf-8")
            except Exception:
                pass
        log("channels", json.dumps(writer.counts, sort_keys=True))
        if isinstance(result, dict) and "gcd" in result:
            log("self-check", "ok" if result.get("ok") else "FAILED",
                "missing", result.get("missing"),
                "gcd_changed", result["gcd"]["changed"])
            if isinstance(result.get("memory_ui"), dict):
                info = result["memory_ui"]
                if info.get("measured"):
                    log("memory->UI offset", info["offset_ms"], "ms",
                        "(assumes the GCD is written in the same frame the cast starts)",
                        "spread", info["spread_ms"], "ms",
                        "pairs", info["pairs"],
                        "status", info.get("status"),
                        "residual max", info["residual_after_max_ms"], "ms",
                        "applied", info.get("applied_offset_ms"), "ms")
                    if info.get("low_confidence"):
                        log("memory->UI offset low confidence: only",
                            info["pairs"], "pair(s)")
                else:
                    log("memory->UI offset not measurable:", info.get("reason"),
                        "rises", info.get("gcd_rises"),
                        "cast starts", info.get("cast_starts"))
                for near in info.get("unpaired_near_start") or []:
                    log("memory->UI ordering mismatch: GCD rise",
                        near.get("delta_ms"), "ms from the cast start of",
                        near.get("skill"), "(rise", near.get("gcd_rise_ms"),
                        "cast start", near.get("cast_start_ms"), ")")
            if isinstance(result.get("quality"), dict):
                mono = result["quality"]["monotonic"]
                bad = [name for name, entry in mono.items() if not entry["ok"]]
                log("monotonic", "ok" if not bad else "FAILED on " + ",".join(bad))
                clock_info = result.get("clock") or {}
                for kind in ("system", "ui"):
                    info = clock_info.get(kind)
                    if info:
                        log("clock", kind, "offset", info["offset_ms"], "ms",
                            "drift", info["drift_ppm"], "ppm",
                            "applied", info["drift_applied"],
                            "fitted drift", info["fitted_drift_ppm"],
                            "+-", info["drift_stderr_ppm"], "ppm",
                            "residual max", info["residual_max_ms"], "ms",
                            "rtt min", info["rtt_min_ms"], "ms")
        if pid and not args.keep_client:
            subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
            log("stopped client", pid)
        if mcp is not None:
            mcp.close()
        if timer_resolution:
            try:
                ctypes.WinDLL("winmm").timeEndPeriod(1)
            except Exception:
                pass
    if failure is not None:
        if isinstance(failure, (KeyboardInterrupt, SystemExit)):
            raise failure
        return 1
    if isinstance(result, dict) and (result.get("ok") or args.no_self_check):
        return 0
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
