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

    pub fn zone_name(&self, id: u16) -> Option<&str> {
        self.zones.iter().find(|z| z.id == id).map(|z| z.name.as_str())
    }
}

/// `cells/XXX_YYY` directory name of a cell.
pub fn cell_dir(world: &str, x: i32, y: i32) -> String {
    format!("worlds/{world}/cells/{x:03}_{y:03}")
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
}
