#!/usr/bin/env python3
"""Replay natural gathering in the Bevy client and compare it with the 1.2 capture.

This is the comparison scaffold for the bundled doodad capture (see
`tools/evidence/gather/capture-plan.txt`).  It runs
`tools/evidence/gather/replay.txt` with `aa-client --headless --mute`, records
the server's C->S / S->C field trace for the run, and reduces it to one record
per gathering skill:

    skill, caster/target object ids, CSStartSkill bytes, SCSkillStarted cast
    time, target kind and refusal code, SCDoodadPhaseChanged groups,
    SCDoodadRemoved, SCItemTaskSuccess deliveries (template id, amount,
    grade), SCCharacterLaborPowerChanged deltas, SCErrorMsg codes

The labor, item and error rows belong to the cast that is running (see
`reduce_packets`), not to every cast before it.

When `tools/evidence/gather/native.json` exists (the 1.2 capture, written by
the capture session) the same fields are compared (a field the native case does
not list is skipped) and a MATCH / DIFFERS table is appended to
`tools/evidence/gather/report.txt`.  Without it the run writes `bevy.json` and a
report that says the native capture is still missing; it does not invent one.

The refusal code is compared as `skill_started.result` (0 = the cast started).
Item deliveries are compared as one total per template id and grade, never as
packet bytes: bag slots and item db ids differ from run to run (see
`item_deliveries`).

The game server must log the fields trace:
`RUST_LOG=info,aaemu_game::net::fields=trace`.

Usage:
    python tools/gather_replay.py --client <aa-client.exe> \
        --login <host:port> --account <account> --character <character> \
        --password-file <password-file> --server-log <game-server.log> \
        [--evidence tools/evidence/gather] [--raw <dir>]
    python tools/gather_replay.py --self-test    (reducer and comparison on synthetic packets)

Offline rule: this script never starts a server or the original client.  It
runs the Bevy binary against an already running lane; the capture itself is a
later, separate session (capture plan).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import struct
import subprocess
import sys
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PACKET_RE = re.compile(
    r"\[(?:TRACE|DEBUG|INFO)\] fields - (?P<dir>[CS]->[SC]) (?:level=\d+ )?opcode=(?P<opcode>0x[0-9A-Fa-f]+) "
    r"(?P<name>\w+) len=(?P<len>\d+) body=(?P<body>[0-9a-fA-F ]*)"
)
OPCODES = {
    "0x052": "CSStartSkill",
    "0x0a1": "SCSkillStarted",
    "0x0a2": "SCSkillFired",
    "0x0a3": "SCSkillEnded",
    "0x10f": "SCDoodadPhaseChanged",
    "0x10d": "SCDoodadRemoved",
    "0x090": "SCItemTaskSuccess",
    "0x045": "SCCharacterLaborPowerChanged",
    "0x10a": "SCErrorMsg",
}
GATHER_SKILLS = {11106, 11107, 11108, 11642, 13802, 13970, 13990, 15874, 16771, 16774}


def start_offset(path: Path) -> int:
    try:
        return path.stat().st_size
    except FileNotFoundError:
        return 0


def read_new(path: Path, offset: int) -> tuple[str, int]:
    if not path.exists():
        return "", offset
    with path.open("rb") as f:
        f.seek(offset)
        data = f.read()
    return data.decode("utf-8", "replace"), offset + len(data)


def bytes_of(text: str) -> bytes:
    return bytes.fromhex(text.replace(" ", ""))


def u32(body: bytes, at: int) -> int:
    return int.from_bytes(body[at:at + 4], "little")


def bc(body: bytes, at: int) -> tuple[int, int]:
    return int.from_bytes(body[at:at + 3], "little"), at + 3


INVENTORY = 2  # aa-net `container::INVENTORY` (items.rs:109): the bag
# The tasks of an SCItemTaskSuccess that `item_deliveries` only walks past: the fields behind the action byte as a struct
# format, as `ItemTask::read` (aa-net items.rs:401) reads them (`x` = skipped bytes).  Not listed: ADD_STACK (4), CREATE (5),
# TAKE (6), CHANGE_OWNER (15) and UPDATE_DETAIL (9), which `item_deliveries` reads itself.
OTHER_TASKS = {
    1: "i",  # CHANGE_MONEY_AMOUNT: amount
    2: "i",  # CHANGE_BANK_MONEY_AMOUNT: amount
    3: "Bi",  # CHANGE_GAME_POINT: kind, amount
    7: "BBQI",  # REMOVE: container, slot, item id, template
    8: "BBBBQQ4x",  # SWAP_SLOT: from and to container + slot, both item ids, 4 bytes of flags
    10: "BBQB8x",  # SET_FLAGS_BITS: container, slot, item id, bits, 8 skipped bytes
    11: "BBQBBBBqq",  # UPDATE_FLAGS: container, slot, item id, bits, three flags, unsecure and unpack time
    12: "Q",  # REMOVE_CRAFTING: item id
    13: "BBQ",  # SEIZE: container, slot, item id
    14: "BBQB",  # CHANGE_GRADE: container, slot, item id, grade
    16: "i",  # CHANGE_AA_POINT: amount
    17: "i",  # CHANGE_BANK_AA_POINT: amount
    18: "B",  # CHANGE_AUTO_USE_AA_POINT: flag
}


def item_deliveries(body: bytes) -> list[dict]:
    """The items one SCItemTaskSuccess puts into the bag, as {template_id, amount, grade} in packet order.

    The layout is the one aa-net decodes: `ItemTasks::read` (crates/aa-net/src/items.rs:378) reads the reason u8, the task
    count u8 and the tasks, `ItemTask::read` (items.rs:401) one task by its action byte, `read_task_item_with_raw`
    (items.rs:498) the item of a CREATE, TAKE or CHANGE_OWNER.  A delivery is such an item added to the bag or an ADD_STACK
    with a positive delta: the two shapes `route_items` (aa-client hud/items.rs:60-72) reports as ADDED_ITEM.  The bag
    slot and the item db id are not part of it: they differ from run to run.  An ADD_STACK names no grade (the stack keeps
    its own) and counts as grade 0, what `route_items` assumes for a stack it does not know; unverified, because no stack
    add was ever captured.  Like `ItemTasks::read`, an action it does not know, or a body that ends inside a task, ends the
    list and keeps the deliveries before it.
    """
    deliveries: list[dict] = []
    at = 0

    def take(fmt: str) -> tuple:  # aa-net's `Reader`: little endian, an error past the end of the body
        nonlocal at
        values = struct.unpack_from("<" + fmt, body, at)
        at += struct.calcsize("<" + fmt)
        return values

    try:
        _reason, tasks = take("BB")
        for _ in range(tasks):
            (action,) = take("B")
            if action == 4:  # ADD_STACK
                container, _slot, _item_id, delta, template = take("BBQiI")
                if container == INVENTORY and delta > 0:
                    deliveries.append({"template_id": template, "amount": delta, "grade": 0})
            elif action in (5, 6, 15):  # CREATE, TAKE, CHANGE_OWNER: container, slot, then the task item
                container, _slot, template, _item_id, grade, _flags, count, block = take("BBIQBBih")
                take(f"{max(block, 0)}x")  # the padded detail block (items.rs:475)
                take("qiIBqq")  # create time, lifespan, made unit, world, unsecure and unpack time
                if container == INVENTORY:
                    deliveries.append({"template_id": template, "amount": count, "grade": grade})
            elif action == 9:  # UPDATE_DETAIL: container, slot, item id, the padded detail block
                *_, block = take("BBQh")
                take(f"{max(block, 0)}x")
            elif action in OTHER_TASKS:
                take(OTHER_TASKS[action])
            else:
                break
    except struct.error:
        pass
    return deliveries


def delivered(deliveries: list[dict]) -> list[dict]:
    """One total per (template id, grade), sorted: whether the bag took the same items as new stacks or as stack adds
    is not part of what a gathering delivers."""
    totals: dict[tuple[int, int], int] = {}
    for item in deliveries:
        key = (item["template_id"], item["grade"])
        totals[key] = totals.get(key, 0) + item["amount"]
    return [{"template_id": template, "amount": amount, "grade": grade}
            for (template, grade), amount in sorted(totals.items())]


def reduce_packets(lines: str) -> list[dict]:
    """One record per CSStartSkill cast, with the answers that name the same skill.

    Labor changes, errors and item deliveries name no skill: they belong to the cast that is running, from its
    CSStartSkill until its SCSkillEnded, a refusal without a timeline (no SCSkillEnded follows) or the next
    CSStartSkill.  The server's `end_skill` sends SCSkillEnded after the labor change and the 2026-10-30 herb run had
    its BAG_FULL before it; that an item delivery also comes before it is unverified (never captured: the bag was
    full).  Anything outside a cast is dropped.  Phase changes and removals follow the doodad id instead: a depletion
    can come after the cast.
    """
    casts: list[dict] = []
    by_skill: dict[int, dict] = {}
    active: dict | None = None  # the cast that owns the answers which name no skill
    active_tl: int | None = None  # its timeline, from SCSkillStarted
    for line in lines.splitlines():
        m = PACKET_RE.search(line)
        if not m:
            continue
        opcode = int(m.group("opcode"), 16)
        name = OPCODES.get(m.group("opcode"))
        body = bytes_of(m.group("body"))
        if name == "CSStartSkill" and len(body) >= 13:
            skill = u32(body, 0)
            caster, at = bc(body, 5)
            target_kind = body[at] if at < len(body) else -1
            target, _ = bc(body, at + 1)
            record = {
                "skill": skill,
                "caster_obj_id": caster,
                "target_kind": target_kind,
                "doodad_obj_id": target,
                "cs_start_skill": m.group("body").strip(),
                "skill_started": None,
                "skill_fired": 0,
                "phase_changes": [],
                "removed": False,
                "item_tasks": [],
                "labor": [],
                "errors": [],
            }
            casts.append(record)
            by_skill[skill] = record
            active, active_tl = record, None
        elif name == "SCSkillStarted" and len(body) >= 21:
            # skill u32, tl u16, caster kind+bc (4), target kind+bc (4), skill object u8, cast u16*10, base cast u16*10,
            # synergy u8, then the SkillResult (aa-net skill.rs): flags u8, and the failure code u8 when bit 0 is set
            skill = u32(body, 0)
            record = by_skill.get(skill)
            if record is not None:
                flags = body[20]
                record["skill_started"] = {
                    "target_kind": body[10],
                    "cast_ms": int.from_bytes(body[15:17], "little") * 10,
                    "result": body[21] if flags & 1 and len(body) > 21 else 0,
                }
                if record is active:
                    tl = int.from_bytes(body[4:6], "little")
                    if tl:
                        active_tl = tl
                    else:
                        active = None  # a refusal at the start has no timeline: no SCSkillEnded follows
        elif name == "SCSkillEnded" and len(body) >= 2:
            if active is not None and int.from_bytes(body[0:2], "little") == active_tl:
                active = None
        elif name == "SCSkillFired":
            skill = u32(body, 0)
            if skill in by_skill:
                by_skill[skill]["skill_fired"] += 1
        elif name == "SCDoodadPhaseChanged" and len(body) >= 8:
            obj_id, at = bc(body, 0)
            group = u32(body, at)
            for record in by_skill.values():
                if record["doodad_obj_id"] == obj_id:
                    record["phase_changes"].append(group)
        elif name == "SCDoodadRemoved" and len(body) >= 3:
            removed, _ = bc(body, 0)
            for record in by_skill.values():
                if record["doodad_obj_id"] == removed:
                    record["removed"] = True
        elif name == "SCCharacterLaborPowerChanged" and len(body) >= 4:
            if active is not None:
                active["labor"].append(int.from_bytes(body[0:4], "little", signed=True))
        elif name == "SCErrorMsg" and len(body) >= 2:
            code = int.from_bytes(body[0:2], "little", signed=True)
            if active is not None and (not active["errors"] or active["errors"][-1] != code):
                active["errors"].append(code)
        elif name == "SCItemTaskSuccess":
            if active is not None:
                active["item_tasks"].extend(item_deliveries(body))
    return [c for c in casts if c["skill"] in GATHER_SKILLS]


def compare(native: list[dict], bevy: list[dict]) -> list[dict]:
    """Match each captured case with the next Bevy cast of its skill (repeats stay separate)."""
    rows = []
    used: set[int] = set()
    for expected in native:
        skill = expected.get("skill")
        index = next((i for i, case in enumerate(bevy) if i not in used and case["skill"] == skill), None)
        if index is None:
            rows.append({"skill": skill, "status": "NOT RUN"})
            continue
        used.add(index)
        case = bevy[index]
        want_started, started = expected.get("skill_started") or {}, case.get("skill_started") or {}
        want_items = expected.get("item_tasks")
        fields = {
            "target_kind": (expected.get("target_kind"), case["target_kind"]),
            "cast_ms": (want_started.get("cast_ms"), started.get("cast_ms")),
            "result": (want_started.get("result"), started.get("result")),  # 0 = started, else the refusal code
            "phase_changes": (expected.get("phase_changes"), case["phase_changes"]),
            "removed": (expected.get("removed"), case["removed"]),
            "labor": (expected.get("labor"), case["labor"]),
            # the items, not the bag slots or item ids they landed in (`item_deliveries`, `delivered`)
            "item_tasks": (None if want_items is None else delivered(want_items), delivered(case["item_tasks"])),
            "errors": (expected.get("errors"), case["errors"]),
        }
        # a field the native case does not list (None) is not compared; an empty list is a listed value
        differs = [name for name, (want, got) in fields.items() if want is not None and want != got]
        rows.append({"skill": skill, "status": "MATCH" if not differs else "DIFFERS", "fields": fields, "differs": differs})
    for index, case in enumerate(bevy):
        if index not in used:
            rows.append({"skill": case["skill"], "status": "EXTRA"})
    return rows


def le(value: int, size: int) -> bytes:
    return value.to_bytes(size, "little")


def delivery(template: int, amount: int, grade: int = 0) -> dict:
    return {"template_id": template, "amount": amount, "grade": grade}


class ReplayTests(unittest.TestCase):
    """The reducer and the comparison on synthetic packets (`--self-test`): no server, client or game files.

    The packet order of a cast is the one in the 2026-10-30 herb run (capture-results.txt) and the server's
    `end_skill`: started, fired, phase change, BAG_FULL error, labor change, ended."""

    PLAYER = 0x625A

    def line(self, direction: str, opcode: int, body: bytes) -> str:
        return f"[TRACE] fields - {direction} opcode=0x{opcode:03x} Packet len={len(body)} body={body.hex(' ')}"

    def start(self, skill: int, doodad: int) -> str:
        # CSStartSkill: skill u32, caster kind u8 + bc, target kind u8 (4 = doodad) + bc, flag u8
        return self.line("C->S", 0x052, le(skill, 4) + b"\0" + le(self.PLAYER, 3) + b"\4" + le(doodad, 3) + b"\0")

    def started(self, skill: int, tl: int, doodad: int, cast_ms: int = 5000, result: bytes = b"\0") -> str:
        # SCSkillStarted: skill u32, tl u16, caster kind + bc, target kind + bc, skill object u8, cast u16 and base
        # cast u16 (x10 ms), synergy u8, then the SkillResult: flags u8 (bit 0: a failure code byte follows)
        body = le(skill, 4) + le(tl, 2) + b"\0" + le(self.PLAYER, 3) + b"\4" + le(doodad, 3) + b"\0"
        return self.line("S->C", 0x0a1, body + le(cast_ms // 10, 2) * 2 + b"\0" + result)

    def fired(self, skill: int, tl: int) -> str:
        return self.line("S->C", 0x0a2, le(skill, 4) + le(tl, 2))  # the reducer reads the skill id only

    def phase(self, doodad: int, group: int) -> str:
        # SCDoodadPhaseChanged: bc, func group u32, time left u32, puzzle group i32, item template u32
        return self.line("S->C", 0x10f, le(doodad, 3) + le(group, 4) + le(180000, 4) + le(0, 8))

    def labor(self, amount: int) -> str:
        # SCCharacterLaborPowerChanged: amount i32, action i32, point i32, step u8
        return self.line("S->C", 0x045, amount.to_bytes(4, "little", signed=True) + le(9, 4) + le(1, 4) + b"\0")

    def error(self, code: int) -> str:
        # SCErrorMsg: code i16, error2 u16, kind u32, notify u8
        return self.line("S->C", 0x10a, le(code, 2) + le(code, 2) + le(0, 4) + b"\1")

    @staticmethod
    def task_item(template: int, item_id: int, count: int, grade: int = 0, block: int = 128) -> bytes:
        # aa-net `read_task_item_with_raw` (items.rs:498): template u32, id u64, grade u8, flags u8, count i32, the padded
        # detail block (i16 length, then the block), create time i64, lifespan i32, made unit u32, world u8, two times i64
        return (le(template, 4) + le(item_id, 8) + bytes([grade, 0]) + le(count, 4) + le(block, 2) + bytes(block)
                + le(10, 8) + le(0, 4) + le(0, 4) + b"\1" + le(0, 16))

    def create(self, template: int, count: int = 1, slot: int = 0, item_id: int = 1, grade: int = 0, action: int = 5,
               container: int = 2, block: int = 128) -> bytes:
        # ItemTask CREATE (5; TAKE 6 and CHANGE_OWNER 15 read the same): container u8 (2 = the bag), slot u8, the task item
        return bytes([action, container, slot]) + self.task_item(template, item_id, count, grade, block)

    @staticmethod
    def stack(template: int, delta: int, slot: int = 0, item_id: int = 1, container: int = 2) -> bytes:
        # ItemTask ADD_STACK (4): container u8 (2 = the bag), slot u8, item id u64, delta i32, template u32
        return bytes([4, container, slot]) + le(item_id, 8) + delta.to_bytes(4, "little", signed=True) + le(template, 4)

    @staticmethod
    def task_body(*tasks: bytes, reason: int = 9) -> bytes:
        # SCItemTaskSuccess (aa-net `ItemTasks::read`, items.rs:378): reason u8, task count u8, the tasks, the force-remove
        # ids (count u8, none here), then the u32 the server's own test appends
        return bytes([reason, len(tasks)]) + b"".join(tasks) + b"\0" + le(0, 4)

    def items(self, *tasks: bytes) -> str:
        return self.line("S->C", 0x090, self.task_body(*tasks))

    def gathered(self, *tasks: bytes) -> dict:
        """The reduced herb cast of a run that delivered `tasks` (and nothing when there are none)."""
        herb = 13990
        lines = [self.start(herb, 5002), self.started(herb, 2, 5002, 4000), self.fired(herb, 2), self.phase(5002, 4514)]
        lines += [self.items(*tasks)] if tasks else []
        (record,) = reduce_packets("\n".join(lines + [self.labor(-1), self.ended(2)]))
        return record

    def refused(self, result: bytes) -> dict:
        """The reduced herb cast that SCSkillStarted answered with `result` (a refusal has no timeline)."""
        (record,) = reduce_packets(self.start(13990, 5002) + "\n" + self.started(13990, 0, 5002, 0, result))
        return record

    def ended(self, tl: int) -> str:
        return self.line("S->C", 0x0a3, le(tl, 2))

    def test_results_belong_to_the_running_cast(self):
        ore, herb = 11106, 13990
        records = reduce_packets("\n".join([
            self.labor(100),  # the GM's addlabor, before any cast
            self.start(ore, 5001), self.started(ore, 1, 5001), self.fired(ore, 1), self.phase(5001, 425),
            self.items(self.create(8001, 1)), self.labor(-3), self.ended(1),
            self.labor(5), self.error(7),  # between the casts: regeneration, other systems
            self.start(herb, 5002), self.started(herb, 2, 5002, 4000), self.fired(herb, 2), self.phase(5002, 4514),
            self.ended(9),  # another unit's timeline does not close this cast
            self.error(25), self.items(self.create(8002, 2)), self.labor(-1), self.ended(2),
            self.labor(5), self.error(8), self.items(self.create(8003, 3)),  # after the last cast
        ]))
        self.assertEqual([r["skill"] for r in records], [ore, herb])
        ore_record, herb_record = records
        self.assertEqual((ore_record["labor"], ore_record["errors"], ore_record["item_tasks"]),
                         ([-3], [], [delivery(8001, 1)]))
        self.assertEqual((herb_record["labor"], herb_record["errors"], herb_record["item_tasks"]),
                         ([-1], [25], [delivery(8002, 2)]))
        self.assertEqual((ore_record["phase_changes"], herb_record["phase_changes"]), ([425], [4514]))

    def test_the_next_cast_closes_a_case_that_never_ended(self):
        records = reduce_packets("\n".join([
            self.start(11106, 5001), self.started(11106, 1, 5001), self.labor(-3),
            self.start(13990, 5002), self.started(13990, 2, 5002, 4000), self.labor(-1),
        ]))
        self.assertEqual([r["labor"] for r in records], [[-3], [-1]])

    def test_a_refused_cast_ends_at_its_refusal(self):
        # a refusal has no timeline (tl 0), so no SCSkillEnded follows it
        (record,) = reduce_packets("\n".join([
            self.start(13990, 5002), self.started(13990, 0, 5002, 0, b"\1\x27"), self.labor(5), self.error(7),
        ]))
        self.assertEqual((record["labor"], record["errors"]), ([], []))

    def test_doodad_changes_follow_the_doodad_after_the_case(self):
        # a depletion (255 -> 425 -> 426, then removal) can come after SCSkillEnded
        (record,) = reduce_packets("\n".join([
            self.start(11106, 5001), self.started(11106, 1, 5001), self.phase(5001, 425), self.ended(1),
            self.phase(5001, 426), self.line("S->C", 0x10d, le(5001, 3)),
        ]))
        self.assertEqual((record["phase_changes"], record["removed"]), ([425, 426], True))

    def test_a_refusal_keeps_its_failure_code(self):
        # SkillResult: flags, then the code if bit 0 (NEED_LABOR_POWER 0x27), a u16 if bit 1 and a u32 if bit 2
        for result, code in ((b"\1\x27", 0x27), (b"\5\x27\7\0\0\0", 0x27), (b"\0", 0), (b"\2\x05\0", 0), (b"\1", 0)):
            with self.subTest(result=result.hex(" ")):
                self.assertEqual(self.refused(result)["skill_started"]["result"], code)

    def test_item_deliveries_follow_the_aa_net_golden_body(self):
        # aa-net `item_task_success_golden_bytes` (items.rs:710): reason 9, +50 copper, a new item (template 8001, id 600,
        # count 5) in bag slot 4, a stack of 8001 (id 555) grows by 3 in slot 1, force remove 7, the trailing u32
        body = (bytes([9, 3]) + b"\1" + le(50, 4) + self.create(8001, 5, slot=4, item_id=600)
                + self.stack(8001, 3, slot=1, item_id=555) + b"\1" + le(7, 8) + le(0, 4))
        self.assertEqual(item_deliveries(body), [delivery(8001, 5), delivery(8001, 3)])

    def test_item_deliveries_walk_past_every_other_task(self):
        # one task of every other action, with the bytes `ItemTask::read` (items.rs:401) takes for it: a wrong size would
        # shift the CREATE behind them
        for block in (0, 128, 129):  # the declared length of a padded detail block is not always 128 (items.rs:475)
            others = {
                1: le(5, 4),  # CHANGE_MONEY_AMOUNT: i32
                2: le(5, 4),  # CHANGE_BANK_MONEY_AMOUNT: i32
                3: b"\1" + le(5, 4),  # CHANGE_GAME_POINT: kind u8, amount i32
                7: b"\2\3" + le(9, 8) + le(8001, 4),  # REMOVE: container, slot, item id u64, template u32
                8: b"\2\1\2\6" + le(9, 8) + le(0, 8) + le(0, 4),  # SWAP_SLOT: two container + slot, two item ids, flags u32
                9: b"\2\3" + le(9, 8) + le(block, 2) + bytes(block),  # UPDATE_DETAIL: container, slot, item id, padded block
                10: b"\2\3" + le(9, 8) + b"\1" + le(0, 8),  # SET_FLAGS_BITS: container, slot, item id, bits u8, 8 skipped
                11: b"\2\3" + le(9, 8) + b"\1\0\0\0" + le(0, 16),  # UPDATE_FLAGS: ..., bits u8, three bool u8, two times i64
                12: le(9, 8),  # REMOVE_CRAFTING: item id
                13: b"\2\3" + le(9, 8),  # SEIZE: container, slot, item id
                14: b"\2\3" + le(9, 8) + b"\2",  # CHANGE_GRADE: container, slot, item id, grade u8
                16: le(5, 4),  # CHANGE_AA_POINT: i32
                17: le(5, 4),  # CHANGE_BANK_AA_POINT: i32
                18: b"\1",  # CHANGE_AUTO_USE_AA_POINT: u8
            }
            with self.subTest(block=block):
                tasks = [bytes([action]) + task for action, task in others.items()]
                body = self.task_body(*tasks, self.create(8002, 2, grade=1, block=block))
                self.assertEqual(item_deliveries(body), [delivery(8002, 2, 1)])
        # a negative declared length reads as 0 (`r.i16()?.max(0)`, items.rs:476)
        no_block = bytes([9, 2, 3]) + le(9, 8) + (-1).to_bytes(2, "little", signed=True)
        self.assertEqual(item_deliveries(self.task_body(no_block, self.create(8002, 2))), [delivery(8002, 2)])

    def test_item_deliveries_keep_what_came_before_a_short_body_or_an_unknown_action(self):
        # `ItemTasks::read` keeps the tasks before the one it cannot read (`truncated`); no prefix of a body may raise
        stack, new = self.stack(8001, 3), self.create(8002, 1)
        body = self.task_body(stack, new)
        complete = ((delivery(8001, 3), 2 + len(stack)), (delivery(8002, 1), 2 + len(stack) + len(new)))
        for end in range(len(body) + 1):
            self.assertEqual(item_deliveries(body[:end]), [d for d, size in complete if end >= size], f"prefix {end}")
        self.assertEqual(item_deliveries(self.task_body(stack, bytes([99]), new)), [delivery(8001, 3)])

    def test_only_items_entering_the_bag_are_deliveries(self):
        body = self.task_body(
            self.create(8001, 1, container=3),  # into the bank (3)
            self.stack(8005, 3, container=3),  # a stack in the bank grows
            self.stack(8002, -2),  # a stack in the bag shrinks
            self.create(8003, 1, action=6),  # TAKE
            self.create(8004, 2, grade=3, action=15),  # CHANGE_OWNER
        )
        self.assertEqual(item_deliveries(body), [delivery(8003, 1), delivery(8004, 2, 3)])

    def test_the_same_items_in_other_slots_match(self):
        # the bag slot and the item db id of a delivery differ from run to run; the items do not
        native = self.gathered(self.create(8001, 2, slot=3, item_id=600), self.create(8002, 1, slot=4, item_id=601))
        bevy = self.gathered(self.create(8001, 2, slot=17, item_id=91234), self.create(8002, 1, slot=18, item_id=91235))
        (row,) = compare([native], [bevy])
        self.assertEqual((row["status"], row["differs"]), ("MATCH", []))

    def test_a_different_delivery_differs(self):
        native = self.gathered(self.create(8001, 2))
        others = {"template": self.create(8002, 2), "amount": self.create(8001, 3), "grade": self.create(8001, 2, grade=1)}
        for what, task in others.items():
            with self.subTest(different=what):
                (row,) = compare([native], [self.gathered(task)])
                self.assertEqual((row["status"], row["differs"]), ("DIFFERS", ["item_tasks"]))
        with self.subTest(different="no delivery"):
            (row,) = compare([native], [self.gathered()])
            self.assertEqual((row["status"], row["differs"]), ("DIFFERS", ["item_tasks"]))

    def test_a_stack_add_counts_like_a_new_stack(self):
        # five items in a new stack, or three more on a stack and two in a new one: the bag split them, not the server
        native = self.gathered(self.create(8001, 5, slot=4))
        bevy = self.gathered(self.stack(8001, 3, slot=1, item_id=555), self.create(8001, 2, slot=9, item_id=700))
        (row,) = compare([native], [bevy])
        self.assertEqual((row["status"], row["differs"]), ("MATCH", []))
        (row,) = compare([native], [self.gathered(self.stack(8001, 3, slot=1, item_id=555), self.create(8001, 1))])
        self.assertEqual((row["status"], row["differs"]), ("DIFFERS", ["item_tasks"]))

    def test_compare_checks_the_skill_result(self):
        # a refusal's failure code (the SkillResult of SCSkillStarted): NEED_LABOR_POWER 0x27 against BAG_FULL 0x2E
        labor, full = self.refused(b"\1\x27"), self.refused(b"\1\x2e")
        self.assertEqual(compare([labor], [self.refused(b"\1\x27")])[0]["status"], "MATCH")
        (row,) = compare([labor], [full])
        self.assertEqual((row["status"], row["differs"]), ("DIFFERS", ["result"]))
        (row,) = compare([self.refused(b"\0")], [labor])  # a cast that started against a refusal
        self.assertEqual((row["status"], row["differs"]), ("DIFFERS", ["result"]))

    @staticmethod
    def case(**changes) -> dict:
        """A reduced cast as `reduce_packets` writes it and `native.json` lists it."""
        case = {"skill": 11106, "target_kind": 4, "skill_started": {"cast_ms": 5000, "result": 0}, "phase_changes": [425],
                "removed": False, "item_tasks": [delivery(8001, 1)], "labor": [-3], "errors": []}
        return case | changes

    def test_compare_checks_labor_items_and_errors(self):
        native = [self.case()]
        self.assertEqual(compare(native, [self.case()])[0]["status"], "MATCH")
        for field, other in (("labor", [-1]), ("errors", [25]), ("item_tasks", [])):
            with self.subTest(field=field):
                (row,) = compare(native, [self.case(**{field: other})])
                self.assertEqual((row["status"], row["differs"]), ("DIFFERS", [field]))

    def test_compare_skips_what_the_native_case_does_not_list(self):
        native = [{k: v for k, v in self.case().items() if k not in ("labor", "item_tasks", "errors")}
                  | {"skill_started": {"cast_ms": 5000}}]  # no result code either
        (row,) = compare(native, [self.case(labor=[-9], errors=[1], item_tasks=[delivery(1, 1)],
                                            skill_started={"cast_ms": 5000, "result": 0x27})])
        self.assertEqual(row["status"], "MATCH")


def self_test() -> int:
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(ReplayTests)
    return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1


def main() -> int:
    if sys.argv[1:] == ["--self-test"]:
        return self_test()
    parser = argparse.ArgumentParser(epilog=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--client", default=os.environ.get("GATHER_BEVY_CLIENT", "aa-client.exe"))
    parser.add_argument("--login", default=os.environ.get("AA_LOGIN", "127.0.0.1:1237"))
    parser.add_argument("--account", required=True)
    parser.add_argument("--character", required=True)
    parser.add_argument("--password-file")
    parser.add_argument("--server-log", required=True, type=Path)
    parser.add_argument("--evidence", type=Path, default=ROOT / "tools/evidence/gather")
    parser.add_argument("--raw", type=Path, help="client log and packet dump (default: <evidence>/raw)")
    parser.add_argument("--script", type=Path, help="default: <evidence>/replay.txt")
    args = parser.parse_args()

    evidence = args.evidence
    script = args.script or evidence / "replay.txt"
    raw = args.raw or evidence / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    offset = start_offset(args.server_log)

    command = [args.client, "--headless", "--mute", "--account", args.account, "--login", args.login,
               "--character", args.character, "--script", str(script)]
    if args.password_file:
        command += ["--password-file", args.password_file]
    client_log = raw / "client.log"
    started = time.time()
    with client_log.open("w", encoding="utf-8") as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, env=os.environ.copy())
    wall = time.time() - started

    text, _ = read_new(args.server_log, offset)
    (raw / "server-packets.log").write_text(text, encoding="utf-8")
    bevy = reduce_packets(text)
    (evidence / "bevy.json").write_text(json.dumps(bevy, indent=2) + "\n", encoding="utf-8")

    native_path = evidence / "native.json"
    report = [f"## Bevy replay ({time.strftime('%Y-%m-%d %H:%M:%S')})", "",
              f"client: {args.client} (--headless --mute)", f"script: {script}  exit: {result.returncode}  wall: {wall:.1f} s",
              f"gather casts: {len(bevy)}", ""]
    if native_path.exists():
        native = json.loads(native_path.read_text(encoding="utf-8"))
        rows = compare(native, bevy)
        report += ["| skill | status | differs |", "|---|---|---|"]
        report += [f"| {row['skill']} | {row['status']} | {', '.join(row.get('differs', []))} |" for row in rows]
    else:
        report += ["native.json is missing: this run is the Bevy side of the scaffold only.",
                   "Record the cases in capture-plan.txt with the original client, then rerun."]
    (evidence / "report.txt").write_text("\n".join(report) + "\n", encoding="utf-8")

    status = "PASS" if result.returncode == 0 else "FAIL"
    print(f"gather replay {status}: {len(bevy)} casts, client exit {result.returncode}, report {evidence / 'report.txt'}")
    return 0 if result.returncode == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
