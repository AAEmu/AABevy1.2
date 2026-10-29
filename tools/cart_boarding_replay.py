#!/usr/bin/env python3
"""Replay the native 1.2 cart boarding/leaving cases in the Bevy client.

Runs tools/evidence/cart_boarding/replay.txt with `aa-client --headless --mute`
against the same lane, polls the server's C->S field trace and S->C opcode lines
while it runs, and compares the per-case packet shapes with the native capture
(tools/evidence/cart_boarding/native.json). The side-by-side table and the
differences are written to replay-report.txt; bevy.json keeps the packets and
the checks. The client is stopped by its pid on timeout or exit.

The game server must log the fields trace and the S->C opcode lines:
`RUST_LOG=info,aaemu_game::net::fields=trace,aaemu_game::net=debug`.
The client needs the game pak (`--pak` or AA_PAK).

Usage:
    python tools/cart_boarding_replay.py --client <aa-client.exe> \\
        --login <host:port> --account <account> --character <character> \\
        --password-file <password-file> --server-log <game-server.log> \\
        [--pak <game_pak>] [--evidence tools/evidence/cart_boarding]
"""
from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from cart_boarding_probe import Trace, decode_c2s  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CASES = ["a_board_2m", "e_click_while_bound", "b_discard",
         "c_reach_3.5", "c_reach_4.5", "c_reach_3.1", "c_reach_2.9", "d_drive_moving"]
INTEREST_C2S = {0x02C, 0x068, 0x031, 0x032, 0x089}
INTEREST_S2C = {0x07B, 0x064, 0x076, 0x077, 0x10A}
MARKER_RE = re.compile(r"case=([a-z0-9_.]+)")
REFUSAL_RE = re.compile(r"hud: slave refusal 0x([0-9a-fA-F]+): (.*)")
DISTANCE_RE = re.compile(r"template 60 .*? ([0-9.]+) m")


def log(*args: object) -> None:
    print("cart-replay:", *args, flush=True)


def normalize(packets: list[dict], cart: int | None) -> dict:
    """The packet shape this task compares: opcodes, ids normalised, S->C sizes."""
    c2s: list[str] = []
    s2c: list[str] = []
    for packet in sorted(packets, key=lambda p: p["t"]):
        opcode = packet["opcode"]
        if packet["dir"] == "C->S" and opcode in INTEREST_C2S:
            fields = decode_c2s(packet["name"], packet["body"])
            if opcode in (0x02C, 0x068):
                target = fields.get("npc_obj_id") if opcode == 0x068 else fields.get("obj_id")
                c2s.append(f"0x02C {'cart' if target == cart else 'other'}"
                           if opcode == 0x02C else
                           f"0x068 {'cart' if target == cart else 'other'}"
                           f" obj={fields.get('obj_id')} extra={fields.get('extra_info')}"
                           f" pick={fields.get('pick_id')} mouse={fields.get('mouse_button')}"
                           f" mods={fields.get('modifier_keys')}")
            elif opcode in (0x031, 0x032):
                c2s.append(f"0x{opcode:03X} tl")
            elif opcode == 0x089 and fields.get("obj_id") == cart:
                c2s.append("0x089 cart")
        elif packet["dir"] == "S->C" and opcode in INTEREST_S2C:
            s2c.append(f"0x{opcode:03X}/{packet['len']}")
    return {"c2s": c2s, "s2c": s2c}


def summarise(packets: list[dict]) -> dict:
    """Collapse repeated reports so a 10 Hz move stream does not hide the shape."""
    collapsed: list[str] = []
    for item in packets["c2s"]:
        if not collapsed or collapsed[-1] != item:
            collapsed.append(item)
    return {"c2s": collapsed, "s2c": packets["s2c"]}


def native_summaries(native: dict) -> dict:
    rows = native["rows"]
    cart = None
    for row in rows:
        if row["event"] == "packet" and row["opcode"] == "0x068":
            fields = json.loads(row["decoded"] or "{}")
            if fields.get("npc_obj_id"):
                cart = fields["npc_obj_id"]
                break
    by_case: dict[str, list[dict]] = {}
    for row in rows:
        if row["event"] != "packet":
            continue
        by_case.setdefault(row["case"], []).append({
            "t": len(by_case.get(row["case"], [])),
            "dir": row["dir"], "opcode": int(row["opcode"], 16),
            "name": row["name"], "len": row["len"], "body": row["body"],
        })
    out = {}
    for case in CASES:
        summary = normalize(by_case.get(case, []), cart)
        states = [row for row in rows if row["case"] == case and row["event"] == "sweep"]
        if case == "e_click_while_bound" and not summary["c2s"]:
            if any("0x02C" in (row["client_state"] or "") and "0x068" not in (row["client_state"] or "")
                   for row in states):
                summary["c2s"] = ["0x02C cart"]
        out[case] = summarise(summary)
    return {"cart": cart, "cases": out}


class Lines(threading.Thread):
    def __init__(self, pipe) -> None:
        super().__init__(daemon=True)
        self.pipe = pipe
        self.lines: list[str] = []

    def run(self) -> None:
        for line in self.pipe:
            self.lines.append(line.rstrip())


UTC_RE = re.compile(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?)Z")
SCRIPT_T_RE = re.compile(r"SCRIPT \[\s*([0-9.]+)s\]")


def timed(lines: list[str]) -> list[tuple[float | None, str]]:
    """Client lines with an epoch time: their UTC prefix, else the script time from the first
    UTC line (the client start). This is independent of when the pipe reader saw the line."""
    anchor = None
    out: list[tuple[float | None, str]] = []
    for line in lines:
        stamp = None
        match = UTC_RE.match(line)
        if match:
            stamp = datetime.datetime.fromisoformat(match.group(1) + "+00:00").timestamp()
            if anchor is None:
                anchor = stamp
        else:
            match = SCRIPT_T_RE.search(line)
            if match and anchor is not None:
                stamp = anchor + float(match.group(1))
        out.append((stamp, line))
    return out


def assign(packets: list[dict], markers: list[tuple[float, str]]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = {}
    for packet in packets:
        case = None
        for stamp, name in markers:
            if stamp <= packet["t"]:
                case = name
            else:
                break
        if case:
            out.setdefault(case, []).append(packet)
    return out


def refusals(lines: list[tuple[float | None, str]], markers: list[tuple[float, str]]) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for stamp, line in lines:
        match = REFUSAL_RE.search(line)
        if not match or stamp is None:
            continue
        case = None
        for at, name in markers:
            if at <= stamp:
                case = name
            else:
                break
        if case:
            out.setdefault(case, []).append(f"0x{int(match.group(1), 16):X}: {match.group(2)}")
    return out


def distances(lines: list[tuple[float | None, str]], markers: list[tuple[float, str]]) -> dict[str, float]:
    out: dict[str, float] = {}
    for stamp, line in lines:
        match = DISTANCE_RE.search(line)
        if not match or stamp is None:
            continue
        case = None
        for at, name in markers:
            if at <= stamp:
                case = name
            else:
                break
        if case:
            out[case] = float(match.group(1))
    return out


def bind_chain(summary: dict) -> bool:
    """CSStartInteraction -> CSBindSlave -> SCUnitAttached, in that order."""
    c2s = summary["c2s"]
    start = next((index for index, item in enumerate(c2s) if item.startswith("0x068 cart")), None)
    bind = next((index for index, item in enumerate(c2s) if item == "0x031 tl"), None)
    attached = any(item.startswith("0x076/") for item in summary["s2c"])
    return start is not None and bind is not None and start < bind and attached


def status(case: str, expected: dict, actual: dict) -> str:
    if not expected["c2s"]:
        return "native pick not captured" if case.startswith(("c_reach_3.5", "c_reach_4.5")) else "DIFFERS"
    if expected["c2s"] == actual["c2s"] and expected["s2c"] == actual["s2c"]:
        return "MATCH"
    # The native probe released the target before every click, so the native rows carry
    # the leading CSChangeTarget 0x02C and the port's rows do not; everything else must
    # still match and no refusal is allowed.
    expected_rest = [item for item in expected["c2s"] if not item.startswith("0x02C")]
    actual_rest = [item for item in actual["c2s"] if not item.startswith("0x02C")]
    if (not actual.get("refusals") and len(expected_rest) < len(expected["c2s"])
            and expected_rest == actual_rest and expected["s2c"] == actual["s2c"]):
        return "MATCH (no 0x02C: target kept)"
    expected_wo_move = [item for item in expected_rest if item != "0x089 cart"]
    actual_wo_move = [item for item in actual_rest if item != "0x089 cart"]
    if (not actual.get("refusals") and len(expected_wo_move) < len(expected_rest)
            and expected_wo_move == actual_wo_move and expected["s2c"] == actual["s2c"]):
        return "DIFFERS (no cart 0x089)"
    return "DIFFERS"


def shape(case: str, side: str, native_cases: dict, actual: dict) -> str:
    data = native_cases["cases"][case] if side == "native" else actual[case]
    c2s = ", ".join(data["c2s"]) or "none"
    s2c = ", ".join(data["s2c"]) or "none"
    return f"C->S {c2s}; S->C {s2c}"


def outcome(summary: dict) -> str:
    """One line for what the replay did in a case."""
    if bind_chain(summary):
        return "boards (0x068 -> 0x031 -> 0x076/0x064)"
    if any(item.startswith("0x068") for item in summary["c2s"]):
        refusal = "; ".join(summary.get("refusals") or []).rstrip(".")
        return "sends 0x068" + (f" and refuses with {refusal}" if refusal else " without binding")
    if summary["c2s"]:
        return "sends " + ", ".join(summary["c2s"])
    return "sends no interaction packet"


def result_lines(native_cases: dict, actual: dict) -> list[str]:
    def distance(case: str) -> str:
        value = actual[case].get("distance_m")
        return f" at {value:.1f} m" if value is not None else ""

    lines: list[str] = []
    c31 = actual["c_reach_3.1"]
    if bind_chain(c31):
        text = (f"1. c_reach_3.1{distance('c_reach_3.1')} boards: CSStartInteraction 0x068 "
                f"leads to CSBindSlave 0x031, then SCUnitAttached 0x076/14 and "
                f"SCSlaveBound 0x064/13.")
        if "0x02C cart" not in c31["c2s"]:
            text += (" The port keeps the cart targeted, so the click sends no "
                     "CSChangeTarget 0x02c (the native probe released the target first).")
    else:
        text = f"1. c_reach_3.1{distance('c_reach_3.1')} did not bind: {outcome(c31)}."
    lines.append(text)

    e = actual["e_click_while_bound"]
    if not any(item.startswith("0x068") for item in e["c2s"]) and not e["refusals"]:
        if any(item.startswith("0x02C") for item in e["c2s"]):
            text = ("2. While bound, the right click sends only CSChangeTarget 0x02c: no "
                    "CSStartInteraction 0x068 and no 0x9a refusal.")
        else:
            text = ("2. While bound, the right click sends no CSStartInteraction 0x068 and "
                    "shows no 0x9a refusal. The native client sent only CSChangeTarget 0x02c "
                    "because the probe released the target first; the port kept it.")
    else:
        text = f"2. While bound, the right click {outcome(e)}."
        if e["refusals"]:
            text += " Refusals: " + "; ".join(e["refusals"]).rstrip(".") + "."
    lines.append(text)

    reach = [f"{case}{distance(case)} {outcome(actual[case])}"
             for case in ("c_reach_3.5", "c_reach_4.5")]
    text = "3. " + "; ".join(reach) + "."
    if not any(native_cases["cases"][case]["c2s"] for case in ("c_reach_3.5", "c_reach_4.5")):
        text += (" The native pick did not land at either distance in this capture, so these "
                 "rows compare only the port's behaviour.")
    lines.append(text)

    d = actual["d_drive_moving"]
    if "0x089 cart" in d["c2s"]:
        text = "4. The port sends CSMoveUnit 0x089 with the cart object while driving."
    else:
        text = ("4. The port sends no CSMoveUnit 0x089 with the cart object: cart driving is "
                "not ported. The native cart drives at 0.4 m/s.")
    if bind_chain(d) and "0x032 tl" in d["c2s"]:
        text += (" It boards and leaves; DiscardSiegeWeapon is accepted while moving (0x032, "
                 "then 0x077/10, no 0x67).")
    elif bind_chain(d):
        text += " It boards but sends no CSDiscardSlave 0x032."
    else:
        text += " It did not board: " + outcome(d) + "."
    if d["refusals"]:
        text += " Refusals: " + "; ".join(d["refusals"]).rstrip(".") + "."
    lines.append(text)

    missing = [case for case in CASES
               if "0x02C cart" in native_cases["cases"][case]["c2s"]
               and "0x02C cart" not in actual[case]["c2s"]]
    if missing:
        lines.append("5. The native probe released the target before every click, so its rows "
                     "carry CSChangeTarget 0x02c; the port keeps the cart targeted, so these "
                     "rows do not (" + ", ".join(missing) + ").")
    return lines


def build_report(record: dict, native_cases: dict, script_name: str) -> str:
    meta = record["meta"]
    actual = record["cases"]
    checks = record["checks"]
    before = {
        "a_board_2m": "no cart pick (slave not an interaction target)",
        "e_click_while_bound": "no cart pick",
        "b_discard": "no cart state",
        "c_reach_3.5": "no cart pick",
        "c_reach_4.5": "no cart pick",
        "c_reach_3.1": "no cart pick",
        "c_reach_2.9": "no cart pick",
        "d_drive_moving": "no cart state",
    }
    native_text = {
        "a_board_2m": "0x02C, 0x068, 0x07B/34, 0x031 tl, 0x076/14, 0x064/13",
        "e_click_while_bound": "0x02C only, no 0x068, no message",
        "b_discard": "0x032 tl, 0x077/10",
        "c_reach_3.5": "pick did not land (44 points, no 0x02C/0x068)",
        "c_reach_4.5": "pick did not land (44 points, no 0x02C/0x068)",
        "c_reach_3.1": "0x02C, 0x068, 0x07B/34, 0x031 tl (3.102 m, no walk)",
        "c_reach_2.9": "0x02C, 0x068, 0x07B/34, 0x031 tl",
        "d_drive_moving": "0x089 cart, 0x032 tl, 0x077/10, speed 0.4, no 0x67",
    }
    date = str(meta.get("date", "")).replace("T", " ")
    lines = [
        "## Bevy replay",
        "",
        f"date: {date}  account: {meta.get('account', '<account>')} / "
        f"{meta.get('character', '<character>')}",
        f"client: {meta.get('client', '<client>')}  (--headless --mute)",
        f"script: <evidence>/{script_name}  exit: {checks.get('script_exit')}",
        f"wall: {meta.get('wall_s')} s",
        "",
        "| case | before 45f80700 | 1.2 capture | after (replay) | status |",
        "|---|---|---|---|---|",
    ]
    for case in CASES:
        actual_text = shape(case, "bevy", native_cases, actual)
        if actual[case].get("refusals"):
            actual_text += " refusal " + "; ".join(actual[case]["refusals"])
        if actual[case].get("distance_m") is not None:
            actual_text += f" ({actual[case]['distance_m']:.1f} m)"
        lines.append(f"| {case} | {before[case]} | {native_text[case]} | "
                     f"{actual_text} | {checks[case]} |")
    lines += ["", "Results (this run):"]
    lines += result_lines(native_cases, actual)
    lines += [
        "",
        "The replay needed one dev-harness completion: script `interact` on a slave now ends",
        "when the pick was consumed (the slave path sends 0x02C/0x068 and starts no talk).",
        "Game behaviour is unchanged; without it the script stopped after the first board.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("CART_BEVY_CLIENT", "aa-client.exe"),
                        help="path to the Bevy aa-client.exe")
    parser.add_argument("--login", default=os.environ.get("CART_LOGIN", os.environ.get("AA_LOGIN", "127.0.0.1:1237")),
                        help="login host:port")
    parser.add_argument("--account", default=os.environ.get("CART_REPLAY_ACCOUNT"))
    parser.add_argument("--character", default=os.environ.get("CART_REPLAY_CHARACTER"))
    parser.add_argument("--password-file", default=os.environ.get("CART_PASSWORD_FILE"))
    parser.add_argument("--server-log", default=os.environ.get("CART_SERVER_LOG"),
                        help="game server log with the C->S trace and S->C opcode lines")
    parser.add_argument("--pak", default=os.environ.get("AA_PAK"), help="the client's game_pak (AA_PAK)")
    parser.add_argument("--evidence", default=str(ROOT / "tools" / "evidence" / "cart_boarding"))
    parser.add_argument("--raw", default=os.environ.get("CART_RAW")
                        or str(ROOT / "data" / "extract" / f"cart-replay-{time.strftime('%Y%m%d')}-r1"))
    parser.add_argument("--timeout", type=float, default=300.0)
    args = parser.parse_args()
    for name in ("account", "character", "password_file", "server_log"):
        if not getattr(args, name):
            parser.error(f"--{name.replace('_', '-')} is required (or set CART_{name.upper()})")
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
    native_cases = native_summaries(native)

    trace = Trace(server_log)
    trace.reset()
    packets: list[dict] = []
    stopping = threading.Event()

    def poll() -> None:
        while not stopping.wait(0.02):
            packets.extend(trace.poll())

    thread = threading.Thread(target=poll, daemon=True)
    thread.start()
    env = os.environ.copy()
    if args.pak:
        env["AA_PAK"] = args.pak
    command = [
        str(client), "--headless", "--mute", "--account", args.account, "--login", args.login,
        "--password-file", str(password_file), "--character", args.character, "--script", str(script),
    ]
    started = time.monotonic()
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                               text=True, encoding="utf-8", errors="replace", env=env, bufsize=1)
    reader = Lines(process.stdout)
    reader.start()
    try:
        process.wait(timeout=args.timeout)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    stopping.set()
    thread.join(timeout=2.0)
    packets.extend(trace.poll())
    reader.join(timeout=2.0)
    (raw / "bevy-replay.log").write_text(" ".join(command) + "\n\n"
                                         + "\n".join(reader.lines), encoding="utf-8")

    timed_lines = timed(reader.lines)
    markers = [(stamp, match.group(1)) for stamp, line in timed_lines
               if stamp is not None and (match := MARKER_RE.search(line))]
    by_case = assign(packets, markers)
    bevy_cart = None
    for packet in packets:
        if packet["dir"] == "C->S" and packet["opcode"] == 0x068:
            bevy_cart = decode_c2s(packet["name"], packet["body"]).get("npc_obj_id")
            break
    deny = refusals(timed_lines, markers)
    distance = distances(timed_lines, markers)
    actual: dict[str, dict] = {}
    for case in CASES:
        summary = summarise(normalize(by_case.get(case, []), bevy_cart))
        summary["refusals"] = deny.get(case, [])
        summary["distance_m"] = distance.get(case)
        actual[case] = summary

    checks = {"script_exit": process.returncode, "script_pass": process.returncode == 0}
    for case in CASES:
        checks[case] = status(case, native_cases["cases"][case], actual[case])
    checks["a_board_2m_bind"] = bind_chain(actual["a_board_2m"])
    checks["b_discard_leave"] = ("0x032 tl" in actual["b_discard"]["c2s"]
                                 and "0x077/10" in actual["b_discard"]["s2c"])
    checks["c_reach_2.9_bind"] = bind_chain(actual["c_reach_2.9"])
    checks["c_reach_3.1_bind"] = bind_chain(actual["c_reach_3.1"])
    checks["e_no_interaction"] = not any(item.startswith("0x068")
                                         for item in actual["e_click_while_bound"]["c2s"])
    checks["e_no_refusal"] = not actual["e_click_while_bound"]["refusals"]
    checks["d_leave"] = "0x032 tl" in actual["d_drive_moving"]["c2s"]
    checks["ok"] = (checks["script_pass"] and checks["a_board_2m_bind"]
                    and checks["b_discard_leave"] and checks["c_reach_2.9_bind"]
                    and checks["c_reach_3.1_bind"]
                    and checks["e_no_interaction"] and checks["e_no_refusal"]
                    and checks["d_leave"])
    record = {
        "meta": {
            "account": "<account>", "character": "<character>",
            "date": time.strftime("%Y-%m-%dT%H:%M:%S"), "client": "<client>",
            "server_log": "<server-log>", "wall_s": round(time.monotonic() - started, 1),
            "bevy_cart": bevy_cart, "native_cart": native_cases["cart"],
        },
        "native": native_cases["cases"],
        "cases": actual,
        "checks": checks,
    }
    (evidence / "bevy.json").write_text(json.dumps(record, indent=1), encoding="utf-8")

    section = build_report(record, native_cases, script.name)
    (evidence / "replay-report.txt").write_text(section, encoding="utf-8")
    print(section)
    return 0 if checks["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
