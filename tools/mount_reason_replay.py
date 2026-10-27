#!/usr/bin/env python3
"""Replay the mount approaches in the Bevy client and compare the reasons.

Runs tools/evidence/mount_reason/replay.txt and replay_front.txt with
`aa-client --headless --mute`, records the server's `CSMountMatePacket fields`
lines after each login, and checks the reasons against the `# expect N`
annotations.  The second account is only used by this replay; it does not have
to be the same account as the 1.2 probe.

The game server must log the CSMountMate fields:
`RUST_LOG=info,aaemu_game::net::fields=trace` (the `aaemu_game::net::fields`
target).

The compare is appended to `--evidence/report.txt` (the script removes an older
`## Bevy replay` section first).  Raw client output goes to `--raw`.

Usage:
    python tools/mount_reason_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --evidence tools/evidence/mount_reason
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def expected_reasons(script: Path) -> list[int]:
    return [int(m.group(1)) for m in re.finditer(r"^#\s*expect\s+(\d+)", script.read_text(encoding="utf-8"), re.M)]


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("MOUNT_BEVY_CLIENT", "aa-client.exe"),
                        help="path to the Bevy aa-client.exe")
    parser.add_argument("--login", default=os.environ.get("MOUNT_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("MOUNT_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("MOUNT_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("MOUNT_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("MOUNT_SERVER_LOG"),
                        help="game server log with the CSMountMate field trace")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "mount_reason"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-reason-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set MOUNT_{name.upper()})")
    client = Path(args.client)
    password_file = Path(args.password_file)
    server_log = Path(args.server_log)
    evidence = Path(args.evidence)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    if not client.exists() or not password_file.exists() or not server_log.exists():
        raise SystemExit(f"missing: {client} {password_file} {server_log}")

    scripts = [evidence / "replay.txt", evidence / "replay_front.txt"]
    lines = [
        "## Bevy replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  account: <account> / <character>",
        "client: <client>",
        "",
        "The same cardinal approaches as the 1.2 probe, driven by the Bevy client's own script runner",
        "(`aa-client --headless --mute`). The reason is the server's `CSMountMatePacket fields` line.",
        "",
    ]
    ok = True
    for script in scripts:
        expected = expected_reasons(script)
        offset = server_log.stat().st_size
        command = [str(client), "--headless", "--mute", "--account", args.account, "--login", args.login,
                   "--password-file", str(password_file), "--character", args.character, "--script", str(script)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        data = server_log.read_bytes()[offset:].decode("utf-8", "replace")
        reasons = [int(m.group(3)) for line in data.splitlines()
                   if (m := re.search(r"CSMountMatePacket fields: CSMountMatePacket \{ tl_id: (\d+), ap: (\d+), reason: (\d+) \}", line))]
        (raw / f"replay-{script.stem}.txt").write_text(" ".join(command) + "\n\n" + result.stdout + result.stderr,
                                                       encoding="utf-8")
        match = result.returncode == 0 and reasons == expected
        ok = ok and match
        shown = (f"<client> --headless --mute --account <account> --login <login> "
                 f"--password-file <password-file> --character <character> --script <evidence>/{script.name}")
        lines += [
            f"{script.name}:",
            f"  command: {shown}",
            f"  exit: {result.returncode}  expected {expected}  captured {reasons}  "
            f"{'OK' if match else 'MISMATCH'}",
        ]
        sides = {"replay.txt": ["back", "right", "left"], "replay_front.txt": ["front"]}.get(script.name, [])
        rows = [row.split(",") for row in (evidence / "native.csv").read_text(encoding="utf-8").splitlines()[1:]]
        for side, reason in zip(sides, reasons):
            native = [int(row[24]) for row in rows if len(row) > 24 and row[4] == side]
            same = bool(native) and all(value == reason for value in native)
            ok = ok and same
            lines.append(f"  {side}: native {native}, Bevy {reason}  {'OK' if same else 'MISMATCH'}")
    lines += [
        "",
        "The behind/right/left run sends 3/2/1 and the separate front run sends 4, the same values the",
        "1.2 capture has for those actual sides (back 3, front 4, right 2, left 1). The walks are the",
        "script runner's ordinary movement, not a teleported state.",
        "",
    ]
    section = "\n".join(lines)
    report = evidence / "report.txt"
    text = report.read_text(encoding="utf-8") if report.exists() else ""
    marker = "\n## Bevy replay\n"
    if marker in text:
        text = text.split(marker, 1)[0].rstrip() + "\n"
    report.write_text(text + section, encoding="utf-8")
    (evidence / "replay-report.txt").write_text(section, encoding="utf-8")
    print(section)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
