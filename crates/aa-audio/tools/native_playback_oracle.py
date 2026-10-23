"""Run the original 1.2 DLL's x86 instance policies in Unicorn (no device/game process).

Requires pefile, unicorn. DLLs are read-only inputs, never distributed here. The pool,
event objects and successful getState/load callbacks are fixtures; selection, event
audibility, simple/complex authored gain and immediate-stop dispatch execute DLL bytes.
This isolates admission from sample decoding, clocks, geometry and the device mixer.
"""

import argparse
import hashlib
from pathlib import Path
import struct

import pefile
from unicorn import Uc, UC_ARCH_X86, UC_MODE_32, UC_HOOK_CODE
from unicorn.x86_const import UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EIP, UC_X86_REG_ESP


EVENT_HASH = "2582e9c58408e8157eb0bdc28b4c787bd15ccde5b8794c0ec529ff102b94db77"
CORE_HASH = "e6bbc885fbcb384cb89f4e0d2d7a4e9e5fe7a86f70a7b90ed06fc35157fd21d6"
BASE, HEAP, STACK, RETURN = 0x10000000, 0x20000000, 0x30000000, 0x40000000


class Native:
    def __init__(self, path, digest):
        data = path.read_bytes()
        assert hashlib.sha256(data).hexdigest() == digest, "unexpected original DLL"
        pe = pefile.PE(data=data)
        self.uc = Uc(UC_ARCH_X86, UC_MODE_32)
        self.uc.mem_map(BASE, (pe.OPTIONAL_HEADER.SizeOfImage + 4095) & ~4095)
        self.uc.mem_write(BASE, pe.get_memory_mapped_image())
        for address, size in [(HEAP, 0x100000), (STACK, 0x10000), (RETURN, 4096)]:
            self.uc.mem_map(address, size)
        self.next = HEAP
        self.hooks = {}
        self.uc.hook_add(UC_HOOK_CODE, self.hook)

    def alloc(self, size=512):
        address = self.next
        self.next += (size + 15) & ~15
        return address

    def put(self, address, value, fmt="I"):
        self.uc.mem_write(address, struct.pack("<" + fmt, value))

    def get(self, address, fmt="I"):
        return struct.unpack("<" + fmt, self.uc.mem_read(address, struct.calcsize(fmt)))[0]

    def ret(self, popped=0, result=0):
        sp = self.uc.reg_read(UC_X86_REG_ESP)
        self.uc.reg_write(UC_X86_REG_EAX, result)
        self.uc.reg_write(UC_X86_REG_EIP, self.get(sp))
        self.uc.reg_write(UC_X86_REG_ESP, sp + 4 + popped)

    def hook(self, uc, address, size, _):
        if address in self.hooks:
            self.hooks[address]()

    def call(self, address, this, *args):
        sp = STACK + 0xF000
        self.put(sp, RETURN)
        for i, arg in enumerate(args):
            self.put(sp + 4 + 4 * i, arg)
        self.uc.reg_write(UC_X86_REG_ESP, sp)
        self.uc.reg_write(UC_X86_REG_ECX, this)
        self.uc.emu_start(address, RETURN, count=100000)
        assert self.uc.reg_read(UC_X86_REG_EIP) == RETURN, "bounded oracle did not return"
        return self.uc.reg_read(UC_X86_REG_EAX)

    def event(self, gain, age=1, volume=1.0, user=1.0, category=1.0):
        event, body, sound, instance, category_obj, group = [self.alloc() for _ in range(6)]
        self.put(event, RETURN)  # fixture event vtable: getState/stop below
        self.put(RETURN + 132, RETURN + 8)
        self.put(RETURN + 16, RETURN + 16)
        self.put(event + 340, body)
        self.put(body, 0x10053B30)  # original simple-event vtable
        self.put(body + 16, sound)
        self.put(sound + 20, instance)
        self.put(instance + 28, gain, "f")
        self.put(event + 68, 8)  # 2D
        self.put(event + 80, age << 32, "Q")  # native age comparison uses the high clock word (+84)
        self.put(event + 48, volume, "f")
        self.put(event + 52, user, "f")
        self.put(event + 128, category_obj)
        self.put(category_obj + 64, group)
        self.put(group + 68, category, "f")
        return event


def select(path, policy, gains, ages, incoming, acquire=False, occupied=None, cursor=0):
    n = Native(path, EVENT_HASH)
    event = n.event(incoming)
    pool, slots, out = n.alloc(), n.alloc(), n.alloc()
    voices = [n.event(gain, age) for gain, age in zip(gains, ages)]
    n.put(event + 112, {3: 0x400, 5: 0x800000}[policy])
    n.put(event + 184, pool)
    n.put(pool, len(voices))
    n.put(pool + 4, slots)
    n.put(pool + 8, cursor)
    for i, voice in enumerate(voices):
        n.put(slots + 4 * i, voice)
    stopped = []

    def state():
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        voice = n.get(sp + 4)
        playing = occupied is None or voices.index(voice) in occupied
        n.put(n.get(sp + 8), 8 if playing else 0)
        n.ret(8)

    def stop():
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        stopped.append((voices.index(n.get(sp + 4)), n.get(sp + 8)))
        n.uc.reg_write(UC_X86_REG_EIP, RETURN)  # stop at the dispatch; no playback setup

    n.hooks[RETURN + 8] = state
    n.hooks[RETURN + 16] = stop
    n.hooks[0x10014D30] = lambda: n.ret(result=0)  # fixed (not unlimited) pool
    n.hooks[0x100127F0] = lambda: n.ret(4)  # already-loaded pool
    if acquire:
        result = n.call(0x10013F30, n.alloc(), event, out, 0)
        if stopped:
            assert stopped[0][1] == 1, "native steal must dispatch stop(immediate=true)"
            return (stopped[0][0], n.get(pool + 8)) if occupied is not None else stopped[0][0]
        assert result == 83, result
        return -1
    result = n.call(0x10011610, n.alloc(), event, out, 0)
    assert result in (0, 83), result
    return voices.index(n.get(out)) if result == 0 else -1


def metrics(path):
    n = Native(path, EVENT_HASH)
    out = n.alloc()
    simple = n.event(0.5, volume=0.75, user=0.25, category=0.5)
    assert n.call(0x1001A150, simple, out) == 0
    assert n.get(out, "f") == 0.046875
    print("native simple metric: instance .5 * event .75 * user .25 * category .5 =", n.get(out, "f"))

    # Complex theoretical gain sums the first active authored instance per layer.
    # No channel/sample/playlist/fade state is created or read.
    event, body = n.event(1.0), n.alloc()
    n.put(event + 340, body)
    n.put(body, 0x10053800)
    head = body + 12
    layers = [n.alloc(), n.alloc()]
    n.put(head, layers[0] + 4)
    for i, layer in enumerate(layers):
        n.put(layer + 4, layers[i + 1] + 4 if i + 1 < len(layers) else head)
        sound, instance = n.alloc(), n.alloc()
        n.put(layer + 52, sound)
        n.put(sound, layer + 52)
        n.put(sound + 16, instance)
        n.put(instance + 28, (0.25, 0.5)[i], "f")
        n.put(layer + 64, layer + 64)  # no envelopes
    assert n.call(0x1001A150, event, out) == 0
    assert n.get(out, "f") == 0.75
    print("native complex metric: layer authored .25 + .5 =", n.get(out, "f"))


def category_metric(path):
    n = Native(path, CORE_HASH)
    parent, child = n.alloc(), n.alloc()
    n.put(parent + 64, 0.25, "f")
    n.put(parent + 44, parent + 44)
    assert n.call(0x100A5440, parent) == 0
    n.put(child + 36, parent)
    n.put(child + 64, 0.5, "f")
    n.put(child + 44, child + 44)
    assert n.call(0x100A5440, child) == 0
    assert n.get(child + 68, "f") == 0.125
    print("native category effective scalar (+0x44): parent .25 * child .5 =", n.get(child + 68, "f"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("event_dll", type=Path)
    parser.add_argument("core_dll", type=Path)
    parser.add_argument("--write-cases", type=Path)
    args = parser.parse_args()
    print("original event/core SHA256:", EVENT_HASH, CORE_HASH)
    metrics(args.event_dll)
    category_metric(args.core_dll)
    cases = [
        ("quietest_not_oldest", 3, [0.8, 0.2, 0.6], [1, 2, 3], 0.1),
        ("all_equal_oldest", 3, [0.2, 0.2, 0.2], [3, 1, 2], 0.0),
        ("partial_tie_pool_order", 3, [0.2, 0.2, 0.8], [3, 1, 2], 0.0),
        ("signed_zero_partial_tie", 3, [0.0, -0.0, 0.8], [3, 1, 2], 0.0),
        ("single_slot", 3, [0.8], [1], 0.0),
        ("louder_admitted", 5, [0.8, 0.2, 0.6], [1, 2, 3], 0.3),
        ("quieter_refused", 5, [0.8, 0.2, 0.6], [1, 2, 3], 0.1),
        ("equal_refused", 5, [0.8, 0.2, 0.6], [1, 2, 3], 0.2),
        ("all_equal_pool_order", 5, [0.2, 0.2, 0.2], [3, 1, 2], 0.3),
        ("all_muted_refused", 5, [0.0, 0.0], [2, 1], 0.0),
        ("all_muted_oldest", 3, [0.0, 0.0], [2, 1], 0.0),
        ("single_equal_refused", 5, [0.8], [1], 0.8),
        ("single_louder_admitted", 5, [0.8], [1], 0.9),
    ]
    rows = ["# name\tpolicy\tpool gains\tages\tincoming gain\tvictim slot (-1 refuses)"]
    for name, policy, gains, ages, incoming in cases:
        victim = select(args.event_dll, policy, gains, ages, incoming)
        acquired = select(args.event_dll, policy, gains, ages, incoming, acquire=True)
        assert acquired == victim, (name, victim, acquired)
        rows.append("\t".join(map(str, [name, policy, ",".join(map(str, gains)), ",".join(map(str, ages)), incoming, victim])))
        print(name, "policy", policy, "victim", victim, "both native paths agree",
              "(immediate stop)" if victim >= 0 else "(refused)")
    for occupied, cursor, want in [([], 0, (1, 1)), ([1], 1, (2, 2)), ([1, 2], 2, (0, 0)),
                                    ([0, 2], 0, (1, 1)), ([0, 1], 1, (2, 2)), ([0, 1, 2], 2, (0, 2))]:
        got = select(args.event_dll, 3, [0.2, 0.5, 0.8], [1, 2, 3], 0.0,
                     acquire=True, occupied=occupied, cursor=cursor)
        assert got == want, (occupied, cursor, got, want)
        print("native slot rotation:", occupied, "cursor", cursor, "=> slot/cursor", got)
    if args.write_cases:
        args.write_cases.write_text("\n".join(rows) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
