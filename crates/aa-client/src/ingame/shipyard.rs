//! Shipyards the server announces (`SCUnitState` of kind shipyard): drawn from their prefab at the model of their
//! construction stage, and swapped when `SCShipyardState` (0x1aa) moves them to the next stage.
//!
//! The 1.2 client keeps a shipyard record per `ShipyardData.Id` (`FUN_394d8b00`) and swaps the unit's model to
//! `ShipyardSteps[step]` (`FUN_394d5140`). `FUN_394d8600` appends one entry after the steps, so `step == steps.len()`
//! is the finished hull (`main_model_id`); step 1000 is the launch ceremony and shows the template's
//! `ceremony_model_id` (with its `ceremony_anim_key`). Any other out-of-range step leaves the drawn model alone.
//! The rows come from the client's own `shipyards` / `shipyard_steps` (`ingame::data::ShipyardTemplate`).
//!
//! `units.rs` leaves shipyard units alone; this module keeps one entity per shipyard within [`SHIPYARD_RADIUS`] and
//! prepares the prefab parts on the workers like the units' models.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use aa_cry::scene::CryAssets;
use aa_cry::workers::Workers;
use aa_net::model::{Unit, UnitKind};
use aa_net::state::Units;
use aa_net::{ShipyardState, Shipyards};
use bevy::prelude::*;

use super::data::{Db, ShipyardTemplate};
use super::doodad_model::{self, Prepared};
use super::player::{LocalPlayer, placement};
use crate::Screen;

/// Shipyards are drawn within this many metres of the player.
pub const SHIPYARD_RADIUS: f32 = 400.0;

/// A drawn shipyard (the fields are read by the self-checking example and tests).
#[derive(Component)]
#[allow(dead_code)]
pub struct ShipyardModel {
    pub obj_id: u32,
    /// The `models.id` of the stage drawn now.
    pub model_id: u32,
    /// The `prefab://` element the model was drawn from.
    pub path: String,
}

struct Drawn {
    model_id: u32,
    entity: Option<Entity>,
    path: String,
    preparing: bool,
    failed: bool,
    /// The drawn model's bounds in the unit's local cry frame (metres), for the HUD's pick.
    bounds: Option<(Vec3, Vec3)>,
}

#[derive(Resource)]
pub struct DrawnShipyards {
    yards: HashMap<u32, Drawn>,
    tx: Sender<(u32, u32, Option<Prepared>)>,
    rx: Mutex<Receiver<(u32, u32, Option<Prepared>)>>,
}

impl Default for DrawnShipyards {
    fn default() -> Self {
        let (tx, rx) = channel();
        DrawnShipyards {
            yards: HashMap::new(),
            tx,
            rx: Mutex::new(rx),
        }
    }
}

impl DrawnShipyards {
    /// The entity drawing the shipyard unit `obj_id`, when one is drawn (the example and tests use it).
    #[allow(dead_code)]
    pub fn entity(&self, obj_id: u32) -> Option<Entity> {
        self.yards.get(&obj_id).and_then(|d| d.entity)
    }

    /// The bounds of the stage model drawn for `obj_id` in the unit's local cry frame.
    pub fn bounds(&self, obj_id: u32) -> Option<(Vec3, Vec3)> {
        self.yards.get(&obj_id).and_then(|d| d.bounds)
    }
}

/// The model the shipyard unit shows now: the stage the received `SCShipyardState` names, else the model of its
/// `SCUnitState`. `step == steps.len()` is the finished hull and 1000 the ceremony model (`FUN_394d5140`); any other
/// out-of-range step keeps `drawn` (1.2 changes nothing), not the unit's model.
pub fn stage_model(
    unit: &Unit,
    shipyards: &Shipyards,
    template: Option<&ShipyardTemplate>,
    drawn: u32,
) -> Option<u32> {
    let UnitKind::Shipyard { id, .. } = unit.kind else {
        return None;
    };
    if let Some(state) = shipyards.of(id) {
        if let Some(model) = state_step_model(state, template) {
            return Some(model);
        }
        if drawn != 0 {
            return Some(drawn);
        }
    }
    (unit.model_id != 0).then_some(unit.model_id)
}

fn state_step_model(state: &ShipyardState, template: Option<&ShipyardTemplate>) -> Option<u32> {
    template.and_then(|t| t.model_at_step(state.step))
}

/// `GetShipyardBaseProgress` / `GetShipyardRemainProgress` (`FUN_394d8600`): the template's total build actions,
/// and what is left of them after the packet's `actions`.
pub fn progress(template: &ShipyardTemplate, state: &ShipyardState) -> (i32, i32) {
    let base = template.all_actions();
    (base, base.saturating_sub(state.actions).max(0))
}

/// Keeps one entity per shipyard unit near the player, at the model of its construction stage.
pub fn sync_shipyards(
    mut commands: Commands,
    mut cry: CryAssets,
    mut drawn: ResMut<DrawnShipyards>,
    units: Res<Units>,
    shipyards: Res<Shipyards>,
    db: Option<Res<Db>>,
    player: Query<&Transform, With<LocalPlayer>>,
) {
    let Some(db) = db else { return };
    let Some(me) = player
        .single()
        .ok()
        .map(|t| aa_world::bevy_to_cry(t.translation))
    else {
        return;
    };
    let arrived: Vec<_> = {
        let rx = drawn.rx.lock().unwrap_or_else(|e| e.into_inner());
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    };
    for (obj_id, model_id, parts) in arrived {
        let Some(unit) = units.get(obj_id) else {
            continue;
        };
        let Some(d) = drawn.yards.get_mut(&obj_id) else {
            continue;
        };
        if d.model_id != model_id {
            continue; // the stage changed while the worker ran: the newer job comes
        }
        d.preparing = false;
        let Some(prepared) = parts else {
            d.failed = true;
            warn!(
                "shipyard: shipyard {obj_id} {:?} (model {model_id}) cannot be drawn",
                unit.name
            );
            continue;
        };
        d.bounds = prepared.bounds;
        let transform = placement(
            Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
            unit.yaw(),
            unit.scale.max(0.01),
        );
        let spawned = doodad_model::spawn(&mut commands, &mut cry, prepared);
        commands.entity(spawned.entity).insert((
            Name::new(format!("shipyard {obj_id} {}", unit.name)),
            ShipyardModel {
                obj_id,
                model_id,
                path: d.path.clone(),
            },
            transform,
            Visibility::default(),
        ));
        if let Some(old) = d.entity.replace(spawned.entity) {
            commands.entity(old).despawn();
        }
        info!(
            "shipyard: shipyard {obj_id} {:?} drawn with model {model_id} ({})",
            unit.name, d.path
        );
    }
    // gone, or too far
    let gone: Vec<u32> = drawn
        .yards
        .keys()
        .filter(|id| {
            units
                .get(**id)
                .is_none_or(|u| flat(u, me) > SHIPYARD_RADIUS * 1.2)
        })
        .copied()
        .collect();
    for id in gone {
        if let Some(Drawn {
            entity: Some(e), ..
        }) = drawn.yards.remove(&id)
        {
            commands.entity(e).despawn();
        }
    }
    let loader = cry.loader();
    for unit in units
        .iter()
        .filter(|u| matches!(u.kind, UnitKind::Shipyard { .. }))
    {
        if flat(unit, me) > SHIPYARD_RADIUS {
            continue;
        }
        let template = match unit.kind {
            UnitKind::Shipyard { template_id, .. } => db.shipyard_template(template_id),
            _ => None,
        };
        let shown = drawn.yards.get(&unit.obj_id).map_or(0, |d| d.model_id);
        let Some(model_id) = stage_model(unit, &shipyards, template.as_ref(), shown) else {
            continue;
        };
        let d = drawn.yards.entry(unit.obj_id).or_insert(Drawn {
            model_id: 0,
            entity: None,
            path: String::new(),
            preparing: false,
            failed: false,
            bounds: None,
        });
        if d.model_id == model_id && (d.entity.is_some() || d.preparing || d.failed) {
            continue;
        }
        let Some(path) = db.prefab_reference(model_id) else {
            d.model_id = model_id;
            d.failed = true;
            warn!(
                "shipyard: model {model_id} of shipyard {} is not a prefab",
                unit.obj_id
            );
            continue;
        };
        d.model_id = model_id;
        d.path = path.clone();
        d.preparing = true;
        d.failed = false;
        let (obj_id, tx, loader) = (unit.obj_id, drawn.tx.clone(), loader.clone());
        Workers::global().spawn(flat(unit, me), move || {
            let parts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                doodad_model::prepare_with_bounds(&loader, &path)
            }))
            .ok()
            .flatten();
            let _ = tx.send((obj_id, model_id, parts));
        });
    }
}

fn flat(unit: &Unit, me: Vec3) -> f32 {
    Vec2::new(unit.pos.x - me.x, unit.pos.y - me.y).length()
}

/// Leaving the game drops every drawn shipyard.
pub fn clear(mut commands: Commands, mut drawn: ResMut<DrawnShipyards>) {
    for (_, d) in drawn.yards.drain() {
        if let Some(e) = d.entity {
            commands.entity(e).despawn();
        }
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<DrawnShipyards>()
        .add_systems(Update, sync_shipyards.run_if(super::world_screens))
        .add_systems(OnExit(Screen::InGame), clear);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingame::seating::fixture_unit;
    use aa_net::{Event, ShipyardEvent};

    fn template() -> ShipyardTemplate {
        ShipyardTemplate {
            main_model_id: 680,
            ceremony_model_id: 683,
            ceremony_anim_key: Some("ceremony".into()),
            build_radius: 25,
            steps: vec![
                super::super::data::ShipyardStep {
                    step: 0,
                    model_id: 673,
                    num_actions: 2,
                },
                super::super::data::ShipyardStep {
                    step: 1,
                    model_id: 674,
                    num_actions: 3,
                },
            ],
        }
    }

    fn state(step: i32, actions: i32) -> ShipyardState {
        ShipyardState {
            id: 9,
            template_id: 5,
            obj_id: 77,
            step,
            actions,
            ..Default::default()
        }
    }

    fn unit() -> Unit {
        let mut unit = fixture_unit(77, 42, None);
        unit.kind = UnitKind::Shipyard {
            id: 9,
            template_id: 5,
        };
        unit.model_id = 673;
        unit
    }

    #[test]
    fn stage_models_follow_the_packet_and_fall_back_to_the_unit() {
        let unit = unit();
        let mut yards = Shipyards::default();
        let template = template();
        assert_eq!(stage_model(&unit, &yards, Some(&template), 0), Some(673));

        yards.apply(&Event::Shipyard(ShipyardEvent::State(state(1, 2))));
        assert_eq!(stage_model(&unit, &yards, Some(&template), 0), Some(674));

        // `sync_build_state` sends `steps.len()` once the last stage is done: the appended main model.
        yards.apply(&Event::Shipyard(ShipyardEvent::State(state(2, 5))));
        assert_eq!(stage_model(&unit, &yards, Some(&template), 674), Some(680));

        // any other out-of-range step keeps the drawn model, not the unit's (`FUN_394d5140` changes nothing).
        yards.apply(&Event::Shipyard(ShipyardEvent::State(state(9, 5))));
        assert_eq!(stage_model(&unit, &yards, Some(&template), 680), Some(680));
        assert_eq!(stage_model(&unit, &yards, Some(&template), 0), Some(673));

        // step 1000 is the launch ceremony, the ceremony model (not the finished hull).
        yards.apply(&Event::Shipyard(ShipyardEvent::State(state(1000, 5))));
        assert_eq!(stage_model(&unit, &yards, Some(&template), 680), Some(683));
        assert_eq!(stage_model(&unit, &yards, None, 680), Some(680));
    }

    #[test]
    fn progress_is_the_template_total_minus_actions() {
        let template = template();
        let mut state = state(1, 2);
        assert_eq!(progress(&template, &state), (5, 3));
        state.actions = 99;
        assert_eq!(progress(&template, &state), (5, 0));
    }
}
