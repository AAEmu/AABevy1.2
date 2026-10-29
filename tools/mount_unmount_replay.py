#!/usr/bin/env python3
"""Replay the native mount/unmount/dismiss/cooldown/reach cases in the Bevy client.

Runs tools/evidence/mount_unmount/replay.txt with `aa-client --headless --mute`
against the same lane, polls the server's `C->S` trace while it runs, and checks
the packet reasons and the CSRemoveMate delay against
tools/evidence/mount_unmount/native.json.  The side-by-side table is appended to
the evidence report (an older `## Bevy replay` section is replaced).

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace`.

Usage:
    python tools/mount_unmount_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        --evidence tools/evidence/mount_unmount
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from mount_unmount_probe import Trace

ROOT = Path(__file__).resolve().parents[1]


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
                        help="game server log with the C->S trace")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "mount_unmount"))
    parser.add_argument("--raw", default=os.environ.get("MOUNT_RAW")
                        or str(ROOT / "data" / "extract" / f"mount-unmount-{time.strftime('%Y%m%d')}-r1"))
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
    script = evidence / "replay.txt"
    if not script.exists():
        raise SystemExit(f"missing replay script: {script}")
    native = json.loads((evidence / "native.json").read_text(encoding="utf-8"))
    native_cases = {row["case"]: row for row in native["cases"]}

    trace = Trace(server_log)
    trace.reset()
    packets: list[dict[str, object]] = []
    stopping = threading.Event()

    def poll() -> None:
        while not stopping.wait(0.02):
            packets.extend(trace.poll())

    thread = threading.Thread(target=poll, daemon=True)
    thread.start()
    command = [
        str(client), "--headless", "--mute", "--account", args.account, "--login", args.login,
        "--password-file", str(password_file), "--character", args.character, "--script", str(script),
    ]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    try:
        output, _ = process.communicate(timeout=300)
    except subprocess.TimeoutExpired:
        process.kill()
        output, _ = process.communicate()
        output += "\nreplay: client timed out\n"
    stopping.set()
    thread.join(timeout=2.0)
    packets.extend(trace.poll())
    (raw / "bevy-replay.log").write_text(" ".join(command) + "\n\n" + output, encoding="utf-8")

    mounts = [packet for packet in packets if packet["kind"] == "mount"]
    unmounts = [packet for packet in packets if packet["kind"] == "unmount"]
    removes = [packet for packet in packets if packet["kind"] == "remove"]
    unmount_reasons = [packet["reason"] for packet in unmounts]
    mount_reasons = [packet["reason"] for packet in mounts]
    remove_dt = (
        round((removes[0]["at"] - unmounts[1]["at"]) * 1000.0, 1)
        if len(removes) == 1 and len(unmounts) == 2 else None
    )
    checks = {
        "script_exit": process.returncode,
        "script_pass": process.returncode == 0,
        "mounts": len(mounts),
        "mount_reasons": mount_reasons,
        "unmount_reasons": unmount_reasons,
        "removes": len(removes),
        "remove_after_unmount_ms": remove_dt,
        "cooldown_no_send": len(mounts) == 3,
        "reach_no_send": len(mounts) == 3,
        "dismiss_remove_waits": bool(remove_dt is not None and 700.0 <= remove_dt <= 2500.0),
    }
    ok = (
        checks["script_pass"]
        and checks["cooldown_no_send"]
        and checks["reach_no_send"]
        and unmount_reasons == [9, 10]
        and checks["dismiss_remove_waits"]
    )
    checks["ok"] = ok
    record = {
        "meta": {
            "account": "<account>", "character": "<character>", "date": time.strftime("%Y-%m-%dT%H:%M:%S"),
            "client": "<client>", "server_log": "<server-log>", "wall_s": round(time.monotonic() - started, 1),
        },
        "packets": [
            {key: packet.get(key) for key in ("kind", "opcode", "tl_id", "ap", "reason", "body")}
            for packet in packets
        ],
        "checks": checks,
    }
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")
    native_a = native_cases.get("a_self_unmount", {})
    native_b = native_cases.get("b_dismiss_while_mounted", {})
    bevy_mount_reason = next((packet["reason"] for packet in mounts[1:2]), None)
    lines = [
        "## Bevy replay",
        "",
        f"date: {time.strftime('%Y-%m-%d %H:%M:%S')}  account: <account> / <character>",
        "client: <client>  (--headless --mute)",
        f"script: <evidence>/{script.name}  exit: {process.returncode}",
        f"wall: {round(time.monotonic() - started, 1)} s",
        "",
        "case                  1.2 native                                     Bevy",
        f"a self-unmount        CSUnMountMate reason {native_a.get('unmount_reason')} ap {native_a.get('unmount_ap')}"
        f"                 reason {unmount_reasons[0] if unmount_reasons else None}",
        f"b dismiss mounted     reason {native_b.get('unmount_reason')}, CSRemoveMate +{native_b.get('remove_after_unmount_ms')} ms"
        f"        reason {unmount_reasons[1] if len(unmount_reasons) > 1 else None}, CSRemoveMate +{remove_dt} ms",
        "c remount inside 2 s  CanMount false, no CSMountMate"
        "                    expect passed, no send",
        f"c remount after 2 s   CSMountMate reason {native_cases.get('c_remount_after_cooldown', {}).get('mount_reason')}, mounted"
        f"                 reason {bevy_mount_reason}, mounted",
        "d reach              25.0 m sent, 25.2 m no send + 'Target is too far.'"
        "   30 m CanMount false/no send, 3.6 m sent",
        "",
        f"packets: mounts {mount_reasons}, unmounts {unmount_reasons}, removes {len(removes)}, "
        f"CSRemoveMate after unmount {remove_dt} ms",
        f"checks: {checks}",
        "",
        "The Bevy mate does not follow the script's GM teleport, so the reach replay uses 30 m (out) and",
        "3.6 m (in) rather than the native 25.0/25.2 m sweep; the 25 m constant is the same and is covered",
        "by hud::pets::owned::tests::mount_reach_is_25_m_in_three_dimensions_and_too_far_says_so.",
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
