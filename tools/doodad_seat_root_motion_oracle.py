"""Replay the retained original-instruction seat root policy, without live DLL loading.

Requires the pinned local original modules and pre-existing pefile/capstone/unicorn.
Writes only ignored replay output. No assets, native modules or goldens are changed.
"""
import hashlib
import importlib
import json
from pathlib import Path
import runpy
import shutil
import sys

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "tools/evidence/doodad_seats/root_motion"
OUT = ROOT / "data/extract/aa-client/seat-root-motion-replay"


def main():
    provenance = json.loads((EVIDENCE / "provenance.json").read_text(encoding="utf-8"))
    modules = provenance["native_binaries"]
    for module in modules:
        actual = hashlib.sha256(Path(module["path"]).read_bytes()).hexdigest()
        assert actual == module["sha256"], module["role"]
    OUT.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(EVIDENCE / "assets.json", OUT / "assets.json")
    sys.path.insert(0, str(EVIDENCE))
    probe = importlib.import_module("probe")
    probe.ROOT = ROOT
    probe.OUT = OUT
    probe.SOURCES = {module["role"]: Path(module["path"]) for module in modules}
    runpy.run_path(str(EVIDENCE / "native_cases.py"), run_name="__main__")
    expected = json.loads((EVIDENCE / "native-cases.json").read_text(encoding="utf-8"))
    actual = json.loads((OUT / "native-cases.json").read_text(encoding="utf-8"))
    assert actual == expected, "original-instruction results changed"
    print("Seat root-policy replay matches every retained original-instruction result.")


if __name__ == "__main__":
    main()
