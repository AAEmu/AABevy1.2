"""Execute the original UTF-8 converter and compiler at their buffer boundaries.

Run: python -B crates/aa-midi/tests/native_limits_oracle.py
Requires the same read-only DLLs and pefile/unicorn/capstone as native_oracle.py.
No live process, server, audio, or client writes. TSV scores use hex-codepoint*count
runs to keep long whitespace and supplementary-character fixtures reviewable.
"""
import hashlib
from itertools import groupby
import json
from pathlib import Path
import struct

from capstone import Cs, CS_ARCH_X86, CS_MODE_32
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_EIP, UC_X86_REG_ESP

import native_oracle as compiler

COMMON = Path('<client12-unpacked>/bin/xlcommon.dll')
GAME_SHA = '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
COMMON_SHA = '0e0881aa837553d7e307a5c6f9d886f3e82a0d72d94b6d666448aedaa94e2203'
HERE = Path(__file__).parent


def convert_native(text, pe, image):
    """Actual XlMultiToWide3301a100; no substituted character conversion."""
    uc = Uc(UC_ARCH_X86, UC_MODE_32)
    base = pe.OPTIONAL_HEADER.ImageBase
    uc.mem_map(base, (len(image) + 4095) & ~4095)
    uc.mem_write(base, image)
    uc.mem_map(0x10000000, 0x100000)
    source, output, stack, stop = 0x10000000, 0x10080000, 0x100ff000, 0x100ffff0
    uc.mem_write(source, text.encode('utf-8') + b'\0' * 8)
    uc.mem_write(output, b'\xa5' * 12016)
    uc.mem_write(stack, struct.pack('<4I', stop, output, 3001, source))
    uc.reg_write(UC_X86_REG_ESP, stack)
    uc.emu_start(0x3301a100, stop, count=200000)
    assert uc.reg_read(UC_X86_REG_EIP) == stop
    values = struct.unpack('<3001I', uc.mem_read(output, 12004))
    count = values.index(0)
    assert uc.reg_read(UC_X86_REG_EAX) == count + 1
    assert bytes(uc.mem_read(output + 12004, 12)) == b'\xa5' * 12
    converted = ''.join(chr(c) for c in values[:count])
    expected = text.split('\0', 1)[0].removeprefix('\ufeff')[:3000]
    assert converted == expected
    return converted


def cases():
    three = ','.join(note + ' ' * 998 for note in 'CDE')
    supplementary = ','.join(note + '\U0001f600' * 998 for note in 'CDE') + ','
    return [
        ('1000_single', 'C' + ' ' * 999, 'Native'),
        ('1001_two', 'C' + ' ' * 998 + ',D', 'Native'),
        ('1001_spaced_number', 'C1' + ' ' * 996 + '2,D', 'Native'),
        ('long_code_error', 'C' + ' ' * 998 + ',O11 D', 'Native'),
        ('1000_final_second', 'C,D' + ' ' * 999, 'Native'),
        ('2000_two', 'C' + ' ' * 998 + ',D' + ' ' * 999, 'Native'),
        ('2999_three', three, 'Native'),
        ('3000_third_comma', three + ',', 'Native'),
        ('3001_truncated', three + ',D', 'Native'),
        ('12000_truncated', three + ',' + 'D' * 9000, 'Native'),
        ('3000_after_bom', '\ufeff' + three + ',', 'Native'),
        ('3000_supplementary', supplementary, 'Native'),
        ('3000_supplementary_bom', '\ufeff' + supplementary, 'Native'),
        ('nul_before_large_suffix', 'C\0' + '\U0001f600' * 4000, 'Native'),
        ('bom_at_nul', '\ufeffC\0D', 'Native'),
        ('one_bom_only', '\ufeff\ufeffC' + ' ' * 998, 'Native'),
        ('empty_still_stops_tracks', 'C,,E' + ' ' * 998, 'Native'),
        ('fourth_track_ignored', 'C,D,E,' + 'C' * 4000, 'Native'),
        # The splitter has NO per-track guard. These execute original writes and
        # retain the resulting aliasing/overflow, but the Rust backend rejects it.
        ('1001_single_overflow', 'C' + ' ' * 1000, 'TooLong'),
        ('3000_single_overflow', 'C' + ' ' * 2999, 'TooLong'),
        ('1000_first_alias', 'C' + ' ' * 999 + ',D', 'TooLong'),
        ('1000_second_alias', 'C,D' + ' ' * 999 + ',E', 'TooLong'),
        ('1000_third_pointer_overwrite', 'C,D,E' + ' ' * 999, 'TooLong'),
        ('1001_second_overflow', 'C,D' + ' ' * 1000, 'TooLong'),
        ('1001_supplementary_overflow', 'C' + '\U0001f600' * 1000, 'TooLong'),
        ('overflow_after_empty_track', 'C,,' + 'E' + ' ' * 1000, 'TooLong'),
        ('1000_before_empty_track', 'C' + ' ' * 999 + ',,E', 'Native'),
    ]


def main():
    assert hashlib.sha256(compiler.DLL.read_bytes()).hexdigest() == GAME_SHA
    data = COMMON.read_bytes()
    assert hashlib.sha256(data).hexdigest() == COMMON_SHA
    pe = pefile.PE(data=data)
    image = pe.get_memory_mapped_image()
    lines = ['# name | converter output as hex-codepoint*count runs | converted count | native result | backend result']
    rows = []
    for name, text, policy in cases():
        converted = convert_native(text, pe, image)
        trace = {}
        result = compiler.compile_native(converted, trace=trace) or 'Failure'
        recipe = ' '.join(f'{ord(c):04x}*{len(list(run))}' for c, run in groupby(converted))
        lines.append('\t'.join([name, recipe, str(len(converted)), result, policy]))
        rows.append({'case': name, 'input_codepoints': len(text), 'input_bytes': len(text.encode()),
                     'input_runs': ' '.join(f'{ord(c):04x}*{len(list(run))}' for c, run in groupby(text)),
                     'converted_codepoints': len(converted), 'native_result': result,
                     'backend_policy': policy, 'splitter': trace})
    (HERE / 'native_limits.tsv').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    (HERE / 'native_limits.json').write_text(json.dumps({
        'game_sha256': GAME_SHA, 'xlcommon_sha256': COMMON_SHA,
        'executed': ['3301a100 converter (capacity 3001)', '390838a0 compiler', '39081d90 splitter'],
        'limitations': 'Compiler CRT/allocator/ASCII helpers/UI stubs as in native_oracle.py; no live lifecycle or audio proof.',
        'cases': rows}, indent=2) + '\n', encoding='utf-8')
    disassembler = Cs(CS_ARCH_X86, CS_MODE_32)
    source = []
    for name, sha, base, mapped, ranges in [
        ('x2game_unpacked.dll', GAME_SHA, compiler.PE.OPTIONAL_HEADER.ImageBase, compiler.IMAGE,
         [(0x39081d90, 84), (0x390838a0, 189), (0x39083480, 522), (0x39083b10, 0x50)]),
        ('xlcommon.dll', COMMON_SHA, pe.OPTIONAL_HEADER.ImageBase, image, [(0x3301a100, 199)])]:
        source.append(f'IMAGE {name} SHA256 {sha}')
        for address, length in ranges:
            source.append(f'\nSOURCE {address:08x}')
            source.extend(f'{i.address:08x}: {i.mnemonic} {i.op_str}'.rstrip()
                          for i in disassembler.disasm(mapped[address-base:address-base+length], address))
    (HERE / 'native_limits_source.txt').write_text('\n'.join(source) + '\n', encoding='utf-8')
    print(f'wrote {len(rows)} executed converter/compiler boundary cases')


if __name__ == '__main__':
    main()
