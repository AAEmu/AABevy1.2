"""Execute original receive/mute instructions before extending player-score playback.

No game process, account, server or native project is modified. Only downstream callbacks
and option/audio interface methods are substituted. Requires pefile and unicorn.
"""
from pathlib import Path
import hashlib
import json
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_EDI, UC_X86_REG_ESI, UC_X86_REG_ESP, UC_X86_REG_EIP

path = Path("D:/aa-re/client12/x2game_unpacked.dll")
sha = hashlib.sha256(path.read_bytes()).hexdigest()
assert sha == "851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e"
pe = pefile.PE(str(path))
native = pe.get_memory_mapped_image()
u = Uc(UC_ARCH_X86, UC_MODE_32)
u.mem_map(pe.OPTIONAL_HEADER.ImageBase, (len(native) + 4095) & ~4095)
u.mem_write(pe.OPTIONAL_HEADER.ImageBase, native)
u.mem_map(0x10000000, 0x20000)
u.mem_map(0x20000000, 0x10000)

def word(address):
    return struct.unpack("<I", u.mem_read(address, 4))[0]

def put(address, *values):
    u.mem_write(address, struct.pack("<" + "I" * len(values), *values))

def ret(value=0, cleanup=0):
    sp = u.reg_read(UC_X86_REG_ESP)
    u.reg_write(UC_X86_REG_EAX, value)
    u.reg_write(UC_X86_REG_EIP, word(sp))
    u.reg_write(UC_X86_REG_ESP, sp + 4 + cleanup)

calls = []
option_value = 0
GET_OPTION, GET_PARAMS, SET_VOLUME = 0x1000F000, 0x1000F010, 0x1000F020
SOUND, SOUND_VT, PARAMS, PARAMS_VT = 0x10000300, 0x10000400, 0x10000600, 0x10000700
MANAGER, WORLD, PLAYER, OPTION, OPTION_VT = 0x10001000, 0x10001200, 0x10001400, 0x10001600, 0x10001700
put(SOUND, SOUND_VT)
put(SOUND_VT + 0xB8, GET_PARAMS)
put(PARAMS, PARAMS_VT)
put(PARAMS_VT + 0xC0, SET_VOLUME)
put(OPTION, OPTION_VT)
put(OPTION_VT + 8, GET_OPTION)

def hook(_, address, size, data):
    sp = u.reg_read(UC_X86_REG_ESP)
    if address == GET_OPTION:
        ret(option_value)
    elif address == GET_PARAMS:
        ret(PARAMS)
    elif address == SET_VOLUME:
        calls.append({"volume_bits": f"{word(sp + 4):08x}"})
        ret(cleanup=4)
    elif address == 0x39082260:
        args = [word(sp + 4 + 4 * i) for i in range(6)]
        calls.append({"note_id": f"{args[1]:08x}{args[0]:08x}",
                      "tooltip": args[2], "title_ptr": args[3],
                      "notes_ptr": args[4], "note_length": args[5]})
        ret()
    elif address == 0x390824E0:
        args = [word(sp + 4 + 4 * i) for i in range(4)]
        calls.append({"length": args[0], "actor": args[1],
                      "author_ptr": args[2], "midi_ptr": args[3]})
        ret()

u.hook_add(UC_HOOK_CODE, hook)
rows = []
frame = 0x20008000

for own in (False, True):
    for current_player in (False, True):
        for option_present in (False, True):
            for option_value in (0, 1, 2):
                calls.clear()
                actor = 47 if own else 99
                put(0x39B02294, WORLD)
                put(WORLD + 0x10C, PLAYER if current_player else 0)
                put(PLAYER + 8, 47)
                put(0x39AE4684, 47)
                put(MANAGER + 0x14, OPTION if option_present else 0)
                put(frame + 8, actor)
                u.reg_write(UC_X86_REG_EBP, frame)
                u.reg_write(UC_X86_REG_ESP, frame - 0x30)
                u.reg_write(UC_X86_REG_ESI, MANAGER)
                u.reg_write(UC_X86_REG_EDI, SOUND)
                u.emu_start(0x39084195, 0x39084211, count=1000)
                expected = "00000000" if option_present and option_value == 1 and not own else "3f800000"
                assert calls == [{"volume_bits": expected}], calls
                rows.append({"case": "other_player_mute", "own": own,
                             "current_player": current_player, "option_present": option_present,
                             "option": option_value, **calls[0]})

packet = 0x10003000
for note_id in (0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF):
    for tooltip in (0, 1):
        calls.clear()
        put(frame + 0xC, packet)
        put(packet + 0xC, note_id)
        u.mem_write(packet + 0x10, bytes([tooltip]))
        put(packet + 0x14, 5)
        u.reg_write(UC_X86_REG_EBP, frame)
        u.reg_write(UC_X86_REG_ESP, frame - 0x40)
        u.emu_start(0x391E167A, 0x391E16A0, count=1000)
        high = 0xFFFFFFFF if note_id & 0x80000000 else 0
        expected = {"note_id": f"{high:08x}{note_id:08x}", "tooltip": tooltip,
                    "title_ptr": packet + 0x18, "notes_ptr": packet + 0x79,
                    "note_length": 5}
        assert calls == [expected], calls
        rows.append({"case": "received_note", **calls[0]})

for length in (0, 1, 32768):
    calls.clear()
    put(frame + 0xC, packet)
    put(packet + 0xC, length)
    put(packet + 0x8010, 0x030201)
    u.reg_write(UC_X86_REG_EBP, frame)
    u.reg_write(UC_X86_REG_ESP, frame - 0x40)
    u.emu_start(0x391E15EA, 0x391E160F, count=1000)
    assert calls == [{"length": length, "actor": 0x030201,
                      "author_ptr": packet + 0x8014, "midi_ptr": packet + 0x10}], calls
    rows.append({"case": "received_midi", **calls[0]})

out = Path("data/extract/aa-client/user-music-playback-native-20261023.json")
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps({"dll_sha256": sha, "cases": rows}, indent=2) + "\n", encoding="utf-8")
print(f"{len(rows)} executed original cases passed -> {out}")
