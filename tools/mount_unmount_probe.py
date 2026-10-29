#!/usr/bin/env python3
"""Capture the 1.2 client's unmount/dismiss reasons, ride cooldown and CanMount reach.

The original client's `x2game.dll` sends the reason byte of `CSUnMountMate`
from `FUN_39361e80` (reason 9 from `X2Mate:UnMountMate`, 10 from
`X2Mate:DismissPet` while mounted), gates mount and unmount on the same
two-second `_DAT_3a169944` stamp, and refuses `CanMount` beyond a squared
distance of 625 (25 m) in `FUN_3935aa10`.  This probe drives the hidden
original through aaemu-clientdriver (no OS input, no injection beyond the
normal client driver), records the server's `C->S` trace lines, and reads the
player/mate positions from the process (PROCESS_VM_READ only) to measure the
distance at which a mount request is refused.  The client's own Lua state is
wrapped so the visible system/chat text of each refusal is kept.

Cases: mount then self-unmount; mount again inside and after the two-second
cooldown; mount attempts around the reach; mount then dismiss while mounted.

Usage:
    python tools/mount_unmount_probe.py --client <archeage.exe> \\
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \\
        --launcher <aaemu-launcher.exe> --login <host:port> \\
        --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --out tools/evidence/mount_unmount
    python tools/mount_unmount_probe.py --pid 1234 ...  # already in world

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace`.  The script reads that log at
`--server-log`; its own packet lines are used for the opcode, body and the
decoded reason.  The 1.2 client must be hidden (`client_launch ... hidden`),
which this script does.  It refuses to run if an anti-cheat module is loaded.

The account may be new: with AutoAccount the first login creates it.  A
missing character is created through the original creation UI when
`--create-character` is passed.
"""
from __future__ import annotations

import argparse
import csv
import json
import math
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mount_reason_probe as base

ROOT = Path(__file__).resolve().parents[1]
ITEM = 8159  # Brown Lilyut Horse
HORSE_NAME = "Brown Lilyut Horse"

CAPTURE_INSTALL = r"""
_CAP = {}
local function _cap_add(name, ...)
  local parts = {}
  for i = 1, select('#', ...) do
    local value = select(i, ...)
    local kind = type(value)
    if kind == 'string' or kind == 'number' or kind == 'boolean' then
      parts[#parts + 1] = tostring(value)
    end
  end
  _CAP[#_CAP + 1] = name .. ': ' .. table.concat(parts, ' | ')
end
for _, name in ipairs({'AddMsgToChatWin', 'AddMessageToSysMsgWindow', 'OnCreateChatMessage',
                       'DispatchChatMessage', 'AddMessageTo', 'ShowEffectMsgMessage'}) do
  local original = _G[name]
  if type(original) == 'function' then
    _G[name] = function(...) _cap_add(name, ...); return original(...) end
  end
end
return 'ok'
"""

CAPTURE_CLEAR = "_CAP = {} return 'ok'"
CAPTURE_READ = "return table.concat(_CAP or {}, '\\n')"

PACKET_RE = re.compile(r"C->S level=1 opcode=0x([0-9A-F]{3}) (\w+) len=(\d+) body=(.*)$")
MOUNT_RE = re.compile(r"CSMountMatePacket fields: CSMountMatePacket \{ tl_id: (\d+), ap: (\d+), reason: (\d+) \}")
UNMOUNT_RE = re.compile(r"CSUnMountMatePacket fields: CSUnMountMatePacket \{ tl_id: (\d+), ap: (\d+), reason: (\d+) \}")
TARGET_RE = re.compile(r"CSChangeMateTargetPacket fields: CSChangeMateTargetPacket \{ tl_id: (\d+), obj_id: (\d+) \}")


def log(*args: object) -> None:
    print("mount-unmount:", *args, flush=True)


def sys_text(mcp: base.Mcp, pid: int, code: str) -> str:
    try:
        return mcp.text("client_lua", pid=pid, target="system", code=code)
    except Exception:
        return ""


def lua_text(mcp: base.Mcp, pid: int, code: str) -> str:
    for _ in range(20):
        try:
            return mcp.text("client_lua", pid=pid, target="ui", code=code)
        except Exception:
            time.sleep(0.1)
    return ""


def create_character(mcp: base.Mcp, pid: int, name: str, allowed: bool) -> None:
    stage = lua_text(mcp, pid, "return X2LoginCharacter:GetCurrentStage()").strip()
    count = int(lua_text(mcp, pid, "return X2LoginCharacter:GetNumLoginCharacters()").strip() or 0) if stage == "4" else 0
    if count == 0:
        if not allowed:
            raise RuntimeError("the account has no character; pass --create-character")
        if stage == "4":
            lua_text(mcp, pid, "X2LoginCharacter:CreateCharacter()")
        for _ in range(240):
            if lua_text(mcp, pid, "return characterCreate and characterCreate.state").strip() == "Step1":
                break
            time.sleep(0.25)
        time.sleep(1.0)
        lua_text(mcp, pid, "characterCreate.OnNextStep()")
        for _ in range(240):
            if lua_text(mcp, pid, "return characterCreate.state").strip() == "Step2":
                break
            time.sleep(0.25)
        time.sleep(1.0)
        lua_text(mcp, pid, "characterCreate.OnNextStep()")
        for _ in range(240):
            if lua_text(mcp, pid, "return characterCreate.state").strip() == "Step3":
                break
            time.sleep(0.25)
        lua_text(
            mcp,
            pid,
            f"SelectAbility(1) step3.content.nameWnd.characterName:SetText({json.dumps(name)}) CreateCharacter(false)",
        )
        base.wait_stage(mcp, pid, 4, 120)
    names = [lua_text(mcp, pid, f"return X2LoginCharacter:GetLoginCharacterName({i})").strip() for i in range(1, 5)]
    names = [n for n in names if n and n != "nil"]
    if name not in names:
        raise RuntimeError(f"character {name!r} not in {names}")
    lua_text(mcp, pid, f"X2LoginCharacter:SelectCharacter({names.index(name) + 1})")
    lua_text(mcp, pid, "characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')")
    base.wait_player(mcp, pid, 240)


class Trace:
    def __init__(self, path: Path):
        self.path = path
        self.offset = path.stat().st_size if path.exists() else 0
        self.lines: list[tuple[float, str]] = []

    def reset(self) -> None:
        self.offset = self.path.stat().st_size if self.path.exists() else 0

    def poll(self) -> list[dict[str, object]]:
        if not self.path.exists():
            return []
        data = self.path.read_bytes()[self.offset:].decode("utf-8", "replace")
        if not data:
            return []
        complete = data.rsplit("\n", 1)[0] if not data.endswith("\n") else data
        consumed = len(complete.encode("utf-8"))
        self.offset += consumed
        now = time.monotonic()
        packets: list[dict[str, object]] = []
        for line in complete.splitlines():
            self.lines.append((now, line))
            match = MOUNT_RE.search(line)
            if match:
                packets.append({
                    "kind": "mount", "opcode": 0xA7, "tl_id": int(match.group(1)),
                    "ap": int(match.group(2)), "reason": int(match.group(3)), "at": now, "line": line.strip(),
                })
                continue
            match = UNMOUNT_RE.search(line)
            if match:
                packets.append({
                    "kind": "unmount", "opcode": 0xA8, "tl_id": int(match.group(1)),
                    "ap": int(match.group(2)), "reason": int(match.group(3)), "at": now, "line": line.strip(),
                })
                continue
            match = TARGET_RE.search(line)
            if match:
                packets.append({
                    "kind": "mate_target", "opcode": 0xA5, "tl_id": int(match.group(1)),
                    "obj_id": int(match.group(2)), "at": now, "line": line.strip(),
                })
                continue
            match = PACKET_RE.search(line)
            if match and int(match.group(1), 16) in (0xA4, 0xA5, 0xA7, 0xA8):
                opcode = int(match.group(1), 16)
                packets.append({
                    "kind": {0xA4: "remove", 0xA5: "mate_target_raw", 0xA7: "mount_raw", 0xA8: "unmount_raw"}[opcode],
                    "opcode": opcode, "tl_id": None, "ap": None, "reason": None, "at": now,
                    "len": int(match.group(3)), "body": match.group(4).strip(), "line": line.strip(),
                })
        return packets


def wait_packets(trace: Trace, wanted: set[str], timeout: float) -> list[dict[str, object]]:
    seen: list[dict[str, object]] = []
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        for packet in trace.poll():
            if packet["kind"] in wanted or packet["kind"].replace("_raw", "") in wanted:
                seen.append(packet)
        if wanted <= {packet["kind"] for packet in seen}:
            break
        time.sleep(0.02)
    return seen


def wait_for(check, timeout: float, interval: float = 0.1):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        last = check()
        if last:
            return last
        time.sleep(interval)
    return last


def capture(mcp: base.Mcp, pid: int) -> str:
    return lua_text(mcp, pid, CAPTURE_READ).strip()


def teleport(mcp: base.Mcp, pid: int, character: str, x: float, y: float, z: float, wait: float = 2.5) -> None:
    mcp.text("client_chat", pid=pid, message=f"/move {character} {x:.2f} {y:.2f} {z:.2f}")
    time.sleep(wait)


def distance2(a: list[float], b: list[float]) -> float:
    return (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2


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
                        help="game server log with the fields trace")
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--bot-account", default=os.environ.get("MOUNT_BOT_ACCOUNT"),
                        help="second account whose summoned horse stays put for the reach sweep")
    parser.add_argument("--bot-character", default=os.environ.get("MOUNT_BOT_CHARACTER"))
    parser.add_argument("--bot-password-file", default=os.environ.get("MOUNT_BOT_PASSWORD_FILE"),
                        help="defaults to --password-file")
    parser.add_argument("--create-character", action="store_true",
                        help="create the character through the original creation UI when the account has none")
    parser.add_argument("--distances", default="20,24,24.8,25,25.2,26,30",
                        help="comma-separated player-to-horse distances (m) for the reach sweep")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "mount_unmount"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-unmount-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set MOUNT_{name.upper()})")
    if args.bot_account and not args.bot_character:
        parser.error("--bot-character is required with --bot-account")
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
    distances = [float(value) for value in args.distances.split(",") if value.strip()]
    out = Path(args.out)
    raw = Path(args.raw)
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)
    json_path = out / "native.json"
    csv_path = out / "native.csv"
    rows: list[dict[str, object]] = []
    trace = Trace(server_log)

    log(f"account {args.account}: {args.character}")
    mcp = base.Mcp(driver, dll, launcher, client, args.login, password_file, args.account, raw / "mcp")
    pid = args.pid
    bot_pid = None
    bot_obj = None
    bot_mcp = None
    bot_name = "Brown Lilyut Horse"
    bot_uid = ""
    bot_horse_pos = (0.0, 0.0, 0.0)
    bot_stop = threading.Event()
    bot_keepalive = None
    try:
        if pid is None:
            launch = mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                              user=args.account, password_file=str(password_file))
            log(launch.replace("\n", " | "))
            match = re.search(r"pid (\d+)", launch)
            if not match:
                raise RuntimeError("client_launch did not return a pid")
            pid = int(match.group(1))
            base.wait_stage(mcp, pid, 3, 240)
            lua_text(mcp, pid, "X2World:EnterWorld(1,0)")
            for _ in range(240):
                if lua_text(mcp, pid, "return X2LoginCharacter:GetCurrentStage()").strip() in ("4", "5"):
                    break
                time.sleep(0.5)
            create_character(mcp, pid, args.character, args.create_character)
            time.sleep(5.0)
        else:
            base.wait_player(mcp, pid, 30)

        install = lua_text(mcp, pid, CAPTURE_INSTALL).strip()
        if install != "ok":
            log("text capture install:", install)

        mem = base.Mem(pid)
        for name, _ in mem.modules():
            lower = name.lower()
            if any(bad in lower for bad in base.ANTI_CHEAT):
                raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
        base_addr = mem.module_base("x2game.dll")
        if not base_addr:
            raise RuntimeError("x2game.dll not loaded")
        log("x2game base", hex(base_addr))

        mate_unit: int | None = None

        def locate_mate(obj_id: int | None = None) -> int:
            nonlocal mate_unit
            if obj_id:
                _, units = base.walk_units(mem, base_addr)
                for key, unit in units:
                    if key == obj_id:
                        mate_unit = unit
                        log("pet unit", obj_id)
                        return mate_unit
            horse = base.lua_pos(mcp, pid, HORSE_NAME)
            found = base.identify(mem, base_addr, horse) if horse else None
            if not found:
                raise RuntimeError("the summoned horse has no unit")
            mate_unit = found[1]
            log("pet unit by position", hex(mate_unit))
            return mate_unit

        def states(timeout: float = 8.0) -> tuple[int, list[float], list[float], list[float]]:
            end = time.monotonic() + timeout
            while True:
                player_unit = mem.u32(mem.u32(base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)
                if mate_unit is None:
                    try:
                        locate_mate()
                    except RuntimeError:
                        pass
                if mate_unit is not None:
                    player_rot, player_pos = base.unit_state(mem, player_unit)
                    mate_rot, mate_pos = base.unit_state(mem, mate_unit)
                    if player_pos and mate_pos:
                        return player_unit, player_pos, mate_pos, mate_rot
                if time.monotonic() >= end:
                    log("states lost: pet", lua_text(mcp, pid, "return tostring(X2Mate:IsPlayerPetExists())").strip(),
                        "mounting", lua_text(mcp, pid, "return tostring(X2Mate:IsPlayerMounting())").strip(),
                        "mate_unit", mate_unit, "player_unit", player_unit)
                    raise RuntimeError("player or horse position lost")
                time.sleep(0.2)

        def reset_pet() -> None:
            if not base.pet_exists(mcp, pid):
                return
            for _ in range(2):
                lua_text(mcp, pid, "X2Mate:DismissPet() return 1")
                if wait_for(lambda: not base.pet_exists(mcp, pid), 6.0, 0.1):
                    break
                time.sleep(2.1)
            if base.pet_exists(mcp, pid):
                raise RuntimeError("the previous pet could not be dismissed")
            time.sleep(2.2)

        def summon() -> None:
            trace.reset()
            if not base.pet_exists(mcp, pid):
                try:
                    mcp.text("client_use_item", pid=pid, item_template_id=ITEM)
                except RuntimeError:
                    mcp.text("client_chat", pid=pid, message=f"/item add self {ITEM} 1")
                    time.sleep(1.5)
                    mcp.text("client_use_item", pid=pid, item_template_id=ITEM)
                wait_for(lambda: base.pet_exists(mcp, pid), 25, 0.3)
            if not base.pet_exists(mcp, pid):
                raise RuntimeError("the horse could not be summoned")
            time.sleep(2.0)
            targets = wait_packets(trace, {"mate_target"}, 3.0)
            pet_obj = int(targets[-1]["obj_id"]) if targets else None
            locate_mate(pet_obj)
            states()

        def ensure_dismounted() -> None:
            if base.mounted(mcp, pid):
                base.dismount(mcp, pid)
            time.sleep(0.2)

        def mount_near(timeout: float = 8.0) -> dict[str, object]:
            player_unit, player_pos, mate_pos, _ = states()
            if distance2(player_pos, mate_pos) > 8.0:
                teleport(mcp, pid, args.character, mate_pos[0] + 2.2, mate_pos[1], mate_pos[2], wait=2.5)
                player_unit, player_pos, mate_pos, _ = states()
            for _ in range(20):
                if lua_text(mcp, pid, "return tostring(X2Mate:CanMount())").strip() == "true":
                    break
                time.sleep(0.25)
            base.settle(mem, player_unit, mate_unit)
            player_unit, player_pos, mate_pos, _ = states()
            log("mount_near dist2", round(distance2(player_pos, mate_pos), 3),
                "CanMount", lua_text(mcp, pid, "return tostring(X2Mate:CanMount())").strip(),
                "mounting", lua_text(mcp, pid, "return tostring(X2Mate:IsPlayerMounting())").strip(),
                "pet", lua_text(mcp, pid, "return tostring(X2Mate:IsPlayerPetExists())").strip(),
                "mountable", lua_text(mcp, pid, "return tostring(X2Mate:IsMountablePet())").strip(),
                "state", lua_text(mcp, pid, "return tostring(X2Mate:PetState())").strip(),
                "canDismiss", lua_text(mcp, pid, "return tostring(X2Mate:CanDismiss())").strip())
            trace.reset()
            lua_text(mcp, pid, CAPTURE_CLEAR)
            start = time.monotonic()
            lua_text(mcp, pid, "X2Mate:MountMate() return 1")
            packets = wait_packets(trace, {"mount"}, timeout)
            mounted = bool(wait_for(lambda: base.mounted(mcp, pid), 4.0, 0.1))
            log("mount_near result packets", [(p.get("kind"), p.get("reason")) for p in packets],
                "mounted", mounted, "text", capture(mcp, pid)[:160])
            return {
                "packets": packets,
                "mounted": mounted,
                "elapsed_ms": round((time.monotonic() - start) * 1000.0, 1),
                "text": capture(mcp, pid),
            }

        bot_pid = None
        bot_obj = None
        if args.bot_account:
            bot_pw = Path(args.bot_password_file or args.password_file)
            bot_mcp = base.Mcp(driver, dll, launcher, client, args.login, bot_pw, args.bot_account, raw / "mcp-bot")
            launch = bot_mcp.text("client_launch", fast_start=True, fast_pak=True, hidden=True,
                                  user=args.bot_account, password_file=str(bot_pw))
            log("bot", launch.replace("\n", " | "))
            match = re.search(r"pid (\d+)", launch)
            if not match:
                raise RuntimeError("bot client_launch did not return a pid")
            bot_pid = int(match.group(1))
            base.wait_stage(bot_mcp, bot_pid, 3, 240)
            lua_text(bot_mcp, bot_pid, "X2World:EnterWorld(1,0)")
            for _ in range(240):
                if lua_text(bot_mcp, bot_pid, "return X2LoginCharacter:GetCurrentStage()").strip() in ("4", "5"):
                    break
                time.sleep(0.5)
            create_character(bot_mcp, bot_pid, args.bot_character, args.create_character)
            time.sleep(3.0)
            if base.pet_exists(bot_mcp, bot_pid):
                lua_text(bot_mcp, bot_pid, "X2Mate:DismissPet() return 1")
                wait_for(lambda: not base.pet_exists(bot_mcp, bot_pid), 8.0, 0.2)
                time.sleep(2.2)
            bot_trace = Trace(server_log)
            bot_trace.reset()
            if not base.pet_exists(bot_mcp, bot_pid):
                try:
                    bot_mcp.text("client_use_item", pid=bot_pid, item_template_id=ITEM)
                except RuntimeError:
                    bot_mcp.text("client_chat", pid=bot_pid, message=f"/item add self {ITEM} 1")
                    time.sleep(1.5)
                    bot_mcp.text("client_use_item", pid=bot_pid, item_template_id=ITEM)
                wait_for(lambda: base.pet_exists(bot_mcp, bot_pid), 25, 0.3)
            if not base.pet_exists(bot_mcp, bot_pid):
                raise RuntimeError("the bot horse could not be summoned")
            time.sleep(2.0)
            targets = wait_packets(bot_trace, {"mate_target"}, 3.0)
            if not targets:
                raise RuntimeError("the bot horse unit id was not traced")
            bot_obj = int(targets[-1]["obj_id"])
            bot_name = lua_text(bot_mcp, bot_pid, "return tostring(X2Unit:UnitName('playerpet'))").strip() or bot_name
            bot_uid = lua_text(bot_mcp, bot_pid, "return tostring(X2Unit:GetUnitId('playerpet'))").strip()
            entities = json.loads(bot_mcp.text("client_entities", pid=bot_pid, limit=100))
            horse = next((v for v in entities.get("entities", {}).values() if v.get("name") == bot_name), None)
            if horse is None or not bot_uid:
                raise RuntimeError("the bot horse unit was not found")
            bot_horse_pos = (horse["position"]["x"], horse["position"]["y"], horse["position"]["z"])
            log("bot horse unit", bot_uid, "name", bot_name, "pos", bot_horse_pos)

            def keepalive() -> None:
                while not bot_stop.wait(3.0):
                    try:
                        bot_mcp.text("client_lua", pid=bot_pid, target="ui", code="return 1")
                    except Exception:
                        return

            bot_keepalive = threading.Thread(target=keepalive, daemon=True)
            bot_keepalive.start()

        reset_pet()
        log("after reset pet", base.pet_exists(mcp, pid), "mounted", base.mounted(mcp, pid))
        summon()
        log("after summon pet", base.pet_exists(mcp, pid), "mounted", base.mounted(mcp, pid), "mate", mate_unit)
        ensure_dismounted()
        log("after ensure pet", base.pet_exists(mcp, pid), "mounted", base.mounted(mcp, pid))

        # (a) mount, then self-unmount
        first = mount_near()
        if not first["mounted"]:
            raise RuntimeError(f"case a: mount did not finish: {first}")
        time.sleep(2.05)
        trace.reset()
        lua_text(mcp, pid, CAPTURE_CLEAR)
        start = time.monotonic()
        lua_text(mcp, pid, "X2Mate:UnMountMate() return 1")
        unmount_packets = wait_packets(trace, {"unmount"}, 3.0)
        wait_for(lambda: not base.mounted(mcp, pid), 5.0)
        a = {
            "case": "a_self_unmount",
            "mount_reason": next((p["reason"] for p in first["packets"] if p["kind"] == "mount"), None),
            "unmount_reason": next((p["reason"] for p in unmount_packets if p["kind"] == "unmount"), None),
            "unmount_ap": next((p["ap"] for p in unmount_packets if p["kind"] == "unmount"), None),
            "unmount_ms": round((time.monotonic() - start) * 1000.0, 1),
            "mounted": base.mounted(mcp, pid),
            "text": capture(mcp, pid),
        }
        rows.append(a)
        log("a", a)
        unmounted_at = time.monotonic()

        # (c) mount inside and after the two-second cooldown
        delay = min(0.5, max(0.05, 2.0 - (time.monotonic() - unmounted_at)))
        time.sleep(delay)
        trace.reset()
        lua_text(mcp, pid, CAPTURE_CLEAR)
        inside_ms = round((time.monotonic() - unmounted_at) * 1000.0, 1)
        can_inside = lua_text(mcp, pid, "return tostring(X2Mate:CanMount())").strip()
        lua_text(mcp, pid, "X2Mate:MountMate() return 1")
        inside_packets = wait_packets(trace, {"mount"}, 1.0)
        inside_text = capture(mcp, pid)
        c1 = {
            "case": "c_remount_inside_cooldown",
            "after_unmount_ms": inside_ms,
            "can_mount": can_inside == "true",
            "mount_sent": bool(inside_packets),
            "mounted": base.mounted(mcp, pid),
            "text": inside_text,
        }
        rows.append(c1)
        log("c1", c1)
        wait = max(0.0, 2.15 - (time.monotonic() - unmounted_at))
        time.sleep(wait)
        second = mount_near()
        c2 = {
            "case": "c_remount_after_cooldown",
            "after_unmount_ms": round((time.monotonic() - unmounted_at) * 1000.0, 1),
            "mount_sent": any(p["kind"] == "mount" for p in second["packets"]),
            "mount_reason": next((p["reason"] for p in second["packets"] if p["kind"] == "mount"), None),
            "mounted": second["mounted"],
            "text": second["text"],
        }
        rows.append(c2)
        log("c2", c2)
        time.sleep(2.05)
        trace.reset()
        lua_text(mcp, pid, "X2Mate:UnMountMate() return 1")
        wait_packets(trace, {"unmount"}, 3.0)
        wait_for(lambda: not base.mounted(mcp, pid), 5.0)

        # (b) mount, then dismiss while mounted
        ensure_dismounted()
        time.sleep(2.05)
        third = mount_near()
        if not third["mounted"]:
            raise RuntimeError(f"case b: mount did not finish: {third}")
        time.sleep(2.05)
        trace.reset()
        lua_text(mcp, pid, CAPTURE_CLEAR)
        started = time.monotonic()
        lua_text(mcp, pid, "X2Mate:DismissPet() return 1")
        packets = wait_packets(trace, {"unmount", "remove"}, 6.0)
        removed = bool(wait_for(lambda: not base.pet_exists(mcp, pid), 6.0, 0.1))
        unmount_packet = next((p for p in packets if p["kind"] == "unmount"), None)
        remove_packet = next((p for p in packets if p["kind"] == "remove"), None)
        b = {
            "case": "b_dismiss_while_mounted",
            "mount_reason": next((p["reason"] for p in third["packets"] if p["kind"] == "mount"), None),
            "unmount_reason": unmount_packet["reason"] if unmount_packet else None,
            "unmount_ap": unmount_packet["ap"] if unmount_packet else None,
            "remove_body": remove_packet.get("body") if remove_packet else None,
            "remove_after_unmount_ms": round((remove_packet["at"] - unmount_packet["at"]) * 1000.0, 1)
            if unmount_packet and remove_packet else None,
            "pet_removed": removed,
            "elapsed_ms": round((time.monotonic() - started) * 1000.0, 1),
            "text": capture(mcp, pid),
        }
        rows.append(b)
        log("b", b)

        # (d) reach sweep against the stationary bot horse (the own pet follows its owner)
        if not bot_uid or bot_pid is None:
            raise RuntimeError("the reach sweep needs --bot-account and --bot-character")
        ensure_dismounted()
        time.sleep(2.05)
        teleport(mcp, pid, args.character, bot_horse_pos[0] + distances[0], bot_horse_pos[1], bot_horse_pos[2], wait=3.0)

        for distance in distances:
            time.sleep(2.05)
            player_unit = mem.u32(mem.u32(base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)
            _, player_pos = base.unit_state(mem, player_unit)
            if not player_pos:
                raise RuntimeError("player position lost")
            teleport(mcp, pid, args.character, bot_horse_pos[0] + distance, bot_horse_pos[1], bot_horse_pos[2], wait=2.5)
            player_unit = mem.u32(mem.u32(base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)
            _, player_pos = base.unit_state(mem, player_unit)
            if not player_pos:
                raise RuntimeError("player position lost")
            measured = math.sqrt(distance2(player_pos, bot_horse_pos))
            trace.reset()
            lua_text(mcp, pid, CAPTURE_CLEAR)
            target = ""
            target_ok = False
            try:
                target = mcp.text("client_target", pid=pid, unit_id=bot_uid, expected_name=bot_name)
                target_ok = '"submitted":true' in target
            except Exception as error:
                target = str(error)
            can = lua_text(mcp, pid, "return tostring(X2Mate:CanMount())").strip() == "true"
            start_pos = list(player_pos)
            started = time.monotonic()
            lua_text(mcp, pid, "X2Mate:MountMate() return 1")
            packets: list[dict[str, object]] = []
            moved = 0.0
            end = time.monotonic() + 12.0
            while time.monotonic() < end:
                packets += trace.poll()
                player_unit = mem.u32(mem.u32(base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)
                _, now_pos = base.unit_state(mem, player_unit)
                if now_pos:
                    moved = max(moved, math.sqrt(distance2(start_pos, now_pos)))
                if any(p["kind"] == "mount" for p in packets):
                    break
                if time.monotonic() - started > 1.5 and not can:
                    break
                time.sleep(0.05)
            sent = any(p["kind"] == "mount" for p in packets)
            if sent:
                wait_for(lambda: base.mounted(mcp, pid), 5.0)
            d = {
                "case": "d_reach",
                "requested_m": round(distance, 3),
                "measured_m": round(measured, 3),
                "measured_sq": round(measured * measured, 3),
                "target_ok": target_ok,
                "can_mount": can,
                "mount_sent": sent,
                "mount_reason": next((p["reason"] for p in packets if p["kind"] == "mount"), None),
                "mount_ap": next((p["ap"] for p in packets if p["kind"] == "mount"), None),
                "moved_m": round(moved, 3),
                "mounted": base.mounted(mcp, pid),
                "elapsed_ms": round((time.monotonic() - started) * 1000.0, 1),
                "text": capture(mcp, pid),
            }
            rows.append(d)
            log("d", d)
            if d["mounted"]:
                time.sleep(2.05)
                trace.reset()
                lua_text(mcp, pid, "X2Mate:UnMountMate() return 1")
                wait_packets(trace, {"unmount"}, 3.0)
                wait_for(lambda: not base.mounted(mcp, pid), 5.0)

        csv_fields: list[str] = []
        for row in rows:
            for key in row:
                if key not in csv_fields:
                    csv_fields.append(key)
        with open(csv_path, "w", encoding="utf-8", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=csv_fields)
            writer.writeheader()
            writer.writerows(rows)
        meta = {
            "account": "<account>", "character": "<character>", "pid": "<pid>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "x2game_base": base_addr,
            "reach_sq": 625.0, "cooldown_ms": 2000, "mount_send_sq": 9.0,
        }
        json_path.write_text(json.dumps({"meta": meta, "cases": rows}, indent=1), encoding="utf-8")

        by_case = {row["case"]: row for row in rows}
        report = [
            "How to reproduce:",
            "  server: RUST_LOG=info,aaemu_game::net::fields=trace (the C->S field trace)",
            "  client: the original 1.2 client (r208022), hidden, character at a summoned Brown Lilyut Horse (item 8159)",
            "  probe: python tools/mount_unmount_probe.py --client <client> --driver <driver> --driver-dll <driver-dll> "
            "--launcher <launcher> --login <host:port> --account <account> --character <character> "
            "--bot-account <bot-account> --bot-character <bot-character> "
            "--password-file <password-file> --server-log <server-log> --create-character --out tools/evidence/mount_unmount",
            "",
            "unmount/dismiss reasons, ride cooldown and CanMount reach: native 1.2 capture",
            f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
            "account: <account> / <character>  pid: <pid>  x2game base: " + f"{base_addr:#x}",
            "server log: <server-log>",
            f"source commit: {subprocess.run(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], capture_output=True, text=True).stdout.strip()}",
            "",
            f"(a) mount reason {by_case.get('a_self_unmount', {}).get('mount_reason')}, self-unmount "
            f"reason {by_case.get('a_self_unmount', {}).get('unmount_reason')} ap {by_case.get('a_self_unmount', {}).get('unmount_ap')}",
            f"(b) dismiss while mounted: unmount reason {by_case.get('b_dismiss_while_mounted', {}).get('unmount_reason')}, "
            f"CSRemoveMate body {by_case.get('b_dismiss_while_mounted', {}).get('remove_body')} after "
            f"{by_case.get('b_dismiss_while_mounted', {}).get('remove_after_unmount_ms')} ms",
            f"(c) remount at {by_case.get('c_remount_inside_cooldown', {}).get('after_unmount_ms')} ms: "
            f"CanMount {by_case.get('c_remount_inside_cooldown', {}).get('can_mount')}, "
            f"CSMountMate {by_case.get('c_remount_inside_cooldown', {}).get('mount_sent')}; after cooldown: "
            f"CSMountMate {by_case.get('c_remount_after_cooldown', {}).get('mount_sent')}, "
            f"mounted {by_case.get('c_remount_after_cooldown', {}).get('mounted')}",
            "",
            "reach sweep (CanMount and CSMountMate per measured distance):",
            "  requested_m  measured_m  can_mount  sent  reason  moved_m  text",
        ]
        for row in rows:
            if row["case"] != "d_reach":
                continue
            text = str(row["text"]).replace("\n", " / ")[:80]
            report.append(
                f"  {row['requested_m']:11.2f}  {row['measured_m']:10.3f}  {str(row['can_mount']):9s}  "
                f"{str(row['mount_sent']):5s}  {str(row['mount_reason']):6s}  {row['moved_m']:7.3f}  {text}"
            )
        report += ["", "Raw server log lines (opcode/body/reason):"]
        for _, line in trace.lines:
            if any(token in line for token in ("0x0A4", "0x0A7", "0x0A8", "CSMountMatePacket fields", "CSUnMountMatePacket fields")):
                report.append("  " + line.strip())
        report += ["", "Files:", f"  native.csv   {csv_path}", f"  native.json  {json_path}", f"  raw run dir  {raw}"]
        (out / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
        (raw / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
        log("done; report", out / "report.txt")
        return 0
    finally:
        bot_stop.set()
        if bot_keepalive is not None:
            bot_keepalive.join(timeout=5.0)
        if bot_pid is not None and bot_mcp is not None:
            try:
                log("stopping bot", bot_pid)
                bot_mcp.text("client_stop", pid=bot_pid)
            except Exception as error:
                log("bot client_stop failed:", error)
                subprocess.run(["taskkill", "/PID", str(bot_pid), "/F"], capture_output=True)
        if bot_mcp is not None:
            bot_mcp.close()
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
