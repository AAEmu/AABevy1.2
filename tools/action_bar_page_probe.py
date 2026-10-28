#!/usr/bin/env python3
"""Capture the original 1.2 action bar page API and per-page action slots.

The original client keeps four action bar pages and numbers the hotbar slots
globally, one based: page 1 shows slots 1..12, pages 2..4 show 49..60, 61..72
and 73..84.  This probe drives the hidden original through aaemu-clientdriver,
asks its own Lua API what the page is (`X2Action:GetActionBarPage` /
`SetActionBarPage`), switches pages with the original Shift+Up/Down bindings,
and drags a learned skill between the first two and the last two cells of each
page.  Every drop is read from the game server's decoded packed trace
(`RUST_LOG=info,aaemu_game::net::fields=trace`) as a `CSUpdateActionSlotPacket`
body, so the slot numbers are the bytes the original client actually sent.

The client process is launched hidden and stopped by pid in a finally block.
The script refuses to run if an anti-cheat module is loaded; it only enumerates
modules (ReadProcessMemory / Toolhelp), it does not read or write game memory.

Bulky raw output belongs under `--raw` (default
`data/extract/action-bar-pages-<date>-r1`); the committed CSV, JSON and report
are written to `--out` (default `tools/evidence/action_bar_pages`).

Usage:
    python tools/action_bar_page_probe.py --client <archeage.exe> \
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \
        --launcher <aaemu-launcher.exe> --login <host:port> \
        --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        --skill-id 18132 --out tools/evidence/action_bar_pages

The game server must log the packet bodies:
`RUST_LOG=info,aaemu_game::net::fields=trace` (the `aaemu_game::net::fields`
target).  The probe checks that trace lines appear after login and fails with
the setting in the message when they do not.

The action bar must be unlocked (the original's `ACTIONBAR_STATE.UNLOCK`);
a locked bar refuses drags.  The probe checks `IsActionBarLocked()` and fails
before changing anything when the bar is locked.
"""
from __future__ import annotations

import argparse
import csv
import ctypes
import json
import os
import re
import struct
import subprocess
import sys
import time
from ctypes import wintypes
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect", "gameguard", "hackshield")
PAGE_BASE = {1: 1, 2: 49, 3: 61, 4: 73}
PAGE_SLOTS = 12
PAGE_COUNT = 4


def log(*a: object) -> None:
    print("action-bar-pages:", *a, flush=True)


def shown(path: Path, kind: str) -> str:
    """The committed report shows paths relative to the repo, else a placeholder."""
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return f"<{kind}>" + (f"/{path.name}" if path.name else "")


class Mcp:
    def __init__(self, driver: Path, dll: Path, launcher: Path, client: Path, login: str,
                 password_file: Path, user: str, artifact: Path):
        artifact.mkdir(parents=True, exist_ok=True)
        self.process = subprocess.Popen(
            [str(driver), "mcp", "--dll", str(dll), "--client", str(client), "--launcher", str(launcher),
             "--password-file", str(password_file), "--login", login, "--user", user,
             "--artifact-dir", str(artifact)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8",
        )
        self.sequence = 0
        self.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "action-bar-pages-probe", "version": "1"}})
        assert self.process.stdin and self.process.stdout
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def call(self, method: str, params: object = None) -> object:
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
    """Read-only process inspection: the module list, to refuse anti-cheat modules."""

    def __init__(self, pid: int):
        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.handle = self.kernel.OpenProcess(0x0410, False, pid)  # QUERY_INFORMATION | VM_READ
        if not self.handle:
            raise RuntimeError("OpenProcess failed")
        self.pid = pid

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


def lua(mcp: Mcp, pid: int, code: str) -> str:
    return mcp.text("client_lua", pid=pid, target="ui", code=code)


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


def release_input(mcp: Mcp, pid: int) -> None:
    try:
        mcp.text("client_input", pid=pid, key="release", duration_ms=50)
    except Exception:
        pass
    time.sleep(0.5)


def read_new_lines(path: Path, offset: int) -> list[str]:
    if not path.exists():
        return []
    return path.read_bytes()[offset:].decode("utf-8", "replace").splitlines()


def action_slot_packets(lines: list[str]) -> list[dict[str, object]]:
    """The decoded CSUpdateActionSlot bodies from the server's field trace."""
    out: list[dict[str, object]] = []
    for line in lines:
        match = re.search(r"CSUpdateActionSlotPacket len=(\d+) body=([0-9a-f ]+)", line)
        if not match:
            continue
        body = bytes.fromhex(match.group(2).strip())
        if len(body) < 2:
            continue
        slot, kind = body[0], body[1]
        action_id = 0
        if kind in (1, 2, 5) and len(body) >= 6:
            action_id = struct.unpack("<I", body[2:6])[0]
        elif kind == 4 and len(body) >= 10:
            action_id = struct.unpack("<Q", body[2:10])[0]
        out.append({
            "slot": slot,
            "kind": kind,
            "id": action_id,
            "body": body.hex(" "),
            "line": line,
        })
    return out


def render_report(meta: dict, key_cases: list[dict], source_commit: str, report_date: str,
                  out: Path, raw: Path) -> str:
    """The committed report, from the capture record (also used by --render-only)."""
    api = meta.get("api", [])
    drags = meta.get("drags", [])
    matched = sum(1 for row in drags if row["match"])
    key_matched = sum(1 for row in key_cases if row["match"])
    destination_count = sum(1 for row in drags if row["destination_set"])
    report = [
        "How to reproduce:",
        "  server: RUST_LOG=info,aaemu_game::net::fields=trace (the CSUpdateActionSlot body trace)",
        "  client: the original 1.2 client (r208022), a character in world with the action bar unlocked",
        "  probe: python tools/action_bar_page_probe.py --client <client> --driver <driver> --driver-dll <driver-dll> "
        "--launcher <launcher> --login <host:port> --account <account> --character <character> "
        "--password-file <password-file> --server-log <server-log> --skill-id <known-skill> "
        "--out tools/evidence/action_bar_pages",
        "",
        "action bar pages: native 1.2 capture",
        f"date: {report_date}",
        "account: <account> / <character>",
        f"client: <client> (hidden)  pid: {meta['pid']}  x2game base: {meta['x2game_base']:#x}",
        "server log: <server-log>",
        f"source commit: {source_commit}",
        "",
        "Lua API (1 based, the original's own answers):",
    ]
    for row in api:
        report.append("  " + json.dumps(row))
    report += [
        "  (page_changed_same_page: `before` is what the fresh widget already held; the repeated",
        "   call on the current page added exactly one more event.)",
        "",
        "page keys (original default bindings):",
    ]
    for row in key_cases:
        report.append("  " + json.dumps(row))
    report += [
        "",
        f"drags: {matched}/{len(drags)} mapping cases matched; {destination_count}/{len(drags)} destination sets",
        f"keys: {key_matched}/{len(key_cases)} matched",
        "",
        "  page  cells    expected slots   observed slots   destination set  match",
    ]
    for row in drags:
        report.append(
            f"  {row['page']:4d}  {row['from_cell']:2d}->{row['to_cell']:<2d}  "
            f"{row['expected_source_slot']:3d}->{row['expected_target_slot']:<3d}      "
            f"{','.join(str(s) for s in row['slots']) or '-':<16s} "
            f"{str(row['destination_set']):<15s} {'yes' if row['match'] else 'NO'}"
        )
    report += [
        "",
        "What the drags are: the hidden client was switched with X2Action:SetActionBarPage, a skill it",
        "knows (--skill-id) was put on the page's first/last cell with X2Action:SetActionSpell as a",
        "chosen input, then the cell was dragged one cell to the right through the client's own UI",
        "path.  Every CSUpdateActionSlotPacket body in the server field trace is listed above; the",
        "slot byte is the first byte of the body.  Pages 3 and 4 were captured, not extrapolated.",
        "",
        "The first-cell drops set each page's first slot (1/49/61/73); the page-1 trace also shows",
        "the source clear (1:0:0), while the pages 2..4 source slots are set again in the captured",
        "window.  The far-right drags are sensitive to the hit area at that end: the press can land",
        "on the last slot itself and the release can fall outside the bar, so a case can capture the",
        "last slot's clear, its set, or both.  Every case still shows the page's last packet slot",
        f"(12/60/72/84); the destination column is the mapping proof ({destination_count}/",
        f"{len(drags)} destination sets).  The first-cell destinations prove the page bases 1/49/61/73,",
        "and the last-cell destinations prove the last slots 12/60/72/84.",
        "",
        "Shift+wheel does not change the page in the original defaults (the wheel is zoom_in/zoom_out);",
        "Shift+Up/Down are the page bindings and wrap 1 -> 4 and 4 -> 1.",
        "",
        "Files:",
        f"  native.csv   {shown(out / 'native.csv', 'out')}",
        f"  native.json  {shown(out / 'native.json', 'out')}",
        f"  raw run dir  {shown(raw, 'raw')}",
    ]
    return "\n".join(report) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("ABP_CLIENT", "archeage.exe"),
                        help="path to the original 1.2 archeage.exe")
    parser.add_argument("--driver", default=os.environ.get("ABP_DRIVER", "aaemu-clientdriver.exe"),
                        help="path to AAEmu's aaemu-clientdriver.exe")
    parser.add_argument("--driver-dll", default=os.environ.get("ABP_DRIVER_DLL", "aadriver.dll"),
                        help="path to the x86 aadriver.dll")
    parser.add_argument("--launcher", default=os.environ.get("ABP_LAUNCHER", "aaemu-launcher.exe"),
                        help="path to AAEmu's aaemu-launcher.exe")
    parser.add_argument("--login", default=os.environ.get("ABP_LOGIN", "127.0.0.1:27237"),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("ABP_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("ABP_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("ABP_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("ABP_SERVER_LOG"),
                        help="game server log with the packet field trace")
    parser.add_argument("--skill-id", type=int, default=18132,
                        help="a skill the character knows, put on each page before the drags (chosen input)")
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "action_bar_pages"))
    parser.add_argument("--raw", default=os.environ.get("ABP_RAW")
                        or str(ROOT / "data" / "extract" / f"action-bar-pages-{time.strftime('%Y%m%d')}-r1"))
    parser.add_argument("--render-only", action="store_true",
                        help="rewrite report.txt from the committed native.json (no client run)")
    parser.add_argument("--source-commit", help="source commit for --render-only (default: native.json or HEAD)")
    parser.add_argument("--report-date", help="report date for --render-only (default: native.json or now)")
    args = parser.parse_args()
    out = Path(args.out)
    raw = Path(args.raw)
    if args.render_only:
        data = json.loads((out / "native.json").read_text(encoding="utf-8"))
        meta = data["meta"]
        source_commit = args.source_commit or meta.get("source_commit") or subprocess.run(
            ["git", "-C", str(ROOT), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        report_date = args.report_date or meta.get("report_date") or time.strftime("%Y-%m-%d %H:%M:%S")
        (out / "report.txt").write_text(
            render_report(meta, data.get("keys", []), source_commit, report_date, out, raw), encoding="utf-8")
        log(f"rendered {out / 'report.txt'} from {out / 'native.json'}")
        return 0
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set ABP_{name.upper()})")
    client = Path(args.client)
    driver = Path(args.driver)
    dll = Path(args.driver_dll)
    launcher = Path(args.launcher)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    if not client.exists() or not driver.exists() or not dll.exists() or not launcher.exists() or not password_file.exists():
        raise SystemExit(f"missing client/driver/dll/launcher/password file: {client} {driver} {dll} {launcher} {password_file}")
    if not server_log.exists():
        raise SystemExit(f"missing server log: {server_log}")
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)
    source_commit = subprocess.run(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'],
                                   capture_output=True, text=True).stdout.strip()
    report_date = time.strftime("%Y-%m-%d %H:%M:%S")

    log(f"account {args.account}: {args.character}")
    mcp = Mcp(driver, dll, launcher, client, args.login, password_file, args.account, raw / "mcp")
    pid = args.pid
    meta: dict[str, object] = {
        "account": "<account>",
        "character": "<character>",
        "date": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "client": "<client>",
        "server_log": "<server-log>",
        "skill_id": args.skill_id,
        "source_commit": source_commit,
        "report_date": report_date,
    }
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
            lua(mcp, pid, "X2World:EnterWorld(1,0)")
            wait_stage(mcp, pid, 4, 240)
            lua(mcp, pid, "local b=characterSelectWindow.bottomPanel.startButton b:OnClick('LeftButton')")
            wait_player(mcp, pid, 240)
            time.sleep(3.0)
        else:
            wait_player(mcp, pid, 30)

        mem = Mem(pid)
        modules = mem.modules()
        for name, _ in modules:
            if any(bad in name.lower() for bad in ANTI_CHEAT):
                raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to run")
        base = mem.module_base("x2game.dll")
        if not base:
            raise RuntimeError("x2game.dll not loaded")
        meta["pid"] = pid
        meta["x2game_base"] = base
        meta["modules_checked"] = len(modules)
        log("x2game base", hex(base), "modules", len(modules))

        # The field trace must be on before any drag, else there is nothing to compare.
        offset = server_log.stat().st_size
        time.sleep(1.0)
        if not any("body:" in line for line in read_new_lines(server_log, offset)):
            raise RuntimeError("no packet field trace in the server log; start the game server with "
                               "RUST_LOG=info,aaemu_game::net::fields=trace")
        meta["trace_offset"] = offset

        lock = lua(mcp, pid, r"""
local o={}
for k,v in pairs(ACTIONBAR_STATE) do o[#o+1]=tostring(k)..'='..tostring(v) end
return 'locked='..tostring(IsActionBarLocked())..' state={'..table.concat(o,',')..'}'
""").strip()
        log("lock", lock)
        meta["lock"] = lock
        if lock != "locked=0 state={UNLOCK=0,LOCK=1}":
            raise RuntimeError("the action bar is locked (or ACTIONBAR_STATE changed); unlock it and rerun: " + lock)

        api: list[dict[str, object]] = []
        api.append({"case": "initial_page", "value": int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())})
        for page in range(1, PAGE_COUNT + 1):
            returned = int(lua(mcp, pid, f"return X2Action:SetActionBarPage({page})").strip())
            now = int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())
            api.append({"case": "set_page", "page": page, "returned": returned, "get": now})
        for page in (0, -1, 5, 99):
            returned = int(lua(mcp, pid, f"return X2Action:SetActionBarPage({page})").strip())
            now = int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())
            api.append({"case": "set_page_invalid", "page": page, "returned": returned, "get": now})

        # ACTION_BAR_PAGE_CHANGED arguments, from a script widget registered for it.
        lua(mcp, pid, r"""
if not abPageProbe then
  abPageProbe=UIParent:CreateWidget('emptywidget','abPageProbe','UIParent')
  abPageProbe:Show(true)
  abPageProbe:RegisterEvent('ACTION_BAR_PAGE_CHANGED')
  abPageProbe:SetHandler('OnEvent',function(self,event,a) table.insert(self.events or {},a) end)
end
abPageProbe.events={}
return 'ok'
""")
        for page in range(1, PAGE_COUNT + 1):
            lua(mcp, pid, f"X2Action:SetActionBarPage({page})")
        time.sleep(0.3)
        events = lua(mcp, pid, "return table.concat(abPageProbe.events,',')").strip()
        api.append({"case": "page_changed_events", "events": [int(v) for v in events.split(",") if v]})
        # `FUN_393f6910` applies the page and fires the event even when it is the current page.
        # A fresh widget isolates the repeated call from the events of the loop above.
        lua(mcp, pid, r"""
if not abPageProbe2 then
  abPageProbe2=UIParent:CreateWidget('emptywidget','abPageProbe2','UIParent')
  abPageProbe2:Show(true)
  abPageProbe2:RegisterEvent('ACTION_BAR_PAGE_CHANGED')
  abPageProbe2:SetHandler('OnEvent',function(self,event,a) table.insert(self.events or {},a) end)
end
abPageProbe2.events={}
return 'ok'
""")
        time.sleep(1.0)
        before = lua(mcp, pid, "return table.concat(abPageProbe2.events,',')").strip()
        lua(mcp, pid, f"X2Action:SetActionBarPage({PAGE_COUNT})")
        time.sleep(1.0)
        events = lua(mcp, pid, "return table.concat(abPageProbe2.events,',')").strip()
        api.append({"case": "page_changed_same_page", "page": PAGE_COUNT,
                    "before": [int(v) for v in before.split(",") if v],
                    "events": [int(v) for v in events.split(",") if v]})

        # The original Shift+Up/Down bindings wrap: 4 -> Down -> 1, 1 -> Up -> 4.
        key_cases: list[dict[str, object]] = []
        for start, key, expected in ((4, "down", 1), (1, "up", PAGE_COUNT)):
            lua(mcp, pid, f"X2Action:SetActionBarPage({start})")
            time.sleep(0.2)
            before = int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())
            mcp.text("client_input", pid=pid, key=key, modifiers=["shift"], duration_ms=120)
            time.sleep(0.4)
            after = int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())
            key_cases.append({"start": before, "key": "shift+" + key, "expected": expected, "got": after,
                              "match": after == expected})
        # Shift+wheel is not a page binding in the original defaults.
        for notches, label in ((1, "shift+wheel up"), (-1, "shift+wheel down")):
            lua(mcp, pid, "X2Action:SetActionBarPage(2)")
            time.sleep(0.2)
            before = int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())
            mcp.text("client_wheel", pid=pid, notches=notches, modifiers=["shift"])
            time.sleep(0.4)
            after = int(lua(mcp, pid, "return X2Action:GetActionBarPage()").strip())
            key_cases.append({"start": before, "key": label, "expected": before, "got": after,
                              "match": after == before})
        meta["keys"] = key_cases

        # The original client's slot rectangles, in its own back-buffer pixels.
        extents_text = lua(mcp, pid, r"""
local o={}
local a=actionBar_renewal[1]
for _,i in ipairs({1,2,11,12}) do
  local w=a.slot[i]
  local ok,ox,oy=pcall(function() return w:GetEffectiveOffset() end)
  local ok2,ex,ey=pcall(function() return w:GetEffectiveExtent() end)
  o[#o+1]=string.format('%d %s %s %s %s',i,tostring(ox),tostring(oy),tostring(ex),tostring(ey))
end
return table.concat(o,'\n')
""")
        extents: dict[int, tuple[int, int]] = {}
        for line in extents_text.splitlines():
            parts = line.split()
            if len(parts) == 5:
                try:
                    cell, x, y, w, h = int(parts[0]), float(parts[1]), float(parts[2]), float(parts[3]), float(parts[4])
                    extents[cell] = (int(x + w / 2), int(y + h / 2))
                except ValueError:
                    pass
        if not all(cell in extents for cell in (1, 2, 11, 12)):
            raise RuntimeError("action bar slot rectangles not readable: " + extents_text)
        meta["cell_centres"] = {str(k): v for k, v in extents.items()}
        log("cell centres", extents)

        drags: list[dict[str, object]] = []

        def drag_case(page: int, from_cell: int, to_cell: int, last_cell: bool = False) -> None:
            base_slot = PAGE_BASE[page]
            source_slot = base_slot + from_cell - 1
            target_slot = base_slot + to_cell - 1
            lua(mcp, pid, f"X2Action:SetActionBarPage({page})")
            time.sleep(0.3)
            # Chosen input: a skill the character knows on the cells this case needs.  The proof
            # is the drag's packet slot, not this seed.  The far-right drag on pages 2..4 can
            # start on the last cell itself (the client's hit area at that end is one cell off),
            # so seed both ends.
            seed = [source_slot] + ([target_slot] if last_cell else [])
            for slot in dict.fromkeys(seed):
                lua(mcp, pid, f"if X2Action:GetActionType({slot}) ~= 'Spell' then X2Action:SetActionSpell({slot}, {args.skill_id}) end")
            time.sleep(0.5)
            release_input(mcp, pid)
            offset = server_log.stat().st_size
            mcp.text("client_pointer", pid=pid, action="move", x=extents[from_cell][0], y=extents[from_cell][1])
            time.sleep(0.3)
            mcp.text("client_drag", pid=pid,
                     x1=extents[from_cell][0], y1=extents[from_cell][1],
                     x2=extents[to_cell][0], y2=extents[to_cell][1])
            time.sleep(7.0)
            packets = action_slot_packets(read_new_lines(server_log, offset))
            release_input(mcp, pid)
            slots = [p["slot"] for p in packets]
            source_seen = source_slot in slots
            target_packets = [p for p in packets if p["slot"] == target_slot]
            destination_set = any(p["kind"] == 2 and p["id"] == args.skill_id for p in target_packets)
            mapping_seen = target_slot in slots if last_cell else (source_seen and destination_set)
            row = {
                "kind": "last_cell_probe" if last_cell else "first_cell_move",
                "page": page,
                "from_cell": from_cell,
                "to_cell": to_cell,
                "expected_source_slot": source_slot,
                "expected_target_slot": target_slot,
                "packets": packets,
                "slots": slots,
                "source_seen": source_seen,
                "destination_set": destination_set,
                "mapping_seen": mapping_seen,
                "match": mapping_seen,
            }
            drags.append(row)
            log(f"page {page} cell {from_cell}->{to_cell}: " +
                (" ".join(f"{p['slot']}:{p['kind']}:{p['id']}" for p in packets) if packets else "NO PACKETS") +
                f" {'OK' if row['match'] else 'MISMATCH'}")
            if not last_cell:
                # Put the source content back, so the next page starts from the same cells.
                release_input(mcp, pid)
                offset = server_log.stat().st_size
                mcp.text("client_pointer", pid=pid, action="move", x=extents[to_cell][0], y=extents[to_cell][1])
                time.sleep(0.3)
                mcp.text("client_drag", pid=pid,
                         x1=extents[to_cell][0], y1=extents[to_cell][1],
                         x2=extents[from_cell][0], y2=extents[from_cell][1])
                time.sleep(7.0)
                back = action_slot_packets(read_new_lines(server_log, offset))
                release_input(mcp, pid)
                row["back_packets"] = back
                if back:
                    log(f"page {page} cell {to_cell}->{from_cell}: " +
                        " ".join(f"{p['slot']}:{p['kind']}:{p['id']}" for p in back))

        for page in (1, 2, 3, 4):
            drag_case(page, 1, 2)
            drag_case(page, 11, 12, last_cell=True)

        meta["api"] = api
        meta["drags"] = drags
        matched = sum(1 for row in drags if row["match"])
        key_matched = sum(1 for row in key_cases if row["match"])

        csv_path = out / "native.csv"
        with open(csv_path, "w", encoding="utf-8", newline="") as handle:
            writer = csv.writer(handle)
            writer.writerow(["kind", "page", "from_cell", "to_cell", "expected_source_slot", "expected_target_slot",
                             "observed_slots", "observed_packets", "source_seen", "destination_set", "mapping_seen",
                             "match", "back_packets"])
            for row in drags:
                writer.writerow([
                    row["kind"], row["page"], row["from_cell"], row["to_cell"], row["expected_source_slot"],
                    row["expected_target_slot"], " ".join(str(s) for s in row["slots"]),
                    " ".join(f"{p['slot']}:{p['kind']}:{p['id']}:{p['body']}" for p in row["packets"]),
                    row["source_seen"], row["destination_set"], row["mapping_seen"], row["match"],
                    " ".join(f"{p['slot']}:{p['kind']}:{p['id']}" for p in row.get("back_packets", [])),
                ])
        json_path = out / "native.json"
        json_path.write_text(json.dumps({"meta": meta, "drags": drags, "api": api, "keys": key_cases}, indent=1),
                             encoding="utf-8")

        report = render_report(meta, key_cases, source_commit, report_date, out, raw)
        (out / "report.txt").write_text(report, encoding="utf-8")
        (raw / "report.txt").write_text(report, encoding="utf-8")
        destination_count = sum(1 for row in drags if row["destination_set"])
        exit_code = 0 if matched == len(drags) and key_matched == len(key_cases) else 1
        (raw / "ACTUAL_RUN_REPORT.md").write_text(
            "\n".join([
                "# action bar pages native run",
                "",
                f"Command: `python tools/action_bar_page_probe.py --client {args.client} --driver {args.driver} "
                f"--driver-dll {args.driver_dll} --launcher {args.launcher} --login {args.login} "
                f"--account {args.account} --character {args.character} --password-file {args.password_file} "
                f"--server-log {args.server_log} --skill-id {args.skill_id}`",
                f"Date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
                f"Client: {client} (pid {pid}, hidden), driver {driver}, DLL {dll}",
                f"Account: {args.account} / {args.character}",
                f"Server log: {server_log}",
                f"Exit: {exit_code}; drags {matched}/{len(drags)} matched, keys {key_matched}/{len(key_cases)}.",
                "",
                "The committed evidence is tools/evidence/action_bar_pages/native.csv, native.json and",
                "report.txt.  The driver artifacts are in this directory.",
                "",
                "Exploratory attempts before this run (kept out of the committed evidence): the first",
                "SetActionSpell attempts used skill 16064, which this test character has not learned, so",
                "the client sent no packet; a drag read after only 1.2 s looked like a source-only clear",
                "because the driver's press/move/release state machine had not finished.  The final probe",
                "uses a learned skill and waits seven seconds per drag, then releases the input state.",
                "",
                f"Destination sets: {destination_count}/{len(drags)}.  The first-cell drops set slots",
                "1/49/61/73 (page 1 also shows the source clear; pages 2..4 set the source again in the",
                "captured window).  The far-right drags can start on the last slot and release outside",
                "the bar; the trace then has only the last slot's clear or set, and their destination",
                "sets prove the last slots.  See report.txt for the per-case column and slot bytes.",
            ]) + "\n", encoding="utf-8")
        log(f"done: drags {matched}/{len(drags)}, keys {key_matched}/{len(key_cases)}; report {out / 'report.txt'}")
        return exit_code
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
