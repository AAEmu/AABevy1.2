"""Original 1.2 high-pass descriptors, activation, envelope updates and bypass.

Requires pefile/Unicorn. DLLs are read-only inputs and are never distributed.
EventLayer sub_100263F0 (activate) and sub_10025B10 (update), including the
original curve evaluator/log mapping, execute in full. Successful DSP API calls
are recorded fixtures; getParameterInfo returns the original core descriptor.
This does not execute the FMOD device graph or claim Kira waveform equivalence.
"""

import argparse
import json
from pathlib import Path

from unicorn.x86_const import UC_X86_REG_ESP

from native_playback_oracle import CORE_HASH, EVENT_HASH, Native


ASSETS = [
    ("login_countryside02", 0.618024), ("login_mar03", 0.57862),
    ("day_countryside02", 0.529268), ("night_wind1", 0.315041),
    ("shield_metal", 0.285171),
]


class Layer:
    def __init__(self, event_dll, ranges, points):
        n = self.n = Native(event_dll, EVENT_HASH)
        self.ranges, self.calls = ranges, []
        self.layer, self.link, self.definition, self.parameter, pointers = [n.alloc() for _ in range(5)]
        n.put(self.layer + 24, 2)  # active layer
        n.put(self.layer + 64, self.link + 4)
        n.put(self.link + 4, self.layer + 64)
        n.put(self.link + 20, self.definition)
        n.put(self.link + 28, 1)  # fixture DSP handle
        n.put(self.link + 32, self.parameter)
        n.put(self.link + 36, 1)  # dirty
        n.put(self.definition + 16, 4)
        n.put(self.definition + 48, pointers)
        n.put(self.definition + 52, len(points), "H")
        n.put(self.definition + 56, 1)  # actual five roots use logarithmic mapping
        for i, (x, y, shape) in enumerate(points):
            point = n.alloc(32)
            n.put(pointers + i * 4, point)
            n.put(point + 8, x, "f")
            n.put(point + 12, y, "f")
            n.put(point + 16, shape)
        n.hooks[0x1003FACC] = self.get_type
        n.hooks[0x1003FAC6] = self.get_info
        n.hooks[0x1003FAC0] = lambda: self.record("parameter", 12)
        n.hooks[0x1003FAD2] = lambda: self.record("bypass", 8)
        n.hooks[0x1003FAD8] = lambda: self.record("active", 8)

    def get_type(self):
        n = self.n
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        n.put(n.get(sp + 8), 5)  # original core descriptor DSP type
        n.ret(8)

    def get_info(self):
        n = self.n
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        low, high, _ = self.ranges[n.get(sp + 8)]
        n.put(n.get(sp + 28), low, "f")
        n.put(n.get(sp + 32), high, "f")
        n.ret(32)

    def record(self, kind, popped):
        n = self.n
        sp = n.uc.reg_read(UC_X86_REG_ESP)
        value = n.get(sp + 12, "f") if kind == "parameter" else n.get(sp + 8)
        self.calls.append((kind, value))
        n.ret(popped)

    def run(self, name, start=False, x=None, dirty=None, disabled=None, active=None):
        n = self.n
        if x is not None:
            n.put(self.parameter + 24, x, "f")
        if dirty is not None:
            n.put(self.link + 36, dirty)
        if disabled is not None:
            flags = n.get(self.definition + 16)
            n.put(self.definition + 16, (flags | 1) if disabled else (flags & ~1))
        if active is not None:
            n.put(self.layer + 24, 2 if active else 0)
        self.calls = []
        result = n.call(0x100263F0 if start else 0x10025B10, self.layer, *([1] if start else []))
        assert result == 0
        row = dict(name=name, calls=self.calls.copy(), dirty=n.get(self.link + 36) & 1,
                   disabled_latch=(n.get(self.definition + 16) & 3))
        print(json.dumps(row))
        return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("event_dll", type=Path)
    parser.add_argument("core_dll", type=Path)
    parser.add_argument("--json", type=Path)
    parser.add_argument("--write-cases", type=Path)
    args = parser.parse_args()
    core = Native(args.core_dll, CORE_HASH)
    ranges = [tuple(core.get(0x10131388 + 0x30 * i + 4 * j, "f") for j in range(3)) for i in range(2)]
    assert ranges == [(10.0, 22000.0, 5000.0), (1.0, 10.0, 1.0)]
    print("native core cutoff/resonance min/max/default:", ranges)
    rows, goldens = [], ["# asset\tnormalized cutoff\tnative Hz"]
    for name, y in ASSETS:
        layer = Layer(args.event_dll, ranges, [(0.0, y, 2)])
        start = layer.run(name + "_start", start=True)
        update = layer.run(name + "_update", dirty=1)
        assert start["calls"][1] == ("active", 1)
        assert start["calls"][0] == update["calls"][0]
        assert update["dirty"] == 0
        rows += [start, update]
        goldens.append(f"{name}\t{y}\t{start['calls'][0][1]}")
    # Every actual root's child resonance curve is zero; the native linear map is Q=1.
    child = Layer(args.event_dll, ranges, [(0.0, 0.0, 2)])
    child.n.put(child.definition + 32, 1)
    row = child.run("child_resonance_zero", start=True)
    assert row["calls"] == [("parameter", 1.0)]
    rows.append(row)
    disabled = Layer(args.event_dll, ranges, [(0.0, 0.5, 2)])
    rows.append(disabled.run("disabled_start", start=True, disabled=True))
    row = disabled.run("disabled_first_update")
    assert row["calls"][0] == ("bypass", 1)
    rows.append(row)
    linear = Layer(args.event_dll, ranges, [(0.0, 0.5, 2)])
    linear.n.put(linear.definition + 56, 0)
    row = linear.run("unsupported_linear_mapping", start=True)
    assert row["calls"][0] == ("parameter", 11005.0)
    rows.append(row)
    layer = Layer(args.event_dll, ranges, [(0.0, 0.0, 2), (1.0, 1.0, 2)])
    for name, kwargs, expected in [
        ("initial_dirty", dict(x=0.5), ["parameter"]),
        ("clean", dict(x=0.75), []),
        ("changed_dirty", dict(x=0.75, dirty=1), ["parameter"]),
        ("disable", dict(disabled=True), ["bypass"]),
        ("disabled_unchanged", {}, []),
        ("disabled_dirty", dict(x=0.25, dirty=1), ["parameter"]),
        ("enable", dict(disabled=False), ["bypass"]),
        ("enabled_unchanged", {}, []),
        ("inactive_dirty", dict(active=False, dirty=1), []),
        ("reactivated", dict(active=True), ["parameter"]),
    ]:
        row = layer.run(name, **kwargs)
        assert [kind for kind, _ in row["calls"]] == expected
        rows.append(row)
    # Execute the original Q=1 high-pass coefficient initializer, with a fixture rate.
    coefficients = []
    for name, y in ASSETS:
        core = Native(args.core_dll, CORE_HASH)
        dsp, system = core.alloc(1024), core.alloc(2048)
        hz = float(next(r for r in goldens[1:] if r.startswith(name + "\t")).split("\t")[2])
        core.put(dsp + 0x14, system)
        core.put(system + 0x55C, 24000)
        core.put(dsp + 0x13C, hz, "f")
        core.put(dsp + 0x148, 1.0, "f")
        assert core.call(0x10059D90, dsp) == 0
        a0, a1, a2, b0, b1, b2 = [core.get(dsp + at, "f") for at in range(0x24C, 0x264, 4)]
        coefficients.append(dict(name=name, rate=24000, hz=hz, q=1.0,
                                 biquad=[b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]))
    evidence = dict(event_sha256=EVENT_HASH, core_sha256=CORE_HASH, ranges=ranges,
                    cases=rows, original_core_coefficients=coefficients)
    goldens[0] += "\tcore b0\tcore b1\tcore b2\tcore a1\tcore a2 (24kHz, Q1)"
    for i, coefficient in enumerate(coefficients, 1):
        goldens[i] += "\t" + "\t".join(map(str, coefficient["biquad"]))
    if args.json:
        args.json.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    if args.write_cases:
        args.write_cases.write_text("\n".join(goldens) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
