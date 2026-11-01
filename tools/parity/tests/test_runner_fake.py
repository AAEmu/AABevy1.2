"""Offline runner test with recorded MCP replies."""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("parity_run_original",
                                              HERE.parent / "run_original.py")
runner = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(runner)

FIXTURES = HERE / "fixtures"


class FakeRunnerTests(unittest.TestCase):
    def run_mini(self, root: pathlib.Path) -> int:
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            return runner.main([
                str(FIXTURES / "mini.yaml"),
                "--capture-root", str(root),
                "--run-id", "offline",
                "--fake-mcp", str(FIXTURES / "fake_mcp.json"),
                "--pid", "4321",
                "--keep-client",
                "--server-log", str(FIXTURES / "server_trace.txt"),
                "--server-log-from-start",
                "--reply-delay", "600",
            ])

    def test_capture_files(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            self.assertEqual(self.run_mini(root), 0)
            run = root / "mini" / "offline"
            expected = [
                run / "steps.jsonl", run / "client_packets.jsonl",
                run / "clock_sync.jsonl", run / "meta.json",
                run / "ui" / "snap_one.json", run / "ui" / "snap_two.json",
                run / "entities" / "snap_one.json",
                run / "entities" / "snap_two.json",
                run / "shots" / "snap_one.png",
                run / "shots" / "snap_two.png",
            ]
            for path in expected:
                self.assertTrue(path.exists(), str(path))

            steps = [json.loads(line) for line in
                     (run / "steps.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(steps), 8)
            self.assertEqual([row["id"] for row in steps],
                             ["gm_heal", "wait_short", "mark_start", "snap_one",
                              "press", "wait_cond", "snap_two", "end"])
            wait = next(row for row in steps if row["id"] == "wait_cond")
            self.assertEqual(wait["condition_source"], "server")
            self.assertTrue(wait["result"]["met"])
            snap = next(row for row in steps if row["id"] == "snap_one")
            self.assertEqual(snap["result"]["entity_count"], 1)
            self.assertEqual(snap["result"]["ui_widgets"], 1)

            ui = json.loads((run / "ui" / "snap_one.json").read_text(
                encoding="utf-8"))
            self.assertEqual(ui["widgets"][0]["paths"], ["testWindow"])
            entities = json.loads((run / "entities" / "snap_one.json").read_text(
                encoding="utf-8"))
            self.assertEqual(entities["unit_state"]["player"]["name"], "Mini")
            self.assertEqual(entities["animation"]["player"]["state"], "idle")
            self.assertIn("mode_actions", entities)

            with (run / "shots" / "snap_one.png").open("rb") as handle:
                self.assertEqual(handle.read(8), b"\x89PNG\r\n\x1a\n")

            packets = [json.loads(line) for line in
                       (run / "client_packets.jsonl").read_text(
                           encoding="utf-8").splitlines()]
            self.assertEqual(len(packets), 3)
            self.assertEqual([row["name"] for row in packets],
                             ["CSMoveUnitPacket", "PingPacket",
                              "CSStartSkillPacket"])
            self.assertEqual(packets[0]["client_ms"], 1_000_000)
            self.assertEqual(packets[0]["clock"], "packet")
            self.assertEqual(packets[2]["clock"], "nearest_anchor")
            self.assertTrue(-100 <= packets[0]["t_ms"] <= 100)

            meta = json.loads((run / "meta.json").read_text(encoding="utf-8"))
            self.assertTrue(meta["result"]["ok"])
            self.assertEqual(meta["client"]["pid"], 4321)
            self.assertEqual(meta["reply_delay"]["hint_ms"], 600)
            self.assertIn("fields=trace", meta["server"]["field_trace"])
            self.assertEqual(meta["packets"]["anchors"], {"move": 1, "ping": 1})
            self.assertEqual(meta["steps"]["snapshots"], ["snap_one", "snap_two"])
            self.assertIsNotNone(meta["clock"]["start_client_ms"])

    def test_portable_verbs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                code = runner.main([
                    str(FIXTURES / "verbs.yaml"),
                    "--capture-root", str(root),
                    "--run-id", "verbs",
                    "--fake-mcp", str(FIXTURES / "fake_mcp_verbs.json"),
                    "--pid", "4321", "--keep-client",
                ])
            self.assertEqual(code, 0, output.getvalue())
            run = root / "verbs" / "verbs"
            rows = [json.loads(line) for line in
                    (run / "steps.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertEqual([row["verb"] for row in rows],
                             ["move-to", "target", "use-skill", "use-skill",
                              "click-ui", "interact", "mount", "dismount",
                              "set-slots"])
            self.assertEqual(rows[1]["result"]["method"], "findobject")
            self.assertEqual(rows[1]["result"]["state"]["name"], "Target")
            self.assertEqual(rows[2]["result"]["key"], "3")
            self.assertEqual(rows[3]["result"]["binded_type"], "function")
            self.assertEqual(rows[4]["result"]["point"], [60, 45])
            self.assertEqual(rows[7]["result"]["kind"], "pet")
            self.assertEqual(rows[8]["result"], "3:Spell:Melee Attack:true")

    def test_condition_timeout_fails_and_keeps_files(self) -> None:
        scenario_text = (
            "name: broken\n"
            "version: 1\n"
            "steps:\n"
            "  - id: never\n"
            "    wait:\n"
            "      until: \"vehicle.mounted == true\"\n"
            "      timeout_ms: 60\n"
            "      poll_ms: 10\n"
            "      source: server\n")
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            scenario_path = root / "broken.yaml"
            scenario_path.write_text(scenario_text, encoding="utf-8")
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                code = runner.main([
                    str(scenario_path), "--capture-root", str(root),
                    "--run-id", "fail",
                    "--fake-mcp", str(FIXTURES / "fake_mcp.json"),
                    "--pid", "4321", "--keep-client",
                ])
            self.assertEqual(code, 1)
            run = root / "broken" / "fail"
            rows = [json.loads(line) for line in
                    (run / "steps.jsonl").read_text(encoding="utf-8").splitlines()]
            self.assertFalse(rows[0]["result"]["met"])
            meta = json.loads((run / "meta.json").read_text(encoding="utf-8"))
            self.assertFalse(meta["result"]["ok"])
            self.assertIn("condition not met", meta["result"]["error"])


if __name__ == "__main__":
    unittest.main()
