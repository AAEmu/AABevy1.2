#!/usr/bin/env python3
"""Capture one trade-pack sale from the hidden original 1.2 client.

The probe launches the hidden client through AAEmu's `aaemu-clientdriver`
(normal client launch; hidden window), waits for the character in world, finds
the specialty trader by name, right-clicks it (this is the player's own
interaction), reads the quote window through the client's Lua state, clicks
Confirm, and records:

  * the client-visible state before / at the quote / after (money string,
    labor, backpack tooltip, specialty ratio, dialog visibility and button),
  * every C->S body and S->C packet line the game server logs for the run
    (`RUST_LOG=info,aaemu_game::net::fields=trace,aaemu_game::net=debug`),
  * three PNG screenshots in the raw directory (width <= 1280).

Nothing is written into the client and no process memory is read; the client is
only driven through its normal Lua/pointer driver API.  It refuses to run when
an anti-cheat module is loaded.

Usage:
    python tools/specialty_sale_probe.py --client <archeage.exe> \
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \
        --launcher <aaemu-launcher.exe> --login <host:port> \
        --account <account> --character <character> \
        --password-file <password-file> --server-log <rs-game.log> \
        --out tools/evidence/specialty_sale

The server log must contain the C->S field trace; the script takes the byte
offset before the client logs in, so only this run's packets are captured.
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import json
import os
import re
import struct
import subprocess
import time
from ctypes import wintypes
from pathlib import Path

ANTI_CHEAT = ("daruguard", "hshield", "ahn", "xigncode", "mrac", "nprotect",
              "gameguard", "hackshield")
PACKET_RE = re.compile(
    r"\[(?:TRACE|DEBUG)\] fields - (?P<dir>[CS]->[SC]) (?:level=\d+ )?opcode=(?P<opcode>0x[0-9A-Fa-f]+) "
    r"(?P<name>\w+) len=(?P<len>\d+) body=(?P<body>[0-9a-fA-F ]*)")
SENT_RE = re.compile(r"GamePacket: S->C type (?P<opcode>[0-9A-F]{3}) (?P<name>\w+) \((?P<len>\d+) bytes\)")
KEEP_RE = re.compile(r"specialty|Special|CSSellBackpackGoods|CSInteractNPC|CSChangeTarget|"
                     r"CSpecialtyRatio|mail|Mail|item task|ItemTask|labor|Labor")
NOISE = {"SCOneUnitMovementPacket", "SCTargetChangedPacket", "SCGimmickMovementPacket",
         "SCUnitModelPostureChangedPacket", "PongPacket", "PingPacket",
         "SCUnitPointsPacket", "SCSkillFiredPacket", "SCSkillEndedPacket",
         "CompressedGamePackets", "SCTimeOfDayPacket"}


def log(*a: object) -> None:
    print("specialty-sale:", *a, flush=True)


class Mcp:
    def __init__(self, driver: Path, dll: Path, launcher: Path, client: Path,
                 login: str, password_file: Path, user: str, artifact: Path):
        self.process = subprocess.Popen(
            [str(driver), "mcp", "--dll", str(dll), "--client", str(client),
             "--launcher", str(launcher), "--password-file", str(password_file),
             "--login", login, "--user", user, "--artifact-dir", str(artifact)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, encoding="utf-8")
        self.sequence = 0
        self.call("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                 "clientInfo": {"name": "specialty-sale-probe", "version": "1"}})
        assert self.process.stdin and self.process.stdout
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0",
                                             "method": "notifications/initialized"}) + "\n")
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
        return "\n".join(c["text"] for c in self.tool(name, **arguments)["content"]
                         if c.get("type") == "text")

    def close(self) -> None:
        try:
            assert self.process.stdin
            self.process.stdin.close()
            self.process.wait(timeout=15)
        except Exception:
            self.process.kill()


class Modules:
    """Toolhelp module names of one process (no memory reads)."""

    def __init__(self, pid: int):
        kernel = ctypes.WinDLL("kernel32", use_last_error=True)

        class ME(ctypes.Structure):
            _fields_ = [("dwSize", wintypes.DWORD), ("th32ModuleID", wintypes.DWORD),
                        ("th32ProcessID", wintypes.DWORD), ("GlblcntUsage", wintypes.DWORD),
                        ("ProccntUsage", wintypes.DWORD), ("modBaseAddr", ctypes.c_void_p),
                        ("modBaseSize", wintypes.DWORD), ("hModule", wintypes.HMODULE),
                        ("szModule", ctypes.c_wchar * 256), ("szExePath", ctypes.c_wchar * 260)]

        snap = kernel.CreateToolhelp32Snapshot(0x18, pid)
        self.names: list[str] = []
        me = ME()
        me.dwSize = ctypes.sizeof(ME)
        ok = kernel.Module32FirstW(snap, ctypes.byref(me))
        while ok:
            self.names.append(me.szModule)
            ok = kernel.Module32NextW(snap, ctypes.byref(me))
        kernel.CloseHandle(snap)


def lua(mcp: Mcp, pid: int, code: str, target: str = "ui") -> str:
    return mcp.text("client_lua", pid=pid, target=target, code=code).strip()


def shoot(mcp: Mcp, pid: int, path: Path) -> int:
    from PIL import Image
    import io
    content = mcp.tool("client_screenshot", pid=pid)["content"]
    image = next(c for c in content if c["type"] == "image")
    raw = base64.b64decode(image["data"], validate=True)
    img = Image.open(io.BytesIO(raw)).convert("RGB")
    if img.width > 1280:
        img = img.resize((1280, round(img.height * 1280 / img.width)), Image.LANCZOS)
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path, "PNG")
    return img.width


def wait_stage(mcp: Mcp, pid: int, stage: int, timeout: float) -> None:
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            last = lua(mcp, pid, "return X2LoginCharacter and X2LoginCharacter:GetCurrentStage()")
            if last == str(stage):
                return
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError(f"stage {stage} not reached: {last}")


def wait_player(mcp: Mcp, pid: int, timeout: float, expected: str) -> None:
    end = time.monotonic() + timeout
    last = ""
    while time.monotonic() < end:
        try:
            state = lua(mcp, pid,
                        "return X2LoginCharacter:GetCurrentStage(), X2Unit and X2Unit:UnitName('player')")
            stage, _, name = state.partition("\t")
            if stage == "0" and name not in ("", "nil"):
                if name != expected:
                    raise RuntimeError(f"logged-in character {name!r} is not {expected!r}")
                return
            last = state
        except Exception as error:
            last = str(error)
        time.sleep(0.5)
    raise RuntimeError("player did not appear: " + last)


def snapshot(mcp: Mcp, pid: int) -> dict:
    code = """
local function tooltip()
  local ok, t = pcall(function() return X2Equipment:GetEquippedItemTooltipText('player', ES_BACKPACK) end)
  if not ok then return 'error:' .. tostring(t) end
  if not t then return 'nil' end
  return string.format('%s|%s|%s|%s|%s|%s', tostring(t.itemType), tostring(t.itemGrade),
    tostring(t.stack), tostring(t.refund), tostring(t.name), tostring(t.lookType))
end
local dialog = GetSpecialtyStoreMessageBoxHandleWindow and GetSpecialtyStoreMessageBoxHandleWindow()
local player_name = X2Unit:UnitName('player')
local money = X2Util:GetMyMoneyString()
local labor = X2Player:GetLaborPower()
local ratio = X2Store:GetSpecialtyRatio()
local target = X2Unit:GetTargetUnitId()
local target_name = X2Unit:UnitName('target')
local out = {
  'name=' .. tostring(player_name),
  'money=' .. tostring(money),
  'labor=' .. tostring(labor),
  'backpack=' .. tooltip(),
  'ratio=' .. tostring(ratio),
  'target=' .. tostring(target),
  'targetname=' .. tostring(target_name),
}
local function size(w)
  local ok, x, y = pcall(function() return w:GetWidth(), w:GetHeight() end)
  if ok and type(x) == 'number' then return x, y end
  ok, x, y = pcall(function() return w:GetEffectiveExtent() end)
  if ok and type(x) == 'number' then return x, y end
  ok, x, y = pcall(function() return w:GetExtent() end)
  if ok and type(x) == 'number' then return x, y end
  return 0, 0
end
if dialog then
  out[#out+1] = 'dialog=' .. tostring(dialog:IsVisible())
  local ox, oy = dialog:GetEffectiveOffset()
  local ex, ey = size(dialog)
  out[#out+1] = string.format('dialogpos=%s,%s,%s,%s', ox, oy, ex, ey)
  local b = dialog.btnOk or dialog.okButton
  if b then
    local bx, by = b:GetEffectiveOffset()
    local bw, bh = size(b)
    local visible = b:IsVisible()
    local enabled = b:IsEnabled()
    out[#out+1] = string.format('ok=%s,%s,%s,%s,%s', tostring(visible), bx, by, bw, bh)
    out[#out+1] = 'okenabled=' .. tostring(enabled)
  end
end
local ux, uy = size(UIParent)
out[#out+1] = string.format('uiparent=%s,%s', ux, uy)
return table.concat(out, '\\n')
"""
    text = lua(mcp, pid, code)
    data: dict[str, str] = {}
    for line in text.splitlines():
        key, _, value = line.partition("=")
        data[key.strip()] = value.strip()
    return data


def npc_world(mcp: Mcp, pid: int, name: str) -> list[float] | None:
    code = """
local want = NAME
for _, e in pairs(System.GetEntities() or {}) do
  local ok, n = pcall(function() return e:GetName() end)
  if ok and n == want then
    local p = e:GetWorldPos()
    return string.format('%.4f\\t%.4f\\t%.4f', p.x, p.y, p.z)
  end
end
return ''
""".replace("NAME", json.dumps(name))
    values = lua(mcp, pid, code, "system").split()
    return [float(v) for v in values] if len(values) == 3 else None


def click(mcp: Mcp, pid: int, x: int, y: int, right: bool) -> None:
    mcp.text("client_pointer", pid=pid, action="right_click" if right else "click", x=x, y=y)


def collect_log(path: Path, offset: int) -> tuple[list[dict], list[str]]:
    if not path.exists():
        return [], []
    text = path.read_bytes()[offset:].decode("utf-8", "replace")
    packets: list[dict] = []
    lines: list[str] = []
    for line in text.splitlines():
        match = PACKET_RE.search(line)
        if match:
            packets.append({
                "dir": match.group("dir"),
                "opcode": match.group("opcode"),
                "name": match.group("name"),
                "len": int(match.group("len")),
                "body": match.group("body").strip(),
            })
            continue
        sent = SENT_RE.search(line)
        if sent:
            packets.append({"dir": "S->C", "opcode": "0x" + sent.group("opcode"),
                            "name": sent.group("name"), "len": int(sent.group("len")), "body": ""})
            continue
        if KEEP_RE.search(line):
            lines.append(line.strip())
    return packets, lines


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("SALE_CLIENT", "archeage.exe"))
    parser.add_argument("--driver", default=os.environ.get("SALE_DRIVER", "aaemu-clientdriver.exe"))
    parser.add_argument("--driver-dll", default=os.environ.get("SALE_DRIVER_DLL", "aadriver.dll"))
    parser.add_argument("--launcher", default=os.environ.get("SALE_LAUNCHER", "aaemu-launcher.exe"))
    parser.add_argument("--login", default=os.environ.get("SALE_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")))
    parser.add_argument("--account", default=os.environ.get("SALE_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("SALE_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("SALE_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("SALE_SERVER_LOG"))
    parser.add_argument("--npc", default=os.environ.get("SALE_NPC", "Hedge"),
                        help="specialty trader entity name")
    parser.add_argument("--out", default=os.environ.get("SALE_OUT", "tools/evidence/specialty_sale"))
    parser.add_argument("--raw", default=os.environ.get("SALE_RAW"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set SALE_{name.upper()})")
    client = Path(args.client)
    driver = Path(args.driver)
    dll = Path(args.driver_dll)
    launcher = Path(args.launcher)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    out = Path(args.out)
    raw = Path(args.raw) if args.raw else Path("data/extract") / f"specialty-sale-{time.strftime('%Y%m%d')}-r1"
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)
    for path in (client, driver, dll, launcher, password_file):
        if not path.exists():
            raise SystemExit(f"missing {path}")
    offset = server_log.stat().st_size if server_log.exists() else 0

    mcp = Mcp(driver, dll, launcher, client, args.login, password_file, args.account, raw / "mcp")
    pid = None
    result: dict = {"account": "<account>", "character": "<character>", "npc": args.npc,
                    "client": "1.2 r208022 (hidden)", "password_file": "<password-file>",
                    "server_log": "<server-log>"}
    try:
        launch = mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                          user=args.account, password_file=str(password_file))
        log(launch.replace("\n", " | "))
        pid = int(re.search(r"pid (\d+)", launch).group(1))
        result["client_pid"] = pid
        anti = [m for m in Modules(pid).names if any(bad in m.lower() for bad in ANTI_CHEAT)]
        if anti:
            raise RuntimeError(f"anti-cheat module loaded: {anti}; refusing to drive the client")
        wait_stage(mcp, pid, 3, 240)
        end = time.monotonic() + 120
        stage = ""
        while time.monotonic() < end:
            lua(mcp, pid, "X2World:EnterWorld(1,0)")
            for _ in range(20):
                try:
                    stage = lua(mcp, pid, "return X2LoginCharacter:GetCurrentStage()")
                except Exception:
                    stage = ""
                if stage == "4":
                    break
                time.sleep(0.5)
            if stage == "4":
                break
        if stage != "4":
            raise RuntimeError("character select stage not reached: " + stage)
        result["stage_after_enter"] = stage
        log("character select")
        lua(mcp, pid, "local b=characterSelectWindow.bottomPanel.startButton b:OnClick('LeftButton')")
        wait_player(mcp, pid, 240, args.character)
        time.sleep(3.0)

        result["before"] = snapshot(mcp, pid)
        shoot(mcp, pid, raw / "before.png")
        log("before", json.dumps(result["before"], ensure_ascii=False))

        world = npc_world(mcp, pid, args.npc)
        if not world:
            raise RuntimeError(f"entity {args.npc!r} not found")
        screen_text = lua(mcp, pid, "return X2Util:ConvertWorldToScreen(%.6f,%.6f,%.6f)"
                          % (world[0], world[1], world[2]))
        screen = [float(v) for v in screen_text.replace("\t", " ").split()]
        result["npc_world"] = world
        result["npc_screen"] = screen
        log("npc", world, "screen", screen)

        # The driver pointer is in the screenshot pixels; try the raw viewport
        # point first, then the DPI-scaled one, until the NPC is the target.
        shot_w = shoot(mcp, pid, raw / "probe-before.png")
        ui = result["before"].get("uiparent", "0,0").split(",")
        sx, sy = screen[0], screen[1]
        factors = [1.0]
        if len(ui) == 2 and float(ui[0]) > 0:
            factors.append(shot_w / float(ui[0]))
        factors = sorted({round(f, 4) for f in factors})
        clicked = None
        for factor in factors:
            x, y = int(round(sx * factor)), int(round(sy * factor))
            for dx, dy in ((0, 0), (0, -40), (0, 40), (-30, 0), (30, 0)):
                click(mcp, pid, x + dx, y + dy, right=True)
                time.sleep(0.6)
                name = lua(mcp, pid, "return X2Unit:UnitName('target')")
                if name == args.npc:
                    clicked = (x + dx, y + dy, factor)
                    break
            if clicked:
                break
        if not clicked:
            raise RuntimeError("right-click did not target the specialty trader")
        result["interaction_click"] = clicked
        log("targeted", clicked)
        time.sleep(2.0)

        info_code = """
local out = {}
local function call(name, ...)
  local fn = X2Interaction[name]
  if type(fn) ~= 'function' then out[#out+1] = name .. '=missing' return nil end
  local ok, a, b, c = pcall(fn, X2Interaction, ...)
  if not ok then out[#out+1] = name .. '=error:' .. tostring(a) return nil end
  out[#out+1] = string.format('%s=%s|%s|%s', name, tostring(a), tostring(b), tostring(c))
  return a
end
local n = call('GetInteractionCount') or call('GetNpcInteractionCount') or call('GetInteractionSkillCount') or 0
out[#out+1] = 'count=' .. tostring(n)
for i = 0, 5 do
  call('GetNpcInteractionText', i)
  call('GetInteractionSkillId', i)
  call('GetInteractionSkillName', i)
  call('GetNpcInteractionMsg', i)
end
return table.concat(out, '\\n')
"""
        info_text = lua(mcp, pid, info_code)
        result["interaction_list"] = info_text.splitlines()
        log("interactions", info_text.replace("\n", " ; "))
        shoot(mcp, pid, raw / "probe-menu.png")

        def quote_state() -> tuple[bool, str]:
            state = lua(mcp, pid, """
local w = GetSpecialtyStoreMessageBoxHandleWindow and GetSpecialtyStoreMessageBoxHandleWindow()
local visible = w and w:IsVisible()
return tostring(visible), tostring(X2Store:GetSpecialtyRatio())
""").replace("\t", " ").split()
            visible = state[0] == "true" if state else False
            ratio = state[1] if len(state) > 1 else ""
            return visible, ratio

        opened = quote_state()
        if not opened[0]:
            # The original asks for the specialty rate when the player picks
            # the trader's specialty entry in the NPC interaction list.
            result["quote_open"] = None
            for index in range(3):
                for selector in (f"X2Interaction:SelectNpcInteraction({index},0)",
                                 f"X2Interaction:SelectNpcInteraction({index + 1},0)"):
                    lua(mcp, pid, selector + "\nX2Interaction:ConfirmNPCInteraction()\nreturn 1")
                    time.sleep(1.2)
                    opened = quote_state()
                    if opened[0] or opened[1] not in ("", "100"):
                        result["quote_open"] = selector
                        break
                if result.get("quote_open"):
                    break
            if not result.get("quote_open"):
                mcp.text("client_input", pid=pid, key="f", duration_ms=80)
                time.sleep(1.5)
                opened = quote_state()
                if opened[0]:
                    result["quote_open"] = "key f"
            if not result.get("quote_open"):
                raise RuntimeError("the specialty quote window did not open")
            log("quote opened by", result["quote_open"], "ratio", opened[1])
        time.sleep(1.0)
        result["quote"] = snapshot(mcp, pid)
        shoot(mcp, pid, raw / "quote.png")
        log("quote", json.dumps(result["quote"], ensure_ascii=False))

        def sell_packets() -> list[dict]:
            packets, _ = collect_log(server_log, offset)
            return [p for p in packets if p["name"] == "CSSellBackpackGoodsPacket"]

        def dialog_visible() -> bool:
            return lua(mcp, pid,
                       "local w=GetSpecialtyStoreMessageBoxHandleWindow() return w and w:IsVisible()") == "true"

        # Confirm through the dialog's own button.  First a physical pointer
        # click at the button geometry the client itself reports; if the
        # hidden window's pointer does not deliver, invoke the same X2 button
        # handler the pointer dispatch calls.  After every attempt stop as soon
        # as the server has the sell packet, so the pack is never sold twice.
        ok = result["quote"].get("ok", "").split(",")
        pos = result["quote"].get("dialogpos", "").split(",")
        ui_w = float(ui[0]) if len(ui) == 2 and float(ui[0]) > 0 else 0.0
        factors = [1.0]
        if ui_w > 0:
            factors.append(shot_w / ui_w)
        factors = sorted({round(f, 4) for f in factors})
        points: list[tuple[float, float]] = []
        if len(ok) == 5:
            bx, by, bw, bh = (float(v) for v in ok[1:])
            points.append((bx + bw / 2, by + bh / 2))
        if len(pos) == 4:
            x, y, w, h = (float(v) for v in pos)
            points += [(x + w * 0.405, y + h * 0.87), (x + w * 0.40, y + 300)]
        result["confirm"] = {"points": points, "factors": factors, "physical": None,
                             "lua": None, "sent": False}
        for px0, py0 in points:
            for factor in factors:
                px, py = int(round(px0 * factor)), int(round(py0 * factor))
                if px <= 0 or py <= 0 or px >= shot_w or py >= 2000:
                    continue
                click(mcp, pid, px, py, right=False)
                time.sleep(1.0)
                if sell_packets() or not dialog_visible():
                    result["confirm"]["physical"] = [px, py, factor]
                    break
            if result["confirm"]["physical"]:
                break
        if not sell_packets() and dialog_visible():
            for handler in ("local b=GetSpecialtyStoreMessageBoxHandleWindow().btnOk "
                            "b:OnClick('LeftButton')",
                            "local b=GetSpecialtyStoreMessageBoxHandleWindow().okButton "
                            "b:OnClick('LeftButton')",
                            "local b=GetSpecialtyStoreMessageBoxHandleWindow().btnOk b:OnClick()"):
                try:
                    lua(mcp, pid, handler + "\nreturn 1")
                except Exception as error:
                    result["confirm"].setdefault("lua_errors", []).append(str(error))
                    continue
                time.sleep(1.5)
                if sell_packets() or not dialog_visible():
                    result["confirm"]["lua"] = handler
                    break
        if not sell_packets() and dialog_visible():
            # This is the dialog's own OK action (`OkProc` calls it in
            # message_box_from_npc_interaction.alb); the hidden window's
            # pointer cannot reach the button.
            lua(mcp, pid, "X2Store:SellBackPackGoods() return 1")
            result["confirm"]["lua_api"] = "X2Store:SellBackPackGoods()"
            time.sleep(1.5)
        result["confirm"]["sent"] = bool(sell_packets())
        log("confirmed", json.dumps(result["confirm"], ensure_ascii=False))
        time.sleep(4.0)
        result["after"] = snapshot(mcp, pid)
        shoot(mcp, pid, raw / "after.png")
        log("after", json.dumps(result["after"], ensure_ascii=False))

        packets, lines = collect_log(server_log, offset)
        kept: list[dict] = []
        noise: dict[str, int] = {}
        for packet in packets:
            if packet["dir"] == "S->C" and packet["name"] in NOISE:
                noise[packet["name"]] = noise.get(packet["name"], 0) + 1
            else:
                kept.append(packet)
        result["packets"] = kept
        result["excluded_packets"] = dict(sorted(noise.items()))
        result["server_lines"] = lines
        (out / "native.json").write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n",
                                         encoding="utf-8")
        report = [
            "specialty sale from the original 1.2 client (hidden, driven through aaemu-clientdriver)",
            f"account <account> / character <character>, npc {args.npc}",
            f"date {time.strftime('%Y-%m-%d %H:%M:%S')}",
            f"sell packet sent: {result['confirm']['sent']}",
            "",
            "before: " + json.dumps(result["before"], ensure_ascii=False),
            "quote:  " + json.dumps(result["quote"], ensure_ascii=False),
            "after:  " + json.dumps(result["after"], ensure_ascii=False),
            "",
            "packets:",
        ]
        report += [f"  {p['dir']} {p['opcode']} {p['name']} len={p['len']} body={p['body']}"
                   for p in kept]
        report += [f"  ({count} {name} omitted)" for name, count in result["excluded_packets"].items()]
        report += ["", "server lines:"] + ["  " + line for line in lines]
        (out / "native-run.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
        log("wrote", out / "native.json")
        return 0
    except BaseException as error:
        (raw / "probe-error.txt").write_text(
            f"{type(error).__name__}: {error}\npartial state:\n"
            + json.dumps(result, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        raise
    finally:
        if pid:
            try:
                mcp.text("client_stop", pid=pid)
                log("stopped client pid", pid)
            except Exception as error:
                log("client_stop failed", error)
        mcp.close()


if __name__ == "__main__":
    raise SystemExit(main())
