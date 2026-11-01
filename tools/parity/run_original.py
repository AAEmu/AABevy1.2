#!/usr/bin/env python3
"""Run one parity scenario on the original 1.2 client and record a capture.

The runner starts the client through the lane's ``Scripts/live-stack.sh client``
(once, with one retry only for a startup crash) or attaches to an existing pid.
It hides the client before the first step, drives it only through the
``aaemu-clientdriver`` MCP tools, and stops it by pid at the end unless
``--keep-client`` is given.  It never calls ``X2World:LeaveWorld``.

Outputs, under ``<capture-root>/<scenario>/<run-id>/``:

* ``steps.jsonl``          one row per scenario step, with the client clock;
* ``ui/<step>.json``       the full visible UI tree (reference/tools/snapshot.lua);
* ``entities/<step>.json`` client entities, animation state, unit, vehicle and
                           mode-action state;
* ``shots/<step>.png``     the rendered frame (local only, never committed);
* ``client_packets.jsonl`` the client's sent packets from the server field
                           trace, aligned to the client's own clock;
* ``clock_sync.jsonl``     the bracket samples behind the wall-to-client fit;
* ``meta.json``           everything needed to read the capture later.

The packet clock uses ``CSMoveUnit.time`` and ``Ping.local`` only; a packet
without one of those fields is tagged ``interpolated`` or ``nearest_anchor``.
See tools/parity/README.md.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import math
import os
import queue
import re
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, NamedTuple

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(HERE))

import packet_clock  # noqa: E402
import scenario as scenario_format  # noqa: E402

SCHEMA = "aabevy.parity.capture/v1"
ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect",
              "gameguard", "hackshield")
KEY_ORDER = {str(number): str(number) for number in range(1, 10)}
KEY_ORDER["10"] = "0"
SYSTEM_CLOCK_CODE = "return System.GetCurrTime()"
UI_CLOCK_CODE = ("local ok,t=pcall(function() return X2Time:GetUiMsec() end) "
                 "if ok and t then return t end return ''")
LIB_PATH = ROOT / "reference" / "tools" / "lib.lua"
SNAPSHOT_PATH = ROOT / "reference" / "tools" / "snapshot.lua"
MCP_TIMEOUT_S = 30.0


def log(*values: object) -> None:
    print("parity:", *values, flush=True)


def ms_now() -> float:
    return time.perf_counter_ns() / 1_000_000.0


def iso_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


class StepFailure(RuntimeError):
    """A step error that still carries the partial result to steps.jsonl."""

    def __init__(self, message: str, result: Any) -> None:
        super().__init__(message)
        self.result = result


class ClockFit(NamedTuple):
    intercept: float
    slope: float
    fitted_slope: float
    slope_stderr: float
    span_ms: float
    drift_applied: bool


class Clock:
    """The combat recorder's weighted wall-to-client clock fit, kept small."""

    DRIFT_SPAN_MS = 60_000.0

    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.samples: dict[str, list[tuple[float, float, float]]] = {
            "system": [], "ui": []}
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
        swx = sum(weight * (midpoint - epoch) for _, midpoint, weight in weighted)
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
                weight * (client - (fitted_intercept + fitted_slope
                                    * (midpoint - epoch))) ** 2
                for client, midpoint, weight in weighted)
            slope_stderr = math.sqrt(
                (residual_sq / (len(samples) - 2)) / centered)
        else:
            slope_stderr = math.inf
        if span >= self.DRIFT_SPAN_MS:
            fitted = ClockFit(fitted_intercept, fitted_slope, fitted_slope,
                              slope_stderr, span, True)
        else:
            intercept = sum(weight * (client - (midpoint - epoch))
                            for client, midpoint, weight in weighted) / sw
            fitted = ClockFit(intercept, 1.0, fitted_slope, slope_stderr,
                              span, False)
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
        if (system_fit is None or ui_fit is None or epoch is None
                or abs(system_fit.slope) < 1e-6):
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
                "start_client_ms": round(start.intercept, 1)
                if start is not None else None,
                "wall_clock": "perf_counter",
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
                out[kind] = {
                    "samples": len(samples),
                    "span_s": round(fit.span_ms / 1000.0, 1),
                    "offset_ms": round(fit.intercept - epoch, 3),
                    "drift_ppm": round((fit.slope - 1.0) * 1e6, 2),
                    "fitted_drift_ppm": round((fit.fitted_slope - 1.0) * 1e6, 2),
                    "drift_applied": fit.drift_applied,
                    "residual_median_ms": round(absolute[len(absolute) // 2], 3),
                    "residual_max_ms": round(absolute[-1], 3),
                }
        return out


class Mcp:
    """Newline JSON-RPC client for ``aaemu-clientdriver mcp``."""

    def __init__(self, driver: Path, dll: Path, launcher: Path, client: Path,
                 password_file: Path | None, login: str, user: str,
                 artifact: Path) -> None:
        command = [str(driver), "mcp", "--dll", str(dll), "--client",
                   str(client), "--launcher", str(launcher), "--login", login,
                   "--user", user, "--artifact-dir", str(artifact)]
        if driver.suffix.lower() == ".py":
            command = [sys.executable] + command
        if password_file is not None:
            command += ["--password-file", str(password_file)]
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True, encoding="utf-8",
            creationflags=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0))
        self.sequence = 0
        self.replies: queue.Queue[str | None] = queue.Queue()
        self.reader = threading.Thread(target=self._reader, name="mcp-reader",
                                       daemon=True)
        self.reader.start()
        assert self.process.stdin and self.process.stdout
        try:
            self.call("initialize", {"protocolVersion": "2025-11-25",
                                     "capabilities": {},
                                     "clientInfo": {"name": "parity-original",
                                                    "version": "1"}})
            self.process.stdin.write(json.dumps(
                {"jsonrpc": "2.0",
                 "method": "notifications/initialized"}) + "\n")
            self.process.stdin.flush()
        except BaseException:
            try:
                self.process.kill()
            except Exception:
                pass
            raise

    def _reader(self) -> None:
        assert self.process and self.process.stdout
        try:
            for line in self.process.stdout:
                self.replies.put(line)
        except Exception:
            pass
        finally:
            self.replies.put(None)

    def _read_matching(self, timeout_s: float = MCP_TIMEOUT_S) -> dict:
        for _ in range(32):
            try:
                line = self.replies.get(timeout=timeout_s)
            except queue.Empty:
                raise RuntimeError(
                    f"driver MCP reply timed out after {timeout_s:g} s "
                    f"(id {self.sequence})")
            if line is None:
                raise RuntimeError("driver MCP closed stdout")
            response = json.loads(line)
            if response.get("id") == self.sequence:
                return response
        raise RuntimeError(f"driver MCP sent no reply for id {self.sequence}")

    def call(self, method: str, params: object = None,
             bracket: dict[str, float] | None = None,
             timeout_s: float = MCP_TIMEOUT_S) -> object:
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
        response = self._read_matching(timeout_s)
        if bracket is not None:
            bracket["after"] = ms_now()
        if "error" in response:
            raise RuntimeError(str(response["error"]))
        return response["result"]

    def tool(self, name: str, bracket: dict[str, float] | None = None,
             **arguments: object) -> dict:
        result = self.call("tools/call",
                           {"name": name, "arguments": arguments},
                           bracket=bracket)
        if result.get("isError"):
            raise RuntimeError("\n".join(item.get("text", "")
                                         for item in result.get("content", [])))
        return result

    def text(self, name: str, bracket: dict[str, float] | None = None,
             **arguments: object) -> str:
        return "\n".join(item["text"] for item in
                         self.tool(name, bracket=bracket, **arguments).get(
                             "content", []) if item.get("type") == "text")

    def screenshot(self, pid: int) -> bytes:
        result = self.tool("client_screenshot", pid=pid)
        text = "\n".join(item["text"] for item in result.get("content", [])
                         if item.get("type") == "text").strip()
        if text and Path(text).exists():
            return Path(text).read_bytes()
        for item in result.get("content", []):
            if item.get("type") == "image" and item.get("data"):
                return base64.b64decode(item["data"])
        raise RuntimeError("client_screenshot returned no PNG")

    def close(self) -> None:
        try:
            assert self.process.stdin
            self.process.stdin.close()
            self.process.wait(timeout=15)
        except Exception:
            self.process.kill()
        if self.process.stdout is not None:
            try:
                self.process.stdout.close()
            except Exception:
                pass
        self.reader.join(timeout=2.0)


class FixtureMcp:
    """Replay recorded MCP replies for offline tests."""

    def __init__(self, fixture: Path) -> None:
        data = json.loads(Path(fixture).read_text(encoding="utf-8"))
        self.rules = [dict(rule) for rule in data.get("rules", [])]
        self.defaults = dict(data.get("defaults", {}))
        self.calls: list[dict] = []

    def _match(self, name: str, arguments: dict) -> dict:
        haystack = json.dumps(arguments, ensure_ascii=False)
        for rule in self.rules:
            if rule.get("tool") != name:
                continue
            if "code_contains" in rule and rule["code_contains"] not in haystack:
                continue
            if ("message_contains" in rule
                    and rule["message_contains"] not in haystack):
                continue
            if "key" in rule and rule["key"] != arguments.get("key"):
                continue
            if "slot" in rule and rule["slot"] != arguments.get("slot"):
                continue
            if "target" in rule and rule["target"] != arguments.get("target"):
                continue
            if rule.get("consume", True):
                self.rules.remove(rule)
            return rule
        return {"tool": name, "reply": self.defaults.get(name, {"text": ""})}

    def call(self, method: str, params: object = None,
             bracket: dict[str, float] | None = None) -> object:
        if method == "initialize":
            return {"protocolVersion": "2025-11-25", "capabilities": {},
                    "serverInfo": {"name": "fixture", "version": "1"}}
        if method == "notifications/initialized":
            return None
        if method != "tools/call":
            raise RuntimeError(f"fixture does not implement {method}")
        assert isinstance(params, dict)
        name = str(params.get("name"))
        arguments = dict(params.get("arguments") or {})
        self.calls.append({"name": name, "arguments": arguments})
        rule = self._match(name, arguments)
        if "values" in rule:
            index = int(rule.get("_index", 0))
            values = rule["values"]
            text = values[min(index, len(values) - 1)] if values else ""
            rule["_index"] = index + 1
            reply = {"text": text}
        else:
            reply = rule.get("reply", {"text": ""})
        if isinstance(reply, str):
            return {"content": [{"type": "text", "text": reply}],
                    "isError": False}
        if "content" in reply:
            return reply
        if "text" in reply:
            return {"content": [{"type": "text", "text": reply["text"]}],
                    "isError": False}
        if "json" in reply:
            return {"content": [{"type": "text",
                                 "text": json.dumps(reply["json"])}],
                    "isError": False}
        if "png_base64" in reply:
            return {"content": [{"type": "image", "mimeType": "image/png",
                                 "data": reply["png_base64"]}], "isError": False}
        raise RuntimeError(f"bad fixture reply for {name}: {reply!r}")

    def tool(self, name: str, bracket: dict[str, float] | None = None,
             **arguments: object) -> dict:
        result = self.call("tools/call", {"name": name, "arguments": arguments},
                           bracket=bracket)
        assert isinstance(result, dict)
        if result.get("isError"):
            raise RuntimeError("\n".join(item.get("text", "")
                                         for item in result.get("content", [])))
        return result

    def text(self, name: str, bracket: dict[str, float] | None = None,
             **arguments: object) -> str:
        return "\n".join(item["text"] for item in
                         self.tool(name, bracket=bracket, **arguments).get(
                             "content", []) if item.get("type") == "text")

    def screenshot(self, pid: int) -> bytes:
        result = self.tool("client_screenshot", pid=pid)
        for item in result.get("content", []):
            if item.get("type") == "image":
                return base64.b64decode(item["data"])
            if item.get("type") == "text" and Path(item["text"]).exists():
                return Path(item["text"]).read_bytes()
        raise RuntimeError("fixture screenshot has no image")

    def close(self) -> None:
        return None


def read_clock(mcp: Any, pid: int, target: str) -> tuple[
        float | None, float, float, str | None]:
    bracket: dict[str, float] = {}
    try:
        if target == "system":
            text = mcp.text("client_lua", pid=pid, target="system",
                            code=SYSTEM_CLOCK_CODE, bracket=bracket)
            value = float(text.strip()) * 1000.0
        else:
            text = mcp.text("client_lua", pid=pid, target="ui",
                            code=UI_CLOCK_CODE, bracket=bracket)
            value = float(text.strip()) if text.strip() else None
        return (value, bracket.get("before", ms_now()),
                bracket.get("after", ms_now()), None)
    except Exception as error:
        return (None, bracket.get("before", ms_now()),
                bracket.get("after", ms_now()), str(error))


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=1, sort_keys=True,
                               ensure_ascii=False) + "\n", encoding="utf-8")


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def load_lane(lanes_dir: Path, lane: str) -> dict[str, str]:
    path = lanes_dir / f"{lane}.env"
    if not path.exists():
        raise RuntimeError(f"no lane file {path}")
    values: dict[str, str] = {}
    for line in read_text(path).splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        values[key.strip()] = value.strip()
    return values


def git_info(repo: Path | None) -> dict:
    if repo is None or not (repo / ".git").exists():
        return {"repo": str(repo) if repo else None, "commit": None,
                "branch": None}
    def run(*args: str) -> str | None:
        try:
            result = subprocess.run(["git", "-C", str(repo), *args],
                                    capture_output=True, text=True, timeout=10)
            return result.stdout.strip() if result.returncode == 0 else None
        except Exception:
            return None
    return {"repo": str(repo), "commit": run("rev-parse", "HEAD"),
            "branch": run("rev-parse", "--abbrev-ref", "HEAD")}


def lane_client_pid(login_port: int) -> int | None:
    command = (
        "Get-CimInstance Win32_Process -Filter \"Name='archeage.exe'\" | "
        "Where-Object { $_.CommandLine -match '-auth_port %d( |$)' } | "
        "ForEach-Object { $_.ProcessId }" % login_port)
    try:
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command", command],
            capture_output=True, text=True, timeout=20)
    except Exception:
        return None
    for line in result.stdout.splitlines():
        line = line.strip()
        if line.isdigit():
            return int(line)
    return None


def start_lane_client(lane: str, lanes_dir: Path, server_dir: Path | None,
                      timeout_s: float, output_path: Path,
                      stable_s: float = 1.5) -> tuple[int, dict]:
    info = load_lane(lanes_dir, lane)
    login_port = int(info.get("PORT_PREFIX", "21") + "237")
    repo = server_dir
    if repo is None and info.get("WT"):
        repo = Path(info["WT"])
    if repo is None:
        raise RuntimeError("lane start needs the server worktree (lane WT or "
                           "--server-dir)")
    script = repo / "Scripts" / "live-stack.sh"
    if not script.exists():
        raise RuntimeError(f"missing {script}")
    live = Path(info.get("LIVE", "")) if info.get("LIVE") else None
    attempts: list[dict] = []
    for attempt in range(2):
        existing = lane_client_pid(login_port)
        if existing:
            attempts.append({"attempt": attempt + 1,
                             "result": "already running", "pid": existing})
            return existing, {"attempts": attempts, "source": "existing",
                              "lane": lane, "live": str(live) if live else None}
        environment = dict(os.environ)
        environment["AAEMU_LANE"] = lane
        output_path.parent.mkdir(parents=True, exist_ok=True)
        handle = output_path.open("a", encoding="utf-8")
        started = time.monotonic()
        process = subprocess.Popen(
            ["bash", str(script), "client"], cwd=str(repo),
            env=environment, stdout=handle, stderr=subprocess.STDOUT,
            text=True)
        deadline = started + timeout_s
        seen_pid: int | None = None
        first_seen = 0.0
        died = False
        pid: int | None = None
        while time.monotonic() < deadline:
            pid = lane_client_pid(login_port)
            now = time.monotonic()
            if pid is not None:
                if pid != seen_pid:
                    seen_pid = pid
                    first_seen = now
                if now - first_seen >= stable_s:
                    break
            else:
                if seen_pid is not None:
                    died = True
                    pid = None
                    break
            time.sleep(min(0.5, max(0.0, deadline - now)))
        handle.close()
        output = (output_path.read_text(encoding="utf-8", errors="replace")
                  if output_path.exists() else "")
        attempts.append({
            "attempt": attempt + 1,
            "pid": pid if pid is not None else seen_pid,
            "died": died,
            "seconds": round(time.monotonic() - started, 1),
            "returncode": process.poll(),
            "output_tail": output[-1200:],
        })
        if pid is not None:
            return pid, {"attempts": attempts, "source": "lane", "lane": lane,
                         "live": str(live) if live else None,
                         "_process": process}
        if process.poll() is None:
            try:
                process.terminate()
                process.wait(timeout=10)
            except Exception:
                pass
        if died and attempt == 0:
            adopted = lane_client_pid(login_port)
            if adopted:
                attempts.append({"attempt": 2, "result": "already running",
                                 "pid": adopted})
                return adopted, {"attempts": attempts, "source": "existing",
                                 "lane": lane,
                                 "live": str(live) if live else None}
            log("client pid died during startup, retrying once")
            continue
        raise RuntimeError(
            "client did not start within %g s:\n%s"
            % (timeout_s, output[-3000:]))
    raise RuntimeError("client did not start")


def loaded_anti_cheat(pid: int) -> str | None:
    command = (
        "(Get-Process -Id %d -ErrorAction SilentlyContinue).Modules | "
        "ForEach-Object { $_.ModuleName }" % pid)
    try:
        result = subprocess.run(
            ["powershell", "-NoProfile", "-Command", command],
            capture_output=True, text=True, timeout=20)
    except Exception:
        return None
    for line in result.stdout.splitlines():
        name = line.strip().lower()
        if any(token in name for token in ANTI_CHEAT):
            return line.strip()
    return None


def stop_pid(pid: int) -> None:
    subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)


def attach_and_hide(mcp: Any, pid: int, stop_on_failure: bool) -> str:
    mcp.text("client_attach", pid=pid)
    mcp.text("client_window", pid=pid, visible=False)
    status = mcp.text("client_status", pid=pid)
    if "hidden=1" not in status:
        if stop_on_failure:
            stop_pid(pid)
            raise RuntimeError(
                f"client could not be hidden; stopped pid {pid}: {status}")
        raise RuntimeError(
            f"client could not be hidden; left pid {pid} running: {status}")
    return status


def lua_file(mcp: Any, pid: int, path: Path, target: str) -> str:
    return mcp.text("client_lua", pid=pid, target=target,
                    code=read_text(path))


def load_ui_library(mcp: Any, pid: int) -> None:
    if not LIB_PATH.exists():
        raise RuntimeError(f"missing {LIB_PATH}")
    for target in ("ui", "system"):
        lua_file(mcp, pid, LIB_PATH, target)


def ui_snapshot(mcp: Any, pid: int) -> dict:
    if not SNAPSHOT_PATH.exists():
        raise RuntimeError(f"missing {SNAPSHOT_PATH}")
    text = lua_file(mcp, pid, SNAPSHOT_PATH, "ui").strip()
    return json.loads(text)


def current_stage(mcp: Any, pid: int, timeout_s: float = 240.0) -> str:
    deadline = time.monotonic() + timeout_s
    value = ""
    while time.monotonic() < deadline:
        try:
            value = mcp.text(
                "client_lua", pid=pid, target="ui",
                code="return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()"
            ).strip()
        except Exception:
            value = ""
        if value in ("0", "3", "4", "5"):
            return value
        time.sleep(0.5)
    raise RuntimeError("login stage did not become readable: " + value)


def wait_stage(mcp: Any, pid: int, wanted: tuple[str, ...],
               timeout_s: float) -> str:
    deadline = time.monotonic() + timeout_s
    last = ""
    while time.monotonic() < deadline:
        try:
            last = mcp.text(
                "client_lua", pid=pid, target="ui",
                code="return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()"
            ).strip()
            if last in wanted:
                return last
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError(f"stage {'/'.join(wanted)} not reached: {last}")


def wait_player(mcp: Any, pid: int, timeout_s: float) -> str:
    deadline = time.monotonic() + timeout_s
    last = ""
    while time.monotonic() < deadline:
        try:
            state = mcp.text(
                "client_lua", pid=pid, target="ui",
                code=("return X2LoginCharacter:GetCurrentStage(), "
                      "X2Unit and X2Unit:UnitName('player')"))
            stage, _, name = state.partition("\t")
            if stage.strip() == "0" and name.strip() not in ("", "nil"):
                return name.strip()
            last = state
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError("player did not appear: " + last)


def create_character(mcp: Any, pid: int, name: str, ability: int) -> None:
    lua = lambda code: mcp.text("client_lua", pid=pid, target="ui", code=code)
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
    lua("SelectAbility(%d) step3.content.nameWnd.characterName:SetText(%s) "
        "CreateCharacter(false)" % (ability, json.dumps(name)))
    wait_stage(mcp, pid, ("4",), 180)


def ensure_world(mcp: Any, pid: int, character: str, ability: int,
                 create: bool, timeout_s: float = 240.0) -> str:
    stage = current_stage(mcp, pid, timeout_s)
    if stage == "3":
        mcp.text("client_lua", pid=pid, target="ui",
                 code="X2World:EnterWorld(1,0) return 1")
        stage = wait_stage(mcp, pid, ("4", "5"), timeout_s)
    if stage == "5":
        if not create:
            raise RuntimeError("the account has no character; pass --create-character")
        create_character(mcp, pid, character, ability)
        stage = "4"
    if stage == "4":
        lua = lambda code: mcp.text("client_lua", pid=pid, target="ui", code=code)
        names = []
        for index in range(1, 6):
            value = lua("return X2LoginCharacter:GetLoginCharacterName(%d)"
                        % index).strip()
            if value and value != "nil":
                names.append(value)
        if character not in names:
            if not create:
                raise RuntimeError(
                    f"character {character!r} not in {names}; pass --create-character")
            create_character(mcp, pid, character, ability)
            names = []
            for index in range(1, 6):
                value = lua("return X2LoginCharacter:GetLoginCharacterName(%d)"
                            % index).strip()
                if value and value != "nil":
                    names.append(value)
            if character not in names:
                raise RuntimeError(
                    f"created character {character!r} not in {names}")
        lua("X2LoginCharacter:SelectCharacter(%d)"
            % (names.index(character) + 1))
        lua("characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')")
    return wait_player(mcp, pid, timeout_s)


HOOK_LUA = r"""
_G.__parity = _G.__parity or { chat = {} }
local o = _G.__parity
local function esc(v)
  if v == nil then return '' end
  return (tostring(v):gsub('[\t\r\n]', ' '))
end
local function add(name, ...)
  if #o.chat >= 4096 then return end
  local parts = { name }
  for i = 1, select('#', ...) do parts[#parts + 1] = esc((select(i, ...))) end
  o.chat[#o.chat + 1] = table.concat(parts, '\t')
end
if type(X2Chat) == 'table' and type(X2Chat.DispatchChatMessage) == 'function'
    and not o.chat_hooked then
  local original = X2Chat.DispatchChatMessage
  o.chat_original = original
  X2Chat.DispatchChatMessage = function(self, ...)
    add('X2Chat.DispatchChatMessage', ...)
    return original(self, ...)
  end
  o.chat_hooked = true
end
o.globals_original = o.globals_original or {}
for _, name in ipairs({'AddMsgToChatWin', 'AddMessageToSysMsgWindow',
                       'OnCreateChatMessage', 'AddMessageTo'}) do
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

UNHOOK_LUA = r"""
local o = _G.__parity
if o then
  if o.chat_hooked and o.chat_original ~= nil then
    X2Chat.DispatchChatMessage = o.chat_original
  end
  if o.globals_original then
    for name, original in pairs(o.globals_original) do _G[name] = original end
  end
end
_G.__parity = nil
return 'removed'
"""

ANIMATION_LUA = r"""
local player_name = PLAYER_NAME
local target_name = TARGET_NAME
local function q(v)
  if v == nil then return '' end
  return (tostring(v):gsub('[|\r\n]', ' '))
end
local function flat(v)
  if type(v) == 'table' then
    return q(v.x) .. ',' .. q(v.y) .. ',' .. q(v.z)
  end
  return q(v)
end
local me, target, best = nil, nil, nil
for _, x in pairs(System.GetEntities() or {}) do
  local ok, name = pcall(function() return x:GetName() end)
  if ok and name == player_name then me = x end
  if ok and name == target_name and me ~= nil then
    local okp, p = pcall(function() return x:GetWorldPos() end)
    local okq, qpos = pcall(function() return me:GetWorldPos() end)
    if okp and okq and p and qpos then
      local dx, dy, dz = p.x - qpos.x, p.y - qpos.y, p.z - qpos.z
      local d = dx * dx + dy * dy + dz * dz
      if best == nil or d < best then best = d; target = x end
    end
  end
end
local function anim(x)
  if x == nil or x.actor == nil then
    return { state = nil, clip_time_error = 'no actor' }
  end
  local state, clip, error_text
  pcall(function() state = x.actor:GetCurrentAnimationState() end)
  local ok, value = pcall(function() return x.actor:GetAnimationTime(0, 0) end)
  if ok then clip = value else error_text = tostring(value) end
  return { state = state, clip_time = clip, clip_time_error = error_text }
end
local out = { player = anim(me), target = anim(target),
  player_pos = me and flat(me:GetWorldPos()) or nil,
  target_pos = target and flat(target:GetWorldPos()) or nil }
return __ref.json(out)
"""

MODE_SLOT_LUA = r"""
local slot = SLOT
local count = X2Unit:GetModeActionsCount() or 0
assert(slot <= math.min(count, 10), 'mode slot is not active')
local b = modeSkillActionBar and modeSkillActionBar.buttons and
  modeSkillActionBar.buttons[slot]
assert(b, 'mode action button is missing')
local x, y = b:GetEffectiveOffset()
local width, height = b:GetEffectiveExtent()
return __ref.json({ slot = slot, can_use = b.canUse,
  binded_type = b:GetBindedType(), hotkey = b:GetHotKey(),
  x = x, y = y, width = width, height = height })
"""

ACTION_SCAN_LUA = r"""
local want = tostring(WANT)
local found = nil
local rows = {}
for slot = 1, 10 do
  local typ = ''
  pcall(function() typ = tostring(X2Action:GetActionType(slot)) end)
  local id = ''
  local ok, t = pcall(X2Action.GetActionTooltipText, X2Action, slot)
  if ok and type(t) == 'table' and t.type ~= nil then id = tostring(t.type) end
  rows[#rows + 1] = slot .. ':' .. typ .. ':' .. id
  if found == nil and id == want then found = slot end
end
return table.concat(rows, '|') .. '#' .. tostring(found)
"""

WIDGET_RECT_LUA = r"""
local w = WIDGET
assert(w, 'widget not found')
local x, y = w:GetEffectiveOffset()
local width, height = w:GetEffectiveExtent()
return x .. ',' .. y .. ',' .. width .. ',' .. height
"""

PLAYER_POS_LUA = r"""
local t = { X2Unit:GetUnitWorldPosition('player') }
if #t >= 3 then return t[1] .. ',' .. t[2] .. ',' .. t[3] end
return ''
"""

CONDITION_LUA = r"""
local combat, dead = '', ''
pcall(function() combat = tostring(X2Unit:UnitCombatState('player')) end)
pcall(function() dead = tostring(X2Unit:UnitIsDead('player')) end)
return combat .. '\t' .. dead
"""


def parse_json_text(text: str) -> Any:
    text = text.strip()
    if not text:
        return None
    return json.loads(text)


def json_or_error(mcp: Any, tool: str, **arguments: object) -> dict:
    try:
        return parse_json_text(mcp.text(tool, **arguments)) or {}
    except Exception as error:
        return {"error": f"{type(error).__name__}: {error}"}


def read_chat(mcp: Any, pid: int) -> list[str]:
    try:
        text = mcp.text("client_lua", pid=pid, target="ui",
                        code="local o=_G.__parity "
                             "if not o then return '' end "
                             "return table.concat(o.chat or {}, '\\n')")
    except Exception:
        return []
    return [line for line in text.splitlines() if line.strip()]


def unit_state(mcp: Any, pid: int, unit: str) -> dict:
    state = json_or_error(mcp, "client_unit_state", pid=pid, target=unit)
    state["unit"] = unit
    return state


def read_entities(mcp: Any, pid: int, limit: int = 200) -> list[dict]:
    payload = json_or_error(mcp, "client_entities", pid=pid, limit=limit)
    rows = []
    for key, value in (payload.get("entities") or {}).items():
        row = dict(value)
        row["engine_id"] = key
        rows.append(row)
    return rows


def read_animation(mcp: Any, pid: int, player_name: str,
                   target_name: str | None) -> dict:
    code = (ANIMATION_LUA
            .replace("PLAYER_NAME", json.dumps(player_name))
            .replace("TARGET_NAME", json.dumps(target_name or "")))
    try:
        return parse_json_text(mcp.text(
            "client_lua", pid=pid, target="system", code=code)) or {}
    except Exception as error:
        return {"error": f"{type(error).__name__}: {error}"}


def condition_state(mcp: Any, pid: int) -> dict:
    player = unit_state(mcp, pid, "player")
    target = unit_state(mcp, pid, "target")
    vehicle = json_or_error(mcp, "client_vehicle", pid=pid, action="status")
    combat = ""
    dead = ""
    try:
        text = mcp.text("client_lua", pid=pid, target="ui",
                        code=CONDITION_LUA)
        combat, _, dead = text.partition("\t")
    except Exception:
        pass
    for name, value in (("combat", combat), ("dead", dead)):
        if value in ("true", "false"):
            player[name] = value == "true"
    if not isinstance(vehicle, dict):
        vehicle = {"error": "not an object"}
    chat = read_chat(mcp, pid)
    return {"player": player, "target": target, "vehicle": vehicle,
            "chat": {"last": chat[-1] if chat else None, "count": len(chat)}}


def wait_condition(mcp: Any, pid: int, condition: dict,
                   clock: Clock | None = None) -> dict:
    deadline = time.monotonic() + condition["timeout_ms"] / 1000.0
    polls = 0
    met = False
    last: dict = {}
    while time.monotonic() <= deadline:
        polls += 1
        last = condition_state(mcp, pid)
        met = scenario_format.condition_holds(condition, last)
        if met:
            break
        time.sleep(condition["poll_ms"] / 1000.0)
    result = {"met": met, "polls": polls, "state": last,
              "condition": condition["expr"], "source": condition["source"],
              "met_client_ms": None, "met_ui_ms": None, "met_t_ms": None}
    if met and clock is not None:
        client, wall_before, wall_after, error = read_clock(
            mcp, pid, "system")
        if client is not None:
            clock.add("system", client, wall_before, wall_after)
            ui = clock.ui_for_system(client)
            result.update(met_client_ms=client, met_ui_ms=ui,
                          met_t_ms=clock.relative(ui), met_clock_error=error)
    return result


def widget_rect(mcp: Any, pid: int, path: str) -> tuple[float, float, float, float]:
    text = mcp.text("client_lua", pid=pid, target="ui",
                    code=WIDGET_RECT_LUA.replace("WIDGET", path))
    values = [float(part) for part in text.replace("\t", ",").split(",")]
    if len(values) != 4:
        raise RuntimeError(f"widget {path} rect: {text!r}")
    return values[0], values[1], values[2], values[3]


def click_widget(mcp: Any, pid: int, path: str, action: str = "click") -> dict:
    x, y, width, height = widget_rect(mcp, pid, path)
    point = (int(round(x + width / 2.0)), int(round(y + height / 2.0)))
    point = (max(0, min(4095, point[0])), max(0, min(4095, point[1])))
    reply = mcp.text("client_pointer", pid=pid, action=action,
                     x=point[0], y=point[1])
    return {"widget": path, "rect": [x, y, width, height],
            "point": list(point), "reply": reply}


def player_position(mcp: Any, pid: int) -> list[float] | None:
    try:
        text = mcp.text("client_lua", pid=pid, target="ui",
                        code=PLAYER_POS_LUA).strip()
        values = [float(value) for value in text.split(",")]
        return values[:3] if len(values) >= 3 else None
    except Exception:
        return None


def entity_position(entity: dict) -> list[float] | None:
    position = entity.get("position")
    if isinstance(position, dict):
        return [float(position.get("x", 0.0)), float(position.get("y", 0.0)),
                float(position.get("z", 0.0))]
    if isinstance(position, list) and len(position) >= 3:
        return [float(position[0]), float(position[1]), float(position[2])]
    return None


def distance(left: list[float] | None, right: list[float] | None) -> float:
    if left is None or right is None:
        return math.inf
    return math.sqrt(sum((a - b) ** 2 for a, b in zip(left, right)))


def target_findobject(mcp: Any, pid: int, spec: dict) -> dict:
    name = spec.get("name")
    template = spec["template"]
    radius = spec.get("radius", 30.0)
    deadline = time.monotonic() + spec["timeout_ms"] / 1000.0
    seen: set[int] = set()
    while time.monotonic() <= deadline:
        mcp.text("client_chat", pid=pid,
                 message=f"/findobject npc {template} {radius:g}")
        time.sleep(0.8)
        for line in read_chat(mcp, pid):
            match = re.search(r"BcId:\s*(\d+)", line)
            if not match:
                continue
            candidate = int(match.group(1))
            if candidate in seen:
                continue
            seen.add(candidate)
            expected = name
            if expected is None:
                try:
                    expected = mcp.text(
                        "client_lua", pid=pid, target="ui",
                        code=("local i=X2Unit:GetUnitInfoById('%X') "
                              "return i and tostring(i.name) or ''"
                              % candidate)).strip()
                except Exception:
                    expected = ""
            if not expected:
                continue
            try:
                mcp.text("client_target", pid=pid,
                         unit_id=format(candidate, "X"), expected_name=expected)
                state = unit_state(mcp, pid, "target")
            except Exception:
                continue
            if (state.get("present") and state.get("name") == expected
                    and int(state.get("health") or 0) > 0):
                return {"method": "findobject", "template": template,
                        "unit_id": format(candidate, "X"), "state": state,
                        "candidates": sorted(seen)}
        time.sleep(0.5)
    raise RuntimeError(
        f"no live target for template {template} name {name!r} "
        f"(candidates {sorted(seen)})")


def target_screen_pick(mcp: Any, pid: int, spec: dict) -> dict:
    name = spec.get("name")
    if not name:
        raise RuntimeError("screen pick needs a target name")
    entries = [entry for entry in read_entities(mcp, pid)
               if name in str(entry.get("name", ""))]
    if not entries:
        raise RuntimeError(f"no entity named {name!r} in client_entities")
    here = player_position(mcp, pid)
    entries.sort(key=lambda entry: distance(here, entity_position(entry)))
    entity = entries[0]
    position = entity_position(entity)
    if position is None:
        raise RuntimeError(f"entity {name!r} has no position")
    try:
        text = mcp.text(
            "client_lua", pid=pid, target="ui",
            code="return X2Util:ConvertWorldToScreen(%.6f,%.6f,%.6f)"
                 % tuple(position))
        parts = [float(part) for part in text.replace(",", " ").split()]
        first = (int(round(parts[0])), int(round(parts[1])))
    except Exception:
        first = (960, 540)
    try:
        size = mcp.text(
            "client_lua", pid=pid, target="ui",
            code="return UIParent:GetScreenWidth(), UIParent:GetScreenHeight()")
        width, height = [int(float(part)) for part in size.split()[:2]]
    except Exception:
        width, height = 1920, 1080
    points = [first]
    for delta in (-40, 40, -80, 80, -120, 120, -160, 160):
        points.append((first[0], first[1] + delta))
    for fraction in (1 / 3.0, 1 / 2.0, 2 / 3.0):
        points.append((width // 2, int(height * fraction)))
    attempts = []
    for x, y in points:
        x = max(0, min(4095, x))
        y = max(0, min(4095, y))
        try:
            mcp.text("client_lua", pid=pid, target="ui",
                     code="pcall(function() X2Unit:ReleaseTarget() end) return 1")
            reply = mcp.text("client_pointer", pid=pid, action="right_click",
                             x=x, y=y)
            time.sleep(0.25)
            state = unit_state(mcp, pid, "target")
        except Exception as error:
            attempts.append({"point": [x, y], "error": str(error)})
            continue
        attempts.append({"point": [x, y], "reply": reply,
                         "target": state.get("name")})
        if state.get("present") and name in str(state.get("name", "")):
            return {"method": "screen_pick", "point": [x, y],
                    "state": state, "attempts": attempts,
                    "entity": {"event": entity.get("engine_id"),
                               "position": position}}
    raise RuntimeError(f"screen pick did not select {name!r}: {attempts}")


def target_unit(mcp: Any, pid: int, spec: dict) -> dict:
    if spec.get("template") is not None:
        return target_findobject(mcp, pid, spec)
    return target_screen_pick(mcp, pid, spec)


def use_skill(mcp: Any, pid: int, spec: dict) -> dict:
    bar = spec["bar"]
    slot = spec["slot"]
    skill_id = spec["id"]
    if bar == "mode":
        data = json_or_error(mcp, "client_mode_actions", pid=pid)
        entry = (data.get("slots") or {}).get(str(slot)) or {}
        binded = entry.get("binded_type")
        if binded == "skill" and not spec["click"]:
            reply = mcp.text("client_mode_input", pid=pid, slot=slot,
                             duration_ms=120)
            if skill_id is not None:
                observed = ((entry.get("info") or {}).get("type"))
                if (not spec["allow_mismatch"] and observed is not None
                        and int(observed) != int(skill_id)):
                    raise RuntimeError(
                        f"mode slot {slot} holds skill {observed}, not {skill_id}")
            return {"bar": "mode", "slot": slot, "binded_type": binded,
                    "reply": reply, "mode": entry}
        if not entry:
            raise RuntimeError(f"mode slot {slot} is not in client_mode_actions")
        bounds = entry.get("bounds") or {}
        x = float(bounds.get("x", 0))
        y = float(bounds.get("y", 0))
        width = float(bounds.get("width", 0))
        height = float(bounds.get("height", 0))
        point = (max(0, min(4095, int(round(x + width / 2.0)))),
                 max(0, min(4095, int(round(y + height / 2.0)))))
        reply = mcp.text("client_pointer", pid=pid, action="click",
                         x=point[0], y=point[1])
        return {"bar": "mode", "slot": slot, "binded_type": binded,
                "point": list(point), "reply": reply, "mode": entry}
    if skill_id is not None:
        text = mcp.text("client_lua", pid=pid, target="ui",
                        code=ACTION_SCAN_LUA.replace("WANT", str(skill_id)))
        rows, _, found = text.partition("#")
        found_slot = int(found) if found.strip().isdigit() else None
        if found_slot is None:
            raise RuntimeError(
                f"skill {skill_id} is not on the action bar: {rows}")
        if slot is not None and slot != found_slot and not spec["allow_mismatch"]:
            raise RuntimeError(
                f"skill {skill_id} is on slot {found_slot}, not {slot}")
        slot = found_slot
        scan = rows
    elif slot is None:
        raise RuntimeError("use-skill needs a slot or id")
    else:
        scan = None
    key = KEY_ORDER.get(str(slot))
    if key is None:
        raise RuntimeError(f"no key for action slot {slot}")
    reply = mcp.text("client_input", pid=pid, key=key, duration_ms=120)
    return {"bar": "action", "slot": slot, "key": key, "reply": reply,
            "scan": scan}


def mount(mcp: Any, pid: int, spec: dict, clock: Clock) -> dict:
    if spec["kind"] == "pet":
        reply = mcp.text("client_lua", pid=pid, target="ui",
                         code="X2Mate:MountMate() return 1")
        return {"kind": "pet", "reply": reply}
    vehicle = json_or_error(mcp, "client_vehicle", pid=pid, action="status")
    if vehicle.get("mounted") or vehicle.get("driver"):
        return {"kind": "vehicle", "already": True, "vehicle": vehicle}
    reply = mcp.text("client_input", pid=pid, key="f", duration_ms=200)
    condition = {"expr": "vehicle.mounted == true", "path": "vehicle.mounted",
                 "op": "==", "literal": True, "source": "server",
                 "timeout_ms": spec["timeout_ms"], "poll_ms": 250}
    waited = wait_condition(mcp, pid, condition, clock)
    return {"kind": "vehicle", "reply": reply, "wait": waited}


def dismount(mcp: Any, pid: int, clock: Clock,
             timeout_ms: int = 8000) -> dict:
    vehicle = json_or_error(mcp, "client_vehicle", pid=pid, action="status")
    if vehicle.get("mounted") or vehicle.get("driver"):
        reply = mcp.text("client_vehicle", pid=pid, action="discard")
        kind = "vehicle"
    else:
        reply = mcp.text("client_lua", pid=pid, target="ui",
                         code="X2Mate:UnMountMate() return 1")
        kind = "pet"
    condition = {"expr": "vehicle.mounted == false",
                 "path": "vehicle.mounted", "op": "==", "literal": False,
                 "source": "server", "timeout_ms": timeout_ms,
                 "poll_ms": 250}
    waited = wait_condition(mcp, pid, condition, clock)
    return {"kind": kind, "reply": reply, "wait": waited}


def snapshot(mcp: Any, pid: int, run_dir: Path, step: dict, clock: Clock) -> dict:
    started = time.monotonic()
    system_ms, clock_before, clock_after, clock_error = read_clock(
        mcp, pid, "system")
    if system_ms is not None:
        clock.add("system", system_ms, clock_before, clock_after)
    shot_path = run_dir / "shots" / f"{step['id']}.png"
    shot_path.parent.mkdir(parents=True, exist_ok=True)
    shot_path.write_bytes(mcp.screenshot(pid))
    shot_ms = round((time.monotonic() - started) * 1000.0, 1)
    ui = ui_snapshot(mcp, pid)
    write_json(run_dir / "ui" / f"{step['id']}.json", ui)
    entities = read_entities(mcp, pid)
    player = unit_state(mcp, pid, "player")
    target = unit_state(mcp, pid, "target")
    animation = read_animation(mcp, pid, str(player.get("name") or ""),
                               str(target.get("name") or "") if target.get(
                                   "present") else None)
    if system_ms is not None:
        ui_ms = clock.ui_for_system(system_ms)
    else:
        ui_ms = clock.estimate()
    state = {
        "step": step["id"],
        "label": step.get("label", step["id"]),
        "client_ms": system_ms,
        "ui_ms": ui_ms,
        "t_ms": clock.relative(ui_ms),
        "clock_error": clock_error,
        "entities": entities,
        "animation": animation,
        "unit_state": {"player": player, "target": target},
        "vehicle": json_or_error(mcp, "client_vehicle", pid=pid,
                                 action="status"),
        "mode_actions": json_or_error(mcp, "client_mode_actions", pid=pid),
    }
    write_json(run_dir / "entities" / f"{step['id']}.json", state)
    return {"label": state["label"], "shot": str(shot_path),
            "ui": str(run_dir / "ui" / f"{step['id']}.json"),
            "entities": str(run_dir / "entities" / f"{step['id']}.json"),
            "entity_count": len(entities),
            "ui_widgets": len(ui.get("widgets") or []),
            "shot_ms": shot_ms}


def run_lua(mcp: Any, pid: int, step: dict) -> str:
    value = step["lua"]
    return mcp.text("client_lua", pid=pid, target=value["target"],
                    code=value["code"])


def set_slots(mcp: Any, pid: int, value: dict) -> str:
    mapping = ",".join(f"[{slot}]={skill}"
                       for slot, skill in sorted(value["slots"].items()))
    code = (r"""
pcall(function() X2Action:SetActionBarPage(%d) end)
local map={%s}
local out={}
for slot,id in pairs(map) do
  pcall(function() X2Action:ClearAction(slot) end)
  local ok,err=pcall(function() X2Action:SetActionSpell(slot, id) end)
  local ok2,t=pcall(X2Action.GetActionTooltipText, X2Action, slot)
  local name='?'
  if ok2 and type(t)=='table' and t.name~=nil then name=tostring(t.name) end
  out[#out+1]=string.format('%%d:%%s:%%s:%%s', slot,
    tostring(X2Action:GetActionType(slot)), name, tostring(ok))
end
table.sort(out)
return table.concat(out, '|')
""" % (value["page"], mapping))
    return mcp.text("client_lua", pid=pid, target="ui", code=code)


def execute_step(mcp: Any, pid: int, run_dir: Path, step: dict,
                 clock: Clock) -> dict:
    verb = step["verb"]
    result: Any = ""
    if verb == "gm":
        result = mcp.text("client_chat", pid=pid, message=step["command"])
    elif verb == "wait":
        wait = step["wait"]
        if "ms" in wait:
            time.sleep(wait["ms"] / 1000.0)
            result = {"wait_kind": "ms", "ms": wait["ms"]}
        else:
            waited = wait_condition(mcp, pid, wait, clock)
            result = {"wait_kind": "condition", **waited}
            if not waited["met"]:
                raise StepFailure(
                    f"condition not met after {wait['timeout_ms']} ms: "
                    f"{wait['expr']}", result)
    elif verb == "key":
        result = mcp.text("client_input", pid=pid, key=step["key"],
                          duration_ms=step["hold_ms"],
                          **({"modifiers": step["modifiers"]}
                             if step.get("modifiers") else {}))
    elif verb == "click-ui":
        result = click_widget(mcp, pid, step["widget"])
    elif verb == "target":
        result = target_unit(mcp, pid, step["target"])
    elif verb == "use-skill":
        result = use_skill(mcp, pid, step["use_skill"])
    elif verb == "interact":
        options = step["interact"]
        if options.get("action"):
            result = mcp.text("client_action", pid=pid,
                              action=options["action"])
        else:
            result = mcp.text("client_input", pid=pid, key=options["key"],
                              duration_ms=options["hold_ms"])
    elif verb == "move-to":
        x, y, z = step["position"]
        result = mcp.text("client_chat", pid=pid,
                          message=f"/move {x:g} {y:g} {z:g}")
    elif verb == "mount":
        result = mount(mcp, pid, step["mount"], clock)
    elif verb == "dismount":
        result = dismount(mcp, pid, clock)
    elif verb == "snapshot":
        result = snapshot(mcp, pid, run_dir, step, clock)
    elif verb == "mark":
        result = {"marker": step["label"]}
    elif verb == "lua":
        result = run_lua(mcp, pid, step)
    elif verb == "set-slots":
        result = set_slots(mcp, pid, step["set_slots"])
    else:
        raise RuntimeError(f"unhandled verb {verb}")
    return result


def write_step(handle: Any, row: dict) -> None:
    handle.write(json.dumps(row, ensure_ascii=False,
                            separators=(",", ":")) + "\n")
    handle.flush()


def remap_steps(path: Path, clock: Clock) -> int:
    """Rewrite steps.jsonl through the final clock fit, like the recorder."""
    if not path.exists():
        return 0
    rows = [json.loads(line) for line in
            path.read_text(encoding="utf-8").splitlines() if line.strip()]
    for row in rows:
        client = row.get("client_ms")
        if client is not None:
            row["ui_ms"] = clock.ui_for_system(client)
            row["t_ms"] = clock.relative(row["ui_ms"])
        after = row.get("after_client_ms")
        if after is not None:
            row["after_ui_ms"] = clock.ui_for_system(after)
            row["after_t_ms"] = clock.relative(row["after_ui_ms"])
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False,
                                    separators=(",", ":")) + "\n")
    return len(rows)


def run_steps(mcp: Any, pid: int, run_dir: Path, steps: list[dict],
              phase: str, clock: Clock, sink: Any, counters: dict) -> None:
    for index, step in enumerate(steps):
        before, before_wall, before_after, before_error = read_clock(
            mcp, pid, "system")
        if before is not None:
            clock.add("system", before, before_wall, before_after)
        started = time.monotonic()
        error_text = ""
        result: Any = ""
        try:
            result = execute_step(mcp, pid, run_dir, step, clock)
        except Exception as error:
            error_text = f"{type(error).__name__}: {error}"
            result = (error.result if isinstance(error, StepFailure)
                      else {"error": error_text})
        after, after_before, after_wall, after_error = read_clock(
            mcp, pid, "system")
        if after is not None:
            clock.add("system", after, after_before, after_wall)
        ui_before = clock.ui_for_system(before) if before is not None else None
        ui_after = clock.ui_for_system(after) if after is not None else None
        row = {
            "schema": SCHEMA, "seq": counters["seq"],
            "phase": phase, "index": index, "id": step["id"],
            "verb": step["verb"], "note": step.get("note", ""),
            "optional": step["optional"],
            "server_tag": step.get("server_tag"),
            "wall_ms": round(before_wall, 1),
            "client_ms": before,
            "ui_ms": ui_before,
            "t_ms": clock.relative(ui_before),
            "after_client_ms": after,
            "after_ui_ms": ui_after,
            "after_t_ms": clock.relative(ui_after),
            "elapsed_client_ms": (int(round(after - before))
                                  if before is not None and after is not None
                                  else None),
            "elapsed_wall_ms": round((time.monotonic() - started) * 1000.0, 1),
            "clock_error": before_error or after_error,
            "result": result,
            "error": error_text or None,
        }
        if step["verb"] == "wait" and "wait" in step and "source" in step["wait"]:
            row["condition_source"] = step["wait"]["source"]
            row["server_tag"] = (step.get("server_tag")
                                 or step["wait"]["source"])
        write_step(sink, row)
        counters["seq"] += 1
        counters["steps"] += 1
        if step["verb"] == "snapshot":
            counters["snapshots"].append(step["id"])
        log("step", phase, index, step["id"], step["verb"],
            (json.dumps(result, ensure_ascii=False)
             if not isinstance(result, str) else result)[:300])
        if error_text and not step["optional"]:
            raise RuntimeError(
                f"step {step['id']} ({step['verb']}) failed: {error_text}")


def parse_reply_delay(value: str | None, lane_env: dict[str, str]) -> dict:
    hint = value
    source = "cli"
    if hint is None:
        hint = os.environ.get("AA_REPLY_DELAY")
        source = "env"
    if hint is None:
        hint = lane_env.get("REPLY_DELAY") or lane_env.get("AA_REPLY_DELAY")
        source = "lane_env"
    if hint is None:
        return {"hint_ms": None, "source": "none"}
    match = re.search(r"-?\d+(?:\.\d+)?", str(hint))
    if not match:
        return {"hint_ms": None, "source": source, "raw": str(hint)}
    return {"hint_ms": float(match.group(0)), "source": source,
            "raw": str(hint)}


def capture_root(args: argparse.Namespace) -> Path:
    value = args.capture_root or os.environ.get("AA_PARITY_ROOT")
    return Path(value) if value else ROOT / "data/local/parity"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Run a parity scenario on the original 1.2 client.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__)
    parser.add_argument("scenario", type=Path)
    parser.add_argument("--capture-root", type=Path, default=None)
    parser.add_argument("--run-id", default=None)
    parser.add_argument("--pid", type=int, default=None,
                        help="attach to an already running client")
    parser.add_argument("--lane", default=None,
                        help="start the client through this lane's live-stack.sh")
    parser.add_argument("--lanes-dir", type=Path,
                        default=Path(os.environ.get("AAEMU_LANES", "")))
    parser.add_argument("--server-dir", type=Path, default=None)
    parser.add_argument("--server-log", type=Path, default=None)
    parser.add_argument("--server-log-from-start", action="store_true",
                        help="read the whole server log instead of the run slice")
    parser.add_argument("--reply-delay", default=None,
                        help="record the lane's reply-delay hint in meta.json")
    parser.add_argument("--driver", type=Path,
                        default=Path(os.environ.get("AA_DRIVER",
                                                    "aaemu-clientdriver.exe")))
    parser.add_argument("--dll", type=Path,
                        default=Path(os.environ.get("AA_DRIVER_DLL",
                                                    "aadriver.dll")))
    parser.add_argument("--launcher", type=Path,
                        default=Path(os.environ.get("AA_LAUNCHER",
                                                    "aaemu-launcher.exe")))
    parser.add_argument("--client", type=Path,
                        default=Path(os.environ.get("AA_CLIENT",
                                                    "archeage.exe")))
    parser.add_argument("--login", default=os.environ.get("AA_LOGIN",
                                                          "127.0.0.1:1237"))
    parser.add_argument("--user", default=os.environ.get("AA_USER",
                                                         "clienttest"))
    parser.add_argument("--password-file", type=Path, default=None)
    parser.add_argument("--character", default=os.environ.get("AA_CHARACTER",
                                                              "ParityT1"))
    parser.add_argument("--character-ability", type=int, default=2)
    parser.add_argument("--create-character", action="store_true")
    parser.add_argument("--start-timeout", type=float, default=120.0)
    parser.add_argument("--keep-client", action="store_true",
                        help="do not stop the client by pid at the end")
    parser.add_argument("--fake-mcp", type=Path, default=None,
                        help="offline test: replay this MCP reply fixture")
    args = parser.parse_args(argv)

    if args.password_file is None:
        password = os.environ.get("AA_PASSWORD_FILE")
        if password:
            args.password_file = Path(password)

    scenario = scenario_format.load_scenario(args.scenario)
    run_id = args.run_id or datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    root = capture_root(args)
    run_dir = root / scenario["name"] / run_id
    run_dir.mkdir(parents=True, exist_ok=True)
    (run_dir / "ui").mkdir(exist_ok=True)
    (run_dir / "entities").mkdir(exist_ok=True)
    (run_dir / "shots").mkdir(exist_ok=True)

    lane_env: dict[str, str] = {}
    if args.lane:
        lane_env = load_lane(args.lanes_dir, args.lane)
    server_dir = args.server_dir
    if server_dir is None and lane_env.get("WT"):
        server_dir = Path(lane_env["WT"])
    server_log = args.server_log
    if server_log is None and lane_env.get("LIVE"):
        server_log = Path(lane_env["LIVE"]) / "logs" / "rs-game.log"

    clock = Clock()
    mcp: Any = None
    pid = args.pid
    started_pid = False
    start_info: dict = {"source": "pid" if pid else "none"}
    started_at = iso_now()
    failure: BaseException | None = None
    result: dict = {"ok": False}
    counters = {"seq": 0, "steps": 0, "snapshots": []}
    server_offset = 0
    packets_summary: dict = {}
    meta_path = run_dir / "meta.json"
    steps_path = run_dir / "steps.jsonl"
    clock_path = run_dir / "clock_sync.jsonl"

    if args.pid is None and args.lane is None and args.fake_mcp is None:
        parser.error("pass --pid, --lane, or --fake-mcp")
    if args.lane and not str(args.lanes_dir):
        parser.error("--lanes-dir or AAEMU_LANES is required with --lane")

    try:
        if server_log is not None and server_log.exists():
            server_offset = 0 if args.server_log_from_start else (
                server_log.stat().st_size)
        if not args.fake_mcp:
            if pid is None:
                pid, start_info = start_lane_client(
                    args.lane, args.lanes_dir, server_dir,
                    args.start_timeout, run_dir / "start.log")
                started_pid = start_info.get("source") == "lane"
                log("client", "started" if started_pid else "adopted",
                    pid, start_info["source"])
            mcp = Mcp(args.driver, args.dll, args.launcher, args.client,
                      args.password_file, args.login, args.user,
                      run_dir / "mcp")
        else:
            mcp = FixtureMcp(args.fake_mcp)
            if pid is None:
                pid = 1
        if not args.fake_mcp:
            bad = loaded_anti_cheat(pid)
            if bad:
                if started_pid:
                    stop_pid(pid)
                raise RuntimeError(
                    f"anti-cheat module loaded: {bad}; refusing to drive")
        status = attach_and_hide(mcp, pid, started_pid)
        log("client hidden", pid)
        player_name = ensure_world(mcp, pid, args.character,
                                   args.character_ability,
                                   args.create_character)
        log("player", player_name)
        load_ui_library(mcp, pid)
        mcp.text("client_lua", pid=pid, target="ui", code=HOOK_LUA)

        system, s_before, s_after, s_error = read_clock(mcp, pid, "system")
        ui, u_before, u_after, u_error = read_clock(mcp, pid, "ui")
        if system is not None:
            clock.add("system", system, s_before, s_after)
        if ui is not None:
            clock.add("ui", ui, u_before, u_after)
        with clock_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps({
                "schema": SCHEMA, "kind": "start",
                "wall_ms": round((s_before + s_after) / 2.0, 1),
                "system_ms": system, "ui_ms": ui,
                "system_rtt_ms": round(s_after - s_before, 3),
                "ui_rtt_ms": round(u_after - u_before, 3),
                "system_error": s_error, "ui_error": u_error,
            }, ensure_ascii=False, separators=(",", ":")) + "\n")

        with steps_path.open("a", encoding="utf-8") as sink:
            if scenario["preconditions"]:
                run_steps(mcp, pid, run_dir, scenario["preconditions"],
                          "precondition", clock, sink, counters)
            run_steps(mcp, pid, run_dir, scenario["steps"], "step", clock,
                      sink, counters)
        system, s_before, s_after, s_error = read_clock(mcp, pid, "system")
        ui, u_before, u_after, u_error = read_clock(mcp, pid, "ui")
        if system is not None:
            clock.add("system", system, s_before, s_after)
        if ui is not None:
            clock.add("ui", ui, u_before, u_after)
        with clock_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps({
                "schema": SCHEMA, "kind": "end",
                "wall_ms": round((s_before + s_after) / 2.0, 1),
                "system_ms": system, "ui_ms": ui,
                "system_rtt_ms": round(s_after - s_before, 3),
                "ui_rtt_ms": round(u_after - u_before, 3),
                "system_error": s_error, "ui_error": u_error,
            }, ensure_ascii=False, separators=(",", ":")) + "\n")
        result = {"ok": True}
    except BaseException as error:
        failure = error
        log("capture failed:", type(error).__name__, error)
    finally:
        if mcp is not None:
            for target in ("ui", "system"):
                try:
                    mcp.text("client_lua", pid=pid, target=target, code=UNHOOK_LUA)
                except Exception:
                    pass
            try:
                mcp.close()
            except Exception:
                pass
        if pid and started_pid and not args.keep_client:
            stop_pid(pid)
            log("stopped client", pid)
        try:
            if server_log is not None and server_log.exists():
                records, _ = packet_clock.read_log(server_log, server_offset)
                packet_clock.align(records, clock)
                count = packet_clock.write_rows(run_dir / "client_packets.jsonl",
                                                records)
                packets_summary = packet_clock.summary(records)
                packets_summary["written"] = count
                log("client packets", count, packets_summary["anchors"])
            else:
                (run_dir / "client_packets.jsonl").write_text("", encoding="utf-8")
                packets_summary = {"written": 0, "reason": "no server log"}
        except Exception as error:
            packets_summary = {"error": f"{type(error).__name__}: {error}"}
            log("packet alignment failed:", error)
        try:
            remap_steps(steps_path, clock)
        except Exception as error:
            log("step clock remap failed:", error)
        start_process = start_info.pop("_process", None)
        if start_process is not None:
            try:
                start_process.wait(timeout=20)
            except Exception:
                pass
        server_info = git_info(server_dir)
        reply_delay = parse_reply_delay(args.reply_delay, lane_env)
        reply_delay["lane_branch"] = lane_env.get("BRANCH")
        reply_delay["server_branch"] = server_info.get("branch")
        reply_delay["server_commit"] = server_info.get("commit")
        meta = {
            "schema": SCHEMA,
            "kind": "original-client",
            "scenario": {
                "name": scenario["name"], "path": scenario["path"],
                "sha256": sha256_file(Path(scenario["path"])),
                "steps": len(scenario["steps"]),
                "preconditions": len(scenario["preconditions"]),
            },
            "run_id": run_id,
            "started_at": started_at, "finished_at": iso_now(),
            "result": {"ok": failure is None and result.get("ok", False),
                       "error": (f"{type(failure).__name__}: {failure}"
                                 if failure is not None else None)},
            "client": {
                "pid": pid, "status": status if "status" in locals() else None,
                "start": start_info,
                "started_by_runner": started_pid,
                "path": str(args.client), "hidden": True,
                "stopped": bool(pid and started_pid and not args.keep_client),
            },
            "lane": args.lane,
            "server": {
                **server_info,
                "log": str(server_log) if server_log else None,
                "log_offset": server_offset,
                "field_trace": "RUST_LOG=info,fields=trace,connection=debug",
            },
            "reply_delay": reply_delay,
            "clock": clock.report(),
            "packets": packets_summary,
            "steps": {"rows": counters["steps"],
                      "snapshots": counters["snapshots"]},
            "files": {"steps": "steps.jsonl",
                      "ui": "ui/<step>.json",
                      "entities": "entities/<step>.json",
                      "shots": "shots/<step>.png",
                      "packets": "client_packets.jsonl"},
        }
        write_json(meta_path, meta)
        log("wrote", run_dir)
    if failure is not None:
        return 1
    return 0 if result.get("ok") else 2


if __name__ == "__main__":
    raise SystemExit(main())
