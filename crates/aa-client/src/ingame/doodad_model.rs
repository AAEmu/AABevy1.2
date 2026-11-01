//! Doodad geometry: a single CGF or every mesh part of its authored prefab.
//! Bond helpers retain the complete prefab/CGF parent chain, in Cry metres.

use std::collections::HashMap;

use aa_cry::scene::{CryAssets, ModelOptions, spawn_loaded};
use aa_cry::{CryLoader, PreparedModel};
use aa_world::coords::cry_rot_to_bevy;
use aa_world::cry_to_bevy;
use bevy::math::Affine3A;
use bevy::prelude::*;

use super::housing::prefab::{self, Part};

struct Piece {
    part: Part,
    model: PreparedModel,
}

pub struct Prepared {
    prefab: bool,
    pieces: Vec<Piece>,
    helpers: HashMap<String, Affine3A>,
    /// The combined LOD 0 bounds of the parts in the prefab's local cry frame (metres).
    pub bounds: Option<(Vec3, Vec3)>,
}

pub struct Spawned {
    pub entity: Entity,
    /// Prefab-local Cry metres; attachment applies the doodad world transform separately.
    pub helpers: HashMap<String, Affine3A>,
}

fn helpers_of(part: &Part, model: &aa_cry::mesh::Model, out: &mut HashMap<String, Affine3A>) {
    let placement = Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos);
    for (index, node) in model.nodes.iter().enumerate() {
        let name = node.name.strip_prefix('$').unwrap_or(&node.name);
        if name.starts_with("bond") {
            out.entry(name.to_string())
                .or_insert_with(|| placement * model.world_transform(index));
        }
    }
}

/// The model's bounds placed by the prefab's part transform, still in the prefab's local cry frame.
fn part_bounds(part: &Part, model: &aa_cry::mesh::Model) -> Option<(Vec3, Vec3)> {
    let (lo, hi) = model.bounds()?;
    let placement = Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos);
    let mut out: Option<(Vec3, Vec3)> = None;
    for x in [lo.x, hi.x] {
        for y in [lo.y, hi.y] {
            for z in [lo.z, hi.z] {
                let p = placement.transform_point3(Vec3::new(x, y, z));
                out = Some(match out {
                    None => (p, p),
                    Some((a, b)) => (a.min(p), b.max(p)),
                });
            }
        }
    }
    out
}

pub fn prepare(loader: &CryLoader, reference: &str) -> Option<Prepared> {
    prepare_inner(loader, reference, false)
}

/// [`prepare`], and also the combined LOD 0 bounds the shipyard HUD picks by.
pub fn prepare_with_bounds(loader: &CryLoader, reference: &str) -> Option<Prepared> {
    prepare_inner(loader, reference, true)
}

fn prepare_inner(loader: &CryLoader, reference: &str, with_bounds: bool) -> Option<Prepared> {
    let prefab = reference.starts_with("prefab://");
    let parts = if prefab {
        prefab::load(reference)
    } else {
        vec![Part {
            model: reference.into(),
            material: None,
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
        }]
    };
    let mut pieces = Vec::new();
    let mut helpers = HashMap::new();
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for part in parts {
        let options = ModelOptions {
            material: part
                .material
                .clone()
                .filter(|p| aa_pak::Pak::global().exists(p)),
            ..default()
        };
        match loader.prepare_model(&part.model, &options) {
            Ok(model) => {
                helpers_of(&part, &model.loaded().model, &mut helpers);
                if with_bounds
                    && let Some((lo, hi)) = part_bounds(&part, model.loaded().model.as_ref())
                {
                    bounds = Some(match bounds {
                        None => (lo, hi),
                        Some((a, b)) => (a.min(lo), b.max(hi)),
                    });
                }
                pieces.push(Piece { part, model });
            }
            Err(error) => {
                warn!(
                    "doodad prefab {reference}: cannot prepare {}: {error}",
                    part.model
                );
                // Do not publish a partially assembled layout as a seat-ready model.
                return None;
            }
        }
    }
    (!pieces.is_empty()).then_some(Prepared {
        prefab,
        pieces,
        helpers,
        bounds,
    })
}

pub fn spawn(commands: &mut Commands, cry: &mut CryAssets, prepared: Prepared) -> Spawned {
    let Prepared {
        prefab,
        pieces,
        helpers,
        bounds: _,
    } = prepared;
    let root = prefab.then(|| {
        commands
            .spawn((
                Name::new("doodad prefab"),
                Transform::IDENTITY,
                Visibility::default(),
            ))
            .id()
    });
    let mut single = None;
    for Piece { part, model } in pieces {
        let loaded = cry.finish_model(model);
        let model = spawn_loaded(commands, &part.model, &loaded, &ModelOptions::default());
        if let Some(root) = root {
            let holder = commands
                .spawn((
                    Transform {
                        translation: cry_to_bevy(part.pos),
                        rotation: cry_rot_to_bevy(part.rot),
                        scale: Vec3::new(part.scale.x, part.scale.z, part.scale.y),
                    },
                    Visibility::default(),
                    ChildOf(root),
                ))
                .id();
            commands.entity(model).insert(ChildOf(holder));
        } else {
            single = Some(model);
        }
    }
    Spawned {
        entity: root.or(single).expect("prepared doodad contains geometry"),
        helpers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_cry::mesh::{Model, Node, NodeKind};

    #[test]
    fn helper_composes_part_and_parent_chain_without_losing_scale() {
        let part = Part {
            model: "chair.cgf".into(),
            material: None,
            pos: Vec3::new(7.0, 8.0, 9.0),
            rot: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(2.0, 3.0, 4.0),
        };
        let node = |name: &str, parent, local| Node {
            name: name.into(),
            chunk_id: 0,
            parent,
            local,
            kind: NodeKind::Helper,
            mesh: None,
            material_name: None,
            properties: String::new(),
        };
        let model = Model {
            nodes: vec![
                node(
                    "group",
                    None,
                    Affine3A::from_scale_rotation_translation(
                        Vec3::splat(2.0),
                        Quat::IDENTITY,
                        Vec3::X,
                    ),
                ),
                node(
                    "$bond-2",
                    Some(0),
                    Affine3A::from_translation(Vec3::Y * 0.5),
                ),
            ],
            ..default()
        };
        let mut helpers = HashMap::new();
        helpers_of(&part, &model, &mut helpers);
        assert_eq!(helpers.len(), 1);
        let helper = helpers["bond-2"];
        assert!(
            helper
                .transform_point3(Vec3::ZERO)
                .distance(Vec3::new(4.0, 10.0, 9.0))
                < 1e-5
        );
        assert!((helper.transform_vector3(Vec3::X).length() - 4.0).abs() < 1e-5);
        assert!(
            !helpers.contains_key("bond0"),
            "signed spot is not replaced with a default"
        );
    }

    #[test]
    fn part_bounds_follow_the_part_transform() {
        let node = Node {
            name: "hull".into(),
            chunk_id: 0,
            parent: None,
            local: Affine3A::IDENTITY,
            kind: NodeKind::Mesh,
            mesh: Some(0),
            material_name: None,
            properties: String::new(),
        };
        let model = Model {
            nodes: vec![node],
            meshes: vec![aa_cry::mesh::MeshData {
                positions: vec![Vec3::ZERO, Vec3::ONE],
                ..default()
            }],
            ..default()
        };
        let part = Part {
            model: "hull.cgf".into(),
            material: None,
            pos: Vec3::new(10.0, 0.0, 0.0),
            rot: Quat::IDENTITY,
            scale: Vec3::splat(2.0),
        };
        let (lo, hi) = part_bounds(&part, &model).expect("hull bounds");
        assert!((lo - Vec3::new(10.0, 0.0, 0.0)).length() < 1e-5, "{lo}");
        assert!((hi - Vec3::new(12.0, 2.0, 2.0)).length() < 1e-5, "{hi}");
    }

    #[test]
    #[ignore = "requires original prefab and chair CGFs"]
    fn original_instrument_helpers_belong_to_the_chair() {
        for (name, expected) in [
            (
                "furniture.piano_a",
                Vec3::new(-0.080078125, -1.2835332, 0.0),
            ),
            (
                "furniture.piano_b",
                Vec3::new(-0.080078125, -1.3024902, 0.0),
            ),
            (
                "furniture.percussion_a",
                Vec3::new(-0.14785767, -0.41488084, 0.0),
            ),
        ] {
            let parts = prefab::load(&format!("prefab://prefabs/interaction_c.xml/{name}"));
            assert_eq!(parts.len(), 2, "{name}");
            let mut helpers = HashMap::new();
            let mut owners = Vec::new();
            for part in &parts {
                let model = aa_cry::mesh::Model::load(&part.model).unwrap();
                if model.nodes.iter().any(|n| n.name == "$bond0") {
                    owners.push(part.model.clone());
                }
                helpers_of(part, &model, &mut helpers);
            }
            assert_eq!(owners.len(), 1);
            assert!(owners[0].contains("chair"));
            let helper = helpers["bond0"];
            assert!(
                helper.transform_point3(Vec3::ZERO).distance(expected) < 1e-5,
                "{name}: {helper:?}"
            );
            let (scale, rotation, _) = helper.to_scale_rotation_translation();
            assert!(scale.distance(Vec3::ONE) < 1e-5);
            assert!(
                rotation.angle_between(Quat::IDENTITY) < 1e-5,
                "piano-body rotation must not rotate chair helper"
            );
        }
    }
}
