"""Print records of a skilltest run: python tools/skilltest/show.py run.jsonl [skill id ...]"""
import json
import sys

recs = [json.loads(l) for l in open(sys.argv[1], encoding="utf-8")]
want = {int(a) for a in sys.argv[2:]}
for r in recs:
    if want and r["skill"] not in want:
        continue
    print("=" * 100)
    keys = ["skill", "name", "phase", "tree", "target_type", "cast_ms", "chan_ms", "cooldown_ms", "plot", "effects", "buffs", "started",
            "refused", "fired", "plot_events", "ended", "stopped", "damage_n", "damage_on_target", "damage_total", "heal_n", "buffs_created",
            "buffs_removed", "cooldown_events", "resets", "unhandled", "decode_failed", "server_errors", "casting_left", "last_used_set", "counts"]
    print({k: r[k] for k in keys})
    for e in r["events"]:
        print("   ", e)
