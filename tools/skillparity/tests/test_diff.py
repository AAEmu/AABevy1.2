import contextlib
import io
import json
import tempfile
import unittest
from pathlib import Path

import _setup
import common
import diff
import make_fixtures as fx

RUN1 = _setup.RUN1


def report(orig=None, bevy=None, server=None, **kw):
    plan = fx.build_plan()
    return diff.build_report(orig if orig is not None else fx.build_orig(), bevy if bevy is not None else fx.build_bevy(),
                             server if server is not None else fx.build_server(), plan, run_id="run1", **kw)


def skill(rep, sid):
    return next(s for s in rep["skills"] if s["skill_id"] == sid)


def diffs(rec, category=None, metric=None, subject=None, field=None):
    return [d for d in rec["differences"] if (category is None or d["category"] == category) and (metric is None or d["metric"] == metric)
            and (subject is None or d["subject"] == subject) and (field is None or d["field"] == field)]


class Fixtures(unittest.TestCase):
    def test_committed_fixtures_are_what_the_generator_makes(self):
        for name, content in fx.build_all().items():
            path = RUN1 / name
            self.assertTrue(path.is_file(), path)
            if name.endswith(".jsonl"):
                on_disk = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
                self.assertEqual(on_disk, json.loads(json.dumps(content)), name)
            else:
                self.assertEqual(json.loads(path.read_text(encoding="utf-8")), content, name)


class Verdicts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rep = report()

    def test_summary_counts(self):
        s = self.rep["summary"]
        self.assertEqual((s["skills"], s["PASS"], s["DIFF"], s["NO-DATA"]), (5, 1, 3, 1))

    def test_identical_within_a_frame_passes(self):
        rec = skill(self.rep, 100)
        self.assertEqual(rec["verdict"], "PASS", rec["differences"])
        self.assertEqual(rec["differences"], [])
        # the target's position is not in the aa-client trace at all: NO-DATA for it, not a pass or a difference
        self.assertIn("pos target", rec["no_data"])

    def test_timing_difference_names_the_clip_and_the_numbers(self):
        rec = skill(self.rep, 200)
        self.assertEqual(rec["verdict"], "DIFF")
        clip = diffs(rec, "timing", "anim", "staff_cast_spell_01")[0]
        # the clip is 2 ms before the server's stamp of SCSkillStarted, inside the slack for the stamp's lag: anchored on it
        self.assertEqual((clip["orig"], clip["bevy"], clip["delta_ms"], clip["raw_delta_ms"]), (33.0, 99.0, 62.0, 66.0))
        self.assertEqual(clip["text"], "clip `staff_cast_spell_01`: starts 62 ms later after SCSkillStarted in aa-client (orig 33 ms, "
                                       "bevy 99 ms after the press; the server answered 4 ms apart)")
        self.assertEqual(rec["worst_delta_ms"], 62.0)
        # the cast bar follows SCSkillStarted, which reached aa-client's server log 4 ms later: 65 ms raw, 61 ms of its own
        cast = diffs(rec, "timing", "castbar", "cast bar", "start_ms")[0]
        self.assertEqual((cast["delta_ms"], cast["raw_delta_ms"], cast["after"], cast["server_shift_ms"]), (61.0, 65.0, "SCSkillStarted", 4.0))
        self.assertEqual(cast["text"], "cast bar: starts 61 ms later after SCSkillStarted in aa-client (orig 40 ms, bevy 105 ms after "
                                       "the press; the server answered 4 ms apart)")
        self.assertEqual(diffs(rec, "timing", "castbar", "cast bar", "stop_ms")[0]["delta_ms"], 61.0)
        self.assertEqual(diffs(rec, "timing", "cooldown", "skill cooldown")[0]["delta_ms"], 56.0)
        self.assertEqual(rec["counts"]["timing"], 8)
        self.assertEqual(rec["counts"]["missing"] + rec["counts"]["extra"] + rec["counts"]["value"], 0)
        self.assertGreater(rec["median_shift_ms"], 55)          # everything is late by about the same: a press offset

    def test_missing_extra_and_value_differences(self):
        rec = skill(self.rep, 300)
        missing = [(d["metric"], d["subject"]) for d in diffs(rec, "missing")]
        self.assertEqual(sorted(missing), [("anim", "sword_dash_land_01"), ("fx", "fx.skill.dust")])
        self.assertEqual([d["subject"] for d in diffs(rec, "extra")], ["ui:error_beep"])
        speed = diffs(rec, "value", "anim", "sword_dash_01", "speed")[0]
        self.assertEqual((speed["orig"], speed["bevy"]), (1.0, 1.25))
        move = diffs(rec, "value", "pos", "player", "displacement_m")[0]
        self.assertEqual((move["orig"], move["bevy"]), (4.0, 3.0))
        self.assertAlmostEqual(move["delta_m"], 1.0, places=2)
        self.assertEqual(diffs(rec, "timing"), [])                 # the clip starts 5 ms apart: inside a frame
        texts = [d["text"] for d in rec["differences"]]
        self.assertIn("clip `sword_dash_land_01`: missing in aa-client; the 1.2 client has it at 400 ms", texts)
        self.assertIn("effect `fx.skill.dust`: missing in aa-client; the 1.2 client has it at 420 ms (1x)", texts)
        self.assertIn("sound `ui:error_beep`: only in aa-client at 900 ms; the 1.2 client does not have it", texts)

    def test_a_skill_one_client_never_cast_is_no_data(self):
        rec = skill(self.rep, 400)
        self.assertEqual(rec["verdict"], "NO-DATA")
        self.assertIn("aa-client", rec["reason"])
        self.assertEqual(rec["differences"], [])

    def test_common_differences_group_the_same_root_cause(self):
        top = self.rep["common_differences"][0]
        self.assertEqual((top["category"], top["metric"], top["subject"]), ("missing", "fx", "fx.skill.dust"))
        self.assertEqual(sorted(top["skills"]), [300, 500])
        self.assertEqual(top["skill_count"], 2)

    def test_server_rows_are_taken_by_time_window(self):
        rec = skill(self.rep, 200)
        o, b = rec["orig"]["net_server"], rec["bevy"]["net_server"]
        self.assertEqual(o["start_skill_ms"], 30.0)
        self.assertEqual(b["start_skill_ms"], 34.0)
        self.assertEqual(sorted(o["answers"]), ["SCSkillEnded", "SCSkillFired", "SCSkillStarted"])
        self.assertEqual(o["answers"]["SCSkillFired"]["first_ms"], 1535.0)

    def test_roles_were_learned_for_the_original(self):
        self.assertEqual(self.rep["clients"]["orig"]["player_inst"], fx.PLAYER_ORIG)
        self.assertEqual(self.rep["clients"]["bevy"]["player_inst"], fx.PLAYER_BEVY)

    def test_tolerance_is_a_frame_of_the_slower_client(self):
        tol = self.rep["tolerances"]
        self.assertAlmostEqual(tol["timing_ms"], 33.3)
        self.assertEqual(tol["pos_m"], 0.1)
        self.assertEqual(tol["speed_rel"], 0.02)
        self.assertEqual(tol["blend_s"], 0.05)

    def test_a_looser_tolerance_turns_the_timing_diff_into_a_pass(self):
        rep = report(tol_ms=100)
        self.assertEqual(skill(rep, 200)["verdict"], "PASS")
        self.assertEqual(skill(rep, 300)["verdict"], "DIFF")        # not a timing difference

    def test_report_is_json_and_markdown(self):
        json.dumps(self.rep)
        md = diff.render_markdown(self.rep)
        self.assertIn("| 200 Test Bolt | DIFF | 0 | 0 | 8 | 0 | 62 |", md)
        self.assertIn("* clip `staff_cast_spell_01`: starts 62 ms later after SCSkillStarted in aa-client (orig 33 ms, bevy 99 ms after the press; "
                      "the server answered 4 ms apart)", md)
        self.assertIn("| 100 Test Slash | PASS |", md)
        self.assertIn("## Most common differences", md)
        self.assertIn("fx `fx.skill.dust`", md)


class NoData(unittest.TestCase):
    def test_a_client_without_pos_rows_is_no_data_for_movement_never_a_pass(self):
        bevy = [r for r in fx.build_bevy() if r["kind"] != "pos"]
        rep = report(bevy=bevy)
        rec = skill(rep, 300)
        self.assertIn("pos", rec["no_data"])
        self.assertEqual(diffs(rec, metric="pos"), [])             # no movement difference can be claimed
        self.assertEqual(rec["verdict"], "DIFF")                    # the clip and effect differences stand
        self.assertEqual(skill(rep, 100)["verdict"], "PASS")        # nothing differs in what both recorded
        self.assertIn("pos", skill(rep, 100)["no_data"])

    def test_a_client_without_castbar_rows_is_no_data_for_the_cast_bar(self):
        bevy = [r for r in fx.build_bevy() if r["kind"] != "castbar"]
        rec = skill(report(bevy=bevy), 200)
        self.assertIn("castbar", rec["no_data"])
        self.assertEqual(diffs(rec, metric="castbar"), [])

    def test_anim_rows_without_a_learnable_player_make_the_skill_no_data(self):
        orig = [dict(r) for r in fx.build_orig()]
        for r in orig:
            if r["kind"] == "anim":
                r.pop("inst", None)                                 # no inst: the role rule cannot find the player
        rep = report(orig=orig)
        rec = skill(rep, 100)
        self.assertEqual(rec["verdict"], "NO-DATA", rec.get("reason"))       # no clip comparison: never a PASS
        self.assertIn("anim", rec["no_data"])
        rec = skill(rep, 200)                                                # the cast bar and effects still differ: DIFF stands
        self.assertEqual(rec["verdict"], "DIFF")
        self.assertIn("anim", rec["no_data"])
        self.assertEqual(diffs(rec, metric="anim"), [])

    def test_no_press_means_no_data(self):
        orig = [r for r in fx.build_orig() if not (r["kind"] == "press" and r["slot"] == 2)]
        rec = skill(report(orig=orig), 200)
        self.assertEqual(rec["verdict"], "NO-DATA")
        self.assertIn("no press", rec["reason"])

    def test_nothing_recorded_in_either_segment_is_no_data(self):
        keep = ("mark", "press", "frame")
        orig = [r for r in fx.build_orig() if r["kind"] in keep]
        bevy = [r for r in fx.build_bevy() if r["kind"] in keep]
        rec = skill(report(orig=orig, bevy=bevy, server=[]), 100)
        self.assertEqual(rec["verdict"], "NO-DATA")

    def test_without_server_rows_the_net_comparison_is_no_data_but_not_a_failure(self):
        rep = report(server=[])
        rec = skill(rep, 100)
        self.assertEqual(rec["verdict"], "PASS")
        self.assertIn("net", rec["no_data"])
        self.assertEqual(rep["server"], None)


class Segments(unittest.TestCase):
    def test_segments_by_marks_then_by_press(self):
        trace = diff.load_trace(fx.build_orig(), "orig")
        segs = diff.find_segments(trace)
        self.assertEqual([s.skill_id for s in segs], [100, 200, 300, 400, 500])
        self.assertTrue(all(s.source == "marks" and s.press_us for s in segs))
        self.assertEqual(segs[0].press_us - segs[0].start_us, 50_000)
        no_marks = diff.load_trace([r for r in fx.build_orig() if r["kind"] != "mark"], "orig")
        by_press = diff.find_segments(no_marks, diff.plan_index(fx.build_plan()))
        self.assertEqual([s.source for s in by_press], ["press"] * 5)
        # five presses and five plan skills on their own slots: plan order names them
        self.assertEqual([s.skill_id for s in by_press], [100, 200, 300, 400, 500])
        self.assertEqual(by_press[0].end_us, by_press[1].press_us - 1)

    def test_press_fallback_names_a_segment_by_its_cs_start_skill(self):
        server = diff.Trace(fx.build_server(), "server")
        no_marks = diff.load_trace([r for r in fx.build_orig() if r["kind"] != "mark"][:], "orig")
        segs = diff.find_segments(no_marks, None, server)
        self.assertEqual([s.skill_id for s in segs], [100, 200, 300, 400, 500])

    def test_a_skill_cast_twice_gets_occurrences(self):
        rows = fx.build_orig() + [
            {"src": "orig", "wall_us": fx.ORIG_BASE + 30_000_000, "kind": "mark", "label": "skill:100:start"},
            {"src": "orig", "wall_us": fx.ORIG_BASE + 30_100_000, "kind": "press", "key": "1", "slot": 1},
            {"src": "orig", "wall_us": fx.ORIG_BASE + 33_000_000, "kind": "mark", "label": "skill:100:end"}]
        segs = diff.find_segments(diff.load_trace(rows, "orig"))
        self.assertEqual([(s.skill_id, s.occurrence) for s in segs if s.skill_id == 100], [(100, 0), (100, 1)])


def quiet(argv):
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        return diff.main(argv)


class Cli(unittest.TestCase):
    def test_main_writes_report_json_and_md(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(quiet([str(RUN1), "--out", d]), 0)
            data = json.loads((Path(d) / "report.json").read_text(encoding="utf-8"))
            self.assertEqual(data["schema"], "skillparity/report/v1")
            self.assertEqual(data["summary"]["DIFF"], 3)
            self.assertTrue((Path(d) / "report.md").read_text(encoding="utf-8").startswith("# Skill parity report"))

    def test_main_refuses_a_capture_without_both_traces(self):
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(quiet([d]), 2)

    def test_skill_filter(self):
        with tempfile.TemporaryDirectory() as d:
            quiet([str(RUN1), "--out", d, "--skills", "200"])
            data = json.loads((Path(d) / "report.json").read_text(encoding="utf-8"))
            self.assertEqual([s["skill_id"] for s in data["skills"]], [200])


if __name__ == "__main__":
    unittest.main()


class Copies(unittest.TestCase):
    def test_layer_one_copies_and_repeated_requests_are_one_start(self):
        def clip(name, t, layer=0):
            return {"clip": name, "t_ms": t, "layer": layer}
        clips = [clip("twohand_ba_combat_idle", 0.3), clip("twohand_ba_combat_idle", 21.5),
                 clip("twohand_co_sk_streakattack_1", 145.4), clip("twohand_co_sk_streakattack_1", 151.9, layer=1),
                 clip("all_co_sk_dashattack", 181.1), clip("all_co_sk_dashattack_ub", 188.3, layer=1),
                 clip("twohand_co_sk_streakattack_1", 757.0)]
        clips.append(clip("bow_co_sk_launch_ub", 900.0, layer=1))
        kept, copies = diff.drop_copies(clips)
        self.assertEqual(copies, 3)
        self.assertEqual([(c["clip"], c["t_ms"]) for c in kept],
                         [("twohand_ba_combat_idle", 0.3), ("twohand_co_sk_streakattack_1", 145.4), ("all_co_sk_dashattack", 181.1),
                          ("twohand_co_sk_streakattack_1", 757.0), ("bow_co_sk_launch_ub", 900.0)])



class ServerAnswers(unittest.TestCase):
    def test_an_event_after_an_answer_is_timed_from_that_answer(self):
        # the server's plot events reached aa-client's log 23 ms later (its tick), both of them
        d = diff.Differ(1, {"timing_ms": 12.0}, [["SCPlotEvent", 120.0], ["SCPlotEvent", 400.0]],
                        [["SCPlotEvent", 143.0], ["SCPlotEvent", 423.0]])
        d.time("anim", "hit", "start_ms", 125.0, 148.0)          # 23 ms later, all of it the server's: no difference
        d.time("anim", "idle", "start_ms", 0.1, 14.6)            # before any answer: timed from the press
        d.time("fx", "dust", "first_ms", 410.0, 450.0)           # 17 ms of its own after the second answer
        self.assertEqual([(x["subject"], x["delta_ms"]) for x in d.diffs], [("idle", 14.5), ("dust", 17.0)])
        self.assertNotIn("after", d.diffs[0])
        self.assertEqual((d.diffs[1]["after"], d.diffs[1]["raw_delta_ms"]), ("SCPlotEvent #2", 40.0))

    def test_an_answer_only_one_client_got_is_no_anchor(self):
        d = diff.Differ(1, {"timing_ms": 12.0}, [["SCSkillStarted", 100.0], ["SCUnitDamaged", 300.0]], [["SCSkillStarted", 110.0]])
        d.time("sound", "hit", "first_ms", 310.0, 350.0)         # after SCSkillStarted (10 ms apart): 30 ms of its own
        self.assertEqual((d.diffs[0]["delta_ms"], d.diffs[0]["after"]), (30.0, "SCSkillStarted"))


class RandomVariants(unittest.TestCase):
    def test_a_random_idle_variant_matches_another_of_its_family(self):
        self.assertEqual(diff.clip_family("twohand_ba_combat_idle_rand_3"), "twohand_ba_combat_idle_rand_N")
        self.assertEqual(diff.clip_family("fist_ba_relaxed_rand_idle"), "fist_ba_relaxed_rand_idle")
        clip = lambda name, t: {"clip": name, "t_ms": t, "layer": 0, "speed": 1.0, "blend": 0.2, "loop": False, "file": name + ".caf"}
        d = diff.Differ(1, {"timing_ms": 12.0, "speed_rel": 0.02, "blend_s": 0.05})
        diff.compare_anim(d, {"clips": [clip("twohand_ba_combat_idle_rand_3", 5907.0)]}, {"clips": [clip("twohand_ba_combat_idle_rand_5", 5910.0)]})
        self.assertEqual(d.diffs, [], "another variant of the family at the same time is no difference")
