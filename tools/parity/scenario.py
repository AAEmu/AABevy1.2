#!/usr/bin/env python3
"""Scenario format for the parity harness (YAML subset or JSON).

A scenario is a list of steps that both the original 1.2 client runner and the
Bevy runner can execute.  This module only parses and validates; it does not
touch a client.  See tools/parity/README.md for the full format.
"""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path
from typing import Any

VERBS = (
    "gm", "chat", "wait", "key", "click-ui", "click_ui", "target",
    "use-skill", "use_skill", "interact", "move-to", "move_to",
    "mount", "dismount", "snapshot", "mark", "lua", "set-slots", "set_slots",
)
ALIASES = {
    "chat": "gm", "click_ui": "click-ui", "use_skill": "use-skill",
    "move_to": "move-to", "set_slots": "set-slots",
}
METADATA = {"id", "note", "optional", "server_tag", "description"}
STEP_OPTIONS = {"hold_ms", "duration_ms", "modifiers"}
SOURCES = {"client", "server", "script"}
KEY_NAMES = set("wasdqrtyuiopfg1234567890") | {"space", "release"}
MODIFIERS = {"ctrl", "shift", "alt"}
WIDGET_PATH_RE = re.compile(
    r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*|\[\d+\])*$")
# file-safe: letters, digits, `_`, `.` and `-`, but not only dots (`.` / `..` would escape the run dir)
STEP_ID_RE = re.compile(r"^(?!\.+$)[A-Za-z0-9_.-]{1,96}$")
CONDITION_RE = re.compile(
    r"^\s*([A-Za-z_][A-Za-z0-9_.]*)\s*(==|!=|<=|>=|<|>|contains)\s*(.+?)\s*$")
FORBIDDEN = ("leaveworld", "system.quit", "application.quit", "quit game")


class ScenarioError(ValueError):
    """A malformed scenario file."""


def _strip_comment(line: str) -> str:
    quote = ""
    escaped = False
    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\" and quote == '"':
            escaped = True
            continue
        if char in ("'", '"'):
            if quote == char:
                quote = ""
            elif not quote:
                quote = char
        elif char == "#" and not quote:
            return line[:index]
    return line


def _scalar(text: str) -> Any:
    text = text.strip()
    if text == "":
        return None
    if text[0:1] == '"' and text[-1:] == '"':
        return json.loads(text)
    if text[0:1] == "'" and text[-1:] == "'":
        return text[1:-1].replace("''", "'")
    lower = text.lower()
    if lower in ("true", "yes", "on"):
        return True
    if lower in ("false", "no", "off"):
        return False
    if lower in ("null", "none", "~"):
        return None
    if text[0:1] == "[" and text[-1:] == "]":
        inner = text[1:-1].strip()
        if not inner:
            return []
        return [_scalar(part) for part in inner.split(",")]
    if text[0:1] == "{" and text[-1:] == "}":
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            pass
    try:
        return int(text, 0)
    except ValueError:
        pass
    try:
        return float(text)
    except ValueError:
        return text


def _split_key(text: str) -> tuple[str, str] | None:
    quote = ""
    escaped = False
    for index, char in enumerate(text):
        if escaped:
            escaped = False
            continue
        if char == "\\" and quote == '"':
            escaped = True
            continue
        if char in ("'", '"'):
            if quote == char:
                quote = ""
            elif not quote:
                quote = char
        elif char == ":" and not quote:
            return text[:index].strip(), text[index + 1:].strip()
    return None


def _tokenize(source: str) -> list[tuple[int, str, int]]:
    lines: list[tuple[int, str, int]] = []
    for number, raw in enumerate(source.splitlines(), 1):
        expanded = raw.replace("\t", "    ")
        stripped = _strip_comment(expanded).rstrip()
        if not stripped.strip():
            continue
        indent = len(stripped) - len(stripped.lstrip(" "))
        lines.append((indent, stripped.lstrip(" "), number))
    return lines


class _Yaml:
    def __init__(self, source: str) -> None:
        self.lines = _tokenize(source)
        self.index = 0

    def parse(self) -> Any:
        if not self.lines:
            return None
        value, _ = self._block(self.lines[0][0])
        if self.index != len(self.lines):
            indent, text, number = self.lines[self.index]
            raise ScenarioError(f"line {number}: unexpected content {text!r}")
        return value

    def _block(self, indent: int) -> tuple[Any, int]:
        if self.index >= len(self.lines):
            return None, indent
        first_indent, first_text, _ = self.lines[self.index]
        if first_indent != indent:
            raise ScenarioError(f"indentation jump at {first_text!r}")
        if first_text.startswith("- ") or first_text == "-":
            return self._list(indent)
        return self._map(indent), indent

    def _list(self, indent: int) -> tuple[list, int]:
        out: list = []
        while self.index < len(self.lines):
            line_indent, text, number = self.lines[self.index]
            if line_indent < indent:
                break
            if line_indent > indent:
                raise ScenarioError(f"line {number}: unexpected indent in list")
            if not (text.startswith("- ") or text == "-"):
                break
            content = text[1:].strip()
            self.index += 1
            if content == "":
                if self.index < len(self.lines) and self.lines[self.index][0] > indent:
                    value, _ = self._block(self.lines[self.index][0])
                else:
                    value = None
                out.append(value)
                continue
            pair = _split_key(content)
            if pair is None:
                out.append(_scalar(content))
                continue
            key, rest = pair
            item: dict[str, Any] = {}
            item_indent = indent + 2
            if rest == "":
                if self.index < len(self.lines) and self.lines[self.index][0] > indent:
                    value, _ = self._block(self.lines[self.index][0])
                else:
                    value = None
                item[key] = value
            else:
                item[key] = _scalar(rest)
            while self.index < len(self.lines):
                next_indent, next_text, next_number = self.lines[self.index]
                if next_indent <= indent:
                    break
                if next_indent < item_indent:
                    raise ScenarioError(f"line {next_number}: bad indent in list item")
                if next_text.startswith("- ") or next_text == "-":
                    break
                extra = self._map(next_indent, stop_indent=indent)
                item.update(extra)
                break
            out.append(item)
        return out, indent

    def _map(self, indent: int, stop_indent: int | None = None) -> dict:
        out: dict[str, Any] = {}
        limit = indent - 1 if stop_indent is None else stop_indent
        while self.index < len(self.lines):
            line_indent, text, number = self.lines[self.index]
            if line_indent < indent or line_indent <= limit:
                break
            if line_indent > indent:
                raise ScenarioError(f"line {number}: unexpected indent in map")
            if text.startswith("- ") or text == "-":
                break
            pair = _split_key(text)
            if pair is None:
                raise ScenarioError(f"line {number}: expected key: value, got {text!r}")
            key, rest = pair
            if key in out:
                raise ScenarioError(f"line {number}: duplicate key {key!r}")
            self.index += 1
            if rest == "":
                if self.index < len(self.lines) and self.lines[self.index][0] > indent:
                    value, _ = self._block(self.lines[self.index][0])
                else:
                    value = None
            else:
                value = _scalar(rest)
            out[key] = value
        return out


def load_yaml(source: str) -> Any:
    """Parse the YAML subset used by scenario files."""
    return _Yaml(source).parse()


def load_scenario(path: Path) -> dict:
    path = Path(path)
    text = path.read_text(encoding="utf-8")
    if path.suffix.lower() == ".json":
        data = json.loads(text)
    else:
        data = load_yaml(text)
    scenario = validate_scenario(data, source=str(path))
    scenario["path"] = str(path)
    return scenario


def _condition(value: Any, where: str) -> dict:
    if not isinstance(value, str):
        raise ScenarioError(f"{where}: condition must be a string")
    match = CONDITION_RE.match(value)
    if not match:
        raise ScenarioError(
            f"{where}: condition {value!r} is not '<path> <op> <literal>'")
    return {"expr": value, "path": match.group(1), "op": match.group(2),
            "literal": _scalar(match.group(3))}


def _normalise_wait(value: Any, where: str) -> dict:
    if isinstance(value, (int, float)):
        return {"ms": int(value)}
    if isinstance(value, str):
        return {"ms": int(_scalar(value))}
    if not isinstance(value, dict):
        raise ScenarioError(f"{where}: wait must be milliseconds or a mapping")
    if "ms" in value:
        if "until" in value:
            raise ScenarioError(f"{where}: wait cannot have both ms and until")
        return {"ms": int(value["ms"])}
    if "until" not in value:
        raise ScenarioError(f"{where}: wait needs ms or until")
    condition = _condition(value["until"], where)
    source = value.get("source")
    if source not in SOURCES:
        raise ScenarioError(
            f"{where}: wait.until needs source: client, server or script")
    timeout = int(value.get("timeout_ms", 10000))
    poll = int(value.get("poll_ms", 250))
    if timeout < 1 or poll < 1:
        raise ScenarioError(f"{where}: wait timeout_ms and poll_ms must be positive")
    return {**condition, "source": source, "timeout_ms": timeout, "poll_ms": poll}


def _normalise_target(value: Any, where: str) -> dict:
    if isinstance(value, str):
        if value == "nearest":
            raise ScenarioError(
                f"{where}: target nearest needs a name or template mapping")
        value = {"name": value, "nearest": True}
    if not isinstance(value, dict):
        raise ScenarioError(f"{where}: target must be a name or mapping")
    name = value.get("name")
    template = value.get("template") or value.get("template_id")
    if template is not None:
        try:
            template = int(template)
        except (TypeError, ValueError):
            raise ScenarioError(f"{where}: target template must be an integer")
    if name is None and template is None:
        raise ScenarioError(f"{where}: target needs a name or template")
    if name is not None and not isinstance(name, str):
        raise ScenarioError(f"{where}: target name must be a string")
    return {"name": name, "template": template,
            "nearest": bool(value.get("nearest", True)),
            "radius": float(value.get("radius", 30.0)),
            "timeout_ms": int(value.get("timeout_ms", 5000))}


def _normalise_use_skill(value: Any, where: str) -> dict:
    if isinstance(value, (int, str)) and not isinstance(value, bool):
        if str(value).startswith("slot:"):
            return {"slot": int(str(value).split(":", 1)[1]), "id": None,
                    "bar": "action", "click": False}
        return {"id": int(value), "slot": None, "bar": "action", "click": False}
    if not isinstance(value, dict):
        raise ScenarioError(f"{where}: use-skill must be an id, slot or mapping")
    skill_id = value.get("id")
    slot = value.get("slot")
    if skill_id is None and slot is None:
        raise ScenarioError(f"{where}: use-skill needs id or slot")
    if slot is not None:
        slot = int(slot)
        if not 1 <= slot <= 10:
            raise ScenarioError(f"{where}: use-skill slot must be 1..10")
    if skill_id is not None:
        skill_id = int(skill_id)
    bar = str(value.get("bar", "action"))
    if bar not in ("action", "mode"):
        raise ScenarioError(f"{where}: use-skill bar must be action or mode")
    if bar == "mode" and slot is None:
        raise ScenarioError(f"{where}: a mode use-skill needs a slot")
    return {"id": skill_id, "slot": slot, "bar": bar,
            "click": bool(value.get("click", False)),
            "allow_mismatch": bool(value.get("allow_mismatch", False))}


def _normalise_mount(value: Any, where: str) -> dict:
    if value is True or value is None:
        return {"kind": "pet", "timeout_ms": 8000}
    if isinstance(value, str):
        return {"kind": value, "timeout_ms": 8000}
    if not isinstance(value, dict):
        raise ScenarioError(f"{where}: mount must be true, a kind or a mapping")
    kind = str(value.get("kind", "pet"))
    if kind not in ("pet", "vehicle"):
        raise ScenarioError(f"{where}: mount kind must be pet or vehicle")
    return {"kind": kind, "timeout_ms": int(value.get("timeout_ms", 8000))}


def _normalise_step(raw: Any, index: int, seen: set[str]) -> dict:
    where = f"step {index}"
    if not isinstance(raw, dict):
        raise ScenarioError(f"{where}: a step must be a mapping")
    unknown = [key for key in raw
               if key not in METADATA and key not in STEP_OPTIONS
               and key not in VERBS]
    if unknown:
        raise ScenarioError(f"{where}: unknown keys {unknown}")
    step_id = raw.get("id")
    if not isinstance(step_id, str) or not STEP_ID_RE.match(step_id):
        raise ScenarioError(f"{where}: id is required and must match [A-Za-z0-9_.-]")
    if step_id in seen:
        raise ScenarioError(f"{where}: duplicate id {step_id!r}")
    seen.add(step_id)
    verb_keys = [key for key in raw if key in VERBS]
    if len(verb_keys) != 1:
        raise ScenarioError(f"{where}: exactly one verb is required, got {verb_keys}")
    key = verb_keys[0]
    verb = ALIASES.get(key, key)
    value = raw[key]
    step = {"id": step_id, "verb": verb, "note": raw.get("note", ""),
            "optional": bool(raw.get("optional", False)),
            "server_tag": raw.get("server_tag")}
    if step["server_tag"] is not None and step["server_tag"] not in SOURCES:
        raise ScenarioError(f"{where}: server_tag must be client, server or script")
    if step["server_tag"] is None and verb in ("gm", "move-to", "set-slots"):
        step["server_tag"] = "script"
    if verb == "gm":
        text = str(value)
        if not text:
            raise ScenarioError(f"{where}: gm command is empty")
        _reject_forbidden(text, where)
        step["command"] = text
    elif verb == "wait":
        step["wait"] = _normalise_wait(value, where)
    elif verb == "key":
        key_name = str(value)
        if key_name not in KEY_NAMES:
            raise ScenarioError(f"{where}: unsupported key {key_name!r}")
        hold = raw.get("hold_ms", raw.get("duration_ms", 80))
        hold = int(hold)
        if not 1 <= hold <= 10000:
            raise ScenarioError(f"{where}: hold_ms must be 1..10000")
        mods = raw.get("modifiers", [])
        if isinstance(mods, str):
            mods = [mods]
        if any(mod not in MODIFIERS for mod in mods):
            raise ScenarioError(f"{where}: modifiers must be ctrl, shift or alt")
        step.update(key=key_name, hold_ms=hold, modifiers=list(mods))
    elif verb == "click-ui":
        path = str(value)
        if not WIDGET_PATH_RE.match(path):
            raise ScenarioError(f"{where}: click-ui needs a plain Lua widget path")
        _reject_forbidden_widget(path, where)
        step["widget"] = path
    elif verb == "target":
        step["target"] = _normalise_target(value, where)
    elif verb == "use-skill":
        step["use_skill"] = _normalise_use_skill(value, where)
    elif verb == "interact":
        if value not in (None, True, False) and not isinstance(value, dict):
            raise ScenarioError(f"{where}: interact must be true/false or a mapping")
        options = value if isinstance(value, dict) else {}
        step["interact"] = {
            "key": str(options.get("key", "f")),
            "hold_ms": int(options.get("hold_ms", 120)),
            "action": options.get("action"),
        }
        if step["interact"]["key"] not in KEY_NAMES:
            raise ScenarioError(f"{where}: interact key is not supported")
    elif verb == "move-to":
        values = value if isinstance(value, list) else str(value).replace(",", " ").split()
        if len(values) != 3:
            raise ScenarioError(f"{where}: move-to needs x y z")
        step["position"] = [float(part) for part in values]
    elif verb == "mount":
        step["mount"] = _normalise_mount(value, where)
    elif verb == "dismount":
        step["dismount"] = True
    elif verb == "snapshot":
        label = step_id if value in (None, True) else str(value)
        step["label"] = label
    elif verb == "mark":
        step["label"] = step_id if value in (None, True) else str(value)
    elif verb == "lua":
        if not isinstance(value, dict) or "code" not in value:
            raise ScenarioError(f"{where}: lua needs a mapping with code")
        code = str(value["code"])
        _reject_forbidden(code, where)
        target = str(value.get("target", "ui"))
        if target not in ("ui", "system", "third"):
            raise ScenarioError(f"{where}: lua target must be ui, system or third")
        step["lua"] = {"target": target, "code": code}
    elif verb == "set-slots":
        if not isinstance(value, dict):
            raise ScenarioError(f"{where}: set-slots needs a mapping")
        slots = value.get("slots") or {}
        step["set_slots"] = {
            "page": int(value.get("page", 1)),
            "slots": {int(slot): int(skill) for slot, skill in slots.items()},
        }
    return step


def _reject_forbidden(text: str, where: str) -> None:
    lowered = text.lower()
    for token in FORBIDDEN:
        if token in lowered:
            raise ScenarioError(
                f"{where}: {token!r} is not allowed in a scenario; stop by pid instead")


FORBIDDEN_WIDGET = ("exit", "quit", "leaveworld", "leave_world",
                   "menuexit", "menu_exit")


def _reject_forbidden_widget(path: str, where: str) -> None:
    lowered = path.lower()
    for token in FORBIDDEN_WIDGET:
        if token in lowered:
            raise ScenarioError(
                f"{where}: click-ui target {path!r} names an exit/quit widget")


def validate_scenario(data: Any, source: str = "<scenario>") -> dict:
    if not isinstance(data, dict):
        raise ScenarioError(f"{source}: the scenario must be a mapping")
    allowed = {"name", "version", "description", "defaults", "steps",
               "preconditions", "tags"}
    unknown = [key for key in data if key not in allowed]
    if unknown:
        raise ScenarioError(f"{source}: unknown top-level keys {unknown}")
    name = data.get("name")
    if not isinstance(name, str) or not STEP_ID_RE.match(name):
        raise ScenarioError(f"{source}: name is required and must be file-safe")
    version = data.get("version", 1)
    if version != 1:
        raise ScenarioError(f"{source}: unsupported scenario version {version!r}")
    defaults = data.get("defaults") or {}
    if not isinstance(defaults, dict):
        raise ScenarioError(f"{source}: defaults must be a mapping")
    raw_steps = data.get("steps")
    if not isinstance(raw_steps, list) or not raw_steps:
        raise ScenarioError(f"{source}: steps must be a non-empty list")
    seen: set[str] = set()
    steps = [_normalise_step(step, index, seen)
             for index, step in enumerate(raw_steps)]
    preconditions: list[dict] = []
    raw_pre = data.get("preconditions") or []
    if not isinstance(raw_pre, list):
        raise ScenarioError(f"{source}: preconditions must be a list")
    for index, step in enumerate(raw_pre):
        preconditions.append(_normalise_step(step, index, seen))
    return {
        "name": name,
        "version": version,
        "description": str(data.get("description", "")),
        "tags": list(data.get("tags") or []),
        "defaults": defaults,
        "preconditions": preconditions,
        "steps": steps,
        "source": source,
    }


def condition_holds(condition: dict, state: dict) -> bool:
    path = condition["path"]
    current: Any = state
    for part in path.split("."):
        if not isinstance(current, dict) or part not in current:
            current = None
            break
        current = current[part]
    wanted = condition["literal"]
    op = condition["op"]
    if current is None:
        if op == "==":
            return wanted is None
        if op == "!=":
            return wanted is not None
        return False
    if op == "contains":
        return str(wanted) in str(current)
    if isinstance(current, bool) or isinstance(wanted, bool):
        left, right = bool(current), bool(wanted)
    elif isinstance(current, (int, float)) and isinstance(wanted, (int, float)):
        left, right = float(current), float(wanted)
    else:
        left, right = str(current), str(wanted)
    if op == "==":
        return left == right
    if op == "!=":
        return left != right
    try:
        if op == "<":
            return left < right
        if op == "<=":
            return left <= right
        if op == ">":
            return left > right
        if op == ">=":
            return left >= right
    except TypeError:
        return False
    return False


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: python tools/parity/scenario.py <scenario.yaml|json>",
              file=sys.stderr)
        return 2
    scenario = load_scenario(Path(argv[1]))
    print(json.dumps(scenario, indent=1, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
