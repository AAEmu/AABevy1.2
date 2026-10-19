"""Builds the per-skill tables of docs/research/skill-functional.md from the skilltest runs.

usage: python tools/skilltest/report.py > data/extract/skilltest/tables.md
Reads data/extract/skilltest/{d_tree*,c_tree*,a_tree*,b_*}.jsonl (d: final pass, hostile target selected on dry ground; c: same before the CashPoint fix; a: friendly skills on self in the water).
"""
import glob
import json
import os
import sys

sys.path.insert(0, os.path.dirname(__file__))
from analyze import REFUSAL, events_summary, problems, sent  # noqa: E402

D = "data/extract/skilltest"
TREES = {1: "Battlerage", 2: "Witchcraft", 3: "Defense", 4: "Auramancy", 5: "Occultism", 6: "Archery", 7: "Sorcery", 8: "Shadowplay",
         9: "Songcraft", 10: "Vitalism"}


def load(pattern):
    out = {}
    for f in sorted(glob.glob(os.path.join(D, pattern))):
        for l in open(f, encoding="utf-8"):
            r = json.loads(l)
            out.setdefault(r["skill"], []).append(r)
    return out


D_, C, A, B = load("d_tree*.jsonl"), load("c_tree*.jsonl"), load("a_tree*.jsonl"), load("b_*.jsonl")
HERE = os.path.dirname(__file__)
NOTES = json.load(open(os.path.join(HERE, "notes.json"), encoding="utf-8")) if os.path.exists(os.path.join(HERE, "notes.json")) else {}


def best(sid):
    """The run to show: the final pass unless the server refused it (friendly skills with a hostile target), then the self/friendly pass."""
    runs = [r[0] for src in (D_, A, C) for r in [src.get(sid)] if r]
    for r in runs:
        if not r["refused"]:
            return r
    return runs[0]


def row(r):
    pr = problems(r)
    verdict = "PASS" if not pr else "CHECK: " + "; ".join(pr)
    note = NOTES.get(str(r["skill"]))
    if note:
        verdict = note
    return "| %d | %s | %s | %s | %s | %s | %s |" % (
        r["skill"], r["name"], sent(r), ("refused: " + ", ".join(REFUSAL.get(x, hex(x)) for x in r["refused"])) if r["refused"] else "accepted",
        events_summary(r), ("unhandled %s" % [hex(u[1]) for u in r["unhandled"]]) if r["unhandled"] else "none", verdict), verdict


HEAD = "| id | skill | client sent | server | events received | unhandled / failed | verdict |\n|---|---|---|---|---|---|---|"

if __name__ == "__main__":
    total = {}
    out = []
    for tree, name in TREES.items():
        src = D_ if any(r[0]["tree"] == tree for r in D_.values()) else C
        ids = []
        for srcs in (D_, C, A):
            for sid, rs in srcs.items():
                if rs[0]["tree"] == tree and sid not in ids:
                    ids.append(sid)
        # keep the tree's order of the run
        for f in ("d", "c"):
            fn = os.path.join(D, "%s_tree%d.jsonl" % (f, tree))
            if os.path.exists(fn):
                ids = [json.loads(l)["skill"] for l in open(fn, encoding="utf-8")]
                break
        rows = [row(best(sid)) for sid in ids]
        ok = sum(1 for _, v in rows if v.startswith("PASS"))
        total[tree] = (ok, len(rows))
        out.append("\n### %d. %s (%d of %d pass)\n\n%s" % (tree, name, ok, len(rows), HEAD))
        out.extend(r for r, _ in rows)
    out.append("\n### Default skills, emotes, ground casts\n\n" + HEAD)
    for sid, rs in B.items():
        out.append(row(rs[-1])[0])
    print("| tree | pass | skills |\n|---|---|---|")
    for t, (ok, n) in total.items():
        print("| %d %s | %d | %d |" % (t, TREES[t], ok, n))
    print("| total | %d | %d |" % (sum(o for o, _ in total.values()), sum(n for _, n in total.values())))
    print("\n".join(out))
