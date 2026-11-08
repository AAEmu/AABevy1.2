"""A small newline-JSON-RPC client for `aaemu-clientdriver mcp` (the MCP driver of the original 1.2 client).

Same wire protocol and same `tool()` / `text()` calls as `Mcp` in tools/combat-side/record_original.py and
tools/parity/run_original.py (so functions written for those, e.g. `find_target`, accept this object), with a reader thread so a
driver that stops answering raises instead of hanging, and `require()` to fail early and clearly when the driver lacks a tool
(the skill parity runs need `client_trace` and, for video, `client_video`; both are newer than the rest).

A driver path ending in `.py` is run with this Python (the offline tests use `tests/fake_driver.py`).
"""
from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any

DEFAULT_TIMEOUT_S = 30.0


class DriverError(RuntimeError):
    """The driver answered with an error, or did not answer."""


class DriverGone(DriverError):
    """The driver process is gone, closed its output, or stopped answering: nothing more can be done with it."""


class MissingToolError(DriverError):
    """The driver has no such tool (an older aaemu-clientdriver)."""


def add_driver_args(ap: argparse.ArgumentParser) -> None:
    """The arguments every driver-using command shares (the same names as record_original.py)."""
    env = os.environ.get
    ap.add_argument("--pid", type=int, default=None, help="the running original client to attach to (started by the lane or you)")
    ap.add_argument("--driver", type=Path, default=Path(env("AA_DRIVER", "aaemu-clientdriver.exe")))
    ap.add_argument("--dll", type=Path, default=Path(env("AA_DRIVER_DLL", "aadriver.dll")))
    ap.add_argument("--launcher", type=Path, default=Path(env("AA_LAUNCHER", "aaemu-launcher.exe")))
    ap.add_argument("--client", type=Path, default=Path(env("AA_CLIENT", "archeage.exe")))
    ap.add_argument("--login", default=env("AA_LOGIN", "127.0.0.1:1237"))
    ap.add_argument("--user", default=env("AA_USER", "clienttest"))
    ap.add_argument("--password-file", type=Path, default=Path(env("AA_PASSWORD_FILE")) if env("AA_PASSWORD_FILE") else None)
    ap.add_argument("--artifact-dir", type=Path, default=None, help="the driver's scratch directory (default: <capture>/orig/mcp)")


class Mcp:
    """One driver process; calls are serialised by the caller (one runner thread)."""

    def __init__(self, driver: Path, dll: Path | None, launcher: Path | None, client: Path | None, password_file: Path | None,
                 login: str, user: str, artifact: Path | None, timeout_s: float = DEFAULT_TIMEOUT_S):
        command = [str(driver), "mcp"]
        for flag, value in (("--dll", dll), ("--client", client), ("--launcher", launcher), ("--login", login), ("--user", user),
                            ("--artifact-dir", artifact), ("--password-file", password_file)):
            if value is not None and str(value) != "":
                command += [flag, str(value)]
        if Path(driver).suffix.lower() == ".py":
            command = [sys.executable] + command
        self.timeout_s = timeout_s
        self.sequence = 0
        self._tools: set[str] | None = None
        self.replies: queue.Queue[str | None] = queue.Queue()
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True,
                                        encoding="utf-8", creationflags=getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0))
        self._reader_thread = threading.Thread(target=self._reader, name="mcp-reader", daemon=True)
        self._reader_thread.start()
        try:
            self.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                     "clientInfo": {"name": "skillparity", "version": "1"}})
            assert self.process.stdin
            self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
            self.process.stdin.flush()
        except BaseException:
            self.close()
            raise

    @classmethod
    def from_args(cls, args: argparse.Namespace, artifact: Path | None = None) -> "Mcp":
        return cls(args.driver, args.dll, args.launcher, args.client, args.password_file, args.login, args.user,
                   args.artifact_dir or artifact)

    def _reader(self) -> None:
        try:
            assert self.process.stdout
            for line in self.process.stdout:
                self.replies.put(line)
        except Exception:
            pass
        finally:
            self.replies.put(None)

    def call(self, method: str, params: object = None, timeout_s: float | None = None) -> Any:
        assert self.process.stdin
        self.sequence += 1
        request: dict[str, Any] = {"jsonrpc": "2.0", "id": self.sequence, "method": method}
        if params is not None:
            request["params"] = params
        try:
            self.process.stdin.write(json.dumps(request) + "\n")
            self.process.stdin.flush()
        except (OSError, ValueError) as error:
            raise DriverGone(f"driver is gone: {error}") from error
        for _ in range(64):                                   # skip stale replies of calls that timed out
            try:
                line = self.replies.get(timeout=timeout_s or self.timeout_s)
            except queue.Empty:
                raise DriverGone(f"driver did not answer {method} within {timeout_s or self.timeout_s:g} s") from None
            if line is None:
                raise DriverGone("driver closed its output")
            try:
                response = json.loads(line)
            except ValueError:
                continue
            if response.get("id") == self.sequence:
                if "error" in response:
                    raise DriverError(str(response["error"]))
                return response["result"]
        raise DriverError(f"driver sent no reply for id {self.sequence}")

    def tools(self) -> set[str] | None:
        """Names of the driver's tools (`tools/list`); None when the driver cannot list them."""
        if self._tools is None:
            try:
                result = self.call("tools/list")
                self._tools = {t["name"] for t in result.get("tools", []) if isinstance(t, dict) and "name" in t}
            except DriverError:
                return None
        return self._tools

    def require(self, *names: str) -> None:
        """Raise MissingToolError naming every tool the driver does not have (nothing when it cannot list its tools)."""
        have = self.tools()
        if have is None:
            return
        missing = [n for n in names if n not in have]
        if missing:
            raise MissingToolError(
                "this aaemu-clientdriver has no tool " + ", ".join(missing) + ": build the driver with the skill parity trace "
                "tools (client_trace, client_video) and pass it with --driver")

    def tool(self, name: str, **arguments: Any) -> dict:
        try:
            result = self.call("tools/call", {"name": name, "arguments": arguments})
        except DriverError as error:
            text = str(error).lower()
            if "unknown tool" in text or "no such tool" in text or "-32601" in text or ("not found" in text and "tool" in text):
                raise MissingToolError(f"the driver has no tool {name!r}: {error}") from error
            raise
        if result.get("isError"):
            raise DriverError("\n".join(c.get("text", "") for c in result.get("content", [])) or f"{name} failed")
        return result

    def text(self, name: str, **arguments: Any) -> str:
        return "\n".join(c["text"] for c in self.tool(name, **arguments).get("content", []) if c.get("type") == "text")

    def close(self) -> None:
        """Close the driver process. The client it was attached to keeps running (this never stops a client)."""
        process = self.process
        try:
            if process.stdin:
                process.stdin.close()
            process.wait(timeout=5)
        except Exception:
            try:
                process.kill()
            except Exception:
                pass
        try:
            self._reader_thread.join(timeout=2)
            if process.stdout:
                process.stdout.close()
        except Exception:
            pass
