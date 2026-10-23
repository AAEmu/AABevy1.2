"""Execute the original 1.2 FMOD layer pan selection/evaluation/multiply instructions.

Requires pefile and Unicorn (as native_playback_oracle.py). No device is opened.
The bounded slice is fmod_eventl.dll!1001D295..1001D2F4 from sub_1001CB90;
its calls to the original first-envelope scan and curve evaluator also execute.
Only the event/layer/list/parameter memory and enclosing stack frame are fixtures.
This proves the pan coefficient, not FMOD's downstream speaker matrix or audibility.
"""

import argparse
from pathlib import Path

from unicorn.x86_const import UC_X86_REG_EBP, UC_X86_REG_EDI, UC_X86_REG_EIP, UC_X86_REG_ESP

from native_playback_oracle import EVENT_HASH, STACK, Native


HOUSING = [(0.0, 0.0, 2), (0.229148, 0.0, 2), (0.322212, 0.646154, 2)]


def coefficient(path, base, x, envelopes):
    n = Native(path, EVENT_HASH)
    event, body, sound, layer = [n.alloc() for _ in range(4)]
    frame = STACK + 0xE000
    n.put(frame - 0xBC, body)
    n.put(body + 4, event)
    n.put(event + 296, base, "f")
    n.put(sound + 24, layer)
    sentinel = layer + 64
    previous = sentinel
    for flags, points in envelopes:
        link, definition, parameter, pointers = [n.alloc() for _ in range(4)]
        n.put(previous, link + 4)
        n.put(link + 20, definition)
        n.put(link + 32, parameter)
        n.put(parameter + 24, x, "f")
        n.put(definition + 16, flags)
        n.put(definition + 48, pointers)
        n.put(definition + 52, len(points), "H")
        for i, (px, py, shape) in enumerate(points):
            point = n.alloc(32)
            n.put(pointers + 4 * i, point)
            n.put(point + 8, px, "f")
            n.put(point + 12, py, "f")
            n.put(point + 16, shape)
        previous = link + 4
    n.put(previous, sentinel)
    n.uc.reg_write(UC_X86_REG_EBP, frame)
    n.uc.reg_write(UC_X86_REG_ESP, frame - 0x400)
    n.uc.reg_write(UC_X86_REG_EDI, sound)
    n.uc.emu_start(0x1001D295, 0x1001D2F4, count=10000)
    assert n.uc.reg_read(UC_X86_REG_EIP) == 0x1001D2F4, "bounded pan slice did not return"
    return n.get(frame - 0xDC, "f")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("event_dll", type=Path)
    parser.add_argument("--write-cases", type=Path)
    args = parser.parse_args()
    constant = lambda y: [(0.0, y, 2)]
    cases = [
        ("no_curve", 0.125, 0.5, []),
        ("zero_event", 0.0, 0.5, [(0x404, constant(0.8))]),
        ("housing_near", 1.0, 0.1, [(0x404, HOUSING)]),
        ("housing_ramp", 1.0, (0.229148 + 0.322212) / 2, [(0x404, HOUSING)]),
        ("housing_far", 1.0, 0.5, [(0x404, HOUSING)]),
        ("event_times_curve", 0.5, 0.5, [(0x404, HOUSING)]),
        ("disabled", 0.125, 0.5, [(0x405, HOUSING)]),
        ("first_disabled", 0.125, 0.5, [(0x405, HOUSING), (0x404, constant(0.2))]),
        ("first_match", 0.5, 0.5, [(0x404, constant(0.25)), (0x404, constant(0.75))]),
        ("skip_other_kind", 0.5, 0.5, [(0xC, constant(0.75)), (0x404, constant(0.25))]),
        ("empty_curve", 0.5, 0.5, [(0x404, [])]),
    ]
    rows = ["# name\tevent pan\tnormalized parameter\tenvelopes (flags@x:y:shape,...;...)\tnative coefficient"]
    print("original fmod_eventl.dll SHA256:", EVENT_HASH)
    for name, base, x, envelopes in cases:
        result = coefficient(args.event_dll, base, x, envelopes)
        encoded = ";".join(f"{flags}@" + ",".join(":".join(map(str, p)) for p in points) for flags, points in envelopes)
        rows.append(f"{name}\t{base}\t{x}\t{encoded}\t{result}")
        print(f"{name}: event={base} parameter={x} native={result}")
    if args.write_cases:
        args.write_cases.write_text("\n".join(rows) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
