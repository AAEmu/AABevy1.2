#!/usr/bin/env python3
"""Summarise the K2 (auto-attack) and K3 (cast/GCD) recorder runs into native.json.

The raw runs are the recorder's per-channel JSONL files (they stay outside the
repository).  This tool only reads them and writes a text/JSON summary; it does
not start or drive the client.

Usage:
    python tools/combat-side/analyze_k2k3.py \
        --capture-root <capture-root>/2026-11-01-combat-k2k3 \
        --out tools/evidence/combat_core/native.json
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def load(run: Path, channel: str) -> list[dict]:
    path = run / f"{channel}.jsonl"
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def row_at(rows: list[dict], t: float, key: str = "t_ms") -> dict:
    best = rows[0] if rows else {}
    for row in rows:
        if row.get(key, 0) <= t:
            best = row
        else:
            break
    return best


def transitions(rows: list[dict], field: str) -> list[tuple[float, object, object]]:
    out: list[tuple[float, object, object]] = []
    previous = None
    for row in rows:
        if field not in row:
            continue
        value = row[field]
        if previous is not None and value != previous:
            out.append((row["t_ms"], previous, value))
        previous = value
    return out


def attack_starts(rows: list[dict]) -> list[float]:
    """The first row of each maximal run of attack-clip states (one per swing)."""
    out: list[float] = []
    inside = False
    for row in rows:
        attack = str(row.get("player_state", "")).startswith("onehand_co_attack")
        if attack and not inside:
            out.append(row["t_ms"])
        inside = attack
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--capture-root", type=Path, required=True,
                        help="the 2026-11-01-combat-k2k3 directory with run/ and run3/")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = args.capture_root
    run = root / "run"
    run2 = root / "run2"
    run3 = root / "run3"

    inputs = load(run, "inputs")
    gcd = load(run, "gcd")
    castbar = load(run, "castbar")
    anim = load(run, "anim_state")
    actions = load(run, "actions")
    chat = load(run, "chat")
    steps = load(run, "steps")

    first_press = next((row for row in inputs if row["seq"] == 0), None)
    first_attack = next((row["t_ms"] for row in anim
                         if row["t_ms"] > (first_press or {}).get("t_ms", 0)
                         and str(row.get("player_state", "")).startswith("onehand_co_attack")), None)
    starts = [t for t in attack_starts(anim) if 9000 <= t <= 47000]
    cadence = [round(b - a, 1) for a, b in zip(starts, starts[1:])]

    hp_zero = next((row["t_ms"] for row in castbar if row.get("target_health") == "0"), None)
    hp_gone = next((row["t_ms"] for row in castbar if hp_zero is not None
                    and row["t_ms"] > hp_zero and row.get("target_health") == ""), None)
    stance = [t for t, old, new in transitions(gcd, "stance_perm") if old == 1 and new == 0]
    stance_after_kill = next((t for t in stance if hp_gone is not None and t > hp_gone), None)
    weapon = transitions(gcd, "weapon_active")
    no_target_press = next((row["t_ms"] for row in inputs
                            if row.get("note") == "auto-attack with no target"), None)

    repress = next((row for row in inputs if row.get("note") == "re-press mid-fight"), None)
    repress_wait = next((row for row in steps if row.get("op") == "wait_target_dead"
                         and "re-press" in str(row.get("note", ""))), None)
    walk_press = next((row for row in inputs if row.get("note") == "walk away mid-fight"), None)
    out_of_range = next((row for row in inputs if row.get("note") == "auto-attack out of range"), None)
    run_state = next((row["t_ms"] for row in anim if "combat_RUN" in str(row.get("player_state", ""))), None)

    def distance_at(t: float) -> float | None:
        row = row_at(anim, t)
        try:
            player = [float(v) for v in row["player_pos"].split(",")]
            victim = [float(v) for v in row["victim_pos"].split(",")]
            return round(((player[0] - victim[0]) ** 2 + (player[1] - victim[1]) ** 2) ** 0.5, 2)
        except Exception:
            return None

    self_run = json.loads((run / "self_check.json").read_text(encoding="utf-8"))
    self_run3 = (json.loads((run3 / "self_check.json").read_text(encoding="utf-8"))
                 if (run3 / "self_check.json").exists() else {})
    self_run2 = (json.loads((run2 / "self_check.json").read_text(encoding="utf-8"))
                 if (run2 / "self_check.json").exists() else {})

    inputs3 = load(run3, "inputs")
    gcd3 = load(run3, "gcd")
    cast3 = load(run3, "castbar")
    act3 = load(run3, "actions")
    chat3 = load(run3, "chat")

    recall_rows = [row for row in cast3 if row.get("player_name") == "Recall"]
    recall_start = recall_rows[0]["t_ms"] if recall_rows else None
    recall_last = recall_rows[-1]["t_ms"] if recall_rows else None
    recall_pre = row_at(act3, (recall_start or 0) + 100)
    recall_mid = row_at(act3, (recall_start or 0) + 1000)
    recall_slot = {
        "at_start": recall_pre.get("slots", {}).get("1", {}),
        "after_1s": recall_mid.get("slots", {}).get("1", {}),
    }
    cd10152 = [{"t_ms": row.get("t_ms"),
                "cd_ex": row.get("slots", {}).get("4", {}).get("cd_ex")}
               for row in act3 if 45500 <= row["t_ms"] <= 47500 and row["seq"] % 20 == 0]

    def auto_press_gcd(seq: int) -> dict:
        press = next((row for row in inputs if row["seq"] == seq), None)
        if press is None:
            return {}
        around = [row for row in gcd if press["t_ms"] - 500 <= row["t_ms"] <= press["t_ms"] + 2500
                  and "gcd_remaining" in row]
        remaining = max((row["gcd_remaining"] for row in around), default=None)
        return {"press_t_ms": press["t_ms"],
                "max_total": max((row["gcd_total"] for row in around), default=None),
                "max_remaining": remaining,
                "gcd_rise": bool(remaining and remaining > 0)}

    refusals = [{"t_ms": row["t_ms"], "text": row.get("text")}
                for row in chat if row.get("likely_refusal")]
    refusals3 = [{"t_ms": row["t_ms"], "text": row.get("text")}
                 for row in chat3 if row.get("likely_refusal")]

    native = {
        "schema": "aabevy.combat-core-native/v1",
        "date": "2026-11-01",
        "client": ("ArcheAge 1.2 r208022, pid 77088, hidden; account clienttest, "
                   "character Combatt1 (level 45, Battlerage/Sorcery/Auramancy, "
                   "one-hand sword 21377)"),
        "server": "AAEmu lane combatcap (our server replies are stimulus, not 1.2 evidence)",
        "capture_root": "<capture-root>/2026-11-01-combat-k2k3",
        "runs": {
            "run": {"status": "ok", "self_check": self_run.get("ok"),
                    "channels": {k: len(load(run, k)) for k in
                                 ("gcd", "anim_state", "castbar", "actions", "inputs", "steps", "chat")},
                    "memory_ui": {k: self_run.get("memory_ui", {}).get(k) for k in
                                  ("status", "offset_ms", "spread_ms", "pairs", "residual_max_ms")}},
            "run3": {"status": "ok", "self_check": self_run3.get("ok"),
                     "channels": {k: len(load(run3, k)) for k in
                                  ("gcd", "anim_state", "castbar", "actions", "inputs", "steps", "chat")},
                     "memory_ui": {k: self_run3.get("memory_ui", {}).get(k) for k in
                                   ("status", "offset_ms", "spread_ms", "pairs", "residual_max_ms")}},
            "run2": {"status": "invalid",
                     "reason": "the character was dead; every skill answered 'Can't be used while dead.'",
                     "self_check": self_run2.get("ok")},
        },
        "k2": {
            "auto_attack": {
                "key": "3", "slot": 3, "skill_id": 2,
                "first_press": {"input_row": first_press,
                                "first_attack_anim_t_ms": first_attack,
                                "delay_ms_from_before_stamp": (None if not first_press or first_attack is None
                                                              else first_attack - first_press["t_ms"]),
                                "first_attack_state": next((row.get("player_state") for row in anim
                                                            if row["t_ms"] == first_attack), None)},
                "swing_anim_starts_t_ms": starts,
                "swing_cadence_ms": cadence,
                "target_hp_zero_t_ms": hp_zero,
                "target_lost_t_ms": hp_gone,
                "stance_1_to_0_t_ms": stance_after_kill,
                "stance_exit_after_target_lost_ms": (None if not stance_after_kill or not hp_gone
                                                     else stance_after_kill - hp_gone),
                "stance_exit_after_no_target_press_ms": (None if not stance_after_kill or not no_target_press
                                                         else stance_after_kill - no_target_press),
                "weapon_active_transitions": weapon,
                "no_target_press_t_ms": no_target_press,
                "repress": {"input": repress, "wait": repress_wait},
                "walk": {"input": walk_press, "run_state_t_ms": run_state,
                         "distance_at_110274": distance_at(110274),
                         "distance_at_113454": distance_at(113454),
                         "distance_at_114592": distance_at(114592),
                         "distance_at_115540": distance_at(115540)},
                "out_of_range_press": out_of_range,
                "gcd_at_presses": [auto_press_gcd(seq) for seq in (0, 2, 3, 4, 6)],
            },
            "answers": {
                "first_swing_delay": ("the first attack clip starts 280 ms after the key-press before-stamp "
                                      "(187 ms after the driver returned); the first damage of that swing is "
                                      "at 1.08 s; the target was already 155 HP down at the press, so the "
                                      "pre-run auto-attack state is not clean"),
                "auto_repeat": ("one press repeats the swing (client attack clip every ~2.0 s) until the target "
                                "dies; the re-press toggles it off after in-flight swings"),
                "stance_exit": ("stance_perm 1->0 0.77 s after the target/corpse was lost and 0.61 s after the "
                                "fight-ending no-target press; UnitCombatState false at the same frame. The "
                                "13.6 s from the last hit is the corpse/target lifetime, not a client 10 s timer"),
                "battle_start_end": ("the 'Combat Start' overlay is part of the first attack clip at t=10086; "
                                     "no chat 'Battle End'; the client combat state goes false at t=59323 and "
                                     "the stance byte 1->0 at t=59322"),
                "repress": ("toggles the auto-attack off: target HP 3317 at t=67344 -> 2454 at t=99332, then "
                            "stable for the 25 s wait; no attack clip after the in-flight swings"),
                "walk_away": ("walking switches to onehand_combat_RUN at t=113119 and stops the attack clips; "
                              "the target stays at its spawn (distance 3.0 -> 10.5 m); the post-walk press at "
                              "t=117394 produces no clip and no StartSkill"),
                "gcd_exempt": ("the auto-attack never changes the cooldown-manager pair (0/1000 before, during "
                               "and after the presses); skill 2 is the GCD-exempt action"),
            },
        },
        "k3": {
            "recall": {"cast_start_t_ms": recall_start, "last_cast_row_t_ms": recall_last,
                       "action_slot_1": recall_slot,
                       "answer": ("the 10 s cast bar is present (total 10000); only the 1000 ms GCD is on the "
                                  "slot during the cast, the 30 min skill cooldown is not started at the "
                                  "request")},
            "cd_10152": {"rows": cd10152},
            "gcd_per_skill": {
                "12026": {"no_buff_ms": 1000, "buff_3024_ms": 500},
                "18132": {"no_buff_ms": 2000, "buff_3024_ms": 1000,
                          "note": "the memory global is 2000 ms; the action slot shows a 500 ms custom cooldown"},
                "11991": {"no_buff_ms": 1000, "buff_3024_ms": 500, "cast_ms": 1500},
                "10752": {"no_buff_ms": 1000, "cast_ms": 1000},
                "10455": {"no_buff_ms": 500,
                          "note": "ignore_global_cooldown is not a zero GCD; the client uses a 500 ms GCD"},
                "10152": {"no_buff_ms": 1000, "cooldown_ms": 30000},
                "10594": {"no_buff_ms": 1000, "cast_ms": 10000, "cooldown_ms": 1800000},
                "2": {"no_buff_ms": 0, "note": "auto-attack is GCD-exempt"},
            },
            "second_press": {
                "during_cast_11991": ("12026 at t=177900 during the Health Lift cast: 'Already performing an "
                                      "action.' at t=177952; no StartSkill; the cast runs to its end"),
                "during_gcd_18132": ("the same skill at t=34029, 550 ms into a 2000 ms GCD: silently dropped "
                                     "(no text, no new GCD, no StartSkill)"),
                "during_gcd_frenzy": ("18132 at t=40659, 420 ms into Frenzy's 500 ms GCD: queued; a new "
                                      "2000 ms GCD starts at t=41132 and the server logs StartSkill 18132 "
                                      "at 10:36:26"),
                "after_gcd": "18132 at t=36859 after the GCD is accepted",
                "rule": ("a second press is buffered only when the current GCD ends within about 0.25 s; "
                         "otherwise it is dropped; during a cast it is refused"),
            },
            "refusals": {
                "run": refusals,
                "run3": refusals3,
                "order": ["cast in progress ('Already performing an action.')",
                          "combat restriction ('Can't be used in combat.')",
                          "skill cooldown ('Can't be used right now.')",
                          "GCD: silent buffer/drop (no text)",
                          "range (probe only, not recorded: 'Target is too far.' for a skill; "
                          "'Invalid target.' for auto-attack)",
                          "dead (run2 only, invalid run: 'Can't be used while dead.')"],
                "sound": ("not captured: the lane starts the client with -aaemu-silent and s_SoundEnable 0; "
                          "the X2Sound hook saw no PlayUISound calls"),
            },
            "cooldown_display": {
                "instant_10152": ("30000,30000 at t=46057 (+178 ms after the t=45879 press), then 29816 at "
                                  "t=46247 -> starts at the request"),
                "instant_12026": ("21000,14073 at t=222058; the countdown matches the t=215083 press -> "
                                  "starts at the request"),
                "cast_10594": ("no 1800000 during the 10 s cast; the slot shows only 1000,858 -> 1000,0; the "
                               "fire start was not observed (cancelled at 8.3 s to avoid the zone change)"),
            },
            "missing_4s_cast": ("the 3.5 s Rampage (14166) is learned server-side but not in the client's "
                                "general skill book: SetActionSpell(9,14166) left the slot empty "
                                "(9:None:?:true at t=128608). The 4 s skills 11347/14679 are also general "
                                "skills not in the book. No ~4 s cast is available on this character without a "
                                "new character/learn path"),
        },
        "verification": {
            "K2_first_swing": ("CAP (client animation + input bracket; the pre-press target damage makes the "
                               "exact start state ambiguous)"),
            "K2_toggle_repeat_walk": "CAP (client animation/HP; the server's damage cadence differs)",
            "K2_stance_exit": ("CAP (client stance byte; the 10 s/13.6 s corpse and combat linger are our "
                               "server's behaviour)"),
            "K3_gcd": ("CAP (cooldown-manager pair, 5 ms); the 2000 ms for 18132 is client-observed but its "
                       "producer (plot vs server SCSkillStarted) is not proven"),
            "K3_buff": "CAP (the client halves the GCD after the server adds buff 3024)",
            "K3_cast_bar": "CAP (UnitCastingInfo; UI frame 52 ms)",
            "K3_cooldown": ("CAP (action slot GetActionCooldownEx; instant at request, cast start not at "
                            "request)"),
            "K3_second_press": "CAP (chat + GCD + server StartSkill trace)",
            "K3_refusals": "CAP for the recorded texts; range texts are probe-only",
        },
        "caveats": [
            "Our server's replies (SCCombatCleared, SCSkillStarted, damage ticks, refusals) are stimulus, "
            "not 1.2 server evidence.",
            "The character died before run2 (the gap while the runs were analysed); run2 is invalid and kept "
            "only for the 'Can't be used while dead.' row.",
            "The first K2 target was already 155 HP down at the find; the auto-attack may have been on from "
            "the pre-run probe. The toggle conclusion rests on the re-press and no-target press.",
            "The weapon byte weapon_active stayed 1 after the first draw; no sheathe was observed. The stance "
            "evidence is the stance_perm byte.",
            "The client's system clock drifted 813 ppm in run; the UI clock is the stable reference. "
            "Input-to-UI comparisons are within the 52 ms frame plus the clock residual.",
        ],
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(native, indent=2, ensure_ascii=False), encoding="utf-8")
    print("wrote", args.out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
