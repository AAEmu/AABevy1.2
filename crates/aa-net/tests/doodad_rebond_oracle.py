"""Execute original 1.2 receive eligibility and local-send/clear instructions.

Run: python -B crates/aa-net/tests/doodad_rebond_oracle.py
Requires the pinned read-only RE inputs and pefile/capstone/unicorn only for
regeneration. No live process, socket, server, renderer or original asset writes.
"""
from pathlib import Path
import hashlib
import json
import struct
import sys
from capstone import Cs, CS_ARCH_X86, CS_MODE_32

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools"))
from user_music_lifecycle_oracle import Native, SHA, RUNTIME_DATA, RUNTIME_SHA

OUT = Path(__file__).parent / "fixtures/doodad-bond"
U, A, AV, D, DV, R, RV, E, EV, C, CV, B, OCC = [0x10070000 + i * 0x10000 for i in range(13)]
W, ENV = 0x10040000, 0x10041000
ACTOR, OWNER = 47, 123


class Probe(Native):
    def install(self):
        pass  # Disable the inherited unrelated music fixtures.

    def __init__(self):
        super().__init__()
        runtime = RUNTIME_DATA.read_bytes()
        assert hashlib.sha256(runtime).hexdigest() == RUNTIME_SHA
        for address in [0x39ac38c8, 0x39ae6d14, 0x39ae6d38, 0x3a169968,
                        0x3a1699f4, 0x3a5e128c]:
            self.put(address, struct.unpack_from("<I", runtime, address - 0x39ac3000)[0])
        assert self.get(0x3a1699f4) == 0
        self.events = []
        self.owner_present = True
        self.put(0x39b02294, W)
        self.put(W + 0x10c, U)
        self.put(0x3a5bb1d8, ENV)
        self.put(U + 8, ACTOR)
        self.put(U + 0x3be8, A)
        for obj, vtable in [(A, AV), (D, DV), (R, RV), (E, EV), (C, CV)]:
            self.put(obj, vtable)
        self.put(A + 0x164, 0x10063000)
        self.put(D + 0x80, 1236)
        self.put(D + 0x364, OCC)
        self.put(OCC, 2, 2, 1, 0, 0, 0, 0, 0, 1236)
        self.hooks.update({
            0x39360020: lambda: self.ret(U if self.arg(0) == ACTOR else 0),
            0x39035110: lambda: self.ret(D if self.owner_present and self.arg(0) == OWNER else 0, pop=4),
            0x390b4990: self.character_id,
            0x39360920: lambda: self.ret(),
            0x390b3750: lambda: self.ret(A),
            0x3900c690: self.queue,
            0x3900b590: self.cancel,
            0x39186810: self.packet,
            0x39155ed0: self.action,
        })
        self.endpoint(DV + 0xc, lambda: self.ret(R))
        self.endpoint(RV + 0x68, lambda: self.ret(C))
        self.endpoint(AV + 4, lambda: self.ret(1))
        self.endpoint(AV + 0x1b8, lambda: self.ret(E))
        self.endpoint(AV + 0x238, self.attach)
        self.put(AV + 0x23c, 0x390c1420)
        self.endpoint(AV + 0x44, lambda: (self.events.append({"refresh_actor": True}), self.ret()))
        self.endpoint(EV + 0x44, self.detach)
        self.put(0x38ff6f34, self.fn(lambda: self.ret()))

    def fn(self, hook):
        address = 0x30004000 + len(self.hooks) * 0x10
        self.hooks[address] = hook
        return address

    def endpoint(self, slot, hook):
        self.put(slot, self.fn(hook))

    def character_id(self):
        self.put(self.arg(0), 42)
        self.ret(self.arg(0), pop=4)

    def attach(self):
        self.events.append({"apply_actor_bond_endpoint": list(struct.unpack("<5I", self.u.mem_read(self.arg(0), 20)))})
        self.put(A + 0x160, self.get(self.arg(0)))
        self.ret(pop=8)

    def detach(self):
        self.events.append({"entity_detach_endpoint": [self.arg(0), self.arg(1)], "bond_owner_at_detach": self.get(U + 0x3b94)})
        self.ret(pop=8)

    def queue(self):
        self.events.append({"queue_endpoint": [self.arg(i) for i in range(5)]})
        self.ret(pop=20)

    def cancel(self):
        self.events.append({"cancel_callback_endpoint": [self.arg(0), self.arg(1)], "bond_owner_at_cancel": self.get(U + 0x3b94)})
        self.ret(pop=8)

    def packet(self):
        packet = self.arg(0)
        self.events.append({"packet_sink": hex(self.get(packet + 4)), "actor": self.get(packet + 12), "owner": self.get(packet + 16)})
        self.ret(pop=4)

    def action(self):
        self.events.append({"action_endpoint": [self.arg(i) for i in range(3)]})
        self.ret(pop=12)

    def bond(self):
        return list(struct.unpack("<5I", self.u.mem_read(U + 0x3b94, 20)))

    def received(self):
        self.call(0x393b2b90, 0, ACTOR, B)


n = Probe()
original, different = [OWNER, 0, 1, 2, 2], [999, 7, 8, 2, 2]
cases, goldens = [], []


def begin():
    n.owner_present = True
    n.put(U + 0x3b00, 0)
    n.put(B, *original)
    n.put(U + 0x3b94, *original)
    n.put(A + 0x160, OWNER)
    n.events.clear()


def clear_locally():
    n.call(0x3935a520, U)
    assert n.bond() == [0, 0xffffffff, 0, 0, 1] and n.get(A + 0x160) == 0
    assert [list(event)[0] for event in n.events] == ["packet_sink", "cancel_callback_endpoint", "entity_detach_endpoint", "action_endpoint", "refresh_actor"]
    assert n.events[0] == {"packet_sink": "0xcd", "actor": ACTOR, "owner": OWNER}
    assert n.events[1]["bond_owner_at_cancel"] == 0
    n.events.clear()


def keep(name, golden=False):
    cases.append({"case": name, "bond_after_u32": n.bond(), "actor_marker_after": n.get(A + 0x160), "observations": n.events.copy()})
    if golden:
        owner, spot, space, point, kind = n.bond()
        signed = lambda value: value if value < 0x80000000 else value - 0x100000000
        goldens.append(f"{name}\t{owner}\t{point}\t{kind}\t{signed(space)}\t{signed(spot)}")


begin(); n.received()
assert n.bond() == original and not n.events
keep("active_identical_SC139_ignored")
begin(); n.put(B, *different); n.received()
assert n.bond() == original and not n.events
keep("active_different_SC139_ignored")
begin(); clear_locally(); n.received()
assert n.bond() == original and len(n.events) == 1 and "apply_actor_bond_endpoint" in n.events[0]
keep("fresh_identical_SC139_after_local_send_without_SC13A", golden=True)
begin(); clear_locally(); n.put(B, *different); n.received()
assert n.bond() == different and len(n.events) == 1 and n.events[0]["queue_endpoint"][:2] == [ACTOR, 999]
keep("fresh_different_owner_SC139_after_local_send_without_SC13A", golden=True)
begin(); clear_locally(); n.owner_present = False
changed = [OWNER, 0x80000000, 0x7fffffff, 254, 255]
n.put(B, *changed); n.received()
assert n.bond() == changed and len(n.events) == 1 and "queue_endpoint" in n.events[0]
keep("fresh_same_owner_changed_signed_fields_after_local_send_without_SC13A", golden=True)
begin(); clear_locally(); n.put(B, *different); n.received()
assert n.bond() == different
n.events.clear(); n.put(B, *original); n.received()
assert n.bond() == different and not n.events
keep("fresh_bond_consumes_eligibility_next_conflict_ignored")
begin(); clear_locally(); n.put(U + 0x3b00, 456); n.received()
assert n.bond() == [0, 0xffffffff, 0, 0, 1] and not n.events
keep("local_clear_does_not_bypass_separate_mount_gate")
begin(); clear_locally(); n.put(B, 0, 0, 1, 2, 2); n.received()
assert n.bond() == [0, 0xffffffff, 0, 0, 1] and not n.events
keep("local_clear_does_not_admit_zero_owner")

limits = "Original receive gate, bond store, local-send constructor and clear execute. Actor/owner lookup, render/attach, readiness registration/cancellation, packet/action/refresh endpoints are fixtures. Missing-owner cases prove storage and readiness wait, not physical seating or interpretation of signed fields. The separate mount gate is native evidence only; root owns that presentation/control gate. No Rust, live game/server, rendering or network parity is claimed."
result = {"game_sha256": SHA, "runtime_sha256": RUNTIME_SHA, "limits": limits, "cases": cases}
(OUT / "rebond-native.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
(OUT / "rebond-native.tsv").write_text("\n".join(goldens) + "\n", encoding="utf-8")
disassembler = Cs(CS_ARCH_X86, CS_MODE_32)
source = [f"x2game_unpacked.dll SHA256 {SHA}", f"Runtime constants SHA256 {RUNTIME_SHA}; empty owner 00000000", limits]
for start, end in [(0x393b2b90, 0x393b2c0e), (0x393566c0, 0x39356790),
                   (0x3935a520, 0x3935a602), (0x393635e0, 0x393637ec)]:
    raw = bytes(n.u.mem_read(start, end - start))
    source.append(f"\nSOURCE {start:08x}:{end:08x} SHA256 {hashlib.sha256(raw).hexdigest()}")
    source.extend(f"{i.address:08x}: {i.mnemonic} {i.op_str}".rstrip() for i in disassembler.disasm(raw, start))
(OUT / "rebond-source.txt").write_text("\n".join(source) + "\n", encoding="utf-8")
print(f"PASS: {len(cases)} native sequences; {len(goldens)} independent received-bond goldens")
