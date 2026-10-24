"""Run only this probe's copied exe, sequentially; retain commands, hashes and logs.

Build first: cargo build -p aa-cry --example native_pass_probe --locked
Python 3; validation uses Pillow (no new Rust dependency).
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[4]
HISTORICAL = ROOT / "data/extract/aa-cry/native-pass-probe-20261024"
DEFAULT_OUT = ROOT / "data/extract/aa-cry/native-pass-probe-review01-fix-20261024/verified"
CONFIGS = {
    "bindless": [],
    "ordinary": ["--no-bindless"],
    "depth": ["--attachments", "depth"],
    "normal": ["--attachments", "normal"],
    "motion": ["--attachments", "motion"],
    "noz-motion": ["--attachments", "motion", "--no-z"],
    "targets": ["--separate-targets"],
    "occlusion": ["--occlusion"],
    "shadow-stock": ["--stock-controls", "--shadows", "--stock-renderer"],
    "shadow-split": ["--stock-controls", "--shadows"],
    "shadow-off": ["--stock-controls", "--shadows", "--stock-renderer", "--no-shadows"],
    "shadow-unmarked": ["--stock-controls", "--shadows", "--unmarked-views"],
    "point-stock": ["--stock-controls", "--shadows", "--point-light", "--stock-renderer"],
    "point-split": ["--stock-controls", "--shadows", "--point-light"],
    "point-off": ["--stock-controls", "--shadows", "--point-light", "--stock-renderer", "--no-shadows"],
    "readiness": ["--readiness-transition"],
    "readiness-ordinary": ["--readiness-transition", "--no-bindless"],
    "none-stock": ["--attachments", "none", "--stock-controls", "--shadows", "--stock-renderer"],
    "none-split": ["--attachments", "none", "--stock-controls", "--shadows"],
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def archive_attachments(directory):
    """Lossless packing of only this probe's readbacks, verified before unlinking."""
    directory = directory.resolve()
    files = list(directory.glob("v*-f*-*.bin"))
    if not files:
        return
    temporary = directory / "attachments.zip.tmp"
    destination = directory / "attachments.zip"
    with zipfile.ZipFile(temporary, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
        for path in files:
            archive.write(path, path.name)
    with zipfile.ZipFile(temporary) as archive:
        assert archive.testzip() is None
        for path in files:
            assert archive.read(path.name) == path.read_bytes()
    temporary.replace(destination)
    for path in files:
        assert path.resolve().parent == directory
        path.unlink()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=DEFAULT_OUT)
    parser.add_argument("--select", nargs="*", choices=CONFIGS)
    parser.add_argument("--archive", action="store_true", help="Losslessly compress readbacks after each run")
    args = parser.parse_args()
    out = args.out.resolve()
    assert not out.is_relative_to(HISTORICAL.resolve()), "507b6546 evidence is historical; choose a new output directory"
    out.mkdir(parents=True, exist_ok=True)
    exe = out / "native-pass-probe-owned-copy.exe"
    shutil.copy2(ROOT / "target/debug/examples/native_pass_probe.exe", exe)
    report_file = out / "captures.json"
    report = json.loads(report_file.read_text()) if report_file.exists() else {}
    binary_hash = sha(exe)
    source_hashes = {str(p.relative_to(ROOT)): sha(p) for p in Path(__file__).parent.iterdir() if p.suffix in (".rs", ".wesl", ".py")}
    try:
        for name in args.select or CONFIGS:
            directory = out / name
            directory.mkdir(exist_ok=True)
            command = [str(exe), "--size", "640x480", "--shot", str(directory / "shot.png"), "--shot-after", "6", "--exit-after", "20"] + CONFIGS[name]
            if name in ("bindless", "ordinary", "depth", "normal", "motion", "noz-motion", "targets", "occlusion", "readiness", "readiness-ordinary", "none-split"):
                command += ["--capture", "--out", str(directory)]
            print("RUN", name, flush=True)
            with (directory / "run.log").open("w", encoding="utf-8") as log:
                result = subprocess.run(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, timeout=55)
            log = (directory / "run.log").read_text(encoding="utf-8")
            bad = [line for line in log.splitlines() if re.search(r"\bERROR\b|panicked|unresolved|Path not found", line, re.I)]
            assert result.returncode == 0 and not bad and (directory / "shot.png").exists(), (name, result.returncode, bad)
            report[name] = dict(command=command, exe_sha256=binary_hash, source_sha256=source_hashes,
                                fps=re.findall(r"aa-dev: fps=([0-9.]+) frame_ms=([0-9.]+)", log),
                                timings=re.findall(r"timing ([^=]+)=([0-9.]+)ms", log),
                                files={p.name: sha(p) for p in directory.iterdir() if p.is_file()})
            if args.archive:
                archive_attachments(directory)
                if (directory / "attachments.zip").exists():
                    report[name]["attachment_archive_sha256"] = sha(directory / "attachments.zip")
            report_file.write_text(json.dumps(report, indent=2), encoding="utf-8")
            print("PASS", name, "fps", report[name]["fps"], flush=True)
    finally:
        # Only the exact copy created above; no process or directory cleanup.
        exe.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
