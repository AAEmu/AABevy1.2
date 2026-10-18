//! Original loot-dice prompts; inventory and winners remain authoritative network state.
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Instant;

use aa_net::loot_roll::{LootRollEvent, LootRolls};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, ObjectLike, Table};

#[allow(dead_code)]
pub(super) mod fixture;

#[derive(Clone, Copy)]
enum Request {
    Next,
    Maximum(i32),
    Action { key: u32, roll: bool },
}

#[derive(Default)]
struct View {
    initialized: bool,
    requests: Vec<Request>,
    queued: HashSet<u32>,
}

#[derive(Clone, Default)]
struct State(Rc<RefCell<View>>);

pub fn install(lua: &Lua) -> mlua::Result<()> {
    let state = State::default();
    lua.set_app_data(state.clone());
    let loot: Table = lua.globals().get("X2Loot")?;
    let s = state.clone();
    loot.set(
        "RequestNextDiceItem",
        lua.create_function(move |_, _: Table| {
            s.0.borrow_mut().requests.push(Request::Next);
            Ok(())
        })?,
    )?;
    let s = state.clone();
    loot.set(
        "SetMaxConcurrentNotifyDiceItem",
        lua.create_function(move |_, (_, count): (Table, i32)| {
            s.0.borrow_mut().requests.push(Request::Maximum(count));
            Ok(())
        })?,
    )?;
    loot.set(
        "DoDiceAction",
        lua.create_function(move |_, (_, key, roll): (Table, u32, bool)| {
            state.0.borrow_mut().requests.push(Request::Action { key, roll });
            Ok(())
        })?,
    )?;
    Ok(())
}

fn advance(rt: &UiRuntime, rolls: &mut LootRolls, now: Instant) {
    let Some(prompt) = rolls.next_prompt_at(now) else { return };
    let Some(item) = rolls.round(prompt.key).and_then(|r| r.item.as_ref()) else {
        return;
    };
    let Some(link) = super::itemlinks::link(item) else {
        warn!("hud: loot dice unsupported item detail {:?} (template {})", item.detail, item.template_id);
        rt.fire_event("SYSMSG", "This loot item's detail format is not supported yet.");
        return;
    };
    let ui_now = rt.lua.globals().get::<Table>("X2Time").and_then(|t| t.call_method::<f64>("GetUiMsec", ()));
    match ui_now {
        Ok(ui_now) => rt.fire_event("TRY_LOOT_DICE", (prompt.ui_key, prompt.ui_timestamp_ms(ui_now, now), link)),
        Err(e) => warn!("hud: loot dice UI clock: {e}"),
    }
}

fn sync(rt: Option<NonSend<UiRuntime>>, mut rolls: ResMut<LootRolls>, mut events: MessageReader<NetEvent>, mut net: MessageWriter<NetCommand>) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        events.clear();
        return;
    };
    let (fresh, requests) = {
        let mut v = state.0.borrow_mut();
        let fresh = !v.initialized;
        v.initialized = true;
        v.queued.retain(|key| rolls.prompts.iter().any(|p| p.ui_key == *key));
        (fresh, std::mem::take(&mut v.requests))
    };
    let now = Instant::now();
    let mut advances = if fresh { rolls.max_concurrent } else { 0 };
    for request in requests {
        match request {
            Request::Next => advances += 1,
            Request::Maximum(count) => rolls.set_max_concurrent(count),
            Request::Action { key, roll } => {
                if state.0.borrow().queued.contains(&key) {
                    continue;
                }
                match rolls.plan_action(key, roll) {
                    Ok(command) => {
                        state.0.borrow_mut().queued.insert(key);
                        net.write(NetCommand(Command::LootRoll(command)));
                    }
                    Err(message) => debug!("hud: ignored loot dice action: {message}"),
                }
            }
        }
    }
    for NetEvent(event) in events.read() {
        match event {
            Event::LootRoll(LootRollEvent::Offered(_)) => advances += 1,
            Event::LootRoll(LootRollEvent::Notified { character_name, item, dice }) => {
                if let Some(link) = super::itemlinks::link(item) {
                    rt.fire_event("LOOT_DICE", (character_name.as_str(), link, *dice));
                }
            }
            Event::LootRoll(LootRollEvent::RequestRejected { message, .. }) => rt.fire_event("SYSMSG", message.as_str()),
            _ => {}
        }
    }
    // Each native request exposes at most one queued item, within the native concurrent limit.
    for _ in 0..advances.min(rolls.max_concurrent) {
        advance(&rt, &mut rolls, now);
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(aa_ui::UiFrame)
            .run_if(in_state(crate::Screen::InGame))
            .run_if(super::hud_up),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::loot_roll::{LootRollCommand, LootRollKey};
    use aa_net::model::Item;
    use bevy::ecs::message::Messages;
    use std::time::Duration;

    fn controller() -> App {
        let rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        rt.lua.load("X2Loot={}; X2Time={}").exec().unwrap();
        install(&rt.lua).unwrap();
        rt.lua.load("X2Time.GetUiMsec=function() return 50000 end; shown={}; UIParent:SetEventHandler('TRY_LOOT_DICE',function(key,time,link) shown[#shown+1]={key=key,time=time,link=link} end)").exec().unwrap();
        let mut rolls = LootRolls::default();
        for index in 1..=4 {
            let key = LootRollKey {
                owner_type: 1,
                owner_obj_id: 123,
                item_index: index,
            };
            rolls.apply_at(
                &Event::LootRoll(LootRollEvent::Offered(Item {
                    id: key.item_id(),
                    template_id: 23388,
                    count: 4,
                    ..default()
                })),
                Instant::now() - Duration::from_secs(10),
            );
        }
        let mut app = App::new();
        app.insert_non_send(rt)
            .insert_resource(rolls)
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(Update, sync);
        app
    }

    fn drain(app: &mut App) -> Vec<LootRollCommand> {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|NetCommand(c)| match c {
                Command::LootRoll(c) => c,
                other => panic!("unexpected {other:?}"),
            })
            .collect()
    }

    #[test]
    fn opaque_ui_keys_map_to_exact_corpse_keys_without_optimistic_results() -> mlua::Result<()> {
        let mut app = controller();
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(#shown==3); assert(shown[1].key==0 and math.abs(shown[1].time-40000)<100); X2Loot:DoDiceAction(0,true); X2Loot:DoDiceAction(0,false); X2Loot:DoDiceAction(999,false)").exec()?;
        app.update();
        let commands = drain(&mut app);
        assert_eq!(
            commands,
            [LootRollCommand {
                key: LootRollKey {
                    owner_type: 1,
                    owner_obj_id: 123,
                    item_index: 1
                },
                roll: true
            }]
        );
        assert_eq!(app.world().resource::<LootRolls>().prompts.len(), 4);
        assert!(!app.world().resource::<LootRolls>().rounds[0].pending);
        app.world_mut()
            .resource_mut::<LootRolls>()
            .apply(&Event::LootRoll(LootRollEvent::Requested(commands[0])));
        app.world().non_send::<UiRuntime>().lua.load("X2Loot:RequestNextDiceItem()").exec()?;
        app.update();
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(#shown==4 and shown[4].key==3); X2Loot:DoDiceAction(1,false)")
            .exec()?;
        app.update();
        assert_eq!(
            drain(&mut app),
            [LootRollCommand {
                key: LootRollKey {
                    owner_type: 1,
                    owner_obj_id: 123,
                    item_index: 2
                },
                roll: false
            }]
        );
        Ok(())
    }

    #[test]
    fn native_concurrency_setter_does_not_request_or_send_a_roll() -> mlua::Result<()> {
        let mut app = controller();
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("X2Loot:SetMaxConcurrentNotifyDiceItem(0)")
            .exec()?;
        app.update();
        assert_eq!(app.world().resource::<LootRolls>().max_concurrent, 1);
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(#shown==1); X2Loot:SetMaxConcurrentNotifyDiceItem(3)")
            .exec()?;
        app.update();
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(#shown==1); X2Loot:RequestNextDiceItem()")
            .exec()?;
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(#shown==2)").exec()?;
        assert!(drain(&mut app).is_empty());
        Ok(())
    }

    fn bind_window(rt: &UiRuntime, key: u32, name: &str) -> mlua::Result<()> {
        let suffix = format!("LootDiceWindow({key})");
        let id = {
            let u = aa_ui::lua::ui(&rt.lua);
            u.tree
                .objects
                .iter()
                .enumerate()
                .find(|(_, o)| o.alive && o.widget().is_some() && o.name.ends_with(&suffix))
                .map(|(id, _)| id as aa_ui::tree::Id)
        };
        let id = id.unwrap_or_else(|| {
            let names = aa_ui::lua::ui(&rt.lua)
                .tree
                .objects
                .iter()
                .filter(|o| o.name.contains("LootDice"))
                .map(|o| (o.name.clone(), o.alive, o.shown))
                .collect::<Vec<_>>();
            panic!("original loot window {key}; candidates {names:?}")
        });
        rt.lua.globals().set(name, aa_ui::lua::object_table(&rt.lua, id)?)
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_roll_pass_deadline_and_item_link_controls() -> Result<(), Box<dyn std::error::Error>> {
        let rt = fixture::runtime(1920.0, 1080.0)?;
        let mut rolls = LootRolls::default();
        for item in fixture::offers() {
            rolls.apply(&Event::LootRoll(LootRollEvent::Offered(item)));
        }
        let text = super::super::itemlinks::link(&fixture::offers()[0]).unwrap();
        rt.lua.globals().set("FIXTURE_ITEM_LINK", text)?;
        rt.lua.load("local i=X2Item:InfoFromLink(FIXTURE_ITEM_LINK); assert(i.itemType==24488 and i.itemGrade==4 and i.stack==1 and #i.name>0); local payload=FIXTURE_ITEM_LINK:match('^|i%d+,%d+,(.+);$'); assert(X2Item:InfoFromLink(payload).name==i.name); assert(X2Item:InfoFromLink('bad')==nil); assert(X2Quest:GetQuestLinkText(FIXTURE_QUEST)=='|q'..FIXTURE_QUEST..';'); local l=AAUI.ChatLink('item',24488,4); assert(l.name==i.name and #l.color==4); local q=AAUI.ChatLink('quest',FIXTURE_QUEST); assert(#q.name>0 and #q.color==4)").exec()?;
        let mut app = App::new();
        app.insert_non_send(rt)
            .insert_resource(rolls)
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(Update, sync);
        app.update();
        {
            let rt = app.world().non_send::<UiRuntime>();
            bind_window(rt, 0, "ROLL_WINDOW")?;
            bind_window(rt, 1, "PASS_WINDOW")?;
            bind_window(rt, 2, "DEADLINE_WINDOW")?;
            rt.lua.load("assert(ROLL_WINDOW:IsVisible() and PASS_WINDOW:IsVisible() and DEADLINE_WINDOW:IsVisible()); ROLL_WINDOW.rollBtn:OnClick('LeftButton'); PASS_WINDOW.giveUpBtn:OnClick('LeftButton'); local old=X2Time.GetUiMsec; X2Time.GetUiMsec=function() return old(X2Time)+60001 end; DEADLINE_WINDOW:OnUpdate(); X2Time.GetUiMsec=old").exec()?;
        }
        app.update();
        let commands = drain(&mut app);
        assert_eq!(commands.len(), 3);
        assert_eq!(
            commands.iter().map(|c| (c.key.item_index, c.roll)).collect::<Vec<_>>(),
            [(1, true), (2, false), (3, false)]
        );
        assert_eq!(app.world().resource::<LootRolls>().prompts.len(), 4);
        for command in commands {
            app.world_mut()
                .resource_mut::<LootRolls>()
                .apply(&Event::LootRoll(LootRollEvent::Requested(command)));
        }
        {
            let rt = app.world().non_send::<UiRuntime>();
            // The original fade releases its area; its delayed request then exposes item four.
            for _ in 0..6 {
                aa_ui::lua::tick(&rt.lua, 100.0);
                aa_ui::lua::flush(&rt.lua);
            }
        }
        app.update();
        {
            let rt = app.world().non_send::<UiRuntime>();
            bind_window(rt, 3, "NEXT_WINDOW")?;
            rt.lua.load("assert(NEXT_WINDOW:IsVisible()); NEXT_WINDOW.titleBar.closeButton:OnClick('LeftButton')").exec()?;
            assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        }
        app.update();
        assert_eq!(drain(&mut app).iter().map(|c| (c.key.item_index, c.roll)).collect::<Vec<_>>(), [(4, false)]);
        Ok(())
    }
}
