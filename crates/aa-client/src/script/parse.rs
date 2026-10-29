//! The script language: one command per line, `#` starts a comment. Parsed up front, so a typo fails before the first
//! command runs (exit code 2).

/// Which unit (or doodad) a command means.
#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    /// The nearest alive unit other than the player.
    Nearest,
    /// The nearest alive unit hostile to the player.
    Hostile,
    /// The nearest alive NPC that is not hostile.
    Friendly,
    /// The nearest corpse (dead unit).
    Corpse,
    Id(u32),
    /// The nearest NPC of this template (`npcs.id`); a bare number in a script is this.
    Template(u32),
    /// The nearest unit whose name contains the text (case-insensitive).
    Name(String),
    SelfUnit,
    /// The current target.
    Current,
}

impl Spec {
    fn parse(s: &str) -> Result<Spec, String> {
        let lower = s.to_ascii_lowercase();
        Ok(match lower.as_str() {
            "nearest" | "any" => Spec::Nearest,
            "hostile" | "mob" | "enemy" => Spec::Hostile,
            "friendly" | "npc" => Spec::Friendly,
            "corpse" | "dead" => Spec::Corpse,
            "self" | "me" => Spec::SelfUnit,
            "current" | "target" => Spec::Current,
            _ => {
                if let Some(v) = lower.strip_prefix("id:") {
                    Spec::Id(v.parse().map_err(|_| format!("bad object id '{s}'"))?)
                } else if let Some(v) = lower.strip_prefix("template:") {
                    Spec::Template(v.parse().map_err(|_| format!("bad template '{s}'"))?)
                } else if s.len() > 5 && lower.starts_with("name:") {
                    Spec::Name(s[5..].to_string())
                } else if let Ok(n) = s.parse() {
                    Spec::Template(n)
                } else {
                    return Err(format!(
                        "not a unit spec: '{s}' (nearest, hostile, friendly, corpse, self, current, id:N, template:N, name:TEXT or an NPC template number)"
                    ));
                }
            }
        })
    }
}

/// A condition of `expect` / `waitfor`; `negate` flips it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cond {
    pub negate: bool,
    pub kind: CondKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CondKind {
    InWorld,
    /// Some target is selected.
    Target,
    TargetName(String),
    TargetTemplate(u32),
    TargetDead,
    TargetAlive,
    /// The target has lost health since it was first seen.
    TargetHurt,
    Dead,
    Alive,
    /// The last `interact` started a talk (the dialog or a service window).
    Dialog,
    Quest(u32),
    QuestDone(u32),
    /// The player is at least this far (metres) from the position of the last `mark` (or of the start).
    Moved(f32),
    Near(f32, f32, f32),
    /// The bag holds at least `count` of the item template.
    Item(u32, i64),
    /// A corpse with loot is in reach.
    Lootable,
    Chat(String),
    /// A client event (the name of an `Event` variant) was seen since the script started.
    Event(String),
    /// No Lua error, decode failure or lost packet batch so far.
    NoErrors,
    /// The Lua expression (in the in-game UI's state) is true.
    Lua(String),
    /// A unit of the spec is in sight.
    Unit(Spec),
    UnitDead(Spec),
    /// The player's health is below this percentage.
    HpBelow(f32),
    /// A duel is running / a challenge waits for an answer.
    Duel,
    DuelChallenge,
    /// The last duel ended with this result key (`draw`, `winner`, `loser`, `runaway`, `remained`).
    DuelResult(String),
    /// The player sits on a mount (`SCUnitAttached`).
    Riding,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    Wait(f32),
    /// The script starts once the character is in the world; this line only documents it.
    Enter,
    Echo(String),
    Mark,
    /// Walk to a position (game coordinates).
    WalkTo(f32, f32),
    /// Walk this many metres in the direction the character faces.
    WalkForward(f32),
    /// Walk next to a unit.
    WalkToUnit(Spec),
    /// Turn the character and camera to a heading (degrees, counter-clockwise from north).
    Face(f32),
    /// Turn the character and camera to a unit.
    FaceUnit(Spec),
    /// Turn the camera alone around the player, by degrees from the way the character faces (a side view for a shot).
    Orbit(f32),
    /// Wheel notches of the camera's zoom, one after the other: positive zooms in (1.2's `zoom_in`), negative out.
    Zoom(f32),
    /// The camera's pitch in degrees (positive looks up), held to its limits, as a drag would leave it.
    Pitch(f32),
    Tp(f32, f32, f32),
    Gm(String),
    Chat(String),
    Target(Spec),
    /// Left click on a unit (the click path of the mouse: the pick, not the ray).
    Click(Spec),
    Cast(u32, Option<Spec>),
    CastSlot(usize),
    /// Keep casting (a skill id, or `slot N`) at a unit every 1.5 s until it is dead, at most this many seconds.
    Fight { skill: Option<u32>, slot: usize, spec: Spec, secs: f32 },
    StopCast,
    Interact(Spec),
    InteractDoodad(Spec, u32),
    Accept(Option<u32>),
    Complete(Option<u32>, i32),
    EndTalk,
    /// `true`: `loot` (fails when there is nothing to take); `tryloot` only reports it.
    Loot(Spec, bool),
    Open(String),
    Key(String),
    /// Hold movement keys for a number of seconds (the `--walk` letters: W A S D Q E, J, X, H).
    Hold(String, f32),
    /// Set the held movement keys without waiting (`keys` alone releases them).
    Keys(String),
    Type(String),
    HudClick(f32, f32, bool),
    HudWidget(String),
    HudHover(f32, f32),
    Lua(String),
    Await { event: String, secs: f32, must: bool },
    WaitFor(Cond, f32),
    Expect(Cond),
    Shot(String),
    Units(Vec<u32>),
    Me,
    Bag,
    /// Request the ordinary skill of a bag item of this template.
    Use(u32),
    Quests,
    /// `true`: log out and wait for the server to close the connection; `quit now` drops the connection.
    Quit(bool),
}

#[derive(Clone, Debug)]
pub struct Line {
    pub no: usize,
    pub text: String,
    pub cmd: Cmd,
}

fn num(s: Option<&&str>, what: &str) -> Result<f32, String> {
    let s = s.ok_or_else(|| format!("missing {what}"))?;
    s.parse().map_err(|_| format!("bad {what} '{s}'"))
}

fn int(s: Option<&&str>, what: &str) -> Result<u32, String> {
    let s = s.ok_or_else(|| format!("missing {what}"))?;
    s.parse().map_err(|_| format!("bad {what} '{s}'"))
}

/// The line after its first `skip` words, spacing kept (Lua strings, chat text).
fn rest(line: &str, skip: usize) -> String {
    let mut s = line.trim();
    for _ in 0..skip {
        s = s.find(char::is_whitespace).map_or("", |i| s[i..].trim_start());
    }
    s.to_string()
}

/// The line without its comment: a `#` that starts the line or stands alone after a space (`#t`, Lua's length operator, stays).
fn strip_comment(line: &str) -> &str {
    let b = line.as_bytes();
    let starts = |i: usize| line[..i].trim().is_empty();
    let alone = |i: usize| b[i - 1].is_ascii_whitespace() && b.get(i + 1).is_none_or(u8::is_ascii_whitespace);
    let at = (0..b.len()).find(|&i| b[i] == b'#' && (starts(i) || alone(i)));
    at.map_or(line, |i| &line[..i]).trim()
}

fn spec(a: &[&str], i: usize, default: Spec) -> Result<Spec, String> {
    a.get(i).map_or(Ok(default), |s| Spec::parse(s))
}

fn parse_cond(a: &[&str]) -> Result<Cond, String> {
    let first = a.first().ok_or("missing condition")?;
    let (negate, name) = match first.strip_prefix('!') {
        Some(n) => (true, n),
        None => (false, *first),
    };
    let b = &a[1..];
    let tail = b.join(" ");
    let kind = match name.to_ascii_lowercase().as_str() {
        "in_world" | "inworld" => CondKind::InWorld,
        "target" => CondKind::Target,
        "target_name" => CondKind::TargetName(tail),
        "target_template" => CondKind::TargetTemplate(int(b.first(), "template")?),
        "target_dead" => CondKind::TargetDead,
        "target_alive" => CondKind::TargetAlive,
        "target_hurt" => CondKind::TargetHurt,
        "dead" => CondKind::Dead,
        "alive" => CondKind::Alive,
        "dialog" | "talk" => CondKind::Dialog,
        "quest" => CondKind::Quest(int(b.first(), "quest id")?),
        "quest_done" => CondKind::QuestDone(int(b.first(), "quest id")?),
        "moved" => CondKind::Moved(num(b.first(), "metres")?),
        "near" => CondKind::Near(num(b.first(), "x")?, num(b.get(1), "y")?, num(b.get(2), "radius")?),
        "item" => CondKind::Item(
            int(b.first(), "item template")?,
            b.get(1).map_or(Ok(1), |c| c.parse().map_err(|_| "bad count".to_string()))?,
        ),
        "lootable" => CondKind::Lootable,
        "chat" => CondKind::Chat(tail),
        "event" => CondKind::Event(b.first().ok_or("missing event name")?.to_string()),
        "no_errors" => CondKind::NoErrors,
        "lua" => CondKind::Lua(tail),
        "unit" => CondKind::Unit(spec(b, 0, Spec::Nearest)?),
        "unit_dead" => CondKind::UnitDead(spec(b, 0, Spec::Corpse)?),
        "hp_below" => CondKind::HpBelow(num(b.first(), "percent")?),
        "riding" | "mounted" => CondKind::Riding,
        "duel" => CondKind::Duel,
        "duel_challenge" => CondKind::DuelChallenge,
        "duel_result" => {
            let key = b.first().ok_or("missing result (draw, winner, loser, runaway, remained)")?.to_ascii_lowercase();
            if !["draw", "winner", "loser", "runaway", "remained"].contains(&key.as_str()) {
                return Err(format!("unknown duel result '{key}' (draw, winner, loser, runaway, remained)"));
            }
            CondKind::DuelResult(key)
        }
        other => return Err(format!("unknown condition '{other}'")),
    };
    Ok(Cond { negate, kind })
}

pub fn parse_line(line: &str) -> Result<Option<Cmd>, String> {
    let text = strip_comment(line);
    if text.is_empty() {
        return Ok(None);
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    let name = words[0].to_ascii_lowercase();
    let a = &words[1..];
    let first = a.first().map(|s| s.to_ascii_lowercase());
    let cmd = match name.as_str() {
        "wait" | "pump" | "sleep" => Cmd::Wait(num(a.first(), "seconds")?),
        "enter" | "login" => Cmd::Enter,
        "echo" => Cmd::Echo(rest(text, 1)),
        "mark" => Cmd::Mark,
        "walk" | "move" | "goto" => match first.as_deref() {
            None => return Err("walk needs a distance, 'X Y' or 'to <unit>'".into()),
            Some("to") => Cmd::WalkToUnit(spec(a, 1, Spec::Nearest)?),
            Some(_) if a.len() == 1 => Cmd::WalkForward(num(a.first(), "metres")?),
            Some(_) => Cmd::WalkTo(num(a.first(), "x")?, num(a.get(1), "y")?),
        },
        "face" | "turn" => match a.first().map(|s| s.parse::<f32>()) {
            Some(Ok(deg)) => Cmd::Face(deg),
            Some(Err(_)) => Cmd::FaceUnit(spec(a, 0, Spec::Nearest)?),
            None => return Err("face needs degrees or a unit".into()),
        },
        "orbit" => Cmd::Orbit(num(a.first(), "degrees")?),
        "zoom" => Cmd::Zoom(num(a.first(), "notches")?),
        "pitch" => Cmd::Pitch(num(a.first(), "degrees")?),
        "tp" => Cmd::Tp(num(a.first(), "x")?, num(a.get(1), "y")?, num(a.get(2), "z")?),
        "gm" | "gmq" => Cmd::Gm(rest(text, 1)),
        "chat" | "say" => Cmd::Chat(rest(text, 1)),
        "target" => Cmd::Target(spec(a, 0, Spec::Nearest)?),
        "click" | "pick" => Cmd::Click(spec(a, 0, Spec::Nearest)?),
        "cast" => match first.as_deref() {
            Some("slot") => Cmd::CastSlot(int(a.get(1), "slot")? as usize),
            _ => Cmd::Cast(int(a.first(), "skill id")?, a.get(1).map(|s| Spec::parse(s)).transpose()?),
        },
        "fight" => {
            let (skill, slot) = if first.as_deref() == Some("slot") { (None, int(a.get(1), "slot")? as usize) } else { (Some(int(a.first(), "skill id")?), 0) };
            let at = if skill.is_none() { 2 } else { 1 };
            Cmd::Fight { skill, slot, spec: spec(a, at, Spec::Hostile)?, secs: a.get(at + 1).map_or(Ok(60.0), |s| s.parse().map_err(|_| format!("bad seconds '{s}'")))? }
        }
        "stopcast" => Cmd::StopCast,
        "interact" | "talk" => match first.as_deref() {
            Some("doodad") => Cmd::InteractDoodad(spec(a, 1, Spec::Nearest)?, int(a.get(2), "skill id")?),
            _ => Cmd::Interact(spec(a, 0, Spec::Friendly)?),
        },
        "accept" => Cmd::Accept(a.first().map(|q| q.parse().map_err(|_| format!("bad quest id '{q}'"))).transpose()?),
        "complete" | "turnin" => Cmd::Complete(
            a.first().map(|q| q.parse().map_err(|_| format!("bad quest id '{q}'"))).transpose()?,
            a.get(1).map_or(Ok(0), |r| r.parse().map_err(|_| format!("bad reward '{r}'")))?,
        ),
        "endtalk" => Cmd::EndTalk,
        "loot" | "tryloot" => Cmd::Loot(spec(a, 0, Spec::Corpse)?, name == "loot"),
        "open" | "toggle" => Cmd::Open(a.first().ok_or("missing hotkey action (toggle_bag, toggle_character, ...)")?.to_string()),
        "close" | "escape" => Cmd::Key("escape".into()),
        "key" => Cmd::Key(a.first().ok_or("missing key name (enter, escape, tab, slotN, f, g ...)")?.to_ascii_lowercase()),
        "hold" => Cmd::Hold(
            a.first().ok_or("hold needs keys (WASDQE, J, X, H)")?.to_ascii_uppercase(),
            num(a.get(1), "seconds")?,
        ),
        "keys" => Cmd::Keys(
            a.first().map(|s| s.to_ascii_uppercase()).unwrap_or_default(),
        ),
        "type" => Cmd::Type(rest(text, 1)),
        "hudclick" | "uiclick" => Cmd::HudClick(num(a.first(), "x")?, num(a.get(1), "y")?, false),
        "hudrclick" | "uirclick" => Cmd::HudClick(num(a.first(), "x")?, num(a.get(1), "y")?, true),
        "uiclickwidget" => {
            if a.is_empty() { return Err("missing widget expression".into()); }
            Cmd::HudWidget(rest(text, 1))
        }
        "hudhover" | "uihover" => Cmd::HudHover(num(a.first(), "x")?, num(a.get(1), "y")?),
        "lua" => Cmd::Lua(rest(text, 1)),
        "await" | "tryawait" => Cmd::Await {
            event: a.first().ok_or("missing event name")?.to_string(),
            secs: a
                .get(1)
                .map_or(Ok(if name == "await" { 60.0 } else { 10.0 }), |s| s.parse().map_err(|_| format!("bad seconds '{s}'")))?,
            must: name == "await",
        },
        "waitfor" => {
            let secs = a.first().and_then(|s| s.parse::<f32>().ok());
            let from = usize::from(secs.is_some());
            Cmd::WaitFor(parse_cond(&a[from..])?, secs.unwrap_or(30.0))
        }
        "expect" => Cmd::Expect(parse_cond(a)?),
        "shot" => Cmd::Shot(a.first().ok_or("missing file name")?.to_string()),
        "units" => Cmd::Units(a.iter().map(|s| s.parse().map_err(|_| format!("bad template '{s}'"))).collect::<Result<_, _>>()?),
        "me" => Cmd::Me,
        "bag" => Cmd::Bag,
        "use" => Cmd::Use(int(a.first(), "item template")?),
        "quests" => Cmd::Quests,
        "quit" | "exit" => Cmd::Quit(first.as_deref() != Some("now")),
        other => return Err(format!("unknown command '{other}'")),
    };
    Ok(Some(cmd))
}

/// Parses a whole script; errors name the line.
pub fn parse(source: &str, file: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        match parse_line(raw) {
            Ok(Some(cmd)) => lines.push(Line { no: i + 1, text: strip_comment(raw).to_string(), cmd }),
            Ok(None) => {}
            Err(e) => return Err(format!("{file}:{}: {e}: {}", i + 1, raw.trim())),
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_commands() {
        let src = "# comment\nwalk 50\nwalk 1 2\nwalk to nearest\ntarget friendly\ncast 11002 hostile\nexpect !target_dead\nwaitfor 5 quest 7\nawait QuestStarted 12\ninteract 24\n";
        let lines = parse(src, "t").unwrap();
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[0].cmd, Cmd::WalkForward(50.0));
        assert_eq!(lines[1].cmd, Cmd::WalkTo(1.0, 2.0));
        assert_eq!(lines[2].cmd, Cmd::WalkToUnit(Spec::Nearest));
        assert_eq!(lines[4].cmd, Cmd::Cast(11002, Some(Spec::Hostile)));
        assert_eq!(lines[5].cmd, Cmd::Expect(Cond { negate: true, kind: CondKind::TargetDead }));
        assert_eq!(lines[6].cmd, Cmd::WaitFor(Cond { negate: false, kind: CondKind::Quest(7) }, 5.0));
        assert_eq!(lines[7].cmd, Cmd::Await { event: "QuestStarted".into(), secs: 12.0, must: true });
        assert_eq!(lines[8].cmd, Cmd::Interact(Spec::Template(24)));
    }

    #[test]
    fn parses_the_hold_command() {
        let lines = parse("hold WA 3.5\nhold s 0.2\nkeys wd\nkeys\n", "t").unwrap();
        assert_eq!(lines[0].cmd, Cmd::Hold("WA".into(), 3.5));
        assert_eq!(lines[1].cmd, Cmd::Hold("S".into(), 0.2));
        assert_eq!(lines[2].cmd, Cmd::Keys("WD".into()));
        assert_eq!(lines[3].cmd, Cmd::Keys(String::new()));
        assert!(parse("hold", "s").is_err());
        assert!(parse("hold W", "s").is_err());
    }

    #[test]
    fn parses_the_duel_commands() {
        let lines = parse("face 90
face name:Dcbot
expect duel
waitfor 9 duel_challenge
expect !duel
expect duel_result Remained
", "t").unwrap();
        assert_eq!(lines[0].cmd, Cmd::Face(90.0));
        assert!(matches!(&lines[1].cmd, Cmd::FaceUnit(Spec::Name(n)) if n == "Dcbot"));
        assert!(matches!(&lines[2].cmd, Cmd::Expect(c) if c.kind == CondKind::Duel && !c.negate));
        assert!(matches!(&lines[3].cmd, Cmd::WaitFor(c, _) if c.kind == CondKind::DuelChallenge));
        assert!(matches!(&lines[4].cmd, Cmd::Expect(c) if c.kind == CondKind::Duel && c.negate));
        assert!(matches!(&lines[5].cmd, Cmd::Expect(c) if c.kind == CondKind::DuelResult("remained".into())));
        assert!(parse("expect duel_result won", "s").is_err());
        assert!(parse("face", "s").is_err());
    }

    #[test]
    fn parses_the_mount_commands() {
        let lines = parse("use 4177\nwaitfor 10 riding\nexpect !mounted\norbit -90\n", "t").unwrap();
        assert_eq!(lines[0].cmd, Cmd::Use(4177));
        assert_eq!(lines[1].cmd, Cmd::WaitFor(Cond { negate: false, kind: CondKind::Riding }, 10.0));
        assert_eq!(lines[2].cmd, Cmd::Expect(Cond { negate: true, kind: CondKind::Riding }));
        assert_eq!(lines[3].cmd, Cmd::Orbit(-90.0));
        let camera = parse("zoom -1
pitch -30
", "t").unwrap();
        assert_eq!((&camera[0].cmd, &camera[1].cmd), (&Cmd::Zoom(-1.0), &Cmd::Pitch(-30.0)));
        assert!(parse("use", "s").is_err() && parse("use horse", "s").is_err());
    }

    #[test]
    fn unknown_commands_fail_with_the_line() {
        let e = parse("wait 1\nfrobnicate 3\n", "s.txt").unwrap_err();
        assert!(e.starts_with("s.txt:2: unknown command 'frobnicate'"), "{e}");
        assert!(parse("expect bogus", "s").is_err());
        assert!(parse("walk", "s").is_err());
    }

    #[test]
    fn widget_click_preserves_the_lua_expression() {
        assert_eq!(parse_line("uiclickwidget newbag.slots.btns[QA_BY_UID(QA_DONOR_UID)]").unwrap(), Some(Cmd::HudWidget("newbag.slots.btns[QA_BY_UID(QA_DONOR_UID)]".into())));
        assert!(parse_line("uiclickwidget").is_err());
    }

    #[test]
    fn comments_keep_lua_length_and_spacing() {
        let lines = parse("  # a comment
lua print(#t ..  'a  b') # trailing
echo a  b#c
", "t").unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].cmd, Cmd::Lua("print(#t ..  'a  b')".into()));
        assert_eq!(lines[1].cmd, Cmd::Echo("a  b#c".into()));
    }
}
