//! Chat: incoming lines and channel changes from aa-net become the UI events the chat scripts
//! listen for (`CHAT_MESSAGE`, `CHAT_JOINED_CHANNEL`, `CHAT_LEAVED_CHANNEL`), and `Enter` opens the
//! chat window's input (the scripts' own edit box, `chatwindow:GetChatEdit()`), whose text goes out
//! as `Command::Chat` (see `HUD.chat_send` in `ingame.lua`).
//!
//! The channel numbers aa-net reports are the client's own (`CHAT_SAY` 0, `CHAT_ZONE` 1 (shout),
//! `CHAT_TRADE` 2, `CHAT_FIND_PARTY` 3, `CHAT_PARTY` 4, `CHAT_RAID` 5, `CHAT_FACTION` 6,
//! `CHAT_EXPEDITION` 7, `CHAT_RACE` 14, whispers -3 / -4, notice -1, system -2), so they pass
//! through unchanged.

use aa_net::state::Chat;
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

fn join(rt: &UiRuntime, game: Option<&GameData>, chat_type: i16, sub_type: i16, faction_id: u32, name: &str) {
    rt.fire_event("CHAT_JOINED_CHANNEL", (chat_type as i64, channel_name(game, chat_type, sub_type, faction_id, name)));
}

fn message(rt: &UiRuntime, m: &aa_net::model::ChatMessage, friends: &aa_net::Friends) {
    if (m.sender_obj_id != 0 || m.sender_id != 0) && friends.is_blocked(&m.sender) { return; }
    rt.fire_event(
        "CHAT_MESSAGE",
        (m.chat_type as i64, m.sender_obj_id.to_string(), false, m.sender.clone(), m.message.clone(), false, None::<String>),
    );
}

/// The channels joined and the lines received before the HUD existed, in order.
pub fn replay(rt: &UiRuntime, chat: &Chat, game: Option<&GameData>, friends: &aa_net::Friends) {
    for c in &chat.channels {
        join(rt, game, c.chat_type, c.sub_type, c.faction_id, &c.name);
    }
    for m in &chat.log {
        message(rt, m, friends);
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
            Event::Chat(m) => message(&rt, m, &data.friends),
            Event::ChatChannelJoined { chat_type, sub_type, faction_id, name } => join(&rt, game, *chat_type, *sub_type, *faction_id, name),
            Event::ChatChannelLeft { chat_type, sub_type, faction_id } => {
                rt.fire_event("CHAT_LEAVED_CHANNEL", (*chat_type as i64, channel_name(game, *chat_type, *sub_type, *faction_id, "")));
            }
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
    pub fn take_tab(&mut self) -> bool {
        std::mem::take(&mut self.tab)
    }

    pub fn take_escape(&mut self) -> bool {
        std::mem::take(&mut self.escape)
    }

    pub fn take_slot(&mut self) -> Option<usize> {
        self.slot.take()
    }

    pub fn take_loot(&mut self) -> Option<bool> {
        self.loot.take()
    }

    pub fn from_args() -> ScriptedKeys {
        let args: Vec<String> = std::env::args().collect();
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
    keys: Res<ButtonInput<KeyCode>>,
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
    let mut open = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
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
