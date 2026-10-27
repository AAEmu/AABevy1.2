#!/usr/bin/env python3
"""Compare the 1.2 house-plate capture with the Bevy client before and after the fix.

Inputs are the native JSON from `tools/house_plate_probe.py`, the debug logs of two
aa-client runs (one on the parent commit, one on the fix) and the plate crops taken
from their screenshots. It prints a report and exits non-zero when any check fails.

The house is design 171, whose `npcs.id` is also 171 (faction 115, hostile). The
native client shows `Test Farmhouse` friendly/housing; before the fix the Bevy plate
read the NPC row (faction 115 -> hostile, orange); after it keeps the packet faction
(101 -> neutral, yellow).
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

PLATE = re.compile(r'plate 273 "Test Farmhouse": faction (\d+) \(unit (\d+)\) own ([\d ]+) -> (\w+)')


def read_text(path: Path) -> str:
    raw = path.read_bytes()
    if raw.startswith(b"\xff\xfe") or raw.startswith(b"\xfe\xff"):
        return raw.decode("utf-16")
    return raw.decode("utf-8", errors="replace")


def plate_line(log: Path) -> tuple[int, int, int, str] | None:
    # tracing wraps a long line after a fixed width: join spaces before reading the numbers
    text = re.sub(r"\s+", " ", read_text(log))
    m = PLATE.search(text)
    return (int(m[1]), int(m[2]), int(m[3].replace(" ", "")), m[4]) if m else None


def yellow_pixels(image: Path) -> tuple[int, int]:
    from PIL import Image

    with Image.open(image) as im:
        px = list(im.convert("RGB").getdata())
    yellow = sum(1 for r, g, b in px if r >= 200 and g >= 160 and b <= 80)
    return yellow, len(px)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--native", type=Path, default=Path("tools/evidence/house_plate/native.json"))
    ap.add_argument("--before-log", type=Path, required=True)
    ap.add_argument("--after-log", type=Path, required=True)
    ap.add_argument("--before-image", type=Path, default=Path("data/extract/house_plate/bevy_before_plate.png"))
    ap.add_argument("--after-image", type=Path, default=Path("data/extract/house_plate/bevy_after_plate.png"))
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()

    native = json.loads(args.native.read_text(encoding="utf-8"))
    house = native["units"][native["house_id"]]
    npc = native["units"][native["npc_id"]]
    before = plate_line(args.before_log)
    after = plate_line(args.after_log)
    before_yellow, before_total = yellow_pixels(args.before_image)
    after_yellow, after_total = yellow_pixels(args.after_image)

    checks = [
        ("native house is friendly", house.get("faction") == "friendly", house.get("faction")),
        ("native house is housing", house.get("type") == "housing", house.get("type")),
        ("native colliding NPC is hostile", npc.get("faction") == "hostile", npc.get("faction")),
        ("before read NPC faction 115", before is not None and before[0] == 115, before),
        ("before relationship was hostile", before is not None and before[3] == "hostile", before),
        ("after kept faction 101", after is not None and after[0] == 101, after),
        ("after relationship is neutral", after is not None and after[3] == "neutral", after),
        ("before crop has no yellow plate pixels", before_yellow < 20, (before_yellow, before_total)),
        ("after crop has yellow plate pixels", after_yellow >= 50, (after_yellow, after_total)),
    ]

    lines = [
        "house plate: native vs Bevy before/after",
        "",
        f"native house {native['house_id']}: {house.get('name')} faction={house.get('faction')} type={house.get('type')}",
        f"native npc   {native['npc_id']}: {npc.get('name')} faction={npc.get('faction')} kind={npc.get('kind')}",
        f"bevy before: {before}",
        f"bevy after:  {after}",
        f"before crop: {before_yellow} yellow of {before_total} px",
        f"after crop:  {after_yellow} yellow of {after_total} px",
        "",
    ]
    failed = False
    for name, ok, seen in checks:
        lines.append(f"[{'ok' if ok else 'FAIL'}] {name}: {seen}")
        failed |= not ok
    lines.append("")
    lines.append("FAIL" if failed else "PASS")
    report = "\n".join(lines) + "\n"
    print(report, end="")
    if args.out:
        args.out.write_text(report, encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
