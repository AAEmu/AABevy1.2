import contextlib
import io
import json
import os
import stat
import sys
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path

import _setup  # noqa: F401
import common
import plan as planlib
import run_bevy as rb
from test_run_original import tiny_plan

HERE = Path(__file__).resolve().parent
FAKE = HERE / "fake_client.py"
KNOWN_WORDS = {"gm", "tp", "target", "key", "wait", "lua", "echo", "stopcast", "uiclick", "trace", "orbit", "zoom", "pitch"}


def script_for(plan=None, **kw):
    plan = plan or tiny_plan()
    kw.setdefault("run_id", "r1")
    text, skipped = rb.build_script(plan, plan["batches"], **kw)
    return text, skipped


def quiet(fn, *a):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = fn(*a)
    return code, out.getvalue(), err.getvalue()


class BuildScript(unittest.TestCase):
    def lines(self, text):
        return [l for l in text.splitlines() if l.strip() and not l.startswith("#")]

    def test_only_known_script_commands_and_a_comment_free_lua_line(self):
        text, skipped = script_for()
        self.assertEqual(skipped, [])
        for line in self.lines(text):
            self.assertIn(line.split()[0], KNOWN_WORDS, line)
        lua = [l for l in self.lines(text) if l.startswith("lua ")]
        self.assertEqual(len(lua), 1)
        self.assertNotIn("#", lua[0])
        self.assertIn("X2Action:SetActionSpell(1, 14376)", lua[0])
        self.assertIn("X2Action:SetActionSpell(2, 10712)", lua[0])

    def test_each_skill_is_target_start_mark_key_wait_end_mark(self):
        text, _ = script_for(setup=False, spawn_doll=False)
        lines = self.lines(text)
        i = lines.index("trace skill:14376:start")
        self.assertEqual(lines[i - 2:i], ["target template:13013", "wait 0.3"])
        self.assertEqual(lines[i + 1], "key slot1")
        self.assertEqual(lines[i + 2], "wait 0.5")                       # wait_ms 500
        self.assertEqual(lines[i + 3], "trace skill:14376:end")
        j = lines.index("trace skill:10712:start")
        self.assertEqual(lines[j - 2], "target self")                      # a self skill
        # a skill with stop_after: wait, stopcast, wait the rest
        self.assertEqual(lines[j + 1:j + 6], ["key slot2", "wait 0.2", "stopcast", "wait 0.2", "trace skill:10712:end"])

    def test_setup_move_doll_and_extra_gm_lines(self):
        text, _ = script_for(move="15530 15323 131", extra_gm=["/ignoreskillcds true"])
        lines = self.lines(text)
        self.assertIn("gm level 50", lines)
        self.assertIn("gm ignoreskillcds true", lines)
        self.assertEqual(lines[lines.index("tp 15530 15323 131") + 1], "wait 2")
        self.assertIn("gm spawn npc 13013", lines)
        bare, _ = script_for(setup=False, spawn_doll=False)
        self.assertNotIn("gm ", bare)

    def test_ground_skills_need_a_click_position(self):
        plan = tiny_plan()
        plan["batches"][0]["skills"][0]["target"] = "ground"
        text, skipped = script_for(plan)
        self.assertEqual([s["id"] for s in skipped], [14376])
        self.assertNotIn("skill:14376:start", text)
        self.assertIn("# skill 14376", text)                              # said in a comment
        text, skipped = script_for(plan, ground_click=(960, 540))
        self.assertEqual(skipped, [])
        lines = self.lines(text)
        i = lines.index("key slot1")
        self.assertEqual(lines[i + 1:i + 3], ["wait 1.2", "uiclick 960 540"])
        self.assertEqual(lines[i + 3], "wait 0")                          # the reticle time came out of the 500 ms wait

    def test_echo_marks_and_video_grace(self):
        text, _ = script_for(mark_command="echo", video_grace=3.0)
        self.assertIn("echo skill:14376:start", text)
        self.assertNotIn("trace ", text)
        text, _ = script_for(video_grace=3.0)
        lines = self.lines(text)
        i = lines.index("trace " + rb.RECORD_START)
        self.assertEqual(lines[i + 1], "wait 3")
        self.assertNotIn(rb.RECORD_START, script_for()[0])

    def test_script_has_no_machine_paths(self):
        text, _ = script_for(move="1 2 3")
        self.assertNotIn(":\\", text)
        self.assertNotIn("/Users/", text)


class Helpers(unittest.TestCase):
    def test_ffmpeg_args_grab_one_window_with_wallclock_timestamps(self):
        args = rb.ffmpeg_args("ffmpeg", "AAEMU 1.2", Path("out.mp4"), 30)
        self.assertEqual(args[0], "ffmpeg")
        self.assertIn("-use_wallclock_as_timestamps", args)
        self.assertIn("-debug_ts", args)
        self.assertEqual(args[args.index("-i") + 1], "title=AAEMU 1.2")
        self.assertEqual(args[args.index("-fps_mode") + 1], "cfr")
        self.assertEqual(args[-1], "out.mp4")
        with self.assertRaises(ValueError):
            rb.ffmpeg_args("ffmpeg", "x", Path("o.mp4"), 30, grab="x11")

    def test_pts_regex_reads_the_wall_clock_of_the_first_video_packet(self):
        line = ("demuxer -> ist_index:0 type:video next_dts:NOPTS next_dts_time:NOPTS next_pts:NOPTS next_pts_time:NOPTS "
                "pkt_pts:1762171234567890 pkt_pts_time:1762171234.56789 pkt_dts:1762171234567890 off:0 off_time:0")
        self.assertEqual(rb.PTS_RE.search(line).group(1), "1762171234567890")
        self.assertIsNone(rb.PTS_RE.search(line.replace("type:video", "type:audio")))
        m = rb.SIZE_RE.search("  Stream #0:0: Video: bmp, bgra, 1280x720, 30 fps, 30 tbr, 1000k tbn")
        self.assertEqual((m.group(1), m.group(2)), ("1280", "720"))

    def test_trace_has_mark(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "t.jsonl"
            self.assertFalse(rb.trace_has_mark(path, "x"))
            path.write_text('{"kind":"press","key":"1"}\n{"kind":"mark","label":"skillparity:record-start"}\n', encoding="utf-8")
            self.assertTrue(rb.trace_has_mark(path, rb.RECORD_START))
            self.assertFalse(rb.trace_has_mark(path, "skill:1:start"))
            path.write_text('{"kind":"echo","text":"skillparity:record-start"}\n', encoding="utf-8")
            self.assertFalse(rb.trace_has_mark(path, rb.RECORD_START))

    def test_client_command(self):
        args = Namespace(exe=Path("aa-client"), account="acc", login="h:1", password_file=Path("pw"), character="Ch", headless=True,
                         client_arg=["--x"])
        cmd = rb.client_command(args, Path("plan.txt"))
        self.assertEqual(cmd[:5], ["aa-client", "--account", "acc", "--login", "h:1"])
        self.assertIn("--require-wall-clock", cmd)
        self.assertEqual(cmd[cmd.index("--script") + 1], "plan.txt")
        self.assertEqual(cmd[-2:], ["--headless", "--x"])
        self.assertEqual(rb.parse_pair("960, 540"), (960, 540))
        self.assertIsNone(rb.parse_pair(""))

    def test_window_recorder_reads_the_first_frame_time_and_size(self):
        class Err:
            def __iter__(self):
                yield "Input #0, gdigrab, from 'title=AAEMU 1.2':\n"
                yield "  Stream #0:0: Video: bmp, bgra, 1280x720, 30 fps, 30 tbr, 1000k tbn\n"
                yield "demuxer -> ist_index:0 type:video next_dts:NOPTS pkt_pts:5 pkt_pts_time:0\n"          # not wall clock: ignored
                yield "demuxer -> ist_index:0 type:video next_dts:NOPTS pkt_pts:1762171234567890 pkt_pts_time:1\n"
                yield "demuxer -> ist_index:0 type:video next_dts:NOPTS pkt_pts:1762171234600000 pkt_pts_time:1\n"
                yield "frame=  3 fps=0.0 q=-1.0 Lsize=      1kB\n"

        class Stdin:
            data = ""

            def write(self, text):
                self.data += text

            def flush(self):
                pass

        class Proc:
            stderr = Err()
            stdin = Stdin()
            returncode = 0

            def poll(self):
                return None

            def wait(self, timeout=None):
                return 0

        rec = rb.WindowRecorder("ffmpeg", "AAEMU 1.2", Path("v.mp4"), 30)
        rec.process = Proc()
        rec.spawned_us = 7
        rec._read()
        self.assertTrue(rec.wait_first_frame(0.1))
        self.assertEqual((rec.first_us, rec.size), (1762171234567890, (1280, 720)))
        info = rec.stop()
        self.assertEqual((info["t0_wall_us"], info["width"], info["height"], info["ffmpeg_exit"]), (1762171234567890, 1280, 720, 0))
        self.assertIn("gdigrab", info["method"])
        self.assertEqual(rec.process.stdin.data, "q\n")
        self.assertTrue(rec.tail)                                      # short tail of non-frame lines

        rec2 = rb.WindowRecorder("ffmpeg", "t", Path("v.mp4"), 30)
        rec2.process = Proc()
        rec2.spawned_us = 99
        info = rec2.stop()                                             # no frame time read: falls back to the spawn time
        self.assertEqual((info["t0_wall_us"], info["method"]), (99, "spawn"))
        self.assertIn("warning", info)


def make_launcher(directory: Path) -> Path:
    """An executable that runs fake_client.py with the arguments it is given."""
    if os.name == "nt":
        path = directory / "fake-aa-client.cmd"
        path.write_text(f'@"{sys.executable}" "{FAKE}" %*\r\n', encoding="utf-8")
    else:
        path = directory / "fake-aa-client.sh"
        path.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{FAKE}" "$@"\n', encoding="utf-8")
        path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.plan = self.root / "plan.json"
        self.plan.write_text(json.dumps(tiny_plan()), encoding="utf-8")

    def test_dry_run_writes_the_script_and_meta_only(self):
        code, out, err = quiet(rb.main, ["--plan", str(self.plan), "--capture-root", str(self.root), "--run-id", "d", "--dry-run"])
        self.assertEqual(code, 0, err)
        bevy = self.root / "d" / "bevy"
        self.assertEqual(sorted(p.name for p in bevy.iterdir()), ["meta.json", "plan.txt"])
        self.assertNotIn("\r", (bevy / "plan.txt").read_bytes().decode())          # LF endings for the client
        meta = json.loads((bevy / "meta.json").read_text(encoding="utf-8"))
        self.assertEqual((meta["src"], meta["run_id"], len(meta["plan"]["skills"])), ("bevy", "d", 2))

    def test_capture_root_is_required(self):
        saved = os.environ.pop("AA_PARITY_ROOT", None)
        try:
            code, _, err = quiet(rb.main, ["--plan", str(self.plan), "--dry-run"])
        finally:
            if saved is not None:
                os.environ["AA_PARITY_ROOT"] = saved
        self.assertEqual(code, 2)
        self.assertIn("capture-root", err)

    def test_missing_exe_and_empty_selection_are_clean_errors(self):
        code, _, err = quiet(rb.main, ["--plan", str(self.plan), "--capture-root", str(self.root), "--run-id", "e", "--account", "a"])
        self.assertEqual(code, 2)
        self.assertIn("--exe", err)
        code, _, err = quiet(rb.main, ["--plan", str(self.plan), "--capture-root", str(self.root), "--trees", "9", "--dry-run"])
        self.assertEqual(code, 2)
        code, _, err = quiet(rb.main, ["--plan", str(self.plan), "--capture-root", str(self.root), "--video", "--headless", "--dry-run"])
        self.assertEqual(code, 2)

    def test_run_against_a_fake_client_collects_its_trace(self):
        launcher = make_launcher(self.root)
        argv = ["--plan", str(self.plan), "--capture-root", str(self.root), "--run-id", "x", "--exe", str(launcher),
                "--account", "acc", "--login", "127.0.0.1:1", "--character", "Ch", "--timeout", "60"]
        code, out, err = quiet(rb.main, argv)
        self.assertEqual(code, 0, out + err)
        bevy = self.root / "x" / "bevy"
        rows, bad = common.read_jsonl(bevy / "trace.jsonl")
        self.assertEqual(bad, 0)
        self.assertEqual([r["label"] for r in rows if r["kind"] == "mark"],
                         ["skill:14376:start", "skill:14376:end", "skill:10712:start", "skill:10712:end"])
        self.assertEqual(common.kinds_in(rows)["press"], 2)
        meta = json.loads((bevy / "meta.json").read_text(encoding="utf-8"))
        self.assertTrue(meta["ok"])
        self.assertEqual((meta["exit_code"], meta["trace_rows"]), (0, len(rows)))
        self.assertIn("--require-wall-clock", meta["command"])
        self.assertIn("fake client done", (bevy / "client.log").read_text(encoding="utf-8", errors="replace"))
        # a second run into the same directory is refused, --force replaces
        code, _, err = quiet(rb.main, argv)
        self.assertEqual(code, 2)
        self.assertIn("exists", err)
        self.assertEqual(quiet(rb.main, argv + ["--force"])[0], 0)
        rows2, _ = common.read_jsonl(bevy / "trace.jsonl")
        self.assertEqual(len(rows2), len(rows))

    def test_a_client_without_trace_support_is_reported(self):
        # a launcher that does nothing: no AA_SKILL_TRACE rows
        if os.name == "nt":
            exe = self.root / "noop.cmd"
            exe.write_text("@exit /b 0\r\n", encoding="utf-8")
        else:
            exe = self.root / "noop.sh"
            exe.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
            exe.chmod(exe.stat().st_mode | stat.S_IXUSR)
        code, out, err = quiet(rb.main, ["--plan", str(self.plan), "--capture-root", str(self.root), "--run-id", "n", "--exe", str(exe),
                                         "--account", "acc", "--timeout", "30"])
        self.assertEqual(code, 1)
        meta = json.loads((self.root / "n" / "bevy" / "meta.json").read_text(encoding="utf-8"))
        self.assertFalse(meta["ok"])
        self.assertIn("AA_SKILL_TRACE", meta["warning"])


if __name__ == "__main__":
    unittest.main()
