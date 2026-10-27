"""Targeted read-only disassembly of the pinned native 1.2 image.

Writes only when explicitly passed --out (within this ignored directory).
No live process, device, game execution, network, Cargo, or original-file writes.
"""
from pathlib import Path
import hashlib
import struct
import sys
import pefile
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

OUT = Path(__file__).resolve().parent
GAME = Path('<client12-unpacked>/x2game_unpacked.dll')
SHA = '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
if '--entity' in sys.argv:
    sys.argv.remove('--entity')
    GAME = Path('<client>/bin32/cryentitysystem.dll')
    SHA = 'd1f0623a45c491888b9a2f3de54270d142d17afe2a63dd4c4ae95a3ea2a7377e'
data = GAME.read_bytes()
assert hashlib.sha256(data).hexdigest() == SHA
pe = pefile.PE(data=data)
BASE = pe.OPTIONAL_HEADER.ImageBase
image = pe.get_memory_mapped_image()
cs = Cs(CS_ARCH_X86, CS_MODE_32)


def dis(a, b):
    return '\n'.join('%08x: %s %s' % (i.address, i.mnemonic, i.op_str)
                     for i in cs.disasm(image[a-BASE:b-BASE], a))


def refs(target):
    rows = []
    for sec in pe.sections:
        if not sec.Characteristics & 0x20000000:
            continue
        begin = sec.VirtualAddress
        end = begin + sec.Misc_VirtualSize
        for op in (b'\xe8', b'\xe9'):
            at = begin
            while (at := image.find(op, at, end)) >= 0:
                disp = struct.unpack_from('<i', image, at+1)[0]
                if BASE+at+5+disp == target:
                    rows.append((BASE+at, 'call' if op == b'\xe8' else 'jmp'))
                at += 1
    needle = struct.pack('<I', target)
    at = 0
    while (at := image.find(needle, at)) >= 0:
        rows.append((BASE+at, 'absolute value (unclassified)'))
        at += 1
    return '\n'.join('%08x: %s -> %08x' % (*x, target) for x in sorted(rows))


def main():
    args = sys.argv[1:]
    dest = None
    if '--out' in args:
        at = args.index('--out')
        dest = (OUT / args[at+1]).resolve()
        assert dest.parent == OUT
        del args[at:at+2]
    chunks = [str(GAME)+' SHA256 '+SHA]
    for arg in args:
        if ':' in arg:
            a, b = (int(x, 16) for x in arg.split(':'))
            raw = image[a-BASE:b-BASE]
            chunks.extend(['\nSOURCE %08x:%08x SHA256 %s' % (a,b,hashlib.sha256(raw).hexdigest()), dis(a,b)])
        else:
            a = int(arg,16)
            chunks.extend(['\nXREF %08x (raw rel32 + absolute candidates)' % a, refs(a)])
    result = '\n'.join(chunks)+'\n'
    if dest:
        dest.write_text(result, encoding='utf-8')
        print(str(dest), len(result), 'bytes')
    else:
        print(result)


if __name__ == '__main__':
    main()
