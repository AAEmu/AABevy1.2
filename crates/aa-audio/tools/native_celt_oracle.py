"""Execute original 1.2 CELT instructions in bounded x86 Unicorn, with no DLL load/device.

Usage: python -B native_celt_oracle.py <fmodexl.dll> <bank055.fsb> --out-dir <ignored-dir>
Requires pefile and unicorn. Inputs must be extracted through aa-pak. No PCM or
original binary is distributed by this script; all output goes to the given directory.
"""

import argparse
import hashlib
import json
from pathlib import Path
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP,
    UC_X86_REG_FPCW, UC_X86_REG_MXCSR,
)

from native_playback_oracle import CORE_HASH


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("core_dll", type=Path)
    parser.add_argument("bank", type=Path)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    dll, bank = args.core_dll.read_bytes(), args.bank.read_bytes()
    assert hashlib.sha256(dll).hexdigest() == CORE_HASH, "unexpected original DLL"
    assert hashlib.sha256(bank).hexdigest() == "aa4e9c8e86620614f924f26897270b4a7462e8a1e4bbdd876bb09aaa7e59a6ae", "unexpected original CELT bank"
    pe = pefile.PE(data=dll)
    data = pe.get_memory_mapped_image()
    cpu = Uc(UC_ARCH_X86, UC_MODE_32)
    cpu.mem_map(0x10000000, (len(data) + 4095) & ~4095)
    cpu.mem_write(0x10000000, data)
    cpu.mem_map(0x20000000, 0x100000)
    cpu.mem_map(0x30000000, 0x100000)
    cpu.mem_map(0x40000000, 4096)
    cpu.reg_write(UC_X86_REG_FPCW, 0x27F)
    cpu.reg_write(UC_X86_REG_MXCSR, 0x1F80)

    def put(address, *values):
        cpu.mem_write(address, struct.pack("<" + "I" * len(values), *values))

    def get(address):
        return struct.unpack("<I", cpu.mem_read(address, 4))[0]

    def call(address, *values):
        stack = 0x300F0000
        put(stack, 0x40000000, *values)
        cpu.reg_write(UC_X86_REG_ESP, stack)
        cpu.reg_write(UC_X86_REG_ECX, 0)
        cpu.emu_start(address, 0x40000000, timeout=2_000_000, count=2_000_000)
        assert cpu.reg_read(UC_X86_REG_EIP) == 0x40000000, "bounded original call failed to return"
        return cpu.reg_read(UC_X86_REG_EAX)

    error, decoder, encoded, pcm = 0x20000000, 0x20000100, 0x20010000, 0x20020000
    mode = call(0x1003D830, 44100, 512, error)
    assert mode == 0x1010A9D0 and get(error) == 0
    state_size = call(0x1003B5E0, mode, 1)
    assert state_size == 9244
    assert call(0x1003B600, decoder, mode, 1, error) == decoder and get(error) == 0
    at, blocks, rows = 128, [], []
    for index in range(227):
        assert bank[at:at + 4] == bytes.fromhex("17c30df3")
        length = struct.unpack_from("<I", bank, at + 4)[0]
        assert length == 186 and at + 8 + length <= len(bank)
        cpu.mem_write(encoded, bank[at + 8:at + 8 + length])
        status = call(0x1003C2C0, decoder, encoded, length, pcm, 512)
        assert status == 0, (index, status)
        output = bytes(cpu.mem_read(pcm, 1024))
        samples = struct.unpack("<512h", output)
        blocks.append(output)
        rows.append(dict(packet=index, offset=at, encoded_bytes=length, status=status,
                         sha256=hashlib.sha256(output).hexdigest(),
                         peak=max(abs(s) for s in samples),
                         rms=(sum((s / 32768) ** 2 for s in samples) / 512) ** 0.5))
        at += 8 + length
    assert at == 44166 and bank[at:] == bytes(26)
    output = b"".join(blocks)
    digest = hashlib.sha256(output).hexdigest()
    assert digest == "b4ae54b5f78ab4025f7f004a28c3144e08e79d4ef9fe41651b8607772aa165f0"
    evidence = dict(core_sha256=CORE_HASH, bank_sha256=hashlib.sha256(bank).hexdigest(),
                    note="Original decoder instructions only; x87 CW=0x27f, MXCSR=0x1f80. No OS-loaded FMOD or mixer.",
                    rate=44100, channels=1, frames=116224, loop_points=[0, 116003],
                    state_size=state_size, pcm_sha256=digest, packets=rows)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "native.s16le").write_bytes(output)
    (args.out_dir / "native.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({k: v for k, v in evidence.items() if k != "packets"}, indent=2))


if __name__ == "__main__":
    main()
