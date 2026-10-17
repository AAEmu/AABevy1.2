"""Builds crates/aa-audio/data/vorbis_setup.bin from python-fsb5's fsb5/vorbis_headers.py.

    python -I make_setup_table.py <vorbis_headers.py> <out.bin> [crc ...]

Without CRCs every entry is written, otherwise only those (the survey prints the ones the game
uses).  The input is a `lookup = {crc: b'...'}` literal; it is parsed with ast.literal_eval and
never executed.  Output: u32 count, then count x { u32 crc, u32 length, setup header bytes }, all
little endian, sorted by CRC.
"""
import ast
import struct
import sys


def main():
    src = open(sys.argv[1], encoding="utf-8").read()
    prefix = "lookup = "
    assert src.startswith(prefix), "unexpected file layout"
    table = ast.literal_eval(src[len(prefix):])
    want = [int(a) for a in sys.argv[3:]] or sorted(table)
    missing = [c for c in want if c not in table]
    if missing:
        sys.exit(f"CRCs not in the table: {missing}")
    out = bytearray(struct.pack("<I", len(want)))
    for crc in sorted(want):
        data = table[crc]
        out += struct.pack("<II", crc, len(data)) + data
    open(sys.argv[2], "wb").write(out)
    print(f"{len(want)} of {len(table)} setup headers, {len(out)} bytes")


main()
