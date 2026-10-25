//! Native Bloodlust getter, acknowledged icon events and physical toggle action.
use std::collections::HashMap;

use aa_net::model::Unit;
use aa_net::state::Units;
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

use super::api::{HudBridge, HudData, Shared};

#[allow(dead_code)]
pub(super) mod fixture;

#[derive(Clone, Copy)]
struct Flag {
    character: bool,
    on: bool,
}

/// Keep event-order state after initial hydration: a coalesced true/false pair must let each
/// original Lua handler query its own transition, rather than the frame's final resource value.
#[derive(Default)]
pub(super) struct View {
    owner: Option<(u32, u32)>,
    active: bool,
    units: HashMap<u32, Flag>,
}

impl View {
    fn announce(&mut self, id: u32, character: bool, on: bool) {
        self.units.entry(id).or_insert(Flag { character, on });
    }

    fn unit(&mut self, unit: &Unit) {
        self.announce(unit.obj_id, unit.is_character(), unit.flags & 0x0008 != 0);
    }

    fn get(&self, id: u32) -> bool {
        self.units.get(&id).is_some_and(|f| f.character && f.on)
    }

    fn apply(&mut self, event: &Event) -> Option<(u32, bool)> {
        match event {
            Event::UnitSpawned(unit) => self.unit(unit),
            Event::PlayerSpawn(spawn) => self.unit(&spawn.unit),
            Event::UnitsRemoved(ids) => {
                for id in ids {
                    self.units.remove(id);
                }
            }
            Event::ForceAttackSet { obj_id, on } => {
                let flag = self.units.get_mut(obj_id)?;
                flag.on = *on;
                // Native391d3c60 emits even a duplicate explicit report. Its getter separately
                // rejects non-characters; no event is invented from announcement or a send.
                return Some((*obj_id, *on));
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => *self = Self::default(),
            _ => {}
        }
        None
    }
}

/// During loading, hydrate from the latest received resources before the original ENTERED_WORLD
/// handlers run. Once active, replay subsequent events in order instead of reloading final flags.
pub(super) fn refresh(world: &World, data: &mut HudData, spawn: &aa_net::session::PlayerSpawn) {
    let owner = (spawn.character().id, spawn.obj_id);
    if data.pvp.active && data.pvp.owner == Some(owner) {
        return;
    }
    let mut view = View {
        owner: Some(owner),
        ..default()
    };
    if let Some(units) = world.get_resource::<Units>() {
        for unit in units.iter() {
            view.unit(unit);
        }
    }
    view.unit(&spawn.unit);
    if let Some(status) = world.get_resource::<aa_net::CombatStatus>() {
        for (id, flag) in &mut view.units {
            if let Some(on) = status.get(*id).and_then(|f| f.force_attack) {
                flag.on = on;
            }
        }
    }
    data.pvp = view;
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = bridge.clone();
    let unit: Table = lua.globals().get("X2Unit")?;
    unit.set(
        "UnitIsForceAttack",
        lua.create_function(move |lua, (_, key): (Value, Value)| {
            let raw_id = super::unit_id::value(&key);
            let Some(key) = key.as_string().map(|s| s.to_string_lossy()) else {
                return Ok(false);
            };
            let id = raw_id.or_else(|| {
                let unit: Table = lua.globals().get("X2Unit").ok()?;
                let get: mlua::Function = unit.get("GetUnitId").ok()?;
                let id: Option<String> = get.call((unit, key)).ok()?;
                super::unit_id::string(&id?)
            });
            // Native394bd710: missing/unresolved or non-character returns boolean false.
            Ok(id.is_some_and(|id| state.borrow().pvp.get(id)))
        })?,
    )
}

fn dispatch(rt: &UiRuntime, bridge: &Shared, event: &Event) {
    let transition = bridge.borrow_mut().pvp.apply(event);
    if let Some((id, on)) = transition {
        rt.fire_event("FORCE_ATTACK_CHANGED", (super::unit_id::format(id), on));
    }
    if let Event::ForceAttackRejected { message, .. } = event {
        warn!("hud: force attack: {message}");
        rt.fire_event("SYSMSG", message.as_str());
    }
}

pub(super) fn route(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    actions: Res<crate::settings::Actions>,
    mut commands: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let first = {
        let mut data = bridge.0.borrow_mut();
        let first = !data.pvp.active;
        data.pvp.active = true;
        first
    };
    if first {
        // Initial icon queries already read the hydrated snapshot. Events retained while the
        // world stage loaded must not become a second set of historical system messages.
        events.clear();
    } else {
        for NetEvent(event) in events.read() {
            dispatch(&rt, &bridge.0, event);
        }
    }
    if !rt.keyboard_captured() {
        for (name, index, button) in actions.hotkeys() {
            if !button && index == 0 && name == "toggle_force_attack" {
                let data = bridge.0.borrow();
                let own = data.snapshot.unit_id.parse::<u32>().ok();
                if let Some(flag) = own.and_then(|id| data.pvp.units.get(&id)).filter(|f| f.character) {
                    // Native39390ec0 inverts the acknowledged unit byte; a previous send does
                    // not change that byte, so repeated unacknowledged toggles send the same value.
                    commands.write(NetCommand(Command::SetForceAttack { on: !flag.on }));
                }
            }
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        route
            .after(super::sync_hud)
            .after(super::HudInputReady)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up),
    )
    .add_systems(OnExit(crate::Screen::InGame), |bridge: NonSend<HudBridge>| {
        bridge.0.borrow_mut().pvp = default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_getter_distinguishes_characters_and_ack_events_replay_in_order() -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
        super::super::api::install(&mut rt, &bridge)?;
        {
            let mut data = bridge.borrow_mut();
            data.snapshot.unit_id = "100".into();
            data.pvp.announce(100, true, true);
            data.pvp.announce(200, false, true);
            data.pvp.announce(256, true, false);
        }
        rt.lua.load("HUD.state.unit_id='100'").exec()?;
        rt.lua
            .load(
                r#"
            assert(X2Unit:UnitIsForceAttack('player')==true)
            assert(X2Unit:UnitIsForceAttack('64')==true and X2Unit:UnitIsForceAttack('100')==false)
            assert(X2Unit:UnitIsForceAttack('c8')==false)
            assert(X2Unit:UnitIsForceAttack('unknown')==false)
            assert(X2Unit:UnitIsForceAttack(nil)==false)
            assert(X2Unit:GetCombatRelationshipStr('missing')==nil)
            HUD.target={unit_id='300',relation='unresolved'}
            assert(X2Unit:GetCombatRelationshipStr('target')==0)
            HUD.target=nil
            seen={}
            UIParent:SetEventHandler('FORCE_ATTACK_CHANGED',function(id,on)
              assert(type(id)=='string' and type(on)=='boolean')
              seen[#seen+1]={id,on,X2Unit:UnitIsForceAttack(id)}
            end)
        "#,
            )
            .exec()?;
        for on in [false, true, true, false] {
            dispatch(&rt, &bridge, &Event::ForceAttackSet { obj_id: 100, on });
        }
        dispatch(&rt, &bridge, &Event::ForceAttackRequested { on: true });
        dispatch(&rt, &bridge, &Event::ForceAttackSet { obj_id: 999, on: true });
        rt.lua
            .load("assert(#seen==4); for _,v in ipairs(seen) do assert(v[2]==v[3]) end; assert(not X2Unit:UnitIsForceAttack('player'))")
            .exec()?;
        bridge.borrow_mut().pvp.announce(100, true, true);
        assert!(!bridge.borrow().pvp.get(100), "duplicate announcements retain the later explicit report");
        dispatch(&rt, &bridge, &Event::ForceAttackSet { obj_id: 200, on: true });
        rt.lua.load("assert(#seen==5 and seen[5][1]=='c8' and seen[5][2]==true and seen[5][3]==false)").exec()?;
        dispatch(&rt, &bridge, &Event::UnitsRemoved(vec![100]));
        dispatch(&rt, &bridge, &Event::ForceAttackSet { obj_id: 100, on: true });
        rt.lua.load("assert(#seen==5 and not X2Unit:UnitIsForceAttack('player'))").exec()?;
        bridge.borrow_mut().pvp.announce(100, true, true);
        assert!(bridge.borrow().pvp.get(100), "a reused ID starts from the new announcement");
        dispatch(&rt, &bridge, &Event::LeftWorld { target: 1 });
        rt.lua.load("assert(not X2Unit:UnitIsForceAttack('64'))").exec()?;
        Ok(())
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_icons_environment_text_and_acknowledged_physical_toggle() -> Result<(), Box<dyn std::error::Error>> {
        use bevy::input::keyboard::{Key, KeyboardInput};
        use bevy::input::{ButtonState, InputPlugin};

        let (rt, bridge, path) = fixture::runtime(1920.0, 1080.0)?;
        rt.lua
            .load(
                r#"
            assert(PVP_PLAYER.pvpIcon:IsVisible())
            assert(PVP_PLAYER:IsVisible() and targetFrame:IsVisible())
            assert(not targetFrame.pvpIcon:IsVisible())
            force_events={}
            UIParent:SetEventHandler('FORCE_ATTACK_CHANGED',function(id,on)
              force_events[#force_events+1]={id,on,X2Unit:UnitIsForceAttack(id)}
            end)
        "#,
            )
            .exec()?;
        for (id, on) in [(200, true), (100, false), (100, true)] {
            dispatch(&rt, &bridge, &Event::ForceAttackSet { obj_id: id, on });
        }
        rt.lua.load("assert(PVP_PLAYER.pvpIcon:IsVisible() and targetFrame.pvpIcon:IsVisible()); assert(#force_events==3); for _,v in ipairs(force_events) do assert(v[2]==v[3]) end").exec()?;
        for source in [0, 2] {
            super::super::combat::environment(
                &rt,
                &aa_net::EnvironmentDamage {
                    source,
                    target_obj_id: 100,
                    amount: 17,
                    gimmick_id: None,
                },
                "Bloodlust fixture",
            );
        }
        rt.lua
            .load(r#"
                local expected={['-17('..envSourceToStr('FALLING')..')']=true,['-17('..envSourceToStr('DROWNING')..')']=true}
                local count=0
                for _,w in ipairs(combatTextFrame.combatTexts) do
                  if w:IsVisible() and expected[w:GetText()] then count=count+1; expected[w:GetText()]=nil end
                end
                assert(count==2,'expected two original environmental displays; found '..count)
            "#)
            .exec()?;
        assert_eq!(bridge.borrow().snapshot.hp, 100);
        let mut app = App::new();
        app.add_plugins(InputPlugin)
            .insert_non_send(rt)
            .insert_non_send(HudBridge(bridge.clone()))
            .init_resource::<crate::settings::Actions>()
            .init_resource::<super::super::HudPointerOverUi>()
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(Update, (crate::settings::update_input, route).chain());
        app.update();
        let edge = |app: &mut App, key_code, state| {
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key: Key::Character("x".into()),
                state,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        };
        let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
        edge(&mut app, KeyCode::ControlLeft, ButtonState::Pressed);
        edge(&mut app, KeyCode::KeyF, ButtonState::Pressed);
        app.update();
        assert!(matches!(drain(&mut app).as_slice(), [Command::SetForceAttack { on: false }]));
        assert!(bridge.borrow().pvp.get(100), "a send does not toggle the acknowledged flag");
        edge(&mut app, KeyCode::KeyF, ButtonState::Released);
        edge(&mut app, KeyCode::KeyF, ButtonState::Pressed);
        app.update();
        assert!(matches!(drain(&mut app).as_slice(), [Command::SetForceAttack { on: false }]));
        app.world_mut().write_message(NetEvent(Event::ForceAttackSet { obj_id: 100, on: false }));
        edge(&mut app, KeyCode::KeyF, ButtonState::Released);
        edge(&mut app, KeyCode::KeyF, ButtonState::Pressed);
        app.update();
        assert!(matches!(drain(&mut app).as_slice(), [Command::SetForceAttack { on: true }]));
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load("assert(not X2Unit:UnitIsForceAttack('player') and not PVP_PLAYER.pvpIcon:IsVisible())")
            .exec()?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
