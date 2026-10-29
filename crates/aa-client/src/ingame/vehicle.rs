//! Vehicles and ships (`models.sub_type` `VehicleModel` / `ShipModel`: airships, carts, boats, siege weapons), drawn from
//! their `normal` look ([`super::data::Db::vehicle_look`]): a single model, or every part of a prefab (the hull and deck
//! brushes, and the `AnimObject` parts such as an airship's wings, propellers and planks). Wheels go on the body's wheel
//! mounts: `wheelN` bones or helper nodes take the `wheel` model, `wheel_type2N` ones `wheel2`.
//!
//! Left out: the damaged and dead looks, the prefabs' particle effects, wheel and propeller spin, turrets.

use std::collections::HashMap;

use aa_anim::ClipLibrary;
use aa_cry::scene::{CryAssets, ModelOptions, Shading, spawn_loaded};
use aa_cry::{CryLoader, PreparedModel};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::math::Affine3A;
use bevy::prelude::*;

use super::data::{VehicleLook, model_file};
use super::housing::prefab::{self, Part};

/// The wheel model slot of a mount name: `wheel0`, `wheel12` take `wheel` (0), `wheel_type2`, `wheel_type21` take
/// `wheel2` (1). Other names (`bone_steeringwheel`, `wheel_l`) are not mounts.
pub fn wheel_slot(name: &str) -> Option<usize> {
    let name = name.strip_prefix('$').unwrap_or(name).to_ascii_lowercase();
    if let Some(rest) = name.strip_prefix("wheel_type2") {
        return rest.bytes().all(|b| b.is_ascii_digit()).then_some(1);
    }
    let rest = name.strip_prefix("wheel")?;
    (!rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())).then_some(0)
}

/// The wheel mounts of a model (bones of a `.chr`, helper nodes of a `.cgf`), model space (cry).
fn mounts(model: &aa_cry::mesh::Model) -> Vec<(usize, Affine3A)> {
    let mut out: Vec<(usize, Affine3A)> = Vec::new();
    if let Some(skeleton) = &model.skeleton {
        out.extend(skeleton.bones.iter().filter_map(|b| Some((wheel_slot(&b.name)?, b.bind_model))));
    }
    if out.is_empty() {
        out.extend(model.nodes.iter().enumerate().filter_map(|(i, n)| Some((wheel_slot(&n.name)?, model.world_transform(i)))));
    }
    out
}

fn affine(part: &Part) -> Affine3A {
    Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos)
}

/// The `$` helper nodes of a vehicle body in unit (Cry) space: `driver`, `passenger0`, ... A vehicle root has no
/// rider Animator, so `mount::seat_world` reads the prefab helper from here.
#[derive(Component, Default)]
pub struct SeatHelpers(pub HashMap<String, Affine3A>);

fn helper_nodes(
    model: &aa_cry::mesh::Model,
    placement: Affine3A,
    out: &mut HashMap<String, Affine3A>,
) {
    if let Some(skeleton) = &model.skeleton {
        for bone in &skeleton.bones {
            if let Some(name) = bone.name.strip_prefix('$') {
                out.entry(name.to_ascii_lowercase())
                    .or_insert(placement * bone.bind_model);
            }
        }
    }
    for (index, node) in model.nodes.iter().enumerate() {
        if let Some(name) = node.name.strip_prefix('$') {
            out.entry(name.to_ascii_lowercase())
                .or_insert_with(|| placement * model.world_transform(index));
        }
    }
}

fn part_helpers(part: &Part, model: &aa_cry::mesh::Model) -> HashMap<String, Affine3A> {
    let mut helpers = HashMap::new();
    helper_nodes(model, affine(part), &mut helpers);
    helpers
}

/// The body helpers of `look` (reads the body models; used by the tests, [`prepare`] reuses the models it loads).
#[cfg(test)]
fn body_helpers(look: &VehicleLook) -> HashMap<String, Affine3A> {
    let mut helpers = HashMap::new();
    for part in body_parts(look) {
        if let Ok(model) = aa_cry::mesh::Model::load(&part.model) {
            helper_nodes(&model, affine(&part), &mut helpers);
        }
    }
    helpers
}

fn body_parts(look: &VehicleLook) -> Vec<Part> {
    if look.normal.to_ascii_lowercase().starts_with("prefab://") {
        prefab::load(&look.normal)
    } else {
        model_file(&look.normal)
            .map(|model| vec![Part { model, material: None, pos: Vec3::ZERO, rot: Quat::IDENTITY, scale: Vec3::ONE }])
            .unwrap_or_default()
    }
}

/// The parts `look` is drawn with, relative to the unit's origin (cry space): the body parts, then the wheels.
pub fn parts(look: &VehicleLook) -> Vec<Part> {
    let body = body_parts(look);
    let mut wheels = Vec::new();
    if look.wheels.iter().any(Option::is_some) {
        for part in &body {
            let Ok(model) = aa_cry::mesh::Model::load(&part.model) else { continue };
            for (slot, mount) in mounts(&model) {
                let Some(wheel) = look.wheels[slot].as_ref().or(look.wheels[0].as_ref()) else { continue };
                let (scale, rot, pos) = (affine(part) * mount).to_scale_rotation_translation();
                wheels.push(Part { model: wheel.clone(), material: None, pos, rot: rot.normalize(), scale });
            }
        }
    }
    let mut out = body;
    out.extend(wheels);
    out
}

/// A prepared part: a static model, or a rigged `.chr` (its models, spawned through [`super::creature::spawn`]).
pub struct Piece {
    part: Part,
    model: PieceModel,
    helpers: HashMap<String, Affine3A>,
}

enum PieceModel {
    Static(PreparedModel),
    Rigged(Vec<PreparedModel>),
}

/// Vehicle materials are native `Illum` (the farm wagon's copper boiler uses the gloss map), so they need the Full
/// shading path rather than the world-prop Basic one.
fn model_options(material: Option<String>, cry_space: bool) -> ModelOptions {
    ModelOptions {
        material: material.filter(|m| aa_pak::Pak::global().exists(m)),
        cry_space,
        shading: Shading::Full,
        ..default()
    }
}

fn static_options(part: &Part) -> ModelOptions {
    model_options(part.material.clone(), false)
}

/// Reads the models of `look` (callable from any thread). Parts whose model cannot be read are left out.
pub fn prepare(loader: &CryLoader, look: &VehicleLook) -> Vec<Piece> {
    parts(look)
        .into_iter()
        .filter_map(|part| {
            let rigged = part.model.ends_with(".chr")
                && aa_anim::pose::shared_rig(&part.model).is_ok_and(|r| !r.is_empty());
            let (model, helpers) = if rigged {
                let models: Vec<PreparedModel> = super::creature::model_files(&part.model, None)
                    .into_iter()
                    .filter_map(|(path, material)| {
                        loader
                            .prepare_model(&path, &model_options(material, true))
                            .ok()
                    })
                    .collect();
                // a `.chr` part is its own main model: read the helpers off the prepared model, not a second load
                let helpers = part_helpers(&part, &models.first()?.loaded().model);
                (PieceModel::Rigged(models), helpers)
            } else {
                match loader.prepare_model(&part.model, &static_options(&part)) {
                    Ok(m) => {
                        let helpers = part_helpers(&part, &m.loaded().model);
                        (PieceModel::Static(m), helpers)
                    }
                    Err(e) => {
                        debug!("vehicle part {}: {e}", part.model);
                        return None;
                    }
                }
            };
            Some(Piece {
                part,
                model,
                helpers,
            })
        })
        .collect()
}

/// The model files [`prepare`] reads (the browser downloads them first).
pub fn model_files(look: &VehicleLook) -> Vec<(String, Option<String>)> {
    parts(look).into_iter().map(|p| (p.model, p.material)).collect()
}

/// Adds a piece's `$` helpers: the first piece to name one keeps it, so the body's helpers win over a wheel part's.
fn merge_helpers(helpers: &mut HashMap<String, Affine3A>, piece: HashMap<String, Affine3A>) {
    for (name, helper) in piece {
        helpers.entry(name).or_insert(helper);
    }
}

/// Spawns the prepared parts under a new root at `transform`.
pub fn spawn(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, pieces: Vec<Piece>, transform: Transform) -> Entity {
    let root = commands.spawn((transform, Visibility::default())).id();
    let mut helpers = HashMap::new();
    for Piece {
        part,
        model,
        helpers: piece_helpers,
    } in pieces
    {
        // the body parts come first: a same-named helper on a wheel part never takes the driver's seat
        merge_helpers(&mut helpers, piece_helpers);
        let placed = Transform {
            translation: cry_to_bevy(part.pos),
            rotation: cry_rot_to_bevy(part.rot),
            scale: Vec3::new(part.scale.x, part.scale.z, part.scale.y),
        };
        let entity = match model {
            PieceModel::Static(prepared) => {
                let options = static_options(&part);
                let loaded = cry.finish_model(prepared);
                let model = spawn_loaded(commands, &part.model, &loaded, &options);
                let holder = commands.spawn((placed, Visibility::default())).id();
                commands.entity(model).insert(ChildOf(holder));
                Some(holder)
            }
            PieceModel::Rigged(models) => {
                for m in models {
                    cry.finish_model(m);
                }
                super::creature::spawn(commands, cry, clips, &part.model, None, placed)
            }
        };
        if let Some(e) = entity {
            commands.entity(e).insert(ChildOf(root));
        }
    }
    commands.entity(root).insert(SeatHelpers(helpers));
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vehicle_parts_request_full_illum_shading() {
        let part = Part {
            model: "objects/env/06_unit/05_carriage/transfers_trailer_a_body.chr".into(),
            material: None,
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        assert_eq!(static_options(&part).shading, Shading::Full);
        let rigged = model_options(None, true);
        assert_eq!(rigged.shading, Shading::Full, "the body .chr keeps the same Full Illum path");
        assert!(rigged.cry_space);
    }

    #[test]
    fn the_body_helper_wins_over_a_same_named_wheel_helper() {
        let body = Affine3A::from_translation(Vec3::new(-0.02, -0.50, 1.35));
        let wheel = Affine3A::from_translation(Vec3::new(9.0, 9.0, 9.0));
        let mut helpers = HashMap::new();
        merge_helpers(&mut helpers, HashMap::from([("driver".to_string(), body)]));
        merge_helpers(&mut helpers, HashMap::from([("driver".to_string(), wheel), ("wheel0".to_string(), wheel)]));
        assert_eq!(helpers["driver"], body, "the body's $driver helper wins");
        assert_eq!(helpers["wheel0"], wheel, "the wheel's own helper is kept");
    }

    #[test]
    fn wheel_mounts_are_named_wheel_n_and_wheel_type2_n() {
        assert_eq!(wheel_slot("wheel0"), Some(0));
        assert_eq!(wheel_slot("Wheel12"), Some(0));
        assert_eq!(wheel_slot("$wheel1"), Some(0));
        assert_eq!(wheel_slot("wheel_type2"), Some(1));
        assert_eq!(wheel_slot("wheel_type21"), Some(1));
        assert_eq!(wheel_slot("wheel"), None);
        assert_eq!(wheel_slot("bone_steeringwheel"), None);
        assert_eq!(wheel_slot("wheel_l"), None);
    }

    #[test]
    #[ignore = "requires the client pak"]
    fn the_wagon_body_exposes_its_driver_helper() {
        let look = VehicleLook {
            normal: "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body".into(),
            wheels: [
                Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf".into()),
                Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf".into()),
            ],
        };
        let helpers = body_helpers(&look);
        let driver = helpers.get("driver").expect("the body's $driver helper");
        let expected = Vec3::new(-0.02, -0.50, 1.35);
        assert!(
            driver.translation.distance(expected.into()) < 0.01,
            "driver at {:?}",
            driver.translation
        );
    }

    #[test]
    #[ignore = "requires the client pak"]
    fn the_airship_has_its_moving_parts_and_a_cart_its_wheels() {
        let airship = parts(&VehicleLook { normal: "prefab://Prefabs/airship.xml/airship.normal".into(), wheels: [None, None] });
        let models: Vec<&str> = airship.iter().map(|p| p.model.as_str()).collect();
        assert!(models.contains(&"objects/env/06_unit/02_airship/airship_base.cgf"), "{models:?}");
        assert!(models.contains(&"objects/env/06_unit/02_airship/airship_propeller_l.chr"), "{models:?}");
        assert_eq!(models.len(), 7, "{models:?}");

        let wheel = "objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf";
        let wheel2 = "objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf";
        let tractor = parts(&VehicleLook {
            normal: "prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body".into(),
            wheels: [Some(wheel.into()), Some(wheel2.into())],
        });
        let wheels: Vec<&Part> = tractor.iter().filter(|p| p.model == wheel || p.model == wheel2).collect();
        assert_eq!(wheels.len(), 4, "{tractor:?}");
        assert_eq!(wheels.iter().filter(|p| p.model == wheel2).count(), 2);
        // left and right of the body
        assert!(wheels.iter().any(|p| p.pos.x < -0.3) && wheels.iter().any(|p| p.pos.x > 0.3), "{wheels:?}");
    }
}
