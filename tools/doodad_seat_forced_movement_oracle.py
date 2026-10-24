"""Replay pinned native seat/controller instructions; no live DLL/process/device.

Run from the repository root with the existing Unicorn/pefile RE dependencies.
Only ignored replay output is written; original and retained evidence is read-only.
"""
from pathlib import Path
import hashlib
import importlib.util
import json

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / 'tools/evidence/doodad_seats/forced_movement'
OUTPUT = ROOT / 'data/extract/aa-client/seat-forced-movement-replay'


def main():
    spec = importlib.util.spec_from_file_location('seat_forced_native', EVIDENCE / 'oracle.py')
    native = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(native)

    def verify_prior():
        rows = json.loads((EVIDENCE / 'baseline.json').read_text())['prior_files']
        for row in rows:
            data = (EVIDENCE / 'prior' / row['name']).read_bytes()
            assert len(data) == row['bytes']
            assert hashlib.sha256(data).hexdigest() == row['sha256'], row['name']
        return len(rows)

    OUTPUT.mkdir(parents=True, exist_ok=True)
    native.OUT = OUTPUT
    native.verify_prior = verify_prior
    native.main()
    actual = json.loads((OUTPUT / 'cases.json').read_text())
    expected = json.loads((EVIDENCE / 'cases.json').read_text())
    assert actual == expected, 'native instruction replay differs from retained evidence'
    print('All retained forced-movement results match exactly')


if __name__ == '__main__':
    main()
