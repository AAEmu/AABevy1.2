"""A stand-in for aa-client in the run_bevy test: reads `--script`, writes `AA_SKILL_TRACE` rows like the real client would.

`trace TEXT` lines become `mark` rows, `key slotN` a `press` row followed (30 ms later) by an `anim` row, everything else is
ignored. Waits are not honoured (the test must be fast); the run ends with exit code 0, or 2 for a script it cannot read.
"""
import json
import os
import sys
import time


def main(argv):
    script = argv[argv.index("--script") + 1] if "--script" in argv else None
    out = os.environ.get("AA_SKILL_TRACE")
    if not script or not out:
        return 2
    frame = 0
    with open(script, encoding="utf-8") as src, open(out, "a", encoding="utf-8", newline="\n") as trace:
        def row(kind, **fields):
            nonlocal frame
            frame += 1
            trace.write(json.dumps({"src": "bevy", "wall_us": time.time_ns() // 1000, "frame": frame, "kind": kind, **fields}) + "\n")
            trace.flush()

        for line in src:
            words = line.split()
            if not words or words[0].startswith("#"):
                continue
            if words[0] == "trace":
                row("mark", label=words[1])
            elif words[0] == "key" and words[1].startswith("slot"):
                slot = int(words[1][4:])
                row("press", key=str(slot % 10), slot=slot)
                time.sleep(0.03)
                row("anim", inst="e1v7", role="player", clip="fake_cast_01", layer=0, speed=1.0, blend=0.1, loop=False)
    print("fake client done")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
