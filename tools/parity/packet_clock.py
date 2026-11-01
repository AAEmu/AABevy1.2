#!/usr/bin/env python3
"""Align the server's C->S field trace to the client's own clock.

The server log timestamp has one-second resolution and the recorder's tail-poll
time is not a send time.  The 1.2 client puts its clock inside some packets:
``CSMoveUnit`` bytes 4-7 and ``Ping`` bytes 16-19 (the same value the server
echoes in ``Pong.local``).  Those are the only anchors this module trusts.  A
packet without an embedded clock gets a time interpolated between the nearest
anchors in log order and is tagged ``clock=interpolated``; nothing is reported
as an exact send time that does not carry one.

See handoff/2026-11-01-evidence-audit-captures.md F1.
"""
from __future__ import annotations

import argparse
import bisect
import json
import re
import struct
import sys
from pathlib import Path
from typing import Any

SCHEMA = "aabevy.parity.client-packets/v1"
C2S_RE = re.compile(
    r"opcode=0x([0-9a-fA-F]+)\s+(\w+)\s+len=(\d+)\s+body=([0-9a-fA-F ]*)")
S2C_RE = re.compile(
    r"GamePacket:\s*S->C\s+type\s+([0-9a-fA-F]+)\s+(\w+)\s+\((\d+)\s+bytes\)")
TIME_RE = re.compile(r"^(\d{2}:\d{2}:\d{2}(?:\.\d+)?)")

TRUSTED = {
    "CSMoveUnitPacket": ("time", 4, 4),
    "PingPacket": ("local", 16, 4),
}


def packet_clock(name: str, body: bytes) -> tuple[int | None, str | None]:
    """Return the client clock embedded in a C->S packet body, when it has one."""
    spec = TRUSTED.get(name)
    if spec is None:
        return None, None
    field, offset, width = spec
    if len(body) < offset + width:
        return None, None
    return struct.unpack_from("<I", body, offset)[0], f"{name}.{field}"


def parse_line(line: str) -> dict | None:
    stamp = TIME_RE.match(line.strip())
    log_time = stamp.group(1) if stamp else None
    match = C2S_RE.search(line)
    if match:
        body_hex = match.group(4).strip()
        try:
            body = bytes.fromhex(body_hex) if body_hex else b""
        except ValueError:
            # a torn or garbled trace line: skip it rather than fail the whole alignment
            return None
        client_ms, anchor = packet_clock(match.group(2), body)
        return {
            "dir": "C->S", "log_time": log_time,
            "opcode": int(match.group(1), 16), "name": match.group(2),
            "len": int(match.group(3)), "body": body_hex.lower(),
            "body_bytes": body, "client_ms": client_ms, "anchor": anchor,
        }
    match = S2C_RE.search(line)
    if match:
        return {
            "dir": "S->C", "log_time": log_time,
            "opcode": int(match.group(1), 16), "name": match.group(2),
            "len": int(match.group(3)), "body": "", "body_bytes": b"",
            "client_ms": None, "anchor": None,
        }
    return None


def parse_log(text: str, first_line: int = 1) -> list[dict]:
    records: list[dict] = []
    for number, line in enumerate(text.splitlines(), first_line):
        record = parse_line(line)
        if record is not None:
            record["line"] = number
            record["raw"] = line
            records.append(record)
    return records


def read_log(path: Path, offset: int = 0) -> tuple[list[dict], int]:
    path = Path(path)
    if not path.exists():
        return [], offset
    size = path.stat().st_size
    if size < offset:
        offset = 0
    with path.open("rb") as handle:
        handle.seek(offset)
        data = handle.read(size - offset)
    # complete lines only, cut on the raw bytes so the returned offset is exact even when the
    # log holds invalid UTF-8; a line still being written is left for the next read
    cut = data.rfind(b"\n") + 1
    complete = data[:cut]
    records = parse_log(complete.decode("utf-8", "replace"))
    return records, offset + len(complete)


def align(records: list[dict], clock: Any = None) -> list[dict]:
    """Fill client_ms/ui_ms/t_ms for every record; anchor times stay exact."""
    positions = [index for index, record in enumerate(records)
                 if record.get("anchor") is not None
                 and record.get("client_ms") is not None]
    for position, record in enumerate(records):
        if record.get("anchor") is not None and record.get("client_ms") is not None:
            record["clock"] = "packet"
            record["anchor_before"] = record.get("anchor")
            record["anchor_after"] = record.get("anchor")
            record["anchor_gap_ms"] = 0
        else:
            split = bisect.bisect_left(positions, position)
            before = records[positions[split - 1]] if split > 0 else None
            after = (records[positions[split]]
                     if split < len(positions) else None)
            if before is not None and after is not None:
                before_index = positions[split - 1]
                after_index = positions[split]
                span = after_index - before_index
                fraction = (position - before_index) / span if span else 0.0
                record["client_ms"] = round(
                    before["client_ms"]
                    + (after["client_ms"] - before["client_ms"]) * fraction)
                record["clock"] = "interpolated"
                record["anchor_gap_ms"] = (
                    after["client_ms"] - before["client_ms"])
            elif before is not None:
                record["client_ms"] = before["client_ms"]
                record["clock"] = "nearest_anchor"
                record["anchor_gap_ms"] = None
            elif after is not None:
                record["client_ms"] = after["client_ms"]
                record["clock"] = "nearest_anchor"
                record["anchor_gap_ms"] = None
            else:
                record["client_ms"] = None
                record["clock"] = "none"
                record["anchor_gap_ms"] = None
            record["anchor_before"] = (before.get("anchor")
                                       if before else None)
            record["anchor_after"] = after.get("anchor") if after else None
        if record.get("client_ms") is not None and clock is not None:
            record["ui_ms"] = clock.ui_for_system(record["client_ms"])
            record["t_ms"] = clock.relative(record["ui_ms"])
        else:
            record["ui_ms"] = None
            record["t_ms"] = None
    return records


def client_rows(records: list[dict]) -> list[dict]:
    rows = []
    for sequence, record in enumerate(
            (record for record in records if record["dir"] == "C->S")):
        rows.append({
            "schema": SCHEMA, "seq": sequence, "line": record["line"],
            "log_time": record.get("log_time"), "dir": "C->S",
            "opcode": record["opcode"], "name": record["name"],
            "len": record["len"], "body": record.get("body", ""),
            "client_ms": record.get("client_ms"),
            "ui_ms": record.get("ui_ms"), "t_ms": record.get("t_ms"),
            "clock": record.get("clock"),
            "anchor": record.get("anchor"),
            "anchor_before": record.get("anchor_before"),
            "anchor_after": record.get("anchor_after"),
            "anchor_gap_ms": record.get("anchor_gap_ms"),
        })
    return rows


def summary(records: list[dict]) -> dict:
    c2s = [record for record in records if record["dir"] == "C->S"]
    exact = [record for record in records if record.get("clock") == "packet"]
    return {
        "records": len(records), "client_packets": len(c2s),
        "exact_clock_packets": len(exact),
        "interpolated_packets": sum(
            1 for record in records if record.get("clock") == "interpolated"),
        "nearest_anchor_packets": sum(
            1 for record in records if record.get("clock") == "nearest_anchor"),
        "untimed_packets": sum(
            1 for record in records if record.get("clock") == "none"),
        "anchors": {
            "move": sum(1 for record in exact
                        if record.get("anchor") == "CSMoveUnitPacket.time"),
            "ping": sum(1 for record in exact
                        if record.get("anchor") == "PingPacket.local"),
        },
        "first_client_ms": min((record["client_ms"] for record in exact),
                               default=None),
        "last_client_ms": max((record["client_ms"] for record in exact),
                              default=None),
    }


def write_rows(path: Path, records: list[dict]) -> int:
    rows = client_rows(records)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False,
                                    separators=(",", ":")) + "\n")
    return len(rows)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--offset", type=int, default=0)
    args = parser.parse_args(argv[1:])
    records, _ = read_log(args.log, args.offset)
    align(records)
    if args.out:
        write_rows(args.out, records)
        print(json.dumps(summary(records), indent=1))
    else:
        for row in client_rows(records):
            print(json.dumps(row, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
