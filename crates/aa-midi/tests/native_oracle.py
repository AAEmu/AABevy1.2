"""Run the original x86 MML compiler in Unicorn; no game process or DLL loading.

Requires the RE machine's pefile/unicorn packages. Only runtime allocation, CRT,
ASCII character helpers, and the error-message UI are stubbed. Compiler/tokenizer,
track splitting, float timing and MIDI byte writer execute original instructions.
Usage: python crates/aa-midi/tests/native_oracle.py [score]
"""
import hashlib
import json
from pathlib import Path
import struct
import sys

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import *

DLL = Path('D:/aa-re/client12/x2game_unpacked.dll')
PE = pefile.PE(str(DLL))
IMAGE = PE.get_memory_mapped_image()


def compile_native(text, program=73, channel=0, quality=1.0, seed=1, octave=5):
    pe = PE
    uc = Uc(UC_ARCH_X86, UC_MODE_32)
    base = pe.OPTIONAL_HEADER.ImageBase
    uc.mem_map(base, (pe.OPTIONAL_HEADER.SizeOfImage + 4095) & ~4095)
    uc.mem_write(base, IMAGE)
    uc.mem_map(0, 0x10000)  # SEH chain (FS:0)
    uc.mem_map(0x10000000, 0x1000000)  # object, input, output and heap
    uc.mem_map(0x20000000, 0x100000)  # stack
    uc.mem_map(0x60000000, 0x10000)  # import stubs and return sentinel
    uc.mem_map(0x6320e000, 0x1000)  # unpacked direct CRT rand call
    obj, source, output = 0x10000000, 0x10010000, 0x10020000
    heap = 0x10100000
    rng = seed
    error = None

    def read(at):
        return struct.unpack('<I', uc.mem_read(at, 4))[0]

    def write(at, value):
        uc.mem_write(at, struct.pack('<I', value & 0xffffffff))

    def ret(value=0, pop=0):
        esp = uc.reg_read(UC_X86_REG_ESP)
        uc.reg_write(UC_X86_REG_EAX, value & 0xffffffff)
        uc.reg_write(UC_X86_REG_EIP, read(esp))
        uc.reg_write(UC_X86_REG_ESP, esp + 4 + pop)

    imports = {}
    for entry in pe.DIRECTORY_ENTRY_IMPORT:
        for imp in entry.imports:
            addr = 0x60000000 + 16 * (len(imports) + 1)
            imports[addr] = imp.name.decode() if imp.name else str(imp.ordinal)
            write(imp.address, addr)

    def hook(_, address, size, data):
        nonlocal heap, rng, error
        esp = uc.reg_read(UC_X86_REG_ESP)
        arg = lambda i: read(esp + 4 + i * 4)
        if address == 0x6000fff0:
            uc.emu_stop()
        elif address == 0x3908390e:
            # Equivalent to the equipment override in FUN_390828b0; no player exists here.
            write(obj + 0xde88, octave)
        elif address == 0x39082dc0:  # vector push_back (plain 20-byte token)
            vec = uc.reg_read(UC_X86_REG_ECX)
            start, end, cap = (read(vec + n * 4) for n in range(3))
            if not start:
                start = end = heap
                heap += 0x10000
                write(vec, start)
                write(vec + 8, start + 0x10000)
            if end + 20 > start + 0x10000:
                raise RuntimeError('oracle token capacity')
            uc.mem_write(end, bytes(uc.mem_read(arg(0), 20)))
            write(vec + 4, end + 20)
            ret(pop=4)
        elif address == 0x3950a1f0:  # deallocation
            ret()
        elif address == 0x392e3420:  # code-error UI, fail before calling the UI
            error = 'Code'
            uc.emu_stop()
        elif address == 0x6320ed92:
            rng = (rng * 214013 + 2531011) & 0xffffffff
            ret((rng >> 16) & 32767)
        elif address in imports:
            name = imports[address]
            if 'XlIsSpace@@YA_NI' in name:
                ret(int(chr(arg(0)) in ' \t\r\n\v\f'))
            elif 'XlToUpper@@YAII' in name:
                c = arg(0)
                ret(c - 32 if 97 <= c <= 122 else c)
            elif 'XlIsDigit@@YA_NI' in name:
                ret(int(48 <= arg(0) <= 57))
            elif name == 'memset':
                uc.mem_write(arg(0), bytes([arg(1) & 255]) * arg(2))
                ret(arg(0))
            elif name == '_time64':
                ret(seed)
                uc.reg_write(UC_X86_REG_EDX, 0)
            elif name == 'srand':
                rng = arg(0)
                ret()
            elif '_Orphan_all@' in name:
                ret()
            elif name == 'rand':
                rng = (rng * 214013 + 2531011) & 0xffffffff
                ret((rng >> 16) & 32767)
            else:
                raise RuntimeError(f'unstubbed import {name} at {address:x}')

    uc.hook_add(UC_HOOK_CODE, hook)
    write(0x39ae4674, obj)
    write(0x39b02294, 0)  # no player override of instrument
    write(obj + 0xdea0, 0x10050000)
    write(obj + 0xdea4, 0x10050000)
    write(obj + 0xdea8, 0x10060000)
    codepoints = [ord(c) for c in text.split('\0')[0]] + [0]
    uc.mem_write(source, struct.pack('<' + 'I' * len(codepoints), *codepoints))
    stack = 0x200ff000
    args = [0x6000fff0, output, 0x8000, source, program, channel,
            struct.unpack('<I', struct.pack('<f', quality))[0]]
    uc.mem_write(stack, struct.pack('<7I', *args))
    uc.reg_write(UC_X86_REG_ESP, stack)
    try:
        uc.emu_start(0x390838a0, 0x6000fff0, count=2000000)
    except Exception as exc:
        raise RuntimeError(f'{exc}; eip={uc.reg_read(UC_X86_REG_EIP):08x}') from exc
    if error:
        return error
    if uc.reg_read(UC_X86_REG_EIP) != 0x6000fff0:
        raise RuntimeError('native instruction budget exhausted')
    length = uc.reg_read(UC_X86_REG_EAX)
    if length > 0x8000:
        return 'TooLong'
    return bytes(uc.mem_read(output, length)).hex()


if __name__ == '__main__':
    if sys.argv[1:] == ['--fixtures']:
        cases = [
            'C', '', ',', ',C', 'C,', 'C,,E', 'C, ,E', 'C,D,E,F', 'R4',
            'N60', 'O C', 'C65', 'O11 C', 'V128 C', 'L0 C', 'T31 C',
            'c# d- e+ f', '도레미파솔라시', 'ドレミファソラシ', '哆来咪发梭拉西',
            'C8 R4 D16. E2', 'C.. D...', 'R4. C', 'L8.. C D4.',
            'O4 V80 L8 T240 C', 'T137 L7 C R7 D', 'C&C', 'C&D&E',
            'C&C&D&D&C', 'C&C&', 'C&R4 D', 'C&N60C', 'C&T60C',
            'O3 V25 L7 T99 C, C', 'C R1, D', 'N60C', 'C4& C4',
            'C. &C', 'C!?x D', 'C4#8 D', 'O10 B+ >C O0 C- <C',
            'C\\0D'.replace('\\0', '\0'), 'C4294967297', 'L4294967300 C', 'C.5D', 'C4&8D', 'ﬀC',
        ]
        lines = ['# score (NUL escaped as \\0) | quality | seed | program | channel | octave | native hex, Code, or Failure']
        for quality in [1.0, 0.5]:
            for score in (cases if quality == 1.0 else ['CDEFGABC', 'CDE,CDE', 'C&D&E&F', 'C&C&D&E']):
                result = compile_native(score, quality=quality, seed=7) or 'Failure'
                lines.append(f'{score.replace(chr(0), chr(92)+"0")}\t{quality}\t7\t73\t0\t5\t{result}')
        for score, program, channel, octave in [('C E F+ C', 35, 9, 3), ('CDE', 24, 0, 5), ('C', 106, 0, 5)]:
            result = compile_native(score, program, channel, octave=octave)
            lines.append(f'{score}\t1\t1\t{program}\t{channel}\t{octave}\t{result}')
        Path(__file__).with_name('native.tsv').write_text('\n'.join(lines) + '\n', encoding='utf-8')
        print(f'wrote {len(lines)-1} cases; DLL sha256: {hashlib.sha256(DLL.read_bytes()).hexdigest()}')
        sys.exit()
    print('DLL sha256:', hashlib.sha256(DLL.read_bytes()).hexdigest())
    for score in sys.argv[1:] or ['C', '', 'C,,E', 'C8 R4 D16. E2', 'C&C', 'O C', 'C65']:
        print(json.dumps({'score': score, 'result': compile_native(score)}, ensure_ascii=True))
