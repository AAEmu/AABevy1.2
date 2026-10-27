"""Execute original 1.2 score-cache, voice-manager and received-start instructions.

Run from the repository root with Python, pefile, capstone and unicorn. Inputs are
read-only pinned DLLs. No process, server, socket or audio device is opened. SDK,
LevelDB, allocation and presentation endpoints are explicitly substituted; the
cache request constructor, linked list, voice tree, removal and dispatch branches
execute original instructions. JSON records observations, not live I/O success.
"""
from pathlib import Path
import hashlib
import json
import struct

from capstone import Cs, CS_ARCH_X86, CS_MODE_32
import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import (
    UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_EBX, UC_X86_REG_ECX,
    UC_X86_REG_EDI, UC_X86_REG_EIP, UC_X86_REG_ESI, UC_X86_REG_ESP,
)

OUT = Path(__file__).parent / "evidence/user_music_lifecycle"
GAME = Path("<client12-unpacked>/x2game_unpacked.dll")
COMMON = Path("<client12-unpacked>/bin/xlcommon.dll")
RUNTIME_DATA = Path("<re-notes>/housing-audit/native12-data-readonly.bin")
RUNTIME_SHA = "7082573df48d7d5229e1f909845733f0cf9ded6ea1a4e3c41a5e276fccde8c9e"
SHA = "851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e"
STOP = 0x3000FFF0
MANAGER, WORLD, PLAYER = 0x10000000, 0x10010000, 0x10012000
TITLE, NOTES, INPUT, OUTPUT = 0x10020000, 0x10021000, 0x10030000, 0x10040000
SENTINEL, SOUND, SOUND_VT, PARAM, PARAM_VT = (
    0x10050000, 0x10051000, 0x10052000, 0x10053000, 0x10054000)
ENV, SDK, SDK_VT, POSITION = 0x10055000, 0x10056000, 0x10057000, 0x10058000
OWN, OTHER = 47, 99


class Native:
    def __init__(self):
        self.u = Uc(UC_ARCH_X86, UC_MODE_32)
        self.images = []
        for path in (GAME, COMMON):
            data = path.read_bytes()
            sha = hashlib.sha256(data).hexdigest()
            if path == GAME:
                assert sha == SHA
            p = pefile.PE(data=data)
            base = p.OPTIONAL_HEADER.ImageBase
            image = p.get_memory_mapped_image()
            self.u.mem_map(base, (len(image) + 4095) & ~4095)
            self.u.mem_write(base, image)
            self.images.append((path.name, sha, base, image))
        for address, size in [(0, 0x1000), (0x10000000, 0x400000),
                              (0x20000000, 0x200000), (0x30000000, 0x10000)]:
            self.u.mem_map(address, size)
        self.hooks = {}
        self.calls = []
        self.heap = 0x10100000
        self.db = {}
        self.save_ok = True
        self.voices = set()
        self.create_ok = self.actor_ok = True
        self.next_voice = 101
        self.u.hook_add(UC_HOOK_CODE, self.hook)
        self.install()
        runtime = RUNTIME_DATA.read_bytes()
        assert len(runtime) == 11670016 and hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
        self.constants = {}
        for address in (0x39AC37A0, 0x39ADE758, 0x39ADDD60, 0x39AC37A4, 0x39AC37A8,
                        0x3A1776E0, 0x3A177710, 0x3A177728):
            value = struct.unpack_from("<I", runtime, address-0x39AC3000)[0]
            self.put(address, value)
            self.constants[f"{address:08x}"] = value

    def put(self, a, *values):
        self.u.mem_write(a, struct.pack("<" + "I" * len(values),
                                     *(v & 0xFFFFFFFF for v in values)))

    def get(self, a):
        return struct.unpack("<I", self.u.mem_read(a, 4))[0]

    def string(self, a, cap=16000):
        result = bytearray()
        while len(result) < cap:
            part = bytes(self.u.mem_read(a+len(result), min(64, cap-len(result), 4096-((a+len(result))&4095))))
            if b"\0" in part:
                return bytes(result)+part.split(b"\0", 1)[0]
            result.extend(part)
        raise AssertionError("unterminated fixture string")

    def arg(self, i):
        return self.get(self.u.reg_read(UC_X86_REG_ESP) + 4 + 4 * i)

    def ret(self, value=0, pop=0):
        sp = self.u.reg_read(UC_X86_REG_ESP)
        self.u.reg_write(UC_X86_REG_EAX, value & 0xFFFFFFFF)
        self.u.reg_write(UC_X86_REG_EIP, self.get(sp))
        self.u.reg_write(UC_X86_REG_ESP, sp + 4 + pop)

    def hook(self, _, address, size, data):
        if address in self.hooks:
            self.hooks[address]()

    def call(self, address, this=0, *args, end=STOP):
        sp = 0x201FF000
        self.put(sp, STOP, *args)
        self.u.reg_write(UC_X86_REG_ESP, sp)
        self.u.reg_write(UC_X86_REG_ECX, this)
        self.u.emu_start(address, end, count=1000000)
        assert self.u.reg_read(UC_X86_REG_EIP) == end, hex(self.u.reg_read(UC_X86_REG_EIP))
        return self.u.reg_read(UC_X86_REG_EAX)

    def alloc(self):
        n = self.arg(0)
        a = self.heap
        self.heap += (n + 31) & ~15
        self.u.mem_write(a, bytes(n))
        self.ret(a)

    def probe(self):
        sp = self.u.reg_read(UC_X86_REG_ESP)
        target = self.get(sp)
        self.u.reg_write(UC_X86_REG_ESP, sp + 4 - self.u.reg_read(UC_X86_REG_EAX))
        self.u.reg_write(UC_X86_REG_EIP, target)

    def memset(self):
        self.u.mem_write(self.arg(0), bytes([self.arg(1) & 255]) * self.arg(2))
        self.ret(self.arg(0))

    def memcpy(self):
        self.u.mem_write(self.arg(0), bytes(self.u.mem_read(self.arg(1), self.arg(2))))
        self.ret(self.arg(0))

    def format_key(self):
        # Observe only: original XlFormatString and its formatter execute below.
        # Windows long is 32 bits: %016lx consumes low32, ignoring pushed high32.
        fmt = self.string(self.arg(2)).decode("ascii")
        assert fmt == "user_music_%02x_%016lx", fmt
        world, lo, hi = self.arg(3), self.arg(4), self.arg(5)
        self.calls.append({"endpoint": "native_format_entry_observed", "format": fmt,
                           "world": world, "id": f"{hi:08x}{lo:08x}"})

    def load(self):
        key = self.string(self.arg(0)).decode()
        value = self.db.get(key)
        if value is not None:
            value = value[:self.arg(2)]
            self.u.mem_write(self.arg(1), value)
            self.put(self.arg(3), len(value))
        self.calls.append({"endpoint": "LevelDbLoad", "key": key,
                           "capacity": self.arg(2), "hit": value is not None})
        self.ret(int(value is not None))

    def save(self):
        key = self.string(self.arg(0)).decode()
        value = bytes(self.u.mem_read(self.arg(1), self.arg(2)))
        if self.save_ok:
            self.db[key] = value
        self.calls.append({"endpoint": "LevelDbSave", "key": key,
                           "length": len(value), "sync": self.arg(3),
                           "title": value[:97].split(b"\0", 1)[0].hex(),
                           "notes": value[97:].hex(), "result": self.save_ok})
        self.ret(int(self.save_ok))

    def sender(self):
        packet = self.arg(0)
        opcode = self.get(packet + 4)
        row = {"endpoint": "39186810_packet_sink", "opcode": f"{opcode:03x}"}
        if opcode == 0x123:
            lo, hi = self.get(packet + 16), self.get(packet + 20)
            row.update(id=f"{hi:08x}{lo:08x}",
                       tooltip=self.u.mem_read(packet + 24, 1)[0],
                       buff_only=self.u.mem_read(packet + 25, 1)[0])
        elif opcode == 0x121:
            n = self.get(packet + 12)
            row.update(length=n, midi=bytes(self.u.mem_read(packet + 16, n)).hex())
        else:
            assert opcode == 0x124, hex(opcode)
        self.calls.append(row)
        self.ret(pop=4)

    def create(self):
        self.calls.append({"endpoint": "SDK_create", "path": self.string(self.arg(0)).decode(),
                           "flags": self.arg(1), "result": self.create_ok})
        self.ret(SOUND if self.create_ok else 0, 8)

    def sound_id(self):
        voice = self.next_voice
        self.next_voice += 1
        self.voices.add(voice)
        self.calls.append({"endpoint": "SDK_voice_id", "voice": voice})
        self.ret(voice)

    def stop_sound(self):
        voice = self.arg(0)
        self.voices.discard(voice)
        self.calls.append({"endpoint": "SDK_stop", "voice": voice, "immediate": self.arg(1)})
        self.ret(pop=8)

    def install(self):
        self.hooks.update({0x3950A1B0: self.alloc, 0x3950A1F0: lambda: self.ret(),
                           0x398B56A0: self.probe, 0x398B562A: self.memset,
                           0x39186810: self.sender,
                           0x393D5170: lambda: self.ret(WORLD),
                           0x3301B6A0: self.format_key, 0x30000020: self.load,
                           0x30000030: self.save, 0x30000040: self.memcpy,
                           0x33034C6E: self.memcpy, 0x33034C04: self.memset,
                           0x30000100: self.create, 0x30000110: lambda: self.ret(pop=4),
                           0x30000120: lambda: self.ret(PARAM),
                           0x30000130: lambda: self.ret(pop=4),
                           0x30000140: lambda: self.ret(PARAM),
                           0x30000150: lambda: self.ret(pop=8),
                           0x30000160: lambda: self.ret(pop=4),
                           0x30000170: lambda: self.ret(pop=16),
                           0x30000180: self.sound_id,
                           0x30000190: lambda: self.ret(pop=4),
                           0x39360020: lambda: self.ret(PLAYER if self.actor_ok else 0),
                           0x390B3850: lambda: self.ret(POSITION),
                           0x393F1470: lambda: self.ret(SOUND if self.arg(0) in self.voices else 0, 4),
                           0x393F1BD0: self.stop_sound,
                           0x393F14A0: lambda: self.ret(pop=16)})
        for iat, target in [(0x38FF6F2C, 0x3301B6A0), (0x38FF6F80, 0x30000020),
                            (0x38FF6F9C, 0x30000030), (0x38FF6F3C, 0x3301B080),
                            (0x38FF6EDC, 0x330199A0), (0x38FF6C5C, 0x3301B000),
                            (0x38FF6E9C, 0x3301A100)]:
            self.put(iat, target)
        p = pefile.PE(str(GAME))
        for d in p.DIRECTORY_ENTRY_IMPORT:
            for x in d.imports:
                if x.name == b"memcpy":
                    self.put(x.address, 0x30000040)
        self.put(0x39AE4674, MANAGER)
        self.put(0x39B02294, WORLD)
        self.put(WORLD + 0x10C, PLAYER)
        self.put(PLAYER + 8, OWN)
        self.put(0x39AE4684, OWN)
        self.put(0x39ADDD58, OWN)
        self.put(0x3A5BB1D8, ENV)
        self.put(ENV + 0x30, SDK)
        self.put(SDK, SDK_VT)
        self.put(SDK_VT + 0x4C, 0x30000100)
        self.put(SOUND, SOUND_VT)
        self.put(PARAM, PARAM_VT)
        for off, target in [(0x3C, 0x30000110), (0xB8, 0x30000120),
                            (0xBC, 0x30000140), (8, 0x30000160),
                            (0x20, 0x30000170), (0x70, 0x30000180)]:
            self.put(SOUND_VT + off, target)
        for off, target in [(0x11C, 0x30000130), (8, 0x30000150), (0xC0, 0x30000190)]:
            self.put(PARAM_VT + off, target)
        self.reset_pending()
        self.reset_tree()

    def reset_pending(self):
        head = 0x10059000
        self.put(head, head, head)
        self.put(0x39ADE74C, head, 0)

    def pending(self):
        head = self.get(0x39ADE74C)
        node = self.get(head)
        values = []
        while node != head:
            assert len(values) < 100
            values.append(f"{self.get(node+12):08x}{self.get(node+8):08x}")
            node = self.get(node)
        assert len(values) == self.get(0x39ADE750)
        return values

    def reset_tree(self):
        self.u.mem_write(MANAGER, bytes(0xDF00))
        self.put(MANAGER, 8, 0, SENTINEL, 0)
        self.put(SENTINEL, SENTINEL, SENTINEL, SENTINEL, 0, 0)
        self.u.mem_write(SENTINEL + 0x14, b"\x01\x01")

    def tree(self):
        # Native std::map size and root; no Python-owned membership model.
        root = self.get(SENTINEL + 4)
        return {"count": self.get(MANAGER + 12), "root_is_nil": root == SENTINEL,
                "actor": self.get(root + 12) if root != SENTINEL else None,
                "voice": self.get(root + 16) if root != SENTINEL else None}


def cache_cases(n):
    rows = []
    for world, note in [(0, 0), (255, 0xFEDCBA9876543210), (128, 0xFFFFFFFF80000000)]:
        for tooltip in (0, 1):
            n.reset_pending()
            n.db.clear()
            n.u.mem_write(WORLD + 0x7B, bytes([world]))
            n.u.mem_write(MANAGER + 0x28, b"PRE\0")
            lo, hi = note & 0xFFFFFFFF, note >> 32
            key = f"user_music_{world:02x}_{lo:016x}"
            n.calls.clear()
            a = n.call(0x39082550, 0, lo, hi, tooltip)
            first = list(n.calls)
            assert a == 0 and n.pending() == [f"{note:016x}"]
            sent = [x for x in first if "opcode" in x]
            assert sent == [{"endpoint": "39186810_packet_sink", "opcode": "123",
                             "id": f"{note:016x}", "tooltip": tooltip, "buff_only": 0}]
            n.calls.clear()
            assert n.call(0x39082550, 0, lo, hi, 1-tooltip) == 0
            assert not any("opcode" in x for x in n.calls)
            rows.append({"case": "miss_then_opposite_flag_deduplicated", "world": world,
                         "id": f"{note:016x}", "tooltip": tooltip, "first": first,
                         "second": list(n.calls), "pending": n.pending()})
            for save_ok in (False, True):
                n.save_ok = save_ok
                n.u.mem_write(TITLE, b"Title\0")
                n.u.mem_write(NOTES, b"T120 CDE\0")
                n.calls.clear()
                # isTooltip is the unused third argument. Sixth length also unused.
                n.call(0x39082260, 0, lo, hi, tooltip, TITLE, NOTES, 0xDEADBEEF)
                assert n.pending() == []
                assert n.string(MANAGER + 0x28) == b"T120 CDE"
                assert not any("opcode" in x for x in n.calls)
                assert next(x for x in n.calls if x["endpoint"] == "LevelDbSave")["key"] == key
                rows.append({"case": "receipt", "save_ok": save_ok, "tooltip": tooltip,
                             "id": f"{note:016x}", "calls": list(n.calls),
                             "pending": n.pending(), "staged_notes": n.string(MANAGER+0x28).decode()})
            for hit_tooltip in (1, 0):
                n.calls.clear()
                assert n.call(0x39082550, 0, lo, hi, hit_tooltip) == 0x39AE16E8
                sent = [x for x in n.calls if "opcode" in x]
                assert len(sent) == int(not hit_tooltip)
                if sent:
                    assert sent[0]["tooltip"] == 0 and sent[0]["buff_only"] == 1
                rows.append({"case": "later_cache_hit", "tooltip": hit_tooltip,
                             "id": f"{note:016x}", "calls": list(n.calls)})
    return rows


def received_note_cases(n):
    rows = []
    saved_hooks = dict(n.hooks)
    n.hooks.update({0x390115A0: lambda: n.ret(pop=16),
                    0x390116D0: lambda: n.ret()})
    packet = 0x10080000
    for raw in (0, 1, 0x7FFFFFFF, 0x80000000, 0xFFFFFFFF):
        for tooltip in (0, 1):
            n.reset_pending()
            n.db.clear()
            n.save_ok = True
            n.u.mem_write(WORLD+0x7B, b"\x81")
            hi = 0xFFFFFFFF if raw & 0x80000000 else 0
            # Pending is created by the actual request path first.
            n.call(0x39082550, 0, raw, hi, 0)
            n.u.mem_write(packet, bytes(0x4000))
            n.put(packet+12, raw, tooltip, 0xDEADBEEF)
            n.u.mem_write(packet+24, b"Received\0")
            n.u.mem_write(packet+121, b"CDE\0")
            n.calls.clear()
            n.call(0x391E1640, 0, 0, packet)
            key = f"user_music_81_{raw:016x}"
            assert key in n.db and not n.pending()
            assert n.string(MANAGER+40) == b"CDE"
            assert not any("opcode" in x for x in n.calls)
            rows.append({"case": "complete_SC1e0_callback", "raw_id": f"{raw:08x}",
                         "tooltip": tooltip, "declared_length": 0xDEADBEEF,
                         "key": key, "pending": n.pending(), "calls": list(n.calls)})
    # Real CopyString truncates bytes, including a split UTF8 sequence. No Unicode
    # validity/declared noteLen gate exists in the cache receipt routine itself.
    for title, notes in [(b"A"*100, b"C"*12005),
                         (b"A"*95+"é".encode(), b"C"*11999+"é".encode()),
                         (b"A\0B", b"C\0D")]:
        n.u.mem_write(TITLE, title+b"\0")
        n.u.mem_write(INPUT, notes+b"\0")
        n.calls.clear()
        n.call(0x39082260, 0, 123, 0, 0, TITLE, INPUT, 0)
        expected_title = title.split(b"\0", 1)[0][:96]
        expected_notes = notes.split(b"\0", 1)[0][:12000]
        saved = next(x for x in n.calls if x["endpoint"] == "LevelDbSave")
        assert bytes.fromhex(saved["title"]) == expected_title
        assert bytes.fromhex(saved["notes"]) == expected_notes
        assert n.string(MANAGER+40) == expected_notes
        rows.append({"case": "Cstring_byte_limits", "input_title_bytes": len(title),
                     "input_notes_bytes": len(notes), "saved_length": saved["length"],
                     "title_bytes": len(expected_title), "notes_bytes": len(expected_notes),
                     "title_tail": expected_title[-8:].hex(), "notes_tail": expected_notes[-8:].hex()})
    n.hooks = saved_hooks
    return rows


def alias_cases(n):
    rows = []
    saved_hooks = dict(n.hooks)
    n.hooks.update({0x390115A0: lambda: n.ret(pop=16),
                    0x390116D0: lambda: n.ret()})
    packet = 0x10080000
    for low, highs in [(0x1234, (1, 2)), (0x80000000, (0, 0xFFFFFFFF))]:
        n.reset_pending()
        n.db.clear()
        n.save_ok = True
        n.u.mem_write(WORLD+0x7B, b"\x02")
        n.calls.clear()
        for high in highs:
            n.call(0x39082550, 0, low, high, 0)
        pending_before = n.pending()
        assert len(pending_before) == 2
        assert len([x for x in n.calls if "opcode" in x]) == 2
        n.u.mem_write(packet, bytes(0x4000))
        n.put(packet+12, low, 0, 3)
        n.u.mem_write(packet+24, b"T\0")
        n.u.mem_write(packet+121, b"CDE\0")
        n.call(0x391E1640, 0, 0, packet)
        pending_after = n.pending()
        reply_hi = 0xFFFFFFFF if low & 0x80000000 else 0
        expected = [f"{h:08x}{low:08x}" for h in highs if h != reply_hi]
        assert pending_after == expected
        key = f"user_music_02_{low:016x}"
        assert list(n.db) == [key]
        assert n.call(0x39082550, 0, low, highs[0], 0) == 0x39AE16E8
        assert n.pending() == expected  # Hit does not repair nonmatching pending IDs.
        rows.append({"case": "low32_DB_alias_full64_pending", "request_ids": pending_before,
                     "reply_id": f"{reply_hi:08x}{low:08x}", "key": key,
                     "pending_after": pending_after, "calls": list(n.calls)})
    n.reset_pending()
    n.db.clear()
    n.calls.clear()
    for world in (3, 4):
        n.u.mem_write(WORLD+0x7B, bytes([world]))
        n.call(0x39082550, 0, 7, 0, 0)
    assert len([x for x in n.calls if "opcode" in x]) == 1
    rows.append({"case": "pending_is_ID_only_across_worlds", "calls": list(n.calls),
                 "pending": n.pending()})
    n.hooks = saved_hooks
    return rows


def voice_cases(n):
    rows = []
    for actor in (OWN, OTHER):
        for outcome in ("natural", "stop", "replace", "failed_create", "missing_actor", "registry_disappeared"):
            n.reset_tree()
            n.voices.clear()
            n.create_ok = n.actor_ok = True
            n.calls.clear()
            n.u.mem_write(INPUT, b"fixture.mid\0")
            voice = n.call(0x390846D0, MANAGER, INPUT, actor)
            assert voice and n.tree()["count"] == 1
            admitted = n.tree()
            if outcome == "natural":
                n.voices.remove(voice)  # SDK reports natural completion at next lookup.
            elif outcome == "stop":
                n.call(0x390840F0, MANAGER, actor)
            elif outcome == "registry_disappeared":
                n.actor_ok = False
            else:
                n.create_ok = outcome != "failed_create"
                n.actor_ok = outcome != "missing_actor"
                second = n.call(0x390846D0, MANAGER, INPUT, actor)
                if outcome == "replace":
                    assert second and second != voice and n.tree()["count"] == 1
                else:
                    assert second == 0 and n.tree()["count"] == 0
            n.call(0x390844A0, MANAGER)
            sent = [x for x in n.calls if "opcode" in x]
            assert len(sent) == int(actor == OWN and outcome == "natural"), sent
            if sent:
                assert sent[0]["opcode"] == "124"
            if outcome not in ("replace", "registry_disappeared"):
                assert n.tree()["count"] == 0
            # A second frame must not repeat End.
            before = len(sent)
            n.call(0x390844A0, MANAGER)
            assert len([x for x in n.calls if "opcode" in x]) == before
            rows.append({"case": outcome, "actor": actor, "admitted": admitted,
                         "after": n.tree(), "calls": list(n.calls)})
    return rows


def wide_cases(n):
    rows = []
    for name, data in [("empty", b""), ("mixed", "Aé도😀".encode()),
                       ("bom", "\ufeffCDE".encode()), ("internal_bom", "C\ufeffD".encode()),
                       ("3000_ascii", b"C"*3000), ("3001_ascii", b"C"*3000+b"D"),
                       ("12000_ascii", b"C"*12000),
                       ("3001_supplementary", "😀".encode()*3000+b"D"),
                       ("embedded_nul", b"C\0D")]:
        n.u.mem_write(INPUT, data+b"\0"*8)
        n.u.mem_write(OUTPUT, b"\xa5"*12016)
        result = n.call(0x3301A100, 0, OUTPUT, 3001, INPUT)
        values = list(struct.unpack("<3001I", n.u.mem_read(OUTPUT, 12004)))
        count = values.index(0)
        expected = data.split(b"\0", 1)[0].decode().removeprefix("\ufeff")[:3000]
        assert values[:count] == [ord(x) for x in expected]
        assert result == count+1
        assert bytes(n.u.mem_read(OUTPUT+12004, 12)) == b"\xa5"*12
        rows.append({"case": name, "input_bytes": len(data), "return_including_nul": result,
                     "stored_codepoints": count, "first": values[:min(5,count)],
                     "last": values[max(0,count-3):count], "next_slot": values[count],
                     "output_sha256": hashlib.sha256(bytes(n.u.mem_read(OUTPUT,4*(count+1)))).hexdigest()})
    return rows


def entry_voice_cases(n):
    """Actual preview/received entry and manager, SDK/file/Lua/compiler endpoints mocked."""
    rows = []
    saved_hooks = dict(n.hooks)

    def compile_fixture():
        n.calls.append({"endpoint": "39083960_compiler_fixture", "capacity": n.arg(1),
                        "notes": n.string(n.arg(2)).hex(), "parameter": n.arg(3)})
        n.u.mem_write(n.arg(0), b"MThd\0\xFF")
        n.ret(6)

    def path_fixture():
        n.calls.append({"endpoint": "39082310_file_fixture", "actor": n.arg(2),
                        "length": n.arg(4)})
        n.u.mem_write(n.arg(0), b"fixture.mid\0")
        n.ret()

    def lua_return():
        n.calls.append({"endpoint": "39079890_Lua_return_fixture", "voice": n.get(n.arg(0))})
        n.ret(1)

    n.hooks.update({0x39083960: compile_fixture, 0x39082310: path_fixture,
                    0x39079890: lua_return, 0x30000200: lambda: n.ret()})
    lua, lua_vt = 0x1005A000, 0x1005A100
    n.put(lua, lua_vt)
    n.put(lua_vt+0x2C, 0x30000200)
    n.u.mem_write(NOTES, b"T120 CDE\0")
    for entry in ("preview", "received"):
        for finish in ("natural", "local_stop", "create_failed"):
            n.reset_tree()
            n.voices.clear()
            n.create_ok = finish != "create_failed"
            n.actor_ok = True
            n.calls.clear()
            if entry == "preview":
                n.call(0x39079B90, lua, lua, NOTES)
            else:
                n.call(0x390824E0, MANAGER, 6, OWN, TITLE, NOTES)
            admitted = n.tree()
            assert admitted["count"] == int(n.create_ok)
            if finish == "natural":
                n.voices.clear()
            elif finish == "local_stop":
                # Actual Lua StopPlayNotes entry resolves own actor, then removes tree entry.
                n.call(0x390797F0, lua, lua)
            n.call(0x390844A0, MANAGER)
            sent = [x for x in n.calls if "opcode" in x]
            assert sent == ([{"endpoint": "39186810_packet_sink", "opcode": "124"}]
                            if finish == "natural" else []), sent
            rows.append({"case": entry+"_"+finish, "admitted": admitted,
                         "after": n.tree(), "calls": list(n.calls)})
    # Execute actual framework admission, mode gate and call. The protected
    # MainGame390f6a20 prologue and earlier frame work are not emulated here.
    n.hooks.update({0x30000210: lambda: n.ret(n.frame_mode),
                    0x30000220: lambda: n.ret(n.frame_admitted, 8),
                    0x30000230: lambda: n.ret(1),
                    0x393F4D90: lambda: n.ret(), 0x39052EA0: lambda: n.ret(),
                    0x3907B3C0: lambda: n.ret()})
    n.put(ENV, SDK)
    n.put(SDK_VT+0xA8, 0x30000210)
    n.put(WORLD+0x44, MANAGER)
    framework, framework_vt = 0x1005B000, 0x1005B100
    n.put(WORLD+0x18, framework)
    n.put(framework, framework_vt)
    n.put(framework_vt+0x20, 0x30000220)
    n.put(framework_vt+0x38, 0x30000230)
    def gate_end():
        n.u.emu_stop()
    n.hooks[0x390F6C9B] = gate_end
    n.hooks[0x390F6BF7] = gate_end
    for n.frame_admitted in (0, 1):
        for n.frame_mode in (0, 1, 2, 3):
            n.reset_tree()
            n.create_ok = n.actor_ok = True
            n.voices.clear()
            n.call(0x390846D0, MANAGER, INPUT, OWN)
            n.voices.clear()
            n.calls.clear()
            n.u.reg_write(UC_X86_REG_ESI, WORLD)
            n.u.reg_write(UC_X86_REG_EBP, 0x201F8000)
            n.u.reg_write(UC_X86_REG_EBX, 0)
            n.u.reg_write(UC_X86_REG_EDI, 0)
            n.call(0x390F6BE6, end=0x390F6C9B if n.frame_admitted else 0x390F6BF7)
            sent = [x for x in n.calls if "opcode" in x]
            assert len(sent) == int(bool(n.frame_admitted) and n.frame_mode != 2)
            rows.append({"case": "framework_and_mode_gate", "mode": n.frame_mode,
                         "framework_admitted": bool(n.frame_admitted),
                         "after": n.tree(), "calls": list(n.calls)})
    n.hooks = saved_hooks
    return rows


def skill_start_cases(n):
    """Execute actual received callback and whole dispatcher; presentation is mocked.

    Registry, static catalog, timeline construction and presentation endpoints are
    fixtures, so these assert dispatch gates/order, not native planner admission.
    Normal compile entry39083ba0 and constructor397acd00 still execute originals.
    """
    rows = []
    saved_hooks = dict(n.hooks)
    skill_manager, skill_desc, timeline, effect_vector, effect, effect_type, action = (
        0x10060000, 0x10061000, 0x10062000, 0x10063000, 0x10064000,
        0x10065000, 0x10066000)
    packet, foreign, mode, mode_vt = 0x10067000, 0x10068000, 0x1006E000, 0x1006E100
    n.put(0x39AFF9AC, skill_manager)
    n.put(skill_manager+12, SENTINEL)
    n.put(skill_manager+28, SENTINEL)
    n.put(0x3A1776CC, mode)
    n.put(mode, mode_vt)
    n.put(mode_vt+8, 0x30000300)
    n.put(foreign+8, OTHER)
    n.put(skill_desc+0x1E8, effect_vector, effect_vector+4)
    n.put(effect_vector, effect)
    n.put(effect+28, effect_type)
    n.put(effect_type+4, 24, action)
    n.u.mem_write(MANAGER+0x28, b"T120 CDE\0")

    def lookup():
        actor = n.arg(0)
        ptr = PLAYER if actor == OWN else foreign if actor == OTHER else 0
        if n.missing_actor:
            ptr = 0
        n.calls.append({"endpoint": "registry_fixture", "actor": actor, "resolved": ptr})
        n.ret(ptr)

    def lookup_timeline():
        n.put(n.arg(0), SENTINEL)
        n.ret(pop=8)

    def make_timeline():
        n.calls.append({"endpoint": "timeline_create_fixture", "admitted": n.timeline_ok})
        n.ret(timeline if n.timeline_ok else 0, 24)

    def compile_fixture():
        text = n.string(n.arg(2))
        n.calls.append({"endpoint": "39083960_compiler_fixture", "capacity": n.arg(1),
                        "notes": text.hex(), "parameter": n.arg(3)})
        n.u.mem_write(n.arg(0), b"MThd\0\xFF")
        n.ret(6)

    def dispatch_fixture():
        n.calls.append({"endpoint": "presentation_event_fixture", "internal": n.arg(0)})
        n.ret(PARAM, 8)

    n.put(0x38FF6F30, 0x30000310)
    def post_action_bookkeeping():
        n.calls.append({"endpoint": "39390850_bookkeeping_fixture",
                        "application_method": n.get(effect+4)})
        n.ret()

    n.hooks.update({
        0x39360020: lookup, 0x39394F30: lookup_timeline, 0x39394E70: lookup_timeline,
        0x390B3670: lambda: n.ret(), 0x394DF3E0: lambda: n.ret(),
        0x39390530: lambda: n.ret(), 0x394E2650: lambda: n.ret(),
        0x396F6390: lambda: n.ret(skill_desc if n.descriptor_ok else 0),
        0x3939A440: make_timeline, 0x39386E80: lambda: n.ret(pop=8),
        0x3940A3E0: lambda: n.ret(), 0x39005B20: lambda: n.ret(),
        0x39898EF0: lambda: n.ret(n.get(0x3A177728)),
        0x30000300: lambda: n.ret(2), 0x30000310: lambda: n.ret(),
        0x39390850: post_action_bookkeeping, 0x39397A20: lambda: n.ret(pop=12),
        0x39202FB0: dispatch_fixture, 0x39202DB0: lambda: n.ret(),
        0x39202530: lambda: n.ret(), 0x390115A0: lambda: n.ret(pop=16),
        0x390116D0: lambda: n.ret(), 0x39083960: compile_fixture,
    })
    cases = []
    for special in (93, 94):
        for kind in range(5):
            for actor in (OWN, OTHER):
                for result in (0, 7):
                    cases.append((special, kind, actor, result, True, True, False, 24, 17))
    cases.extend([(93, 0, OWN, 0, False, True, False, 24, 17),
                  (93, 0, OWN, 0, True, False, False, 24, 17),
                  (93, 0, OWN, 0, True, True, True, 24, 17),
                  (93, 0, OWN, 0, True, True, False, 23, 17),
                  (93, 0, OWN, 0, True, True, False, 24, 0),
                  (97, 0, OWN, 0, True, True, False, 24, 17)])
    cases = [(*case, 3 if case[0] in (93, 94) else 1) for case in cases]
    cases.extend((s, 0, OWN, 0, True, True, False, 24, 17, method)
                 for s in (93, 94) for method in (0, 1, 2, 4, 5))
    for special, kind, actor, result, n.timeline_ok, n.descriptor_ok, n.missing_actor, typ, tl, method in cases:
        skill_id = 22213 if special == 93 else 22214 if special == 94 else 22215
        n.u.mem_write(packet, bytes(0x200))
        n.put(packet+12, skill_id, tl, result, 0)
        # Native SkillCaster memory union: unit/mount object+8; item object+14.
        # Unk1/doodad cannot be projected to an own unit here by a legacy actor ID.
        n.put(packet+32, kind)
        n.put(packet+40, actor)
        n.put(packet+52, actor)
        n.put(action, special)
        n.put(effect+4, method)
        n.put(effect_type+4, typ)
        n.calls.clear()
        n.call(0x391D57F0, 0, 0, packet)
        sent = [x for x in n.calls if "opcode" in x]
        expected = (kind in (0, 2, 3) and actor == OWN and result == 0 and n.timeline_ok
                    and n.descriptor_ok and not n.missing_actor and typ == 24 and tl != 0
                    and special in (93, 94))
        assert len(sent) == int(expected), (special, kind, actor, result, n.calls)
        if sent:
            assert sent[0]["opcode"] == ("121" if special == 93 else "124")
        compiled = [x for x in n.calls if x["endpoint"] == "39083960_compiler_fixture"]
        if compiled:
            assert compiled[0]["parameter"] == 33 and compiled[0]["capacity"] == 32768
            assert compiled[0]["notes"] == b"T120 CDE".hex()
        rows.append({"special": special, "skill_id": skill_id, "raw_caster_kind": kind,
                     "actor": actor, "result": result, "timeline": tl,
                     "timeline_created": n.timeline_ok, "descriptor_present": n.descriptor_ok,
                     "missing_actor": n.missing_actor, "effect_type": typ,
                     "application_method": method,
                     "calls": list(n.calls)})
    n.hooks = saved_hooks
    return rows


def normal_compile_cases(n):
    """Execute actual normal entry, wrapper and u32 converter, compiler body mocked."""
    rows = []
    saved_hooks = dict(n.hooks)
    instrument, midi_desc, proficiency = 0x10070000, 0x10070100, 0x10070200
    n.put(instrument, 1)
    n.put(midi_desc, 12)

    def compile_sink():
        ptr = n.arg(2)
        values = []
        while n.get(ptr+4*len(values)):
            values.append(n.get(ptr+4*len(values)))
            assert len(values) <= 3000
        n.calls.append({"endpoint": "390838a0_compile_fixture", "capacity": n.arg(1),
                        "codepoints": len(values), "first": values[:4], "last": values[-3:],
                        "program": n.arg(3), "bank": n.arg(4), "quality_bits": f"{n.arg(5):08x}"})
        n.u.mem_write(n.arg(0), b"MThd\0\xff")
        n.ret(6)

    n.hooks.update({0x39695DC0: lambda: n.ret(instrument),
                    0x39703B50: lambda: n.ret(midi_desc),
                    0x3930B100: lambda: n.ret(proficiency, 4),
                    0x397A9330: lambda: n.ret(10000),
                    0x397A8D10: lambda: n.ret(1),
                    0x39081F80: lambda: n.ret(), 0x390838A0: compile_sink})
    for text in ("", "CDE", "C"*3000+"D", "😀"*3000, "\ufeffCDE"):
        n.u.mem_write(MANAGER+0x28, text.encode()+b"\0")
        n.calls.clear()
        n.call(0x39083BA0, MANAGER)
        sent = [x for x in n.calls if "opcode" in x]
        assert len(sent) == int(bool(text))
        rows.append({"case": "normal_compile", "input_bytes": len(text.encode()),
                     "input_codepoints": len(text), "parameter_global": n.get(0x39AC37A0),
                     "calls": list(n.calls)})
    n.hooks = saved_hooks
    return rows


def source(n):
    cs = Cs(CS_ARCH_X86, CS_MODE_32)
    ranges = [(0x39082550, 346), (0x39082260, 174), (0x39082200, 83),
              (0x397ACD70, 31), (0x39079B90, 355), (0x390824E0, 99),
              (0x390797F0, 70), (0x390840F0, 80), (0x390844A0, 556),
              (0x390846D0, 436), (0x390F6C64, 55), (0x39083BA0, 177),
              (0x39084890, 311), (0x390F755D, 51),
              (0x39083960, 561), (0x391D57F0, 162), (0x3939A810, 2036),
              (0x391E1640, 129), (0x396F6390, 62), (0x39730EE0, 960),
              (0x39730620, 2227), (0x3972C390, 474), (0x39731870, 7089),
              (0x396E8D40, 50), (0x39390850, 82)]
    ranges.extend([(0x390F6A20, 43), (0x390F6A4B, 0x457), (0x39000B12, 11)])
    ranges.extend([(0x39695DC0, 62), (0x39703B50, 63), (0x396F6FB0, 112),
                   (0x396CB3F0, 0x784), (0x396BA730, 0x4B), (0x3971DEE0, 380)])
    lines = ["Read-only original instructions; SHA256 " + SHA]
    for name, sha, base, image in n.images:
        lines.append(f"\nIMAGE {name} SHA256 {sha}")
        selected = ranges if name == GAME.name else [(0x3301A100, 199), (0x3301B080, 21),
                                                     (0x3301B630, 141), (0x33007B70, 0x2080)]
        for address, length in selected:
            if name == GAME.name:
                # The pre-existing .data capture has matching original instructions
                # for every researched region, despite a few unrelated loader writes.
                runtime_text = Path("<re-notes>/housing-audit/native12-text-readonly.bin").read_bytes()
                assert runtime_text[address-0x38FF1000:address-0x38FF1000+length] == image[address-base:address-base+length]
            lines.append(f"\nSOURCE {address:08x}")
            lines.extend(f"{i.address:08x}: {i.mnemonic} {i.op_str}".rstrip()
                         for i in cs.disasm(image[address-base:address-base+length],address))
    return "\n".join(lines)+"\n"


if __name__ == "__main__":
    n = Native()
    assert n.get(0x399A646C) == 0x390F6A20
    rows = {"cache": cache_cases(n), "received_note": received_note_cases(n), "aliases": alias_cases(n),
            "voice_manager": voice_cases(n), "wide": wide_cases(n),
            "entry_voice": entry_voice_cases(n), "skill_start": skill_start_cases(n),
            "normal_compile": normal_compile_cases(n)}
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT/"native-source.txt").write_text(source(n), encoding="utf-8")
    (OUT/"native-cases.json").write_text(json.dumps({
        "images": [{"name": name, "sha256": sha} for name,sha,_,_ in n.images],
        "existing_runtime_data": {"path": str(RUNTIME_DATA), "sha256": RUNTIME_SHA,
                                  "base": "39ac3000", "constants": n.constants},
        "frame_entry": {"vtable_slot": "399a646c", "original_value": "390f6a20",
                        "protected_entry_not_emulated": True,
                        "executed_framework_and_mode_gates": "390f6be6..390f6c9b"},
        "limitations": "SDK, DB, CRT memcpy/memset, allocator and presentation endpoints are fixtures; native formatter/CopyString/converter execute. No live wire/audio success.",
        "cases": rows}, indent=2)+"\n", encoding="utf-8")
    print({k:len(v) for k,v in rows.items()})
