"""Read-only verification of original modelview Init's pre-assembly writes.

Static bytes/data only. Does not load/execute a DLL or claim actor/assembly parity.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

ROOT = Path(__file__).resolve().parent.parent
GOLDEN = ROOT / "crates/aa-client/src/hud/modelview/trial/native-init.json"


def verify(dll: Path, golden: Path) -> None:
    expected = json.loads(golden.read_text(encoding="utf-8"))
    digest = hashlib.sha256()
    with dll.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(block)
        assert digest.hexdigest() == expected["module_sha256"], "wrong original module"
        source.seek(0)
        header = source.read(0x1000)
        pe = struct.unpack_from("<I", header, 0x3C)[0]
        assert header[pe:pe + 4] == b"PE\0\0"
        assert struct.unpack_from("<H", header, pe + 24)[0] == 0x10B, "expected PE32"
        count = struct.unpack_from("<H", header, pe + 6)[0]
        optional_size = struct.unpack_from("<H", header, pe + 20)[0]
        base = struct.unpack_from("<I", header, pe + 52)[0]
        assert base == int(expected["module_image_base"], 16)

        def read(va: int, length: int) -> tuple[int, bytes]:
            rva = va - base
            for index in range(count):
                section = pe + 24 + optional_size + index * 40
                virtual_size, address, raw_size, raw_pointer = struct.unpack_from("<IIII", header, section + 8)
                if address <= rva < address + max(virtual_size, raw_size):
                    assert rva - address + length <= raw_size, "not initialized file data"
                    offset = raw_pointer + rva - address
                    source.seek(offset)
                    return offset, source.read(length)
            raise ValueError(f"unmapped VA {va:x}")

        start, end = map(lambda value: int(value, 16), expected["init_range"])
        offset, code = read(start, end - start)
        assert offset == expected["init_file_offset"]
        assert code.hex() == expected["init_bytes"]
        offset, constants = read(int(expected["data_va"], 16), 6)
        assert offset == expected["data_file_offset"]
        assert constants.hex() == expected["initial_appearance_hex"]
        # Original store of zero to widget+2e98; constant loads/stores follow it.
        _, selector = read(int(expected["selector_zero_write_va"], 16), 10)
        assert selector == bytes.fromhex("c786982e000000000000")
        assert expected["initial_selector"] == 0
    print("PASS: pinned original Init bytes, selector-zero store and six initialized bytes; static proof only")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dll", type=Path, default=Path("<client12-unpacked>/x2game_unpacked.dll"))
    parser.add_argument("--golden", type=Path, default=GOLDEN)
    args = parser.parse_args()
    verify(args.dll, args.golden)
