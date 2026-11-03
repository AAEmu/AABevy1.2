"""Start the original 1.2 client for a skill parity run and bring it into the world.

    py -3 tools/skillparity/launch_original.py --launcher <lane>/bin/aaemu-launcher.exe --client <client>/bin32/archeage.exe
        --dll <lane-target>/aadriver/aadriver.dll --driver <lane-target>/live/aaemu-clientdriver.exe
        --login 127.0.0.1:24237 --user clienttest --password-file <file> --character Skilltrace [--pid-file run.pid]

The launcher is started detached (the client outlives this script), with sound on (the trace's `sound` rows need a working sound
system: a silent client's CreateSound returns nothing) and a hidden window. As soon as the script system answers, the game's
volume cvars are set to 0, so nothing is heard. Then the character is entered (server select, character select, enter world,
`tools/parity/run_original.py ensure_world`) and the client pid is printed (and written to --pid-file). An already running client
is adopted with --pid instead of starting a new one. Never stops a client.
"""
from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent / "parity"))

import mcp_driver  # noqa: E402

MUTE = ("for _, n in ipairs({'s_GameMasterVolume','s_GameSFXVolume','s_GameMusicVolume','s_GameDialogVolume',"
        "'s_MasterVolume','s_SFXVolume','s_MusicVolume','s_DialogVolume'}) do "
        "pcall(function() if System.GetCVar(n) ~= nil then System.SetCVar(n, 0) end end) end return 1")


def log(*a: object) -> None:
    print("[launch]", *a, file=sys.stderr, flush=True)


def client_pids() -> set[int]:
    out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq archeage.exe", "/FO", "CSV", "/NH"], capture_output=True, text=True).stdout
    pids = set()
    for line in out.splitlines():
        parts = [p.strip('"') for p in line.split('","')]
        if len(parts) > 1 and parts[1].strip('"').isdigit():
            pids.add(int(parts[1].strip('"')))
    return pids


def start(args: argparse.Namespace) -> int:
    before = client_pids()
    cmd = [str(args.launcher), "--client", str(args.client), "--login", args.login, "--user", args.user,
           "--password-file", str(args.password_file), "--driver-dll", str(args.dll), "--multi-client", "--sound"]
    if not args.visible:
        cmd.append("--hidden")
    flags = getattr(subprocess, "DETACHED_PROCESS", 0) | getattr(subprocess, "CREATE_NEW_PROCESS_GROUP", 0)
    subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, creationflags=flags)
    deadline = time.time() + 120
    while time.time() < deadline:
        new = client_pids() - before
        if new:
            return min(new)
        time.sleep(1)
    raise SystemExit("the client did not start")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mcp_driver.add_driver_args(ap)
    ap.add_argument("--character", required=True)
    ap.add_argument("--visible", action="store_true", help="a visible window instead of the hidden 1280x720 one")
    ap.add_argument("--pid-file", type=Path, default=None)
    ap.add_argument("--timeout", type=float, default=300.0)
    args = ap.parse_args(argv)
    pid = args.pid or start(args)
    log("client pid", pid)
    mcp = mcp_driver.Mcp.from_args(args, None)
    try:
        import run_original as parity  # tools/parity/run_original.py

        muted = False
        deadline = time.time() + args.timeout
        while time.time() < deadline and not muted:
            try:
                muted = mcp.text("client_lua", pid=pid, target="system", code=MUTE).strip() == "1"
            except Exception:  # the script system is not up yet
                time.sleep(1)
        log("volume muted" if muted else "could not mute the volume")
        player = parity.ensure_world(mcp, pid, args.character, ability=1, create=False, timeout_s=args.timeout)
        mcp.text("client_lua", pid=pid, target="system", code=MUTE)
        log("in the world:", player)
    finally:
        mcp.close()
    if args.pid_file:
        args.pid_file.write_text(str(pid))
    print(pid)
    return 0


if __name__ == "__main__":
    sys.exit(main())
