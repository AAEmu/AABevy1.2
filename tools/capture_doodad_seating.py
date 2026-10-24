"""Unattended original-asset seating controls; no server connection.

Pass a task-owned copy of the example binary. Every capture is rejected on a
runtime warning/error, missing readiness metrics, wrong seat state, or a missing PNG.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ANSI = re.compile(r"\x1b\[[0-9;]*m")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--label", default="")
    args = parser.parse_args()
    binary = args.binary.resolve()
    assert re.fullmatch(r"[a-z0-9-]*", args.label), "use a plain result label"
    group = "doodad-seating" + ("-" + args.label if args.label else "")
    shots = ROOT / "shots/resume" / group
    logs = ROOT / "data/local/resume" / group
    shots.mkdir(parents=True, exist_ok=True)
    logs.mkdir(parents=True, exist_ok=True)
    paths = sorted((ROOT / "crates/aa-client/src/ingame/seating").glob("*.rs"))
    paths += [ROOT / "crates/aa-client" / p for p in [
        "src/ingame/seating.rs", "src/ingame/units.rs", "src/ingame/player.rs",
        "src/ingame/anim.rs", "src/ingame/idle.rs", "src/ingame/combat.rs",
        "src/ingame/mod.rs", "src/ingame/skill_motion.rs", "src/hud/doodad.rs", "examples/doodad_seating.rs",
    ]]
    sources = {str(p.relative_to(ROOT)).replace("\\", "/"): digest(p) for p in paths}
    cases = [
        ("piano-before", ["--before"], False),
        ("piano-after", [], True),
        ("piano-side-after", ["--side"], True),
        ("drum-before", ["--drum", "--before"], False),
        ("drum-after", ["--drum"], True),
        ("unbond-after", ["--unbond-after", "12"], False),
    ]
    result = {"scope": "Original assets and production seating with explicit synthetic receipts, no socket/live-server/native-render proof. Before omits the bond in the same final binary.",
              "binary": str(binary), "binary_sha256": digest(binary), "source_sha256": sources,
              "cases": []}
    for name, flags, seated in cases:
        shot = shots / (name + ".png")
        if shot.exists():
            raise RuntimeError(f"capture path already exists: {shot}; choose a fresh result set")
        command = [str(binary), *flags, "--size", "1280x720", "--shot", str(shot), "--shot-after", "20"]
        print("Capturing", name, flush=True)
        proc = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=120)
        log = logs / (name + ".log")
        log.write_bytes(proc.stdout)
        text = ANSI.sub("", proc.stdout.decode("utf-8", errors="replace"))
        errors = re.findall(r"^.*(?:\bWARN\b|\bERROR\b|\berror:|module not found|panicked at|validation error).*$", text, re.M | re.I)
        # Screenshot capture can itself request a new pipeline after this point.
        # Check the last scene measurement before the request, plus the save ACK.
        before_shot = text.split("aa-dev: screenshot ->", 1)[0]
        rows = re.findall(r"seat-control: before=(true|false) seated=(true|false) position=(.*?) scale=(.*?) loop=(true|false) doodads=(\d+) pipelines=(\d+) waiting=(\d+)", before_shot)
        assert proc.returncode == 0 and not errors, (name, proc.returncode, errors[:4])
        assert shot.exists() and rows, (name, "missing capture/metrics")
        assert "Screenshot saved to" in text, (name, "missing save acknowledgement")
        last = rows[-1]
        assert (last[1] == "true") == seated and int(last[5]) == 1 and int(last[7]) == 0, (name, last)
        if seated:
            assert last[4] == "true", (name, "base loop not reached", last)
        result["cases"].append({"name": name, "command": command, "exit": proc.returncode,
            "log": str(log.relative_to(ROOT)), "log_sha256": digest(log), "png": str(shot.relative_to(ROOT)),
            "png_sha256": digest(shot), "last": last, "fps": [float(x) for x in re.findall(r"aa-dev: fps=([\d.]+)", text)],
            "warning_error_count": len(errors)})
        print(name, "passed", last, flush=True)
    assert result["binary_sha256"] == digest(binary), "binary changed during captures"
    assert all(digest(ROOT / path) == value for path, value in sources.items()), "sources changed during captures"
    (logs / "captures.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print("All captures passed; inspect every PNG before claiming visual success.")


if __name__ == "__main__":
    main()
