//! The cast bar: the blue bar over the action bar that fills while the player casts a skill ("Recall").
//!
//! The bar is the original script's (`unitframe/player_casting_bar.lua` with `components/castingbar.lua`;
//! the target frame has one too): it listens for `SPELLCAST_START(spellName, castingTime, caster)`,
//! `SPELLCAST_STOP(caster)` and `SPELLCAST_SUCCEEDED(caster)` (`caster` is the unit token the bar was made
//! for, `"player"` or `"target"`) and every frame asks `X2Unit:UnitCastingInfo(unit)` for
//! `(spellName, castingTime, currentTime, show)` in milliseconds. A cast fills the bar; a channel, once the
//! cast is done, empties it (the script's "down" direction).
//!
//! [`route_casts`] turns aa-net's skill events into those events and keeps the casts of the units the bars
//! are about (`HudData::casts`); `HUD.casting_info(unit)` reads them for `ingame.lua`.

use std::collections::HashMap;
use std::time::Instant;

use aa_net::Event;
use aa_net::state::{Player, Target};
use bevy::prelude::*;
use mlua::Lua;

use super::api::HudBridge;

/// A cast or channel in progress.
#[derive(Clone, Debug)]
pub struct Cast {
    pub name: String,
    pub tl: u16,
    pub cast_ms: u32,
    pub started: Instant,
    /// When the cast was done and the channel began.
    pub fired: Option<Instant>,
    pub channel_ms: u32,
}

impl Cast {
    /// `(castingTime, currentTime)` in ms: the bar fills while casting and empties while channeling.
    fn times(&self) -> (u32, u32) {
        match self.fired {
            Some(at) if self.channel_ms > 0 => (self.channel_ms, self.channel_ms.saturating_sub(at.elapsed().as_millis() as u32)),
            Some(_) => (self.cast_ms, self.cast_ms),
            None => (self.cast_ms, (self.started.elapsed().as_millis() as u32).min(self.cast_ms)),
        }
    }
}

/// The casts the bars ask about, by caster object id, and who "player" and "target" are.
#[derive(Debug, Default)]
pub struct Casts {
    pub casts: HashMap<u32, Cast>,
    pub own: u32,
    pub target: u32,
}

impl Casts {
    /// `X2Unit:UnitCastingInfo`: `(spellName, castingTime, currentTime)` of a unit token.
    fn info(&self, unit: &str) -> Option<(String, u32, u32)> {
        let id = match unit {
            "player" => self.own,
            "target" => self.target,
            _ => return None,
        };
        let cast = self.casts.get(&id).filter(|_| id != 0)?;
        let (total, current) = cast.times();
        (total > 0).then(|| (cast.name.clone(), total, current))
    }

    /// The token the bars know a caster by.
    fn token(&self, obj_id: u32) -> Option<&'static str> {
        if obj_id == 0 {
            None
        } else if obj_id == self.own {
            Some("player")
        } else if obj_id == self.target {
            Some("target")
        } else {
            None
        }
    }
}

/// `HUD.casting_info(unit)`.
pub fn install(lua: &Lua, hud: &mlua::Table, bridge: &super::api::Shared) -> mlua::Result<()> {
    let b = bridge.clone();
    hud.raw_set(
        "casting_info",
        lua.create_function(move |_, unit: Option<String>| {
            Ok(match unit.and_then(|u| b.borrow().casts.info(&u)) {
                Some((name, total, current)) => (Some(name), Some(total), Some(current), Some(true)),
                None => (None, None, None, None),
            })
        })?,
    )
}

/// What a cast did to the scripts.
enum Fire {
    Start(String, u32, &'static str),
    Stop(&'static str),
    Succeeded(&'static str),
}

/// aa-net's skill events as the cast bar's events.
pub fn route_casts(mut events: MessageReader<aa_net::NetEvent>, rt: Option<NonSend<aa_ui::UiRuntime>>, bridge: NonSend<HudBridge>, player: Res<Player>, target: Res<Target>) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let mut fires: Vec<Fire> = Vec::new();
    {
        let mut data = bridge.0.borrow_mut();
        let shared = data.game.clone();
        let game = shared.get();
        let name_of = |id: u32| game.and_then(|g| g.skills.get(&id)).map_or_else(String::new, |s| s.named.name.clone());
        let now = Instant::now();
        let c = &mut data.casts;
        c.own = own;
        c.target = target.obj_id.unwrap_or(0);
        for aa_net::NetEvent(event) in events.read() {
            match event {
                Event::SkillStarted(s) if s.result == 0 && s.cast_ms > 0 => {
                    let cast = Cast { name: name_of(s.skill_id), tl: s.tl, cast_ms: s.cast_ms, started: now, fired: None, channel_ms: 0 };
                    if let Some(token) = c.token(s.caster.obj_id) {
                        fires.push(Fire::Start(cast.name.clone(), cast.cast_ms, token));
                    }
                    c.casts.insert(s.caster.obj_id, cast);
                }
                Event::PlotEvent(p) => {
                    let known = c.casts.get(&p.caster_obj_id).is_some_and(|k| k.tl == p.tl);
                    if p.casting_ms > 0 {
                        let cast = Cast { name: name_of(p.skill_id), tl: p.tl, cast_ms: p.casting_ms, started: now, fired: None, channel_ms: 0 };
                        if let Some(token) = c.token(p.caster_obj_id) {
                            fires.push(Fire::Start(cast.name.clone(), cast.cast_ms, token));
                        }
                        c.casts.insert(p.caster_obj_id, cast);
                    } else if known {
                        // the next step of the plot: the cast of the one before is done
                        c.casts.remove(&p.caster_obj_id);
                        fires.extend(c.token(p.caster_obj_id).map(Fire::Succeeded));
                    }
                }
                Event::SkillFired(f) => {
                    let Some(cast) = c.casts.get_mut(&f.caster.obj_id).filter(|k| k.tl == f.tl) else { continue };
                    if f.channeling_ms > 0 {
                        cast.fired = Some(now);
                        cast.channel_ms = f.channeling_ms as u32;
                    } else {
                        c.casts.remove(&f.caster.obj_id);
                        fires.extend(c.token(f.caster.obj_id).map(Fire::Succeeded));
                    }
                }
                // the end of a channel is a success, the end of a cast that never fired a cancel
                Event::SkillEnded { tl } | Event::PlotEnded { tl } => {
                    let ids: Vec<u32> = c.casts.iter().filter(|(_, k)| k.tl == *tl).map(|(id, _)| *id).collect();
                    for id in ids {
                        let channeled = c.casts.remove(&id).is_some_and(|k| k.fired.is_some());
                        let token = c.token(id);
                        fires.extend(token.map(|t| if channeled || matches!(event, Event::PlotEnded { .. }) { Fire::Succeeded(t) } else { Fire::Stop(t) }));
                    }
                }
                Event::CastingStopped { tl, .. } | Event::PlotStopped { tl, .. } => {
                    let ids: Vec<u32> = c.casts.iter().filter(|(_, k)| k.tl == *tl).map(|(id, _)| *id).collect();
                    for id in ids {
                        c.casts.remove(&id);
                        fires.extend(c.token(id).map(Fire::Stop));
                    }
                }
                Event::SkillStopped { obj_id, .. } | Event::UnitDeath { obj_id } => {
                    if c.casts.remove(obj_id).is_some() {
                        fires.extend(c.token(*obj_id).map(Fire::Stop));
                    }
                }
                _ => {}
            }
        }
        // casts of units that left (or whose end packet was lost) do not pile up
        c.casts.retain(|_, k| k.started.elapsed().as_secs() < 600);
    }
    // (the scripts call back into `HUD.casting_info`: no borrow of the bridge while they run)
    for fire in fires {
        match fire {
            Fire::Start(name, ms, token) => rt.fire_event("SPELLCAST_START", (name, i64::from(ms), token)),
            Fire::Stop(token) => rt.fire_event("SPELLCAST_STOP", token),
            Fire::Succeeded(token) => rt.fire_event("SPELLCAST_SUCCEEDED", token),
        }
    }
}
