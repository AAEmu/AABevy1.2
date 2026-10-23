"""Execute native instrument override initialization and real buff lookup, no live process.

The two IDs come from the pinned, pre-existing read-only runtime data dump.
Native 390828b0, provider vtable399d1990+0xc and 39387690 all execute unchanged.
Only the existing oracle's CRT memset endpoint is substituted on this path.
"""
from pathlib import Path
import hashlib
import json
import struct

from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from user_music_lifecycle_oracle import Native, MANAGER, PLAYER, WORLD, RUNTIME_DATA, RUNTIME_SHA

OUT = Path(__file__).parent / "evidence/user_music_voice"
PROVIDER = PLAYER + 0x3E80
VTABLE = 0x399D1990
LISTS = [(0x20, 0x28), (0xD28, 0xD30), (0x16F0, 0x16F8)]


def main():
    n = Native()
    dump = RUNTIME_DATA.read_bytes()
    assert hashlib.sha256(dump).hexdigest() == RUNTIME_SHA
    constants = {f"{a:08x}": struct.unpack_from("<I", dump, a-0x39AC3000)[0]
                 for a in (0x39AC3798, 0x39AC379C)}
    assert list(constants.values()) == [5068, 8079]
    for address, value in constants.items():
        n.put(int(address, 16), value)
    assert n.get(VTABLE+12) == 0x3938A330
    table = [tuple(struct.unpack("<II", n.u.mem_read(0x399A1E94+8*i, 8))) for i in range(5)]
    assert table[3:] == [(0, 0), (35, 9)]

    cases = [("no_game", [[], [], []], False, False, (73, 0, 5)),
             ("no_player", [[], [], []], True, False, (73, 0, 5)),
             ("empty", [[], [], []], True, True, (73, 0, 5)),
             ("unrelated_id", [[5069, 8080], [], []], True, True, (73, 0, 5))]
    for kind in range(3):
        for buff, expected in [(5068, (0, 0, 5)), (8079, (35, 9, 3))]:
            lists = [[], [], []]
            lists[kind] = [buff]
            cases.append((f"kind{kind}_template{buff}", lists, True, True, expected))
    cases += [("both_piano_priority", [[8079], [5068], []], True, True, (0, 0, 5)),
              ("both_same_list", [[], [], [8079, 5068]], True, True, (0, 0, 5))]
    rows = []
    for name, lists, game, player, expected in cases:
        n.u.mem_write(PROVIDER, bytes(0x2100))
        n.put(PROVIDER, VTABLE)
        n.put(0x39B02294, WORLD if game else 0)
        n.put(WORLD+0x10C, PLAYER if player else 0)
        n.put(MANAGER+0x18, 0, 0)
        for (count, start), values in zip(LISTS, lists):
            n.put(PROVIDER+count, len(values))
            for index, template in enumerate(values):
                row = PROVIDER+start+index*0x68
                n.put(row, 0xDEADBEEF, template)
        n.call(0x390828B0, MANAGER, 73, 0)
        result = (n.get(MANAGER+0xDE90), n.get(MANAGER+0xDE94), n.get(MANAGER+0xDE78))
        initial = (n.get(MANAGER+0xDE98), n.get(MANAGER+0xDE9C), n.get(MANAGER+0xDE88))
        assert result == initial == expected, (name, result, initial)
        rows.append({"case": name, "lists": lists, "game": game, "player": player,
                     "program": result[0], "channel": result[1], "octave": result[2]})
    OUT.mkdir(exist_ok=True)
    result = {"source": "original compiler initializer plus original template-ID lookup across all three lists",
              "dll_sha256": n.images[0][1], "runtime_dump_sha256": RUNTIME_SHA,
              "constants": constants, "voice_table": table, "cases": rows,
              "substitutions": ["CRT memset via existing isolated Native harness"],
              "limits": ["controlled valid native list storage; not a network, device or complete runtime-buff-manager test"]}
    (OUT / "native-cases.json").write_text(json.dumps(result, indent=2)+"\n")
    cs = Cs(CS_ARCH_X86, CS_MODE_32)
    lines = []
    for address, length in [(0x390828B0, 367), (0x3938A330, 9), (0x39387690, 117)]:
        lines.append(f"SOURCE {address:08x}")
        for instruction in cs.disasm(bytes(n.u.mem_read(address, length)), address):
            lines.append(f"{instruction.address:08x}: {instruction.mnemonic} {instruction.op_str}")
    (OUT / "native-source.txt").write_text("\n".join(lines)+"\n")
    print(f"native voice override cases: {len(rows)} passed")


if __name__ == "__main__":
    main()
