"""Retain original music packet fixtures and the lane's actual reader/writer methods.

Run from repo root: python crates/aa-net/tests/music_oracle.py
Requires pefile/unicorn (RE only). Executes original x86 constructors/serializers and
xlcommon XlStringSize. Archive primitive/blob sinks, CopyString and memcpy are mocked;
the serializers choose the actual member addresses, lengths, terminators and order.
No game process, server build or server mutation. Extracted Rust methods execute in
aa-net's tests using a minimal std-only scaffold, not the server's handlers.
"""
import hashlib
from pathlib import Path
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP

OUT = Path(__file__).parent / 'fixtures/music'
OUT.mkdir(parents=True, exist_ok=True)
DLL = Path('D:/aa-re/client12/x2game_unpacked.dll')
assert hashlib.sha256(DLL.read_bytes()).hexdigest() == '851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e'
u = Uc(UC_ARCH_X86, UC_MODE_32)
pe = pefile.PE(str(DLL))
for p in [pe, pefile.PE('D:/aa-re/client12/bin/xlcommon.dll')]:
    data = p.get_memory_mapped_image()
    base = p.OPTIONAL_HEADER.ImageBase
    u.mem_map(base, (len(data) + 4095) & ~4095)
    u.mem_write(base, data)
for address, size in [(0x10000000, 0x50000), (0x20000000, 0x10000), (0x30000000, 0x10000)]:
    u.mem_map(address, size)


def read(address):
    return struct.unpack('<I', u.mem_read(address, 4))[0]


def write(address, *values):
    u.mem_write(address, struct.pack('<' + 'I' * len(values), *(v & 0xffffffff for v in values)))


def cstring(address):
    return bytes(u.mem_read(address, 12002)).split(b'\0', 1)[0]


def ret(value=0, pop=0):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, value)
    u.reg_write(UC_X86_REG_EIP, read(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4 + pop)


obj, archive, vt = 0x10000000, 0x10030000, 0x10031000
title_ptr, notes_ptr = 0x10010000, 0x10020000
stack, stop = 0x2000ff00, 0x3000fff0
wire = bytearray()
write(archive, vt)
for off in [0x1c, 0x38, 0x44, 0x48, 0x4c, 0x78, 0xdc]:
    write(vt + off, 0x30000100 + off)
for imp_dll in pe.DIRECTORY_ENTRY_IMPORT:
    for imp in imp_dll.imports:
        if imp.name and b'XlCopyString@@' in imp.name:
            write(imp.address, 0x30001000)
        elif imp.name == b'memcpy':
            write(imp.address, 0x30001010)
write(0x38ff6edc, 0x330199a0)  # Execute actual XlStringSize byte loop.


def hook(_, address, size, data):
    sp = u.reg_read(UC_X86_REG_ESP)
    arg = lambda i: read(sp + 4 + i * 4)
    if address == 0x30001000:
        value = cstring(arg(2))[:arg(1) - 1]
        u.mem_write(arg(0), value + b'\0')
        ret(arg(0))
    elif address == 0x30001010:
        u.mem_write(arg(0), bytes(u.mem_read(arg(1), arg(2))))
        ret(arg(0))
    elif 0x30000100 <= address <= 0x300001dc:
        off = address - 0x30000100
        if off == 0x1c:
            ret(0)  # write archive
        elif off == 0xdc:
            value = bytes(u.mem_read(arg(1), arg(2)))
            wire.extend(struct.pack('<H', len(value)) + value)
            ret(pop=12)
        else:
            width = {0x38: 8, 0x44: 1, 0x48: 8, 0x4c: 4, 0x78: 1}[off]
            wire.extend(u.mem_read(arg(1), width))
            ret(pop=12)


u.hook_add(UC_HOOK_CODE, hook)


def call(address, args):
    write(stack, stop, *args)
    u.reg_write(UC_X86_REG_ESP, stack)
    u.reg_write(UC_X86_REG_ECX, obj)
    u.emu_start(address, stop, count=200000)
    if u.reg_read(UC_X86_REG_EIP) != stop:
        raise RuntimeError('native instruction budget exhausted')


cases = []
for name, title, notes, slot in [
    ('upload_ascii', b'Song', b'T120 C', 0),
    ('upload_unicode', '\ud55c\uae00'.encode(), 'C \ub3c4\U0001f600'.encode(), 255),
    ('upload_empty_native', b'', b'', 0),
    ('upload_cstring_native', b'A\0B', b'C\0D', 7),
]:
    u.mem_write(obj, bytes(0x10000))
    u.mem_write(title_ptr, title + b'\0')
    u.mem_write(notes_ptr, notes + b'\0')
    call(0x397acd20, [0x01020304, 0x05060708, (2 << 8) | (slot << 24), title_ptr, notes_ptr])
    wire.clear()
    call(0x397bc740, [archive])
    cases.append((name, 0x122, bytes(wire)))
for name, midi in [('midi_bytes', b'MThd\0\xff'), ('midi_empty_native', b'')]:
    u.mem_write(notes_ptr, midi + b'\0')
    call(0x397acd00, [len(midi), notes_ptr])
    wire.clear()
    call(0x397b1400, [archive])
    cases.append((name, 0x121, bytes(wire)))
for tooltip, buff in [(0, 0), (1, 0), (0, 1), (1, 1)]:
    call(0x397acd70, [0x01020304, 0x05060708, tooltip, buff])
    wire.clear()
    call(0x397b1480, [archive])
    cases.append((f'request_{tooltip}{buff}', 0x123, bytes(wire)))
wire.clear()
call(0x3901c170, [archive])
cases.append(('end', 0x124, bytes(wire)))
u.mem_write(notes_ptr, bytes([0xa5]) * 32768)
call(0x397acd00, [32768, notes_ptr])
wire.clear()
call(0x397b1400, [archive])
(OUT / 'native-midi-max.body').write_bytes(wire)
u.mem_write(title_ptr, b'T' * 96 + b'\0')
u.mem_write(notes_ptr, b'C' * 12000 + b'\0')
call(0x397acd20, [1, 0, (2 << 8) | (255 << 24), title_ptr, notes_ptr])
wire.clear()
call(0x397bc740, [archive])
(OUT / 'native-upload-max.body').write_bytes(wire)
(OUT / 'native.tsv').write_text(
    '# original x2game SHA256 ' + hashlib.sha256(DLL.read_bytes()).hexdigest() + '\n' +
    '\n'.join(f'{name}\t{opcode:03x}\t{body.hex()}' + ('\t# empty body' if not body else '')
              for name, opcode, body in cases) + '\n',
    encoding='utf8')

# Extract complete function text by balanced braces, preserving bodies verbatim.
def function(source, signature, start=0):
    begin = source.index(signature, start)
    brace = source.index('{', begin)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[begin:end]


ROOT = Path('D:/aaemu-wt/bevy/rust/crates')
stream_path = ROOT / 'aaemu-commons/src/packet_stream.rs'
music_path = ROOT / 'aaemu-game/src/systems/services/music.rs'
special_path = ROOT / 'aaemu-game/src/systems/skills/packets_special.rs'
stream = stream_path.read_text(encoding='utf8')
music = music_path.read_text(encoding='utf8')
special = special_path.read_text(encoding='utf8')
rust = ['// Generated by tests/music_oracle.py; actual lane method bodies, minimal scaffold.']
for path in [stream_path, music_path, special_path]:
    rust.append(f'// {path}: SHA256 {hashlib.sha256(path.read_bytes()).hexdigest()}')
rust.append('''#![allow(dead_code)]
#[derive(Debug)]
pub enum MarshalError { EndOfStream { pos:usize,wanted:usize,len:usize } }
type Result<T> = std::result::Result<T, MarshalError>;
pub struct PacketStream { buf:Vec<u8>,pos:usize }
impl PacketStream {''')
for name in ['from_bytes', 'new', 'remaining', 'into_bytes', 'take', 'take_array<const N: usize>',
             'read_u8', 'read_i16', 'read_u16', 'read_u32', 'read_u64', 'read_bytes',
             'read_bytes_sized', 'read_string', 'write_raw', 'write_u8', 'write_u16',
             'write_u32', 'write_i32', 'write_bc', 'write_bytes_sized', 'write_str']:
    rust.append('pub ' + function(stream, 'fn ' + name + '(' if '<' not in name else 'fn ' + name))
rust.extend(['}', function(stream, 'fn decode_string(')])
for packet, fields in [
    ('CSSaveUserMusicNotesPacket', 'pub song_size:u32,pub item_id:u64,pub value2:u16,pub title:String,pub song:String'),
    ('CSSendUserMusicPacket', 'pub song_size:u32,pub data:Vec<u8>'),
    ('CSRequestMusicNotesPacket', 'pub id64:u64,pub value2:u16'),
    ('CSEndMusicPacket', ''),
]:
    rust.append(f'#[derive(Debug)] pub struct {packet} {{ {fields} }}' if fields else f'#[derive(Debug)] pub struct {packet};')
    method = function(music, 'fn read(', music.index('impl ClientPacket for ' + packet))
    rust.append(f'impl {packet} {{ pub {method} }}')
rust.append('''pub struct SongData {pub id:u32,pub title:String,pub song:String}
pub struct SCUserNoteLoadedPacket {pub song:Option<SongData>}
pub struct SCSendUserMusicPacket {pub obj_id:u32,pub author:String,pub midi:Vec<u8>}
pub struct SCPauseUserMusicPacket {pub obj_id:u32}
pub trait Marshal { fn write(&self,s:&mut PacketStream); }''')
for packet, source in [('SCUserNoteLoadedPacket', music), ('SCSendUserMusicPacket', special), ('SCPauseUserMusicPacket', special)]:
    method = function(source, 'fn write(', source.index('impl Marshal for ' + packet))
    rust.append(f'impl Marshal for {packet} {{ {method} }}')
(OUT / 'lane.rs').write_text('\n\n'.join(rust) + '\n', encoding='utf8')
print(f'Retained {len(cases)} native writer fixtures and actual lane read/write excerpts in {OUT}')
