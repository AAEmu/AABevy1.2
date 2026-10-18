//! Native local breath controller. DIVE events and raw corrections have separate meanings.
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

use super::api::{HudBridge, HudData, Shared};

#[allow(dead_code)]
pub(super) mod fixture;

#[derive(Default, Debug)]
pub(super) struct View {
    actor: Option<u32>,
    started: u32,
    capacity: u32,
}

impl View {
    fn apply(&mut self, event: &Event, now: u32, capacity: u32) -> Option<bool> {
        match event {
            Event::PlayerSpawn(spawn) => self.actor = Some(spawn.obj_id),
            Event::UnitsRemoved(ids) if self.actor.is_some_and(|id| ids.contains(&id)) => self.actor = None,
            Event::UnderWater { start } => {
                // Native3930b600 requires an actor even for END; the caller emits each report.
                if self.actor.is_some() {
                    if *start {
                        self.started = now;
                        self.capacity = capacity;
                    } else {
                        self.started = 0;
                    }
                }
                return Some(*start);
            }
            Event::SetBreath { breath } => {
                // Native3930b050 also writes while inactive/without an actor. No DIVE event.
                self.started = now.wrapping_sub(self.capacity).wrapping_add(*breath);
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => *self = Self::default(),
            _ => {}
        }
        None
    }

    fn time(&mut self, now: u32, capacity: u32) -> (u32, u32) {
        if self.started == 0 || self.actor.is_none() {
            return (0, 0);
        }
        if capacity != self.capacity {
            // Native uses x87 registers and truncates a signed64 result to its low32 bits.
            // f64 approximates that normal finite calculation. Native old-capacity-zero
            // floating-point behavior is unresolved; explicitly restart that app-only edge.
            let elapsed = if self.capacity == 0 {
                0
            } else {
                (f64::from(now.wrapping_sub(self.started)) / f64::from(self.capacity) * f64::from(capacity)).trunc() as u64 as u32
            };
            self.started = now.wrapping_sub(elapsed);
            self.capacity = capacity;
        }
        (self.capacity, self.capacity.wrapping_sub(now).wrapping_add(self.started))
    }
}

fn now(data: &HudData) -> u32 {
    // The native clock's epoch is unknown. Use a local ms origin, retaining u32 modular
    // differences and reserving zero at startup for the controller's inactive sentinel.
    (data.start.elapsed().as_millis() as u32).wrapping_add(1)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = bridge.clone();
    let player: Table = lua.globals().get("X2Player")?;
    player.set(
        "GetBreathTime",
        lua.create_function(move |_, _: Value| {
            let mut data = state.borrow_mut();
            let now = now(&data);
            let capacity = data.tips.lung_capacity_ms();
            let (total, left) = data.breath.time(now, capacity);
            // Native3905aca0 converts unsigned32 to ScriptAnyValue float before Lua number.
            Ok((f64::from(total as f32), f64::from(left as f32)))
        })?,
    )
}

fn dispatch(rt: Option<&UiRuntime>, bridge: &Shared, event: &Event) {
    let dive = {
        let mut data = bridge.borrow_mut();
        let now = now(&data);
        let capacity = data.tips.lung_capacity_ms();
        data.breath.apply(event, now, capacity)
    };
    if let (Some(rt), Some(start)) = (rt, dive) {
        rt.fire_event(if start { "DIVE_START" } else { "DIVE_END" }, ());
    }
}

fn track(mut events: MessageReader<NetEvent>, bridge: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>) {
    // Keep the timer during Loading too: LEFT_LOADING and initial script queries need it.
    let rt = rt.as_deref().filter(|_| built.up);
    for NetEvent(event) in events.read() {
        dispatch(rt, &bridge.0, event);
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track
            .before(super::prebuild_hud)
            .before(super::build_hud)
            .before(super::sync_hud)
            .before(aa_ui::UiFrame),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_breath_keeps_native_event_order_corrections_and_capacity_fraction() {
        let mut view = View::default();
        assert_eq!(view.apply(&Event::UnderWater { start: true }, 100, 60_000), Some(true));
        assert_eq!(view.time(200, 60_000), (0, 0));
        view.actor = Some(100);
        view.apply(&Event::UnderWater { start: true }, 1000, 60_000);
        assert_eq!(view.time(16_000, 60_000), (60_000, 45_000));
        assert_eq!(view.time(16_000, 120_000), (120_000, 90_000));
        assert_eq!(view.apply(&Event::SetBreath { breath: 12_345 }, 16_000, 120_000), None);
        assert_eq!(view.time(16_100, 120_000), (120_000, 12_245));
        view.apply(&Event::UnderWater { start: true }, 16_100, 120_000);
        assert_eq!(view.time(16_100, 120_000), (120_000, 120_000));
        view.apply(&Event::UnderWater { start: false }, 16_100, 120_000);
        assert_eq!(view.time(16_101, 120_000), (0, 0));
        view.apply(&Event::SetBreath { breath: 9 }, 16_101, 120_000);
        assert_eq!(
            view.time(16_101, 120_000),
            (120_000, 9),
            "raw correction can reactivate the native timer without a DIVE event"
        );
        assert_eq!(
            view.time(16_111, 120_000),
            (120_000, u32::MAX),
            "native remaining arithmetic wraps rather than clamping"
        );
        view.apply(&Event::UnitsRemoved(vec![100]), 16_111, 120_000);
        let saved = (view.started, view.capacity);
        view.apply(&Event::UnderWater { start: false }, 16_111, 120_000);
        assert_eq!((view.started, view.capacity), saved, "no actor leaves the timer untouched");
        assert_eq!(view.time(16_111, 1), (0, 0));
        view.actor = Some(200);
        view.apply(&Event::UnderWater { start: true }, u32::MAX - 500, 60_000);
        assert_eq!(view.time(499, 60_000), (60_000, 59_000));
        view.apply(&Event::LeftWorld { target: 1 }, 500, 60_000);
        assert_eq!(view.time(500, 60_000), (0, 0));
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_breath_bar_tracks_received_dive_events_and_loading() -> Result<(), Box<dyn std::error::Error>> {
        let (rt, bridge, path) = fixture::runtime(1920.0, 1080.0)?;
        rt.lua
            .load(
                r#"
            assert(not playerBreathBar:IsVisible())
            local total,left=X2Player:GetBreathTime(); assert(total==0 and left==0)
            dive_events={}
            UIParent:SetEventHandler('DIVE_START',function(...)
              assert(select('#',...)==0)
              local total,left=X2Player:GetBreathTime()
              dive_events[#dive_events+1]={true,total,left}
            end)
            UIParent:SetEventHandler('DIVE_END',function(...)
              assert(select('#',...)==0)
              local total,left=X2Player:GetBreathTime()
              dive_events[#dive_events+1]={false,total,left}
            end)
        "#,
            )
            .exec()?;
        dispatch(Some(&rt), &bridge, &Event::UnderWater { start: true });
        dispatch(Some(&rt), &bridge, &Event::UnderWater { start: true });
        dispatch(Some(&rt), &bridge, &Event::SetBreath { breath: 30_000 });
        rt.lua
            .load(
                r#"
            assert(playerBreathBar:IsVisible() and #dive_events==2)
            assert(dive_events[1][1] and dive_events[1][2]==60000)
            playerBreathBar:OnUpdate()
            local total,left=X2Player:GetBreathTime()
            assert(total==60000 and left>29000 and left<=30000)
            local min,max=playerBreathBar.statusBar:GetMinMaxValues()
            assert(min==0 and max==60000)
            assert(playerBreathBar.statusBar:GetValue()>29000)
        "#,
            )
            .exec()?;
        rt.fire_event("LEFT_LOADING", ());
        rt.lua.load("assert(playerBreathBar:IsVisible())").exec()?;
        dispatch(Some(&rt), &bridge, &Event::UnderWater { start: false });
        dispatch(Some(&rt), &bridge, &Event::UnderWater { start: false });
        // Original HideAll() uses its authored 200 ms fade; visibility changes on completion.
        aa_ui::lua::tick(&rt.lua, 250.0);
        rt.lua
            .load("assert(not playerBreathBar:IsVisible() and #dive_events==4); assert(dive_events[3][2]==0 and dive_events[3][3]==0)")
            .exec()?;
        dispatch(Some(&rt), &bridge, &Event::SetBreath { breath: 16_777_217 });
        rt.lua.load("assert(not playerBreathBar:IsVisible() and #dive_events==4)").exec()?;
        rt.fire_event("LEFT_LOADING", ());
        rt.lua.load("assert(playerBreathBar:IsVisible())").exec()?;
        dispatch(Some(&rt), &bridge, &Event::LeftWorld { target: 1 });
        rt.fire_event("LEFT_LOADING", ());
        aa_ui::lua::tick(&rt.lua, 250.0);
        rt.lua.load("assert(not playerBreathBar:IsVisible())").exec()?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
