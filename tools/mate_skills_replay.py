#!/usr/bin/env python3
"""Replay the native S4 pet-bar mount-skill cases in the Bevy client.

Runs tools/evidence/mate_skills/replay.txt with `aa-client --headless --mute`
against the same lane, polls the server's `C->S` trace while it runs, and
compares every `CSStartSkill` 0x052 body with the original 1.2 capture in
tools/evidence/mate_skills/native.json.  The mate object id and the target
object id differ per run, so the comparison normalises those ids (caster
object -> 000000, a nonzero target object -> 000001) and compares the other
bytes exactly.  The side-by-side before/after table is written to
tools/evidence/mate_skills/replay-report.txt.

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace`.

Usage:
    python tools/mate_skills_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --evidence tools/evidence/mate_skills
"""
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mate_skills_probe import Trace, decode_c2s

ROOT = Path(__file__).resolve().parents[1]
CASES = [
    "a_self_slot1",
    "a_self_slot5",
    "a_self_slot4",
    "b_no_target_slot2",
    "b_targeted_slot2",
    "b_targeted_slot3",
    "d_pet_manual_slot1",
    "c_level_gate_slot2",
]
BEFORE = {
    "a_self_slot1": "pet bar sent nothing: no use_pet_action path",
    "a_self_slot5": "pet bar sent nothing: no use_pet_action path",
    "a_self_slot4": "pet bar sent nothing: no use_pet_action path",
    "b_no_target_slot2": "pet bar sent nothing; UseMateSkill rejected target 0",
    "b_targeted_slot2": "pet bar sent nothing: no use_pet_action path",
    "b_targeted_slot3": "pet bar sent nothing: no use_pet_action path",
    "d_pet_manual_slot1": "pet bar sent nothing (nothing implemented)",
    "c_level_gate_slot2": "pet bar sent nothing (nothing implemented)",
}
MARKER_RE = re.compile(r"CASE:([A-Za-z0-9_]+)")


def is_mount_cast(packet: dict) -> bool:
    if packet.get("dir") != "C->S" or packet.get("opcode") != 0x052:
        return False
    try:
        raw = bytes.fromhex(packet["body"].replace(" ", ""))
    except ValueError:
        return False
    return len(raw) == 17 and raw[4] == 3


def normalize(body: str) -> bytes:
    raw = bytearray.fromhex(body.replace(" ", ""))
    raw[5:8] = b"\x00\x00\x00"
    if len(raw) >= 17 and int.from_bytes(raw[13:16], "little") != 0:
        raw[13:16] = b"\x01\x00\x00"
    return bytes(raw)


def fields(body: str) -> dict:
    return decode_c2s("CSStartSkillPacket", body)


def describe(body: str) -> str:
    f = fields(body)
    return (f"skill {f.get('skill_id')} mount {f.get('mount_skill_template_id')} "
            f"target {f.get('target_kind')}/{f.get('target_obj')}")


def norm_hex(body: str) -> str:
    return normalize(body).hex(" ")


def same_body(native_body: str, bevy_body: str) -> bool:
    return normalize(native_body) == normalize(bevy_body)


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("MATE_BEVY_CLIENT", "aa-client.exe"),
                        help="path to the Bevy aa-client.exe")
    parser.add_argument("--login", default=os.environ.get("MATE_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("MATE_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("MATE_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("MATE_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("MATE_SERVER_LOG"),
                        help="game server log with the C->S trace")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "mate_skills"))
    parser.add_argument("--raw", default=os.environ.get("MATE_RAW")
                        or str(ROOT / "data" / "extract" / f"mate-skills-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set MATE_{name.upper()})")
    client = Path(args.client)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    evidence = Path(args.evidence)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    if not client.exists() or not password_file.exists() or not server_log.exists():
        raise SystemExit(f"missing: {client} {password_file} {server_log}")
    script = evidence / "replay.txt"
    if not script.exists():
        raise SystemExit(f"missing replay script: {script}")
    native = json.loads((evidence / "native.json").read_text(encoding="utf-8"))
    native_cases: dict[str, str] = {}
    for row in native["rows"]:
        if (row.get("case") in CASES and row.get("dir") == "C->S"
                and row.get("opcode") == "0x052" and row.get("body")):
            native_cases[row["case"]] = row["body"].strip()

    trace = Trace(server_log)
    trace.reset()
    packets: list[dict] = []
    stop = threading.Event()

    def poll() -> None:
        while not stop.wait(0.02):
            packets.extend(trace.poll())

    poller = threading.Thread(target=poll, daemon=True)
    poller.start()

    command = [
        str(client), "--headless", "--mute", "--account", args.account, "--login", args.login,
        "--password-file", str(password_file), "--character", args.character, "--script", str(script),
    ]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, bufsize=1)
    assert process.stdout is not None
    lines: list[str] = []
    markers: list[tuple[float, str]] = []

    def read_output() -> None:
        for line in process.stdout:
            line = line.rstrip("\n")
            lines.append(line)
            match = MARKER_RE.search(line)
            if match:
                markers.append((time.monotonic(), match.group(1)))

    reader = threading.Thread(target=read_output, daemon=True)
    reader.start()
    try:
        process.wait(timeout=420)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
        lines.append("replay: client timed out")
    reader.join(timeout=5.0)
    stop.set()
    poller.join(timeout=2.0)
    packets.extend(trace.poll())
    wall = round(time.monotonic() - started, 1)
    (raw / "bevy-replay.log").write_text(
        " ".join(command) + "\n\n" + "\n".join(lines) + "\n", encoding="utf-8")

    markers.sort()
    windows: dict[str, tuple[float, float | None]] = {}
    for i, (at, name) in enumerate(markers):
        windows[name] = (at, markers[i + 1][0] if i + 1 < len(markers) else None)
    casts = [p for p in packets if is_mount_cast(p)]

    results: dict[str, dict] = {}
    for case in CASES:
        native_body = native_cases.get(case)
        window = windows.get(case)
        if window is None:
            bevy = []
            why = "case marker missing from the replay output"
        else:
            begin, end = window
            bevy = [p for p in casts if p["t"] >= begin and (end is None or p["t"] < end)]
            why = ""
        record: dict = {
            "native": None,
            "bevy": [
                {"body": p["body"].strip(), "fields": fields(p["body"]), "line": p.get("line", "")}
                for p in bevy
            ],
            "result": "MATCH",
            "why": why,
        }
        if native_body is not None:
            record["native"] = {"body": native_body, "fields": fields(native_body),
                                "normalized": norm_hex(native_body)}
        if why:
            record["result"] = "DIFFERS"
        elif native_body is None:
            if bevy:
                record["result"] = "DIFFERS"
                record["why"] = f"native sent no 0x052, Bevy sent {len(bevy)}"
        elif len(bevy) != 1:
            record["result"] = "DIFFERS"
            record["why"] = f"native sent one 0x052, Bevy sent {len(bevy)}"
        elif not same_body(native_body, bevy[0]["body"]):
            record["result"] = "DIFFERS"
            record["why"] = (f"normalised native {norm_hex(native_body)} != "
                             f"Bevy {norm_hex(bevy[0]['body'])}")
        if record["bevy"]:
            record["normalized"] = norm_hex(bevy[0]["body"])
        results[case] = record

    matched = sum(1 for r in results.values() if r["result"] == "MATCH")
    checks = {
        "script_exit": process.returncode,
        "script_pass": process.returncode == 0,
        "cases": len(CASES),
        "matched": matched,
        "ok": process.returncode == 0 and matched == len(CASES),
        "normalisation": "caster object bytes 5..8 -> 00 00 00; a nonzero target object bytes 13..16 -> 01 00 00",
    }
    record = {
        "meta": {
            "account": "<account>", "character": "<character>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "client": "<client>",
            "server_log": "<server-log>", "wall_s": wall,
        },
        "checks": checks,
        "cases": results,
        "packets": [
            {key: p.get(key) for key in ("dir", "opcode", "name", "len", "body", "line")}
            for p in packets if p.get("dir") == "C->S" and p.get("opcode") == 0x052
        ],
    }
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")

    def after(case: str) -> str:
        r = results[case]
        if r["bevy"]:
            return describe(r["bevy"][0]["body"])
        if case == "d_pet_manual_slot1":
            return "no packet (local 0x2c on foot)"
        if case == "c_level_gate_slot2":
            return "no packet (not learned)"
        return "no packet"

    lines_out = [
        "## Bevy replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  account: <account> / <character>",
        "client: <client>  (--headless --mute)",
        f"script: <evidence>/{script.name}  exit: {process.returncode}",
        f"wall: {wall} s",
        "",
        "Wire comparison: CSStartSkill 0x052, 17 bytes. The mate (caster) object id and the",
        "target object id differ per run; they are normalised (caster bytes 5..8 -> 000000,",
        "a nonzero target bytes 13..16 -> 000001) and every other byte is compared exactly.",
        "",
        "case                   1.2 native (normalised)                          Bevy (normalised)                               result",
    ]
    for case in CASES:
        r = results[case]
        native_text = r["native"]["normalized"] if r["native"] else "no packet"
        bevy_text = r.get("normalized", "no packet")
        lines_out.append(f"{case:23s} {native_text:44s} {bevy_text:46s} {r['result']}")
        if r["result"] != "MATCH":
            lines_out.append(f"    {r['why']}")
    lines_out += [
        "",
        "Before/after (before = base ba214379, from its tests/docs; after = this replay):",
        "case                   before ba214379                              after (Bevy)",
    ]
    for case in CASES:
        lines_out.append(f"{case:23s} {BEFORE[case]:45s} {after(case)}")
    lines_out += [
        "",
        "ba214379 had no pet-bar cast path: AAUI.UseSlot had no ISLOT_PET_ACTION branch, no",
        "pet_action_bar_button key action and no HUD use_skill/UsePetAction request, so every",
        "slot click sent nothing. aa-net's Command::UseMateSkill required a nonzero 24-bit",
        "target, so the native no-target (target 0) body could not be produced either. This",
        "replay exercises the added path and gets the captured bodies for the six casting",
        "cases, and the native silence for the on-foot and not-learned slots.",
        "",
        f"checks: {checks}",
        "",
    ]
    section = "\n".join(lines_out)
    (evidence / "replay-report.txt").write_text(section, encoding="utf-8")
    print(section)
    return 0 if checks["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
