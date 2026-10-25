//! Physical screen coordinates for the toolkit's native combat displays.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use aa_net::model::UnitKind;
use aa_net::state::Units;
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use mlua::{Lua, Table};

use super::super::api::HudBridge;
use super::super::plates::{PlateState, head_gap, unit_height};
use crate::Screen;

/// The existing head-anchor calibration; native world-space sizing remains unverified.
const ANCHOR_GAP: f32 = 0.18;

#[derive(Clone, Default)]
struct Positions(Rc<RefCell<ProjectionState>>);

#[derive(Default)]
struct ProjectionState {
    own: u32,
    requested: HashSet<u32>,
    points: HashMap<u32, Vec2>,
}

impl ProjectionState {
    fn target(&self, target: &str) -> Option<u32> {
        match target {
            "" | "player" => (self.own != 0).then_some(self.own),
            _ => super::super::unit_id::string(target),
        }
    }
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    let positions = Positions::default();
    lua.set_app_data(positions.clone());
    let aaui: Table = lua.globals().get("AAUI")?;
    aaui.set(
        "CombatTextPosition",
        lua.create_function(move |lua, (_source, target): (String, String)| {
            // Do not borrow HudBridge: this callback can run inside a game API event.
            let mut state = positions.0.borrow_mut();
            let Some(id) = state.target(&target) else {
                return Ok(None);
            };
            state.requested.insert(id);
            state
                .points
                .get(&id)
                .map(|point| {
                    let out = lua.create_table()?;
                    out.set("x", point.x)?;
                    out.set("y", point.y)?;
                    Ok(out)
                })
                .transpose()
        })?,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn update(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    plates: Res<PlateState>,
    camera: Query<(&Camera, &Transform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
    transforms: Query<&Transform>,
    windows: Query<&Window, With<PrimaryWindow>>,
) {
    let Some(rt) = rt else { return };
    let Some(positions) = rt.lua.app_data_ref::<Positions>() else {
        return;
    };
    let (own, own_entity, own_pos) = {
        let d = bridge.0.borrow();
        (
            d.snapshot.unit_id.parse::<u32>().unwrap_or(0),
            d.player_entity,
            d.pos,
        )
    };
    let mut state = positions.0.borrow_mut();
    state.own = own;
    state.points.clear();
    let mut requested = std::mem::take(&mut state.requested);
    if own != 0 {
        requested.insert(own);
    }
    let Some((camera, transform)) = camera.iter().find(|(c, _)| c.is_active) else {
        return;
    };
    let Ok(window) = windows.single() else { return };
    let global = GlobalTransform::from(*transform);
    let scale_factor = window.resolution.scale_factor();
    for id in requested {
        let unit = units.get(id);
        let entity = if id == own {
            own_entity
        } else {
            plates.entities.get(&id).copied()
        };
        let root = entity
            .and_then(|e| transforms.get(e).ok())
            .map(|t| t.translation)
            .or_else(|| {
                if id == own {
                    Some(aa_world::cry_to_bevy(own_pos))
                } else {
                    unit.map(|u| aa_world::cry_to_bevy(Vec3::new(u.pos.x, u.pos.y, u.pos.z)))
                }
            });
        let Some(root) = root else { continue };
        let character =
            id == own || unit.is_some_and(|u| matches!(u.kind, UnitKind::Character { .. }));
        let height = unit_height(
            plates.heights.get(&id).map(|h| h.0),
            unit.map_or(1.0, |u| u.scale),
            character,
        );
        let anchor = root + Vec3::Y * (height + head_gap(height, ANCHOR_GAP));
        if let Ok(point) = camera.world_to_viewport(&global, anchor) {
            let point = point * scale_factor;
            if point.is_finite()
                && point.cmpge(Vec2::ZERO).all()
                && point
                    .cmple(Vec2::new(
                        window.physical_width() as f32,
                        window.physical_height() as f32,
                    ))
                    .all()
            {
                state.points.insert(id, point);
            }
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::route_combat)
            .after(crate::ingame::camera::follow_player)
            .after(crate::ingame::units::move_units)
            .after(super::super::cameras::dev_fov)
            .before(aa_ui::UiFrame)
            .run_if(in_state(Screen::InGame))
            .run_if(super::super::hud_up),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_hook_keeps_identity_and_does_not_borrow_game_state() -> mlua::Result<()> {
        let rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
        install(&rt.lua)?;
        let positions = rt.lua.app_data_ref::<Positions>().unwrap().clone();
        {
            let mut state = positions.0.borrow_mut();
            state.own = 100;
            state.points.insert(100, Vec2::new(640.0, 360.0));
            state.points.insert(200, Vec2::new(900.0, 400.0));
        }
        rt.lua.load("local p=AAUI.CombatTextPosition('c8','player'); assert(p.x==640 and p.y==360); p=AAUI.CombatTextPosition('64','c8'); assert(p.x==900 and p.y==400); assert(AAUI.CombatTextPosition('64','12c')==nil); assert(AAUI.CombatTextPosition('64','unresolved')==nil)").exec()?;
        assert_eq!(
            positions.0.borrow().requested,
            HashSet::from([100, 200, 300])
        );
        positions.0.borrow_mut().points.clear();
        rt.lua
            .load("assert(AAUI.CombatTextPosition('64','c8')==nil)")
            .exec()
    }
}
