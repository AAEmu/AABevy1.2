"""Offline golden fixtures made with Python's zlib, independently of flate2.

Run from any directory: python crates/aa-net/tests/fixtures/deflate/generate.py
No network, game assets, server or account is accessed.
"""
import hashlib
import json
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parent
LIMIT = 16 * 1024 * 1024


def packet(opcode, body):
    return struct.pack("<HH", 0, opcode) + body


def points(i):
    # packets/world.rs SCUnitPointsPacket writes each displayed pool multiplied by 100.
    return packet(0xBA, (i + 1).to_bytes(3, "little")
                  + struct.pack("<ii", (300 + i * 31) * 100, (20 + i % 71) * 100))


def compress(raw, strategy=zlib.Z_DEFAULT_STRATEGY):
    compressor = zlib.compressobj(9, zlib.DEFLATED, -15, 8, strategy)
    return compressor.compress(raw) + compressor.flush()


fixed_raw = points(0) + packet(0xB4, b"\x01") + points(1)
dynamic_raw = b"".join(points(i) for i in range(256)) + packet(0xB4, b"\x01")
fixed = compress(fixed_raw, zlib.Z_FIXED)
dynamic = compress(dynamic_raw)
assert (fixed[0] >> 1) & 3 == 1
assert (dynamic[0] >> 1) & 3 == 2

# A non-final stored block ends on a byte boundary. Its successor is the independently
# produced final dynamic Huffman stream; this is one stream, not concatenated streams.
stored = struct.pack("<BHH", 0, len(fixed_raw), ~len(fixed_raw) & 0xFFFF) + fixed_raw
mixed = stored + dynamic
unknown_raw = points(0) + packet(0x7FFF, b"\x01\x02\x03") + packet(0xB4, b"\x01")
truncated_packet_raw = packet(0xB4, b"\x01") + points(0)[:-1]
cases = {
    "fixed": (fixed_raw, fixed),
    "dynamic": (dynamic_raw, dynamic),
    "mixed": (fixed_raw + dynamic_raw, mixed),
    "unknown-opcode": (unknown_raw, compress(unknown_raw, zlib.Z_FIXED)),
    "truncated-packet": (truncated_packet_raw, compress(truncated_packet_raw)),
    "empty": (b"", compress(b"")),
    "at-limit": (bytes(LIMIT), compress(bytes(LIMIT))),
    "over-limit": (bytes(LIMIT + 1), compress(bytes(LIMIT + 1))),
}
metadata = {"producer": "Python zlib", "zlib_runtime": zlib.ZLIB_RUNTIME_VERSION,
            "format": "raw RFC1951, wbits=-15", "fixtures": {}}
for name, (raw, compressed) in cases.items():
    assert zlib.decompress(compressed, -15) == raw
    (ROOT / (name + ".deflate")).write_bytes(compressed)
    if len(raw) < LIMIT:
        (ROOT / (name + ".raw")).write_bytes(raw)
    metadata["fixtures"][name] = {
        "compressed_bytes": len(compressed), "raw_bytes": len(raw),
        "compressed_sha256": hashlib.sha256(compressed).hexdigest(),
        "raw_sha256": hashlib.sha256(raw).hexdigest(),
    }
(ROOT / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
print(json.dumps(metadata, indent=2))
