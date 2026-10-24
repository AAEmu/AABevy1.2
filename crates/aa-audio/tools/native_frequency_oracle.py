"""Verify the 26 retained original 1.2 instance loader/frequency/normalization cases.

Usage: python -B native_frequency_oracle.py <original-fmod_eventl.dll> [--json <ignored-output>]
Requires the existing pefile/Unicorn harness; no DLL/device loading or synthesized mix.
All executable bytes come from the SHA-256-pinned original DLL, read in place.
File-read and Channel::setFrequency boundaries are explicit fixtures. The retained
2026-10-23 audit goldens include all inputs, native outputs, addresses and bounds;
their original JSON content is hash-pinned below with LF newline normalization.
No original DLL/audio is shipped.
"""
import argparse
from functools import partial
import hashlib
import json
import math
from pathlib import Path
import struct
from native_playback_oracle import EVENT_HASH, Native, STACK
from unicorn.x86_const import (UC_X86_REG_EAX, UC_X86_REG_EBX, UC_X86_REG_EBP,
                              UC_X86_REG_EDI, UC_X86_REG_ESI, UC_X86_REG_ESP,
                              UC_X86_REG_EIP, UC_X86_REG_FPCW, UC_X86_REG_MXCSR)

GOLDENS = Path(__file__).resolve().parent.parent / 'tests/native_frequency_evidence.json'
GOLDEN_HASH = 'a30862c8db7d14dc63f8c03385f7a7a3680c0691d08f52059264c5cdef62f942'
bits = lambda f: struct.unpack('<I', struct.pack('<f', f))[0]
f32 = lambda f: struct.unpack('<f', struct.pack('<f', f))[0]


def loader(path, sync, enable):
    n = Native(path, EVENT_HASH)
    sound, definition, stream = [n.alloc() for _ in range(3)]
    n.put(sound+20, definition)
    raw = [7, enable, bits(.25), bits(.6), bits(123), bits(.8), bits(.1), bits(.2), 2, 3]
    serialized = struct.pack('<HffII10I', 5, .125, .75, sync, 0x201, *raw)
    at, reads = 0, []
    def read():
        nonlocal at
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        dest, size, count, output = [n.get(sp+4+4*i) for i in range(4)]
        total = size*count
        assert at+total <= len(serialized)
        n.uc.mem_write(dest, serialized[at:at+total])
        reads.append(dict(file_offset=at, bytes=total, destination=dest))
        at += total
        if output: n.put(output, count)
        n.ret(16)
    n.hooks[0x100023B0] = read
    status = n.call(0x1002C790, sound, stream, 0x450000)
    assert status == 0 and at == len(serialized) == 58
    assert n.get(sound+36) == (4 | (0x20 if sync else 0))
    flags = (0x100 if enable else 0) | 0x200 | 2 | 3 << 4
    assert n.get(definition+56) == flags
    expected = {12:5, 20:bits(.125), 24:bits(.75), 52:7,
                40:raw[2], 44:raw[3], 48:raw[4], 28:raw[5], 32:raw[6], 36:raw[7], 56:flags}
    assert all(n.get(definition+o) == v for o,v in expected.items())
    sync_query = n.call(0x1002C270, sound) & 0xff
    assert sync_query == bool(sync)
    return dict(serialized_sync_word=sync, serialized_raw1=enable, sound_flags=n.get(sound+36),
                sync_query=sync_query, definition_words={str(o):n.get(definition+o) for o in sorted(expected)},
                consumed=at, reads=[dict(file_offset=r['file_offset'], bytes=r['bytes'],
                    destination=('definition+'+str(r['destination']-definition))
                    if definition <= r['destination'] < definition+512 else 'loader stack') for r in reads])


def rate(path, name, enabled=True, use_control=False, base=.25, blend=0., offset=0.,
         primary=.5, control=.75, nominal=44100., sound_pitch=1.25, layer_pitch=.5,
         sync=False):
    n = Native(path, EVENT_HASH)
    n.uc.reg_write(UC_X86_REG_FPCW, 0x27f)
    n.uc.reg_write(UC_X86_REG_MXCSR, 0x1f80)
    body, event, sound, layer, definition = [n.alloc() for _ in range(5)]
    n.put(body+4, event); n.put(sound+24, layer); n.put(sound+20, definition)
    flags = (0x100 if enabled else 0) | (0x200 if use_control else 0)
    n.put(definition+40, base, 'f'); n.put(definition+44, blend, 'f')
    n.put(definition+48, offset, 'f'); n.put(definition+56, flags)
    n.put(sound+36, 4 | (0x20 if sync else 0))
    for pointer, x in [(event+44, primary), (layer+36, control)]:
        if x is not None:
            param = n.alloc(); n.put(pointer, param); n.put(param+24, x, 'f')
    # Fixture context only: normalized parameter object slots and enclosing locals.
    frame = STACK+0xe000
    for off, value in [(-0x100, nominal), (-0x108, sound_pitch), (-0xc0, layer_pitch), (-0xcc, 1.)]:
        n.put(frame+off, value, 'f')
    n.put(sound+52, 42); n.put(sound+56, 42)
    n.put(sound+108, 1., 'f'); n.put(sound+112, -666., 'f')
    calls = []
    def frequency():
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        assert n.get(sp+4) == 42
        calls.append(n.get(sp+8, 'f')); n.ret(8)
    n.hooks[0x1003FA72] = frequency
    n.uc.reg_write(UC_X86_REG_EAX, 0)  # mixer mute query result AL=false
    for reg, value in [(UC_X86_REG_EBP,frame), (UC_X86_REG_ESP,frame-0x400),
                       (UC_X86_REG_EBX,definition), (UC_X86_REG_ESI,body), (UC_X86_REG_EDI,sound)]:
        n.uc.reg_write(reg, value)
    n.uc.emu_start(0x1001D908, 0x1001DA3F, timeout=1_000_000, count=2000)
    assert n.uc.reg_read(UC_X86_REG_EIP) == 0x1001DA3F
    assert len(calls) == 1
    b, m, add = map(f32, [base, blend, offset])
    v = control if use_control else primary
    v = b if v is None else f32(v)
    before = f32(nominal)*f32(sound_pitch)*f32(layer_pitch)
    factor = v/(b if b != 0 else f32(1e-8))*(1-m)+m
    expected = (max(0., before*factor) if enabled else before)+add
    expected = f32(expected)
    assert math.isclose(calls[0], expected, rel_tol=2e-7, abs_tol=1e-3), (name,calls,expected)
    return dict(name=name, enabled=enabled, use_control=use_control, base=base, blend=blend,
                additive_hz=offset, primary=primary, control=control, nominal_hz=nominal,
                sound_pitch=sound_pitch, layer_pitch=layer_pitch, sync_word=sync,
                native_setFrequency=calls[0], native_playback_rate=calls[0]/nominal, formula_hz=expected)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('event_dll', type=Path)
    parser.add_argument('--json', type=Path, help='optional run evidence; retained goldens are never rewritten')
    args = parser.parse_args()
    golden_bytes = GOLDENS.read_bytes()
    assert hashlib.sha256(golden_bytes.replace(b'\r\n', b'\n')).hexdigest() == GOLDEN_HASH, 'retained audit goldens changed'
    goldens = json.loads(golden_bytes)
    frequency = partial(rate, args.event_dll)
    cases = [
        frequency('disabled', enabled=False), frequency('default_primary'), frequency('layer_control', use_control=True),
        frequency('blend_to_unity', blend=.5), frequency('unity_blend', blend=1),
        frequency('missing_primary', primary=None), frequency('missing_control', use_control=True, control=None),
        frequency('zero_base', base=0), frequency('negative_rate_clamp', primary=-.5, offset=20),
        frequency('additive_hz_after_multiply', offset=123), frequency('disabled_additive_hz', enabled=False, offset=123),
        frequency('sync_does_not_enable_scaling', enabled=False, sync=True),
        frequency('sync_does_not_change_scaling', sync=True),
        frequency('normalized_parameter', primary=.25, base=.5, sound_pitch=1, layer_pitch=1),
        frequency('disabled_negative_not_clamped', enabled=False, sound_pitch=-1),
        frequency('postclamp_additive_can_be_negative', primary=-.5, offset=-20),
        frequency('missing_primary_zero_base', primary=None, base=0),
    ]
    normalized = []
    for value in [10., 20., 30., 0., 40.]:
        n = Native(args.event_dll, EVENT_HASH); p = n.alloc()
        n.put(p+44, 10., 'f'); n.put(p+48, 30., 'f')
        assert n.call(0x10026DE0, p, bits(value)) == 0
        actual = n.get(p+24, 'f'); expected = min(1., max(0., (value-10)/20))
        assert actual == expected
        normalized.append(dict(value=value, min=10., max=30., native_normalized=actual))
    result = dict(event_dll=str(args.event_dll), sha256=EVENT_HASH,
                  loader=dict(address='1002C790', cases=[loader(args.event_dll,s,e) for s,e in [(0,0),(1,0),(0,1),(1,1)]]),
                  rate=dict(slice='1001D908..1001DA3F', count_bound=2000, timeout_us=1000000,
                            fpcw='0x27f', mxcsr='0x1f80', cases=cases),
                  normalized_parameter=dict(address='10026DE0', cases=normalized),
                  limits='Original loader/math/dispatch instructions, with fixture read/setFrequency endpoints. No FMOD output, decoding, resampler or captured mix. No active original auto-frequency instance exists.')
    for section in ('sha256', 'loader', 'rate', 'normalized_parameter', 'limits'):
        assert result[section] == goldens[section], f'original audit golden mismatch: {section}'
    assert (len(result['loader']['cases']), len(cases), len(normalized)) == (4, 17, 5)
    if args.json:
        assert args.json.resolve() != GOLDENS, 'retained goldens are read-only'
        args.json.write_text(json.dumps(result,indent=2)+'\n', encoding='utf-8')
    print('All 26 original cases match the retained audit goldens:', GOLDEN_HASH)
    print('loader cases:',len(result['loader']['cases']),'; rate cases:',len(cases),'; normalization:',len(normalized))
    for c in cases: print(c['name'],c['native_setFrequency'],c['native_playback_rate'])


if __name__ == '__main__': main()
