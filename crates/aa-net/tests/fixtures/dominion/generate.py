"""Pinned territory call-site bodies and independent raw-deflate alignment fixture.

Original x2game SHA256 851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e.
The original serializers executed with synthetic typed scalar sinks, not concrete
CryNetwork primitives. Headers use the existing Rust/lane convention. No bool/blob
primitive proof, native malformed-input policy or transport capture is claimed.
"""
from pathlib import Path
import hashlib
import json
import struct
import zlib

OUT = Path(__file__).resolve().parent
CASES = [
    ("sc020", 0x20, "397b7a70", "3412efcdab89"),
    ("sc021", 0x21, "397b7ab0", "3412efcdab8910325476efcdab8967452381"),
    ("sc022", 0x22, "397b7b10", "3412efcdab896745238121436587a9cbed8ff0debc9a7856341200006040"),
    ("sc023", 0x23, "397b7b60", "3412efcdab8910325476"),
    ("sc024", 0x24, "397b7bb0", "3412efcdab89674523813412"),
    ("sc025", 0x25, "397b7c00", "3412"),
    ("sc0eb", 0xeb, "397b48f0", "3412"),
]


def main():
    rows = []
    raw = bytearray()
    for name, opcode, serializer, hex_body in CASES:
        body = bytes.fromhex(hex_body)
        filename = name + ".body.bin"
        (OUT / filename).write_bytes(body)
        rows.append(dict(file=filename, opcode=opcode, serializer=serializer,
                         bytes=len(body), sha256=hashlib.sha256(body).hexdigest(), body_hex=hex_body))
        # Repeat each receipt before a final unrelated empty policy-list packet.
        raw.extend((struct.pack("<HH", 0, opcode) + body) * 2)
    raw.extend(bytes.fromhex("00000b0000"))
    encoder = zlib.compressobj(wbits=-15)
    compressed = encoder.compress(raw) + encoder.flush()
    payload = b"\xdd\x04" + struct.pack("<H", 2 * len(CASES) + 1) + compressed
    assert zlib.decompress(payload[4:], wbits=-15) == raw
    (OUT / "repeated-deltas.dd04.bin").write_bytes(payload)
    metadata = dict(scope=__doc__, cases=rows, zlib_version=zlib.ZLIB_RUNTIME_VERSION,
                    compressed_sha256=hashlib.sha256(payload).hexdigest(),
                    compressed_bytes=len(payload), inner_bytes=len(raw), inner_hex=raw.hex())
    (OUT / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf8")


if __name__ == "__main__":
    main()
