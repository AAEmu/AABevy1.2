import json
import tempfile
import unittest
from pathlib import Path

import _setup  # noqa: F401
import common
import make_fixtures


class NamesAndFiles(unittest.TestCase):
    def test_norm_name_follows_the_readme(self):
        self.assertEqual(common.norm_name("Game\\Animations\\Staff_Cast_01.CAF"), "animations/staff_cast_01.caf")
        self.assertEqual(common.norm_name("FX.Magic.Charge"), "fx.magic.charge")
        self.assertEqual(common.norm_name(None), "")

    def test_skill_label(self):
        self.assertEqual(common.parse_skill_label("skill:14376:start"), ("14376", "start"))
        self.assertEqual(common.parse_skill_label("skill:14376:end"), ("14376", "end"))
        self.assertIsNone(common.parse_skill_label("skillparity:record-start"))
        self.assertEqual(common.as_skill_id("14376"), 14376)
        self.assertEqual(common.as_skill_id("slot3"), "slot3")

    def test_trace_writer_and_reader_roundtrip(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "orig" / "trace.jsonl"
            w = common.TraceWriter(path, "orig")
            w.mark("skill:1:start", 1, wall_us=2_000)
            w.write({"kind": "press", "wall_us": 1_000, "key": "1", "slot": 1})
            w.close()
            with path.open("a", encoding="utf-8") as handle:
                handle.write("not json\n\n")
            rows, bad = common.read_jsonl(path)
            self.assertEqual(bad, 1)
            self.assertEqual([r["kind"] for r in rows], ["press", "mark"])      # sorted by wall_us
            self.assertEqual(rows[1]["label"], "skill:1:start")
            self.assertEqual(rows[0]["src"], "orig")
            self.assertEqual(common.read_jsonl(Path(d) / "missing.jsonl"), ([], 0))

    def test_frame_ms_from_dt_rows_and_from_frame_counter(self):
        rows = [{"kind": "frame", "wall_us": i, "dt_ms": 16.7} for i in range(5)]
        self.assertAlmostEqual(common.frame_ms(rows), 16.7)
        rows = [{"kind": "anim", "wall_us": 1_000_000 + i * 33_333, "frame": i} for i in range(6)]
        self.assertAlmostEqual(common.frame_ms(rows), 33.333, places=2)
        self.assertIsNone(common.frame_ms([{"kind": "anim", "wall_us": 1}]))


class Roles(unittest.TestCase):
    def test_locomotion_and_hit_clips(self):
        self.assertTrue(common.is_locomotion("sword_combat_idle_01"))
        self.assertTrue(common.is_locomotion("all_run_f"))
        self.assertFalse(common.is_locomotion("staff_cast_spell_01"))
        self.assertFalse(common.is_locomotion("sword_attack_jump_01"))     # a skill word wins over a locomotion word
        self.assertTrue(common.is_hit_reaction("scarecrow_hit_01"))
        self.assertFalse(common.is_hit_reaction("sword_attack_01"))

    def test_player_inst_is_the_first_skill_clip_after_the_first_press(self):
        rows = make_fixtures.build_orig()
        found = common.learn_player_inst(rows)
        self.assertEqual(found["inst"], make_fixtures.PLAYER_ORIG)
        self.assertEqual(found["clip"], "sword_attack_01")          # the idle clip 10 ms after the press is skipped
        self.assertAlmostEqual(found["dt_ms"], 33.0)

    def test_nothing_learned_without_a_press_or_after_a_second(self):
        rows = [r for r in make_fixtures.build_orig() if r["kind"] != "press"]
        self.assertIsNone(common.learn_player_inst(rows))
        late = [{"kind": "press", "wall_us": 1_000_000}, {"kind": "anim", "wall_us": 2_500_000, "inst": "x", "clip": "staff_cast_01"}]
        self.assertIsNone(common.learn_player_inst(late))

    def test_assign_roles_labels_player_target_and_other(self):
        rows = make_fixtures.build_orig()
        rows.append({"kind": "anim", "wall_us": make_fixtures.ORIG_BASE + 1_100_000, "inst": "0x9999", "clip": "bird_fly_01"})
        rows.sort(key=lambda r: r["wall_us"])
        summary = common.assign_roles(rows)
        self.assertEqual(summary["player_inst"], make_fixtures.PLAYER_ORIG)
        self.assertEqual(summary["target_inst"], make_fixtures.TARGET_ORIG)
        roles = {r["inst"]: r["role"] for r in rows if r["kind"] == "anim"}
        self.assertEqual(roles[make_fixtures.PLAYER_ORIG], "player")
        self.assertEqual(roles[make_fixtures.TARGET_ORIG], "target")
        self.assertEqual(roles["0x9999"], "other")

    def test_labelled_traces_are_left_alone(self):
        rows = make_fixtures.build_bevy()
        before = json.dumps(rows, sort_keys=True)
        summary = common.ensure_roles(rows)
        self.assertEqual(json.dumps(rows, sort_keys=True), before)
        self.assertEqual(summary["player_inst"], make_fixtures.PLAYER_BEVY)

    def test_unlearnable_trace_keeps_rows_unlabelled(self):
        rows = [{"kind": "anim", "wall_us": 5, "inst": "a", "clip": "staff_cast_01"}]
        common.assign_roles(rows)
        self.assertNotIn("role", rows[0])


if __name__ == "__main__":
    unittest.main()
