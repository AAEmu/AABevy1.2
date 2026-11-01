"""Scenario parser and validator tests."""
from __future__ import annotations

import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("parity_scenario",
                                              HERE.parent / "scenario.py")
scenario = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(scenario)

SCENARIOS = HERE.parent / "scenarios"


class ScenarioTests(unittest.TestCase):
    def test_first_scenarios_parse(self) -> None:
        wagon = scenario.load_scenario(SCENARIOS / "wagon_bar.yaml")
        attack = scenario.load_scenario(SCENARIOS / "auto_attack.yaml")
        self.assertEqual(wagon["name"], "wagon_bar")
        self.assertEqual(attack["name"], "auto_attack")
        self.assertEqual(len(wagon["steps"]), len({step["id"] for step in wagon["steps"]}))
        self.assertEqual(len(attack["steps"]), len({step["id"] for step in attack["steps"]}))
        self.assertTrue(any(step["verb"] == "mount" for step in wagon["steps"]))
        self.assertTrue(any(step["verb"] == "snapshot" for step in wagon["steps"]))
        self.assertTrue(any(step["verb"] == "use-skill" for step in attack["steps"]))
        sources = [step["wait"]["source"] for step in wagon["steps"]
                   if step["verb"] == "wait" and "source" in step["wait"]]
        self.assertEqual(sources, ["server", "server", "server"])

    def test_nested_yaml(self) -> None:
        data = scenario.load_yaml(
            "steps:\n"
            "  - id: one\n"
            "    target:\n"
            "      nearest: true\n"
            "      name: \"rat_brown \"\n"
            "      template: 6409\n"
            "    use-skill:\n"
            "      slot: 6\n"
            "      bar: mode\n")
        step = data["steps"][0]
        self.assertEqual(step["target"]["template"], 6409)
        self.assertEqual(step["target"]["name"], "rat_brown ")
        self.assertEqual(step["use-skill"]["slot"], 6)

    def test_missing_id_rejected(self) -> None:
        with self.assertRaises(scenario.ScenarioError):
            scenario.validate_scenario({
                "name": "x", "steps": [{"mark": "one"}]})

    def test_duplicate_id_rejected(self) -> None:
        with self.assertRaises(scenario.ScenarioError):
            scenario.validate_scenario({
                "name": "x",
                "steps": [{"id": "a", "mark": "a"},
                          {"id": "a", "mark": "b"}]})

    def test_unknown_verb_rejected(self) -> None:
        with self.assertRaises(scenario.ScenarioError):
            scenario.validate_scenario({
                "name": "x", "steps": [{"id": "a", "dance": True}]})

    def test_condition_needs_source(self) -> None:
        with self.assertRaises(scenario.ScenarioError):
            scenario.validate_scenario({
                "name": "x",
                "steps": [{"id": "a", "wait": {
                    "until": "target.present == true", "timeout_ms": 10}}]})

    def test_leaveworld_rejected(self) -> None:
        with self.assertRaises(scenario.ScenarioError):
            scenario.validate_scenario({
                "name": "x",
                "steps": [{"id": "a", "lua": {
                    "target": "ui", "code": "X2World:LeaveWorld()"}}]})

    def test_condition_evaluation(self) -> None:
        state = {"vehicle": {"mounted": True}, "target": {"name": "rat_brown "}}
        self.assertTrue(scenario.condition_holds(
            {"path": "vehicle.mounted", "op": "==", "literal": True}, state))
        self.assertTrue(scenario.condition_holds(
            {"path": "target.name", "op": "contains", "literal": "rat"}, state))
        self.assertFalse(scenario.condition_holds(
            {"path": "target.present", "op": "==", "literal": True}, state))


if __name__ == "__main__":
    unittest.main()
