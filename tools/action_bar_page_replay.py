#!/usr/bin/env python3
"""Replay the action bar page capture in the Bevy client, against the same server.

The script `tools/evidence/action_bar_pages/replay.txt` selects the four pages
through `X2Action:SetActionBarPage` and puts a skill on the first visible cell
through `HUD.drag_action_set` - the same request a real drop queues.  The game
server's packet trace (`RUST_LOG=info,aaemu_game::net::fields=trace`) then has to
show the four `CSUpdateActionSlotPacket` slots the original sent: 1, 49, 61 and
73 (page 1..4, first cell).

Usage:
    python tools/action_bar_page_replay.py --client <aa-client.exe> \
        --login <host:port> --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        --out tools/evidence/action_bar_pages

The client runs headless and muted (`--headless --mute --script ...`).  Bulky
logs go under `--raw`; the compare output is written to the evidence directory.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PAGES = [(1, 1, 18132), (2, 49, 18132), (3, 61, 18132), (4, 73, 18132)]


def log(*a: object) -> None:
    print("action-bar-pages-replay:", *a, flush=True)


def shown(path: Path, kind: str) -> str:
    """The committed report shows paths relative to the repo, else a placeholder."""
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return f"<{kind}>" + (f"/{path.name}" if path.name else "")


def packets(lines: list[str]) -> list[dict[str, object]]:
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
        out.append({"slot": slot, "kind": kind, "id": action_id, "body": body.hex(" ")})
    return out


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("ABR_CLIENT"), help="path to aa-client.exe")
    parser.add_argument("--script", default=str(ROOT / "tools" / "evidence" / "action_bar_pages" / "replay.txt"))
    parser.add_argument("--login", default=os.environ.get("ABR_LOGIN", "127.0.0.1:27237"))
    parser.add_argument("--account", default=os.environ.get("ABR_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("ABR_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("ABR_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("ABR_SERVER_LOG"))
    parser.add_argument("--skill-id", type=int, default=18132)
    parser.add_argument("--timeout", type=float, default=420.0)
    parser.add_argument("--out", default=str(ROOT / "tools" / "evidence" / "action_bar_pages"))
    parser.add_argument("--raw", default=os.environ.get("ABR_RAW")
                        or str(ROOT / "data" / "extract" / f"action-bar-pages-replay-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("client", "account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set ABR_{name.upper()})")
    client = Path(args.client)
    script = Path(args.script)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    out = Path(args.out)
    raw = Path(args.raw)
    for path, what in ((client, "client"), (script, "script"), (password_file, "password file"), (server_log, "server log")):
        if not path.exists():
            raise SystemExit(f"missing {what}: {path}")
    out.mkdir(parents=True, exist_ok=True)
    raw.mkdir(parents=True, exist_ok=True)
    expected = [(page, slot, args.skill_id) for page, slot, _ in PAGES]
    script_text = script.read_text(encoding="utf-8")
    if str(args.skill_id) not in script_text:
        raise SystemExit(f"the replay script does not use skill {args.skill_id}")

    offset = server_log.stat().st_size
    command = [str(client), "--headless", "--mute", "--account", args.account, "--login", args.login,
               "--password-file", str(password_file), "--character", args.character, "--script", str(script)]
    log("running", " ".join(command))
    started = time.time()
    try:
        run = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", errors="replace",
                             timeout=args.timeout, cwd=str(ROOT))
    except subprocess.TimeoutExpired as error:
        (raw / "replay.log").write_text((error.stdout or "") + "\nTIMEOUT\n", encoding="utf-8")
        raise SystemExit(f"aa-client timed out after {args.timeout} s")
    elapsed = time.time() - started
    (raw / "replay.log").write_text(run.stdout + "\n--- stderr ---\n" + run.stderr, encoding="utf-8")
    time.sleep(1.0)
    trace = server_log.read_bytes()[offset:].decode("utf-8", "replace").splitlines()
    observed = packets(trace)
    (raw / "server-packets.txt").write_text("\n".join(trace), encoding="utf-8")

    # The page sequence must appear in order; other packets may be interleaved.
    found: list[dict[str, object]] = []
    index = 0
    for packet in observed:
        if index < len(expected) and packet["slot"] == expected[index][1]:
            found.append(packet)
            index += 1
    passed = run.returncode == 0 and index == len(expected) and all(
        packet["kind"] == 2 and packet["id"] == args.skill_id for packet in found
    )
    report = [
        "How to reproduce:",
        "  server: RUST_LOG=info,aaemu_game::net::fields=trace (the CSUpdateActionSlot body trace)",
        "  replay: python tools/action_bar_page_replay.py --client <aa-client.exe> --login <host:port> "
        "--account <account> --character <character> --password-file <password-file> "
        "--server-log <server-log> --out tools/evidence/action_bar_pages",
        "",
        "action bar pages: Bevy replay against the same server",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
        "account: <account> / <character>",
        "script: <replay-script>",
        f"client exit: {run.returncode}  wall: {elapsed:.1f} s",
        "",
        "expected (page, first-cell global slot, skill):",
    ]
    for page, slot, skill in expected:
        report.append(f"  page {page}: slot {slot}, skill {skill}")
    report += ["", "observed CSUpdateActionSlot packets (slot:kind:id:body):"]
    for packet in observed:
        report.append(f"  {packet['slot']}:{packet['kind']}:{packet['id']}:{packet['body']}")
    report += ["", f"matched page sequence: {index}/{len(expected)}", f"result: {'PASS' if passed else 'FAIL'}"]
    if passed:
        report += [
            "",
            "What this proves: the Bevy client's page API selected pages 1..4, a repeated call on the",
            "current page added one ACTION_BAR_PAGE_CHANGED and page 5 was ignored (the replay script's",
            "asserts passed with client exit 0), and the drop path mapped the first visible cell to the",
            "original's global slots 1, 49, 61 and 73, which the server decoded.",
            "",
            "What it does not prove: animated page-control pixels or drag gestures; this replay calls the same",
            "HUD drag request a drop queues, not a mouse drag.  The native capture proves the original's",
            "mouse-drop slots; the API and request mapping are the shared parts.",
        ]
    report += ["", f"raw run dir: {shown(raw, 'raw')}",
               f"files: {shown(out / 'replay-report.txt', 'out')}"]
    (out / "replay-report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
    (raw / "replay-report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")
    (raw / "ACTUAL_RUN_REPORT.md").write_text(
        "\n".join([
            "# action bar pages Bevy replay",
            "",
            f"Command: `{' '.join(command)}`",
            f"Date: {time.strftime('%Y-%m-%d %H:%M:%S')}",
            f"Account: {args.account} / {args.character}",
            f"Server log: {server_log}",
            f"Client exit: {run.returncode}; matched page sequence {index}/{len(expected)}; result {'PASS' if passed else 'FAIL'}.",
            "",
            "The committed compare output is tools/evidence/action_bar_pages/replay-report.txt; the replay",
            "script is tools/evidence/action_bar_pages/replay.txt.  The client stdout/stderr and the server",
            "trace are in this directory.",
        ]) + "\n", encoding="utf-8")
    log(f"exit {run.returncode}, matched {index}/{len(expected)}: {'PASS' if passed else 'FAIL'}")
    log("script summary:")
    for line in run.stdout.splitlines():
        if line.startswith("SCRIPT "):
            log("  " + line)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
