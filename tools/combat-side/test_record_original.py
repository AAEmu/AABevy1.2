"""Offline tests for the original-client combat recorder.

Run with:
    python -m unittest discover -s tools/combat-side -p "test_*.py" -v
"""
from __future__ import annotations

import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import threading
import time
import unittest
from types import SimpleNamespace
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("record_original", HERE / "record_original.py")
rec = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(rec)


def gcd_row(client_ms: int, remaining: int, total: int, wall_ms: float | None = None) -> dict:
    return {"channel": "gcd", "clock": "estimate", "client_ms": client_ms,
            "wall_ms": client_ms if wall_ms is None else wall_ms,
            "gcd_remaining": remaining, "gcd_total": total}


def input_row(key: str, before_wall_ms: float) -> dict:
    return {"channel": "inputs", "key": key, "before_wall_ms": before_wall_ms}


def cast_channel(starts: list[int]) -> list[dict]:
    rows = []
    for start in starts:
        rows.append({"channel": "castbar", "clock": "ui", "native_ms": start - 100,
                     "player_name": "", "player_show": "false", "player_current": ""})
        rows.append({"channel": "castbar", "clock": "ui", "native_ms": start,
                     "player_name": "Flamebolt", "player_show": "true",
                     "player_current": "0"})
    return rows


class ClockTests(unittest.TestCase):
    def make_clock(self, span_s: int, drift_ppm: float = 20.0) -> rec.Clock:
        clock = rec.Clock()
        base = 1_000_000.0
        for index in range(span_s + 1):
            midpoint = base + index * 1000.0
            client = (midpoint - base) * (1.0 + drift_ppm / 1_000_000.0) + 50_000.0
            clock.add("ui", client, midpoint - 1.0, midpoint + 1.0)
        return clock

    def test_short_span_keeps_slope_one(self) -> None:
        report = self.make_clock(10).report()["ui"]
        self.assertFalse(report["drift_applied"])
        self.assertEqual(report["drift_ppm"], 0.0)
        self.assertAlmostEqual(report["fitted_drift_ppm"], 20.0, places=2)
        self.assertIsNotNone(report["drift_stderr_ppm"])

    def test_long_span_applies_drift(self) -> None:
        report = self.make_clock(70).report()["ui"]
        self.assertTrue(report["drift_applied"])
        self.assertAlmostEqual(report["drift_ppm"], 20.0, places=2)
        self.assertFalse(report["drift_warning"])

    def test_large_applied_drift_warns_without_gating(self) -> None:
        report = self.make_clock(70, drift_ppm=500.0).report()
        self.assertTrue(report["ui"]["drift_warning"])
        self.assertIn("not gated", report["drift_warning_policy"])


class GcdSelfCheckTests(unittest.TestCase):
    def check(self, rows: list[dict], inputs: list[dict]) -> dict:
        writer = rec.ChannelWriter(pathlib.Path(tempfile.mkdtemp()), rec.Clock())
        return rec.self_check(writer, rows, inputs, 0, 0, {}, [])

    def complete_rows(self) -> list[dict]:
        rows = [gcd_row(0, 0, 0)]
        rows.append(gcd_row(100, 1000, 1000))
        for step in range(1, 11):
            rows.append(gcd_row(100 + step * 100, 1000 - step * 100, 1000))
        return rows

    def test_complete_press_linked_rise_passes(self) -> None:
        report = self.check(self.complete_rows(), [input_row("1", 50.0)])
        self.assertTrue(report["gcd"]["changed"])
        self.assertEqual(report["gcd"]["complete_rises"], 1)
        self.assertEqual(report["gcd"]["linked_complete_rises"], 1)

    def test_incrementing_counter_fails(self) -> None:
        rows = [gcd_row(0, 0, 0)]
        for step in range(1, 60):
            rows.append(gcd_row(step * 100, step, 1000))
        report = self.check(rows, [input_row("1", 50.0)])
        self.assertFalse(report["gcd"]["changed"])
        self.assertEqual(report["gcd"]["complete_rises"], 0)

    def test_slow_fall_fails(self) -> None:
        rows = self.complete_rows()
        rows[-1] = gcd_row(2000, 0, 1000)
        report = self.check(rows, [input_row("1", 50.0)])
        self.assertFalse(report["gcd"]["changed"])
        self.assertEqual(report["gcd"]["short_or_slow_rises"], 1)

    def test_countdown_only_fails(self) -> None:
        rows = [gcd_row(0, 1000, 1000)]
        for step in range(1, 11):
            rows.append(gcd_row(step * 100, 1000 - step * 100, 1000))
        report = self.check(rows, [input_row("1", 50.0)])
        self.assertFalse(report["gcd"]["changed"])

    def test_most_rises_must_follow_a_press(self) -> None:
        rows = self.complete_rows()
        rows.extend(gcd_row(3000 + row["client_ms"], row["gcd_remaining"], row["gcd_total"])
                    for row in self.complete_rows()[1:])
        report = self.check(rows, [input_row("1", 50.0)])
        self.assertFalse(report["gcd"]["changed"])
        self.assertEqual(report["gcd"]["complete_rises"], 2)
        self.assertEqual(report["gcd"]["linked_complete_rises"], 1)

    def short_run(self, start: int) -> list[dict]:
        return [gcd_row(start + step * 50, 200 - step * 50, 200)
                for step in range(5)]

    def test_one_press_cannot_start_two_rises(self) -> None:
        rows = [gcd_row(0, 0, 0)]
        rows.extend(self.short_run(100))
        rows.extend(self.short_run(400))
        report = self.check(rows, [input_row("1", 50.0)])
        self.assertEqual(report["gcd"]["complete_rises"], 2)
        self.assertEqual(report["gcd"]["linked_complete_rises"], 1)
        self.assertFalse(report["gcd"]["changed"])

    def test_press_outside_the_window_is_not_linked(self) -> None:
        report = self.check(self.complete_rows(), [input_row("1", -1000.0)])
        self.assertEqual(report["gcd"]["complete_rises"], 1)
        self.assertEqual(report["gcd"]["linked_complete_rises"], 0)
        self.assertFalse(report["gcd"]["changed"])


class MemoryUiOffsetTests(unittest.TestCase):
    def channels(self) -> dict[str, list[dict]]:
        cast = [
            {"channel": "castbar", "clock": "ui", "native_ms": 1000,
             "player_name": "", "player_show": "false", "player_current": ""},
            {"channel": "castbar", "clock": "ui", "native_ms": 1000,
             "player_name": "Flamebolt", "player_show": "true", "player_current": "0"},
            {"channel": "castbar", "clock": "ui", "native_ms": 1500,
             "player_name": "Flamebolt", "player_show": "true", "player_current": "500"},
            {"channel": "castbar", "clock": "ui", "native_ms": 1900,
             "player_name": "", "player_show": "false", "player_current": ""},
            {"channel": "castbar", "clock": "ui", "native_ms": 2000,
             "player_name": "Flamebolt", "player_show": "true", "player_current": "0"},
        ]
        gcd = [gcd_row(1027, 0, 0), gcd_row(1027, 1000, 1000),
               gcd_row(2025, 0, 0), gcd_row(2025, 1000, 1000)]
        return {"gcd": gcd, "castbar": cast}

    def test_measures_and_shifts_memory_rows(self) -> None:
        measured = rec.measure_memory_ui_offset(self.channels())
        self.assertTrue(measured["measured"])
        self.assertTrue(measured["measurable"])
        self.assertEqual(measured["pairs"], 2)
        self.assertEqual(measured["offset_ms"], 26.0)
        self.assertEqual(measured["spread_ms"], 2.0)
        self.assertEqual(measured["status"], "confirmed")
        self.assertTrue(measured["confirmed"])
        self.assertFalse(measured["low_confidence"])
        self.assertEqual(measured["unpaired_near_start"], [])
        self.assertFalse(measured["ordering_mismatch"])
        self.assertIsNone(measured["reason"])
        self.assertIn("same frame", measured["assumption"])
        self.assertEqual(measured["raw_field"], "client_ms_raw")
        run = pathlib.Path(tempfile.mkdtemp())
        (run / "gcd.jsonl").write_text("\n".join(
            json.dumps(row) for row in self.channels()["gcd"]) + "\n", encoding="utf-8")
        shifted = rec.shift_estimate_rows(run, measured["offset_ms"])
        self.assertEqual(shifted, 4)
        rows = [json.loads(line) for line in (run / "gcd.jsonl").read_text().splitlines()]
        self.assertEqual([row["client_ms"] for row in rows], [1001, 1001, 1999, 1999])
        self.assertEqual([row["client_ms_raw"] for row in rows], [1027, 1027, 2025, 2025])
        after = rec.measure_memory_ui_offset({"gcd": rows, "castbar": self.channels()["castbar"]})
        self.assertEqual(after["offset_ms"], 26.0)
        self.assertLessEqual(after["residual_after_max_ms"], 1.0)

    def test_instant_rise_without_a_cast_start_is_not_measurable(self) -> None:
        measured = rec.measure_memory_ui_offset(
            {"gcd": [gcd_row(1000, 0, 0), gcd_row(1000, 1000, 1000)], "castbar": []})
        self.assertFalse(measured["measurable"])
        self.assertEqual(measured["pairs"], 0)
        self.assertEqual(measured["reason"], "no-cast-starts")
        self.assertEqual(measured["unpaired_near_start"], [])
        self.assertTrue(measured["ok"])

    def test_starts_that_never_pair_are_not_measurable(self) -> None:
        measured = rec.measure_memory_ui_offset(
            {"gcd": [gcd_row(1000, 0, 0), gcd_row(4000, 1000, 1000)],
             "castbar": cast_channel([1000])})
        self.assertEqual(measured["gcd_rises"], 1)
        self.assertEqual(measured["cast_starts"], 1)
        self.assertEqual(measured["pairs"], 0)
        self.assertEqual(measured["reason"], "no-pairs")
        self.assertFalse(measured["measurable"])
        self.assertEqual(measured["unpaired_near_start"], [])
        self.assertFalse(measured["ordering_mismatch"])
        self.assertTrue(measured["ok"])

    def test_single_pair_is_low_confidence_not_confirmed(self) -> None:
        measured = rec.measure_memory_ui_offset(
            {"gcd": [gcd_row(1000, 0, 0), gcd_row(1027, 1000, 1000)],
             "castbar": cast_channel([1000])})
        self.assertEqual(measured["pairs"], 1)
        self.assertTrue(measured["measured"])
        self.assertFalse(measured["confirmed"])
        self.assertTrue(measured["low_confidence"])
        self.assertEqual(measured["status"], "low-confidence")
        self.assertTrue(measured["ok"])

    def test_unpaired_rise_after_a_near_cast_start_fails(self) -> None:
        measured = rec.measure_memory_ui_offset(
            {"gcd": [gcd_row(1000, 0, 0), gcd_row(1027, 1000, 1000),
                     gcd_row(3000, 0, 0), gcd_row(3027, 1000, 1000),
                     gcd_row(6000, 0, 0), gcd_row(6300, 1000, 1000)],
             "castbar": cast_channel([1000, 3000, 6000])})
        self.assertEqual(measured["pairs"], 2)
        self.assertTrue(measured["confirmed"])
        self.assertEqual(measured["unpaired_near_start"],
                         [{"cast_start_ms": 6000, "gcd_rise_ms": 6300,
                           "delta_ms": 300.0, "skill": "Flamebolt"}])
        self.assertTrue(measured["ordering_mismatch"])
        self.assertFalse(measured["ok"])

    def test_unpaired_rise_before_a_near_cast_start_fails(self) -> None:
        measured = rec.measure_memory_ui_offset(
            {"gcd": [gcd_row(1000, 0, 0), gcd_row(1027, 1000, 1000),
                     gcd_row(3000, 0, 0), gcd_row(3027, 1000, 1000),
                     gcd_row(5900, 0, 0), gcd_row(5900, 1000, 1000)],
             "castbar": cast_channel([1000, 3000, 6000])})
        self.assertEqual(measured["pairs"], 2)
        self.assertEqual(measured["unpaired_near_start"],
                         [{"cast_start_ms": 6000, "gcd_rise_ms": 5900,
                           "delta_ms": -100.0, "skill": "Flamebolt"}])
        self.assertTrue(measured["ordering_mismatch"])
        self.assertFalse(measured["ok"])

    def test_outlier_pair_fails_the_residual_gate(self) -> None:
        measured = rec.measure_memory_ui_offset(
            {"gcd": [gcd_row(1000, 0, 0), gcd_row(1027, 1000, 1000),
                     gcd_row(3025, 0, 0), gcd_row(3027, 1000, 1000),
                     gcd_row(5000, 0, 0), gcd_row(5090, 1000, 1000)],
             "castbar": cast_channel([1000, 3000, 5000])})
        self.assertEqual(measured["pairs"], 3)
        self.assertEqual(measured["offset_ms"], 27.0)
        self.assertTrue(measured["measurable"])
        self.assertFalse(measured["ok"])
        self.assertEqual(measured["residual_after_max_ms"], 63.0)

    def test_shift_only_touches_the_memory_channels(self) -> None:
        run = pathlib.Path(tempfile.mkdtemp())
        rows = {
            "gcd": {"channel": "gcd", "clock": "estimate", "client_ms": 1000},
            "anim_controller": {"channel": "anim_controller", "clock": "estimate",
                                "client_ms": 1000},
            "unit_state": {"channel": "unit_state", "clock": "estimate", "client_ms": 1000},
            "meta": {"channel": "meta", "clock": "estimate", "client_ms": 1000},
        }
        for name, row in rows.items():
            (run / f"{name}.jsonl").write_text(json.dumps(row) + "\n", encoding="utf-8")
        shifted = rec.shift_estimate_rows(run, 27)
        self.assertEqual(shifted, 2)
        for name in ("gcd", "anim_controller"):
            row = json.loads((run / f"{name}.jsonl").read_text().splitlines()[0])
            self.assertEqual(row["client_ms"], 973)
            self.assertEqual(row["client_ms_raw"], 1000)
        for name in ("unit_state", "meta"):
            row = json.loads((run / f"{name}.jsonl").read_text().splitlines()[0])
            self.assertEqual(row["client_ms"], 1000)
            self.assertNotIn("client_ms_raw", row)

    def test_shift_is_idempotent(self) -> None:
        run = pathlib.Path(tempfile.mkdtemp())
        (run / "gcd.jsonl").write_text(
            json.dumps({"channel": "gcd", "clock": "estimate", "client_ms": 1000,
                        "t_ms": 100}) + "\n", encoding="utf-8")
        self.assertEqual(rec.shift_estimate_rows(run, 27), 1)
        first = json.loads((run / "gcd.jsonl").read_text().splitlines()[0])
        self.assertEqual(first["client_ms"], 973)
        self.assertEqual(first["client_ms_raw"], 1000)
        self.assertEqual(first["t_ms"], 73)
        self.assertEqual(rec.shift_estimate_rows(run, 27), 0)
        second = json.loads((run / "gcd.jsonl").read_text().splitlines()[0])
        self.assertEqual(second, first)
        self.assertEqual(rec.shift_estimate_rows(run, 10), 1)
        third = json.loads((run / "gcd.jsonl").read_text().splitlines()[0])
        self.assertEqual(third["client_ms"], 990)
        self.assertEqual(third["client_ms_raw"], 1000)
        self.assertEqual(third["t_ms"], 90)


class McpReplyTests(unittest.TestCase):
    def test_stale_reply_is_skipped(self) -> None:
        mcp = object.__new__(rec.Mcp)
        mcp.sequence = 7
        mcp.skipped = 0
        mcp.process = SimpleNamespace(stdout=io.StringIO(
            '{"jsonrpc":"2.0","id":6,"result":{"stale":true}}\n'
            '{"jsonrpc":"2.0","id":7,"result":{"fresh":true}}\n'))
        self.assertEqual(mcp._read_matching()["result"], {"fresh": True})
        self.assertEqual(mcp.skipped, 1)

    def test_closed_stdout_raises(self) -> None:
        mcp = object.__new__(rec.Mcp)
        mcp.sequence = 1
        mcp.skipped = 0
        mcp.process = SimpleNamespace(stdout=io.StringIO(""))
        with self.assertRaises(RuntimeError):
            mcp._read_matching()


class MainInterruptTests(unittest.TestCase):
    class FakeMcp:
        def __init__(self, *args: object, **kwargs: object):
            self.interrupted = False

        def text(self, name: str, bracket: dict | None = None, **arguments: object) -> str:
            code = str(arguments.get("code", ""))
            if not self.interrupted and "GetCurrTime" in code:
                self.interrupted = True
                raise KeyboardInterrupt()
            return ""

        def resync(self) -> None:
            pass

        def close(self) -> None:
            pass

    def test_keyboard_interrupt_writes_a_failed_self_check(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            for name in ("driver.exe", "aadriver.dll", "launcher.exe", "archeage.exe"):
                (root / name).write_bytes(b"")
            run = root / "run"
            argv = ["record_original.py", "--pid", "999999", "--run-dir", str(run),
                    "--driver", str(root / "driver.exe"),
                    "--dll", str(root / "aadriver.dll"),
                    "--launcher", str(root / "launcher.exe"),
                    "--client", str(root / "archeage.exe"),
                    "--keep-client"]
            with mock.patch.object(rec, "Mcp", self.FakeMcp), \
                    mock.patch.object(rec, "attach_and_hide", lambda *a: None), \
                    mock.patch.object(rec, "ensure_world", lambda *a, **k: "Tester"), \
                    mock.patch.object(sys, "argv", argv):
                with self.assertRaises(KeyboardInterrupt):
                    rec.main()
            check = json.loads((run / "self_check.json").read_text(encoding="utf-8"))
            self.assertFalse(check["ok"])
            self.assertIn("KeyboardInterrupt", check["error"])


class SamplerRecoveryTests(unittest.TestCase):
    class RecoveringMem:
        def __init__(self) -> None:
            self.live = False
            self.local = 0x2000
            self.actor = 0x1000

        def u32(self, address: int) -> int:
            if address == rec.G_CONNECTION - rec.DUMP_BASE:
                return self.local
            if address == self.local + 0x10C:
                return self.actor if self.live else 0
            return 0

        def u32s_exact(self, address: int, count: int) -> list[int]:
            if not self.live:
                raise rec.MemoryReadError("actor unavailable")
            return [0] * count

        def u8_exact(self, address: int) -> int:
            if not self.live:
                raise rec.MemoryReadError("actor unavailable")
            return 0

        def u32_exact(self, address: int) -> int:
            if not self.live:
                raise rec.MemoryReadError("actor unavailable")
            return 0

    def test_actor_gap_marks_rows_and_resumes(self) -> None:
        mem = self.RecoveringMem()
        run = pathlib.Path(tempfile.mkdtemp())
        writer = rec.ChannelWriter(run, rec.Clock())
        sampler = rec.MemorySampler(mem, rec.Clock(), writer, 0, 0x1000, 0, 0, 0,
                                    recovery=0.6, interval=0.01)
        threading.Timer(0.3, setattr, args=(mem, "live", True)).start()
        sampler.start()
        time.sleep(1.0)
        sampler.stop()
        writer.close()
        self.assertIsNone(sampler.error)
        rows = [json.loads(line) for line in (run / "gcd.jsonl").read_text().splitlines()]
        self.assertTrue(any(row.get("gap") for row in rows))
        self.assertTrue(any(row.get("gcd_remaining") == 0 for row in rows))


class SamplerRefreshTests(unittest.TestCase):
    class NullActorMem:
        """The actor global reads null while the old actor still reads."""

        def __init__(self) -> None:
            self.null = True
            self.local = 0x2000
            self.actor = 0x1000

        def u32(self, address: int) -> int:
            if address == rec.G_CONNECTION - rec.DUMP_BASE:
                return self.local
            if address == self.local + 0x10C:
                return 0 if self.null else self.actor
            return 0

        def u32s_exact(self, address: int, count: int) -> list[int]:
            return [0] * count

        def u8_exact(self, address: int) -> int:
            return 0

        def u32_exact(self, address: int) -> int:
            return 0

    def test_null_actor_enters_recovery_instead_of_stale_rows(self) -> None:
        mem = self.NullActorMem()
        run = pathlib.Path(tempfile.mkdtemp())
        writer = rec.ChannelWriter(run, rec.Clock())
        sampler = rec.MemorySampler(mem, rec.Clock(), writer, 0, 0x1000, 0, 0, 0,
                                    recovery=2.0, interval=0.01,
                                    refresh_interval=0.05)
        threading.Timer(0.2, setattr, args=(mem, "null", False)).start()
        sampler.start()
        time.sleep(0.6)
        sampler.stop()
        writer.close()
        self.assertIsNone(sampler.error)
        rows = [json.loads(line) for line in (run / "gcd.jsonl").read_text().splitlines()]
        self.assertTrue(any(row.get("gap") for row in rows))
        self.assertTrue(any(row.get("gcd_remaining") == 0 and not row.get("gap")
                            for row in rows))


class MonotonicTests(unittest.TestCase):
    def test_backstep_is_reported(self) -> None:
        report = rec.monotonic_report({"x": [{"client_ms": 100}, {"client_ms": 90}]})
        self.assertFalse(report["x"]["ok"])
        self.assertEqual(report["x"]["max_backstep_ms"], -10)


if __name__ == "__main__":
    unittest.main()
