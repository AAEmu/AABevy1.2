"""Stills of the player (sheathed and drawn weapon; full, mid and close; front, side, back) into shots/reel/after3/stills/.

    python stills.py [NN ...]      # all nine, or the numbered ones; needs the stage (run_reel.py setup) and target/agents/reel/after3/aa-client.exe
"""
import json, os, subprocess, sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import plan as P, run_reel as R
c = P.connect()
row = c.execute("select id, casting_time, channeling_time, effect_delay, projectile_id, plot_id, target_type_id, ability_level, target_relation_id from skills where id=10399").fetchone()
steps, _ = P.skill_events(c, 3, row, 100, 8.0)
fake = ";".join(steps)
out = Path("shots/reel/after3/stills"); out.mkdir(parents=True, exist_ok=True)
exe = str(Path("target/agents/reel/after3/aa-client.exe").resolve())
# name, drawn, cam-dist, cam-yaw, cam-pitch, fov, shot-after
shots = [
 ("01_sheathed_full_front", 0, 5.0, 215, -5, 45, 14),
 ("02_sheathed_mid_front", 0, 3.6, 180, 0, 45, 14),
 ("03_sheathed_close_front", 0, 2.0, 180, 5, 40, 14),
 ("04_sheathed_full_back", 0, 5.0, 0, -5, 45, 14),
 ("05_sheathed_mid_side", 0, 3.6, 90, 0, 45, 14),
 ("06_drawn_full_front", 1, 5.0, 215, -5, 45, 13.2),
 ("07_drawn_mid_side", 1, 3.6, 90, 0, 45, 13.2),
 ("08_drawn_close_front", 1, 2.2, 180, 5, 40, 13.2),
 ("09_drawn_full_three_quarter", 1, 5.0, 135, -8, 45, 13.2),
]
only = sys.argv[1:]
for name, drawn, dist, yaw, pitch, fov, after in shots:
    if only and name[:2] not in only: continue
    R.wait_ram()
    env = dict(os.environ); env["AA_CAMERA_NO_CLIP"] = "1"
    if drawn: env["AA_COMBAT_FAKE"] = fake; env["AA_COMBAT_FAKE_TARGET"] = str(json.loads((R.WORK/"stage.json").read_text())["dummy_obj"]); env["AA_COMBAT_LOG"] = "0"
    cmd = [exe, "--enter-world", "--user", R.ACCOUNT, "--character", R.CHARACTER, "--hour", "12", "--size", "1600x900", "--no-hud", "--mute",
           "--cam-dist", str(dist), "--cam-yaw", str(yaw), "--cam-pitch", str(pitch), "--cam-fov", str(fov), *R.stage_args(), "--shot", str(out/(name+".png")), "--shot-after", str(after)]
    r = subprocess.run(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=200)
    print(name, r.returncode, flush=True)
