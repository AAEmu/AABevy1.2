//! The slash commands of the chat input.
//!
//! The words come from the game database like the client's own table: `slash_commands` rows of
//! `ChatCommand` name a channel (`/ex` guild, `/fa` family, `/co` raid leaders, `/tr` trial, `/r` reply
//! to the last whisper ...), rows of `SlashFunction` a function (`/invite`, `/kick`, `/roll`, `/block`,
//! `/ccreate` ...). The toolkit's chat input already swallows the channel switches it knows (`/s`, `/p`,
//! `/w Name` ...); what reaches `AAUI.ChatSend` as a line that still starts with a slash is looked up
//! here (`HUD.slash` in `commands.lua`), and every other slash line goes out as a chat line (or an
//! emote, see `api.rs`).
//!
//! Functions that have a game API (teams, friends, family, guild) call the same `X2*` methods the
//! original windows call, so their checks and answers are the windows' own. Rolling the dice, player
//! channels and the two skills of the table (`/report`, the unstuck `/escape`) have packets or skills of
//! their own: they reach the app as [`Request::ChatCommand`] and [`Request::Skill`].

use std::collections::HashMap;
use std::sync::OnceLock;

use aa_net::ChatCommand;
use mlua::{Lua, Table};
use rusqlite::{Connection, OpenFlags};

use super::super::api::{Request, Shared};

/// `chat_type` of the reply command (`/r`): the last whisper's sender, as a whisper.
#[cfg(test)]
const REPLY: i16 = -5;

/// What a slash word stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    /// A channel command: the rest of the line is said in this channel.
    Channel(i16),
    /// A function (`slash_functions` id) and the skill the row names (0: none).
    Function { id: u32, skill: u32 },
}

/// The slash words of the database, lower case.
#[derive(Debug, Default)]
pub struct SlashTable {
    words: HashMap<String, Word>,
}

/// Words the database lacks that the original texts mention (`ui_texts` "Command format: /addfamily name title").
const EXTRA: &[(&str, u32)] = &[("addfamily", 18), ("familytitle", 22)];

impl SlashTable {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut channels: HashMap<i64, i16> = HashMap::new();
        let mut q = conn.prepare("SELECT id, chat_type_id FROM chat_commands")?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.flatten() {
            channels.insert(row.0, row.1 as i16);
        }
        let mut words = HashMap::new();
        let mut q = conn.prepare("SELECT action_id, action_type, skill_id, command_list FROM slash_commands WHERE action_type IN ('ChatCommand', 'SlashFunction') ORDER BY id")?;
        let rows = q.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i64>>(2)?.unwrap_or(0), r.get::<_, String>(3)?))
        })?;
        for (action, kind, skill, list) in rows.flatten() {
            let word = if kind == "ChatCommand" {
                match channels.get(&action) {
                    Some(&chat_type) => Word::Channel(chat_type),
                    None => continue,
                }
            } else {
                Word::Function { id: action as u32, skill: skill as u32 }
            };
            for w in list.split_whitespace() {
                words.entry(w.to_lowercase()).or_insert(word);
            }
        }
        for (w, id) in EXTRA {
            words.entry((*w).to_string()).or_insert(Word::Function { id: *id, skill: 0 });
        }
        Ok(SlashTable { words })
    }

    /// The table of `AA_DB` (or the default database); empty when it cannot be read.
    pub fn from_env() -> Self {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI);
        match conn.map_err(|e| e.to_string()).and_then(|c| Self::load(&c).map_err(|e| e.to_string())) {
            Ok(t) => t,
            Err(e) => {
                bevy::log::warn!("chat: slash commands: {path}: {e}");
                SlashTable::default()
            }
        }
    }

    fn shared() -> &'static SlashTable {
        static TABLE: OnceLock<SlashTable> = OnceLock::new();
        TABLE.get_or_init(SlashTable::from_env)
    }

    /// `/word rest of the line` -> what the word is and the rest, trimmed. `None`: not a slash line, or a word the
    /// table does not know.
    fn parse<'a>(&self, text: &'a str) -> Option<(Word, &'a str)> {
        let text = text.trim_start().strip_prefix('/')?;
        let (word, rest) = match text.split_once(char::is_whitespace) {
            Some((w, r)) => (w, r.trim()),
            None => (text, ""),
        };
        self.words.get(word.to_lowercase().as_str()).map(|w| (*w, rest))
    }
}

/// Installs `HUD.slash_parse(text)` -> `"channel", chat_type, message` | `"function", id, args, skill` | nothing, and
/// `HUD.chat_command(kind, ...)`, which `commands.lua` uses; and loads that file.
pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    hud.raw_set(
        "slash_parse",
        lua.create_function(|lua, text: String| {
            Ok(match SlashTable::shared().parse(&text) {
                Some((Word::Channel(chat_type), rest)) => mlua::MultiValue::from_iter([
                    mlua::Value::String(lua.create_string("channel")?),
                    mlua::Value::Integer(chat_type.into()),
                    mlua::Value::String(lua.create_string(rest)?),
                ]),
                Some((Word::Function { id, skill }, rest)) => mlua::MultiValue::from_iter([
                    mlua::Value::String(lua.create_string("function")?),
                    mlua::Value::Integer(id.into()),
                    mlua::Value::String(lua.create_string(rest)?),
                    mlua::Value::Integer(skill.into()),
                ]),
                None => mlua::MultiValue::new(),
            })
        })?,
    )?;
    let b = bridge.clone();
    hud.raw_set(
        "chat_command",
        lua.create_function(move |_, (kind, a, b2, c): (String, mlua::Value, mlua::Value, mlua::Value)| {
            let text = |v: &mlua::Value| match v {
                mlua::Value::String(s) => s.to_string_lossy(),
                _ => String::new(),
            };
            let number = |v: &mlua::Value| match v {
                mlua::Value::Integer(n) => *n,
                mlua::Value::Number(n) => *n as i64,
                _ => 0,
            };
            let request = match kind.as_str() {
                "roll" => Request::ChatCommand(ChatCommand::RollDice { max: number(&a).clamp(0, i64::from(i32::MAX)) as i32 }),
                "join" => Request::ChatCommand(ChatCommand::JoinChannel { name: text(&a), password: text(&b2), create: matches!(c, mlua::Value::Boolean(true)) }),
                "leave" => Request::ChatCommand(ChatCommand::LeaveChannel { identity: number(&a) }),
                "skill" => Request::Skill { skill: number(&a).clamp(0, i64::from(u32::MAX)) as u32, target_name: text(&b2) },
                "target" => Request::TargetByName { name: text(&a) },
                _ => return Ok(()),
            };
            b.borrow_mut().requests.push(request);
            Ok(())
        })?,
    )
}

/// The unit in view named `name` (the spelling does not matter), a player before an NPC of that name.
pub fn unit_named(units: &aa_net::state::Units, name: &str) -> Option<u32> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut found = units.iter().filter(|u| u.name.eq_ignore_ascii_case(name));
    let first = found.next()?;
    Some(first.obj_id)
}

const SCRIPT: &str = include_str!("commands.lua");

/// `commands.lua`, after the scripts it calls into.
pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(SCRIPT).set_name("@commands.lua").exec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> SlashTable {
        let mut words = HashMap::new();
        words.insert("invite".to_string(), Word::Function { id: 1, skill: 0 });
        words.insert("ex".to_string(), Word::Channel(7));
        words.insert("report".to_string(), Word::Function { id: 40, skill: 20256 });
        SlashTable { words }
    }

    #[test]
    fn a_slash_line_is_split_into_word_and_rest() {
        let t = table();
        assert_eq!(t.parse("/invite Bob"), Some((Word::Function { id: 1, skill: 0 }, "Bob")));
        assert_eq!(t.parse("  /INVITE   Bob  Smith "), Some((Word::Function { id: 1, skill: 0 }, "Bob  Smith")));
        assert_eq!(t.parse("/ex hello all"), Some((Word::Channel(7), "hello all")));
        assert_eq!(t.parse("/report"), Some((Word::Function { id: 40, skill: 20256 }, "")));
        assert_eq!(t.parse("/unknown x"), None);
        assert_eq!(t.parse("invite Bob"), None);
        assert_eq!(t.parse("/"), None);
    }

    #[test]
    fn the_game_database_names_the_original_words() {
        let t = SlashTable::from_env();
        if t.words.is_empty() {
            return; // no database on this machine
        }
        assert_eq!(t.parse("/invite Bob").map(|p| p.0), Some(Word::Function { id: 1, skill: 0 }));
        assert_eq!(t.parse("/roll 20").map(|p| p.0), Some(Word::Function { id: 7, skill: 0 }));
        assert_eq!(t.parse("/ex hi").map(|p| p.0), Some(Word::Channel(7)));
        assert_eq!(t.parse("/fa hi").map(|p| p.0), Some(Word::Channel(9)));
        assert_eq!(t.parse("/r thanks").map(|p| p.0), Some(Word::Channel(REPLY)));
        assert_eq!(t.parse("/report").map(|p| p.0), Some(Word::Function { id: 40, skill: 20256 }));
        assert_eq!(t.parse("/addfamily Bob Son").map(|p| p.0), Some(Word::Function { id: 18, skill: 0 }));
    }

    fn bridge() -> super::super::super::api::Shared {
        let mut data = super::super::super::api::HudData::default();
        data.character_id = 11;
        std::rc::Rc::new(std::cell::RefCell::new(data))
    }

    #[test]
    fn typed_slash_lines_become_the_requests_of_the_original_api() -> Result<(), Box<dyn std::error::Error>> {
        if SlashTable::from_env().words.is_empty() {
            return Ok(()); // no database on this machine
        }
        let mut rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = bridge();
        super::super::super::api::install(&mut rt, &bridge)?;
        rt.lua
            .load(
                r#"
            assert(HUD.slash("/invite Bob"))
            assert(HUD.slash("/roll 20"))
            assert(HUD.slash("/roll"))
            assert(HUD.slash("/ccreate Room pw"))
            assert(HUD.slash("/ex hello all"))
            assert(HUD.slash("/r thanks"))   -- nobody whispered yet: nothing to answer
            HUD.last_whisper = "Carl"
            assert(HUD.slash("/r thanks"))
            assert(HUD.slash("/target Dana"))
            assert(HUD.slash("/report Eve"))
            assert(HUD.slash("/ex"))         -- a bare channel command only switches the channel
            assert(not HUD.slash("/sit"))    -- emotes are not ours
            assert(not HUD.slash("hello"))
            assert(not HUD.slash("/nosuchcommand x"))
            "#,
            )
            .exec()?;
        assert_eq!(
            bridge.borrow().requests,
            vec![
                Request::Party(aa_net::party::PartyCommand::Invite { team_id: 0, is_party: true, target_name: "Bob".into() }),
                Request::ChatCommand(ChatCommand::RollDice { max: 20 }),
                Request::ChatCommand(ChatCommand::RollDice { max: 100 }),
                Request::ChatCommand(ChatCommand::JoinChannel { name: "Room".into(), password: "pw".into(), create: true }),
                Request::Chat { chat_type: 7, message: "hello all".into(), target: String::new() },
                Request::Chat { chat_type: -3, message: "thanks".into(), target: "Carl".into() },
                Request::TargetByName { name: "Dana".into() },
                Request::Skill { skill: 20256, target_name: "Eve".into() },
            ]
        );
        Ok(())
    }

    #[test]
    fn a_rolled_die_makes_the_original_chat_line() -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
        super::super::super::api::install(&mut rt, &bridge())?;
        let text: Option<String> = rt.lua.load(r#"return HUD.dice_text("Bob", 100, 42)"#).eval()?;
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            assert!(text.contains("Bob") && text.contains("42") && text.contains("(1-100)"), "{text}");
        }
        Ok(())
    }
}
