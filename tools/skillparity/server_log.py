#!/usr/bin/env python3
"""Turn the lane server's game log into `net` rows (`src:"server"`) of the skillparity/v1 trace.

    python tools/skillparity/server_log.py <rs-game.log> --date 2026-11-03 --out <capture>/server.jsonl \\
        [--trace <capture>/orig/trace.jsonl --trace <capture>/bevy/trace.jsonl] [--ops all|REGEX] [--tz local|utc|+HH:MM]

The log has the local time of day only. Lines look like

    22:02:52 [INFO] handlers - StartSkill: Id 16790, flag 0, caster=26891, target=26593
    22:02:52.123456 [INFO] handlers - StartSkill: ...                  (AAEMU_LOG_MS=1: the fraction is optional)

The date comes from `--date` (default: a `YYYY-MM-DD` in the log's file name, else today); the time zone is the machine's
local one unless `--tz` says otherwise. A time that jumps back by more than 12 h counts as past midnight.

Line shapes understood (real ones from the server source, `rust/crates/aaemu-game/src`):

* `StartSkill: Id N, flag F, caster=C, target=T`            skill handler, INFO      -> `CSStartSkill` (dir out)
* `C->S level=L opcode=0xNNN CSStartSkillPacket len=N body=HEX`   `RUST_LOG=aaemu_game::net::fields=trace`
                                                          -> `CSStartSkill` decoded from the body (skill, caster, target)
                                                             and any other C->S packet by name (hex body kept in `fields.body`)
* `GamePacket: C->S NAME conn=N`                            `aaemu_game::net=debug`  -> C->S packet by name
* `GamePacket: S->C type 0A1 SCSkillStartedPacket (N bytes)`  `aaemu_game::net=debug`  -> S->C packet by name (dir in)
* `SCSkillStartedPacket { id: 14376, tl: 5, caster: Unit(1), ... }`  the `{:?}` of a sent packet, trace level of
  `aaemu_game::net::connection`: decoded fields, merged into the S->C row right before it (or a row of its own)

Packet names lose their `Packet` suffix (`SCSkillStartedPacket` -> `SCSkillStarted`, README names). Row fields: `dir`, `op`,
`skill_id`, `tl`, `obj`, `fields`. The default `--ops` keeps the skill, plot, cast, cooldown, damage, buff and movement-impulse
packets; `--ops all` keeps every packet (movement packets are many).
"""
from __future__ import annotations

import argparse
import json
import re
import struct
import sys
from datetime import date as Date, datetime, time as Time, timedelta, timezone, tzinfo
from pathlib import Path
from typing import Any, Iterable, Iterator

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common  # noqa: E402

ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
# `22:02:52 [INFO] handlers - text`, `22:02:52.123456 [TRACE] ...`; the level is optional
HEADER_RE = re.compile(r"^\s*(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?\s*(?:\[(\w+)\])?\s*(.*)$")
START_SKILL_RE = re.compile(r"StartSkill: Id (\d+), flag (\d+), caster=(\d+), target=(\d+)")
C2S_FIELDS_RE = re.compile(r"C->S\s+level=(\d+)\s+opcode=0x([0-9a-fA-F]+)\s+(\S+)\s+len=(\d+)\s+body=([0-9a-fA-F ]*)")
C2S_DEBUG_RE = re.compile(r"GamePacket:\s*C->S\s+(\S+?)(?:\s+conn=(\d+))?\s*$")
S2C_DEBUG_RE = re.compile(r"GamePacket:\s*S->C\s+type\s+([0-9a-fA-F]+)\s+(\S+)(?:\s+\((\d+)\s+bytes\))?")
DEBUG_STRUCT_RE = re.compile(r"^(?:\w+::)*(\w+Packet)\s*(\{.*\})\s*$")
DATE_IN_NAME_RE = re.compile(r"(\d{4})-(\d{2})-(\d{2})")

# what `--ops` keeps by default: everything a skill cast touches (README: server answers, effects, impulses)
DEFAULT_OPS = (r"Skill|Plot|Cast|Cooldown|Damage|Heal|Buff|Effect|Impulse|Knock|Dash|Teleport|Death|Aggro|Target|"
               r"Combat|Stun|Mana|Stat\w*Changed|Doodad|Projectile")
MERGE_MS = 60          # a field/debug line of the same packet within this many ms belongs to the same row
START_SKILL_DEDUPE_US = 1_000_000


def strip_op(name: str) -> str:
    """`.G2C.SCSkillStartedPacket` / `SCSkillStartedPacket` -> `SCSkillStarted`."""
    name = name.rsplit(".", 1)[-1].strip()
    return name[:-6] if name.endswith("Packet") and len(name) > 6 else name


def parse_tz(text: str | None) -> tzinfo | None:
    """`local` (None: the machine's zone), `utc` or `+HH:MM` / `-HH:MM`."""
    if text is None or text.lower() == "local":
        return None
    if text.lower() in ("utc", "z"):
        return timezone.utc
    m = re.fullmatch(r"([+-])(\d{1,2})(?::?(\d{2}))?", text.strip())
    if not m:
        raise ValueError(f"bad --tz {text!r}: use local, utc or +HH:MM")
    delta = timedelta(hours=int(m.group(2)), minutes=int(m.group(3) or 0))
    return timezone(delta if m.group(1) == "+" else -delta)


def wall_us_of(day: Date, hh: int, mm: int, ss: int, micro: int, tz: tzinfo | None) -> int:
    """Unix microseconds of a local time of day on `day`; `tz` None = the machine's local zone."""
    naive = datetime.combine(day, Time(hh, mm, ss))
    aware = naive.replace(tzinfo=tz) if tz is not None else naive.astimezone()
    return int(aware.timestamp()) * 1_000_000 + micro


def split_header(line: str) -> tuple[tuple[int, int, int, int], str, str] | None:
    """((h, m, s, micro), level, message) of a log line, or None when it has no time of day."""
    m = HEADER_RE.match(ANSI_RE.sub("", line.rstrip("\r\n")))
    if not m:
        return None
    frac = m.group(4)
    micro = int(frac.ljust(6, "0")[:6]) if frac else 0
    return (int(m.group(1)), int(m.group(2)), int(m.group(3)), micro), (m.group(5) or ""), m.group(6)


# ---- Rust `{:?}` output -> python -----------------------------------------------------------------------------------

_TOKEN_RE = re.compile(r'\s*(?:(?P<str>"(?:[^"\\]|\\.)*")|(?P<num>-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|(?P<id>[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)|(?P<p>[{}()\[\],:]))')


class DebugParseError(ValueError):
    pass


def _tokens(text: str) -> list[tuple[str, str]]:
    out: list[tuple[str, str]] = []
    pos = 0
    while pos < len(text):
        m = _TOKEN_RE.match(text, pos)
        if not m:
            if text[pos:].strip() == "":
                break
            raise DebugParseError(f"unexpected {text[pos:pos + 12]!r}")
        pos = m.end()
        kind = m.lastgroup or ""
        out.append((kind, m.group(kind)))
    return out


def parse_debug(text: str) -> Any:
    """Parse Rust `{:?}` text (structs, tuple variants, arrays, numbers, strings, None/Some) into dicts/lists/scalars.

    A struct `{ a: 1 }` becomes {"a": 1}; a variant `Unit(5)` becomes {"Unit": 5} (several values: {"Name": [..]});
    `Position { x: 1.0 }` becomes {"Position": {"x": 1.0}}; `None` is None and `Some(x)` is x.
    """
    toks = _tokens(text)
    pos = [0]

    def peek() -> tuple[str, str]:
        return toks[pos[0]] if pos[0] < len(toks) else ("end", "")

    def take(expect: str | None = None) -> tuple[str, str]:
        tok = peek()
        if tok[0] == "end" or (expect is not None and tok[1] != expect):
            raise DebugParseError(f"expected {expect or 'more'} at token {pos[0]}")
        pos[0] += 1
        return tok

    def seq(close: str) -> list[Any]:
        items: list[Any] = []
        while peek()[1] != close:
            items.append(value())
            if peek()[1] == ",":
                take(",")
        take(close)
        return items

    def fields() -> dict[str, Any]:
        out: dict[str, Any] = {}
        while peek()[1] != "}":
            key = take()[1]
            take(":")
            out[key] = value()
            if peek()[1] == ",":
                take(",")
        take("}")
        return out

    def value() -> Any:
        kind, tok = take()
        if kind == "str":
            try:
                return json.loads(tok)
            except ValueError:
                return tok[1:-1]
        if kind == "num":
            return float(tok) if any(c in tok for c in ".eE") else int(tok)
        if kind == "id":
            if tok == "true":
                return True
            if tok == "false":
                return False
            if tok == "None":
                return None
            nxt = peek()[1]
            if nxt == "{":
                take("{")
                body = fields()
                return body if not tok or tok[0].islower() else {tok: body}
            if nxt == "(":
                take("(")
                items = seq(")")
                if tok == "Some":
                    return items[0] if items else None
                return {tok: items[0] if len(items) == 1 else items}
            return tok
        if tok == "[":
            return seq("]")
        if tok == "(":
            return seq(")")
        if tok == "{":
            return fields()
        raise DebugParseError(f"unexpected token {tok!r}")

    # the packet's own struct is a top-level `Name { .. }`: return its fields
    kind, name = peek()
    if kind == "id" and pos[0] + 1 < len(toks) and toks[pos[0] + 1][1] == "{":
        take()
        take("{")
        return fields()
    return value()


def obj_of(value: Any) -> int | None:
    """The object id inside a decoded caster / target / plot object: `Unit(5)`, `{"Item": {"obj_id": 5}}`, 5."""
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, dict):
        if "obj_id" in value and isinstance(value["obj_id"], int):
            return value["obj_id"]
        if len(value) == 1:
            return obj_of(next(iter(value.values())))
    if isinstance(value, list) and value:
        return obj_of(value[0])
    return None


def summarize_fields(name: str, decoded: dict) -> dict:
    """skill_id / tl / obj of a decoded packet and the rest as `fields`."""
    out: dict[str, Any] = {}
    skill = decoded.get("skill_id")
    if skill is None and ("Skill" in name or "Plot" in name):
        skill = decoded.get("id")
    if isinstance(skill, int):
        out["skill_id"] = skill
    tl = decoded.get("tl")
    if isinstance(tl, int):
        out["tl"] = tl
    for key in ("caster", "obj_id", "unit", "source", "attacker"):
        obj = obj_of(decoded.get(key))
        if obj is not None:
            out["obj"] = obj
            break
    out["fields"] = decoded
    return out


# ---- C->S bodies --------------------------------------------------------------------------------------------------

class _Reader:
    def __init__(self, data: bytes):
        self.data, self.pos = data, 0

    def take(self, n: int) -> bytes:
        if self.pos + n > len(self.data):
            raise ValueError("short body")
        chunk = self.data[self.pos:self.pos + n]
        self.pos += n
        return chunk

    def u8(self) -> int:
        return self.take(1)[0]

    def u32(self) -> int:
        return struct.unpack("<I", self.take(4))[0]

    def i64(self) -> int:
        return struct.unpack("<q", self.take(8))[0]

    def f32(self) -> float:
        return struct.unpack("<f", self.take(4))[0]

    def bc(self) -> int:
        b = self.take(3)
        return b[0] | b[1] << 8 | b[2] << 16


def _fixed(v: int) -> float:
    return round((v >> 32) / 4096.0, 4)


def decode_cs_start_skill(body: bytes) -> dict | None:
    """`CSStartSkillPacket` (systems/skills/handlers.rs): skill u32, caster (type u8 + bc ..), target (type u8 + ..), flag u8."""
    try:
        r = _Reader(body)
        skill = r.u32()
        ctype = r.u8()
        caster: dict[str, Any] = {"type": ctype}
        if ctype in (0, 1, 4):
            caster["obj"] = r.bc()
        elif ctype == 2:
            caster.update(obj=r.bc(), item_id=struct.unpack("<Q", r.take(8))[0], item_template=r.u32())
            r.u8()
            r.u32()
        elif ctype == 3:
            caster.update(obj=r.bc(), mount_skill=r.u32())
        else:
            return {"skill_id": skill}
        ttype = r.u8()
        target: dict[str, Any] = {"type": ttype}
        if ttype == 0 or ttype == 4:
            target["obj"] = r.bc()
        elif ttype == 1:
            target.update(x=_fixed(r.i64()), y=_fixed(r.i64()), z=round(r.f32(), 3))
        elif ttype == 2:
            target.update(x=_fixed(r.i64()), y=_fixed(r.i64()), z=round(r.f32(), 3))
        elif ttype == 5:
            target.update(x=_fixed(r.i64()), y=_fixed(r.i64()), z=round(r.f32(), 3))
        elif ttype == 3:
            target.update(obj=r.bc())
        flag = None
        try:
            flag = r.u8()
        except ValueError:
            pass
        return {"skill_id": skill, "caster": caster, "target": target, "flag": flag}
    except ValueError:
        return None


# ---- the parser ---------------------------------------------------------------------------------------------------

class LogParser:
    """Feed log lines in order with `feed(line)`; collect rows from `.rows`. Keeps the little state the merging needs."""

    def __init__(self, day: Date, tz: tzinfo | None = None, ops: str | None = DEFAULT_OPS,
                 since_us: int | None = None, until_us: int | None = None):
        self.day = day
        self.tz = tz
        self.ops_re = re.compile(ops) if ops and ops != "all" else None
        self.since_us, self.until_us = since_us, until_us
        self.rows: list[dict] = []
        self._last_tod: int | None = None          # seconds since midnight of the previous timed line
        self._last_start: dict[int, dict] = {}     # skill id -> last CSStartSkill row (dedupe of the handler's line)
        self._last_in: dict | None = None          # last S->C row (the field line follows its debug line)
        self._last_out: dict | None = None
        self.unparsed = 0

    # -- time
    def _wall(self, tod: tuple[int, int, int, int]) -> int:
        seconds = tod[0] * 3600 + tod[1] * 60 + tod[2]
        if self._last_tod is not None and seconds < self._last_tod - 12 * 3600:
            self.day = self.day + timedelta(days=1)
        self._last_tod = seconds
        return wall_us_of(self.day, tod[0], tod[1], tod[2], tod[3], self.tz)

    def _keep(self, op: str) -> bool:
        return self.ops_re is None or bool(self.ops_re.search(op))

    def _emit(self, row: dict) -> dict:
        row["_emitted"] = True
        self.rows.append(row)
        return row

    # -- one line
    def feed(self, line: str) -> None:
        if not any(k in line for k in ("StartSkill", "Packet", "opcode=")):
            # cheap filter before the regexes; still advance the clock on timed lines so a midnight is seen
            return
        head = split_header(line)
        if head is None:
            return
        tod, _level, msg = head
        wall = self._wall(tod)
        if (self.since_us is not None and wall < self.since_us) or (self.until_us is not None and wall > self.until_us):
            return
        # strip the `target - ` part of `handlers - message`
        text = msg.split(" - ", 1)[1] if " - " in msg[:60] else msg

        m = C2S_FIELDS_RE.search(msg)
        if m:
            self._c2s_fields(wall, m)
            return
        m = START_SKILL_RE.search(msg)
        if m:
            self._start_skill_line(wall, m)
            return
        m = S2C_DEBUG_RE.search(msg)
        if m:
            op = strip_op(m.group(2))
            row = {"src": "server", "kind": "net", "wall_us": wall, "dir": "in", "op": op,
                   "fields": {"len": int(m.group(3))} if m.group(3) else {}}
            row["fields"]["opcode"] = "0x" + m.group(1).upper()
            self._last_in = row
            if self._keep(op):
                self._emit(row)
            return
        m = C2S_DEBUG_RE.search(msg)
        if m:
            op = strip_op(m.group(1))
            last = self._last_out
            if last is not None and last["op"] == op and wall - last["wall_us"] <= MERGE_MS * 1000:
                if m.group(2):
                    last["fields"]["conn"] = int(m.group(2))
                return
            row = {"src": "server", "kind": "net", "wall_us": wall, "dir": "out", "op": op,
                   "fields": {"conn": int(m.group(2))} if m.group(2) else {}}
            self._last_out = row
            if self._keep(op):
                self._emit(row)
            return
        m = DEBUG_STRUCT_RE.match(text.strip())
        if m:
            self._struct_line(wall, m.group(1), m.group(2))
            return
        self.unparsed += 1

    def _c2s_fields(self, wall: int, m: re.Match) -> None:
        op = strip_op(m.group(3))
        body_hex = m.group(5).strip()
        row: dict[str, Any] = {"src": "server", "kind": "net", "wall_us": wall, "dir": "out", "op": op,
                               "fields": {"opcode": "0x" + m.group(2).upper(), "len": int(m.group(4)), "level": int(m.group(1))}}
        if op == "CSStartSkill":
            try:
                body = bytes.fromhex(body_hex)
            except ValueError:
                body = b""
            decoded = decode_cs_start_skill(body)
            if decoded:
                row["skill_id"] = decoded["skill_id"]
                caster = decoded.get("caster") or {}
                if "obj" in caster:
                    row["obj"] = caster["obj"]
                row["fields"].update({k: v for k, v in decoded.items() if k != "skill_id"})
            self._last_start[row.get("skill_id", -1)] = row
        elif body_hex:
            row["fields"]["body"] = body_hex.lower()
        self._last_out = row
        if self._keep(op):
            self._emit(row)

    def _start_skill_line(self, wall: int, m: re.Match) -> None:
        skill, flag, caster, target = (int(m.group(i)) for i in range(1, 5))
        prior = self._last_start.get(skill)
        if prior is not None and 0 <= wall - prior["wall_us"] <= START_SKILL_DEDUPE_US:
            # the field trace already made this row (the handler logs about 100 ms after the packet arrived)
            prior["fields"].setdefault("handler_logged_us", wall)
            prior["fields"].setdefault("flag", flag)
            if prior.get("obj") is None:
                prior["obj"] = caster
            return
        row = {"src": "server", "kind": "net", "wall_us": wall, "dir": "out", "op": "CSStartSkill", "skill_id": skill,
               "obj": caster, "fields": {"flag": flag, "target": target, "from": "StartSkill log line"}}
        self._last_start[skill] = row
        self._last_out = row
        if self._keep("CSStartSkill"):
            self._emit(row)

    def _struct_line(self, wall: int, name: str, body: str) -> None:
        try:
            decoded = parse_debug(body)
        except DebugParseError:
            self.unparsed += 1
            return
        if not isinstance(decoded, dict):
            return
        op = strip_op(name)
        info = summarize_fields(op, decoded)
        last = self._last_in
        if last is not None and last["op"] == op and 0 <= wall - last["wall_us"] <= MERGE_MS * 1000 and "decoded" not in last:
            last["fields"].update(decoded)
            for key in ("skill_id", "tl", "obj"):
                if key in info:
                    last[key] = info[key]
            last["decoded"] = True
            if not last.get("_emitted") and self._keep(op):
                self._emit(last)
            return
        row = {"src": "server", "kind": "net", "wall_us": wall, "dir": "in", "op": op, "decoded": True, **info}
        self._last_in = row
        if self._keep(op):
            self._emit(row)

    def finish(self) -> list[dict]:
        for row in self.rows:
            row.pop("decoded", None)
            row.pop("_emitted", None)
            if not row.get("fields"):
                row.pop("fields", None)
        self.rows.sort(key=lambda r: r["wall_us"])
        return self.rows


def parse_lines(lines: Iterable[str], day: Date, tz: tzinfo | None = None, ops: str | None = DEFAULT_OPS,
                since_us: int | None = None, until_us: int | None = None) -> list[dict]:
    parser = LogParser(day, tz, ops, since_us, until_us)
    for line in lines:
        parser.feed(line)
    return parser.finish()


def iter_file_lines(path: Path, offset: int = 0) -> Iterator[str]:
    with path.open("rb") as handle:
        if offset:
            handle.seek(offset)
        for raw in handle:
            yield raw.decode("utf-8", "replace")


def day_for(path: Path, given: str | None) -> Date:
    if given:
        return Date.fromisoformat(given)
    m = DATE_IN_NAME_RE.search(path.name)
    if m:
        return Date(int(m.group(1)), int(m.group(2)), int(m.group(3)))
    return Date.today()


def window_of_traces(paths: list[Path], pad_s: float) -> tuple[int, int] | None:
    """(since_us, until_us) around the rows of the given trace files, so a huge log is cut to the capture."""
    lo, hi = None, None
    for p in paths:
        rows, _ = common.read_jsonl(p)
        stamps = [r["wall_us"] for r in rows if isinstance(r.get("wall_us"), (int, float))]
        if stamps:
            lo = min(stamps) if lo is None else min(lo, min(stamps))
            hi = max(stamps) if hi is None else max(hi, max(stamps))
    if lo is None or hi is None:
        return None
    pad = int(pad_s * 1_000_000)
    return int(lo) - pad, int(hi) + pad


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("log", type=Path, help="the lane server's game log (rs-game.log, Server.log)")
    ap.add_argument("--out", type=Path, default=None, help="server.jsonl to write (default: print the rows)")
    ap.add_argument("--date", default=None, help="date of the log's time of day, YYYY-MM-DD (default: from the file name, else today)")
    ap.add_argument("--tz", default="local", help="local (default), utc or +HH:MM")
    ap.add_argument("--ops", default=DEFAULT_OPS, help="regex of packet names to keep, or 'all'")
    ap.add_argument("--trace", type=Path, action="append", default=[], help="cut the log to the time range of these trace files")
    ap.add_argument("--pad", type=float, default=10.0, help="seconds around the --trace range (default 10)")
    ap.add_argument("--since-us", type=int, default=None)
    ap.add_argument("--until-us", type=int, default=None)
    ap.add_argument("--offset", type=int, default=0, help="start reading at this byte (e.g. the log size before the run)")
    args = ap.parse_args(argv)
    if not args.log.is_file():
        print(f"server_log: no such log {args.log}", file=sys.stderr)
        return 1
    since, until = args.since_us, args.until_us
    if args.trace:
        window = window_of_traces(args.trace, args.pad)
        if window is None:
            print("server_log: the --trace files have no timed rows", file=sys.stderr)
            return 1
        since, until = (window[0] if since is None else since), (window[1] if until is None else until)
    parser = LogParser(day_for(args.log, args.date), parse_tz(args.tz), args.ops, since, until)
    for line in iter_file_lines(args.log, args.offset):
        parser.feed(line)
    rows = parser.finish()
    if args.out:
        common.write_jsonl(args.out, rows)
        print(f"server_log: {len(rows)} net rows -> {args.out} ({parser.unparsed} packet-like lines not understood)")
    else:
        for row in rows:
            print(json.dumps(row, ensure_ascii=False, separators=(",", ":")))
    if not rows:
        print("server_log: no rows (check --date, --tz, the time range and that the log has the packet trace on)", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
