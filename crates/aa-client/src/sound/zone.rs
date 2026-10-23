//! The sound pack (`soundmood_*`) of the place the player stands in.
//!
//! A zone group (`zone_groups.sound_pack_id`) has the music of its land; a sub zone (`sub_zones.sound_pack_id`:
//! towns, camps, housing areas, dungeon rooms) has its own. The sub zone is the smallest `AreaShape` polygon around the
//! player (the pak's `level_design/zone/*/client/subzone_area.xml`, as the HUD finds the sub zone name), and it wins over
//! the zone, going up its parents until one has a pack. The original's music manager reads the two in that order too
//! (`X2::GameClient::MusicManager`, `FUN_39052be0`: the tracks `music_01`, `music_02` ... of the sub zone's pack, and when it
//! has none the zone group's), so a place whose sub zone has a pack without music still plays its land's.
//!
//! [`ZonePacks`] holds the packs of the place under the listener, worked out twice a second for the music director and the
//! moods.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use aa_audio::{SoundPacks, SpatialListener};
use bevy::prelude::*;

use super::Packs;
use crate::Screen;

/// A sub zone outline.
struct Shape {
    id: u32,
    /// west, south, east, north (game metres)
    bounds: [f32; 4],
    points: Vec<[f32; 2]>,
}

impl Shape {
    fn contains(&self, x: f32, y: f32) -> bool {
        let b = self.bounds;
        if x < b[0] || x > b[2] || y < b[1] || y > b[3] {
            return false;
        }
        let mut inside = false;
        let mut j = self.points.len() - 1;
        for i in 0..self.points.len() {
            let (pi, pj) = (self.points[i], self.points[j]);
            if (pi[1] > y) != (pj[1] > y) && x < (pj[0] - pi[0]) * (y - pi[1]) / (pj[1] - pi[1]) + pi[0] {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    fn area(&self) -> f32 {
        (self.bounds[2] - self.bounds[0]) * (self.bounds[3] - self.bounds[1])
    }
}

/// zone -> group -> pack, sub zone -> parent and pack, and the sub zone outlines.
#[derive(Default)]
pub struct ZoneData {
    /// `zones.zone_key` -> `zone_groups.id`
    zone_group: HashMap<u32, u32>,
    /// `zone_groups.id` -> pack name
    group_pack: HashMap<u32, String>,
    /// `sub_zones.id` -> (parent, pack name)
    sub: HashMap<u32, (u32, Option<String>)>,
    shapes: Vec<Shape>,
}

impl ZoneData {
    fn load() -> rusqlite::Result<ZoneData> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let mut data = ZoneData::default();
        let mut q = conn.prepare("SELECT zone_key, group_id FROM zones WHERE group_id IS NOT NULL")?;
        for r in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
            let (k, g) = r?;
            data.zone_group.insert(k, g);
        }
        let mut q = conn.prepare("SELECT g.id, p.name FROM zone_groups g JOIN sound_packs p ON p.id = g.sound_pack_id")?;
        for r in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
            let (g, p) = r?;
            data.group_pack.insert(g, p);
        }
        let mut q = conn.prepare("SELECT s.id, s.parent_sub_zone_id, p.name FROM sub_zones s LEFT JOIN sound_packs p ON p.id = s.sound_pack_id")?;
        for r in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Option<u32>>(1)?, r.get::<_, Option<String>>(2)?)))? {
            let (id, parent, pack) = r?;
            data.sub.insert(id, (parent.unwrap_or(0), pack));
        }
        data.load_shapes();
        Ok(data)
    }

    /// The outlines of `main_world`'s sub zones (about 90 small files in the pak).
    fn load_shapes(&mut self) {
        let pak = aa_pak::Pak::global();
        let world = "worlds/main_world";
        let Ok(world_xml) = pak.read_string(&format!("{world}/world.xml")) else { return };
        let mut origins: HashMap<u32, (f32, f32)> = HashMap::new();
        for tag in world_xml.split("<Zone ").skip(1) {
            let tag = format!(" {}", tag.split('>').next().unwrap_or(""));
            if let (Some(id), Some(x), Some(y)) = (attr(&tag, "id"), attr(&tag, "originX"), attr(&tag, "originY"))
                && let (Ok(id), Ok(x), Ok(y)) = (id.parse(), x.parse(), y.parse())
            {
                origins.insert(id, (x, y));
            }
        }
        let prefix = format!("{world}/level_design/zone/");
        let files: Vec<String> = pak.list(&prefix).filter(|p| p.ends_with("/client/subzone_area.xml")).map(str::to_string).collect();
        for file in files {
            let zone: Option<u32> = file.split("level_design/zone/").nth(1).and_then(|r| r.split('/').next()).and_then(|z| z.parse().ok());
            let Some(origin) = zone.and_then(|z| origins.get(&z)).copied() else { continue };
            if let Ok(text) = pak.read_string(&file) {
                parse_shapes(&text, origin, &mut self.shapes);
            }
        }
    }

    /// The packs at a game position (x east, y north), the most specific first: the sub zone's, going up its parents, then the
    /// zone's. `zone_key` is the zone under the position, `group` the zone group the server last told (a fallback).
    pub fn packs_at(&self, x: f32, y: f32, zone_key: Option<u32>, group: Option<u32>) -> Vec<&str> {
        let mut out = Vec::new();
        let shape = self.shapes.iter().filter(|s| s.contains(x, y)).min_by(|a, b| a.area().total_cmp(&b.area()));
        let mut id = shape.map(|s| s.id);
        for _ in 0..8 {
            let Some(i) = id else { break };
            let Some((parent, pack)) = self.sub.get(&i) else { break };
            if let Some(p) = pack {
                out.push(p.as_str());
                break;
            }
            id = (*parent != 0 && *parent != i).then_some(*parent);
        }
        let zone = zone_key.and_then(|k| self.zone_group.get(&k).copied()).or(group).and_then(|g| self.group_pack.get(&g));
        if let Some(z) = zone
            && !out.contains(&z.as_str())
        {
            out.push(z);
        }
        out
    }

    /// The most specific pack at a position.
    pub fn pack_at(&self, x: f32, y: f32, zone_key: Option<u32>, group: Option<u32>) -> Option<&str> {
        self.packs_at(x, y, zone_key, group).into_iter().next()
    }
}

/// The sound packs of the place under the listener, most specific first (empty outside the game and until the zone data is in).
#[derive(Resource, Default, Debug)]
pub(super) struct ZonePacks {
    pub packs: Vec<String>,
    next_check: f64,
}

/// Looks up the packs of the listener's place twice a second.
pub(super) fn track_zone_packs(
    zones: Res<ZoneLoad>,
    screen: Res<State<Screen>>,
    time: Res<Time>,
    net_world: Option<Res<aa_net::state::WorldInfo>>,
    world: Option<Res<aa_world::stream::WorldInfo>>,
    listener: Query<&GlobalTransform, With<SpatialListener>>,
    mut out: ResMut<ZonePacks>,
) {
    if *screen.get() != Screen::InGame {
        if !out.packs.is_empty() {
            out.packs.clear();
        }
        return;
    }
    let now = time.elapsed_secs_f64();
    if now < out.next_check {
        return;
    }
    out.next_check = now + 0.5;
    let (Some(zones), Some(ear)) = (zones.get(), listener.iter().next().map(|t| aa_world::bevy_to_cry(t.translation()))) else { return };
    let zone_key = world.as_deref().and_then(|w| w.0.zone_at(ear)).map(u32::from).or_else(|| net_world.as_deref().map(|w| w.zone_key));
    let group = net_world.as_deref().and_then(|w| w.zone_group_id);
    let packs: Vec<String> = zones.packs_at(ear.x, ear.y, zone_key, group).into_iter().map(str::to_string).collect();
    if packs != out.packs {
        if super::log_enabled() {
            info!("audio: zone sound packs {:?} -> {:?}", out.packs, packs);
        }
        out.packs = packs;
    }
}

fn attr<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = text.find(&key)? + key.len();
    let end = text[at..].find('"')?;
    Some(&text[at..at + end])
}

fn pair(text: &str) -> Option<(f32, f32)> {
    let mut it = text.split(',');
    Some((it.next()?.trim().parse().ok()?, it.next()?.trim().parse().ok()?))
}

/// One `AreaShape` entity per polygon: the points relative to the entity, the entity relative to its cell in the zone, the zone
/// to its origin cell in `world.xml` (cells are 1024 m). The area's `Id` is the sub zone id.
fn parse_shapes(text: &str, origin: (f32, f32), out: &mut Vec<Shape>) {
    for entity in text.split("<Entity ").skip(1) {
        let (Some(pos), Some(cx), Some(cy)) = (attr(entity, "Pos").and_then(pair), attr(entity, "cellX"), attr(entity, "cellY")) else { continue };
        let (Ok(cx), Ok(cy)) = (cx.parse::<f32>(), cy.parse::<f32>()) else { continue };
        let Some(id) = entity.find("<Area Id=\"").and_then(|a| entity[a + 10..].split('"').next()).and_then(|v| v.parse::<u32>().ok()) else { continue };
        let (gx, gy) = ((origin.0 + cx) * 1024.0 + pos.0, (origin.1 + cy) * 1024.0 + pos.1);
        let points: Vec<[f32; 2]> = entity.split("<Point Pos=\"").skip(1).filter_map(|p| pair(p.split('"').next()?)).map(|(x, y)| [gx + x, gy + y]).collect();
        if points.len() < 3 {
            continue;
        }
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &points {
            bounds = [bounds[0].min(p[0]), bounds[1].min(p[1]), bounds[2].max(p[0]), bounds[3].max(p[1])];
        }
        out.push(Shape { id, bounds, points });
    }
}

/// The sound database and the zone data, loading on a thread.
#[derive(Resource)]
pub struct ZoneLoad(Arc<OnceLock<ZoneData>>);

impl ZoneLoad {
    /// Starts reading the sound packs into `packs` and the zone data, when `wanted`.
    pub fn start(wanted: bool, packs: Packs) -> ZoneLoad {
        let zones = Arc::new(OnceLock::new());
        if wanted {
            let z = zones.clone();
            let started = std::time::Instant::now();
            std::thread::Builder::new()
                .name("sound-db".into())
                .spawn(move || {
                    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
                    match SoundPacks::open(&path) {
                        Ok(p) => {
                            let _ = packs.0.set(p);
                        }
                        Err(e) => warn!("sound: cannot read the sound packs from {path}: {e}"),
                    }
                    match ZoneData::load() {
                        Ok(d) => {
                            info!("sound: sound packs and {} sub zone outlines read in {:.0} ms", d.shapes.len(), started.elapsed().as_secs_f32() * 1000.0);
                            let _ = z.set(d);
                        }
                        Err(e) => warn!("sound: cannot read the zone sound data: {e}"),
                    }
                })
                .ok();
        }
        ZoneLoad(zones)
    }

    pub fn get(&self) -> Option<&ZoneData> {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_zone_pack_wins_over_the_zone() {
        let mut d = ZoneData::default();
        d.zone_group.insert(179, 5);
        d.group_pack.insert(5, "soundmood_w_solzreed".into());
        // a town (pack) with a district (no pack) in it, and a plain sub zone
        d.sub.insert(10, (0, Some("soundmood_town".into())));
        d.sub.insert(11, (10, None));
        d.sub.insert(12, (0, None));
        let square = |id, x0: f32, y0: f32, x1: f32, y1: f32| Shape { id, bounds: [x0, y0, x1, y1], points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]] };
        d.shapes = vec![square(10, 0.0, 0.0, 100.0, 100.0), square(11, 40.0, 40.0, 60.0, 60.0), square(12, 200.0, 0.0, 300.0, 100.0)];
        assert_eq!(d.pack_at(50.0, 50.0, Some(179), None), Some("soundmood_town"));
        assert_eq!(d.pack_at(10.0, 10.0, Some(179), None), Some("soundmood_town"));
        // a sub zone without a pack and without a parent's: the zone's
        assert_eq!(d.pack_at(250.0, 50.0, Some(179), None), Some("soundmood_w_solzreed"));
        // outside every outline: the zone's, by the key or by the server's group
        assert_eq!(d.pack_at(500.0, 500.0, Some(179), None), Some("soundmood_w_solzreed"));
        assert_eq!(d.pack_at(500.0, 500.0, None, Some(5)), Some("soundmood_w_solzreed"));
        assert_eq!(d.pack_at(500.0, 500.0, None, None), None);
    }

    #[test]
    fn both_packs_are_listed_most_specific_first() {
        let mut d = ZoneData::default();
        d.zone_group.insert(179, 5);
        d.group_pack.insert(5, "soundmood_w_solzreed".into());
        d.sub.insert(10, (0, Some("soundmood_town".into())));
        d.sub.insert(12, (0, Some("soundmood_w_solzreed".into())));
        let square = |id, x0: f32, y0: f32, x1: f32, y1: f32| Shape { id, bounds: [x0, y0, x1, y1], points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]] };
        d.shapes = vec![square(10, 0.0, 0.0, 100.0, 100.0), square(12, 200.0, 0.0, 300.0, 100.0)];
        // a town: its pack, then the land's (the music of the land plays when the town has none of its own)
        assert_eq!(d.packs_at(50.0, 50.0, Some(179), None), ["soundmood_town", "soundmood_w_solzreed"]);
        // the same pack twice is once
        assert_eq!(d.packs_at(250.0, 50.0, Some(179), None), ["soundmood_w_solzreed"]);
        assert_eq!(d.packs_at(500.0, 500.0, Some(179), None), ["soundmood_w_solzreed"]);
        assert!(d.packs_at(500.0, 500.0, None, None).is_empty());
    }

    #[test]
    fn outlines_are_read() {
        let xml = r#"<Objects><Entity Name="a" Pos="10,20,0" cellX="1" cellY="0"><Area Id="77"><Points>
            <Point Pos="0,0,0"/><Point Pos="10,0,0"/><Point Pos="10,10,0"/></Points></Area></Entity></Objects>"#;
        let mut out = Vec::new();
        parse_shapes(xml, (2.0, 3.0), &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, 77);
        assert_eq!(out[0].points[0], [3.0 * 1024.0 + 10.0, 3.0 * 1024.0 + 20.0]);
        assert!(out[0].contains(3.0 * 1024.0 + 18.0, 3.0 * 1024.0 + 23.0));
    }
}
