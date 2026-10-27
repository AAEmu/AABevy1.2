//! Vehicles and ships (`models.sub_type` `VehicleModel` / `ShipModel`: airships, carts, boats, siege weapons), drawn from
//! their `normal` look ([`super::data::Db::vehicle_look`]): a single model, or every part of a prefab (the hull and deck
//! brushes, and the `AnimObject` parts such as an airship's wings, propellers and planks). Wheels go on the body's wheel
//! mounts: `wheelN` bones or helper nodes take the `wheel` model, `wheel_type2N` ones `wheel2`.
//!
//! Left out: the damaged and dead looks, the prefabs' particle effects, wheel and propeller spin, turrets.

use aa_anim::ClipLibrary;
use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
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

/// The parts `look` is drawn with, relative to the unit's origin (cry space): the body parts, then the wheels.
pub fn parts(look: &VehicleLook) -> Vec<Part> {
    let body = if look.normal.to_ascii_lowercase().starts_with("prefab://") {
        prefab::load(&look.normal)
    } else {
        model_file(&look.normal)
            .map(|model| vec![Part { model, material: None, pos: Vec3::ZERO, rot: Quat::IDENTITY, scale: Vec3::ONE }])
            .unwrap_or_default()
    };
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
}

enum PieceModel {
    Static(PreparedModel),
    Rigged(Vec<PreparedModel>),
}

fn static_options(part: &Part) -> ModelOptions {
    ModelOptions { material: part.material.clone().filter(|m| aa_pak::Pak::global().exists(m)), ..default() }
}

/// Reads the models of `look` (callable from any thread). Parts whose model cannot be read are left out.
pub fn prepare(loader: &CryLoader, look: &VehicleLook) -> Vec<Piece> {
    parts(look)
        .into_iter()
        .filter_map(|part| {
            let rigged = part.model.ends_with(".chr") && aa_anim::pose::shared_rig(&part.model).is_ok_and(|r| !r.is_empty());
            let model = if rigged {
                let models: Vec<PreparedModel> = super::creature::model_files(&part.model, None)
                    .into_iter()
                    .filter_map(|(path, material)| loader.prepare_model(&path, &ModelOptions { material, cry_space: true, ..default() }).ok())
                    .collect();
                if models.is_empty() {
                    return None;
                }
                PieceModel::Rigged(models)
            } else {
                match loader.prepare_model(&part.model, &static_options(&part)) {
                    Ok(m) => PieceModel::Static(m),
                    Err(e) => {
                        debug!("vehicle part {}: {e}", part.model);
                        return None;
                    }
                }
            };
            Some(Piece { part, model })
        })
        .collect()
}

/// The model files [`prepare`] reads (the browser downloads them first).
pub fn model_files(look: &VehicleLook) -> Vec<(String, Option<String>)> {
    parts(look).into_iter().map(|p| (p.model, p.material)).collect()
}

/// Spawns the prepared parts under a new root at `transform`.
pub fn spawn(commands: &mut Commands, cry: &mut CryAssets, clips: &ClipLibrary, pieces: Vec<Piece>, transform: Transform) -> Entity {
    let root = commands.spawn((transform, Visibility::default())).id();
    for Piece { part, model } in pieces {
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
    root
}

#[cfg(test)]
mod tests {
    use super::*;

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
