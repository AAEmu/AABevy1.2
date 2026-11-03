import contextlib
import io
import json
import struct
import tempfile
import unittest
from datetime import date, datetime, timezone
from pathlib import Path

import _setup  # noqa: F401
import server_log as sl

DAY = date(2026, 11, 3)
UTC = timezone.utc


def utc_us(h, m, s, micro=0, day=DAY):
    return int(datetime(day.year, day.month, day.day, h, m, s, tzinfo=UTC).timestamp()) * 1_000_000 + micro


def parse(lines, **kw):
    kw.setdefault("tz", UTC)
    return sl.parse_lines(lines, DAY, **kw)


def cs_start_skill_body(skill=16790, caster=26891, target=26593, flag=0):
    return (struct.pack("<I", skill) + bytes([0]) + caster.to_bytes(3, "little") + bytes([0]) + target.to_bytes(3, "little")
            + bytes([flag]))


class Timestamps(unittest.TestCase):
    LINE = "22:02:52 [INFO] handlers - StartSkill: Id 16790, flag 0, caster=26891, target=26593"

    def test_whole_seconds(self):
        rows = parse([self.LINE])
        self.assertEqual(len(rows), 1)
        r = rows[0]
        self.assertEqual((r["src"], r["kind"], r["dir"], r["op"]), ("server", "net", "out", "CSStartSkill"))
        self.assertEqual((r["skill_id"], r["obj"]), (16790, 26891))
        self.assertEqual(r["fields"]["target"], 26593)
        self.assertEqual(r["wall_us"], utc_us(22, 2, 52))

    def test_microseconds_with_aaemu_log_ms(self):
        rows = parse(["22:02:52.123456 [INFO] handlers - StartSkill: Id 16790, flag 0, caster=26891, target=26593"])
        self.assertEqual(rows[0]["wall_us"], utc_us(22, 2, 52, 123456))

    def test_milliseconds_and_odd_fraction_lengths(self):
        a = parse(["22:02:52.123 [INFO] handlers - StartSkill: Id 1, flag 0, caster=2, target=3"])[0]["wall_us"]
        b = parse(["22:02:52.1234567 [INFO] handlers - StartSkill: Id 1, flag 0, caster=2, target=3"])[0]["wall_us"]
        self.assertEqual(a, utc_us(22, 2, 52, 123000))
        self.assertEqual(b, utc_us(22, 2, 52, 123456))

    def test_local_time_zone_is_the_default_and_utc_offsets_work(self):
        local = sl.parse_lines([self.LINE], DAY)[0]["wall_us"]
        expected = int(datetime(2026, 11, 3, 22, 2, 52).astimezone().timestamp()) * 1_000_000
        self.assertEqual(local, expected)
        plus2 = sl.parse_lines([self.LINE], DAY, sl.parse_tz("+02:00"))[0]["wall_us"]
        self.assertEqual(plus2, utc_us(20, 2, 52))
        self.assertIs(sl.parse_tz("utc"), UTC)
        self.assertIsNone(sl.parse_tz("local"))
        with self.assertRaises(ValueError):
            sl.parse_tz("Europe/Paris")

    def test_midnight_rolls_the_date_over(self):
        rows = parse(["23:59:59.900000 [INFO] handlers - StartSkill: Id 1, flag 0, caster=2, target=3",
                      "00:00:00.100000 [INFO] handlers - StartSkill: Id 2, flag 0, caster=2, target=3"])
        self.assertEqual(rows[1]["wall_us"] - rows[0]["wall_us"], 200_000)

    def test_date_comes_from_the_file_name_else_today(self):
        self.assertEqual(sl.day_for(Path("Server.2026-10-15.log"), None), date(2026, 10, 15))
        self.assertEqual(sl.day_for(Path("rs-game.log"), "2026-01-02"), date(2026, 1, 2))
        self.assertEqual(sl.day_for(Path("rs-game.log"), None), date.today())

    def test_ansi_colours_and_lines_without_packets_are_ignored(self):
        rows = parse(["\x1b[2m22:02:52\x1b[0m \x1b[32m INFO\x1b[0m handlers: StartSkill: Id 5, flag 0, caster=1, target=2",
                      "22:02:53 [INFO] data - Loaded 7 battlefields", "garbage", ""])
        self.assertEqual([r["skill_id"] for r in rows], [5])

    def test_window_filter(self):
        rows = parse([self.LINE], since_us=utc_us(22, 3, 0))
        self.assertEqual(rows, [])
        rows = parse([self.LINE], since_us=utc_us(22, 2, 0), until_us=utc_us(22, 3, 0))
        self.assertEqual(len(rows), 1)


class PacketLines(unittest.TestCase):
    def test_c2s_field_trace_decodes_the_start_skill_body_and_absorbs_the_handler_line(self):
        body = cs_start_skill_body().hex(" ")
        rows = parse([
            f"22:02:52.050000 [TRACE] fields - C->S level=1 opcode=0x052 CSStartSkillPacket len=17 body={body} ",
            "22:02:52.150000 [INFO] handlers - StartSkill: Id 16790, flag 0, caster=26891, target=26593"])
        self.assertEqual(len(rows), 1)                    # one CSStartSkill, not two
        r = rows[0]
        self.assertEqual((r["op"], r["dir"], r["skill_id"], r["obj"]), ("CSStartSkill", "out", 16790, 26891))
        self.assertEqual(r["wall_us"], utc_us(22, 2, 52, 50000))
        self.assertEqual(r["fields"]["target"], {"type": 0, "obj": 26593})
        self.assertEqual(r["fields"]["handler_logged_us"], utc_us(22, 2, 52, 150000))

    def test_ground_target_position_is_decoded(self):
        body = (struct.pack("<I", 10664) + bytes([0]) + (26891).to_bytes(3, "little") + bytes([1])
                + struct.pack("<q", int(15534.5 * 4096) << 32) + struct.pack("<q", int(15330.25 * 4096) << 32)
                + struct.pack("<ff", 130.0, 0.0) + bytes(6) + bytes([0]))
        rows = parse([f"22:00:00 [TRACE] fields - C->S level=1 opcode=0x052 CSStartSkillPacket len={len(body)} body={body.hex(' ')} "])
        target = rows[0]["fields"]["target"]
        self.assertEqual((target["type"], target["x"], target["y"], target["z"]), (1, 15534.5, 15330.25, 130.0))

    def test_other_c2s_packets_keep_their_hex_body_with_ops_all(self):
        lines = ["22:00:00 [TRACE] fields - C->S level=1 opcode=0x00C CSMoveUnitPacket len=4 body=01 02 03 04 "]
        self.assertEqual(parse(lines), [])                                   # not skill related: dropped by default
        rows = parse(lines, ops="all")
        self.assertEqual((rows[0]["op"], rows[0]["fields"]["body"]), ("CSMoveUnit", "01 02 03 04"))

    def test_c2s_debug_line_is_a_row_unless_the_field_trace_just_made_one(self):
        rows = parse(["22:00:00.000 [DEBUG] registry - GamePacket: C->S CSStopCastingPacket conn=7"])
        self.assertEqual((rows[0]["op"], rows[0]["dir"], rows[0]["fields"]["conn"]), ("CSStopCasting", "out", 7))
        rows = parse(["22:00:00.000 [TRACE] fields - C->S level=1 opcode=0x053 CSStopCastingPacket len=0 body=",
                      "22:00:00.001 [DEBUG] registry - GamePacket: C->S CSStopCastingPacket conn=7"])
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["fields"]["conn"], 7)

    def test_s2c_debug_line_and_its_decoded_struct_become_one_row(self):
        rows = parse([
            "22:02:52.200000 [DEBUG] connection - GamePacket: S->C type 0A1 SCSkillStartedPacket (31 bytes)",
            "22:02:52.200100 [TRACE] connection - SCSkillStartedPacket { id: 16790, tl: 5, caster: Unit(26891), "
            "target: Unit(26593), skill_object: None, real_cast_time_div10: 150, base_cast_time_div10: 150, cast_synergy: 0, "
            "extra_flags: 0, extra_byte: 0, extra_ushort: 0, extra_uint: 0 }"])
        self.assertEqual(len(rows), 1)
        r = rows[0]
        self.assertEqual((r["dir"], r["op"], r["skill_id"], r["tl"], r["obj"]), ("in", "SCSkillStarted", 16790, 5, 26891))
        self.assertEqual(r["fields"]["opcode"], "0x0A1")
        self.assertEqual(r["fields"]["len"], 31)
        self.assertEqual(r["fields"]["caster"], {"Unit": 26891})
        self.assertEqual(r["fields"]["real_cast_time_div10"], 150)

    def test_struct_line_alone_makes_a_row(self):
        rows = parse(["22:02:53 [TRACE] connection - SCPlotEventPacket { tl: 5, event_id: 12130, skill_id: 18132, caster: Unit(1), "
                      "target: Position { x: 1.5, y: 2.5, z: 3.0, roll: 0, pitch: 0, yaw: -5 }, obj_id: 0, casting_time: 0, "
                      "flag: 0, item_id: 0, target_unit_count: 1 }"])
        r = rows[0]
        self.assertEqual((r["op"], r["skill_id"], r["tl"], r["obj"]), ("SCPlotEvent", 18132, 5, 1))
        self.assertEqual(r["fields"]["target"], {"Position": {"x": 1.5, "y": 2.5, "z": 3.0, "roll": 0, "pitch": 0, "yaw": -5}})

    def test_s2c_without_fields_keeps_name_and_size_and_the_csharp_form_works(self):
        rows = parse(["22:02:52 [DEBUG] connection - GamePacket: S->C type 0A2 SCSkillFiredPacket (40 bytes)",
                      "22:02:52 [DEBUG] PacketMarshaler - GamePacket: S->C type 0A3 .G2C.SCSkillEndedPacket "])
        self.assertEqual([r["op"] for r in rows], ["SCSkillFired", "SCSkillEnded"])
        self.assertNotIn("skill_id", rows[0])

    def test_default_ops_keep_skill_packets_and_drop_the_chatty_ones(self):
        lines = ["22:02:52 [DEBUG] connection - GamePacket: S->C type 0A2 SCSkillFiredPacket (40 bytes)",
                 "22:02:52 [DEBUG] connection - GamePacket: S->C type 011 SCOneUnitMovementPacket (30 bytes)",
                 "22:02:52 [DEBUG] connection - GamePacket: S->C type 012 SCUnitDamagedPacket (30 bytes)"]
        self.assertEqual([r["op"] for r in parse(lines)], ["SCSkillFired", "SCUnitDamaged"])
        self.assertEqual(len(parse(lines, ops="all")), 3)

    def test_rows_are_sorted_by_time(self):
        rows = parse(["22:02:54 [INFO] handlers - StartSkill: Id 2, flag 0, caster=1, target=3",
                      "22:02:53.5 [INFO] handlers - StartSkill: Id 1, flag 0, caster=1, target=3"])
        self.assertEqual([r["skill_id"] for r in rows], [1, 2])


class DebugParser(unittest.TestCase):
    def test_structs_variants_arrays_strings_and_options(self):
        text = 'Foo { a: 1, b: -2.5, c: "x, y", d: [1, 2, 3], e: Some(7), f: None, g: Unit(4), h: Item { id: 9, n: true }, i: (1, 2) }'
        got = sl.parse_debug(text)
        self.assertEqual(got, {"a": 1, "b": -2.5, "c": "x, y", "d": [1, 2, 3], "e": 7, "f": None, "g": {"Unit": 4},
                               "h": {"Item": {"id": 9, "n": True}}, "i": [1, 2]})

    def test_a_torn_line_is_an_error_not_a_crash(self):
        with self.assertRaises(sl.DebugParseError):
            sl.parse_debug("Foo { a: 1, b: [1, 2")
        rows = parse(["22:02:52 [TRACE] connection - SCSkillStartedPacket { id: 1, tl: "])
        self.assertEqual(rows, [])

    def test_obj_of(self):
        self.assertEqual(sl.obj_of({"Unit": 5}), 5)
        self.assertEqual(sl.obj_of({"Item": {"obj_id": 8, "item_id": 1}}), 8)
        self.assertEqual(sl.obj_of(7), 7)
        self.assertIsNone(sl.obj_of({"Position": {"x": 1.0}}))


class Cli(unittest.TestCase):
    def test_main_writes_server_jsonl_and_cuts_by_trace_window(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / "Server.2026-11-03.log"
            log.write_text("22:02:52.5 [INFO] handlers - StartSkill: Id 7, flag 0, caster=1, target=2\n"
                           "23:30:00 [INFO] handlers - StartSkill: Id 8, flag 0, caster=1, target=2\n", encoding="utf-8")
            out = Path(d) / "server.jsonl"
            trace = Path(d) / "trace.jsonl"
            trace.write_text(json.dumps({"wall_us": utc_us(22, 2, 50), "kind": "press"}) + "\n", encoding="utf-8")
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(sl.main([str(log), "--tz", "utc", "--out", str(out), "--trace", str(trace), "--pad", "5"]), 0)
            rows = [json.loads(line) for line in out.read_text(encoding="utf-8").splitlines()]
            self.assertEqual([r["skill_id"] for r in rows], [7])

    def test_missing_log(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(sl.main(["no-such.log"]), 1)


if __name__ == "__main__":
    unittest.main()
