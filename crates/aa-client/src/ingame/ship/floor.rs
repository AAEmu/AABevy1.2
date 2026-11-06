//! The walkable surface of a ship hull, salvaged from the older own-ship work
//! (`test/ships` deck floor): the upward-facing triangles of the hull and deck brushes in
//! the ship's frame (cry space: x right, y forward, z up, origin at the hull
//! root), bucketed by 1 m cells. The rowboat's `smallboat_body.cgf` and the
//! galleon's `galleon_body.cgf` are the brushes `smallboat.xml` / `galleon.xml`
//! name `body`.

use std::collections::HashMap;

use bevy::prelude::{Vec2, Vec3};

use super::super::housing::prefab::{self, Part};

/// A triangle whose normal is at least this vertical is a floor (a wall or a
/// hull side is not); the winding of the file's faces is not trusted.
const FLOOR_NORMAL_Z: f32 = 0.5;

#[derive(Debug, Default)]
pub struct ShipFloor {
    tris: Vec<[Vec3; 3]>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    /// XY bounds of the floor triangles, for the cheap range gate.
    bounds: Option<(Vec2, Vec2)>,
}

impl ShipFloor {
    pub fn new(tris: impl IntoIterator<Item = [Vec3; 3]>) -> ShipFloor {
        let mut floor = ShipFloor::default();
        for tri in tris {
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            if normal.length_squared() < 1e-12 || normal.normalize().z.abs() < FLOOR_NORMAL_Z {
                continue;
            }
            let id = floor.tris.len() as u32;
            let (lo, hi) = (
                tri[0].min(tri[1]).min(tri[2]),
                tri[0].max(tri[1]).max(tri[2]),
            );
            let (lo, hi) = (Vec2::new(lo.x, lo.y), Vec2::new(hi.x, hi.y));
            floor.bounds = Some(match floor.bounds {
                Some((min, max)) => (min.min(lo), max.max(hi)),
                None => (lo, hi),
            });
            for cx in lo.x.floor() as i32..=hi.x.floor() as i32 {
                for cy in lo.y.floor() as i32..=hi.y.floor() as i32 {
                    floor.cells.entry((cx, cy)).or_default().push(id);
                }
            }
            floor.tris.push(tri);
        }
        floor
    }

    /// The floor triangles of the hull brush `part` of `model` (render nodes of
    /// the highest detail).
    pub fn from_model(model: &aa_cry::mesh::Model, part: &Part) -> ShipFloor {
        let place = |p: Vec3| part.pos + part.rot * (p * part.scale);
        let mut tris = Vec::new();
        for node in model.lod_nodes(0) {
            let Some(mesh) = model.nodes[node].mesh.and_then(|m| model.meshes.get(m)) else {
                continue;
            };
            let to_model = model.world_transform(node);
            let at = |i: u32| {
                mesh.positions
                    .get(i as usize)
                    .map(|p| place(to_model.transform_point3(*p)))
            };
            for tri in mesh.indices.as_chunks::<3>().0 {
                if let (Some(a), Some(b), Some(c)) = (at(tri[0]), at(tri[1]), at(tri[2])) {
                    tris.push([a, b, c]);
                }
            }
        }
        ShipFloor::new(tris)
    }

    /// The walkable floor of a `ShipModel`'s `ship_models.normal` prefab: the up-facing triangles of every
    /// hull and deck brush ([`is_deck_part`]), merged; `None` when the pak has no readable one.
    pub fn load(normal: &str) -> Option<ShipFloor> {
        let mut tris = Vec::new();
        for part in prefab::load(normal).iter().filter(|p| is_deck_part(&p.model)) {
            if let Ok(model) = aa_cry::mesh::Model::load(&part.model) {
                tris.extend(ShipFloor::from_model(&model, part).tris);
            }
        }
        let floor = ShipFloor::new(tris);
        (!floor.is_empty()).then_some(floor)
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// The XY bounds of the floor triangles in the ship frame.
    pub fn bounds(&self) -> Option<(Vec2, Vec2)> {
        self.bounds
    }

    /// The highest floor at `(x, y)` of the ship's frame that is not above
    /// `max_z`.
    pub fn height(&self, x: f32, y: f32, max_z: f32) -> Option<f32> {
        let ids = self.cells.get(&(x.floor() as i32, y.floor() as i32))?;
        let mut best: Option<f32> = None;
        for &id in ids {
            let [a, b, c] = self.tris[id as usize];
            // barycentric coordinates of (x, y) in the triangle's projection
            let (v0, v1, v2) = (
                Vec2::new(b.x - a.x, b.y - a.y),
                Vec2::new(c.x - a.x, c.y - a.y),
                Vec2::new(x - a.x, y - a.y),
            );
            let det = v0.x * v1.y - v1.x * v0.y;
            if det.abs() < 1e-9 {
                continue;
            }
            let u = (v2.x * v1.y - v1.x * v2.y) / det;
            let v = (v0.x * v2.y - v2.x * v0.y) / det;
            if u < -1e-4 || v < -1e-4 || u + v > 1.0 + 1e-4 {
                continue;
            }
            let z = a.z + u * (b.z - a.z) + v * (c.z - a.z);
            if z <= max_z && best.is_none_or(|h| z > h) {
                best = Some(z);
            }
        }
        best
    }
}

/// Whether a prefab part is a hull or deck brush a player can stand on. 1.2 walks on the CryPhysics of the whole
/// prefab (the decoded set is unknown), so this is inferred from the prefab sweep: a static `.cgf` that is a hull
/// (`*_body*`, or named like the prefab file: `boxship`, `cruise_a`, `soloship_a`, `speedboat_a`, `fishboat`,
/// `ray`) or a raised structure (`*_helms`, `*_add*` of the cruise ship). Animated `.chr` / `.cga` parts,
/// decoration, rigging, cannons, anchors, rudders, ladders, damage variants and the `box.cgf` helper are left out.
fn is_deck_part(model: &str) -> bool {
    let Some(stem) = model.strip_suffix(".cgf").map(|m| m.rsplit('/').next().unwrap_or(m)) else {
        return false;
    };
    const DECOR: [&str; 15] = [
        "ndeco", "barrel", "chainbox", "rope", "shield", "cannon", "sail", "flag", "anchor", "rudder", "ladder",
        "damage", "demage", "item", "chair",
    ];
    if stem == "box" || DECOR.iter().any(|d| stem.contains(d)) {
        return false;
    }
    const HULLS: [&str; 7] = ["boxship", "cruise_a", "soloship_a", "speedboat_a", "fishboat", "ray", "coupleduckship"];
    stem.contains("_body") || stem.ends_with("_helms") || HULLS.iter().any(|h| stem.starts_with(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ship_floor_answers_the_highest_surface_under_the_limit() {
        // a main deck at z 2 (x -4..4, y -10..10) and a poop deck at z 6
        // (y 10..16), one wall (vertical) and a ceiling at 9
        let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
            [
                [
                    Vec3::new(x0, y0, z),
                    Vec3::new(x1, y0, z),
                    Vec3::new(x1, y1, z),
                ],
                [
                    Vec3::new(x0, y0, z),
                    Vec3::new(x1, y1, z),
                    Vec3::new(x0, y1, z),
                ],
            ]
        };
        let wall = [
            Vec3::new(4.0, -10.0, 2.0),
            Vec3::new(4.0, 10.0, 2.0),
            Vec3::new(4.0, 10.0, 5.0),
        ];
        let floor = ShipFloor::new(
            quad(-4.0, 4.0, -10.0, 10.0, 2.0)
                .into_iter()
                .chain(quad(-4.0, 4.0, 10.0, 16.0, 6.0))
                .chain([wall])
                .chain(quad(-4.0, 4.0, -10.0, 10.0, 9.0)),
        );
        assert_eq!(
            floor.height(0.0, 0.0, 3.0),
            Some(2.0),
            "the main deck under the player"
        );
        assert_eq!(
            floor.height(0.0, 0.0, 20.0),
            Some(9.0),
            "the highest surface under a high limit"
        );
        assert_eq!(floor.height(1.0, 12.5, 7.0), Some(6.0), "the poop deck");
        assert_eq!(
            floor.height(1.0, 12.5, 4.0),
            None,
            "a step up of more than the limit is a wall"
        );
        assert_eq!(
            floor.height(6.0, 0.0, 9.0),
            None,
            "beside the hull there is no deck"
        );
    }

    #[test]
    fn the_rowboat_hull_has_a_deck_when_the_pak_is_present() {
        if !aa_cry::pak_available() {
            return;
        }
        let floor = ShipFloor::load("prefab://prefabs/smallboat.xml/smallboat.smallboat_a")
            .expect("the smallboat hull is readable");
        assert!(!floor.is_empty());
        assert!(
            floor.height(0.0, 0.0, 10.0).is_some(),
            "a floor at the hull root"
        );
    }

    #[test]
    fn deck_parts_are_hulls_and_raised_structures_not_decoration() {
        for yes in [
            "objects/ships/smallboat_body.cgf",
            "objects/ships/fishboat_body_step02.cgf",
            "objects/ships/boxship.cgf",
            "objects/ships/cruise_a.cgf",
            "objects/ships/cruise_a_add05.cgf",
            "objects/ships/soloship_a.cgf",
            "objects/ships/speedboat_a.cgf",
            "objects/ships/caravel_helms.cgf",
            "objects/ships/galleon_body.cgf",
        ] {
            assert!(is_deck_part(yes), "{yes}");
        }
        for no in [
            "objects/ships/caravel_sail1.cgf",
            "objects/ships/caravel_anchor.chr",
            "objects/ships/fishboat_wheel.cga",
            "objects/ships/galleon_cannon.cgf",
            "objects/ships/galleon_rudder.cgf",
            "objects/ships/box.cgf",
            "objects/env/barrel_oak_b.cgf",
            "objects/ships/boxship_ladder_1.cgf",
            "objects/ships/soloship_a_damage50.cgf",
        ] {
            assert!(!is_deck_part(no), "{no}");
        }
    }

    /// Every `ship_models.normal` prefab of the game database has a floor under its `$driver` helper and a
    /// passenger point (within 1.5 m below it); prints the ships that do not (needs `AA_PAK` and `AA_DB`).
    #[test]
    fn every_ship_prefab_has_a_deck_under_its_seats_when_the_data_is_present() {
        if !aa_cry::pak_available() {
            return;
        }
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(db) = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return;
        };
        let mut st = db.prepare("SELECT DISTINCT normal FROM ship_models WHERE normal IS NOT NULL").unwrap();
        let normals: Vec<String> = st.query_map([], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        let (mut empty, mut seatless, mut checked) = (Vec::new(), Vec::new(), 0);
        for normal in normals {
            let parts = prefab::load(&normal);
            let Some(floor) = ShipFloor::load(&normal) else {
                empty.push(normal);
                continue;
            };
            let mut seats = Vec::new();
            for part in &parts {
                let Ok(model) = aa_cry::mesh::Model::load(&part.model) else { continue };
                for (i, node) in model.nodes.iter().enumerate() {
                    let name = node.name.trim_start_matches('$');
                    if name == "driver" || name.starts_with("passenger") {
                        let at = model.world_transform(i).transform_point3(Vec3::ZERO);
                        seats.push((name.to_string(), part.pos + part.rot * (at * part.scale)));
                    }
                }
            }
            checked += seats.len();
            for (name, at) in seats {
                let under = floor.height(at.x, at.y, at.z + 0.5);
                if under.is_none_or(|h| at.z - h > 1.5) {
                    seatless.push(format!("{normal} {name} at {at:?} floor {under:?}"));
                }
            }
        }
        eprintln!("ships without a floor: {empty:#?}
seats without a floor within 1.5 m: {seatless:#?}");
        assert!(checked > 0 && seatless.is_empty(), "{seatless:#?}");
    }
}
