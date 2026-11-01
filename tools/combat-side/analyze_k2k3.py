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
        "evidence_classes": {
            "client": ("the 1.2 client's own send, memory/UI byte or animation, with no server reply or "
                       "scripted step in the causal path"),
            "server-dependent": ("includes our server's reply (SCSkillStarted, SCSkillFired, SCCombatCleared, "
                                 "the auto-attack repeat timer, the range check) or a scripted step"),
            "inferred": ("not separable from our server's reply at localhost, or a one-sample ordering; "
                         "stated as inferred, never as a 1.2 rule"),
        },
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
                                                            if row["t_ms"] == first_attack), None),
                                "evidence_class": "server-dependent",
                                "evidence_note": ("the clip is our server's SCSkillFired.fire_anim_id; the "
                                                  "280 ms includes the driver, the client frame and our reply")},
                "swing_anim_starts_t_ms": starts,
                "swing_cadence_ms": cadence,
                "target_hp_zero_t_ms": hp_zero,
                "target_lost_t_ms": hp_gone,
                "stance_1_to_0_t_ms": stance_after_kill,
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
                "evidence_classes": {
                    "first_press.input_row": "client",
                    "first_press.first_attack_anim_t_ms": "server-dependent",
                    "first_press.delay_ms_from_before_stamp": "server-dependent",
                    "first_press.first_attack_state": "server-dependent",
                    "swing_anim_starts_t_ms": "server-dependent",
                    "swing_cadence_ms": "server-dependent",
                    "target_hp_zero_t_ms": "server-dependent",
                    "target_lost_t_ms": "server-dependent",
                    "stance_1_to_0_t_ms": "server-dependent",
                    "weapon_active_transitions": "client",
                    "no_target_press_t_ms": "client",
                    "repress.input": "client",
                    "repress.wait": "server-dependent",
                    "walk.input": "client",
                    "walk.run_state_t_ms": "client",
                    "walk.distance_at_110274": "client",
                    "walk.distance_at_113454": "client",
                    "walk.distance_at_114592": "client",
                    "walk.distance_at_115540": "client",
                    "out_of_range_press": "client",
                    "gcd_at_presses": "client",
                },
                "evidence_notes": {
                    "swing": ("one CSStartSkill 2 per press; our server schedules the repeat at the weapon "
                              "speed (2000 ms) and sends SCSkillFired"),
                    "target_lost": "our server's SCCombatCleared; the script cleared the target at t=58529",
                    "stance": ("the stance byte drops with the client's combat state after our server's "
                               "SCCombatCleared; it is not a client timer. Decompile rule: FUN_391d6a60 "
                               "(OnCombatCleared) clears the +0x3bba latch through vtable +0x48 = "
                               "FUN_39355ec0, and FUN_39355ee0 refreshes a 10 s deadline while the latch is "
                               "set (1b0066d4:docs/research/port-combat/side-by-side.md:52-68)"),
                    "repress": ("the re-press sent another CSStartSkill 2 (server_slice.log:24360); no "
                                "CSStopCasting; our server stopped the damage"),
                    "walk": ("the client sent no packet while walking; our server's auto_attack_tick skips "
                             "the swing when distance > max_range"),
                    "out_of_range": ("the client sent CSStartSkill 2 (server_slice.log:43513); our server "
                                     "accepted it as a repeat start and did nothing, so no clip followed"),
                    "gcd": ("who writes the pair (the request or our SCSkillStarted) is unknown at "
                            "localhost, so the skill-2 exemption is inferred"),
                },
                "removed": ("stance_exit_after_target_lost_ms and stance_exit_after_no_target_press_ms are "
                            "removed: they are not a client timer; see evidence_notes.stance"),
            },
            "answers": {
                "first_swing_delay": {
                    "evidence_class": "server-dependent",
                    "text": ("the first attack clip starts 280 ms after the key-press before-stamp (187 ms "
                             "after the driver returned); the clip is our server's SCSkillFired.fire_anim_id, "
                             "so the 280 ms includes the driver, the client frame and our reply. The first "
                             "damage after the clip starts is at t=11081 (995 ms into the clip, 1.28 s after "
                             "the press); the target was already 155 HP down at the press, so the pre-run "
                             "auto-attack state is not clean"),
                },
                "auto_repeat": {
                    "evidence_class": "server-dependent",
                    "text": ("the client sends one CSStartSkill 2 per press; our server schedules the repeat at "
                             "the weapon speed (2000 ms) and sends SCSkillFired, so the client attack clip "
                             "starts every ~2.0 s until the target dies. A re-press sends another CSStartSkill "
                             "2 (server_slice.log:24360), not a stop packet; our server stopped the damage"),
                },
                "stance_exit": {
                    "evidence_class": "server-dependent",
                    "text": ("stance_perm 1->0 at t=59322 and UnitCombatState false at t=59323, after the "
                             "script cleared the target (t=58529) and the no-target press (t=58709). The "
                             "former stance-exit delays are removed because they are not a 1.2 stance timer: "
                             "the script cleared the target, the no-target press sent nothing, and the byte "
                             "drops after our server's SCCombatCleared. Decompile rule (client): "
                             "FUN_391d6a60 (OnCombatCleared) clears "
                             "the +0x3bba latch through vtable +0x48 = FUN_39355ec0, and FUN_39355ee0 "
                             "refreshes a 10 s deadline while the latch is set "
                             "(1b0066d4:docs/research/port-combat/side-by-side.md:52-68)"),
                },
                "battle_start_end": {
                    "evidence_class": "server-dependent",
                    "text": ("the 'Combat Start' overlay is part of the first attack clip at t=10086 (the clip "
                             "is our server's SCSkillFired.fire_anim_id); no chat 'Battle End'; the client "
                             "combat state goes false at t=59323 and the stance byte 1->0 at t=59322, after "
                             "our server's SCCombatCleared"),
                },
                "repress": {
                    "evidence_class": "server-dependent",
                    "text": ("the re-press sent another CSStartSkill 2 (server_slice.log:24360) and no "
                             "CSStopCasting. Our server stopped the repeat/damage: target HP 3317 at t=67344 "
                             "-> 2454 at t=99332, then stable for the 25 s wait, with no attack clip after the "
                             "in-flight swings. The client facts are the send and the absent clips, not a "
                             "client toggle"),
                },
                "walk_away": {
                    "evidence_class": "server-dependent",
                    "text": ("walking switches to onehand_combat_RUN at t=113119; the client sent no packet and "
                             "our server's auto_attack_tick skips the swing when distance > max_range, so the "
                             "attack clips stop. The target stays at its spawn (distance 3.0 -> 10.5 m). The "
                             "post-walk press at t=117444 sent CSStartSkill 2 (server_slice.log:43513); our "
                             "server accepted it as a repeat start and did nothing, so no clip followed"),
                },
                "gcd_exempt": {
                    "evidence_class": "inferred",
                    "text": ("the auto-attack never changes the cooldown-manager pair (0/1000 before, during "
                             "and after the presses); skill 2 is the GCD-exempt action if the pair is written "
                             "at request. Who writes the pair (the request or our SCSkillStarted) is unknown, "
                             "so the exemption is inferred, not proven"),
                },
            },
        },
        "k3": {
            "recall": {
                "cast_start_t_ms": recall_start, "last_cast_row_t_ms": recall_last,
                "action_slot_1": recall_slot,
                "evidence_classes": {
                    "cast_start_t_ms": "client",
                    "last_cast_row_t_ms": "client",
                    "action_slot_1": "client",
                    "answer": "server-dependent",
                },
                "answer": {
                    "evidence_class": "server-dependent",
                    "text": ("the 10 s cast bar is present (total 10000), but that total is our "
                             "SCSkillStarted.real_cast_time_div10; the client UI only displays it. The slot "
                             "shows only the 1000 ms GCD during the cast and the 30 min skill cooldown is not "
                             "started at the request (client memory)"),
                },
            },
            "cd_10152": {
                "rows": cd10152,
                "evidence_class": "inferred",
                "note": ("the slot values are client memory; whether the cooldown starts at the request or at "
                         "our SCSkillStarted/SCSkillFired is not separable at localhost"),
            },
            "gcd_per_skill": {
                "evidence_class": "client",
                "note": ("cooldown-manager pair, 5 ms; buff 3024 is applied by our server; 18132's 2000 ms "
                         "producer is inferred (client plot or our SCSkillStarted)"),
                "12026": {"no_buff_ms": 1000, "buff_3024_ms": 500},
                "18132": {"no_buff_ms": 2000, "buff_3024_ms": 1000,
                          "producer_class": "inferred",
                          "note": "the memory global is 2000 ms; the action slot shows a 500 ms custom cooldown"},
                "11991": {"no_buff_ms": 1000, "buff_3024_ms": 500, "cast_ms": 1500},
                "10752": {"no_buff_ms": 1000, "cast_ms": 1000},
                "10455": {"no_buff_ms": 500,
                          "note": "ignore_global_cooldown is not a zero GCD; the client uses a 500 ms GCD"},
                "10152": {"no_buff_ms": 1000, "cooldown_ms": 30000},
                "10594": {"no_buff_ms": 1000, "cast_ms": 10000, "cooldown_ms": 1800000},
                "2": {"no_buff_ms": 0, "note": "auto-attack is GCD-exempt (inferred)"},
            },
            "second_press": {
                "during_cast_11991": {
                    "evidence_class": "server-dependent",
                    "text": ("12026 at t=177900 during the Health Lift cast: 'Already performing an action.' "
                             "at t=177952; no StartSkill; the cast runs to its end. The cast state itself came "
                             "from our SCSkillStarted"),
                },
                "during_gcd_18132": {
                    "evidence_class": "client",
                    "text": ("the same skill at t=34029, about 1.68 s left by the memory GCD (the report's "
                             "550 ms uses press stamps): silently dropped (no text, no new GCD, no StartSkill)"),
                },
                "during_gcd_frenzy": {
                    "evidence_class": "server-dependent",
                    "text": ("18132 at t=40659, about 0.23 s left by the memory GCD (Frenzy rise ~40389, end "
                             "~40889): queued; a new 2000 ms GCD starts at t=41132, about the normal "
                             "press->rise lag, and our server logs StartSkill 18132 at 10:36:26"),
                },
                "after_gcd": {
                    "evidence_class": "client",
                    "text": "18132 at t=36859 after the GCD is accepted",
                },
                "rule": {
                    "evidence_class": "inferred",
                    "text": ("a second press during a cast is refused. During a GCD, one queued sample "
                             "(~0.23 s left) and one dropped sample (~1.68 s left) bound the window only "
                             "between about 0.23 s and 1.68 s; 0.25 s is not measured. The queued skill's new "
                             "GCD starts about 0.2 s after the current GCD ends, the same press->rise lag as "
                             "a normal press, so that delay is our reply/pipeline latency, not a queue rule"),
                },
            },
            "refusals": {
                "run": refusals,
                "run3": refusals3,
                "texts_class": "client",
                "order_class": "inferred",
                "order": ["cast in progress ('Already performing an action.')",
                          "combat restriction ('Can't be used in combat.')",
                          "skill cooldown ('Can't be used right now.')",
                          "GCD: silent buffer/drop (no text)",
                          "range (probe only, not recorded: 'Target is too far.' for a skill; "
                          "'Invalid target.' for auto-attack)",
                          "dead (run2 only, invalid run: 'Can't be used while dead.')"],
                "order_note": ("no press met two conditions at once; this is a list of separate observations, "
                               "not an observed priority"),
                "sound": ("not captured: the lane starts the client with -aaemu-silent and s_SoundEnable 0; "
                          "the X2Sound hook saw no PlayUISound calls"),
            },
            "cooldown_display": {
                "instant_10152": {
                    "evidence_class": "inferred",
                    "text": ("30000,30000 at t=46057 (+178 ms after the t=45879 press), then 29816 at "
                             "t=46247; the slot starts within 178 ms, but the request and our "
                             "SCSkillStarted/SCSkillFired are within about 1 ms of each other, so 'at "
                             "request' is not separable from the reply"),
                },
                "instant_12026": {
                    "evidence_class": "inferred",
                    "text": ("21000,14073 at t=222058; the countdown matches the t=215083 press, but the "
                             "request, start and fire are within about 1 ms, so 'at request' is inferred"),
                },
                "cast_10594": {
                    "evidence_class": "server-dependent",
                    "text": ("no 1800000 during the 10 s cast; the slot shows only 1000,858 -> 1000,0; the "
                             "cast length is our SCSkillStarted and the fire start was not observed "
                             "(cancelled at 8.3 s to avoid the zone change)"),
                },
            },
            "missing_4s_cast": {
                "evidence_class": "client",
                "text": ("the 3.5 s Rampage (14166) is learned server-side but not in the client's general "
                         "skill book: SetActionSpell(9,14166) left the slot empty (9:None:?:true at "
                         "t=128608). The 4 s skills 11347/14679 are also general skills not in the book. No "
                         "~4 s cast is available on this character without a new character/learn path"),
            },
        },
        "verification": {
            "K2_first_swing": ("server-dependent (the clip is our server's SCSkillFired.fire_anim_id; the "
                               "pre-press target damage also makes the exact start state ambiguous)"),
            "K2_toggle_repeat_walk": ("server-dependent (our repeat timer and range check; the client sends "
                                      "one CSStartSkill 2 per press and nothing while walking)"),
            "K2_stance_exit": ("server-dependent timing + client decompile rule (the stance clears on "
                               "OnCombatCleared; the 10 s deadline is refreshed while engaged). The former "
                               "stance-exit delays are removed; they are not a client timer"),
            "K3_gcd": ("client (cooldown-manager pair, 5 ms); the 2000 ms for 18132 is client-observed but "
                       "its producer (plot vs our SCSkillStarted) is inferred"),
            "K3_buff": "client (the client halves the GCD); buff 3024 is applied by our server",
            "K3_cast_bar": ("server-dependent durations (our SCSkillStarted.real_cast_time_div10); the UI "
                            "frame is client"),
            "K3_cooldown": ("inferred (the action slot GetActionCooldownEx; the request and our "
                            "SCSkillStarted/SCSkillFired are within about 1 ms, so instant start cannot be "
                            "told apart from fire)"),
            "K3_second_press": ("client texts/pair + server-dependent cast state and repeat; the queue "
                                "window and its ~0.2 s latency are inferred"),
            "K3_refusals": "client for the recorded texts; the order is inferred; range texts are probe-only",
        },
        "caveats": [
            "Our server's replies (SCCombatCleared, SCSkillStarted, SCSkillFired, damage ticks, refusals) are "
            "stimulus, not 1.2 server evidence.",
            "The character died before run2 (the gap while the runs were analysed); run2 is invalid and kept "
            "only for the 'Can't be used while dead.' row.",
            "The first K2 target was already 155 HP down at the find; the auto-attack may have been on from "
            "the pre-run probe.",
            "The K2 re-press sent another CSStartSkill 2 (no stop packet) and our server stopped the damage; "
            "a client auto-attack toggle is not proven.",
            "The K2 no-target press sent nothing; the script cleared the target and our server sent "
            "SCCombatCleared.",
            "The K2 out-of-range press sent CSStartSkill 2; our server accepted it as a repeat start and did "
            "nothing.",
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
