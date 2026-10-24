"""Retain targeted original graph nodes from aa-pak.exe cat piped over stdin."""
from pathlib import Path
import sys
import xml.etree.ElementTree as E

text = sys.stdin.read()
root = E.fromstring(text.lstrip("\ufeff"))
names = {"fist_ac_sit_down_start", "fist_ac_sit_down_loop", "fist_ac_sit_up",
         "music_co_sk_sit_down_piano_start", "music_co_sk_sit_down_piano_cast",
         "music_co_sk_sit_down_drum_start", "music_co_sk_sit_down_drum_cast"}
out = ["Source: game/animations/graphs/player_full_body.xml through aa-pak.exe cat.",
       "Targeted state definitions and links; no full graph interpretation is claimed."]
for e in root.iter():
    if (e.tag == "State" and e.get("id") in names and e.get("allowSelect") is not None
        or e.tag == "Link" and (e.get("from") in names or e.get("to") in names)):
        out.append(E.tostring(e, encoding="unicode").strip())
path = Path(__file__).parent / "graph-source.txt"
path.write_text("\n".join(out) + "\n", encoding="utf-8")
print(path, len(out) - 2, "state/link snippets")
