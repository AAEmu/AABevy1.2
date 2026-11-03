import contextlib
import io
import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path

import _setup  # noqa: F401
import plan


def make_db(path: Path) -> None:
    """A tiny game database with only the tables and columns plan.py reads: invented rows, not game data."""
    c = sqlite3.connect(path)
    cols = ("id INT, name TEXT, ability_id INT, ability_level INT, target_type_id INT, target_selection_id INT, "
            "target_relation_id INT, casting_time INT, channeling_time INT, effect_delay INT, effect_speed REAL, projectile_id INT, "
            "plot_id INT, cooldown_time INT, default_gcd TEXT, custom_gcd INT, min_range INT, max_range INT, active_weapon_id INT, "
            "start_autoattack TEXT, mana_cost INT, \"SHOW\" TEXT")
    c.execute(f"create table skills ({cols})")
    c.execute("create table plot_events (id INT, plot_id INT, position INT)")
    c.execute("create table plot_next_events (event_id INT, next_event_id INT, delay INT, fail TEXT, add_anim_cs_time TEXT)")
    c.execute("create table localized_texts (tbl_name TEXT, tbl_column_name TEXT, idx INT, en_us TEXT)")

    def skill(sid, ability, level, ttype=4, cast=0, chan=0, delay=0, speed=0.0, proj=None, plot=None, show="t", name=None, cd=0):
        c.execute("insert into skills values (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                  (sid, name or f"skill{sid}", ability, level, ttype, 2, 0, cast, chan, delay, speed, proj, plot, cd, "t", 0, 0, 20, 0,
                   "f", 0, show))
        c.execute("insert into localized_texts values ('skills','name',?,?)", (sid, f"Skill {sid}"))

    for i in range(11):                                  # tree 1: 11 shown skills, level order = id order
        skill(100 + i, 1, i + 1)
    skill(150, 1, 5, show="f")                           # a hidden skill of tree 1: never in the plan
    skill(200, 2, 1, ttype=4, cast=1500, delay=288, speed=15.0, proj=137)          # a bolt: flight 288 + 10 m / 15 m/s
    skill(201, 2, 2, ttype=0, chan=60000)                # self channel, longer than the cap
    skill(202, 2, 3, ttype=6)                            # ground
    skill(203, 2, 4, ttype=5)                            # self (friendly types)
    skill(204, 2, 5, ttype=1, plot=7)                    # a plot skill
    skill(2, 0, 0, ttype=4, show="t", name="melee")      # the melee attack: ability 0, not in a tree
    skill(14495, 0, 0, show="t")
    skill(16064, 0, 0, show="t")
    skill(16287, 0, 0, ttype=0, show="t")
    for eid, pos in ((1, 1), (2, 2), (3, 3), (4, 4)):
        c.execute("insert into plot_events values (?,?,?)", (eid, 7, pos))
    c.execute("insert into plot_next_events values (1,2,300,'f','f')")
    c.execute("insert into plot_next_events values (2,3,100,'f','t')")       # +450 ms clip sync
    c.execute("insert into plot_next_events values (1,4,9000,'t','f')")      # a fail branch: ignored
    c.execute("insert into localized_texts values ('items','name',5660,'Test Greatsword')")
    c.execute("insert into localized_texts values ('npcs','name',13013,'Test Doll')")
    c.commit()
    c.close()


class PlanFromDb(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.db = Path(cls.tmp.name) / "test.sqlite3"
        make_db(cls.db)
        cls.conn = plan.connect(cls.db)

    @classmethod
    def tearDownClass(cls):
        cls.conn.close()
        cls.tmp.cleanup()

    def build(self, **kw):
        kw.setdefault("trees", [1, 2])
        return plan.build_plan(self.conn, **kw)

    def test_eleven_skills_are_two_batches_with_ten_slots_and_one_with_twelve(self):
        p = self.build(trees=[1], defaults="none")
        self.assertEqual([len(b["skills"]) for b in p["batches"]], [10, 1])
        self.assertEqual(p["batches"][0]["slots"]["10"], 109)
        self.assertEqual(p["batches"][0]["skills"][9]["key"], "0")
        self.assertEqual(p["batches"][1]["slots"], {"1": 110})            # the 11th skill is re-slotted on slot 1
        self.assertNotIn(150, [s["id"] for s in p["skills"]])               # SHOW <> 't'
        p12 = self.build(trees=[1], defaults="none", slots_per_page=12)
        self.assertEqual(len(p12["batches"]), 1)
        self.assertEqual([s["key"] for s in p12["batches"][0]["skills"]][-2:], ["0", "-"])        # the 11th skill is on `-`
        with self.assertRaises(ValueError):
            self.build(slots_per_page=13)

    def test_skills_keep_skill_book_order_and_carry_names(self):
        p = self.build(trees=[1], defaults="none")
        self.assertEqual([s["id"] for s in p["skills"]], list(range(100, 111)))
        self.assertEqual(p["skills"][0]["name"], "Skill 100")             # the English text, not the stored name
        self.assertEqual(p["skills"][0]["slot"], 1)
        self.assertEqual(p["skills"][10]["batch"], 2)

    def test_target_modes(self):
        p = self.build(trees=[2], defaults="none")
        by = {s["id"]: s for s in p["skills"]}
        self.assertEqual(by[200]["target"], "unit")
        self.assertEqual((by[200]["select"], by[200]["needs_target"], by[200]["needs_click"]), ("dummy", True, False))
        self.assertEqual((by[201]["target"], by[201]["select"], by[201]["needs_target"]), ("self", "self", False))
        self.assertEqual((by[202]["target"], by[202]["needs_click"]), ("ground", True))
        self.assertEqual(by[203]["target"], "self")
        self.assertEqual(plan.target_mode(4), "unit")
        self.assertEqual({plan.target_mode(t) for t in (6, 11, 15, 18)}, {"ground"})

    def test_wait_is_cast_plus_channel_plus_flight_plus_tail(self):
        by = {s["id"]: s for s in self.build(trees=[2], defaults="none")["skills"]}
        bolt = by[200]
        self.assertEqual((bolt["cast_ms"], bolt["flight_ms"]), (1500, 288 + 667))          # 10 m / 15 m/s for a projectile
        self.assertEqual(bolt["wait_ms"], 1500 + 955 + 2500)
        self.assertNotIn("stop_after_ms", bolt)
        channel = by[201]
        self.assertEqual(channel["channel_capped_ms"], 6000)
        self.assertEqual(channel["stop_after_ms"], 6000)
        self.assertEqual(channel["wait_ms"], 6000 + 2500)
        self.assertEqual(by[202]["wait_ms"], 3000)                                          # the minimum wait

    def test_plot_span_is_the_longest_delay_chain_without_the_fail_branch(self):
        self.assertEqual(plan.plot_span_ms(self.conn, 7), 300 + 100 + 450)
        self.assertEqual(plan.plot_span_ms(self.conn, None), 0)
        self.assertEqual(plan.plot_span_ms(self.conn, 999), 0)
        by = {s["id"]: s for s in self.build(trees=[2], defaults="none")["skills"]}
        self.assertEqual(by[204]["plot_span_ms"], 850)
        self.assertEqual(by[204]["wait_ms"], 3350)                                          # 850 + 2500 tail

    def test_defaults_batch_has_the_melee_attack_as_a_repeating_skill(self):
        p = self.build(trees=[1], defaults="basic", only_skills=None)
        last = p["batches"][-1]
        self.assertEqual((last["tree"], last["tree_name"]), (0, "Defaults"))
        self.assertEqual(last["slots"], {"1": 2, "2": 14495, "3": 16064, "4": 16287})
        melee = last["skills"][0]
        self.assertTrue(melee["autoattack"])
        self.assertEqual((melee["stop_after_ms"], melee["wait_ms"]), (5000, 7500))
        self.assertEqual(self.build(trees=[1], defaults="none")["batches"][-1]["tree"], 1)

    def test_weapon_per_tree_and_item_names(self):
        p = self.build(trees=[1, 2], defaults="none")
        weapon = p["batches"][0]["weapon"]
        self.assertEqual((weapon["name"], weapon["mainhand"], weapon["offhand"], weapon["wield"]), ("two-handed sword", 5660, 0, 2))
        self.assertEqual(weapon["names"]["mainhand"], "Test Greatsword")
        override = self.build(trees=[1], defaults="none", weapons={1: {"name": "club", "mainhand": 1, "wield": 1}})
        self.assertEqual(override["batches"][0]["weapon"]["name"], "club")

    def test_dummy_setup_and_schema(self):
        p = self.build(trees=[1], defaults="none")
        self.assertEqual(p["schema"], "skillparity/plan/v1")
        self.assertEqual((p["dummy"]["template"], p["dummy"]["name"]), (13013, "Test Doll"))
        self.assertIn("/ignoreskillcds true", p["setup"]["gm"])
        self.assertEqual(p["counts"], {"batches": 2, "skills": 11, "trees": [1]})
        self.assertNotIn(str(self.tmp.name), json.dumps(p))                                # no machine paths in the plan

    def test_only_skills_and_select_batches(self):
        p = self.build(trees=[1, 2], defaults="none", only_skills=[100, 201])
        self.assertEqual([s["id"] for s in p["skills"]], [100, 201])
        full = self.build(trees=[1, 2], defaults="none")
        picked = plan.select_batches(full, trees=[2], skills=[201, 202])
        self.assertEqual([[s["id"] for s in b["skills"]] for b in picked], [[201, 202]])
        self.assertEqual(picked[0]["slots"], {"2": 201, "3": 202})                          # slots keep their numbers

    def test_cli_writes_and_load_plan_reads(self):
        out = Path(self.tmp.name) / "out" / "plan.json"
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(plan.main(["--db", str(self.db), "--trees", "1,2", "--out", str(out)]), 0)
        loaded = plan.load_plan(out)
        self.assertEqual(loaded["counts"]["trees"], [0, 1, 2])
        bad = Path(self.tmp.name) / "bad.json"
        bad.write_text("{}", encoding="utf-8")
        with self.assertRaises(ValueError):
            plan.load_plan(bad)


class WithoutDatabase(unittest.TestCase):
    def test_no_db_is_a_clean_exit_not_a_traceback(self):
        saved = os.environ.pop("AA_DB", None)
        try:
            err = io.StringIO()
            with contextlib.redirect_stderr(err):
                self.assertEqual(plan.main(["--trees", "1"]), 2)
            self.assertIn("AA_DB", err.getvalue())
            with contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(plan.main(["--db", "does-not-exist.sqlite3"]), 2)
        finally:
            if saved is not None:
                os.environ["AA_DB"] = saved


@unittest.skipUnless(os.environ.get("AA_DB") and Path(os.environ.get("AA_DB", "")).is_file(), "AA_DB is not set: plan from the real database skipped")
class WithRealDatabase(unittest.TestCase):
    def test_the_110_shown_skills_of_the_ten_trees(self):
        conn = plan.connect(os.environ["AA_DB"])
        p = plan.build_plan(conn, list(plan.TREES), defaults="none")
        self.assertEqual(p["counts"]["skills"], 110)
        self.assertEqual(p["counts"]["batches"], 20)
        per_tree = {}
        for s in p["skills"]:
            per_tree[s["tree"]] = per_tree.get(s["tree"], 0) + 1
        self.assertEqual(set(per_tree.values()), {11})
        self.assertEqual({s["id"] for s in p["skills"] if s["target"] == "ground"}, {10664, 13281, 13286})
        self.assertEqual(next(s for s in p["skills"] if s["id"] == 18132)["name"], "Triple Slash")


if __name__ == "__main__":
    unittest.main()
