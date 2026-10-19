"""Automatic verdicts for skilltest runs.

usage: python tools/skilltest/analyze.py file.jsonl ...  [--md]
Prints one line per skill with the problems the packet counts show; --md prints the markdown table rows of the report.
"""
import json
import re
import sys

REFUSAL = {0x01: "failure", 0x02: "source died", 0x04: "target died", 0x07: "already casting", 0x08: "cooldown", 0x09: "no target",
           0x0A: "no health", 0x0B: "no mana", 0x0C: "obstacle", 0x0E: "too close", 0x0F: "too far", 0x10: "out of angle",
           0x11: "in combat", 0x12: "moving", 0x13: "stunned", 0x17: "silenced", 0x1E: "invalid skill", 0x1F: "ability inactive",
           0x20: "ability level", 0x22: "invalid target", 0x23: "invalid location", 0x2C: "not now", 0x2D: "no permission", 0x32: "requirements"}


def sent(r):
    m = re.match(r"sent (\w+) \{ ([^}]*) \}( \(mode (\w+)\))?", r["events"][0]) if r["events"] else None
    if not m:
        return "?"
    kind, body, _, mode = m.groups()
    fields = dict(x.split(": ", 1) for x in body.split(", ") if ": " in x)
    if kind == "UseSkillAt":
        pos = re.search(r"x: ([\d.]+), y: ([\d.]+)", body)
        return "ground (%.0f, %.0f)" % (float(pos.group(1)), float(pos.group(2))) if pos else "ground"
    t = fields.get("target_obj_id", "?")
    return "self" if t == "0" else "unit"


def problems(r, strict_effects=True):
    p = []
    if r["refused"]:
        p.append("refused: " + ", ".join(REFUSAL.get(x, hex(x)) for x in r["refused"]))
    elif r["fired"] + r["plot_events"] + r["started"] == 0:
        p.append("no answer")
    elif r["ended"] == 0 and r["stopped"] == 0:
        p.append("no end event")
    if r["casting_left"]:
        p.append("cast state stuck")
    if "DRIVER_STILL_CASTING" in r["counts"]:
        p.append("still casting after window")
    if r["unhandled"]:
        p.append("unhandled " + ",".join("%#x" % u[1] for u in r["unhandled"]))
    if r["decode_failed"]:
        p.append("decode failed")
    if r["server_errors"]:
        p.append("server error %s" % r["server_errors"])
    if not r["refused"] and strict_effects:
        eff = r["effects"]
        if "Damage" in eff and r["damage_n"] == 0:
            p.append("no damage seen")
        if "Heal" in eff and r["heal_n"] == 0:
            p.append("no heal seen")
        if r["buffs"] and not r["buffs_created"]:
            p.append("no buff seen")
    return p


def events_summary(r):
    c = r["counts"]
    parts = []
    if r["started"]:
        parts.append("Started")
    if r["fired"]:
        parts.append("Fired x%d" % r["fired"])
    if r["plot_events"]:
        parts.append("Plot x%d" % r["plot_events"])
    if r["ended"]:
        parts.append("Ended x%d" % r["ended"])
    if r["stopped"]:
        parts.append("Stopped x%d" % r["stopped"])
    if r["damage_n"]:
        parts.append("Dmg x%d (%d)" % (r["damage_n"], r["damage_total"]))
    if r["heal_n"]:
        parts.append("Heal x%d" % r["heal_n"])
    if r["buffs_created"]:
        parts.append("Buff+ x%d" % len(r["buffs_created"]))
    if r["buffs_removed"]:
        parts.append("Buff- x%d" % r["buffs_removed"])
    if r["resets"]:
        parts.append("CdReset")
    if r["cooldown_events"]:
        parts.append("Cooldowns x%d" % r["cooldown_events"])
    for k in ("BlinkUnit", "Teleported", "UnitDeath", "UnitsRemoved"):
        if c.get(k):
            parts.append(k)
    return ", ".join(parts)


if __name__ == "__main__":
    md = "--md" in sys.argv
    for f in [a for a in sys.argv[1:] if not a.startswith("--")]:
        for l in open(f, encoding="utf-8"):
            r = json.loads(l)
            pr = problems(r)
            if md:
                print("| %d | %s | %s | %s | %s | %s |" % (r["skill"], r["name"], sent(r), "refused" if r["refused"] else "accepted", events_summary(r), "; ".join(pr) or "ok"))
            else:
                print("%2d %6d %-24s %s" % (r["tree"], r["skill"], r["name"][:24], "; ".join(pr) or "ok"))
