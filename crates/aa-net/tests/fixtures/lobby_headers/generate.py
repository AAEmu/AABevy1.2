"""Offline scalar-writer fixtures; no server, credentials or aa-net encoder.

Lane `packets/g2c.rs::tests::lobby_packet_frames_are_byte_exact` supplies the
three literal goldens. Native 397aeb90 and 397b8b00 confirm payment/premium
field order. The varied payment exercises nonzero realPayTime (the lane
currently writes zero); it is a constructed reader fixture, not a capture.
"""
from pathlib import Path
import hashlib
import json
import struct
import zlib

out = Path(__file__).resolve().parent
lane = Path('<aaemu-src>/rust/crates/aaemu-game/src/packets/g2c.rs')
native = Path('<client12-unpacked>/x2game_unpacked.dll')


def frame(opcode, body):
    return b'\xdd\x01\x00\x00' + struct.pack('<H', opcode) + body


payment = frame(0x1c2, struct.pack('<iiqqq', 1, 1, 0, 1893456000, 0))
premium = frame(0x201, struct.pack('<iBB', 1, 1, 1))
slots = frame(0x1e2, b'\x02')
assert payment == bytes.fromhex('dd010000c2010100000001000000000000000000000080d8db70000000000000000000000000')
assert premium == bytes.fromhex('dd0100000102010000000101')
assert slots == bytes.fromhex('dd010000e20102')

# Include packet-like bytes in the previously unread i64; it must be one field.
varied = frame(0x1c2, struct.pack('<iiqqq', -7, 0x7fffffff, -2**63, 2**63-1, 0x1122334401e20000))
raw_batch = b''.join(b'\x00\x00' + part[4:] for part in [varied, slots, premium, frame(0x0b4, b'\x01')])
compressor = zlib.compressobj(level=9, wbits=-15)
batch = b'\xdd\x04\x04\x00' + compressor.compress(raw_batch) + compressor.flush()

files = {
    'lane-payment.frame': payment,
    'lane-premium.frame': premium,
    'lane-extra-slots.frame': slots,
    'varied-payment.frame': varied,
    'adjacent-batch.frame': batch,
}
for name, data in files.items():
    (out/name).write_bytes(data)


def record(path):
    data = path.read_bytes()
    return dict(path=str(path), bytes=len(data), sha256=hashlib.sha256(data).hexdigest())


metadata = dict(
    description='Constructed offline serializer fixtures, not native/lane traffic captures.',
    sources=[record(lane), record(native)],
    native_archives=['397aeb90: i32 payMethod, i32 payLocation, dates payStart/payEnd, i64 realPayTime',
                     '397b8b00: i32 point, u8 oldPg, u8 pg'],
    zlib=zlib.ZLIB_VERSION,
    fixtures=[dict(name=name, bytes=len(data), sha256=hashlib.sha256(data).hexdigest()) for name, data in files.items()],
)
# Retained original archive outputs are generated separately by the isolated oracle.
if (out/'native.tsv').exists():
    metadata['native_golden'] = record(out/'native.tsv')
    evidence = Path('data/extract/aa-net/lobby-header-20261025/native-header-oracle.json')
    if evidence.exists():
        metadata['native_execution'] = dict(
            qualification='8 read/write executions; scalar/date archive leaves modeled, not concrete CryNetwork primitives or captured traffic.',
            evidence=record(evidence),
            oracle=record(evidence.parent/'native_header_oracle.py'),
            function_ranges=json.loads(evidence.read_text(encoding='utf8'))['function_ranges'],
        )
(out/'metadata.json').write_text(json.dumps(metadata, indent=2)+'\n', encoding='utf8')
print('Generated', len(files), 'offline lobby fixtures.')
