import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import _setup  # noqa: F401
import common
import diff
import mcp_driver
import plan as planlib
import run_original as ro

HERE = Path(__file__).resolve().parent
FAKE = HERE / "fake_driver.py"


def tiny_plan():
    skills = [
        {"id": 14376, "name": "Earthen Grip", "slot": 1, "key": "1", "batch": 1, "target": "unit", "select": "dummy", "cast_ms": 1000,
         "channel_ms": 0, "wait_ms": 500, "cooldown_ms": 15000},
        {"id": 10712, "name": "Purge", "slot": 2, "key": "2", "batch": 1, "target": "self", "select": "self", "cast_ms": 0,
         "channel_ms": 0, "wait_ms": 400, "cooldown_ms": 30000, "stop_after_ms": 200},
    ]
    batch = {"batch": 1, "tree": 2, "tree_name": "Witchcraft", "weapon": {"name": "staff"},
             "slots": {"1": 14376, "2": 10712}, "skills": skills}
    return {"schema": planlib.PLAN_SCHEMA, "slots_per_page": 10, "dummy": {"template": 13013, "name": "Forward Scarecrow"},
            "setup": {"gm": ["/level 50"]}, "batches": [batch], "skills": skills}


def driver_args(extra=()):
    fake = str(FAKE)
    return ["--pid", "1234", "--driver", fake, "--dll", fake, "--launcher", fake, "--client", fake, "--login", "x:1", "--user", "u",
            *extra]


def quiet(fn, *a):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = fn(*a)
    return code, out.getvalue(), err.getvalue()


class Converters(unittest.TestCase):
    def test_clock_map_uses_the_sample_with_the_smallest_round_trip(self):
        clock = ro.ClockMap("ui")
        clock.add(10_000, 1_000_000, 1_010_000)            # 10 ms round trip: mid 1_005_000
        clock.add(20_000, 2_000_000, 2_001_000)            # 1 ms round trip: mid 2_000_500, offset -17_999_500 us
        self.assertEqual(clock.wall_us(20_000), 2_000_500)
        self.assertEqual(clock.wall_us(20_100), 2_000_500 + 100_000)
        self.assertEqual(ro.ClockMap("x").wall_us(5), None)
        report = clock.report()
        self.assertEqual((report["samples"], report["min_rtt_ms"]), (2, 1.0))

    def test_ui_lines_become_castbar_and_cooldown_rows(self):
        clock = ro.ClockMap("ui")
        clock.add(0, 1_000_000, 1_000_000)
        rows = ro.convert_ui_rows([
            "C|100|start|Earthen Grip|1000|0|", "C|1100|stop|Earthen Grip|1000|1000|",
            "K|150|1|1000|1000|15000|15000|start", "K|16150|1|0|0|0|0|stop", "junk", "C|bad"],
            clock, {"channel_ms": 0, "cast_ms": 1000})
        kinds = [(r["kind"], r.get("state"), r.get("gcd")) for r in rows]
        self.assertEqual(kinds, [("castbar", "start", None), ("castbar", "stop", None), ("cooldown", None, False),
                                 ("cooldown", None, True), ("cooldown", None, False)])
        start = rows[0]
        self.assertEqual((start["wall_us"], start["total_ms"], start["name"], start["unit"], start["channel"]),
                         (1_100_000, 1000, "Earthen Grip", "player", False))
        skill_cd, gcd_cd = rows[2], rows[3]
        self.assertEqual((skill_cd["total_ms"], skill_cd["remain_ms"], skill_cd["slot"]), (15000, 15000, 1))
        self.assertEqual((gcd_cd["total_ms"], gcd_cd["remain_ms"]), (1000, 1000))
        self.assertEqual(rows[4]["remain_ms"], 0)

    def test_channel_flag_only_when_the_plan_says_a_channel_without_a_cast(self):
        clock = ro.ClockMap("ui")
        clock.add(0, 0, 0)
        row = ro.convert_ui_rows(["C|5|start|Play Dead|60000|0|"], clock, {"channel_ms": 60000, "cast_ms": 0})[0]
        self.assertIs(row["channel"], True)
        row = ro.convert_ui_rows(["C|5|start|X|1000|0|"], clock, {"channel_ms": 500, "cast_ms": 1000})[0]
        self.assertNotIn("channel", row)
        row = ro.convert_ui_rows(["C|5|start|X|1000|0|"], clock, None)[0]
        self.assertNotIn("channel", row)

    def test_system_lines_become_pos_rows_in_game_coordinates(self):
        clock = ro.ClockMap("system")
        clock.add(7000, 5_000_000, 5_000_000)
        origin = ro.parse_origin("1000.0,2000.0,0.0")
        rows = ro.convert_sys_rows(["P|7100.0|player|10.5|20.25|30.0|-90.00", "P|7200|target|1|2|3|", "P|bad"], clock, origin)
        self.assertEqual(len(rows), 2)
        self.assertEqual((rows[0]["unit"], rows[0]["x"], rows[0]["y"], rows[0]["z"], rows[0]["yaw"]), ("player", 1010.5, 2020.25, 30.0, 270.0))
        self.assertEqual(rows[0]["wall_us"], 5_100_000)
        self.assertNotIn("yaw", rows[1])                                    # not known: left out
        self.assertEqual(ro.convert_sys_rows(["P|7100|player|1|2|3|"], clock, None)[0]["x"], 1.0)
        self.assertIsNone(ro.parse_origin("nonsense"))

    def test_lua_helpers(self):
        code = ro.lua_timer("sp_ui", 1, "return ''")
        self.assertIn('"sp_ui"', code)
        self.assertIn("Script.SetTimer(1, tick)", code)
        self.assertNotIn("SAMPLE", code)
        self.assertEqual(ro.lua_str('a"b\\c'), '"a\\"b\\\\c"')
        self.assertIn("'_rows'", ro.TAKE_ROWS % (ro.lua_str("sp_ui"), ro.lua_str("sp_ui")))


class EndToEnd(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._settle = ro.SETTLE_S
        ro.SETTLE_S = 0.05

    @classmethod
    def tearDownClass(cls):
        ro.SETTLE_S = cls._settle

    def run_it(self, root, extra=(), tools=None):
        plan_path = Path(root) / "plan.json"
        plan_path.write_text(json.dumps(tiny_plan()), encoding="utf-8")
        argv = ["run", *driver_args(), "--plan", str(plan_path), "--capture-root", str(root), "--run-id", "t1", *extra]
        return quiet(ro.main, argv)

    def test_run_writes_trace_meta_and_video(self):
        with tempfile.TemporaryDirectory() as d:
            code, out, err = self.run_it(d, ["--video", "--ffmpeg", "unused"])
            self.assertEqual(code, 0, out + err)
            orig = Path(d) / "t1" / "orig"
            rows, bad = common.read_jsonl(orig / "trace.jsonl")
            self.assertEqual(bad, 0)
            kinds = common.kinds_in(rows)
            self.assertEqual(kinds["mark"], 4)
            self.assertEqual(kinds["press"], 2)                                  # the driver's rows: none added by the runner
            self.assertFalse([r for r in rows if r.get("source") == "runner"])
            marks = [r["label"] for r in rows if r["kind"] == "mark"]
            self.assertEqual(marks, ["skill:14376:start", "skill:14376:end", "skill:10712:start", "skill:10712:end"])
            # the original's anim rows had no role: the README rule filled them in
            anims = [r for r in rows if r["kind"] == "anim"]
            self.assertTrue(anims)
            self.assertEqual({r["role"] for r in anims if r["inst"] == "0xAA01"}, {"player"})
            self.assertEqual({r["role"] for r in anims if r["inst"] == "0xBB02"}, {"target"})
            self.assertEqual(sorted(r["wall_us"] for r in rows), [r["wall_us"] for r in rows])
            # sampler rows
            cast = [r for r in rows if r["kind"] == "castbar"]
            self.assertEqual([r["state"] for r in cast][:2], ["start", "stop"])
            self.assertEqual((cast[0]["name"], cast[0]["total_ms"], cast[0]["unit"]), ("Fake Cast", 1500, "player"))
            cd = [r for r in rows if r["kind"] == "cooldown" and r["remain_ms"] > 0]
            self.assertTrue([r for r in cd if r["gcd"] is False and r["total_ms"] == 15000])
            self.assertTrue([r for r in cd if r["gcd"] is True and r["total_ms"] == 1000])
            pos = [r for r in rows if r["kind"] == "pos"]
            self.assertEqual((pos[0]["x"], pos[0]["y"], pos[0]["z"]), (1010.0, 2020.0, 30.0))   # + the seamless origin
            self.assertTrue([r for r in pos if r["unit"] == "target"])
            # the trace timing is alignable: the press comes after its start mark, the cast bar within ~100 ms of it
            trace = diff.load_trace(rows, "orig")
            segs = diff.find_segments(trace)
            self.assertEqual([s.skill_id for s in segs], [14376, 10712])
            self.assertTrue(all(s.press_us and s.press_us >= s.start_us for s in segs))
            m = diff.client_metrics(trace, None, segs[0], tiny_plan()["skills"][0])
            self.assertAlmostEqual(m["castbar"]["casts"][0]["start_ms"], 45, delta=40)
            self.assertAlmostEqual(m["anim"]["first_skill_ms"], 40, delta=40)
            self.assertEqual(m["fx"]["fx.test.bolt"]["count"], 1)
            # meta and video
            meta = json.loads((orig / "meta.json").read_text(encoding="utf-8"))
            self.assertEqual((meta["src"], meta["character"], meta["ok"]), ("orig", "FakeChar", True))
            self.assertEqual(meta["roles"]["player_inst"], "0xAA01")
            self.assertEqual([s["id"] for s in meta["plan"]["skills"]], [14376, 10712])
            self.assertGreater(meta["clock"]["ui"]["samples"], 3)
            self.assertEqual(meta["origin"], [1000.0, 2000.0, 0.0])
            video = json.loads((orig / "video.json").read_text(encoding="utf-8"))
            self.assertEqual((video["fps"], video["width"]), (30, 1280))
            self.assertTrue((orig / "video.mp4").is_file())

    def test_driver_without_client_trace_fails_clearly_and_writes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            plan_path = Path(d) / "plan.json"
            plan_path.write_text(json.dumps(tiny_plan()), encoding="utf-8")
            # a driver script that lists no client_trace: this runs fake_driver.py with --tools
            wrapper = Path(d) / "old_driver.py"
            wrapper.write_text(
                "import runpy, sys\nsys.argv = [sys.argv[0], '--tools', 'client_attach,client_status,client_lua,client_input']\n"
                f"runpy.run_path({str(FAKE)!r}, run_name='__main__')\n", encoding="utf-8")
            argv = ["run", "--pid", "1", "--driver", str(wrapper), "--dll", str(FAKE), "--launcher", str(FAKE), "--client", str(FAKE),
                    "--plan", str(plan_path), "--capture-root", d, "--run-id", "t2"]
            code, out, err = quiet(ro.main, argv)
            self.assertEqual(code, 3)
            self.assertIn("client_trace", err)
            self.assertFalse((Path(d) / "t2" / "orig" / "trace.jsonl").exists())

    def test_an_existing_trace_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(self.run_it(d)[0], 0)
            with self.assertRaises(SystemExit) as ctx:
                self.run_it(d)
            self.assertIn("exists", str(ctx.exception))

    def test_ground_skill_without_a_click_is_skipped_with_a_warning(self):
        with tempfile.TemporaryDirectory() as d:
            p = tiny_plan()
            p["batches"][0]["skills"][0]["target"] = "ground"
            plan_path = Path(d) / "plan.json"
            plan_path.write_text(json.dumps(p), encoding="utf-8")
            argv = ["run", *driver_args(), "--plan", str(plan_path), "--capture-root", d, "--run-id", "g", "--skills", "14376"]
            code, out, err = quiet(ro.main, argv)
            self.assertEqual(code, 0)
            meta = json.loads((Path(d) / "g" / "orig" / "meta.json").read_text(encoding="utf-8"))
            self.assertEqual(meta["skipped"][0]["id"], 14376)
            self.assertFalse(meta["ok"])
            rows, _ = common.read_jsonl(Path(d) / "g" / "orig" / "trace.jsonl")
            self.assertEqual(common.kinds_in(rows).get("mark", 0), 0)

    def test_setup_and_bar_commands(self):
        with tempfile.TemporaryDirectory() as d:
            plan_path = Path(d) / "plan.json"
            plan_path.write_text(json.dumps(tiny_plan()), encoding="utf-8")
            code, out, err = quiet(ro.main, ["bar", *driver_args(), "--plan", str(plan_path), "--batch", "1"])
            self.assertEqual(code, 0, out + err)
            self.assertIn("slots", out)
            code, out, err = quiet(ro.main, ["setup", *driver_args(), "--plan", str(plan_path), "--gm", "/ignoreskillcds true", "--move", "1 2 3"])
            self.assertEqual(code, 0, out + err)
            self.assertIn("gm /level 50", out)
            self.assertIn("gm /ignoreskillcds true", out)
            self.assertIn("gm /move 1 2 3", out)

    def test_roles_command_rewrites_a_trace_in_place(self):
        with tempfile.TemporaryDirectory() as d:
            import make_fixtures as fx
            path = Path(d) / "trace.jsonl"
            common.write_jsonl(path, fx.build_orig())
            code, out, _ = quiet(ro.main, ["roles", str(path)])
            self.assertEqual(code, 0)
            self.assertEqual(json.loads(out)["player_inst"], fx.PLAYER_ORIG)
            rows, _ = common.read_jsonl(path)
            self.assertEqual({r["role"] for r in rows if r["kind"] == "anim" and r["inst"] == fx.PLAYER_ORIG}, {"player"})


class Driver(unittest.TestCase):
    def test_tools_and_require(self):
        fake = str(FAKE)
        m = mcp_driver.Mcp(Path(fake), None, None, None, None, "x:1", "u", None)
        try:
            self.assertIn("client_trace", m.tools())
            m.require("client_trace", "client_video")
            with self.assertRaises(mcp_driver.MissingToolError):
                m.require("client_nonexistent")
            with self.assertRaises(mcp_driver.MissingToolError):
                m.tool("client_nonexistent")
            self.assertEqual(m.text("client_status", pid=1).split()[0], "pid=1")
        finally:
            m.close()

class WorldFxTest(unittest.TestCase):
    def test_the_seamless_origin_is_added_to_an_effect_position(self):
        row = {"kind": "fx", "pos": [4266.17, 5087.998, 132.65]}
        ro.world_fx(row, (11264.0, 10240.0, 0.0))
        self.assertEqual(row["pos"], [15530.17, 15327.998, 132.65])
        self.assertEqual(row["pos_local"], [4266.17, 5087.998, 132.65])
        ro.world_fx(row, (11264.0, 10240.0, 0.0))
        self.assertEqual(row["pos"], [15530.17, 15327.998, 132.65], "applied once")

    def test_a_zero_position_and_other_rows_are_left_alone(self):
        row = {"kind": "fx", "pos": [0, 0, 0]}
        self.assertEqual(ro.world_fx(row, (11264.0, 10240.0, 0.0))["pos"], [0, 0, 0])
        other = {"kind": "sound", "pos": [1.0, 2.0, 3.0]}
        self.assertEqual(ro.world_fx(other, (11264.0, 10240.0, 0.0))["pos"], [1.0, 2.0, 3.0])


if __name__ == "__main__":
    unittest.main()
