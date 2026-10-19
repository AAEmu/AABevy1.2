"""Runs `equip_sweep --sheets` for every slot and model and stitches the contact sheets.

    python -I tools/equip_sweep/run_sheets.py <exe> [--slots head,chest] [--models 10,11] [--out shots/aa-anim/equip]

<exe> is a copy of target/debug/examples/equip_sweep.exe (so cargo is free for other builds).
Waits until 6 GB of RAM are free before every start (shared machine).
"""

import ctypes
import os
import subprocess
import sys
import time

SLOTS = {
    # slot: (poses, number of 32 item sheets)
    "head": ("idle,action", 1),
    "chest": ("idle,action", 1),
    "waist": ("idle,action", 1),
    "legs": ("idle,action,sit", 1),
    "hands": ("idle,action", 1),
    "feet": ("idle,action,sit", 1),
    "back": ("idle,action,sit", 1),
    "costume": ("idle,action,sit", 3),
    "main1h": ("idle,action", 1),
    "twohand": ("idle,action", 1),
    "sword_main": ("idle,action", 1),
    "sword_off": ("idle,action", 1),
    "bow": ("idle,action", 1),
    "shield": ("idle,action", 1),
    "string": ("idle,action", 1),
    "percussion": ("idle,action", 1),
    "tube": ("idle,action", 1),
}
MODELS = [10, 11, 16, 17, 18, 19, 20, 21]


class MemStatus(ctypes.Structure):
    _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
        (n, ctypes.c_ulonglong) for n in ("total", "avail", "tpf", "apf", "tv", "av", "ext")
    ]


def free_gb():
    m = MemStatus()
    m.length = ctypes.sizeof(m)
    ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(m))
    return m.avail / 2**30


def option(name, default):
    if name in sys.argv:
        return sys.argv[sys.argv.index(name) + 1]
    return default


def main():
    exe = os.path.abspath(sys.argv[1])
    out = option("--out", "shots/aa-anim/equip")
    slots = option("--slots", ",".join(SLOTS)).split(",")
    models = [int(m) for m in option("--models", ",".join(map(str, MODELS))).split(",")]
    raw = out + "/_raw"
    for model in models:
        for slot in slots:
            poses, sheets = SLOTS[slot]
            for k in range(sheets):
                while free_gb() < 6.0:
                    print("waiting for memory", round(free_gb(), 1), flush=True)
                    time.sleep(20)
                tag = "abc"[k]
                cmd = [exe, "--sheets", "--slot", slot, "--model", str(model), "--poses", poses, "--n", "32",
                       "--skip", str(32 * k), "--out", raw, "--tag", tag]
                print(" ".join(cmd), flush=True)
                try:
                    subprocess.run(cmd, timeout=300, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                except subprocess.TimeoutExpired:
                    print("timeout", slot, model, flush=True)
                subprocess.run([sys.executable, "-I", "tools/equip_sweep/stitch.py", raw, out])


if __name__ == "__main__":
    main()
