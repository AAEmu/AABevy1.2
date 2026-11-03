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

/// The `$ladder` helper of a doodad model, in the doodad root's Bevy space.
///
/// 1.2 reads an element named `$ladder` (`FUN_393aaad0`); its matrix and the box of its child give
/// the climb frame. Here the helper's local Z is the climb axis and the model geometry's extent
/// along it gives the segment. The helper's local Y is kept for the relative-angle kinds
/// (2, 3, 6, 7, 10); kinds 1, 4, 5, 8 and 9 ignore it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderHelper {
    /// Bottom and top of the climb segment, in the doodad's local Bevy space.
    pub bottom: Vec3,
    pub top: Vec3,
    /// Unit bottom-to-top direction.
    pub axis: Vec3,
    /// The helper's local Y in the doodad's local Bevy space.
    pub forward: Vec3,
}

pub struct Prepared {
    prefab: bool,
    pieces: Vec<Piece>,
    helpers: HashMap<String, Affine3A>,
    /// The combined LOD 0 bounds of the parts in the prefab's local cry frame (metres).
    pub bounds: Option<(Vec3, Vec3)>,
    ladder: Option<LadderHelper>,
}

pub struct Spawned {
    pub entity: Entity,
    /// Prefab-local Cry metres; attachment applies the doodad world transform separately.
    pub helpers: HashMap<String, Affine3A>,
    /// The climb helper, when the model has a usable `$ladder`.
    pub ladder: Option<LadderHelper>,
    /// The combined LOD 0 bounds in the prefab local Cry frame, for the interaction gap.
    pub bounds: Option<(Vec3, Vec3)>,
}

/// The `$ladder` helper of a model part, in the doodad root's Bevy space. The helper's matrix is
/// `part * node` in Cry space; the box of the mesh chunk the node names (its child box) is
/// transformed into helper-local space so its Z extent is the climb segment. `None` without an
/// exact `$ladder` element or a usable box (1.2 also requires the element and its child box,
/// `FUN_393aaad0`).
fn ladder_of(part: &Part, model: &aa_cry::mesh::Model) -> Option<LadderHelper> {
    let index = model
        .nodes
        .iter()
        .position(|n| n.name.eq_ignore_ascii_case("$ladder"))?;
    let node = model.world_transform(index);
    let object = *model.node_objects.get(index)?;
    let (min, max) = model.mesh_box_of_chunk.get(&object).copied()?;
    let inverse = node.inverse();
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for corner in 0..8 {
        let p = Vec3::new(
            if corner & 1 == 0 { min.x } else { max.x },
            if corner & 2 == 0 { min.y } else { max.y },
            if corner & 4 == 0 { min.z } else { max.z },
        );
        let p = inverse.transform_point3(p);
        lo = lo.min(p.z);
        hi = hi.max(p.z);
    }
    if !(hi - lo).is_finite() || hi - lo < 0.2 {
        return None;
    }
    let part = Affine3A::from_scale_rotation_translation(part.scale, part.rot, part.pos);
    let helper = part * node;
    let bottom = cry_to_bevy(helper.transform_point3(Vec3::new(0.0, 0.0, lo)));
    let top = cry_to_bevy(helper.transform_point3(Vec3::new(0.0, 0.0, hi)));
    let axis = (top - bottom).normalize_or_zero();
    if axis == Vec3::ZERO {
        return None;
    }
    let forward = cry_to_bevy(helper.transform_vector3(Vec3::Y)).normalize_or_zero();
    Some(LadderHelper {
        bottom,
        top,
        axis,
        forward,
    })
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
            animation: None,
        }]
    };
    let mut pieces = Vec::new();
    let mut helpers = HashMap::new();
    let mut bounds: Option<(Vec3, Vec3)> = None;
    let mut ladder = None;
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
                if ladder.is_none() {
                    ladder = ladder_of(&part, &model.loaded().model);
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
        ladder,
    })
}

pub fn spawn(commands: &mut Commands, cry: &mut CryAssets, prepared: Prepared) -> Spawned {
    let Prepared {
        prefab,
        pieces,
        helpers,
        bounds,
        ladder,
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
        ladder,
        bounds,
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
            animation: None,
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

    /// The `$ladder` helper and the box it names give the climb segment (needs `AA_PAK`).
    #[test]
    fn the_original_ladder_models_have_a_usable_climb_segment() {
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        for (path, length, tolerance) in [
            (
                "objects/env/01_nuia/001_ndeco/climb/ndeco_do_ladder01.cgf",
                5.050,
                0.01,
            ),
            (
                "objects/env/06_unit/01_ship/smallboat/smallboat_a_ladder_left.cgf",
                1.677,
                0.01,
            ),
        ] {
            let model = aa_cry::mesh::Model::load(path).expect(path);
            let part = Part {
                model: path.into(),
                material: None,
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                scale: Vec3::ONE,
                animation: None,
            };
            let ladder =
                ladder_of(&part, &model).unwrap_or_else(|| panic!("{path}: no $ladder segment"));
            assert!(
                (ladder.bottom.distance(ladder.top) - length).abs() < tolerance,
                "{path}: {ladder:?}"
            );
            // Cry Z (up) is Bevy Y
            assert!(ladder.axis.y > 0.99, "{path}: {:?}", ladder.axis);
            assert!(ladder.forward.length() > 0.99, "{path}: {ladder:?}");
        }
    }

    fn node(name: &str, parent: Option<usize>) -> Node {
        Node {
            name: name.into(),
            chunk_id: 0,
            parent,
            local: Affine3A::IDENTITY,
            kind: NodeKind::Helper,
            mesh: None,
            material_name: None,
            properties: String::new(),
        }
    }

    fn part() -> Part {
        Part {
            model: "ladder.cgf".into(),
            material: None,
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            scale: Vec3::ONE,
            animation: None,
        }
    }

    /// Only an exact `$ladder` helper is a climb element (`XlCompareStringI("$ladder")`).
    #[test]
    fn a_helper_named_ladder_without_the_dollar_is_not_a_climb() {
        for name in ["ladder", "$ladder01", "$LADDER01"] {
            let mut model = Model {
                nodes: vec![node(name, None)],
                node_objects: vec![10],
                ..default()
            };
            model
                .mesh_box_of_chunk
                .insert(10, (Vec3::ZERO, Vec3::Z * 3.0));
            assert!(ladder_of(&part(), &model).is_none(), "{name}");
        }
        // the exact name is accepted (case-insensitively, as the native compare)
        let mut model = Model {
            nodes: vec![node("$LADDER", None)],
            node_objects: vec![10],
            ..default()
        };
        model
            .mesh_box_of_chunk
            .insert(10, (Vec3::ZERO, Vec3::Z * 3.0));
        assert!(ladder_of(&part(), &model).is_some());
    }

    /// The climb segment is the box of the mesh chunk `$ladder` names, not the model bounds.
    #[test]
    fn the_climb_segment_uses_the_ladder_child_box() {
        let mut model = Model {
            nodes: vec![node("$ladder", None)],
            node_objects: vec![10],
            ..default()
        };
        model
            .mesh_box_of_chunk
            .insert(10, (Vec3::ZERO, Vec3::Z * 2.0));
        // the whole model's other geometry is ten metres tall: it must not be the climb
        model.mesh_boxes = vec![(Vec3::ZERO, Vec3::Z * 10.0)];
        let ladder = ladder_of(&part(), &model).expect("the $ladder box");
        assert!(
            (ladder.bottom.distance(ladder.top) - 2.0).abs() < 1e-5,
            "{ladder:?}"
        );
    }

    /// A ship-side ladder prefab carries the `$ladder` in its ladder CGF (needs `AA_PAK`).
    #[test]
    fn the_original_ship_side_ladder_prefabs_are_climbable() {
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        for reference in [
            "prefab://Prefabs/galleon.xml/galleon.galleon_ladder",
            "prefab://Prefabs/galleon.xml/galleon.ladder_rear_left",
            "prefab://prefabs/smallboat.xml/smallboat.smallboat_a_ladder_left",
        ] {
            let parts = super::super::housing::prefab::load(reference);
            assert!(!parts.is_empty(), "{reference}");
            let mut ladder = None;
            for part in &parts {
                if let Ok(model) = aa_cry::mesh::Model::load(&part.model) {
                    ladder = ladder_of(part, &model);
                    if ladder.is_some() {
                        break;
                    }
                }
            }
            let ladder = ladder.unwrap_or_else(|| panic!("{reference}: no $ladder"));
            assert!(
                ladder.bottom.distance(ladder.top) > 0.5,
                "{reference}: {ladder:?}"
            );
        }
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
            animation: None,
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
