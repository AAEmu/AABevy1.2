import contextlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

import _setup  # noqa: F401
import common
import diff
import make_fixtures as fx
import sidebyside as sb

HERE = Path(__file__).resolve().parent
RUN1 = HERE / "fixtures" / "run1"


def quiet(fn, *a):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = fn(*a)
    return code, out.getvalue(), err.getvalue()


def traces():
    plan = json.loads((RUN1 / "plan.json").read_text(encoding="utf-8"))
    orig = diff.load_trace(common.read_jsonl(RUN1 / "orig" / "trace.jsonl")[0], "orig")
    bevy = diff.load_trace(common.read_jsonl(RUN1 / "bevy" / "trace.jsonl")[0], "bevy")
    return orig, bevy, diff.plan_index(plan)


# videos that start 0.2 s before each run's base time
ORIG_VIDEO = {"t0_wall_us": fx.ORIG_BASE - 200_000, "fps": 30, "path": "o.mp4"}
BEVY_VIDEO = {"t0_wall_us": fx.BEVY_BASE - 200_000, "fps": 30, "path": "b.mp4"}


class Cuts(unittest.TestCase):
    def test_cut_for_starts_before_the_press_and_pads_when_the_video_is_short_on_the_left(self):
        video = {"t0_wall_us": 1_000_000}
        self.assertEqual(sb.cut_for(video, 3_050_000, 0.5, 3.5), {"ss": 1.55, "pad": 0.0, "press_in_video": 2.05, "length": 3.5})
        padded = sb.cut_for(video, 1_200_000, 0.5, 3.5)             # the press is 0.2 s into the video: 0.3 s of black first
        self.assertEqual((padded["ss"], padded["pad"], padded["length"]), (0.0, 0.3, 3.2))
        self.assertIsNone(sb.cut_for(video, 900_000, 0.5, 3.5))     # the press is before the video started

    def test_plan_cuts_align_each_side_on_its_own_press(self):
        orig, bevy, plan = traces()
        cuts = sb.plan_cuts(orig, bevy, ORIG_VIDEO, BEVY_VIDEO, plan, 0.5, 15.0)
        by = {c["skill_id"]: c for c in cuts}
        self.assertEqual([c["skill_id"] for c in cuts], [100, 200, 300, 400, 500])            # the plan's order
        first = by[100]
        self.assertAlmostEqual(first["left"]["press_in_video"], 1.25, places=3)              # press 1.05 s after the base, video 0.2 s earlier
        self.assertAlmostEqual(first["right"]["press_in_video"], 1.25, places=3)
        self.assertAlmostEqual(first["left"]["ss"], 0.75, places=3)
        self.assertEqual(first["pre"], 0.5)
        self.assertAlmostEqual(first["length"], 0.5 + 2.95, places=3)                        # to the end of the longer segment
        self.assertEqual(first["press_us"], {"orig": fx.ORIG_BASE + 1_050_000, "bevy": fx.BEVY_BASE + 1_050_000})
        self.assertIn("aa-client", by[400]["skipped"])                                         # only the original recorded it
        self.assertEqual(by[400]["name"], "Test Only")
        self.assertNotIn("skipped", by[500])

    def test_max_seconds_and_only_filter(self):
        orig, bevy, plan = traces()
        cuts = sb.plan_cuts(orig, bevy, ORIG_VIDEO, BEVY_VIDEO, plan, 0.5, 1.0, only=["200", 500])
        self.assertEqual([c["skill_id"] for c in cuts], [200, 500])
        self.assertAlmostEqual(cuts[0]["length"], 1.5, places=3)                              # 0.5 s before + 1 s cap

    def test_a_press_before_the_video_starts_is_skipped(self):
        orig, bevy, plan = traces()
        late = dict(ORIG_VIDEO, t0_wall_us=fx.ORIG_BASE + 1_100_000)                          # video starts after skill 100's press
        cuts = {c["skill_id"]: c for c in sb.plan_cuts(orig, bevy, late, BEVY_VIDEO, plan, 0.5, 15.0)}
        self.assertIn("original", cuts[100]["skipped"])
        self.assertNotIn("skipped", cuts[200])

    def test_a_segment_without_a_press_is_skipped(self):
        orig, bevy, plan = traces()
        bevy.rows = [r for r in bevy.rows if r.get("kind") != "press"]
        bevy.by_kind.pop("press", None)
        cuts = {c["skill_id"]: c for c in sb.plan_cuts(orig, bevy, ORIG_VIDEO, BEVY_VIDEO, plan, 0.5, 15.0)}
        self.assertIn("press", cuts[100].get("skipped", ""))


class Filter(unittest.TestCase):
    cut = {"pre": 0.5, "length": 3.5, "left": {"ss": 0.75, "pad": 0.0, "length": 3.5}, "right": {"ss": 0.0, "pad": 0.3, "length": 3.2}}

    def test_without_text_is_scale_and_hstack_only(self):
        graph = sb.build_filter(self.cut, "100 Test", None, 540, 30, text=False)
        self.assertIn("[0:v]fps=30,scale=-2:540,setsar=1,setpts=PTS-STARTPTS[l]", graph)
        self.assertIn("tpad=start_duration=0.3", graph)                       # the padded side only
        self.assertEqual(graph.count("tpad"), 1)
        self.assertTrue(graph.endswith("[l][r]hstack=inputs=2[o]"))
        self.assertNotIn("drawtext", graph)

    def test_with_text_counter_starts_at_the_press(self):
        graph = sb.build_filter(self.cut, "100 Test: Slash!", "C:\\Windows\\Fonts\\arial.ttf", 540, 30)
        self.assertEqual(graph.count("drawtext"), 5)                          # name + counter per side, one title
        self.assertIn("fontfile='C\\:/Windows/Fonts/arial.ttf'", graph)
        self.assertIn("(t-0.5)*1000", graph)
        self.assertIn("text='100 Test  Slash'", graph)                        # characters that need quoting are dropped
        self.assertIn("1.2 client", graph)
        self.assertIn("aa-client", graph)

    def test_command_maps_the_graph_and_cuts_both_inputs(self):
        cmd = sb.build_command("ffmpeg", ORIG_VIDEO, BEVY_VIDEO, dict(self.cut), Path("out.mp4"), "t", None, 540, 30, 20, text=False)
        self.assertEqual([cmd[i + 1] for i, c in enumerate(cmd) if c == "-i"], ["o.mp4", "b.mp4"])
        self.assertEqual([cmd[i + 1] for i, c in enumerate(cmd) if c == "-ss"], ["0.75", "0.0"])
        self.assertEqual(cmd[cmd.index("-map") + 1], "[o]")
        self.assertEqual(cmd[-1], "out.mp4")
        self.assertIn("-an", cmd)

    def test_small_helpers(self):
        self.assertEqual(sb.slug("Test Slash!"), "test_slash")
        self.assertEqual(sb.slug("!!!"), "skill")
        self.assertEqual(sb.safe_text("a:b'c;d"), "a b c d")
        text = sb.concat_list_text([Path("a b.mp4"), Path("it's.mp4")])
        self.assertEqual(text.count("\n"), 2)
        self.assertIn("'\\''", text)

    def test_find_ffmpeg_and_font_prefer_arguments_then_environment(self):
        with tempfile.TemporaryDirectory() as d:
            fake = Path(d) / "ffmpeg-fake"
            fake.write_text("x", encoding="utf-8")
            font = Path(d) / "f.ttf"
            font.write_text("x", encoding="utf-8")
            with mock.patch.dict(os.environ, {"FFMPEG": str(fake), "SKILLPARITY_FONT": str(font)}):
                self.assertEqual(sb.find_ffmpeg(), str(fake))
                self.assertEqual(sb.find_font(), str(font))
                self.assertEqual(sb.find_ffmpeg("does-not-exist"), str(fake))
            with mock.patch.dict(os.environ, {"FFMPEG": "", "SKILLPARITY_FONT": ""}), mock.patch("shutil.which", return_value=None):
                self.assertIsNone(sb.find_ffmpeg("nope"))


def make_run(root: Path, with_videos: bool = True) -> Path:
    run = root / "run"
    shutil.copytree(RUN1, run)
    if with_videos:
        for side, video in (("orig", ORIG_VIDEO), ("bevy", BEVY_VIDEO)):
            (run / side / "video.mp4").write_bytes(b"not a video")
            info = {k: v for k, v in video.items() if k != "path"}
            info.update(width=640, height=360)
            (run / side / "video.json").write_text(json.dumps(info), encoding="utf-8")
    return run


class Cli(unittest.TestCase):
    def test_dry_run_prints_commands_and_writes_a_manifest(self):
        with tempfile.TemporaryDirectory() as d:
            run = make_run(Path(d))
            code, out, err = quiet(sb.main, [str(run), "--dry-run", "--no-text", "--ffmpeg", str(run / "orig" / "video.mp4")])
            self.assertEqual(code, 0, err)
            self.assertEqual(out.count("-filter_complex"), 4)                 # 100, 200, 300, 500
            self.assertIn("skipped, no segment in aa-client", out)
            manifest = json.loads((run / "sidebyside" / "sidebyside.json").read_text(encoding="utf-8"))
            self.assertEqual(manifest["height"], 360)                          # the smaller video's height
            self.assertEqual(manifest["fps"], 30)
            self.assertFalse(manifest["font"])
            self.assertEqual([c["skill_id"] for c in manifest["cuts"]], [100, 200, 300, 400, 500])
            self.assertEqual(manifest["cuts"][0]["file"], "100_test_slash.mp4")
            self.assertEqual(len(manifest["warnings"]), 1)
            self.assertNotIn("path", manifest["videos"]["orig"])

    def test_missing_video_info_or_ffmpeg_is_a_clean_error(self):
        with tempfile.TemporaryDirectory() as d:
            run = make_run(Path(d), with_videos=False)
            code, _, err = quiet(sb.main, [str(run), "--dry-run"])
            self.assertEqual(code, 2)
            self.assertIn("video.mp4", err)
            run2 = make_run(Path(d) / "second")
            (run2 / "bevy" / "video.json").unlink()
            code, _, err = quiet(sb.main, [str(run2), "--dry-run"])
            self.assertEqual(code, 2)
            self.assertIn("video.json", err)
            run3 = make_run(Path(d) / "third")
            with mock.patch.object(sb, "find_ffmpeg", return_value=None):
                code, _, err = quiet(sb.main, [str(run3)])
            self.assertEqual(code, 2)
            self.assertIn("ffmpeg", err)


def usable_ffmpeg():
    exe = sb.find_ffmpeg()
    if not exe:
        return None
    try:
        encoders = subprocess.run([exe, "-hide_banner", "-encoders"], capture_output=True, text=True, timeout=30).stdout
    except (OSError, subprocess.SubprocessError):
        return None
    return exe if "libx264" in encoders else None


@unittest.skipUnless(usable_ffmpeg(), "no ffmpeg with libx264 (--ffmpeg is not a test option: set FFMPEG or put ffmpeg on PATH)")
class WithRealFfmpeg(unittest.TestCase):
    def test_one_skill_end_to_end(self):
        ffmpeg = usable_ffmpeg()
        with tempfile.TemporaryDirectory() as d:
            run = make_run(Path(d))
            for side, color in (("orig", "red"), ("bevy", "blue")):
                target = run / side / "video.mp4"
                proc = subprocess.run([ffmpeg, "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
                                       f"color=c={color}:s=320x180:r=10:d=5", "-c:v", "mpeg4", str(target)], capture_output=True, text=True)
                self.assertEqual(proc.returncode, 0, proc.stderr)
                info = json.loads((run / side / "video.json").read_text(encoding="utf-8"))
                info.update(width=320, height=180, fps=10)
                (run / side / "video.json").write_text(json.dumps(info), encoding="utf-8")
            code, out, err = quiet(sb.main, [str(run), "--ffmpeg", ffmpeg, "--skills", "100", "--no-text"])
            self.assertEqual(code, 0, out + err)
            made = run / "sidebyside" / "100_test_slash.mp4"
            self.assertTrue(made.is_file() and made.stat().st_size > 1000)
            self.assertTrue((run / "sidebyside" / "all.mp4").is_file())
            probe = subprocess.run([ffmpeg, "-hide_banner", "-i", str(made)], capture_output=True, text=True)
            self.assertIn("640x180", probe.stderr)                             # two 320-wide halves side by side
            # with captions: either they work or the cut is redone without them, never a failure
            code, out, err = quiet(sb.main, [str(run), "--ffmpeg", ffmpeg, "--skills", "100", "--out", str(run / "text")])
            self.assertEqual(code, 0, out + err)
            self.assertTrue((run / "text" / "100_test_slash.mp4").is_file())


if __name__ == "__main__":
    unittest.main()
