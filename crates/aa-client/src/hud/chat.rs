//! Chat: incoming lines and channel changes from aa-net become the UI events the chat scripts
//! listen for (`CHAT_MESSAGE`, `CHAT_JOINED_CHANNEL`, `CHAT_LEAVED_CHANNEL`), and `Enter` opens the
//! chat window's input (the scripts' own edit box, `chatwindow:GetChatEdit()`), whose text goes out
//! as `Command::Chat` (see `HUD.chat_send` in `ingame.lua`).
//!
//! The channel numbers aa-net reports are the client's own (`CHAT_SAY` 0, `CHAT_ZONE` 1 (shout),
//! `CHAT_TRADE` 2, `CHAT_FIND_PARTY` 3, `CHAT_PARTY` 4, `CHAT_RAID` 5, `CHAT_FACTION` 6,
//! `CHAT_EXPEDITION` 7, `CHAT_RACE` 14, whispers -3 / -4, notice -1, system -2), so they pass
//! through unchanged.

pub(super) mod commands;
pub(super) mod config;
pub(super) mod ui_data;

use aa_net::state::{Chat, Player};
use aa_net::{Event, NetEvent};
use aa_ui::{UiKey, UiRuntime};
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use super::api::HudBridge;
use super::data::GameData;
use super::{HudBuilt, HudKeyboardFocus};

/// The name the scripts show for a joined channel: the zone for the zone channels, the faction for
/// the faction ones, the user's name for user channels.
fn channel_name(game: Option<&GameData>, chat_type: i16, sub_type: i16, faction_id: u32, name: &str) -> String {
    if !name.is_empty() {
        return name.to_string();
    }
    let game = match game {
        Some(g) => g,
        None => return String::new(),
    };
    match chat_type {
        // shout / trade / party search: the zone group
        1..=3 => game.zone_groups.get(&(sub_type as u32)).map(|g| g.text.clone()).unwrap_or_default(),
        // faction (nation), alliance
        6 | 14 => game.factions.get(&faction_id).cloned().unwrap_or_default(),
        _ => String::new(),
    }
}

/// `HUD.user_channels` (name -> identity), which `/cleave` looks the channel up in.
fn user_channels(rt: &UiRuntime) -> Option<mlua::Table> {
    rt.lua.globals().get::<mlua::Table>("HUD").ok()?.get::<mlua::Table>("user_channels").ok()
}

fn join(rt: &UiRuntime, game: Option<&GameData>, chat_type: i16, sub_type: i16, faction_id: u32, name: &str) {
    if aa_net::chat_command::is_user_channel(chat_type)
        && let Some(channels) = user_channels(rt)
    {
        let _ = channels.set(name.to_lowercase(), aa_net::channel_identity(chat_type, sub_type, faction_id) as f64);
    }
    rt.fire_event("CHAT_JOINED_CHANNEL", (chat_type as i64, channel_name(game, chat_type, sub_type, faction_id, name)));
}

fn leave(rt: &UiRuntime, game: Option<&GameData>, chat_type: i16, sub_type: i16, faction_id: u32) {
    if aa_net::chat_command::is_user_channel(chat_type)
        && let Some(channels) = user_channels(rt)
    {
        let identity = aa_net::channel_identity(chat_type, sub_type, faction_id) as f64;
        let gone: Vec<String> = channels.pairs::<String, f64>().flatten().filter(|(_, id)| *id == identity).map(|(name, _)| name).collect();
        for name in gone {
            let _ = channels.set(name, mlua::Value::Nil);
        }
    }
    rt.fire_event("CHAT_LEAVED_CHANNEL", (chat_type as i64, channel_name(game, chat_type, sub_type, faction_id, "")));
}

/// The dice someone in range rolled, as the chat line the scripts show (`CHAT_DICE_VALUE`).
fn dice(rt: &UiRuntime, d: &aa_net::DiceRolled) {
    let text = rt
        .lua
        .globals()
        .get::<mlua::Table>("HUD")
        .and_then(|hud| hud.get::<mlua::Function>("dice_text"))
        .and_then(|f| f.call::<String>((d.name.clone(), d.max, d.value)));
    match text {
        Ok(text) => rt.fire_event("CHAT_DICE_VALUE", text),
        Err(e) => warn!("chat: dice line: {e}"),
    }
}

/// `bound`: the speaker is a unit the client has (the player or one in range): the script draws the speech bubble of say, party,
/// raid and guild lines over it (`speakerInChatBound`, see `speech.rs`). Replayed lines are old news: no bubble.
fn message(rt: &UiRuntime, m: &aa_net::model::ChatMessage, friends: &aa_net::Friends, bound: bool) {
    if (m.sender_obj_id != 0 || m.sender_id != 0) && friends.is_blocked(&m.sender) { return; }
    if m.chat_type == aa_net::model::chat_type::WHISPERED
        && let Ok(hud) = rt.lua.globals().get::<mlua::Table>("HUD")
    {
        // (`/r` answers the last whisper)
        let _ = hud.set("last_whisper", m.sender.clone());
    }
    rt.fire_event(
        "CHAT_MESSAGE",
        (m.chat_type as i64, format!("{:x}", m.sender_obj_id), false, m.sender.clone(), m.message.clone(), bound, None::<String>),
    );
}

/// The channels joined and the lines received before the HUD existed, in order.
pub fn replay(rt: &UiRuntime, chat: &Chat, game: Option<&GameData>, friends: &aa_net::Friends) {
    for c in &chat.channels {
        join(rt, game, c.chat_type, c.sub_type, c.faction_id, &c.name);
    }
    for m in &chat.log {
        message(rt, m, friends, false);
    }
}

#[cfg(test)]
mod blocked_tests {
    use super::*;

    fn line(name:&str,id:u32) -> aa_net::model::ChatMessage {
        aa_net::model::ChatMessage { chat_type:0,sub_type:0,channel_faction:0,sender_obj_id:id,
            sender_id:id,sender_race:1,faction_id:1,sender:name.into(),message:"hello".into(),ability:0 }
    }

    #[test]
    fn blocked_chat_is_filtered_for_live_and_replayed_lines_without_losing_raw_log() -> Result<(),Box<dyn std::error::Error>> {
        let rt=UiRuntime::new("en_us",1280.0,720.0)?;
        rt.lua.load("lines={}; UIParent:SetEventHandler('CHAT_MESSAGE',function(channel,id,flag,sender,text) lines[#lines+1]=sender end)").exec()?;
        let friends=aa_net::Friends {blocked:vec![aa_net::friend::BlockedUser {id:1,name:"Blocked".into()}],..default()};
        let mut chat=Chat::default();
        for m in [line("bLoCkEd",1),line("Visible",2),line("Blocked",0)] {
            chat.apply(&Event::Chat(m));
        }
        replay(&rt,&chat,None,&friends);
        assert_eq!(chat.log.len(),3);
        rt.lua.load("assert(#lines==2 and lines[1]=='Visible' and lines[2]=='Blocked'); lines={}").exec()?;
        let bridge=std::rc::Rc::new(std::cell::RefCell::new(super::super::api::HudData {friends,..default()}));
        let mut app=App::new(); app.add_message::<NetEvent>(); app.add_systems(Update,route_chat);
        app.insert_resource(HudBuilt {up:true,fresh:false}); app.insert_non_send(HudBridge(bridge)); app.insert_non_send(rt);
        app.world_mut().write_message(NetEvent(Event::Chat(line("blocked",1))));
        app.world_mut().write_message(NetEvent(Event::Chat(line("Visible",2))));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(#lines==1 and lines[1]=='Visible')").exec()?;
        Ok(())
    }
}

/// Raises the chat events as aa-net reports them.
pub fn route_chat(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    mut built: ResMut<HudBuilt>,
    bridge: NonSend<HudBridge>,
    units: Option<Res<aa_net::state::Units>>,
    player: Option<Res<Player>>,
) {
    // lines that arrived before (or while) the stage was built were replayed from the resource
    if !built.up || std::mem::take(&mut built.fresh) {
        events.clear();
        return;
    }
    let Some(rt) = rt else { return };
    let data = bridge.0.borrow();
    let game = data.game.get();
    for NetEvent(event) in events.read() {
        match event {
            Event::Chat(m) => {
                let own = player.as_ref().and_then(|p| p.spawn.as_ref()).is_some_and(|s| s.obj_id == m.sender_obj_id);
                let bound = m.sender_obj_id != 0 && (own || units.as_ref().is_some_and(|u| u.get(m.sender_obj_id).is_some()));
                message(&rt, m, &data.friends, bound)
            }
            Event::ChatChannelJoined { chat_type, sub_type, faction_id, name } => join(&rt, game, *chat_type, *sub_type, *faction_id, name),
            Event::ChatChannelLeft { chat_type, sub_type, faction_id } => leave(&rt, game, *chat_type, *sub_type, *faction_id),
            Event::Dice(d) => dice(&rt, d),
            _ => {}
        }
    }
}

/// `--hud-key NAME T` / `--hud-type TEXT T`: scripted keyboard for unattended runs (`enter`,
/// `escape`, `backspace`, `tab`, `slot3`, `f` / `g` (loot) ...; typed text), at T seconds after the HUD is up.
#[derive(Resource, Default)]
pub struct ScriptedKeys {
    steps: Vec<(f32, String, bool)>,
    started: Option<f32>,
    /// A scripted `tab` is waiting for `targeting::tab_target`.
    tab: bool,
    escape: bool,
    /// A scripted `slotN` (action bar slot N, 1 based) waits for `panels::action_keys`.
    slot: Option<usize>,
    /// A scripted `f` (loot all, `Some(true)`) or `g` (open the loot) waits for `loot::loot_input`.
    loot: Option<bool>,
}

impl ScriptedKeys {
    /// A key of `--hud-key` (or typed text) now, for the script runner (`script`).
    pub fn push_now(&mut self, v: &str, is_text: bool) {
        self.steps.push((f32::MIN, v.to_string(), is_text));
    }

    pub fn take_tab(&mut self) -> bool {
        std::mem::take(&mut self.tab)
    }

    pub fn take_escape(&mut self) -> bool {
        std::mem::take(&mut self.escape)
    }

    pub fn take_slot(&mut self) -> Option<usize> {
        self.slot.take()
    }

    /// A scripted `slotN` (1 based) waiting for `panels::action_keys`, for tests.
    #[cfg(test)]
    pub(crate) fn press_slot_for_test(&mut self, slot: usize) {
        self.slot = Some(slot.saturating_sub(1));
    }

    pub fn take_loot(&mut self) -> Option<bool> {
        self.loot.take()
    }

    pub fn from_args() -> ScriptedKeys {
        let args: Vec<String> = aa_dev::args();
        let mut steps = Vec::new();
        for (i, a) in args.iter().enumerate() {
            if (a == "--hud-key" || a == "--hud-type")
                && let (Some(v), Some(t)) = (args.get(i + 1), args.get(i + 2).and_then(|t| t.parse::<f32>().ok()))
            {
                steps.push((t, v.clone(), a == "--hud-type"));
            }
        }
        steps.sort_by(|a, b| a.0.total_cmp(&b.0));
        steps.reverse(); // pop from the end
        ScriptedKeys { steps, ..ScriptedKeys::default() }
    }
}

fn ui_key(name: &str) -> Option<UiKey> {
    Some(match name {
        "enter" => UiKey::Enter,
        "escape" => UiKey::Escape,
        "backspace" => UiKey::Backspace,
        "delete" => UiKey::Delete,
        "tab" => UiKey::Tab,
        "left" => UiKey::Left,
        "right" => UiKey::Right,
        "home" => UiKey::Home,
        "end" => UiKey::End,
        _ => return None,
    })
}

/// `Enter` with no edit box focused opens the chat input (and a scripted `--hud-key enter` does the
/// same). While an edit box has the focus the toolkit already routes the keys to it. The input opens
/// one frame after the key: the toolkit forwards the same key press to the focused box in whatever
/// order its systems run, and the very press that opens the input must not also send it.
pub fn chat_keys(
    actions: Res<crate::settings::Actions>,
    mut typed: MessageReader<KeyboardInput>,
    focus: Res<HudKeyboardFocus>,
    rt: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut script: ResMut<ScriptedKeys>,
    mut pending: Local<Option<bool>>,
) {
    let Some(rt) = rt else {
        typed.clear();
        return;
    };
    // opening requested by the last frame's key
    if let Some(slash) = pending.take()
        && !focus.0
        && !rt.keyboard_captured()
        && let Err(e) = rt.exec("@chat_open", &format!("HUD.chat_open({})", if slash { "\"/\"" } else { "nil" }))
    {
        warn!("hud: chat input: {e}");
    }
    let mut open = actions.just_pressed("open_chat", 0);
    // "/" opens the input with a slash already typed, like the real client
    let slash = typed.read().any(|e| e.state.is_pressed() && matches!(&e.logical_key, Key::Character(c) if c.as_str() == "/"));
    // scripted keys, relative to the first frame they were looked at
    let now = time.elapsed_secs();
    let started = *script.started.get_or_insert(now);
    while script.steps.last().is_some_and(|s| now - started >= s.0) {
        let (_, v, is_text) = script.steps.pop().unwrap();
        if is_text {
            rt.type_text(&v);
        } else if let Some(n) = v.strip_prefix("slot").and_then(|n| n.parse::<usize>().ok()) {
            script.slot = Some(n.saturating_sub(1));
        } else if v == "f" || v == "g" {
            script.loot = Some(v == "f");
        } else if let Some(k) = ui_key(&v) {
            if focus.0 || rt.keyboard_captured() {
                rt.key(&k);
            } else if k == UiKey::Enter {
                open = true;
            } else if k == UiKey::Tab {
                script.tab = true;
            } else if k == UiKey::Escape {
                script.escape = true;
            }
        }
    }
    if focus.0 || rt.keyboard_captured() {
        return;
    }
    if open || slash {
        *pending = Some(slash && !open);
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;

    fn runtime() -> Result<UiRuntime, Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(super::super::api::HudData::default()));
        super::super::api::install(&mut rt, &bridge)?;
        Ok(rt)
    }

    #[test]
    fn player_channels_are_remembered_by_name_until_left() -> Result<(), Box<dyn std::error::Error>> {
        let rt = runtime()?;
        join(&rt, None, 15, 0, 7, "Room");
        let identity = aa_net::channel_identity(15, 0, 7) as f64;
        rt.lua.load(format!("assert(HUD.user_channels.room == {identity})")).exec()?;
        // an ordinary channel is not one of the player's
        join(&rt, None, 4, 0, 0, "");
        rt.lua.load("local n = 0; for _ in pairs(HUD.user_channels) do n = n + 1 end; assert(n == 1)").exec()?;
        leave(&rt, None, 15, 0, 7);
        rt.lua.load("assert(HUD.user_channels.room == nil)").exec()?;
        Ok(())
    }

    #[test]
    fn the_dice_make_the_chat_line_of_the_scripts() -> Result<(), Box<dyn std::error::Error>> {
        let rt = runtime()?;
        rt.lua.load("lines = {}; UIParent:SetEventHandler('CHAT_DICE_VALUE', function(text) lines[#lines + 1] = text end)").exec()?;
        dice(&rt, &aa_net::DiceRolled { name: "Bob".into(), max: 100, value: 42 });
        rt.lua.load("assert(#lines == 1 and string.find(lines[1], 'Bob', 1, true) and string.find(lines[1], '42', 1, true), tostring(lines[1]))").exec()?;
        Ok(())
    }

    #[test]
    fn an_incoming_whisper_is_what_the_reply_command_answers() -> Result<(), Box<dyn std::error::Error>> {
        let rt = runtime()?;
        let friends = aa_net::Friends::default();
        let whisper = aa_net::model::ChatMessage {
            chat_type: aa_net::model::chat_type::WHISPERED,
            sub_type: 0,
            channel_faction: 0,
            sender_obj_id: 5,
            sender_id: 5,
            sender_race: 1,
            faction_id: 1,
            sender: "Carl".into(),
            message: "psst".into(),
            ability: 0,
        };
        message(&rt, &whisper, &friends, false);
        rt.lua.load("assert(HUD.last_whisper == 'Carl')").exec()?;
        Ok(())
    }
}
