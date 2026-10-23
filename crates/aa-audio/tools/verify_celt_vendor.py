"""Verify the vendored subset and complete patch against the pinned Xiph archive.

Usage: python -B verify_celt_vendor.py <celt-v0.11.tar.gz>
No downloading, extraction or mutation is performed.
"""

import argparse
import difflib
import hashlib
import json
from pathlib import Path
import tarfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    args = parser.parse_args()
    assert hashlib.sha256(args.archive.read_bytes()).hexdigest() == "4fa104630f9bab52a5009cfd2c7a3b7431c8f5876a2704643caef1c09705e0a0"
    root = Path(__file__).resolve().parent.parent / "vendor/celt-0.11.0"
    records = json.loads((root / "FILES.json").read_text())
    differences = []
    with tarfile.open(args.archive) as archive:
        for row in records:
            upstream = archive.extractfile("celt-v0.11/" + row["upstream_path"]).read()
            vendored = (root / row["file"]).read_bytes()
            assert hashlib.sha256(upstream).hexdigest() == row["upstream_sha256"], row["file"]
            assert hashlib.sha256(vendored).hexdigest() == row["vendored_sha256"], row["file"]
            differences.extend(difflib.unified_diff(upstream.decode().splitlines(True), vendored.decode().splitlines(True),
                fromfile="a/" + row["upstream_path"], tofile="b/" + row["upstream_path"]))
    assert "".join(differences) == (root / "aa-celt.patch").read_text()
    print(f"Verified pinned archive, {len(records)} source/license hashes and complete upstream patch")


if __name__ == "__main__":
    main()
