//! World descriptions: `worlds/<name>/world.g` (zones and cells) and `world.xml` (which zone owns
//! each 64 m sector).
//!
//! `world.g` is the short text form:
//!
//! ```text
//! name main_world
//! cellX 35
//! cellY 38
//! oceanLevel 100
//! zones
//!  zone ( name w_gweonid_forest_1, id 129 )
//! cells
//!  cell ( x 9, y 14, zones ( 129, 143, 164, 181 ) )     (a single zone is written without parentheses)
//! ```
//!
//! `world.xml` has `ZoneList/Zone(id, originX, originY)/cellList/cell(x, y)/sectorList/sector(x, y)`: the
//! sectors a zone owns. Zones carry the environment (`zone/<id>/{env.xml, time_of_day.xml, clouds.xml}`).

use std::collections::HashMap;

use bevy::math::Vec3;

use crate::coords::{CELL_SIZE, SECTOR_SIZE};

#[derive(Clone, Debug)]
pub struct Zone {
    pub id: u16,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct World {
    pub name: String,
    pub cells_x: i32,
    pub cells_y: i32,
    pub ocean_level: f32,
    pub zones: Vec<Zone>,
    /// Cell -> zone ids touching it, from `world.g`.
    pub cells: HashMap<(i32, i32), Vec<u16>>,
    /// Cell -> 16 x 16 sector owners (`sx * 16 + sy`, 0 = none), from `world.xml`; empty until
    /// [`World::load_sector_zones`].
    pub sector_zones: HashMap<(i32, i32), Box<[u16; 256]>>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("bad xml: {0}")]
    Xml(#[from] roxmltree::Error),
}

impl World {
    /// Reads `worlds/<name>/world.g` from the pak.
    pub fn load(name: &str) -> Result<World, WorldError> {
        let text = aa_pak::Pak::global().read_string(&format!("worlds/{name}/world.g"))?;
        Ok(World::parse_g(&text))
    }

    pub fn parse_g(text: &str) -> World {
        let mut w = World::default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("name ") {
                w.name = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("cellX ") {
                w.cells_x = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("cellY ") {
                w.cells_y = rest.trim().parse().unwrap_or(0);
            } else if let Some(rest) = line.strip_prefix("oceanLevel ") {
                w.ocean_level = rest.trim().parse().unwrap_or(0.0);
            } else if let Some(rest) = line.strip_prefix("zone (") {
                let body = rest.trim_end_matches(')').trim();
                let mut zone = Zone { id: 0, name: String::new() };
                for part in body.split(',') {
                    let part = part.trim();
                    if let Some(n) = part.strip_prefix("name ") {
                        zone.name = n.trim().to_string();
                    } else if let Some(i) = part.strip_prefix("id ") {
                        zone.id = i.trim().parse().unwrap_or(0);
                    }
                }
                w.zones.push(zone);
            } else if let Some(rest) = line.strip_prefix("cell (") {
                // `x 9, y 14, zones ( 129, 143 )` or `x 2, y 24, zones 295` or no zones at all
                let (head, zones) = match rest.find("zones") {
                    Some(i) => (&rest[..i], &rest[i + 5..]),
                    None => (rest, ""),
                };
                let mut x = 0;
                let mut y = 0;
                for part in head.split(',') {
                    let part = part.trim().trim_end_matches(')').trim();
                    if let Some(v) = part.strip_prefix("x ") {
                        x = v.trim().parse().unwrap_or(0);
                    } else if let Some(v) = part.strip_prefix("y ") {
                        y = v.trim().parse().unwrap_or(0);
                    }
                }
                let ids: Vec<u16> = zones
                    .split(|c: char| !c.is_ascii_digit())
                    .filter(|s| !s.is_empty())
                    .filter_map(|s| s.parse().ok())
                    .collect();
                w.cells.insert((x, y), ids);
            }
        }
        w
    }

    /// Reads `world.xml` (4.6 MB for main_world) for the sector -> zone ownership.
    pub fn load_sector_zones(&mut self) -> Result<(), WorldError> {
        let text = aa_pak::Pak::global().read_string(&format!("worlds/{}/world.xml", self.name))?;
        let doc = roxmltree::Document::parse(&text)?;
        for zone in doc.descendants().filter(|n| n.has_tag_name("Zone")) {
            let id: u16 = zone.attribute("id").and_then(|v| v.parse().ok()).unwrap_or(0);
            for cell in zone.descendants().filter(|n| n.has_tag_name("cell")) {
                let cx: i32 = cell.attribute("x").and_then(|v| v.parse().ok()).unwrap_or(0);
                let cy: i32 = cell.attribute("y").and_then(|v| v.parse().ok()).unwrap_or(0);
                let owners = self.sector_zones.entry((cx, cy)).or_insert_with(|| Box::new([0; 256]));
                for s in cell.descendants().filter(|n| n.has_tag_name("sector")) {
                    let sx: usize = s.attribute("x").and_then(|v| v.parse().ok()).unwrap_or(0);
                    let sy: usize = s.attribute("y").and_then(|v| v.parse().ok()).unwrap_or(0);
                    if sx < 16 && sy < 16 {
                        owners[sx * 16 + sy] = id;
                    }
                }
            }
        }
        Ok(())
    }

    /// Native world-layer point query: sector owner under global Cry-space XY, with no cell fallback.
    ///
    /// Needs [`World::load_sector_zones`]. Native `365F3250` truncates both cell and sector indices
    /// toward zero, checks their bounds, then returns the exact owner (`365F2680`). Z is ignored.
    /// This is the spatial zone lookup used by trial locations, independently of loaded subzone
    /// Area entities. An absent cell, unowned sector or invalid XY returns `None`.
    pub fn sector_zone_at(&self, pos: Vec3) -> Option<u16> {
        if !pos.x.is_finite() || !pos.y.is_finite() {
            return None;
        }
        let cx = (pos.x * (1.0 / CELL_SIZE)) as i32;
        let cy = (pos.y * (1.0 / CELL_SIZE)) as i32;
        if cx < 0 || cy < 0 || cx >= self.cells_x || cy >= self.cells_y {
            return None;
        }
        let sx = ((pos.x - cx as f32 * CELL_SIZE) * (1.0 / SECTOR_SIZE)) as i32;
        let sy = ((pos.y - cy as f32 * CELL_SIZE) * (1.0 / SECTOR_SIZE)) as i32;
        if !(0..16).contains(&sx) || !(0..16).contains(&sy) {
            return None;
        }
        let owners = self.sector_zones.get(&(cx, cy))?;
        let id = owners[(sx * 16 + sy) as usize];
        (id != 0).then_some(id)
    }

    /// Zone owning the sector under a cry-space position (needs [`World::load_sector_zones`]); falls
    /// back to the first zone of the cell.
    pub fn zone_at(&self, pos: Vec3) -> Option<u16> {
        let cx = (pos.x / CELL_SIZE).floor() as i32;
        let cy = (pos.y / CELL_SIZE).floor() as i32;
        let sx = (((pos.x - cx as f32 * CELL_SIZE) / SECTOR_SIZE) as usize).min(15);
        let sy = (((pos.y - cy as f32 * CELL_SIZE) / SECTOR_SIZE) as usize).min(15);
        if let Some(owners) = self.sector_zones.get(&(cx, cy)) {
            let id = owners[sx * 16 + sy];
            if id != 0 {
                return Some(id);
            }
        }
        self.cells.get(&(cx, cy)).and_then(|z| z.first().copied())
    }

    /// Native seamless ToD weights (crysystem 365EEC20): inspect the 5x5 sectors around
    /// the view, retain each zone's greatest `1-distanceToSector/90` weight, at most ten.
    /// The manager normalizes their sum when it blends raw curve values (300A5010).
    /// Coordinates here are global; the original works in an equivalent local 8-cell grid.
    pub fn environment_weights(&self, pos: Vec3) -> Vec<(u16, f32)> {
        let mut weights: Vec<(u16, f32)> = Vec::with_capacity(10);
        if !pos.is_finite() {
            return weights;
        }
        let sx = (pos.x / SECTOR_SIZE).floor() as i32;
        let sy = (pos.y / SECTOR_SIZE).floor() as i32;
        for x in sx - 2..=sx + 2 {
            for y in sy - 2..=sy + 2 {
                let Some(owners) = self.sector_zones.get(&(x.div_euclid(16), y.div_euclid(16)))
                else {
                    continue;
                };
                let id = owners[(x.rem_euclid(16) * 16 + y.rem_euclid(16)) as usize];
                if id == 0 {
                    continue;
                }
                let min = bevy::math::Vec2::new(x as f32, y as f32) * SECTOR_SIZE;
                let max = min + bevy::math::Vec2::splat(SECTOR_SIZE);
                let distance = (pos.truncate() - pos.truncate().clamp(min, max)).length();
                let weight = (1.0 - distance / 90.0).clamp(0.0, 1.0);
                if weight <= 0.0 || weights.len() >= 10 {
                    continue;
                }
                if let Some((_, existing)) = weights.iter_mut().find(|(zone, _)| *zone == id) {
                    *existing = existing.max(weight);
                } else {
                    weights.push((id, weight));
                }
            }
        }
        if weights.is_empty()
            && let Some(zone) = self.zone_at(pos)
        {
            weights.push((zone, 1.0));
        }
        weights
    }

    pub fn zone_name(&self, id: u16) -> Option<&str> {
        self.zones.iter().find(|z| z.id == id).map(|z| z.name.as_str())
    }
}

/// `cells/XXX_YYY` directory name of a cell.
pub fn cell_dir(world: &str, x: i32, y: i32) -> String {
    format!("worlds/{world}/cells/{x:03}_{y:03}")
}

/// The folder under `game/worlds/` whose `world.g` lists `zone`, or `None` for an unknown zone.
///
/// `SCLoadInstance` carries a zone key (`zone_id`); the server's world templates are built one per
/// `game/worlds/<name>/world.xml` (AAEmu `WorldTemplates::load`), and a zone key belongs to exactly one
/// of them (`WorldTemplates::get_world_id_by_zone_key`). The folder name, not the template's runtime
/// world id, is what [`World::load`] takes.
pub fn world_for_zone(zone: u32) -> Option<String> {
    let zone = u16::try_from(zone).ok()?;
    if !crate::pak_available() {
        return None;
    }
    let pak = aa_pak::Pak::global();
    let mut worlds: Vec<&str> = pak
        .list("worlds/")
        .filter_map(|path| {
            let path = path.strip_prefix("game/").unwrap_or(path);
            let name = path.strip_prefix("worlds/")?.strip_suffix("/world.g")?;
            (!name.is_empty() && !name.contains('/')).then_some(name)
        })
        .collect();
    worlds.sort_unstable();
    worlds.dedup();
    for name in worlds {
        let Ok(text) = pak.read_string(&format!("worlds/{name}/world.g")) else { continue };
        if World::parse_g(&text).zones.iter().any(|z| z.id == zone) {
            return Some(name.to_string());
        }
    }
    None
}

/// [`world_for_zone`] over already parsed worlds: the first `(folder name, world)` that lists `zone`.
pub fn find_world_for_zone<'a>(
    worlds: impl IntoIterator<Item = (&'a str, &'a World)>,
    zone: u32,
) -> Option<&'a str> {
    let zone = u16::try_from(zone).ok()?;
    worlds
        .into_iter()
        .find(|(_, world)| world.zones.iter().any(|z| z.id == zone))
        .map(|(name, _)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_world_g() {
        let w = World::parse_g(
            "name t\ncellX 2\ncellY 3\noceanLevel 100\nzones\n zone ( name a_b, id 129 )\n zone ( name c, id 7 )\ncells\n cell ( x 0, y 0 )\n cell ( x 9, y 14, zones ( 129, 143 ) )\n cell ( x 2, y 24, zones 295 )\n",
        );
        assert_eq!((w.cells_x, w.cells_y, w.ocean_level), (2, 3, 100.0));
        assert_eq!(w.zones.len(), 2);
        assert_eq!(w.zone_name(129), Some("a_b"));
        assert_eq!(w.cells[&(9, 14)], vec![129, 143]);
        assert_eq!(w.cells[&(2, 24)], vec![295]);
        assert!(w.cells[&(0, 0)].is_empty());
    }

    #[test]
    fn zone_mapping_uses_the_world_that_lists_the_zone() {
        let main = World::parse_g("name main_world\nzones\n zone ( name w_solzreed_1, id 142 )\n");
        let instance = World::parse_g("name instance_training_camp\nzones\n zone ( name instance_training_camp, id 198 )\n");
        let worlds = [("main_world", &main), ("instance_training_camp", &instance)];
        assert_eq!(find_world_for_zone(worlds, 198), Some("instance_training_camp"));
        assert_eq!(find_world_for_zone(worlds, 142), Some("main_world"));
        assert_eq!(find_world_for_zone(worlds, 999), None);
        assert_eq!(find_world_for_zone(worlds, u32::MAX), None);
    }

    #[test]
    fn real_zone_mapping_reads_the_pak_world_folders() {
        if !crate::pak_available() {
            return;
        }
        // zone 198 = instance_training_camp, 278 = instance_nachashgar, 300 = instance_prologue,
        // 142 = w_solzreed_1 in main_world (from each world.g under game/worlds).
        assert_eq!(world_for_zone(198).as_deref(), Some("instance_training_camp"));
        assert_eq!(world_for_zone(278).as_deref(), Some("instance_nachashgar"));
        assert_eq!(world_for_zone(300).as_deref(), Some("instance_prologue"));
        assert_eq!(world_for_zone(142).as_deref(), Some("main_world"));
        assert_eq!(world_for_zone(9999), None);
    }

    #[test]
    fn real_worlds() {
        if !crate::pak_available() {
            return;
        }
        let mut w = World::load("main_world").unwrap();
        assert_eq!(w.cells_x, 35);
        assert!(w.cells.len() > 1000);
        w.load_sector_zones().unwrap();
        let z = w.zone_at(Vec3::new(9.5 * 1024.0, 14.5 * 1024.0, 0.0)).unwrap();
        assert!(w.zone_name(z).is_some(), "zone {z}");
    }

    #[test]
    fn native_sector_query_bounds_truncation_and_no_cell_fallback() {
        let mut w = World { cells_x: 2, cells_y: 3, ..Default::default() };
        let mut owners = Box::new([0; 256]);
        owners[0] = 129;
        owners[1] = 130;
        owners[15 * 16] = 131;
        w.sector_zones.insert((0, 0), owners);
        let mut owners = Box::new([0; 256]);
        owners[0] = 7;
        w.sector_zones.insert((1, 0), owners);
        // The original 365F3250/365F32E0/365F2680 was executed with this sparse layout.
        for (x, y, zone) in [
            (0.0, 0.0, Some(129)),
            (-0.1, 0.0, Some(129)),
            (-63.999, 0.0, Some(129)),
            (-64.0, 0.0, None),
            (-1023.9, 0.0, None),
            (-1024.0, 0.0, None),
            (0.0, -63.999, Some(129)),
            (0.0, -64.0, None),
            (63.999, 0.0, Some(129)),
            (64.0, 0.0, None),
            (0.0, 63.999, Some(129)),
            (0.0, 64.0, Some(130)),
            (1023.999, 0.0, Some(131)),
            (1024.0, 0.0, Some(7)),
            (2047.9, 0.0, None),
            (2048.0, 0.0, None),
            (0.0, 3072.0, None),
            (64.0, 64.0, None),
            (f32::NAN, 0.0, None),
            (f32::INFINITY, 0.0, None),
            (f32::NEG_INFINITY, 0.0, None),
            (2_199_023_255_552.0, 0.0, None),
        ] {
            assert_eq!(w.sector_zone_at(Vec3::new(x, y, f32::NAN)), zone, "{x},{y}");
        }
        w.cells.insert((0, 0), vec![999]);
        let unowned = Vec3::new(64.0, 64.0, 0.0);
        assert_eq!(w.zone_at(unowned), Some(999));
        assert_eq!(w.sector_zone_at(unowned), None);
        w.sector_zones.clear();
        assert_eq!(w.sector_zone_at(Vec3::ZERO), None);
    }

    #[test]
    fn real_native_sector_query_original_x86_goldens() {
        if !crate::pak_available() {
            return;
        }
        let mut w = World::load("main_world").unwrap();
        w.load_sector_zones().unwrap();
        // Native point-query execution over all 35*38 original cell centres, including unowned
        // sectors. FNV preserves the complete X-major sequence without embedding another owner map.
        let mut hash = 0xcbf29ce484222325_u64;
        for x in 0..w.cells_x {
            for y in 0..w.cells_y {
                let zone = w.sector_zone_at(Vec3::new(
                    x as f32 * CELL_SIZE + 512.0,
                    y as f32 * CELL_SIZE + 512.0,
                    0.0,
                ));
                hash = (hash ^ u64::from(zone.unwrap_or(0))).wrapping_mul(0x100000001b3);
            }
        }
        assert_eq!(hash, 0x7284f3a5117503e5);
        for (x, y, zone) in [
            (15712.0, 15807.99, 179),
            (15712.0, 15808.0, 210),
            (15712.0, 15808.01, 210),
            (12428.0, 10430.0, 149),
            (21380.0, 8390.0, 158),
            (9728.0, 14848.0, 129),
        ] {
            assert_eq!(w.sector_zone_at(Vec3::new(x, y, 0.0)), Some(zone));
        }
    }

    #[test]
    fn native_environment_distance_max_weights_and_limit() {
        let mut world = World::default();
        // Two half planes, repeated over many sectors: repetitions must not add weight.
        let mut owners = Box::new([1; 256]);
        for x in 8..16 {
            for y in 0..16 {
                owners[x * 16 + y] = 2;
            }
        }
        world.sector_zones.insert((0, 0), owners);
        for offset in [-90.0_f32, -45.0, 0.0, 45.0, 90.0] {
            let weights = world.environment_weights(Vec3::new(512.0 + offset, 512.0, 999.0));
            let weight = |id| {
                weights
                    .iter()
                    .find(|(z, _)| *z == id)
                    .map_or(0.0, |(_, w)| *w)
            };
            assert!((weight(1) - (1.0 - offset.max(0.0) / 90.0)).abs() < 1e-6);
            assert!((weight(2) - (1.0 - (-offset).max(0.0) / 90.0)).abs() < 1e-6);
        }
        // A diagonal sector uses nearest AABB distance, not distance to its center.
        world.sector_zones.get_mut(&(0, 0)).unwrap()[9 * 16 + 9] = 3;
        let weights = world.environment_weights(Vec3::new(560.0, 560.0, -500.0));
        let w = weights.iter().find(|(z, _)| *z == 3).unwrap().1;
        assert!((w - (1.0 - (16.0_f32 * 16.0 * 2.0).sqrt() / 90.0)).abs() < 1e-6);
        for (i, z) in world
            .sector_zones
            .get_mut(&(0, 0))
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            *z = (i + 1) as u16;
        }
        assert_eq!(
            world
                .environment_weights(Vec3::new(512.0, 512.0, 0.0))
                .len(),
            10
        );
        assert!(world.environment_weights(Vec3::splat(f32::NAN)).is_empty());
    }

    #[test]
    fn real_solzreed_sea_border_weights_are_continuous() {
        if !crate::pak_available() {
            return;
        }
        let mut world = World::load("main_world").unwrap();
        world.load_sector_zones().unwrap();
        let a = Vec3::new(15712.0, 15808.0 - 0.01, 150.0);
        let b = a + Vec3::Y * 0.02;
        assert_eq!(world.zone_at(a), Some(179));
        assert_eq!(world.zone_at(b), Some(210));
        let wa = world.environment_weights(a);
        let wb = world.environment_weights(b);
        assert_eq!(wa.len(), 2);
        assert_eq!(wb.len(), 2);
        for zone in [179, 210] {
            let value = |weights: &[(u16, f32)]| {
                let sum: f32 = weights.iter().map(|(_, w)| w).sum();
                weights.iter().find(|(z, _)| *z == zone).unwrap().1 / sum
            };
            assert!((value(&wa) - value(&wb)).abs() < 0.001);
        }
    }
}
