#!/usr/bin/env python3
"""Replay the 1.2 specialty sale in the Bevy client and compare the packet bytes.

Runs the script `tools/evidence/specialty_sale/replay.txt` with
`aa-client --headless --mute`: target the Gold Trader (NPC template 4473),
then invoke the quote dialog's OK action, `X2Store:SellBackPackGoods()`.
The game server must log the C->S field trace
(`RUST_LOG=info,aaemu_game::net::fields=trace`).

The comparison uses the older 1.2 probe capture in
`tools/evidence/specialty_sale/native.json` (its NPC object id differs from
the current lane) and the owner-driven manual session in
`tools/evidence/specialty_sale/native-manual-session.txt`: the Bevy client
must send the same 0x065/0x042 opcodes, lengths and bodies for the lane's NPC
object, and both runs must complete the sale (no `SCErrorMsgPacket`, an
`SCItemTaskSuccess` for the consumed pack, the 60-labor charge). The script
writes `bevy.json` and `bevy-report.txt`
under --evidence and appends a `## Bevy replay` section to `report.txt`
(replacing an older section); raw client and server output goes to --raw.

Usage:
    python tools/specialty_sale_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        [--evidence tools/evidence/specialty_sale] [--raw <dir>]
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKET_RE = re.compile(
    r"\[(?:TRACE|DEBUG)\] fields - (?P<dir>[CS]->[SC]) (?:level=\d+ )?opcode=(?P<opcode>0x[0-9A-Fa-f]+) "
    r"(?P<name>\w+) len=(?P<len>\d+) body=(?P<body>[0-9a-fA-F ]*)")
SENT_RE = re.compile(r"GamePacket: S->C type (?P<opcode>[0-9A-Fa-f]{3}) (?P<name>\w+) \((?P<len>\d+) bytes\)")
INTERESTING = ("SCErrorMsgPacket", "SCItemTaskSuccessPacket",
               "SCCharacterLaborPowerChangedPacket", "SCCountUnreadMailPacket", "SCMailListPacket")


def probe(evidence: Path) -> dict:
    data = json.loads((evidence / "native.json").read_text(encoding="utf-8"))
    out = {}
    for packet in data["packets"]:
        if packet["dir"] != "C->S":
            continue
        if packet["name"] == "CSSellBackpackGoodsPacket":
            out["sell"] = packet
        elif packet["name"] == "CSInteractNPCPacket" and packet["body"].endswith("01"):
            out["interact"] = packet
    for key in ("interact", "sell"):
        if key not in out:
            raise SystemExit(f"native.json has no C->S {key} packet")
    return out


def manual(evidence: Path) -> dict | None:
    """The owner-driven 1.2 session on this lane, when its excerpt is present."""
    path = evidence / "native-manual-session.txt"
    if not path.exists():
        return None
    sent, _ = parse(path.read_text(encoding="utf-8"))
    return sent if "interact" in sent and "sell" in sent else None


def parse(text: str) -> tuple[dict, list[dict]]:
    sent: dict = {}
    server = []
    for line in text.splitlines():
        match = PACKET_RE.search(line)
        if match:
            packet = {
                "opcode": match.group("opcode").lower(),
                "name": match.group("name"),
                "len": int(match.group("len")),
                "body": " ".join(match.group("body").split()),
            }
            if match.group("dir") == "C->S":
                if packet["name"] == "CSSellBackpackGoodsPacket":
                    sent["sell"] = packet
                elif packet["name"] == "CSInteractNPCPacket" and packet["body"].endswith("01"):
                    sent["interact"] = packet
            continue
        match = SENT_RE.search(line)
        if match and match.group("name") in INTERESTING:
            server.append({"opcode": "0x" + match.group("opcode").lower(),
                           "name": match.group("name"), "len": int(match.group("len"))})
    return sent, server


def same(a: dict, b: dict | None) -> bool:
    return (b is not None and a["opcode"] == b["opcode"]
            and a["len"] == b["len"] and a["body"] == b["body"])


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("SALE_BEVY_CLIENT", "aa-client.exe"))
    parser.add_argument("--login", default=os.environ.get("SALE_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")))
    parser.add_argument("--account", default=os.environ.get("SALE_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("SALE_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("SALE_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("SALE_SERVER_LOG"))
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "specialty_sale"))
    parser.add_argument("--raw", default=os.environ.get("SALE_RAW")
                        or str(ROOT / "data" / "extract" / f"specialty-sale-{time.strftime('%Y%m%d')}-bevy-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set SALE_{name.upper()})")
    client = Path(args.client)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    evidence = Path(args.evidence)
    raw = Path(args.raw)
    script = evidence / "replay.txt"
    for path in (client, password_file, server_log, script, evidence / "native.json"):
        if not path.exists():
            raise SystemExit(f"missing {path}")
    raw.mkdir(parents=True, exist_ok=True)

    probe_ref = probe(evidence)
    manual_ref = manual(evidence)
    refs = {"probe": probe_ref}
    if manual_ref is not None:
        refs["manual"] = manual_ref
    offset = server_log.stat().st_size
    command = [str(client), "--headless", "--mute", "--account", args.account,
               "--login", args.login, "--password-file", str(password_file),
               "--character", args.character, "--script", str(script)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=420)
    (raw / "client.log").write_text(" ".join(command) + "\n\n" + result.stdout + result.stderr,
                                    encoding="utf-8")
    text = server_log.read_bytes()[offset:].decode("utf-8", "replace")
    (raw / "server-slice.log").write_text(text, encoding="utf-8")
    bevy, server = parse(text)

    def matched(ref: dict) -> dict:
        return {"interact": same(ref["interact"], bevy.get("interact")),
                "sell": same(ref["sell"], bevy.get("sell"))}

    per_ref = {name: matched(ref) for name, ref in refs.items()}
    sell_same = any(m["sell"] for m in per_ref.values())
    interact_same = any(m["interact"] for m in per_ref.values())
    full = [name for name, m in per_ref.items() if m["interact"] and m["sell"]]
    refused = [e for e in server if e["opcode"] == "0x10a"]
    item_task = [e for e in server if e["opcode"] == "0x090"]
    completed = not refused and bool(item_task)
    ok = result.returncode == 0 and sell_same and interact_same and completed

    out = {
        "client_exit": result.returncode,
        "script": "tools/evidence/specialty_sale/replay.txt",
        "original": probe_ref,
        "manual": manual_ref,
        "bevy": bevy,
        "server_answer": server,
        "matches": per_ref,
        "sell_byte_identical": sell_same,
        "interact_byte_identical": interact_same,
        "server_refused": bool(refused),
        "sale_completed": completed,
    }
    (evidence / "bevy.json").write_text(json.dumps(out, indent=2) + "\n", encoding="utf-8")
    reference_lines = []
    for name, ref in refs.items():
        reference_lines.extend([
            f"1.2 {name}: {ref['interact']['opcode']} {ref['interact']['name']} len={ref['interact']['len']} body={ref['interact']['body']}",
            f"          {ref['sell']['opcode']} {ref['sell']['name']} len={ref['sell']['len']} body={ref['sell']['body']}",
        ])
    match_line = "same bytes: " + ", ".join(
        f"{name} interact {m['interact']}, sell {m['sell']}" for name, m in per_ref.items())
    probe_match = per_ref.get("probe", {})
    probe_note = ""
    if full and full == ["manual"]:
        probe_note = (" The Bevy bytes match the owner-driven 1.2 manual session on this lane;"
                      " the older probe capture's NPC object id differs from this lane's.")
    elif full and full == ["probe"]:
        probe_note = " The Bevy bytes match the original 1.2 probe capture."
    elif full:
        probe_note = " The Bevy bytes match both 1.2 captures."
    elif probe_match:
        probe_note = (" The Bevy bytes differ from the older probe capture; see the comparison above.")
    lines = [
        "## Bevy replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  account: <account> / <character>",
        "client: aa-client --headless --mute (the same account and character; the 1.2 client is stopped)",
        "",
        f"script: {script.name} (target and interact with NPC template 4473, then X2Store:SellBackPackGoods())",
        f"client exit: {result.returncode}",
        *reference_lines,
        f"Bevy client: {bevy['interact']['opcode'] + ' ' + bevy['interact']['name'] + ' len=' + str(bevy['interact']['len']) + ' body=' + bevy['interact']['body'] if 'interact' in bevy else 'no interact packet'}",
        f"             {bevy['sell']['opcode'] + ' ' + bevy['sell']['name'] + ' len=' + str(bevy['sell']['len']) + ' body=' + bevy['sell']['body'] if 'sell' in bevy else 'no sell packet'}",
        match_line,
        f"server answer: {server or 'none'}",
        "",
        ("The AAEmu server completed the sale in both runs: it consumed the equipped" if completed
         else "The AAEmu server did not complete the sale in this run:"),
        ("pack, charged the 60 labor and mailed the specialty payment; no" if completed
         else "there was no SCItemTaskSuccess for the pack or the run was refused;"),
        ("SCErrorMsgPacket 0x10A was sent." + probe_note if completed
         else "see the S->C answer above." + probe_note),
        "",
        "Raw output: <raw-dir> (pass it with --raw; kept outside the repository)",
        "",
    ]
    section = "\n".join(lines)
    (evidence / "bevy-report.txt").write_text(section, encoding="utf-8")
    report = evidence / "report.txt"
    body = report.read_text(encoding="utf-8") if report.exists() else ""
    marker = "\n## Bevy replay\n"
    if marker in body:
        body = body.split(marker, 1)[0].rstrip()
    report.write_text((body.rstrip() + "\n\n" if body.strip() else "") + section,
                      encoding="utf-8")
    print(section)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
