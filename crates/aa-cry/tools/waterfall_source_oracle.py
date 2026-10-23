"""Execute shipped 1.2 x86 TexMod update; compare type-2 coordinates at fixed times.

Requires pefile, unicorn, numpy. Uses the prior read-only extracted renderer DLL,
or --dll PATH. Assets/shaders are read only through tools/aa-pak.exe. No native
process is attached. --check compares the retained golden against this execution.
"""
from pathlib import Path
import argparse
import hashlib
import json
import struct
import subprocess
import xml.etree.ElementTree as ET
import numpy as np
import pefile
import unicorn as uc
from unicorn.x86_const import (UC_X86_REG_ESP, UC_X86_REG_ECX, UC_X86_REG_EDX,
                              UC_X86_REG_FPCW, UC_X86_REG_MXCSR)

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / 'data/extract/aa-cry/waterfall'
GOLDEN = Path(__file__).with_name('waterfall_evidence') / 'native_oracle.json'
FUNCTION = 0x3822BEB0
SIZE = 14267
DLL_SHA = 'ebf051bba5d349a6b3bf6d4eb676908115b73bb22d35d5a0cc917c325426589d'


def read_pak(path):
    data = subprocess.run([str(ROOT / 'tools/aa-pak.exe'), 'cat', path],
                          check=True, capture_output=True).stdout
    dest = OUT / path
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(data)
    return data


class Native:
    def __init__(self, path):
        self.raw = path.read_bytes()
        assert hashlib.sha256(self.raw).hexdigest() == DLL_SHA, 'unexpected renderer version'
        pe = pefile.PE(data=self.raw)
        self.base = pe.OPTIONAL_HEADER.ImageBase
        self.data = pe.get_memory_mapped_image()

    def matrix(self, time, tile, offset, types, rate, amplitude, phase):
        cpu = uc.Uc(uc.UC_ARCH_X86, uc.UC_MODE_32)
        cpu.mem_map(self.base, (len(self.data) + 4095) & ~4095)
        cpu.mem_write(self.base, self.data)
        for address in [0x60000000, 0x61000000, 0x62000000, 0x70000000, 0x71000000]:
            cpu.mem_map(address, 0x10000)

        def write(address, fmt, *values):
            cpu.mem_write(address, struct.pack(fmt, *values))

        # Original globals point at a zeroed mock renderer; explicit positive a4
        # avoids runtime clock lookup/cache. No called instruction is replaced.
        write(0x383B17C4, '<I', 0x60000000)
        write(0x61000000 + 52, '<I', 0x62000000)
        for offset_bytes, fmt, values in [(4, '<2B', types), (8, '<2f', tile),
                                          (20, '<2f', offset), (88, '<2f', rate),
                                          (96, '<2f', amplitude), (104, '<2f', phase)]:
            write(0x62000000 + offset_bytes, fmt, *values)
        stack = 0x70008000
        write(stack, '<IIf', 0x71000000, 0, time)
        for register, value in [(UC_X86_REG_ESP, stack), (UC_X86_REG_ECX, 0x61000000),
                                (UC_X86_REG_EDX, 0), (UC_X86_REG_FPCW, 0x37F),
                                (UC_X86_REG_MXCSR, 0x1F80)]:
            cpu.reg_write(register, value)
        cpu.emu_start(FUNCTION, 0x71000000, count=100000)
        return np.array(struct.unpack('<16f', cpu.mem_read(0x62000000 + 112, 64))).reshape(4, 4)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dll', type=Path, default=ROOT / 'data/extract/aa-cry/ida/render/cryrenderd3d10.dll')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    native = Native(args.dll)
    provenance = {}
    for name in ['illum.cfx', 'vertexlib.cfi', 'fraglib.cfi', 'fxsamplerdefs.cfi', 'modificatortc.cfi']:
        path = 'game/shaders/hwscripts/cryfx/' + name
        provenance[path] = hashlib.sha256(read_pak(path)).hexdigest()
    cases = []
    for path, name in [
        ('game/objects/effects/waterfall/waterfall_size_15.mtl', ''),
        ('game/objects/env/02_harihara/01_ma/ma_waterfall_a.mtl', 'water'),
        ('game/objects/env/05_nature/08_cry_source/ice/waterfall/waterfall_second.mtl', ''),
    ]:
        raw = read_pak(path)
        provenance[path] = hashlib.sha256(raw).hexdigest()
        material = next(m for m in ET.fromstring(raw).iter('Material') if m.get('Name', '') == name)
        tm = material.find("Textures/Texture[@Map='Diffuse']/TexMod")
        attrs = tm.attrib
        pair = lambda suffix: tuple(float(attrs.get('TexMod_' + axis + suffix, 0)) for axis in 'UV')
        cases.append(dict(path=path, tile=tuple(float(attrs.get('Tile' + a, 1)) for a in 'UV'),
                          offset=tuple(float(attrs.get('Offset' + a, 0)) for a in 'UV'),
                          types=tuple(int(x) for x in pair('OscillatorType')), rate=pair('OscillatorRate'),
                          amplitude=pair('OscillatorAmplitude'), phase=pair('OscillatorPhase')))
    cases.append(dict(path='two-axis signed/phase control', tile=(2., 3.), offset=(.1, .2),
                      types=(2, 2), rate=(.1, -.25), amplitude=(.5, -.7), phase=(3., 7.)))
    rows = []
    error = 0.
    for case in cases:
        values = {k: v for k, v in case.items() if k != 'path'}
        for time in [0., .5, 1., 2., 4., 3599.75, 3600.25, 3601., 7201.]:
            matrix = native.matrix(time, **values)
            rate, amplitude, tile, offset = (np.array(case[k], dtype=np.float32)
                                              for k in ['rate', 'amplitude', 'tile', 'offset'])
            amplitude = np.where(np.array(case['types']) == 2, amplitude, np.float32(0))
            expected_offset = (amplitude * (rate * np.float32(time))) * tile + offset
            err = float(np.max(np.abs(matrix[3, :2] - expected_offset)))
            error = max(error, err)
            assert err <= .0002, (case, time, matrix, expected_offset)
            assert np.array_equal(matrix[[0, 1], [0, 1]], tile)
            rows.append(dict(path=case['path'], seconds=time, translation=matrix[3, :2].tolist()))
    result = dict(renderer_sha256=DLL_SHA, function='0x3822BEB0', function_size=SIZE,
                  function_sha256=hashlib.sha256(native.data[FUNCTION-native.base:FUNCTION-native.base+SIZE]).hexdigest(),
                  inputs='seconds passed explicitly as native a4; default coordinate generation, zero rotation, no tile animation',
                  clock='native negative-a4 fallback reads current render-thread seconds; Bevy clock uses unwrapped elapsed_secs, not globals.time',
                  limits='matrix execution only; no native GPU draw, deformation, dual pass or perturbation validation',
                  sources=provenance, cases=cases, samples=rows, max_translation_error=error)
    result = json.loads(json.dumps(result))
    if args.check:
        assert result == json.loads(GOLDEN.read_text()), 'native evidence changed'
    else:
        GOLDEN.parent.mkdir(exist_ok=True)
        GOLDEN.write_text(json.dumps(result, indent=2) + '\n')
    print(f'{len(rows)} original x86 matrices pass; max translation error {error}')


if __name__ == '__main__':
    main()
