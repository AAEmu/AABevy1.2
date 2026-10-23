"""Execute original 1.2 player-score gain/option instructions in bounded x86 fixtures.

Requires pefile and unicorn. Read-only DLL inputs; no game, network or audio device.
Only the downstream CVar setter, device property setter and mute interfaces are mocked.
The cached MIDI clamp, gain formula, semantic setter and master writes execute DLL bytes.
Gain goldens are pre-device coefficients, not measured native FMOD output totals.
"""

import argparse
import hashlib
import json
from pathlib import Path
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_EBX, UC_X86_REG_ECX, UC_X86_REG_EDI,
    UC_X86_REG_EIP, UC_X86_REG_ESI, UC_X86_REG_ESP,
    UC_X86_REG_XMM0, UC_X86_REG_XMM1, UC_X86_REG_XMM2,
)


CRY_HASH = "4b851d049604b03cba2d016367aac602664b1ec5bf4ce834bf1aa5c490058bf6"
GAME_HASH = "851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e"
HEAP, STACK, RETURN = 0x10000000, 0x20000000, 0x30000000


def bits(value):
    return struct.unpack("<I", struct.pack("<f", value))[0]


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


class Native:
    def __init__(self, path, digest):
        data = path.read_bytes()
        assert hashlib.sha256(data).hexdigest() == digest, "unexpected original DLL"
        pe = pefile.PE(data=data)
        self.u = Uc(UC_ARCH_X86, UC_MODE_32)
        base = pe.OPTIONAL_HEADER.ImageBase
        self.u.mem_map(base, (pe.OPTIONAL_HEADER.SizeOfImage + 4095) & ~4095)
        self.u.mem_write(base, pe.get_memory_mapped_image())
        for address, size in [(HEAP, 0x20000), (STACK, 0x10000), (RETURN, 4096)]:
            self.u.mem_map(address, size)
        self.hooks = {}
        self.u.hook_add(UC_HOOK_CODE, self.hook)

    def put(self, address, value, fmt="I"):
        self.u.mem_write(address, struct.pack("<" + fmt, value))

    def get(self, address, fmt="I"):
        return struct.unpack("<" + fmt, self.u.mem_read(address, struct.calcsize(fmt)))[0]

    def ret(self, cleanup):
        sp = self.u.reg_read(UC_X86_REG_ESP)
        self.u.reg_write(UC_X86_REG_EIP, self.get(sp))
        self.u.reg_write(UC_X86_REG_ESP, sp + 4 + cleanup)

    def hook(self, _, address, size, data):
        if address in self.hooks:
            self.hooks[address]()

    def run(self, start, end):
        self.u.emu_start(start, end, count=1000)
        assert self.u.reg_read(UC_X86_REG_EIP) == end, "bounded native case did not end"

    def call(self, address, this, *args):
        sp = STACK + 0xF000
        self.put(sp, RETURN)
        for i, value in enumerate(args):
            self.put(sp + 4 + i * 4, value)
        self.u.reg_write(UC_X86_REG_ESP, sp)
        self.u.reg_write(UC_X86_REG_ECX, this)
        self.run(address, RETURN)

    def float_result(self):
        # Store and pop the original x87 return, preserving its original f32 conversion.
        self.u.mem_write(RETURN, b"\xd9\x1d" + struct.pack("<I", HEAP + 0x1F000))
        self.u.ctl_remove_cache(RETURN, RETURN + 4096)
        self.run(RETURN, RETURN + 6)
        return self.get(HEAP + 0x1F000, "f")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cry", type=Path)
    parser.add_argument("game", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--tsv", type=Path)
    args = parser.parse_args()
    cry, game = Native(args.cry, CRY_HASH), Native(args.game, GAME_HASH)
    system, sound = HEAP, HEAP + 0x8000
    cry.put(0x360CD334, system)
    # Bounded constructor tail: native unity load and global/master-scale initialization.
    cry.u.reg_write(UC_X86_REG_ESI, system)
    cry.run(0x3604D35E, 0x3604D408)
    assert cry.get(system + 0x4324, "f") == 1.0
    assert cry.get(system + 0x42FC, "f") == 1.0
    cry.put(sound + 0x24, 0x84)  # original 390846d0 CreateSound flags
    setter = cry.get(0x360A1638 + 8)
    assert setter == 0x36039680
    cry.call(setter, sound, 8)
    assert cry.get(sound + 0x20) == 8
    rows = [{"case": "semantic_setter", "vtable": "360a1638", "slot": 8,
             "function": f"{setter:08x}", "semantic": 8}]
    rows.append({"case": "constructor_scales", "global_scale": 1.0, "master_scale": 1.0})

    def cache_midi(user, game_midi):
        cry.put(system + 0x180, user, "f")
        cry.put(system + 0x184, game_midi, "f")
        cry.u.reg_write(UC_X86_REG_ESI, system)
        cry.u.reg_write(UC_X86_REG_XMM0, bits(1.0))
        cry.u.reg_write(UC_X86_REG_XMM1, bits(0.5))
        cry.u.reg_write(UC_X86_REG_XMM2, 0x7FFFFFFF)
        cry.run(0x36050A92, 0x36050B10)
        result = (cry.get(system + 0x50A0, "f"), cry.get(system + 0x50A4, "f"))
        assert result == (max(0.0, min(1.0, user)), max(0.0, min(1.0, game_midi)))
        return result

    for user, game_midi in [(-0.25, 1.25), (0.0, 0.0), (0.25, 0.5), (1.0, 1.0)]:
        rows.append({"case": "midi_cache_clamp", "input": [user, game_midi],
                     "cached": cache_midi(user, game_midi)})

    # Supported bounded route: source/instance/global/master scales at unity.
    goldens = []
    cases = [("unity", 1, 1, 1, 1, 1), ("bgm_zero", 1, 1, 1, 1, 0),
             ("sfx_zero", 1, 1, 1, 0, 1), ("bgm_sfx_zero", 1, 1, 1, 0, 0),
             ("master_zero", 1, 1, 0, 1, 1), ("user_zero", 1, 0, 1, 1, 1),
             ("voice_zero", 0, 1, 1, 1, 1), ("voice_half", .5, 1, 1, 1, 1),
             ("user_quarter", 1, .25, 1, 1, 1), ("master_half", 1, 1, .5, 1, 1),
             ("combined", .5, .25, .5, 0, 0), ("other_sliders", .5, .25, .5, .125, .75)]

    def gain(voice, user, master, sfx=1, bgm=1, source=1, instance=1, scale=1, semantic=8):
        cry.put(sound + 0x20, semantic)
        cry.put(sound + 0x78, instance, "f")
        for offset, value in [(0x4300, sfx), (0x4304, 1), (0x430C, bgm), (0x4310, 1), (0x4324, scale)]:
            cry.put(system + offset, value, "f")
        cache_midi(user, master)
        cry.call(0x360310C0, sound, bits(voice), bits(source))
        result = cry.float_result()
        expected = f32(f32(f32(f32(f32(voice * source) * instance) * user) * master) * scale)
        if semantic != 8:
            expected = f32(f32(f32(voice * source) * instance) * sfx * scale)
        assert result == expected, (result, expected)
        return result

    for name, voice, user, master, sfx, bgm in cases:
        result = gain(voice, user, master, sfx, bgm)
        goldens.append((name, voice, user, master, sfx, bgm, result))
        rows.append({"case": "midi_gain", "name": name, "voice": voice, "user_music": user,
                     "master": master, "sfx": sfx, "bgm": bgm, "gain": result})
    for name, extra in [("source_factor", {"source": .5}), ("instance_factor", {"instance": .25}),
                        ("global_scale", {"scale": .5}), ("sfx_control", {"semantic": 0})]:
        result = gain(.5, .25, .5, sfx=.125, bgm=0, **extra)
        rows.append({"case": "additional_native_factor", "name": name, "factor": extra, "gain": result})

    # Native master setter's downstream device property write is a fixture; its MIDI writes execute.
    device, vtable, callback = HEAP + 0x10000, HEAP + 0x11000, RETURN + 0x100
    cry.put(system + 0x1C8, device)
    cry.put(device, vtable)
    cry.put(vtable + 0x6C, callback)
    properties = []

    def set_property():
        sp = cry.u.reg_read(UC_X86_REG_ESP)
        ptr = cry.get(sp + 8)
        properties.append((cry.get(sp + 4), cry.get(ptr), cry.get(ptr + 4, "f")))
        cry.ret(8)

    cry.hooks[callback] = set_property
    for master in (0, .5, 1):
        for master_scale in (.25, 1):
            cry.put(system + 0x42FC, master_scale, "f")
            cry.call(0x360446A0, system, bits(master))
            effective = cry.get(system + 0x184, "f")
            assert effective == master * master_scale
            assert properties[-1] == (10, 2, effective)
            rows.append({"case": "master_to_game_midi", "master": master,
                         "master_scale": master_scale, "game_midi": effective})

    # Follow the downstream property write separately: FMOD entry points are callbacks,
    # but the system selection and master-fader arguments are original instructions.
    primary, secondary, group = HEAP + 0x12000, HEAP + 0x13000, HEAP + 0x14000
    cry.put(device + 0x34, primary)
    cry.put(device + 0x30, secondary)
    group_calls = []

    def get_master_group():
        sp = cry.u.reg_read(UC_X86_REG_ESP)
        assert cry.get(sp + 4) == primary
        cry.put(cry.get(sp + 8), group)
        cry.u.reg_write(UC_X86_REG_EAX, 0)
        cry.ret(8)

    def set_master_group():
        sp = cry.u.reg_read(UC_X86_REG_ESP)
        assert cry.get(sp + 4) == group
        group_calls.append(cry.get(sp + 8, "f"))
        cry.u.reg_write(UC_X86_REG_EAX, 0)
        cry.ret(8)

    cry.hooks.update({0x3607C9FE: get_master_group, 0x3607C9F2: set_master_group})
    frame = STACK + 0x8000
    for master in (0.0, .25, .5, 1.0):
        cry.put(frame + 12, master, "f")
        for register, value in [(UC_X86_REG_EBP, frame), (UC_X86_REG_ESP, frame - 0x100),
                                 (UC_X86_REG_ESI, device), (UC_X86_REG_EBX, 0)]:
            cry.u.reg_write(register, value)
        cry.run(0x36002D35, 0x36002D86)
        assert group_calls[-1] == master
        rows.append({"case": "downstream_fmod_master", "device_system_offset": "34",
                     "group_gain": group_calls[-1]})

    descriptor, buffer = HEAP + 0x15000, HEAP + 0x17000
    factory_calls = []

    def allocate_buffer():
        cry.u.reg_write(UC_X86_REG_EAX, buffer)
        cry.ret(12)

    def construct_buffer():
        sp = cry.u.reg_read(UC_X86_REG_ESP)
        assert cry.get(sp + 4) == descriptor
        factory_calls.append(cry.get(sp + 8))
        cry.ret(8)

    cry.hooks.update({0x36014810: allocate_buffer, 0x3603E120: construct_buffer})
    for buffer_type in (1, 5):
        cry.put(descriptor + 0x420, buffer_type)
        cry.call(0x36002240, device, descriptor)
        assert factory_calls[-1] == primary
        rows.append({"case": "direct_buffer_system", "buffer_type": buffer_type,
                     "device_system_offset": "34"})

    # Execute the same generic indexed setter the original option dispatcher uses.
    option_writes = []

    def set_cvar():
        sp = game.u.reg_read(UC_X86_REG_ESP)
        name = bytes(game.u.mem_read(game.get(sp + 4), 64)).split(b"\0")[0].decode()
        option_writes.append((name, game.get(sp + 8, "f"), game.get(sp + 12)))
        game.ret(12)

    game.hooks[0x39205740] = set_cvar
    for option, expected in [(35, "s_GameMasterVolume"), (36, "s_MusicVolume"),
                             (37, "s_SFXVolume"), (40, "s_MIDIVolume")]:
        game.call(0x39206060, HEAP, option, bits(.25), 0)
        assert option_writes[-1] == (expected, .25, 0)
        rows.append({"case": "option_binding", "option": option, "cvar": expected})

    def set_int_cvar():
        sp = game.u.reg_read(UC_X86_REG_ESP)
        name = bytes(game.u.mem_read(game.get(sp + 4), 64)).split(b"\0")[0].decode()
        assert (name, game.get(sp + 8), game.get(sp + 12)) == ("user_music_disable_others", 1, 0)
        game.ret(12)

    game.hooks[0x39205980] = set_int_cvar
    game.call(0x39206090, HEAP, 41, 1, 0)
    rows.append({"case": "option_binding", "option": 41, "cvar": "user_music_disable_others", "type": "integer"})

    # Same original mute fragment as root's oracle, retained independently with routing evidence.
    params, params_vt, option_obj, option_vt = HEAP + 0x300, HEAP + 0x400, HEAP + 0x500, HEAP + 0x600
    actor_sound, sound_vt, manager, world, player = HEAP + 0x700, HEAP + 0x800, HEAP + 0x1000, HEAP + 0x1200, HEAP + 0x1400
    get_option, get_params, set_volume = RETURN + 0x100, RETURN + 0x110, RETURN + 0x120
    for address, value in [(actor_sound, sound_vt), (sound_vt + 0xB8, get_params),
                           (params, params_vt), (params_vt + 0xC0, set_volume),
                           (option_obj, option_vt), (option_vt + 8, get_option)]:
        game.put(address, value)
    mute_gains = []

    def return_option():
        game.u.reg_write(UC_X86_REG_EAX, option_value)
        game.ret(0)

    def return_params():
        game.u.reg_write(UC_X86_REG_EAX, params)
        game.ret(0)

    def set_voice():
        mute_gains.append(game.get(game.u.reg_read(UC_X86_REG_ESP) + 4, "f"))
        game.ret(4)

    game.hooks.update({get_option: return_option, get_params: return_params, set_volume: set_voice})
    frame = STACK + 0x8000
    for own in (False, True):
        for current_player in (False, True):
            for present in (False, True):
                for option_value in (0, 1, 2):
                    actor = 47 if own else 99
                    for address, value in [(0x39B02294, world), (world + 0x10C, player if current_player else 0),
                                           (player + 8, 47), (0x39AE4684, 47),
                                           (manager + 0x14, option_obj if present else 0), (frame + 8, actor)]:
                        game.put(address, value)
                    for register, value in [(UC_X86_REG_EBP, frame), (UC_X86_REG_ESP, frame - 0x30),
                                             (UC_X86_REG_ESI, manager), (UC_X86_REG_EDI, actor_sound)]:
                        game.u.reg_write(register, value)
                    game.run(0x39084195, 0x39084211)
                    expected = 0.0 if present and option_value == 1 and not own else 1.0
                    assert mute_gains[-1] == expected
                    rows.append({"case": "disable_other", "own": own, "current_player": current_player,
                                 "option_present": present, "option": option_value, "voice_gain": expected})

    report = {"cry_sha256": CRY_HASH, "game_sha256": GAME_HASH, "cases": len(rows), "rows": rows}
    if args.json:
        args.json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.tsv:
        args.tsv.write_text("case\tvoice\tuser_music\tmaster\tsfx\tbgm\tpre_device_gain\n" +
                            "".join("\t".join(map(str, row)) + "\n" for row in goldens), encoding="utf-8")
    print(f"native player music: {len(rows)} cases passed; {len(goldens)} pre-device coefficient goldens")


if __name__ == "__main__":
    main()
