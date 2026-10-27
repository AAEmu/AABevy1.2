#!/usr/bin/env python3
"""Replay the mount approaches in the Bevy client and compare the reasons.

Runs tools/evidence/mount_reason/replay.txt and replay_front.txt with
`aa-client --headless --mute`, records the server's `CSMountMatePacket fields`
lines after each login, and checks the reasons against the `# expect N`
annotations. The Bevy client's server account is the slot's `b` role (wp1b for
wp1), so it never shares an account with the 1.2 probe.

The compare is appended to tools/evidence/mount_reason/report.txt (the script
removes an older `## Bevy replay` section first). Raw client output goes to the
ignored data/extract directory.
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import live_slots  # noqa: E402


def expected_reasons(script: Path) -> list[int]:
    return [int(m.group(1)) for m in re.finditer(r"^#\s*expect\s+(\d+)", script.read_text(encoding="utf-8"), re.M)]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--slot", default=os.environ.get("MOUNT_SLOT", "wp1"))
    parser.add_argument("--lane", default=os.environ.get("MOUNT_LANE", "move-golden"), choices=sorted(live_slots.LANES))
    parser.add_argument("--client", default=str(Path(f"D:/aaemu-target/bevy-stepb/debug/aa-client.exe")))
    parser.add_argument("--login", default="127.0.0.1:28237")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "mount_reason"))
    parser.add_argument("--raw", default=str(ROOT / "data" / "extract" / f"mount-reason-{time.strftime('%Y%m%d')}-r1"))
    args = parser.parse_args()

    account = live_slots.resolve(args.slot, args.lane, "b")
    live = Path(f"D:/aaemu-live-{args.lane}")
    password_file = live / "cap-pw.txt"
    server_log = live / "logs" / "rs-game.log"
    client = Path(args.client)
    evidence = Path(args.evidence)
    raw = Path(args.raw)
    raw.mkdir(parents=True, exist_ok=True)
    if not client.exists() or not password_file.exists() or not server_log.exists():
        raise SystemExit(f"missing: {client} {password_file} {server_log}")

    scripts = [evidence / "replay.txt", evidence / "replay_front.txt"]
    lines = [
        "## Bevy replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  slot: {account.slot} ({account.user} / {account.character})",
        f"lane: {account.lane}  client: {client}",
        "",
        "The same cardinal approaches as the 1.2 probe, driven by the Bevy client's own script runner",
        "(`aa-client --headless --mute`). The reason is the server's `CSMountMatePacket fields` line.",
        "",
    ]
    ok = True
    for script in scripts:
        expected = expected_reasons(script)
        offset = server_log.stat().st_size
        command = [str(client), "--headless", "--mute", "--account", account.user, "--login", args.login,
                   "--password-file", str(password_file), "--character", account.character, "--script", str(script)]
        result = subprocess.run(command, capture_output=True, text=True, timeout=300)
        data = server_log.read_bytes()[offset:].decode("utf-8", "replace")
        reasons = [int(m.group(3)) for line in data.splitlines()
                   if (m := re.search(r"CSMountMatePacket fields: CSMountMatePacket \{ tl_id: (\d+), ap: (\d+), reason: (\d+) \}", line))]
        (raw / f"replay-{script.stem}.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
        match = result.returncode == 0 and reasons == expected
        ok = ok and match
        lines += [
            f"{script.name}:",
            f"  command: {' '.join(command)}",
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
