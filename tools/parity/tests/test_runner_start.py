"""Startup detection, stop policy and MCP timeout tests."""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import pathlib
import tempfile
import textwrap
import unittest
from argparse import Namespace
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("parity_runner_start",
                                              HERE.parent / "run_original.py")
runner = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(runner)

FIXTURES = HERE / "fixtures"


def make_lane(root: pathlib.Path, name: str = "slow") -> tuple[pathlib.Path, pathlib.Path]:
    lanes = root / "lanes"
    lanes.mkdir()
    server = root / "server"
    (server / "Scripts").mkdir(parents=True)
    (server / "Scripts" / "live-stack.sh").write_text("exit 0\n", encoding="utf-8")
    (lanes / f"{name}.env").write_text(
        f"LANE={name}\nBRANCH=test/{name}\nPORT_PREFIX=99\n"
        f"WT={server.as_posix()}\nLIVE={(root / 'live').as_posix()}\n",
        encoding="utf-8")
    return lanes, server


class StartTests(unittest.TestCase):
    def test_slow_start_keeps_polling_after_script_exit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            lanes, server = make_lane(root)
            calls: list[int] = []

            def fake_pid(port: int) -> int | None:
                calls.append(port)
                return 4321 if len(calls) >= 4 else None

            with mock.patch.object(runner, "lane_client_pid",
                                   side_effect=fake_pid):
                pid, info = runner.start_lane_client(
                    "slow", lanes, server, 5.0, root / "start.log",
                    stable_s=0.1)
            self.assertEqual(pid, 4321)
            self.assertEqual(info["source"], "lane")
            self.assertEqual(len(info["attempts"]), 1)
            self.assertGreaterEqual(len(calls), 5)

    def test_retry_only_after_known_pid_died(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            lanes, server = make_lane(root)
            calls: list[int] = []

            def fake_pid(port: int) -> int | None:
                calls.append(port)
                if len(calls) == 1:
                    return None
                if len(calls) == 2:
                    return 111
                if len(calls) <= 5:
                    return None
                return 222

            with mock.patch.object(runner, "lane_client_pid",
                                   side_effect=fake_pid):
                pid, info = runner.start_lane_client(
                    "slow", lanes, server, 5.0, root / "start.log",
                    stable_s=0.1)
            self.assertEqual(pid, 222)
            self.assertEqual(info["source"], "lane")
            self.assertEqual(len(info["attempts"]), 2)
            self.assertEqual(info["attempts"][0]["pid"], 111)
            self.assertEqual(info["attempts"][1]["pid"], 222)

    def test_existing_lane_client_is_adopted_not_started(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            lanes, server = make_lane(root)
            with mock.patch.object(runner, "lane_client_pid",
                                   return_value=777):
                pid, info = runner.start_lane_client(
                    "slow", lanes, server, 2.0, root / "start.log",
                    stable_s=0.1)
            self.assertEqual(pid, 777)
            self.assertEqual(info["source"], "existing")
            self.assertEqual(len(info["attempts"]), 1)


class StopPolicyTests(unittest.TestCase):
    def test_fake_run_never_stops_a_pid(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            output = io.StringIO()
            with mock.patch.object(runner, "stop_pid") as stop:
                with contextlib.redirect_stdout(output):
                    code = runner.main([
                        str(FIXTURES / "mini.yaml"),
                        "--capture-root", str(root),
                        "--run-id", "fake",
                        "--fake-mcp", str(FIXTURES / "fake_mcp.json"),
                        "--pid", "1",
                    ])
            self.assertEqual(code, 0, output.getvalue())
            stop.assert_not_called()
            meta = json.loads((root / "mini" / "fake" / "meta.json").read_text(
                encoding="utf-8"))
            self.assertFalse(meta["client"]["started_by_runner"])
            self.assertFalse(meta["client"]["stopped"])


    def test_adopted_lane_client_is_not_stopped(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            lanes, server = make_lane(root)
            fixture = runner.FixtureMcp(FIXTURES / "fake_mcp.json")
            output = io.StringIO()
            with mock.patch.object(runner, "stop_pid") as stop:
                with mock.patch.object(
                        runner, "start_lane_client",
                        return_value=(4321, {"source": "existing",
                                             "attempts": []})):
                    with mock.patch.object(runner, "Mcp",
                                           return_value=fixture):
                        with contextlib.redirect_stdout(output):
                            code = runner.main([
                                str(FIXTURES / "mini.yaml"),
                                "--capture-root", str(root),
                                "--run-id", "adopted",
                                "--lane", "slow",
                                "--lanes-dir", str(lanes),
                                "--server-dir", str(server),
                            ])
            self.assertEqual(code, 0, output.getvalue())
            stop.assert_not_called()
            meta = json.loads((root / "mini" / "adopted" / "meta.json").read_text(
                encoding="utf-8"))
            self.assertFalse(meta["client"]["started_by_runner"])
            self.assertFalse(meta["client"]["stopped"])


class CaptureRootTests(unittest.TestCase):
    def test_default_is_under_repo_root(self) -> None:
        with mock.patch.dict("os.environ", {"AA_PARITY_ROOT": ""}):
            root = runner.capture_root(Namespace(capture_root=None))
        self.assertEqual(root, runner.ROOT / "data/local/parity")


class McpTimeoutTests(unittest.TestCase):
    def test_hung_driver_times_out(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            driver = root / "fake_driver.py"
            driver.write_text(textwrap.dedent("""
                import json
                import sys
                import time
                for line in sys.stdin:
                    request = json.loads(line)
                    if request.get("method") == "initialize":
                        sys.stdout.write(json.dumps({
                            "jsonrpc": "2.0", "id": request["id"],
                            "result": {"protocolVersion": "2025-11-25",
                                       "capabilities": {}}}) + "\\n")
                        sys.stdout.flush()
                    elif request.get("method") == "tools/call":
                        time.sleep(5)
            """), encoding="utf-8")
            mcp = runner.Mcp(driver, root / "dll", root / "launcher",
                             root / "client", None, "127.0.0.1:1", "user",
                             root / "artifacts")
            try:
                with self.assertRaisesRegex(RuntimeError, "timed out"):
                    mcp.call("tools/call",
                             {"name": "client_lua", "arguments": {}},
                             timeout_s=0.2)
            finally:
                mcp.process.kill()
                mcp.close()


if __name__ == "__main__":
    unittest.main()
