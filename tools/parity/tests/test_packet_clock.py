"""Packet-clock alignment tests, including the K2 capture regression."""
from __future__ import annotations

import importlib.util
import json
import os
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SCENARIO_SPEC = importlib.util.spec_from_file_location(
    "parity_scenario_pc", ROOT / "scenario.py")
scenario = importlib.util.module_from_spec(SCENARIO_SPEC)
assert SCENARIO_SPEC.loader is not None
SCENARIO_SPEC.loader.exec_module(scenario)

RUNNER_SPEC = importlib.util.spec_from_file_location(
    "parity_runner_pc", ROOT / "run_original.py")
runner = importlib.util.module_from_spec(RUNNER_SPEC)
assert RUNNER_SPEC.loader is not None
RUNNER_SPEC.loader.exec_module(runner)

packet_clock = runner.packet_clock
K2_RUN_TEXT = os.environ.get("AA_PARITY_K2_RUN", "")
K2_RUN = pathlib.Path(K2_RUN_TEXT) if K2_RUN_TEXT else None


class PacketClockTests(unittest.TestCase):
    def test_move_and_ping_fields(self) -> None:
        move = bytes.fromhex("a6 21 00 01 40 42 0f 00") + b"\0" * 25
        self.assertEqual(packet_clock.packet_clock("CSMoveUnitPacket", move),
                         (1_000_000, "CSMoveUnitPacket.time"))
        ping = b"\0" * 16 + bytes.fromhex("40 42 0f 00")
        self.assertEqual(packet_clock.packet_clock("PingPacket", ping),
                         (1_000_000, "PingPacket.local"))
        self.assertEqual(packet_clock.packet_clock("CSStartSkillPacket", b"\0" * 13),
                         (None, None))
        self.assertEqual(packet_clock.packet_clock("PingPacket", b"\0" * 4),
                         (None, None))

    def test_read_log_keeps_a_torn_line_for_the_next_read(self) -> None:
        import tempfile
        line = ("10:00:00 [TRACE] fields - C->S level=1 opcode=0x089 "
                "CSMoveUnitPacket len=33 body=a6 21 00 01 e8 03 00 00 "
                + "00 " * 25).rstrip()
        with tempfile.TemporaryDirectory() as tmp:
            log = pathlib.Path(tmp) / "server.log"
            # half of the line, no newline yet: nothing is read, the offset stays
            log.write_bytes(line[:70].encode())
            self.assertEqual(packet_clock.read_log(log, 0), ([], 0))
            # an invalid UTF-8 byte before it must not shift the byte offset
            log.write_bytes(b"\xff junk\n" + line.encode() + b"\n" + line[:40].encode())
            records, offset = packet_clock.read_log(log, 0)
            self.assertEqual(len(records), 1)
            self.assertEqual(offset, len(b"\xff junk\n" + line.encode() + b"\n"))
        # a garbled body is skipped, not raised
        self.assertIsNone(packet_clock.parse_line(line[:-1] + "zz"))

    def test_run_ids_and_names_cannot_escape_the_run_dir(self) -> None:
        for bad in [".", "..", "...", "a/b", ""]:
            self.assertIsNone(scenario.STEP_ID_RE.match(bad), bad)
        for good in ["k2", "run.1", "2026-11-01T10Z", "..a"]:
            self.assertIsNotNone(scenario.STEP_ID_RE.match(good), good)

    def test_interpolation_and_tags(self) -> None:
        move = ("10:00:00 [TRACE] fields - C->S level=1 opcode=0x089 "
                "CSMoveUnitPacket len=33 body=a6 21 00 01 e8 03 00 00 "
                + "00 " * 25)
        ping = ("10:00:01 [TRACE] fields - C->S level=2 opcode=0x012 "
                "PingPacket len=20 body=" + "00 " * 16 + "d0 07 00 00")
        other = ("10:00:00 [TRACE] fields - C->S level=1 opcode=0x052 "
                 "CSStartSkillPacket len=13 body=02 00 00 00 00 a6 21 00 00 "
                 "0e 22 00 00")
        records = packet_clock.parse_log("\n".join([move, other, ping]))
        packet_clock.align(records)
        self.assertEqual(records[0]["clock"], "packet")
        self.assertEqual(records[0]["client_ms"], 1000)
        self.assertEqual(records[1]["clock"], "interpolated")
        self.assertEqual(records[1]["client_ms"], 1500)
        self.assertEqual(records[2]["client_ms"], 2000)
        rows = packet_clock.client_rows(records)
        self.assertEqual(len(rows), 3)
        self.assertEqual(rows[1]["anchor_before"], "CSMoveUnitPacket.time")
        self.assertEqual(rows[1]["anchor_after"], "PingPacket.local")

    def k2_clock(self, run: pathlib.Path) -> runner.Clock:
        clock = runner.Clock()
        for line in (run / "clock_sync.jsonl").read_text(
                encoding="utf-8").splitlines():
            if not line.strip():
                continue
            row = json.loads(line)
            for kind in ("system", "ui"):
                value = row.get(kind + "_ms")
                if value is None:
                    continue
                rtt = float(row.get(kind + "_rtt_ms") or 0.0)
                clock.add(kind, value, row["wall_ms"] - rtt / 2.0,
                          row["wall_ms"] + rtt / 2.0)
        return clock

    @unittest.skipUnless(K2_RUN is not None and K2_RUN.is_dir(),
                         "set AA_PARITY_K2_RUN to handoff/2026-11-01-combat-k2k3/run")
    def test_k2_alignment_reproduces_corrected_times(self) -> None:
        clock = self.k2_clock(K2_RUN)
        records, _ = packet_clock.read_log(K2_RUN / "server_slice.log")
        packet_clock.align(records, clock)
        by_line = {record["line"]: record for record in records}

        clear = next(record for record in records
                     if record["name"] == "CSChangeTargetPacket"
                     and record.get("body", "").startswith("00 00 00")
                     and record["line"] > 20000)
        self.assertEqual(clear["line"], 21337)
        self.assertTrue(58_300 <= clear["t_ms"] <= 58_800,
                        clear["t_ms"])

        first = by_line[3522]
        self.assertEqual(first["name"], "CSStartSkillPacket")
        self.assertTrue(9_600 <= first["t_ms"] <= 10_100, first["t_ms"])

        repress = by_line[24360]
        self.assertEqual(repress["name"], "CSStartSkillPacket")
        self.assertTrue(68_400 <= repress["t_ms"] <= 68_850, repress["t_ms"])

        out_of_range = by_line[43513]
        self.assertEqual(out_of_range["name"], "CSStartSkillPacket")
        self.assertTrue(117_250 <= out_of_range["t_ms"] <= 117_750,
                        out_of_range["t_ms"])

        cleared = by_line[21576]
        self.assertEqual(cleared["name"], "SCCombatClearedPacket")
        self.assertTrue(59_000 <= cleared["t_ms"] <= 59_500, cleared["t_ms"])

        between = [record for record in records
                   if 21337 < record["line"] < 23610
                   and record["name"] == "CSStartSkillPacket"]
        self.assertEqual(between, [])

        summary = packet_clock.summary(records)
        self.assertGreater(summary["exact_clock_packets"], 1000)
        self.assertGreater(summary["anchors"]["move"], 100)
        self.assertGreater(summary["anchors"]["ping"], 500)
        self.assertIsNotNone(summary["first_client_ms"])
        self.assertIsNotNone(summary["last_client_ms"])


if __name__ == "__main__":
    unittest.main()
