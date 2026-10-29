#!/usr/bin/env python3
"""Capture the 1.2 client's pet-bar mount-skill cases (S4).

The original client's pet action bar (PetActionButton1..5) runs the 1.2 gate
`FUN_3933e630` on a click and, while riding, sends one 17-byte `CSStartSkill`
0x052 with the mate as the kind-3 caster, the `mount_skills.id` and the row's
combat skill (`FUN_39399630` -> `FUN_393989e0`).  This probe drives the hidden
original through aaemu-clientdriver (Lua and the client's own pointer/action
APIs, no OS input), clicks the pet-bar widgets at their centres, and records
the server's C->S field trace plus the S->C opcode lines.

Cases: Heir's Horse (item 27264): mount; self slots 1, 5 and 4 with no target;
targeted slots 2 and 3 with a Farm Wagon (`/slave spawn 60`) as the current
target; slot 2 with no target; a slot on foot.  Then the level-5 Brown Lilyut
Horse (item 8159): its slots 2..5 are not learned, so clicking them sends
nothing and shows the local refusal.

Usage:
    python tools/mate_skills_probe.py --client <archeage.exe> \\
        --driver <aaemu-clientdriver.exe> --driver-dll <aadriver.dll> \\
        --launcher <aaemu-launcher.exe> --login <host:port> \\
        --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --out tools/evidence/mate_skills
    python tools/mate_skills_probe.py --pid 1234 ...  # already in world

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace`.  The script reads that log at
`--server-log`; its own packet lines supply the opcode, body and decoded
fields.  The 1.2 client must be hidden (`client_launch ... hidden`), which this
script does.  It refuses to run if an anti-cheat module is loaded.

The account may be new: with AutoAccount the first login creates it.  A
missing character is created through the original creation UI when
`--create-character` is passed.
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import re
import struct
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import mount_reason_probe as base
import mount_unmount_probe as mu

ROOT = Path(__file__).resolve().parents[1]
HEIR_ITEM = 27264  # Heir's Horse, npc 13022, mate level 40
HORSE_ITEM = 8159  # Brown Lilyut Horse, npc 5431, mate level 5
CART = 60          # Farm Wagon (a server-spawned slave)

C2S_RE = re.compile(r"C->S level=(\d+) opcode=0x([0-9A-Fa-f]{3}) (\w+) len=(\d+) body=(.*)$")
S2C_RE = re.compile(r"GamePacket: S->C type ([0-9A-Fa-f]{3}) (\w+) \((\d+) bytes\)")
SKILL_RE = re.compile(r"StartSkill: Id (\d+), flag (\d+), caster=(\d+), target=(\d+)")


def log(*args: object) -> None:
    print("mate-skills:", *args, flush=True)


def u24(body: str, offset: int = 0) -> int:
    raw = bytes.fromhex(body.replace(" ", ""))
    return int.from_bytes(raw[offset:offset + 3], "little")


def decode_c2s(name: str, body: str) -> dict:
    """Decoded fields for the C->S packets this capture cares about (byte-exact)."""
    try:
        raw = bytes.fromhex(body.replace(" ", ""))
    except ValueError:
        return {"error": "bad hex"}
    out: dict = {"bytes": len(raw)}
    try:
        if name == "CSStartSkillPacket":
            out["skill_id"] = struct.unpack_from("<I", raw, 0)[0]
            kind = raw[4]
            out["caster_kind"] = kind
            out["caster_obj"] = u24(body, 5)
            pos = 8
            if kind == 2:  # item
                out["mount_skill_template_id"] = None
                out["note"] = "item caster"
            elif kind == 3:  # mount
                out["mount_skill_template_id"] = struct.unpack_from("<I", raw, pos)[0]
                pos += 4
            out["target_kind"] = raw[pos] if pos < len(raw) else None
            out["target_obj"] = u24(body, pos + 1) if pos + 1 + 3 <= len(raw) else None
            out["flag"] = raw[pos + 4] if pos + 4 < len(raw) else None
        elif name == "CSChangeTargetPacket":
            out["obj_id"] = u24(body, 0)
        else:
            out["raw"] = raw.hex()
    except Exception as error:  # malformed body
        out["error"] = f"{type(error).__name__}: {error}"
    return out


class Trace:
    """Incremental reader of the game server log (C->S trace + S->C opcode lines)."""

    def __init__(self, path: Path):
        self.path = Path(path)
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw: list[str] = []

    def reset(self) -> None:
        self.offset = self.path.stat().st_size if self.path.exists() else 0
        self.raw = []

    def poll(self) -> list[dict]:
        if not self.path.exists():
            return []
        data = self.path.read_bytes()[self.offset:].decode("utf-8", "replace")
        if not data:
            return []
        complete = data.rsplit("\n", 1)[0] if not data.endswith("\n") else data
        self.offset += len(complete.encode("utf-8"))
        now = time.monotonic()
        out: list[dict] = []
        for line in complete.splitlines():
            self.raw.append(line)
            m = C2S_RE.search(line)
            if m:
                out.append({"t": now, "dir": "C->S", "opcode": int(m.group(2), 16),
                            "name": m.group(3), "len": int(m.group(4)),
                            "body": m.group(5).strip(), "line": line.strip()})
                continue
            m = S2C_RE.search(line)
            if m:
                out.append({"t": now, "dir": "S->C", "opcode": int(m.group(1), 16),
                            "name": m.group(2), "len": int(m.group(3)),
                            "body": "", "line": line.strip()})
                continue
            m = SKILL_RE.search(line)
            if m:
                out.append({"t": now, "dir": "log", "opcode": -1, "name": "StartSkill",
                            "len": 0,
                            "body": f"skill={m.group(1)} flag={m.group(2)} caster={m.group(3)} target={m.group(4)}",
                            "line": line.strip()})
        return out


def collect(trace: Trace, seconds: float, stop=None) -> list[dict]:
    packets: list[dict] = []
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        packets += trace.poll()
        if stop is not None and stop(packets):
            break
        time.sleep(0.02)
    return packets


def has(packets: list[dict], direction: str, opcode: int) -> bool:
    return any(p["dir"] == direction and p["opcode"] == opcode for p in packets)


def lua(mcp: base.Mcp, pid: int, code: str) -> str:
    for _ in range(20):
        try:
            return mcp.text("client_lua", pid=pid, target="ui", code=code)
        except Exception:
            time.sleep(0.1)
    return ""


def capture_text(mcp: base.Mcp, pid: int) -> str:
    return lua(mcp, pid, mu.CAPTURE_READ).strip()


def capture_clear(mcp: base.Mcp, pid: int) -> None:
    lua(mcp, pid, mu.CAPTURE_CLEAR)


class Slice:
    """The S4 rows -> native.csv / native.json."""

    def __init__(self, outdir: Path, name: str, trace: Trace):
        self.out = Path(outdir)
        self.name = name
        self.trace = trace
        self.rows: list[dict] = []
        self.notes: list[str] = []
        self.packets: list[dict] = []
        self.started = time.strftime("%Y-%m-%dT%H:%M:%S")

    def note(self, text: str) -> None:
        self.notes.append(text)
        log(self.name, "note:", text)

    def state(self, case: str, event: str, **fields: object) -> None:
        self.rows.append({
            "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "case": case, "event": event,
            "dir": "", "opcode": "", "name": "", "len": "", "body": "",
            "decoded": "", "client_state": json.dumps(fields, ensure_ascii=False, default=str),
            "text": fields.get("text", ""),
        })

    def add(self, case: str, packets: list[dict], decoded: bool = True) -> None:
        self.packets.extend(packets)
        for p in packets:
            fields = {}
            if decoded and p["dir"] == "C->S":
                fields = decode_c2s(p["name"], p["body"])
            elif decoded and p["dir"] == "log":
                fields = {"log": p["body"]}
            self.rows.append({
                "time": time.strftime("%Y-%m-%dT%H:%M:%S"), "case": case,
                "event": "packet" if p["dir"] != "log" else "server-log",
                "dir": p["dir"], "opcode": f"0x{p['opcode']:03X}" if p["opcode"] >= 0 else "",
                "name": p["name"], "len": p["len"], "body": p["body"],
                "decoded": json.dumps(fields, ensure_ascii=False, default=str),
                "client_state": "", "text": "",
            })

    def write(self) -> None:
        self.out.mkdir(parents=True, exist_ok=True)
        fields = ["time", "case", "event", "dir", "opcode", "name", "len", "body",
                  "decoded", "client_state", "text"]
        with open(self.out / "native.csv", "w", encoding="utf-8", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=fields)
            writer.writeheader()
            writer.writerows(self.rows)
        meta = {
            "slice": self.name,
            "started": self.started,
            "written": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "client": "original 1.2 r208022 archeage.exe (hidden, one launch)",
            "driver": "aaemu-clientdriver (Lua/pointer/client actions, no OS input)",
            "server_log": "<server-log>",
            "rows": len(self.rows),
            "packets": len(self.packets),
        }
        (self.out / "native.json").write_text(
            json.dumps({"meta": meta, "rows": self.rows}, indent=1, ensure_ascii=False),
            encoding="utf-8")


class Ctx:
    def __init__(self, mcp: base.Mcp, pid: int, mem: base.Mem, base_addr: int, trace: Trace, out: Path):
        self.mcp = mcp
        self.pid = pid
        self.mem = mem
        self.base_addr = base_addr
        self.trace = trace
        self.out = out
        self.slices: dict[str, Slice] = {}

    def ui(self, code: str) -> str:
        return lua(self.mcp, self.pid, code)

    def player_unit(self) -> int:
        return self.mem.u32(self.mem.u32(self.base_addr + (base.G_CONNECTION - base.DUMP_BASE)) + 0x10C)

    def units(self) -> dict[int, int]:
        return dict(base.walk_units(self.mem, self.base_addr)[1])

    def unit_pos(self, unit: int) -> list[float] | None:
        return base.unit_state(self.mem, unit)[1]

    def unit_kind(self, unit: int) -> int:
        return self.mem.u32(unit + 0x410)

    def collect(self, seconds: float, stop=None) -> list[dict]:
        return collect(self.trace, seconds, stop)

    def chat(self, message: str) -> None:
        self.mcp.text("client_chat", pid=self.pid, message=message)

    def pet_exists(self) -> bool:
        return base.pet_exists(self.mcp, self.pid)

    def mounted(self) -> bool:
        return base.mounted(self.mcp, self.pid)

    def mount_status(self) -> dict:
        code = ("local function s(v) if v==nil then return 'nil' end return tostring(v) end "
                "return table.concat({s(X2Mate:IsPlayerPetExists()),s(X2Mate:IsPlayerMounting()),"
                "s(X2Mate:CanMount()),s(X2Mate:PetState()),s(X2Mate:IsMountablePet())},'|')")
        parts = self.ui(code).strip().split("|")
        keys = ["pet_exists", "mounting", "can_mount", "pet_state", "mountable"]
        out = dict(zip(keys, [p == "true" for p in parts[:3]] + parts[3:]))
        out["pet_name"] = self.ui("return tostring(X2Unit:UnitName('playerpet'))").strip()
        return out

    def pet_bar(self) -> dict:
        code = ("local function s(v) if v==nil then return 'nil' end return tostring(v) end "
                "local n=X2Mate:GetNumMountSkills() or 0 local r={}"
                " for i=1,math.min(n,10) do local t=X2Mate:GetMountSkill(i)"
                " r[#r+1]=i..':'..s(type(t)=='table' and t.learned)..':'..s(type(t)=='table' and t.skillUsable) end"
                " return s(n)..'|'..table.concat(r,',')")
        text = self.ui(code).strip()
        count, _, rest = text.partition("|")
        return {"count": count, "slots": rest}

    def pet_button(self, index: int) -> dict:
        code = ("local b=PetActionButton%d if not b then return '' end local x,y=b:GetEffectiveOffset()"
                " local w,h=b:GetEffectiveExtent() local t=b:GetTooltip()"
                " return string.format('%%d %%d %%d %%d %%s %%s',x,y,w,h,tostring(b:IsVisible()),"
                "tostring(type(t)=='table' and t.name or t))" % index)
        parts = self.ui(code).strip().split(" ", 5)
        if len(parts) < 5:
            return {}
        return {"x": int(parts[0]), "y": int(parts[1]), "w": int(parts[2]), "h": int(parts[3]),
                "visible": parts[4], "tooltip": parts[5] if len(parts) > 5 else ""}

    def click_widget(self, index: int) -> tuple[int, int] | None:
        button = self.pet_button(index)
        if not button:
            return None
        x = button["x"] + button["w"] // 2
        y = button["y"] + button["h"] // 2
        self.mcp.text("client_pointer", pid=self.pid, action="move", x=x, y=y)
        time.sleep(0.1)
        self.mcp.text("client_pointer", pid=self.pid, action="click", x=x, y=y)
        return x, y

    def entities(self) -> list[dict]:
        try:
            payload = json.loads(self.mcp.text("client_entities", pid=self.pid, limit=200))
        except Exception:
            return []
        rows = []
        for key, value in (payload.get("entities") or {}).items():
            row = dict(value)
            row["engine_id"] = key
            rows.append(row)
        return rows

    def target_name(self, alias: str = "target") -> str:
        return self.ui(f"return tostring(X2Unit:UnitName('{alias}'))").strip()

    def target_id(self, alias: str = "target") -> str:
        return self.ui(f"return tostring(X2Unit:GetUnitId('{alias}'))").strip()

    def slice(self, name: str) -> Slice:
        if name not in self.slices:
            self.slices[name] = Slice(self.out, name, self.trace)
        return self.slices[name]


def reset_pet(ctx: Ctx) -> None:
    base.dismount(ctx.mcp, ctx.pid)
    if ctx.pet_exists():
        for _ in range(2):
            ctx.ui("X2Mate:DismissPet() return 1")
            end = time.monotonic() + 6.0
            while time.monotonic() < end and ctx.pet_exists():
                time.sleep(0.2)
            if not ctx.pet_exists():
                break
        time.sleep(2.2)


def summon(ctx: Ctx, item: int, sl: Slice, case: str) -> bool:
    reset_pet(ctx)
    ok = False
    for attempt in range(1, 4):
        ctx.chat(f"/item add self {item} 1")
        time.sleep(2.0)
        try:
            ctx.mcp.text("client_use_item", pid=ctx.pid, item_template_id=item)
        except Exception as error:
            sl.state(case, "use_item_failed", attempt=attempt, error=str(error))
        end = time.monotonic() + 10.0
        while time.monotonic() < end and not ctx.pet_exists():
            time.sleep(0.3)
        ok = ctx.pet_exists()
        sl.state(case, "summon", attempt=attempt, item=item, pet_exists=ok, mount=ctx.mount_status())
        if ok:
            break
        time.sleep(3.0)
    time.sleep(2.0)
    return ok


def spawn_cart(ctx: Ctx, sl: Slice, case: str) -> tuple[int, list[float], str]:
    before = ctx.units()
    ctx.trace.reset()
    ctx.chat(f"/slave spawn {CART}")
    deadline = time.monotonic() + 15.0
    unit = None
    while time.monotonic() < deadline:
        units = ctx.units()
        new = [(key, value) for key, value in units.items() if key not in before]
        if new:
            unit = new[-1]
            break
        time.sleep(0.3)
    if not unit:
        raise RuntimeError("no new slave unit after /slave spawn")
    key, unit_addr = unit
    time.sleep(1.0)
    ents = [e for e in ctx.entities() if e.get("class") == "prefab_unit"]
    if not ents:
        raise RuntimeError("the cart entity did not appear")
    ent = next((e for e in ents
                if str(e.get("engine_id")) in (str(key), f"{key:x}", f"{key:X}")), ents[-1])
    pos = ent["position"]
    sl.add(case, ctx.collect(0.8))
    sl.state(case, "spawn", cart_name=ent["name"], cart_obj=key, cart_hex=f"{key:X}",
             cart_pos=[pos["x"], pos["y"], pos["z"]], unit_kind=ctx.unit_kind(unit_addr),
             unit_tl=ctx.mem.u32(unit_addr + 0x41C))
    return key, [pos["x"], pos["y"], pos["z"]], ent["name"]


def pet_click(ctx: Ctx, sl: Slice, case: str, index: int, seconds: float = 2.5) -> list[dict]:
    button = ctx.pet_button(index)
    sl.state(case, "widget", slot=index, button=button, bar=ctx.pet_bar())
    if not button:
        return []
    ctx.trace.reset()
    capture_clear(ctx.mcp, ctx.pid)
    point = ctx.click_widget(index)
    packets = ctx.collect(seconds, stop=lambda p: has(p, "C->S", 0x052))
    packets += ctx.collect(1.0)
    sl.add(case, packets)
    sl.state(case, "clicked", slot=index, point=list(point) if point else None,
             start_skill=has(packets, "C->S", 0x052),
             skill_started=has(packets, "S->C", 0x0A1), skill_fired=has(packets, "S->C", 0x0A2),
             skill_ended=has(packets, "S->C", 0x0A3), impulse=has(packets, "S->C", 0x079),
             error_msg=has(packets, "S->C", 0x0A0), mount=ctx.mount_status(),
             target=ctx.target_name(), text=capture_text(ctx.mcp, ctx.pid))
    return packets


def slice_s4(ctx: Ctx) -> None:
    sl = ctx.slice("s4")
    sl.note("S4 mount skills. Mate = Heir's Horse (item 27264, npc 13022): mate level 40, the same five "
            "mount skills as the Brown Lilyut Horse but all usable. Slots are the server's received order "
            "npc_mount_skills [83, 1, 13, 6, 3] -> skills 17092 (self), 11016 (targeted), 11369 "
            "(targeted), 11037 (self), 11309 (self); every row has a mount_attached_skills rider skill "
            "(18228/11013/17094/11036/15506). The pet bar is PetActionButton1..5; clicks go through the "
            "driver's pointer API at the widget centre (UI space == 2554x1411 back buffer).")

    if not summon(ctx, HEIR_ITEM, sl, "setup"):
        sl.note("NOT CAPTURED: the Heir's Horse could not be summoned.")
        return
    sl.state("setup", "pet_bar", **ctx.pet_bar())
    ctx.trace.reset()
    ctx.ui("X2Mate:MountMate() return 1")
    packets = ctx.collect(5.0, stop=lambda p: has(p, "S->C", 0x076))
    sl.add("setup", packets)
    time.sleep(1.5)
    sl.state("setup", "mounted", mount=ctx.mount_status(), packets=len(packets))
    sl.state("setup", "buttons", buttons=[ctx.pet_button(i) for i in range(1, 6)])

    # (a) self buff slot 1 (83 -> 17092) and the other slots for the slot map
    for index in (1, 5, 4):
        pet_click(ctx, sl, f"a_self_slot{index}", index)
        time.sleep(1.0)

    # (b) targeted skill with a target selected (a cart: the server-spawned slave)
    cart_obj = None
    try:
        cart_obj, _cart_pos, cart_name = spawn_cart(ctx, sl, "b_targeted")
    except Exception as error:
        sl.state("b_targeted", "cart_failed", error=str(error))
    if cart_obj:
        try:
            ctx.mcp.text("client_target", pid=ctx.pid, unit_id=f"{cart_obj:X}", expected_name=cart_name)
        except Exception as error:
            sl.state("b_targeted", "target_failed", error=str(error))
        sl.state("b_targeted", "target", target=cart_name, obj=cart_obj,
                 name=ctx.target_name(), id=ctx.target_id(), mount=ctx.mount_status())
        for index in (2, 3):
            pet_click(ctx, sl, f"b_targeted_slot{index}", index)
            time.sleep(1.0)
        # no target at all (the targeted row with nothing selected)
        ctx.ui("X2Unit:ReleaseTarget() return 1")
        time.sleep(0.4)
        sl.state("b_no_target", "released", target=ctx.target_name())
        pet_click(ctx, sl, "b_no_target_slot2", 2)
    else:
        sl.note("NOT CAPTURED: no unit for the targeted skill (b).")

    # (d) not riding: no pet_manual flow, only the local refusal
    base.dismount(ctx.mcp, ctx.pid)
    time.sleep(2.2)
    sl.state("d_pet_manual", "before", mount=ctx.mount_status(), pet_bar=ctx.pet_bar(),
             buttons=[ctx.pet_button(i) for i in range(1, 6)])
    pet_click(ctx, sl, "d_pet_manual_slot1", 1, seconds=1.2)
    sl.state("d_pet_manual", "after_slot_click", mount=ctx.mount_status(),
             text=capture_text(ctx.mcp, ctx.pid))

    # (c) mate-level gate on the level-5 horse
    if summon(ctx, HORSE_ITEM, sl, "c_level_gate"):
        sl.state("c_level_gate", "pet_bar", **ctx.pet_bar())
        ctx.trace.reset()
        ctx.ui("X2Mate:MountMate() return 1")
        ctx.collect(5.0, stop=lambda p: has(p, "S->C", 0x076))
        time.sleep(1.5)
        sl.state("c_level_gate", "mounted", mount=ctx.mount_status(),
                 buttons=[ctx.pet_button(i) for i in range(1, 6)])
        for index in (2, 3, 4, 5):
            pet_click(ctx, sl, f"c_level_gate_slot{index}", index, seconds=1.6)
            time.sleep(0.7)
    else:
        sl.note("NOT CAPTURED: the level-5 horse (8159) could not be summoned.")

    errors = [p for p in sl.packets if p["dir"] == "S->C" and p["opcode"] == 0x0A0]
    sl.note("SCErrorMsg 0x0a0 in this slice: "
            + (", ".join(p["line"] for p in errors) if errors else "none"))


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("MATE_CLIENT", "archeage.exe"),
                        help="path to the original 1.2 archeage.exe")
    parser.add_argument("--driver", default=os.environ.get("MATE_DRIVER", "aaemu-clientdriver.exe"),
                        help="path to AAEmu's aaemu-clientdriver.exe")
    parser.add_argument("--driver-dll", default=os.environ.get("MATE_DRIVER_DLL", "aadriver.dll"),
                        help="path to the x86 aadriver.dll")
    parser.add_argument("--launcher", default=os.environ.get("MATE_LAUNCHER", "aaemu-launcher.exe"),
                        help="path to AAEmu's aaemu-launcher.exe")
    parser.add_argument("--login", default=os.environ.get("MATE_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("MATE_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("MATE_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("MATE_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("MATE_SERVER_LOG"),
                        help="game server log with the fields trace")
    parser.add_argument("--pid", type=int, help="attach to an already running, in-world client instead of launching")
    parser.add_argument("--create-character", action="store_true",
                        help="create the character through the original creation UI when the account has none")
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "mate_skills"))
    parser.add_argument("--raw", default=os.environ.get("MATE_RAW")
                        or str(ROOT / "data" / "extract" / f"mate-skills-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set MATE_{name.upper()})")
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
    out = Path(args.out)
    raw = Path(args.raw)
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)

    log(f"account {args.account}: {args.character}")
    mcp = base.Mcp(driver, dll, launcher, client, args.login, password_file, args.account, raw / "mcp")
    pid = args.pid
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
            lua(mcp, pid, "X2World:EnterWorld(1,0)")
            for _ in range(240):
                if lua(mcp, pid, "return X2LoginCharacter:GetCurrentStage()").strip() in ("4", "5"):
                    break
                time.sleep(0.5)
            mu.create_character(mcp, pid, args.character, args.create_character)
            time.sleep(5.0)
        else:
            base.wait_player(mcp, pid, 30)

        install = lua(mcp, pid, mu.CAPTURE_INSTALL).strip()
        if install != "ok":
            log("text capture install:", install)

        mem = base.Mem(pid)
        for name, _ in mem.modules():
            if any(bad in name.lower() for bad in base.ANTI_CHEAT):
                raise RuntimeError(f"anti-cheat module loaded: {name}; refusing to read")
        base_addr = mem.module_base("x2game.dll")
        if not base_addr:
            raise RuntimeError("x2game.dll not loaded")
        log("x2game base", hex(base_addr))

        trace = Trace(server_log)
        ctx = Ctx(mcp, pid, mem, base_addr, trace, out)
        sl = ctx.slice("s4")
        try:
            slice_s4(ctx)
        finally:
            sl.write()
            report = [
                "How to reproduce:",
                "  server: RUST_LOG=info,aaemu_game::net::fields=trace (the C->S field trace)",
                "  client: the original 1.2 client (r208022), hidden, mate at a Heir's Horse (item 27264) "
                "and a level-5 Brown Lilyut Horse (item 8159)",
                "  probe: python tools/mate_skills_probe.py --client <client> --driver <driver> "
                "--driver-dll <driver-dll> --launcher <launcher> --login <host:port> --account <account> "
                "--character <character> --password-file <password-file> --server-log <server-log> "
                "--create-character --out tools/evidence/mate_skills",
                "",
                "pet-bar mount skills: native 1.2 capture",
                f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
                f"account: {args.account} / {args.character}  x2game base: {base_addr:#x}",
                f"lane login: {args.login}",
                "server log: <server-log>",
                "source commit: " + subprocess.run(
                    ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                    capture_output=True, text=True).stdout.strip(),
                "",
            ] + sl.notes + [
                "",
                "Files:",
                "  native.csv   tools/evidence/mate_skills/native.csv",
                "  native.json  tools/evidence/mate_skills/native.json",
                "  raw run dir  <raw>",
            ]
            (out / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
            log("done; report", out / "report.txt")
        return 0
    finally:
        if pid is not None:
            try:
                log("stopping client", pid)
                mcp.text("client_stop", pid=pid)
            except Exception as error:
                log("client_stop failed:", error)
                subprocess.run(["taskkill", "/PID", str(pid), "/F"], capture_output=True)
        mcp.close()


if __name__ == "__main__":
    raise SystemExit(main())
