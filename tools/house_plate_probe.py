#!/usr/bin/env python3
"""Capture the 1.2 client's own view of a house and a colliding NPC.

The driver is an instrumented r208022 client (aaemu-clientdriver) injected
into the original client. It exposes the stock Lua API through the client's
UI state, so the values below are what the original client itself reports --
not values read from a server database.

The client must already be in world. Typical run:

  aaemu-launcher.exe --hidden ...
  aaemu-clientdriver.exe lua -t ui --pid PID "X2Chat:SendChatMessage(CHAT_SAY,'/spawn npc 171','')"
  aaemu-clientdriver.exe lua -t ui --pid PID "..."   # discover the spawned
                                                     # client id (hex BcId)
  python tools/house_plate_probe.py --driver DRIVER.exe --pid PID \
      --house-id 111 --npc-id ac2f --out tools/evidence/house_plate

Only the driver executable is used; this script does not import any other
project tooling and takes every lane-specific value as an argument.
"""
from __future__ import annotations

import argparse
import csv
import json
import subprocess
import sys
import time
from pathlib import Path

FIELDS = [
    "name",
    "faction",
    "type",
    "kind",
    "level",
    "x",
    "y",
    "z",
    "info_hp",
    "info_max_hp",
    "info_grade",
    "info_building_state",
    "info_house_category",
    "info_owner_name",
    "info_base_progress",
    "info_remain_progress",
    "info_is_portal",
    "info_nick_name",
]

LUA_CAPTURE = r"""
local ids = {house=args.house, npc=args.npc}
local function scalar(v)
  local t = type(v)
  if t == 'table' or t == 'function' or t == 'userdata' then return nil end
  return tostring(v)
end
local function unit(id)
  local lines = {}
  local function add(k, v)
    v = scalar(v)
    if v ~= nil then lines[#lines+1] = id .. '\t' .. k .. '\t' .. v:gsub('[\t\r\n]', ' ') end
  end
  add('name', X2Unit:GetUnitNameById(id))
  add('faction', X2Unit:GetUnitFactionById(id))
  add('type', X2Unit:GetUnitTypeById(id))
  add('kind', X2Unit:GetUnitKindById(id))
  add('level', X2Unit:GetUnitLevelById(id))
  local x, y, z = X2Unit:GetUnitWorldPosition(id)
  add('x', x); add('y', y); add('z', z)
  local ok, info = pcall(X2Unit.GetUnitInfoById, X2Unit, id)
  if ok and type(info) == 'table' then
    for k, v in pairs(info) do add('info_' .. tostring(k), v) end
  end
  return table.concat(lines, '\n')
end
return unit(ids.house) .. '\n' .. unit(ids.npc)
"""


def driver(exe: str, *args: str, timeout: int = 30) -> str:
    proc = subprocess.run(
        [exe, *args], capture_output=True, text=True, timeout=timeout,
        encoding="utf-8", errors="replace",
    )
    if proc.returncode != 0:
        raise SystemExit(f"driver failed ({proc.returncode}): {proc.stderr.strip()}")
    return proc.stdout


def lua(exe: str, pid: int, code: str, timeout: int = 30) -> str:
    return driver(exe, "lua", "-t", "ui", "--pid", str(pid), code, timeout=timeout)


def parse_lines(text: str) -> dict[str, dict[str, str]]:
    units: dict[str, dict[str, str]] = {}
    for line in text.splitlines():
        parts = line.split("\t")
        if len(parts) != 3:
            continue
        unit_id, key, value = parts
        units.setdefault(unit_id, {})[key] = value
    return units


def crop(src: Path, dst: Path, box: tuple[int, int, int, int]) -> bool:
    try:
        from PIL import Image
    except ImportError:
        print("Pillow not installed; leaving the full screenshot in place", file=sys.stderr)
        if src != dst:
            dst.write_bytes(src.read_bytes())
        return False
    with Image.open(src) as im:
        im.crop(box).save(dst)
    return True


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--driver", required=True, help="aaemu-clientdriver.exe attached to the running client")
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--house-id", required=True, help="client unit id as hex, e.g. 111")
    ap.add_argument("--npc-id", required=True, help="client unit id as hex, e.g. ac2f")
    ap.add_argument("--out", type=Path, default=Path("tools/evidence/house_plate"))
    ap.add_argument("--crop", required=True,
                    help="x0,y0,x1,y1 crop of the house plate in the scene screenshot")
    args = ap.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    in_world = lua(args.driver, args.pid, "return tostring(X2:IsEnteredWorld())").strip()
    if in_world != "true":
        raise SystemExit(f"client {args.pid} is not in world (got {in_world!r})")

    code = LUA_CAPTURE.replace("args.house", repr(args.house_id)).replace("args.npc", repr(args.npc_id))
    units = parse_lines(lua(args.driver, args.pid, code))

    scene = args.out / "_house_scene_full.png"
    driver(args.driver, "screenshot", "--pid", str(args.pid), str(scene))
    Path("data/extract/house_plate").mkdir(parents=True, exist_ok=True)
    crop(scene, Path("data/extract/house_plate") / "house_plate.png", tuple(int(v) for v in args.crop.split(",")))
    scene.unlink(missing_ok=True)

    result = {
        "captured_at": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "driver": args.driver,
        "pid": args.pid,
        "client": "ArcheAge 1.2 r208022 (hidden)",
        "house_id": args.house_id,
        "npc_id": args.npc_id,
        "units": units,
    }
    (args.out / "native.json").write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    with (args.out / "native.csv").open("w", newline="", encoding="utf-8") as fh:
        writer = csv.writer(fh)
        writer.writerow(["case", "id", *FIELDS])
        for case, unit_id in (("house", args.house_id), ("npc", args.npc_id)):
            row = units.get(unit_id, {})
            writer.writerow([case, unit_id, *[row.get(k, "") for k in FIELDS]])

    house = units.get(args.house_id, {})
    npc = units.get(args.npc_id, {})
    print(f"house {args.house_id}: {house.get('name')} faction={house.get('faction')} "
          f"type={house.get('type')} cat={house.get('info_house_category')}")
    print(f"npc   {args.npc_id}: {npc.get('name')} faction={npc.get('faction')} "
          f"type={npc.get('type')} kind={npc.get('kind')}")
    print(f"wrote {args.out / 'native.json'} and {args.out / 'native.csv'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
