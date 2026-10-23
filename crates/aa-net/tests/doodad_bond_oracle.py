"""Execute original bond archives; retain goldens and actual read-only lane methods.

Run from repo root: python -B crates/aa-net/tests/doodad_bond_oracle.py
Requires pefile, capstone, unicorn. No live process, server or socket is accessed.
Only primitive archive calls are substituted; packet/bond archive code executes.
"""
from pathlib import Path
import hashlib
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'tools'))
from user_music_lifecycle_oracle import Native, RUNTIME_DATA, RUNTIME_SHA, SHA
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

OUT = Path(__file__).parent / 'fixtures/doodad-bond'
OUT.mkdir(exist_ok=True)
n = Native()
packet, archive, vt = 0x10080000, 0x10081000, 0x10082000
n.put(archive, vt)
wire = bytearray()
incoming = b''
reading = False
at = 0


def primitive(off):
    global at
    if off == 0x1c:
        n.ret(int(reading))
        return
    ptr = n.arg(1)
    size = n.arg(2) if off in (0xcc, 0xd0) else {0x44: 1, 0x3c: 4, 0x4c: 4}[off]
    if reading:
        assert at + size <= len(incoming)
        n.u.mem_write(ptr, incoming[at:at+size])
        at += size
    else:
        wire.extend(n.u.mem_read(ptr, size))
    n.ret(pop=12)


for off in (0x1c, 0x44, 0x3c, 0x4c, 0xcc, 0xd0):
    address = 0x30000100 + off
    n.put(vt + off, address)
    n.hooks[address] = lambda off=off: primitive(off)

runtime = RUNTIME_DATA.read_bytes()
assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
sentinel = struct.unpack_from('<I', runtime, 0x3A5E128C - 0x39AC3000)[0]
n.put(0x3A5E128C, sentinel)
rows = []
# Space and spot are deliberately independent and signed, not seat-count guesses.
for actor, character, owner, point, kind, space, spot in [
    (0x010203, 0x11223344, 0x0a0b0c, 2, 2, 1, 0),
    (0xffffff, 0xffffffff, 0xffffff, 254, 255, -2147483648, 2147483647),
    (1, 1, 2, 0, 0, -1, -2),
]:
    for label, opcode, address in [('bond', 0x139, 0x397b5ab0), ('unbond', 0x13a, 0x397b5b10), ('request', 0xcd, 0x397bb470)]:
        n.u.mem_write(packet, bytes(128))
        if label == 'bond':
            n.put(packet + 12, actor, owner, spot, space, point, kind)
            expected = actor.to_bytes(3, 'little') + bytes([point]) + owner.to_bytes(3, 'little') + bytes([kind]) + struct.pack('<ii', space, spot)
        elif label == 'unbond':
            n.put(packet + 12, actor, character, owner)
            expected = actor.to_bytes(3, 'little') + struct.pack('<I', character) + owner.to_bytes(3, 'little')
        else:
            n.call(0x397ac640, packet, actor, owner)
            expected = actor.to_bytes(3, 'little') + owner.to_bytes(3, 'little')
        reading = False
        wire.clear()
        n.call(address, packet, archive)
        assert wire == expected, (label, wire.hex(), expected.hex())
        incoming = bytes(wire)
        reading, at = True, 0
        n.u.mem_write(packet, bytes(128))
        n.call(address, packet, archive)
        assert at == len(incoming)
        if label == 'bond':
            assert [n.get(packet + i) for i in (12, 16, 20, 24, 28, 32)] == [actor, owner, spot & 0xffffffff, space & 0xffffffff, point, kind]
        else:
            assert n.get(packet+12) == actor
            assert n.get(packet+16) == (character if label == 'unbond' else owner)
            if label == 'unbond': assert n.get(packet+20) == owner
        rows.append(f'{label}\t{opcode:x}\t{actor}\t{character}\t{owner}\t{point}\t{kind}\t{space}\t{spot}\t{incoming.hex()}')
# Native's empty SC139 is only actor BC3 + ff; it is not an unbond acknowledgement.
reading = False
n.put(packet + 12, 0x010203, sentinel, 0xffffffff, 0, 0, 1)
wire.clear()
n.call(0x397b5ab0, packet, archive)
assert wire == bytes.fromhex('030201ff')
rows.append('absent\t139\t66051\t0\t0\t255\t0\t0\t0\t' + wire.hex())
incoming, reading, at = bytes(wire), True, 0
n.u.mem_write(packet, bytes(128))
n.call(0x397b5ab0, packet, archive)
assert at == 4 and n.get(packet+16) == sentinel
(OUT / 'native.tsv').write_text('\n'.join(rows) + '\n', encoding='utf8')
cs = Cs(CS_ARCH_X86, CS_MODE_32)
source = [f'x2game_unpacked.dll SHA256 {SHA}', f'Runtime constants SHA256 {RUNTIME_SHA}; empty owner {sentinel:08x}', 'Archive primitives only are mocked; native packet/bond instructions execute in read and write modes.']
for address, length in [(0x391b0e10, 0xc4), (0x391b0ee0, 0x7b), (0x397b5ab0, 0x5d),
                        (0x397b5b10, 0xb0), (0x3983bf10, 0x185), (0x397b25d0, 0x70),
                        (0x397ac640, 0x13), (0x397bb470, 0xa0), (0x393566f0, 0xb0),
                        (0x391db770, 0x73), (0x391db7f0, 0x77), (0x393b2b90, 0x7e),
                        (0x393566c0, 0x27), (0x3935a520, 0xe2)]:
    raw = bytes(n.u.mem_read(address, length))
    source.append(f'\nSOURCE {address:08x} SHA256 {hashlib.sha256(raw).hexdigest()}')
    source.extend(f'{i.address:08x}: {i.mnemonic} {i.op_str}'.rstrip() for i in cs.disasm(raw, address))
(OUT / 'native-source.txt').write_text('\n'.join(source) + '\n', encoding='utf8')


def block(text, marker):
    start = text.index(marker)
    at = text.index('{', start) + 1
    level = 1
    while level:
        level += (text[at] == '{') - (text[at] == '}')
        at += 1
    return text[start:at]


root = Path('D:/aaemu-wt/bevy/rust/crates')
paths = [root / 'aaemu-game/src/models/doodad.rs', root / 'aaemu-game/src/systems/npcs/doodad/packets.rs', root / 'aaemu-commons/src/packet_stream.rs']
model, packets, stream = [p.read_text(encoding='utf8') for p in paths]
lane = ['// Actual read-only lane methods; generated by doodad_bond_oracle.py.', '#![allow(dead_code)]',
        'use crate::music::tests::lane::{PacketStream, Marshal, MarshalError};', 'type Result<T> = std::result::Result<T, MarshalError>;']
lane += [f'// {p.as_posix()} SHA256 {hashlib.sha256(p.read_bytes()).hexdigest()}' for p in paths]
lane += ['#[derive(Debug, Clone, Copy, PartialEq, Eq)]', block(model, 'pub struct BondDoodad'), block(model, 'impl BondDoodad')]
for name in ['SCBondDoodadPacket', 'SCUnbondDoodadPacket']:
    lane += ['#[derive(Debug, Clone, Copy, PartialEq, Eq)]', block(packets, f'pub struct {name}'), block(packets, f'impl Marshal for {name}')]
lane += ['#[derive(Debug, Clone, Copy, PartialEq, Eq)]', block(packets, 'pub struct CSUnbondDoodadPacket'), 'impl CSUnbondDoodadPacket {', block(packets[packets.index('impl ClientPacket for CSUnbondDoodadPacket'):], 'fn read(').replace('fn read(', 'pub fn read('), '}', 'impl PacketStream {', block(stream, 'pub fn read_bc('), '}']
(OUT / 'lane.rs').write_text('\n\n'.join(lane) + '\n', encoding='utf8')
unit_path = root / 'aaemu-game/src/world/unit_state.rs'
unit_source = unit_path.read_text(encoding='utf8')
ability_path = root / 'aaemu-game/src/models/char_data.rs'
ability_source = ability_path.read_text(encoding='utf8')
(OUT / 'lane-snapshot-source.txt').write_text(f'// {unit_path.as_posix()} SHA256 {hashlib.sha256(unit_path.read_bytes()).hexdigest()}\n' + block(unit_source, 'fn write_unit_state(') + f'\n\n// {ability_path.as_posix()} SHA256 {hashlib.sha256(ability_path.read_bytes()).hexdigest()}\n' + block(ability_source, 'impl Default for CharacterAbilities') + '\n', encoding='utf8')
print(f'{len(rows)} native archive goldens; 20 original read/write executions; actual lane writers/reader and character snapshot source retained')
