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
from unittest import mock

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
        self.assertIn("HUD.drag_action_set(1, 'skill', 14376)", lua[0])
        self.assertIn("HUD.drag_action_set(2, 'skill', 10712)", lua[0])

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
        i = lines.index("trace " + rb.VIDEO_START)
        self.assertEqual(lines[i + 1], "wait 3")
        self.assertNotIn(rb.VIDEO_START, script_for()[0])
        text, _ = script_for(mark_command="echo", video_grace=1.0)
        self.assertIn("echo video-start", text)                           # the client opens its video gate on this label too

    def test_script_has_no_machine_paths(self):
        text, _ = script_for(move="1 2 3")
        self.assertNotIn(":\\", text)
        self.assertNotIn("/Users/", text)


class Helpers(unittest.TestCase):
    def test_video_client_args_make_the_client_film_itself_from_the_script_mark(self):
        args = rb.video_client_args(Path("bevy/video.mp4"), 30, "ffmpeg")
        self.assertEqual(args[args.index("--video") + 1], str(Path("bevy/video.mp4")))
        self.assertEqual(args[args.index("--video-fps") + 1], "30")
        self.assertEqual(args[args.index("--ffmpeg") + 1], "ffmpeg")
        self.assertIn("--video-gate", args)
        self.assertEqual(rb.VIDEO_START, "video-start")

    def test_read_video_info_and_what_makes_a_video_unusable(self):
        good = {"t0_wall_us": 1762171234567890, "fps": 30, "width": 1920, "height": 1080, "frames": 300, "repeated": 20, "skipped": 0,
                "dropped": 0, "ffmpeg_exit": 0, "method": "aa-dev --video"}
        self.assertIsNone(rb.video_problem(good))
        self.assertIn("no frames", rb.video_problem({**good, "frames": 0}))
        self.assertIn("ffmpeg exited with 1", rb.video_problem({**good, "ffmpeg_exit": 1}))
        self.assertIn("exited with None", rb.video_problem({**good, "ffmpeg_exit": None}))
        self.assertIn("resized", rb.video_problem({**good, "stopped": "the window was resized"}))
        self.assertIn("boom", rb.video_problem({**good, "error": "boom"}))
        self.assertIn("start time", rb.video_problem({**good, "t0_wall_us": None}))
        with tempfile.TemporaryDirectory() as d:
            out = Path(d)
            self.assertIn("no usable video sidecar", rb.read_video_info(out)["error"])
            (out / "video.mp4.json").write_text("{not json", encoding="utf-8")
            self.assertIn("no usable video sidecar", rb.read_video_info(out)["error"])
            (out / "video.mp4.json").write_text(json.dumps(good), encoding="utf-8")
            self.assertEqual(rb.read_video_info(out), good)

    def test_client_command(self):
        args = Namespace(exe=Path("aa-client"), account="acc", login="h:1", password_file=Path("pw"), character="Ch", headless=True,
                         client_arg=["--x"])
        cmd = rb.client_command(args, Path("plan.txt"))
        self.assertEqual(cmd[:5], ["aa-client", "--account", "acc", "--login", "h:1"])
        self.assertIn("--require-wall-clock", cmd)
        self.assertEqual(cmd[cmd.index("--script") + 1], "plan.txt")
        self.assertEqual(cmd[-2:], ["--headless", "--x"])
        self.assertNotIn("--video", cmd)
        cmd = rb.client_command(args, Path("plan.txt"), ["--video", "v.mp4", "--video-gate"])
        self.assertEqual(cmd[-4:], ["--video", "v.mp4", "--video-gate", "--x"])          # before the user's own extras
        self.assertEqual(rb.parse_pair("960, 540"), (960, 540))
        self.assertIsNone(rb.parse_pair(""))


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


def make_video_launcher(directory: Path, write_sidecar: bool, fields: dict) -> Path:
    """The fake client plus what `aa-client --video OUT` leaves behind: OUT.json and video.json next to it."""
    wrapper = directory / "fake_video_client.py"
    wrapper.write_text(
        "import json, sys\n"
        f"sys.path.insert(0, {str(HERE)!r})\n"
        "import fake_client\n"
        "argv = sys.argv[1:]\n"
        "code = fake_client.main(argv)\n"
        f"if {write_sidecar!r} and '--video' in argv:\n"
        "    out = argv[argv.index('--video') + 1]\n"
        "    fps = int(argv[argv.index('--video-fps') + 1])\n"
        "    info = {'t0_wall_us': 1762171234567890, 'fps': fps, 'width': 1920, 'height': 1080, 'frames': 300, 'repeated': 20,\n"
        "            'skipped': 0, 'dropped': 0, 'ffmpeg_exit': 0, 'method': 'aa-dev --video'}\n"
        f"    info.update({fields!r})\n"
        "    for name in (out + '.json', out.rsplit('video.mp4', 1)[0] + 'video.json'):\n"
        "        open(name, 'w').write(json.dumps(info))\n"
        "sys.exit(code)\n", encoding="utf-8")
    if os.name == "nt":
        path = directory / "fake-video-client.cmd"
        path.write_text(f'@"{sys.executable}" "{wrapper}" %*\r\n', encoding="utf-8")
    else:
        path = directory / "fake-video-client.sh"
        path.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{wrapper}" "$@"\n', encoding="utf-8")
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

    def run_video(self, run_id, *extra, sidecar=True, **sidecar_fields):
        """run_bevy --video against a fake client that writes the sidecar the way `aa-client --video` does."""
        launcher = make_video_launcher(self.root, sidecar, sidecar_fields)
        argv = ["--plan", str(self.plan), "--capture-root", str(self.root), "--run-id", run_id, "--exe", str(launcher), "--account", "acc",
                "--timeout", "60", "--video", "--ffmpeg", "my-ffmpeg", "--fps", "25", *extra]
        code, out, err = quiet(rb.main, argv)
        bevy = self.root / run_id / "bevy"
        return code, out + err, bevy, json.loads((bevy / "meta.json").read_text(encoding="utf-8"))

    def test_video_run_passes_the_client_its_flags_and_keeps_its_sidecar(self):
        code, text, bevy, meta = self.run_video("v1")
        self.assertEqual(code, 0, text)
        cmd = meta["command"]
        self.assertEqual(cmd[cmd.index("--video") + 1], str(bevy / "video.mp4"))
        self.assertEqual((cmd[cmd.index("--video-fps") + 1], cmd[cmd.index("--ffmpeg") + 1]), ("25", "my-ffmpeg"))
        self.assertIn("--video-gate", cmd)
        self.assertIn("--require-wall-clock", cmd)
        self.assertEqual((meta["video"]["t0_wall_us"], meta["video"]["fps"], meta["video"]["frames"]), (1762171234567890, 25, 300))
        self.assertTrue(meta["ok"])
        self.assertNotIn("video_problem", meta)
        self.assertIn("video-start", (bevy / "plan.txt").read_text(encoding="utf-8"))

    def test_video_run_fails_when_the_video_is_missing_or_broken(self):
        code, text, bevy, meta = self.run_video("v2", sidecar=False)
        self.assertEqual(code, 1)
        self.assertFalse(meta["ok"])
        self.assertIn("no usable video sidecar", meta["video_problem"])
        self.assertIn("warning: video", text)
        code, _, bevy, meta = self.run_video("v3", ffmpeg_exit=1, frames=12)
        self.assertEqual(code, 1)
        self.assertIn("ffmpeg exited with 1", meta["video_problem"])
        code, _, bevy, meta = self.run_video("v4", stopped="the window changed from 1920x1080 to 800x600: capture stopped")
        self.assertEqual(code, 1)
        self.assertIn("stopped early", meta["video_problem"])

    def test_video_run_does_not_read_the_last_runs_sidecar(self):
        self.assertEqual(self.run_video("v5")[0], 0)
        # same directory again with --force and a client that writes nothing: the old sidecar must not count
        code, _, bevy, meta = self.run_video("v5", "--force", sidecar=False)
        self.assertEqual(code, 1)
        self.assertIn("no usable video sidecar", meta["video_problem"])

    def test_video_needs_ffmpeg(self):
        saved = os.environ.pop("FFMPEG", None)
        try:
            with mock.patch.object(rb.shutil, "which", return_value=None):
                code, _, err = quiet(rb.main, ["--plan", str(self.plan), "--capture-root", str(self.root), "--run-id", "nf", "--account", "a",
                                               "--exe", str(make_launcher(self.root)), "--video"])
        finally:
            if saved is not None:
                os.environ["FFMPEG"] = saved
        self.assertEqual(code, 2)
        self.assertIn("needs ffmpeg", err)

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
