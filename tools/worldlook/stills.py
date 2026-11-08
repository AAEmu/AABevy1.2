"""An aa-client script that takes the stills `orig.py shot` took, from the same camera at the same hour.

    py -3 -I tools/worldlook/stills.py OUT.txt SHOTS_DIR TAG STATE.json [STATE.json ...]

Each still is `hour`, `face` (the character's heading), `view` (the original's camera position and direction) and `shot
SHOTS_DIR/TAG_N.png`. Run it with `aa-client ... --script OUT.txt --no-hud --cam-fov 57` (the engine's 0.9948 rad vertical
field of view); `AA_TUNE=...` tries atmosphere settings without a build.
"""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path


def main(argv: list[str]) -> int:
    if len(argv) < 4:
        raise SystemExit(__doc__)
    out, shots, tag, states = Path(argv[0]), Path(argv[1]), argv[2], argv[3:]
    lines = [f"echo worldlook {tag}", "wait 25"]
    for i, path in enumerate(states, 1):
        s = json.loads(Path(path).read_text(encoding="utf-8"))
        pos, look = s["camera_pos"], s["camera_dir"]
        lines += [
            f"hour {s['cvars']['e_time_of_day']:.4f}",
            f"face {math.degrees(s['player_angles'][2]):.2f}",
            "wait 1",
            "view {:.4f} {:.4f} {:.4f} {:.6f} {:.6f} {:.6f}".format(*pos, *look),
            "wait 4",
            f"shot {(shots / f'{tag}_{i}.png').as_posix()}",
        ]
    lines += ["view off", "hour off", "quit now"]
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(out)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
