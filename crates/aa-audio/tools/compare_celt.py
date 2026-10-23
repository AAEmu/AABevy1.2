"""Compare original-instruction and production CELT PCM, without time/gain fitting.

Usage: python -B compare_celt.py <native.s16le> <actual.s16le> --json <evidence.json>
The inputs remain local; the JSON contains hashes and numerical errors, no PCM fixture.
"""

import argparse
import collections
import hashlib
import json
import math
from pathlib import Path
import struct


def metrics(native, actual):
    error = [a - n for a, n in zip(actual, native)]
    signal = sum(n * n for n in native)
    noise = sum(e * e for e in error)
    return dict(max_error_lsb=max(abs(e) for e in error),
                rms_error_lsb=math.sqrt(noise / len(error)),
                snr_db=10 * math.log10(signal / noise) if noise and signal else None,
                exact_samples=sum(e == 0 for e in error),
                error_histogram=dict(sorted(collections.Counter(error).items())))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("native", type=Path)
    parser.add_argument("actual", type=Path)
    parser.add_argument("--json", type=Path, required=True)
    args = parser.parse_args()
    native_bytes, actual_bytes = args.native.read_bytes(), args.actual.read_bytes()
    assert len(native_bytes) == len(actual_bytes) == 232448
    assert hashlib.sha256(native_bytes).hexdigest() == "b4ae54b5f78ab4025f7f004a28c3144e08e79d4ef9fe41651b8607772aa165f0"
    native = struct.unpack("<116224h", native_bytes)
    actual = struct.unpack("<116224h", actual_bytes)
    result = dict(rate=44100, channels=1, frames=len(native), loop_points=[0, 116003],
                  native_sha256=hashlib.sha256(native_bytes).hexdigest(),
                  actual_sha256=hashlib.sha256(actual_bytes).hexdigest(),
                  native_rms=math.sqrt(sum((s / 32768) ** 2 for s in native) / len(native)),
                  actual_rms=math.sqrt(sum((s / 32768) ** 2 for s in actual) / len(actual)),
                  comparison=metrics(native, actual), packets=[])
    for index in range(227):
        a, z = index * 512, (index + 1) * 512
        result["packets"].append(dict(packet=index,
            native_sha256=hashlib.sha256(native_bytes[a * 2:z * 2]).hexdigest(),
            actual_sha256=hashlib.sha256(actual_bytes[a * 2:z * 2]).hexdigest(),
            **metrics(native[a:z], actual[a:z])))
    args.json.parent.mkdir(parents=True, exist_ok=True)
    args.json.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({k: v for k, v in result.items() if k != "packets"}, indent=2))


if __name__ == "__main__":
    main()
