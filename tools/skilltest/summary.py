"""One line per skill of skilltest runs: python tools/skilltest/summary.py file.jsonl ..."""
import json
import sys

for f in sys.argv[1:]:
    for l in open(f, encoding="utf-8"):
        r = json.loads(l)
        c = r["counts"]
        print(f'{r["tree"]:>2} {r["skill"]:>6} {r["name"][:22]:<22} t{r["target_type"]} cast{r["cast_ms"]:>5} ch{r["chan_ms"]:>6} '
              f'ref={r["refused"]} st={r["started"]} fi={r["fired"]} pl={r["plot_events"]} en={r["ended"]} sp={r["stopped"]} '
              f'dmg={r["damage_n"]} heal={r["heal_n"]} b+={len(r["buffs_created"])} b-={r["buffs_removed"]} cd={r["cooldown_events"]}/{r["resets"]} '
              f'unh={len(r["unhandled"])} dec={len(r["decode_failed"])} err={r["server_errors"]} cast_left={r["casting_left"]} fx={",".join(r["effects"])}')
