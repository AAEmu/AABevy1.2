//! Equipment attachment sweep: dresses the eight playable models with one item at a time and looks
//! for items that do not sit right on the body. Two modes.
//!
//! ```text
//! # 1. static analysis, no window: every distinct visual of every slot on every model
//! cargo run -p aa-anim --example equip_sweep --release -- --analyze [--slot head] [--model 10] [--out DIR]
//!     writes DIR/findings.tsv (one line per problem) and DIR/totals.tsv (probes per slot and model);
//!     DIR defaults to data/extract/aa-anim/equip_sweep
//!
//! # 2. contact sheets (needs a GPU window; >= 6 GB RAM free)
//! cargo run -p aa-anim --example equip_sweep --release -- --sheets [--slot head] [--model 10] [--pose idle|sit|leap]
//!     [--n 32] [--skip 0] [--cols 8] [--poses idle,action,sit] [--items id,id,..] [--frame centre_y,height,cell_width] [--out shots/aa-anim/equip/_raw]
//! ```
//!
//! The analysis emulates what the renderer does with an item: a `.chr` part is skinned to the base
//! skeleton of the race by controller id, bones the base does not have get a joint under their
//! parent (or at the model root when no ancestor is shared), and the part uses the base bind poses
//! for the bones it shares (`aa_cry::loader::prepare_model`, `scene::spawn_loaded_full`). The
//! vertices are skinned on the CPU in the bind pose and in animated poses and compared with the
//! nude body, face and hair of the same model.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use aa_anim::cdf::{Attachment, AttachmentKind, BindingKind};
use aa_anim::db::model_id;
use aa_anim::pose::{Rig, SampleOptions, shared_rig};
use aa_anim::{Cal, Cdf, CharacterDesc, Costume, GameDb, Gender, Pose, Race};
use aa_cry::material::{Material, TextureMap};
use aa_cry::mesh::Model;
use aa_cry::skeleton::Skeleton;
use glam::{Affine3A, Vec3};
use rusqlite::{Connection, OpenFlags};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn main() {
    if flag("--sheets") {
        sheets::run();
    } else {
        analysis::run();
    }
}

// ---------------------------------------------------------------------------------------------
// catalogue: which items go in which slot
// ---------------------------------------------------------------------------------------------

/// The bone-less description of where an equipment slot sits on a body, as fractions of body height
/// (model space, z up) for the centre of the item's bounding box.
#[derive(Clone, Copy, Debug)]
struct Slot {
    key: &'static str,
    kind: SlotKind,
    /// Expected `[lo, hi]` of the item bounding box centre height / body height.
    region: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotKind {
    /// `item_armors.slot_type_id`
    Armor(i64),
    /// Weapon class: `item_weapons` of this holdable class worn in an equipment slot.
    Weapon(Class),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// holdable slot 14 (maces, spears, wands) in the main hand
    Main1h,
    /// holdable slot 16 (two handed)
    TwoHand,
    /// holdable slot 17 (swords, axes ...) in the main hand
    SwordMain,
    /// the same swords in the off hand
    SwordOff,
    /// holdable slot 18
    Bow,
    /// holdable slot 20
    Shield,
    /// holdable slot 21 by `holdables.code`
    String,
    Percussion,
    Tube,
}

const SLOTS: [Slot; 18] = [
    Slot { key: "head", kind: SlotKind::Armor(1), region: (0.80, 1.10) },
    Slot { key: "chest", kind: SlotKind::Armor(3), region: (0.55, 0.95) },
    Slot { key: "waist", kind: SlotKind::Armor(4), region: (0.45, 0.70) },
    Slot { key: "legs", kind: SlotKind::Armor(5), region: (0.05, 0.60) },
    Slot { key: "hands", kind: SlotKind::Armor(6), region: (0.30, 0.75) },
    Slot { key: "feet", kind: SlotKind::Armor(7), region: (0.00, 0.20) },
    Slot { key: "back", kind: SlotKind::Armor(9), region: (0.30, 1.00) },
    Slot { key: "costume", kind: SlotKind::Armor(31), region: (0.30, 0.75) },
    Slot { key: "underwear", kind: SlotKind::Armor(13), region: (0.30, 0.75) },
    Slot { key: "main1h", kind: SlotKind::Weapon(Class::Main1h), region: (0.35, 0.80) },
    Slot { key: "twohand", kind: SlotKind::Weapon(Class::TwoHand), region: (0.35, 1.10) },
    Slot { key: "sword_main", kind: SlotKind::Weapon(Class::SwordMain), region: (0.35, 0.80) },
    Slot { key: "sword_off", kind: SlotKind::Weapon(Class::SwordOff), region: (0.35, 0.80) },
    Slot { key: "bow", kind: SlotKind::Weapon(Class::Bow), region: (0.40, 1.00) },
    Slot { key: "shield", kind: SlotKind::Weapon(Class::Shield), region: (0.40, 0.95) },
    Slot { key: "string", kind: SlotKind::Weapon(Class::String), region: (0.30, 1.00) },
    Slot { key: "percussion", kind: SlotKind::Weapon(Class::Percussion), region: (0.30, 1.00) },
    Slot { key: "tube", kind: SlotKind::Weapon(Class::Tube), region: (0.30, 1.00) },
];

/// One item in a slot (one per distinct visual).
#[derive(Clone, Debug)]
struct Item {
    id: u32,
    name: String,
    level: i64,
    grade: i64,
    /// armour material (1 cloth, 2 leather, 3 plate ...) or the holdable code
    flavour: String,
    /// number of items that share this visual
    shared: usize,
    weapon: Option<WeaponRow>,
}

#[derive(Clone, Debug)]
struct WeaponRow {
    path: String,
    code: String,
    offset: [f32; 3],
    rotation: [f32; 3],
    worn_scale: f32,
    drawn_scale: f32,
    two_handed: bool,
    staff: bool,
}

fn open_db() -> Connection {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).expect("game database")
}

fn is_npc(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("npc") || n.contains("npc ") || n.starts_with("- ") || n.starts_with("event_")
}

/// Every distinct visual of a slot, one representative item each.
fn catalogue(conn: &Connection, slot: &Slot) -> Vec<Item> {
    let mut groups: BTreeMap<String, Vec<Item>> = BTreeMap::new();
    match slot.kind {
        SlotKind::Armor(st) => {
            let mut stmt = conn
                .prepare(
                    "SELECT a.item_id, i.name, i.LEVEL, i.fixed_grade, a.type_id, a.asset_id FROM item_armors a JOIN items i ON i.id = a.item_id \
                     WHERE a.slot_type_id = ?1 AND EXISTS (SELECT 1 FROM item_armor_assets x WHERE x.armor_asset_id = a.asset_id)",
                )
                .unwrap();
            let rows = stmt
                .query_map([st], |r| {
                    Ok((
                        Item {
                            id: r.get(0)?,
                            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            level: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            grade: r.get::<_, Option<i64>>(3)?.unwrap_or(-1),
                            flavour: r.get::<_, Option<i64>>(4)?.unwrap_or(0).to_string(),
                            shared: 1,
                            weapon: None,
                        },
                        r.get::<_, i64>(5)?,
                    ))
                })
                .unwrap();
            for (item, asset) in rows.flatten() {
                groups.entry(asset.to_string()).or_default().push(item);
            }
        }
        SlotKind::Weapon(class) => {
            let (hslot, code) = match class {
                Class::Main1h => (14, None),
                Class::TwoHand => (16, None),
                Class::SwordMain | Class::SwordOff => (17, None),
                Class::Bow => (18, None),
                Class::Shield => (20, None),
                Class::String => (21, Some("string_instrument")),
                Class::Percussion => (21, Some("percussion_instrument")),
                Class::Tube => (21, Some("tube_instrument")),
            };
            let mut stmt = conn
                .prepare(
                    "SELECT w.item_id, i.name, i.LEVEL, i.fixed_grade, h.code, ia.path, \
                     ia.attachment_offset_pos_x, ia.attachment_offset_pos_y, ia.attachment_offset_pos_z, \
                     ia.attachment_offset_rot_x, ia.attachment_offset_rot_y, ia.attachment_offset_rot_z, \
                     w.worn_scale, w.drawn_scale, h.slot_type_id, h.pose_id \
                     FROM item_weapons w JOIN items i ON i.id = w.item_id JOIN item_assets ia ON ia.id = w.asset_id \
                     JOIN holdables h ON h.id = w.holdable_id WHERE h.slot_type_id = ?1",
                )
                .unwrap();
            let rows = stmt
                .query_map([hslot], |r| {
                    let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                    let path = r.get::<_, String>(5)?.replace('\\', "/").to_ascii_lowercase();
                    let hcode = r.get::<_, Option<String>>(4)?.unwrap_or_default();
                    Ok((
                        Item {
                            id: r.get(0)?,
                            name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                            level: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                            grade: r.get::<_, Option<i64>>(3)?.unwrap_or(-1),
                            flavour: hcode.clone(),
                            shared: 1,
                            weapon: Some(WeaponRow {
                                path: path.clone(),
                                code: hcode,
                                offset: [f(6)?, f(7)?, f(8)?],
                                rotation: [f(9)?, f(10)?, f(11)?],
                                worn_scale: f(12)?,
                                drawn_scale: f(13)?,
                                two_handed: r.get::<_, Option<i64>>(14)?.unwrap_or(0) == 16,
                                staff: r.get::<_, Option<i64>>(15)?.unwrap_or(0) == 4,
                            }),
                        },
                        path,
                    ))
                })
                .unwrap();
            for (item, path) in rows.flatten() {
                if code.is_some_and(|c| item.weapon.as_ref().is_some_and(|w| w.code != c)) {
                    continue;
                }
                groups.entry(path).or_default().push(item);
            }
        }
    }
    let mut out = Vec::new();
    for (_, mut items) in groups {
        let shared = items.len();
        // prefer a real player item (not an npc / event placeholder) with a level, then the lowest id
        items.sort_by_key(|i| (is_npc(&i.name), i.level <= 0, i.id));
        let mut rep = items.swap_remove(0);
        rep.shared = shared;
        out.push(rep);
    }
    out.sort_by_key(|i| (i.flavour.clone(), i.level, i.id));
    out
}

/// `n` items spread over flavour (cloth / leather / plate, weapon code), level and grade: evenly spaced picks of the
/// catalogue sorted by (flavour, level), plus the highest graded items and the ones that are part of a set.
fn sample(conn: &Connection, items: &[Item], n: usize) -> Vec<Item> {
    let mut real: Vec<&Item> = items.iter().filter(|i| !is_npc(&i.name)).collect();
    if real.len() < n {
        real = items.iter().collect();
    }
    let sets: Vec<u32> = conn
        .prepare("SELECT DISTINCT item_id FROM item_set_items")
        .and_then(|mut s| s.query_map([], |r| r.get(0)).map(|r| r.flatten().collect()))
        .unwrap_or_default();
    let mut picked: Vec<Item> = Vec::new();
    let take = |i: &Item, picked: &mut Vec<Item>| {
        if !picked.iter().any(|p| p.id == i.id) {
            picked.push(i.clone());
        }
    };
    // 3 graded items, 3 set items
    for i in real.iter().filter(|i| i.grade >= 2).step_by((real.len() / 40).max(1)).take(3) {
        take(i, &mut picked);
    }
    for i in real.iter().filter(|i| sets.contains(&i.id)).take(3) {
        take(i, &mut picked);
    }
    let rest = n.saturating_sub(picked.len());
    if !real.is_empty() {
        for k in 0..rest {
            let idx = if rest > 1 { k * (real.len() - 1) / (rest - 1) } else { 0 };
            take(real[idx], &mut picked);
        }
        // duplicates collapsed above: top up from the middle of the list
        let mut j = 1;
        while picked.len() < n.min(real.len()) && j < real.len() {
            take(real[(j * 7) % real.len()], &mut picked);
            j += 1;
        }
    }
    picked.sort_by_key(|i| (i.flavour.clone(), i.level, i.id));
    picked
}

fn models() -> Vec<(Race, Gender, u32)> {
    Race::ALL.iter().flat_map(|&r| Gender::ALL.iter().map(move |&g| (r, g, model_id(r, g)))).collect()
}

fn model_label(model: u32) -> String {
    match aa_anim::Race::from_model_id(model) {
        Some((r, g)) => format!("{}_{}", r.prefix(), g.letter()),
        None => model.to_string(),
    }
}

// ---------------------------------------------------------------------------------------------
// static analysis
// ---------------------------------------------------------------------------------------------

mod analysis {
    use super::*;

    /// Poses the parts are skinned in: bind, the in-game idle, sitting down, a leap attack.
    const POSES: [(&str, f32); 3] = [("idle", 0.0), ("sit", 0.6), ("leap", 0.4)];
    const CLIPS: [&str; 3] = ["fist_ba_relaxed_idle", "fist_ac_sit_down_loop", "all_co_sk_leapattack"];

    pub struct Finding {
        pub cat: &'static str,
        pub severity: &'static str,
        pub slot: &'static str,
        pub model: u32,
        pub item: u32,
        pub name: String,
        pub level: i64,
        pub npc: bool,
        pub path: String,
        pub detail: String,
    }

    #[derive(Default, Clone)]
    pub struct Totals {
        pub items: usize,
        pub items_shared: usize,
        pub npc_items: usize,
        pub with_visual: usize,
        pub parts: usize,
        pub failed: usize,
    }

    /// Reference geometry (nude body, face, hair) of a model in every pose.
    struct Reference {
        /// vertices per pose, index 0 = bind
        pose: Vec<Vec<Vec3>>,
        grid: HashMap<(i32, i32, i32), Vec<u32>>,
        min: Vec3,
        max: Vec3,
    }

    const CELL: f32 = 0.08;

    fn cell(p: Vec3) -> (i32, i32, i32) {
        ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32, (p.z / CELL).floor() as i32)
    }

    impl Reference {
        /// Mean displacement in pose `pi` of the reference vertices around `p` on the nearest surface (within 3 cm of the nearest one).
        fn local_displacement(&self, p: Vec3, pi: usize, nearest: f32) -> Option<Vec3> {
            let c = cell(p);
            let (mut sum, mut n) = (Vec3::ZERO, 0.0f32);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(v) = self.grid.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                            for &i in v {
                                let d = self.pose[0][i as usize].distance(p);
                                if d <= nearest + 0.03 {
                                    sum += self.pose[pi][i as usize] - self.pose[0][i as usize];
                                    n += 1.0;
                                }
                            }
                        }
                    }
                }
            }
            (n > 0.0).then(|| sum / n)
        }

        /// Index and distance of the reference vertex nearest to `p` within `max_r`.
        fn nearest(&self, p: Vec3, max_r: f32) -> Option<(usize, f32)> {
            if !p.is_finite() || p.abs().max_element() > 1000.0 {
                return None;
            }
            let c = cell(p);
            let rings = (max_r / CELL).ceil() as i32;
            let mut best: Option<(usize, f32)> = None;
            for dx in -rings..=rings {
                for dy in -rings..=rings {
                    for dz in -rings..=rings {
                        if let Some(v) = self.grid.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                            for &i in v {
                                let d = self.pose[0][i as usize].distance(p);
                                if d <= max_r && best.is_none_or(|(_, b)| d < b) {
                                    best = Some((i as usize, d));
                                }
                            }
                        }
                    }
                }
            }
            best
        }
    }

    pub struct Base {
        pub model: u32,
        skel: Arc<Skeleton>,
        /// world matrices of the base bones per pose (0 = bind)
        worlds: Vec<Vec<Affine3A>>,
        pose_names: Vec<&'static str>,
        reference: Reference,
        pub height: f32,
        pub db: GameDb,
        pub body_path: String,
        /// triangles of the nude body in model space (authored positions, like the Cut hulls)
        body_tris: Vec<[Vec3; 3]>,
        pub race: Race,
        pub gender: Gender,
    }

    fn worlds_of(rig: &Rig, pose: &Pose) -> Vec<Affine3A> {
        pose.model_space(rig).into_iter().map(|(r, t)| Affine3A::from_rotation_translation(r, t)).collect()
    }

    /// Skinning matrices of a part's own bones (see the module comment).
    fn bone_mats(own: &Skeleton, base: &Skeleton, map: &[Option<usize>], base_w: &[Affine3A]) -> Vec<Affine3A> {
        let mut w: Vec<Affine3A> = Vec::with_capacity(own.len());
        let mut m = Vec::with_capacity(own.len());
        for (i, b) in own.bones.iter().enumerate() {
            let wi = match map[i] {
                Some(j) => base_w[j],
                None => match b.parent {
                    Some(p) => w[p] * b.bind_local,
                    None => b.bind_local,
                },
            };
            let ibp = match map[i] {
                Some(j) => base.bones[j].bind_model.inverse(),
                None => b.bind_model.inverse(),
            };
            m.push(wi * ibp);
            w.push(wi);
        }
        m
    }

    #[derive(Default)]
    pub struct Geo {
        authored: Vec<Vec3>,
        /// skinned positions per pose (0 = bind)
        pose: Vec<Vec<Vec3>>,
        edges: Vec<(u32, u32)>,
        /// sub-material id of every triangle (the triangles are `edges` in pairs: (a,b), (b,c))
        tri_mat: Vec<u32>,
        skinned_verts: usize,
        rigid_verts: usize,
        meshes: usize,
        /// weight share on bones that are in the base skeleton / ride an ancestor / have no shared ancestor
        w_total: f32,
        w_ride: f32,
        w_orphan: f32,
        bone_w: HashMap<String, (f32, bool)>,
        shift_max: f32,
        shift_n: usize,
        nonfinite: usize,
        /// used sub-material ids
        used_mats: Vec<u32>,
    }

    fn skin_part(base: &Base, model: &Model, path: &str) -> Geo {
        let mut g = Geo { pose: vec![Vec::new(); base.worlds.len()], ..Default::default() };
        let own: Option<Skeleton> = model.skeleton.clone().or_else(|| if model.is_skinned() { Skeleton::load(path).ok() } else { None });
        let map = own.as_ref().map(|s| s.map_to(&base.skel));
        let mats: Option<Vec<Vec<Affine3A>>> = match (&own, &map) {
            (Some(o), Some(m)) => Some(base.worlds.iter().map(|w| bone_mats(o, &base.skel, m, w)).collect()),
            _ => None,
        };
        // 0 mapped, 1 rides an ancestor that is mapped, 2 orphan
        let class: Vec<u8> = match (&own, &map) {
            (Some(o), Some(m)) => (0..o.len())
                .map(|i| {
                    if m[i].is_some() {
                        return 0;
                    }
                    let mut p = o.bones[i].parent;
                    while let Some(pi) = p {
                        if m[pi].is_some() {
                            return 1;
                        }
                        p = o.bones[pi].parent;
                    }
                    2
                })
                .collect(),
            _ => Vec::new(),
        };
        for ni in model.lod_nodes(0) {
            let Some(mi) = model.nodes[ni].mesh else { continue };
            let mesh = &model.meshes[mi];
            if model.skeleton.is_some() && mesh.indices.len() <= 3 {
                continue;
            }
            g.meshes += 1;
            g.nonfinite += mesh.positions.iter().filter(|p| !p.is_finite() || p.abs().max_element() > 1000.0).count();
            for s in &mesh.subsets {
                if s.index_count > 0 && !g.used_mats.contains(&s.material_id) {
                    g.used_mats.push(s.material_id);
                }
            }
            let first = g.authored.len() as u32;
            let skinned = mesh.skin.is_some() && mats.is_some();
            if skinned {
                let (mats, skin) = (mats.as_ref().unwrap(), mesh.skin.as_ref().unwrap());
                let own = own.as_ref().unwrap();
                for (v, &p) in mesh.positions.iter().enumerate() {
                    g.authored.push(p);
                    let (js, ws) = (skin.joints[v], skin.weights[v]);
                    for (pi, m) in mats.iter().enumerate() {
                        let mut acc = Vec3::ZERO;
                        let mut wsum = 0.0;
                        for k in 0..4 {
                            let (j, w) = (js[k] as usize, ws[k]);
                            if w <= 0.0 || j >= m.len() {
                                continue;
                            }
                            acc += w * m[j].transform_point3(p);
                            wsum += w;
                        }
                        g.pose[pi].push(if wsum > 0.0 { acc / wsum } else { p });
                    }
                    for k in 0..4 {
                        let (j, w) = (js[k] as usize, ws[k]);
                        if w <= 0.0 || j >= class.len() {
                            continue;
                        }
                        g.w_total += w;
                        match class[j] {
                            1 => g.w_ride += w,
                            2 => g.w_orphan += w,
                            _ => {}
                        }
                        if class[j] != 0 {
                            let e = g.bone_w.entry(own.bones[j].name.clone()).or_insert((0.0, class[j] == 2));
                            e.0 += w;
                        }
                    }
                    let d = g.pose[0].last().unwrap().distance(p);
                    g.shift_max = g.shift_max.max(d);
                    if d > 0.02 {
                        g.shift_n += 1;
                    }
                }
                g.skinned_verts += mesh.positions.len();
            } else {
                let tm = model.world_transform(ni);
                for &p in &mesh.positions {
                    let q = tm.transform_point3(p);
                    g.authored.push(q);
                    for pose in g.pose.iter_mut() {
                        pose.push(q);
                    }
                }
                g.rigid_verts += mesh.positions.len();
            }
            let mut mats = vec![u32::MAX; mesh.indices.len() / 3];
            for sub in &mesh.subsets {
                for t in (sub.first_index / 3) as usize..((sub.first_index + sub.index_count) / 3) as usize {
                    if let Some(m) = mats.get_mut(t) {
                        *m = sub.material_id;
                    }
                }
            }
            g.tri_mat.extend(mats);
            for t in mesh.indices.chunks_exact(3) {
                g.edges.push((first + t[0], first + t[1]));
                g.edges.push((first + t[1], first + t[2]));
            }
        }
        g
    }

    pub fn build_base(race: Race, gender: Gender) -> Base {
        let db = GameDb::open().expect("game database");
        let mut desc = CharacterDesc::default_for(&db, race, gender).expect("default look");
        desc.costume = Costume::None;
        let assets = desc.resolve(&db).expect("nude character");
        let rig = shared_rig(&assets.skeleton).expect("skeleton");
        let cal = Cal::shared(&assets.cal).expect("cal");
        let clips = aa_anim::ClipLibrary::default();
        let mut poses = vec![rig.bind.clone()];
        let mut names = vec!["bind"];
        for (i, name) in CLIPS.iter().enumerate() {
            let Some(entry) = cal.animation(name) else { continue };
            let opts = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: true };
            let Ok(bc) = clips.bound(&rig, &entry.path, opts) else { continue };
            let mut p = rig.bind.clone();
            p.sample(&rig, &bc.clip, &bc.binding, bc.clip.duration * POSES[i].1, bc.options);
            poses.push(p);
            names.push(POSES[i].0);
        }
        let worlds: Vec<Vec<Affine3A>> = poses.iter().map(|p| worlds_of(&rig, p)).collect();
        let mut base = Base {
            model: model_id(race, gender),
            skel: rig.skeleton.clone(),
            worlds,
            pose_names: names,
            reference: Reference { pose: Vec::new(), grid: HashMap::new(), min: Vec3::MAX, max: Vec3::MIN },
            height: 1.8,
            db,
            body_path: String::new(),
            body_tris: Vec::new(),
            race,
            gender,
        };
        // reference cloud: body + face + hair
        let mut pose: Vec<Vec<Vec3>> = vec![Vec::new(); base.worlds.len()];
        for part in &assets.parts {
            if part.kind == aa_anim::PartKind::Costume {
                continue;
            }
            let Ok(model) = Model::load(&part.path) else { continue };
            if part.kind == aa_anim::PartKind::Body {
                base.body_path = part.path.clone();
                for ni in model.lod_nodes(0) {
                    let Some(mi) = model.nodes[ni].mesh else { continue };
                    let mesh = &model.meshes[mi];
                    base.body_tris.extend(mesh.indices.chunks_exact(3).map(|t| [mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]]));
                }
            }
            let g = skin_part(&base, &model, &part.path);
            for (dst, src) in pose.iter_mut().zip(&g.pose) {
                dst.extend_from_slice(src);
            }
        }
        let (mut mn, mut mx) = (Vec3::MAX, Vec3::MIN);
        let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, p) in pose[0].iter().enumerate() {
            mn = mn.min(*p);
            mx = mx.max(*p);
            grid.entry(cell(*p)).or_default().push(i as u32);
        }
        base.height = (mx.z - mn.z).max(1.0);
        base.reference = Reference { pose, grid, min: mn, max: mx };
        base
    }

    /// Does the ray `o + t d` (0 < t < max) cross the triangle? (Moeller-Trumbore, both sides.)
    fn ray_hits(o: Vec3, d: Vec3, t: &[Vec3; 3], max: f32) -> bool {
        let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
        let pv = d.cross(e2);
        let det = e1.dot(pv);
        if det.abs() < 1e-9 {
            return false;
        }
        let inv = 1.0 / det;
        let tv = o - t[0];
        let u = tv.dot(pv) * inv;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let qv = tv.cross(e1);
        let v = d.dot(qv) * inv;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        let dist = e2.dot(qv) * inv;
        dist > 1e-4 && dist < max
    }

    /// Body triangles hidden by the `hulls` of a garment that are not behind any visible garment triangle seen from outside
    /// (a ray from the triangle away from the body axis finds no garment within 30 cm): holes in the body.
    /// Returns (hidden triangles, exposed ones).
    fn cut_holes(base: &Base, hulls: &[aa_anim::cut::Hull], garment: &[[Vec3; 3]]) -> (usize, usize) {
        let mut grid: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (i, t) in garment.iter().enumerate() {
            let (mn, mx) = (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]));
            let (a, b) = (cell(mn), cell(mx));
            if (b.0 - a.0).max(b.1 - a.1).max(b.2 - a.2) > 40 {
                continue;
            }
            for x in a.0..=b.0 {
                for y in a.1..=b.1 {
                    for z in a.2..=b.2 {
                        grid.entry((x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        let r = &base.reference;
        let axis = (r.min + r.max) * 0.5;
        let (mut hidden, mut open) = (0, 0);
        for t in &base.body_tris {
            if !hulls.iter().any(|h| h.contains(t[0]) && h.contains(t[1]) && h.contains(t[2])) {
                continue;
            }
            hidden += 1;
            let c = (t[0] + t[1] + t[2]) / 3.0;
            let mut n = (t[1] - t[0]).cross(t[2] - t[0]);
            if n.length() < 1e-9 {
                continue;
            }
            n = n.normalize();
            // outward: away from the body axis
            if n.dot(Vec3::new(c.x - axis.x, c.y - axis.y, 0.0)) < 0.0 {
                n = -n;
            }
            let mut covered = false;
            let mut seen: Vec<u32> = Vec::new();
            'march: for step in 0..16 {
                let p = c + n * (0.01 + 0.02 * step as f32);
                let k = cell(p);
                if let Some(v) = grid.get(&k) {
                    for &i in v {
                        if seen.contains(&i) {
                            continue;
                        }
                        seen.push(i);
                        if ray_hits(c, n, &garment[i as usize], 0.30) {
                            covered = true;
                            break 'march;
                        }
                    }
                }
            }
            if !covered {
                open += 1;
            }
        }
        (hidden, open)
    }

    fn bbox(points: &[Vec3]) -> (Vec3, Vec3) {
        let (mut mn, mut mx) = (Vec3::MAX, Vec3::MIN);
        for p in points {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        (mn, mx)
    }

    /// Unit direction of the longest extent of a point cloud (power iteration on the covariance).
    fn principal_axis(points: &[Vec3]) -> Vec3 {
        let mean = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
        let mut axis = Vec3::new(0.3, 0.5, 0.8).normalize();
        for _ in 0..24 {
            let mut next = Vec3::ZERO;
            for p in points {
                let d = *p - mean;
                next += d * d.dot(axis);
            }
            if next.length() < 1e-9 {
                break;
            }
            axis = next.normalize();
        }
        axis
    }

    fn pct(v: &mut [f32], q: f32) -> f32 {
        if v.is_empty() {
            return 0.0;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[((v.len() - 1) as f32 * q) as usize]
    }

    pub type Note = (&'static str, &'static str, String);

    /// Judges one skinned part; `(category, severity, detail)`.
    fn judge(base: &Base, slot: &Slot, g: &Geo, has_cut: bool) -> Vec<Note> {
        let mut out: Vec<Note> = Vec::new();
        if g.authored.is_empty() {
            out.push(("no_mesh", "fail", "no LOD0 render mesh".into()));
            return out;
        }
        let h = base.height;
        if g.nonfinite > 0 {
            out.push(("bad_vertices", "fail", format!("{} vertices are NaN or farther than 1 km", g.nonfinite)));
            return out;
        }
        if g.skinned_verts == 0 {
            out.push(("unskinned", "fail", format!("{} rigid vertices, no skin: the part never follows the body", g.rigid_verts)));
        } else if g.rigid_verts > 0 {
            out.push(("rigid_in_skin", "warn", format!("{} rigid vertices next to {} skinned ones", g.rigid_verts, g.skinned_verts)));
        }
        if g.w_total > 0.0 {
            let (orphan, ride) = (g.w_orphan / g.w_total, g.w_ride / g.w_total);
            let names = |orphan: bool| {
                let mut v: Vec<_> = g.bone_w.iter().filter(|(_, (_, o))| *o == orphan).map(|(n, (w, _))| (n.clone(), *w)).collect();
                v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                v.iter().take(4).map(|(n, w)| format!("{n}:{:.0}%", 100.0 * w / g.w_total)).collect::<Vec<_>>().join(",")
            };
            if orphan > 0.02 {
                out.push(("bones_orphan", "fail", format!("{:.0}% of the skin weight on bones missing from the base skeleton with no shared ancestor (stay at the model root): {}", orphan * 100.0, names(true))));
            }
            if ride > 0.02 {
                out.push(("bones_ride", "info", format!("{:.0}% of the skin weight on bones missing from the base skeleton that ride a shared ancestor: {}", ride * 100.0, names(false))));
            }
        }
        if g.skinned_verts > 0 && g.shift_n as f32 > 0.01 * g.skinned_verts as f32 {
            out.push(("bind_shift", "fail", format!("{:.1}% of the vertices sit more than 2 cm from their authored place in the bind pose (max {:.0} cm): the part's bones do not match the base skeleton", 100.0 * g.shift_n as f32 / g.skinned_verts as f32, g.shift_max * 100.0)));
        }
        let (mn, mx) = bbox(&g.pose[0]);
        let size = mx - mn;
        let c = (mn + mx) * 0.5;
        let r = &base.reference;
        let zc = (c.z - r.min.z) / h;
        let (lo, hi) = slot.region;
        let origin = c.length() < 0.15 && size.max_element() < 0.6;
        if origin && slot.key != "feet" {
            out.push(("at_origin", "fail", format!("bounding box centre ({:.2},{:.2},{:.2}) size {:.2}x{:.2}x{:.2} at the model origin", c.x, c.y, c.z, size.x, size.y, size.z)));
        } else if (zc < lo - 0.08 || zc > hi + 0.08) && (size.z < 0.6 * h || mx.z < r.min.z + lo * h || mn.z > r.min.z + hi * h) {
            out.push(("wrong_region", "fail", format!("bbox centre at {:.0}% of the body height, expected {:.0}-{:.0}% for {} (size {:.2}x{:.2}x{:.2})", zc * 100.0, lo * 100.0, hi * 100.0, slot.key, size.x, size.y, size.z)));
        }
        let rc = (r.min + r.max) * 0.5;
        if (c.x - rc.x).abs().max((c.y - rc.y).abs()) > 0.6 {
            out.push(("off_axis", "fail", format!("bbox centre {:.2} m to the side of the body axis", (c.x - rc.x).abs().max((c.y - rc.y).abs()))));
        }
        if size.max_element() > 2.4 || size.max_element() < 0.04 {
            out.push(("bad_size", "fail", format!("bbox size {:.2}x{:.2}x{:.2} m", size.x, size.y, size.z)));
        } else if mn.z < r.min.z - 0.25 || mx.z > r.max.z + 0.6 {
            out.push(("extends_beyond_body", "warn", format!("bbox z {:.2}..{:.2} against the body {:.2}..{:.2}", mn.z, mx.z, r.min.z, r.max.z)));
        }
        // nearest nude body vertex: floating / lag in the poses
        let step = (g.pose[0].len() / 3000).max(1);
        let mut dists: Vec<f32> = Vec::new();
        let mut lag_n = vec![0usize; g.pose.len()];
        let mut lag_bad = vec![0usize; g.pose.len()];
        let mut lag_max = vec![0f32; g.pose.len()];
        for v in (0..g.pose[0].len()).step_by(step) {
            let p = g.pose[0][v];
            match r.nearest(p, 0.32) {
                Some((_, d)) => {
                    dists.push(d);
                    if d < 0.06 {
                        for pi in 1..g.pose.len() {
                            let Some(db) = r.local_displacement(p, pi, d) else { continue };
                            if db.length() < 0.08 {
                                continue;
                            }
                            let di = g.pose[pi][v] - p;
                            let lag = (di - db).length();
                            lag_n[pi] += 1;
                            lag_max[pi] = lag_max[pi].max(lag);
                            if lag > 0.10 {
                                lag_bad[pi] += 1;
                            }
                        }
                    }
                }
                None => dists.push(0.32),
            }
        }
        let p10 = pct(&mut dists.clone(), 0.10);
        let med = pct(&mut dists, 0.5);
        if p10 > 0.08 {
            out.push(("floating", "fail", format!("closest 10% of the vertices are {:.0} cm from the nude body, median {:.0} cm: the part hangs in the air", p10 * 100.0, med * 100.0)));
        }
        for pi in 1..g.pose.len() {
            if lag_n[pi] >= 40 && lag_bad[pi] as f32 > 0.25 * lag_n[pi] as f32 {
                out.push(("pose_lag", "fail", format!("{}: {:.0}% of {} vertices that touch the body lag the body's local motion by more than 10 cm (max {:.0} cm): the part stays behind when the body moves", base.pose_names[pi], 100.0 * lag_bad[pi] as f32 / lag_n[pi] as f32, lag_n[pi], lag_max[pi] * 100.0)));
            }
        }
        // stretched triangles in the animated poses
        for pi in 1..g.pose.len() {
            let (mut bad, mut worst) = (0usize, 0f32);
            for &(a, b) in &g.edges {
                let l0 = g.pose[0][a as usize].distance(g.pose[0][b as usize]);
                let l1 = g.pose[pi][a as usize].distance(g.pose[pi][b as usize]);
                if l1 > l0 * 3.0 && l1 - l0 > 0.12 {
                    bad += 1;
                    worst = worst.max(l1);
                }
            }
            if bad > 3 && bad as f32 > 0.003 * g.edges.len() as f32 {
                out.push(("stretched", "fail", format!("{}: {} of {} edges stretched more than 3x (longest {:.2} m)", base.pose_names[pi], bad, g.edges.len(), worst)));
            }
        }
        if matches!(slot.key, "chest" | "legs" | "hands" | "feet" | "costume") && !has_cut {
            if slot.key == "costume" {
                out.push(("no_cut_hull", "fail", "one-piece costume without a Cut hull and not replacing the body: the nude body stays underneath, z-fights with the costume's own skin and shows through".into()));
            } else {
                out.push(("no_cut_hull", "warn", "no Cut hull: the nude body is not hidden under the garment".into()));
            }
        }
        out
    }

    /// Materials of a part: missing material, missing sub-material for a used id, missing textures.
    fn judge_material(model: &Model, path: &str, mtl_override: Option<&str>, g: &Geo) -> (Vec<Note>, Option<Material>) {
        let mut out: Vec<Note> = Vec::new();
        let pak = aa_pak::Pak::global();
        let mtl_path = mtl_override.map(str::to_string).or_else(|| aa_cry::scene::find_mtl(path, model));
        let Some(mp) = mtl_path else {
            out.push(("no_material", "fail", "no .mtl found for the model".into()));
            return (out, None);
        };
        if !pak.exists(&mp) {
            out.push(("no_material", "fail", format!("{mp} is not in the pak")));
            return (out, None);
        }
        let Ok(mtl) = Material::load(&mp) else {
            out.push(("bad_material", "fail", format!("{mp} does not parse")));
            return (out, None);
        };
        let mut missing: Vec<String> = Vec::new();
        for &id in &g.used_mats {
            let Some(sub) = mtl.sub(id as usize) else {
                out.push(("subset_no_material", "fail", format!("{mp} has no sub-material {id}: that part of the mesh is not drawn")));
                continue;
            };
            if matches!(sub.shader.to_ascii_lowercase().as_str(), "cut" | "nodraw") {
                continue;
            }
            for t in sub.textures.iter().filter(|t| matches!(t.map, TextureMap::Diffuse | TextureMap::Normal)) {
                let tp = t.pak_path();
                if !tp.is_empty() && !pak.exists(&tp) && !missing.contains(&tp) {
                    missing.push(tp);
                }
            }
        }
        if !missing.is_empty() {
            out.push(("missing_texture", "fail", format!("{} texture(s) not in the pak: {}", missing.len(), missing.iter().take(3).cloned().collect::<Vec<_>>().join(", "))));
        }
        (out, Some(mtl))
    }

    /// CA_BONE object attachments of a cdf part.
    fn judge_attachments(base: &Base, atts: &[Attachment]) -> Vec<Note> {
        let mut out: Vec<Note> = Vec::new();
        let pak = aa_pak::Pak::global();
        for a in atts {
            if a.kind != AttachmentKind::Bone || a.binding_kind != BindingKind::Object || a.binding.is_empty() || a.flags & 1 != 0 {
                continue;
            }
            let (joint, local) = match a.bone_binding(&base.skel) {
                Ok(v) => v,
                Err(e) => {
                    out.push(("attach_unbound", "fail", format!("attachment {} ({}): {e}", a.name, a.binding)));
                    continue;
                }
            };
            let path = if a.binding.ends_with(".cdf") { Cdf::load(&a.binding).map(|c| c.model).unwrap_or_default() } else { a.binding.clone() };
            if path.is_empty() || !pak.exists(&path) {
                out.push(("attach_model_missing", "fail", format!("attachment {}: {} is not in the pak", a.name, a.binding)));
                continue;
            }
            let Ok(model) = Model::load(&path) else {
                out.push(("attach_model_missing", "fail", format!("attachment {}: {path} does not load", a.name)));
                continue;
            };
            let Some((mn, mx)) = model.bounds() else { continue };
            let m = base.worlds[0][joint] * local;
            let corners: Vec<Vec3> = (0..8)
                .map(|i| m.transform_point3(Vec3::new(if i & 1 == 0 { mn.x } else { mx.x }, if i & 2 == 0 { mn.y } else { mx.y }, if i & 4 == 0 { mn.z } else { mx.z })))
                .collect();
            let (bn, bx) = bbox(&corners);
            let c = (bn + bx) * 0.5;
            let r = &base.reference;
            let dz = (c.z - r.min.z) / base.height;
            let side = (c.x - (r.min.x + r.max.x) * 0.5).abs().max((c.y - (r.min.y + r.max.y) * 0.5).abs());
            if dz < -0.1 || dz > 1.25 || side > 0.9 {
                out.push(("attach_off_body", "fail", format!("attachment {} ({path}) centre ({:.2},{:.2},{:.2}) is {:.0}% of the body height, {:.2} m to the side", a.name, c.x, c.y, c.z, dz * 100.0, side)));
            }
        }
        out
    }

    /// Resolves a visual path to (`.chr`, `.mtl`, attachments) like `CharacterDesc::resolve`.
    fn resolve(path: &str) -> Result<(String, Option<String>, Vec<Attachment>), String> {
        if path.ends_with(".cdf") {
            let cdf = Cdf::load(path).map_err(|e| format!("cdf: {e}"))?;
            Ok((cdf.model, cdf.material, cdf.attachments))
        } else {
            Ok((path.to_string(), None, Vec::new()))
        }
    }

    /// Does the item, worn alone, take the nude body off (a costume)?
    fn body_replaced(base: &Base, item: u32) -> bool {
        let Ok(mut desc) = CharacterDesc::default_for(&base.db, base.race, base.gender) else { return false };
        desc.costume = Costume::Items(vec![item]);
        desc.resolve(&base.db).is_ok_and(|a| a.parts.iter().all(|p| p.kind != aa_anim::PartKind::Body))
    }

    fn armor_probe(base: &Base, slot: &Slot, item: &Item, findings: &mut Vec<Finding>, tot: &mut Totals) {
        let pak = aa_pak::Pak::global();
        let visuals = base.db.item_visuals(item.id, base.model).unwrap_or_default();
        let mk = |cat: &'static str, severity: &'static str, path: &str, detail: String| Finding {
            cat,
            severity,
            slot: slot.key,
            model: base.model,
            item: item.id,
            name: item.name.clone(),
            level: item.level,
            npc: is_npc(&item.name),
            path: path.to_string(),
            detail,
        };
        tot.items += 1;
        tot.items_shared += item.shared;
        tot.npc_items += is_npc(&item.name) as usize;
        if visuals.is_empty() {
            findings.push(mk("no_visual_variant", "info", "", "no item_assets row for this model".into()));
            return;
        }
        tot.with_visual += 1;
        let before = findings.iter().filter(|f| f.severity == "fail").count();
        for v in &visuals {
            tot.parts += 1;
            if !pak.exists(&v.path) {
                findings.push(mk("missing_file", "fail", &v.path, "visual path is not in the pak".into()));
                continue;
            }
            if v.path.ends_with(".dds") {
                findings.push(mk("texture_only_visual", "info", &v.path, "the visual is a texture (underwear overlay), not a mesh".into()));
                continue;
            }
            let (chr, mtl, atts) = match resolve(&v.path) {
                Ok(r) => r,
                Err(e) => {
                    findings.push(mk("bad_cdf", "fail", &v.path, e));
                    continue;
                }
            };
            if !pak.exists(&chr) {
                findings.push(mk("missing_file", "fail", &chr, format!("model of {} is not in the pak", v.path)));
                continue;
            }
            let model = match Model::load(&chr) {
                Ok(m) => m,
                Err(e) => {
                    findings.push(mk("bad_model", "fail", &chr, e.to_string()));
                    continue;
                }
            };
            let g = skin_part(base, &model, &chr);
            let (mnotes, mat) = judge_material(&model, &chr, mtl.as_deref(), &g);
            let hulls = mat.as_ref().map(|m| aa_anim::cut::cut_hulls(&model, m)).unwrap_or_default();
            let has_cut = !hulls.is_empty();
            if has_cut {
                // visible garment triangles (not the Cut / NoDraw hull) in the bind pose
                let hidden: Vec<bool> = g
                    .tri_mat
                    .iter()
                    .map(|&id| mat.as_ref().and_then(|m| m.sub(id as usize)).is_none_or(|s| matches!(s.shader.to_ascii_lowercase().as_str(), "cut" | "nodraw")))
                    .collect();
                let garment: Vec<[Vec3; 3]> = (0..g.edges.len() / 2)
                    .filter(|&t| !hidden.get(t).copied().unwrap_or(true))
                    .map(|t| [g.pose[0][g.edges[2 * t].0 as usize], g.pose[0][g.edges[2 * t].1 as usize], g.pose[0][g.edges[2 * t + 1].1 as usize]])
                    .collect();
                let (hid, open) = cut_holes(base, &hulls, &garment);
                if open >= 40 && open as f32 >= 0.08 * hid as f32 {
                    findings.push(mk("cut_hole", "info", &chr, format!("the Cut hull hides {hid} body triangles, {open} ({:.0}%) of them are not behind any garment triangle seen from outside (ray along the outward normal, 30 cm); EXPERIMENTAL, not confirmed by the sheets", 100.0 * open as f32 / hid.max(1) as f32)));
                }
            }
            // a costume replaces the nude body (`CharacterDesc::resolve`): no body for a Cut hull to hide
            let replaces_body = slot.key == "costume" && body_replaced(base, item.id);
            let mut notes = judge(base, slot, &g, has_cut || replaces_body);
            notes.extend(mnotes);
            notes.extend(judge_attachments(base, &atts));
            for (cat, sev, detail) in notes {
                findings.push(mk(cat, sev, &chr, detail));
            }
        }
        if findings.iter().filter(|f| f.severity == "fail").count() > before {
            tot.failed += 1;
        }
    }

    // ---- weapons: replicate aa-client/src/ingame/weapons.rs `carried` for the sheathed placement ----

    /// The sheath bone of a weapon (`weapons::sheath_bone`).
    pub fn sheath_bone(code: &str, class: Class, two_handed: bool, staff: bool) -> &'static str {
        if staff {
            return "item_staff_l";
        }
        match (code, class) {
            ("bow", _) => "item_bow_l",
            ("shield", _) => "item_arm_l",
            _ if two_handed => "item_back_c",
            (_, Class::SwordOff) => "item_side_r",
            (_, Class::Main1h) | (_, Class::SwordMain) => "item_side_l",
            _ => "item_back_c",
        }
    }

    /// The hand bone a weapon is held with (`weapons::carried`).
    pub fn hand_bone(code: &str, class: Class) -> &'static str {
        match (code, class) {
            ("string_instrument", _) => "item_hand_l",
            (_, Class::Percussion | Class::Tube) => "item_hand_r",
            ("bow", _) => "item_hand_l",
            (_, Class::SwordOff) => "item_hand_l",
            _ => "item_hand_r",
        }
    }

    pub fn weapon_transform(w: &WeaponRow) -> Affine3A {
        let [rx, ry, rz] = w.rotation.map(f32::to_radians);
        let q = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
        let s = if w.worn_scale > 0.0 { w.worn_scale } else { 1.0 };
        Affine3A::from_scale_rotation_translation(Vec3::splat(s), q, Vec3::from(w.offset))
    }

    fn weapon_probe(base: &Base, slot: &Slot, class: Class, item: &Item, findings: &mut Vec<Finding>, tot: &mut Totals) {
        let w = item.weapon.as_ref().unwrap();
        let pak = aa_pak::Pak::global();
        let mk = |cat: &'static str, severity: &'static str, detail: String| Finding {
            cat,
            severity,
            slot: slot.key,
            model: base.model,
            item: item.id,
            name: item.name.clone(),
            level: item.level,
            npc: is_npc(&item.name),
            path: w.path.clone(),
            detail,
        };
        tot.items += 1;
        tot.items_shared += item.shared;
        tot.npc_items += is_npc(&item.name) as usize;
        tot.with_visual += 1;
        tot.parts += 1;
        let before = findings.iter().filter(|f| f.severity == "fail").count();
        'probe: {
            if !pak.exists(&w.path) {
                findings.push(mk("missing_file", "fail", "weapon model is not in the pak".into()));
                break 'probe;
            }
            // a `.cdf` weapon visual is a character definition (model + attachments), not a mesh: aa-client's `attach_carried`
            // loads the path with `cry.load_model`, which cannot read it, so the weapon is never drawn
            let mut model_path = w.path.clone();
            if w.path.ends_with(".cdf") {
                match Cdf::load(&w.path) {
                    Ok(cdf) => {
                        let atts = cdf.attachments.iter().filter(|a| !a.binding.is_empty() && a.flags & 1 == 0).count();
                        findings.push(mk("weapon_cdf", "fail", format!("the weapon visual is a .cdf: not loadable as a mesh, the weapon is not drawn (its model is {}, {atts} attachments)", cdf.model)));
                        model_path = cdf.model;
                    }
                    Err(e) => {
                        findings.push(mk("weapon_cdf", "fail", format!("the weapon visual is a .cdf that does not parse: {e}")));
                        break 'probe;
                    }
                }
            }
            let model = match Model::load(&model_path) {
                Ok(m) => m,
                Err(e) => {
                    findings.push(mk("bad_model", "fail", e.to_string()));
                    break 'probe;
                }
            };
            let sheath = sheath_bone(&w.code, class, w.two_handed, w.staff);
            let hand = hand_bone(&w.code, class);
            let mut verts: Vec<Vec3> = Vec::new();
            for ni in model.lod_nodes(0) {
                let Some(mi) = model.nodes[ni].mesh else { continue };
                let tm = model.world_transform(ni);
                let mesh = &model.meshes[mi];
                let step = (mesh.positions.len() / 300).max(1);
                verts.extend(mesh.positions.iter().step_by(step).map(|p| tm.transform_point3(*p)));
            }
            for (what, bone, scale) in [("sheath", sheath, w.worn_scale), ("hand", hand, w.drawn_scale)] {
                let Some(bi) = base.skel.find(bone) else {
                    findings.push(mk("weapon_bone_missing", "fail", format!("{what} bone {bone} is not in the {} skeleton", model_label(base.model))));
                    continue;
                };
                if verts.is_empty() {
                    findings.push(mk("no_mesh", "fail", "weapon model has no render mesh".into()));
                    break 'probe;
                }
                let t = if what == "sheath" {
                    weapon_transform(w)
                } else {
                    // drawn in the hand: the hand bone holds it as modelled, only the scale applies
                    let s = if scale > 0.0 { scale } else { 1.0 };
                    Affine3A::from_scale(Vec3::splat(s))
                };
                let m = base.worlds[0][bi] * t;
                let placed: Vec<Vec3> = verts.iter().map(|v| m.transform_point3(*v)).collect();
                let (bn, bx) = bbox(&placed);
                let size = bx - bn;
                let longest = size.max_element();
                if !longest.is_finite() || longest > 100.0 {
                    findings.push(mk("bad_size", "fail", format!("{what}: bounding box {:.0} m across: broken vertex data", longest)));
                    break 'probe;
                }
                let r = &base.reference;
                let c = (bn + bx) * 0.5;
                let zc = (c.z - r.min.z) / base.height;
                let side = (c.x - (r.min.x + r.max.x) * 0.5).abs().max((c.y - (r.min.y + r.max.y) * 0.5).abs());
                // the gap between the weapon and the body: the closest weapon vertex to any body vertex
                let gap = placed.iter().filter_map(|p| r.nearest(*p, 0.6).map(|(_, d)| d)).fold(0.6f32, f32::min);
                if what == "sheath" && (zc < slot.region.0 - 0.15 || zc > slot.region.1 + 0.15 || side > 0.8) {
                    findings.push(mk("weapon_off_body", "fail", format!("sheathed on {bone}: bbox centre at {:.0}% of the body height, {:.2} m to the side, gap to the body {:.0} cm, size {:.2}x{:.2}x{:.2}", zc * 100.0, side, gap * 100.0, size.x, size.y, size.z)));
                }
                if what == "hand" && (zc < 0.2 || zc > 1.1 || side > 0.8) {
                    findings.push(mk("weapon_off_body", "fail", format!("in hand on {bone}: bbox centre at {:.0}% of the body height, {:.2} m to the side, size {:.2}x{:.2}x{:.2}", zc * 100.0, side, size.x, size.y, size.z)));
                }
                if longest < 0.08 || longest > 2.9 {
                    findings.push(mk("bad_size", "fail", format!("{what}: longest side {:.2} m (scale {:.2})", longest, scale)));
                }
                // instruments are only drawn while they are played (aa-client hides them in the sheath)
                let instrument = matches!(class, Class::String | Class::Percussion | Class::Tube);
                if what == "sheath" && longest > 0.5 && !instrument {
                    let axis = principal_axis(&placed);
                    // a sword hangs at the hip at a slant; flag what lies flat (within 10 degrees of horizontal for the hip, 15 on the back)
                    if axis.z.abs() < if matches!(class, Class::TwoHand) { 0.26 } else { 0.17 } {
                        let from_vertical = axis.z.abs().acos().to_degrees();
                        findings.push(mk("sheath_horizontal", "fail", format!("sheathed on {bone}: the long axis is {:.0} degrees from vertical (points {:+.2},{:+.2},{:+.2} in model space, z up), {:.2} m long, centre {:.0}% of the body height", from_vertical, axis.x, axis.y, axis.z, longest, zc * 100.0)));
                    }
                }
                if gap > 0.12 && what == "sheath" && !w.staff {
                    findings.push(mk("floating", "fail", format!("{what} on {bone}: nearest weapon vertex is {:.0} cm from the body", gap * 100.0)));
                }
            }
            if aa_cry::scene::find_mtl(&model_path, &model).is_none() {
                findings.push(mk("no_material", "fail", "no .mtl found for the weapon model".into()));
            }
        }
        if findings.iter().filter(|f| f.severity == "fail").count() > before {
            tot.failed += 1;
        }
    }

    fn analyze_model(race: Race, gender: Gender, slots: &[Slot], items: &HashMap<&'static str, Vec<Item>>) -> (Vec<Finding>, Vec<(&'static str, Totals)>) {
        let base = build_base(race, gender);
        eprintln!("[{}] base ready: height {:.2} m, poses {:?}", model_label(base.model), base.height, base.pose_names);
        let mut findings = Vec::new();
        let mut totals = Vec::new();
        for slot in slots {
            let mut tot = Totals::default();
            for item in &items[slot.key] {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match slot.kind {
                    SlotKind::Armor(_) => armor_probe(&base, slot, item, &mut findings, &mut tot),
                    SlotKind::Weapon(c) => weapon_probe(&base, slot, c, item, &mut findings, &mut tot),
                }));
                if r.is_err() {
                    findings.push(Finding {
                        cat: "analysis_panic",
                        severity: "fail",
                        slot: slot.key,
                        model: base.model,
                        item: item.id,
                        name: item.name.clone(),
                        level: item.level,
                        npc: is_npc(&item.name),
                        path: String::new(),
                        detail: "the analysis itself panicked on this item".into(),
                    });
                    tot.failed += 1;
                }
            }
            eprintln!("[{}] {:<10} items {:>4} visuals {:>4} failed {:>4}", model_label(base.model), slot.key, tot.items, tot.with_visual, tot.failed);
            totals.push((slot.key, tot));
        }
        (findings, totals)
    }

    pub fn run() {
        let only_slot = arg("--slot");
        let only_model: Option<u32> = arg("--model").and_then(|m| m.parse().ok());
        let out_dir = PathBuf::from(arg("--out").unwrap_or_else(|| "data/extract/aa-anim/equip_sweep".into()));
        std::fs::create_dir_all(&out_dir).unwrap();
        let conn = open_db();
        let slots: Vec<Slot> = SLOTS.iter().copied().filter(|s| only_slot.as_ref().is_none_or(|k| k == s.key)).collect();
        let mut items: HashMap<&'static str, Vec<Item>> = HashMap::new();
        for s in &slots {
            let all = catalogue(&conn, s);
            eprintln!("slot {:<10} {} distinct visuals", s.key, all.len());
            items.insert(s.key, all);
        }
        let items = Arc::new(items);
        let mut all_findings: Vec<Finding> = Vec::new();
        let mut all_totals: Vec<(u32, &'static str, Totals)> = Vec::new();
        std::thread::scope(|scope| {
            let mut handles = Vec::new();
            for (race, gender, model) in models().into_iter().filter(|m| only_model.is_none_or(|o| o == m.2)) {
                let (slots, items) = (&slots, items.clone());
                handles.push(scope.spawn(move || (model, analyze_model(race, gender, slots, &items))));
            }
            for h in handles {
                let (model, (f, t)) = h.join().expect("model thread");
                all_findings.extend(f);
                all_totals.extend(t.into_iter().map(|(s, t)| (model, s, t)));
            }
        });
        let mut tsv = String::from("category\tseverity\tslot\tmodel\titem\tlevel\tnpc\tname\tpath\tdetail\n");
        for f in &all_findings {
            let _ = writeln!(tsv, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", f.cat, f.severity, f.slot, model_label(f.model), f.item, f.level, f.npc as u8, f.name.replace('\t', " "), f.path, f.detail.replace('\t', " "));
        }
        std::fs::write(out_dir.join("findings.tsv"), tsv).unwrap();
        let mut tsv = String::from("slot\tmodel\titems\titems_sharing_visuals\tnpc_items\twith_visual\tparts\tfailed\n");
        for (m, s, t) in &all_totals {
            let _ = writeln!(tsv, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", s, model_label(*m), t.items, t.items_shared, t.npc_items, t.with_visual, t.parts, t.failed);
        }
        std::fs::write(out_dir.join("totals.tsv"), tsv).unwrap();
        // the sample the contact sheets use
        let mut tsv = String::from("slot\titem\tlevel\tgrade\tflavour\tshared\tname\n");
        for s in &slots {
            for i in sample(&conn, &items[s.key], 40) {
                let _ = writeln!(tsv, "{}\t{}\t{}\t{}\t{}\t{}\t{}", s.key, i.id, i.level, i.grade, i.flavour, i.shared, i.name);
            }
        }
        std::fs::write(out_dir.join("sample.tsv"), tsv).unwrap();
        let mut by_cat: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        for f in &all_findings {
            *by_cat.entry((f.severity, f.cat)).or_default() += 1;
        }
        for ((sev, cat), n) in by_cat {
            eprintln!("{sev:<5} {cat:<22} {n}");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// contact sheets
// ---------------------------------------------------------------------------------------------

mod sheets {
    use std::path::Path;

    use aa_anim::cdf::{Attachment, AttachmentKind, BindingKind};
    use aa_anim::pose::SampleOptions;
    use aa_anim::{AnimPlugin, Animator, Cal, Character, CharacterDesc, ClipLibrary, Costume, GameDb, spawn_character};
    use aa_cry::Shading;
    use aa_cry::scene::{CryAssets, CryPlugin, ModelOptions, spawn_loaded};
    use bevy::camera::RenderTarget;
    use bevy::prelude::*;
    use bevy::render::view::screenshot::{Screenshot, save_to_disk};
    use bevy::winit::WinitSettings;

    use super::{Class, Item, SLOTS, Slot, SlotKind, analysis, arg, catalogue, model_label, models, open_db, sample};

    /// Seconds to wait after the characters were spawned (textures load asynchronously).
    const SETTLE: f32 = 14.0;

    /// Camera framing of a slot: height of the middle of the picture, height of the picture (m) and the width of one cell (m).
    fn framing(slot: &Slot) -> (f32, f32, f32) {
        match slot.key {
            "head" => (1.64, 0.78, 0.90),
            "chest" => (1.30, 1.00, 1.00),
            "waist" => (1.00, 0.90, 0.95),
            "legs" => (0.65, 1.40, 0.95),
            "hands" => (1.00, 1.20, 1.00),
            "feet" => (0.30, 0.80, 0.85),
            "back" => (1.15, 1.60, 1.10),
            _ => (0.98, 2.15, 1.20),
        }
    }

    pub fn views(slot: &Slot) -> Vec<&'static str> {
        match slot.key {
            "back" | "twohand" | "bow" | "costume" | "string" => vec!["front", "side", "back"],
            _ => vec!["front", "side"],
        }
    }

    #[derive(Resource)]
    struct Plan {
        slot: Slot,
        model: u32,
        target: Handle<Image>,
        out: String,
        prefix: String,
        cells: Vec<(Entity, usize)>,
        shots: Vec<(String, &'static str, usize)>,
        next: usize,
        phase: Phase,
        frames: u32,
        spawned_at: f32,
        arranged_at: f32,
        drawn: bool,
        /// pose name of the clips currently applied
        applied: String,
    }

    #[derive(PartialEq, Eq, Clone, Copy)]
    enum Phase {
        Settle,
        Arrange,
        Wait,
        Capture,
        Done,
    }

    /// A weapon waiting for the skeleton of its wearer.
    #[derive(Component)]
    struct Pending(Vec<PendingWeapon>);

    struct PendingWeapon {
        path: String,
        sheath: &'static str,
        sheath_t: Transform,
        hand: &'static str,
        hand_t: Transform,
    }

    #[derive(Component)]
    struct Weapon {
        sheath: Entity,
        hand: Entity,
        sheath_t: Transform,
        hand_t: Transform,
    }

    pub fn run() {
        let slot_key = arg("--slot").unwrap_or_else(|| "head".into());
        if !SLOTS.iter().any(|s| s.key == slot_key) {
            eprintln!("unknown slot {slot_key}");
            return;
        }
        let model: u32 = arg("--model").and_then(|m| m.parse().ok()).unwrap_or(10);
        if !models().iter().any(|m| m.2 == model) {
            eprintln!("unknown model {model}");
            return;
        }
        App::new()
            .add_plugins((aa_dev::default_plugins("aa-anim equip_sweep"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
            .insert_resource(WinitSettings::continuous())
            .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
            .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 700.0, ..default() })
            .add_systems(Startup, setup)
            .add_systems(Update, (attach_weapons, attach_part_objects, place_weapons, drive).chain())
            .run();
    }

    fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>, mut exit: MessageWriter<AppExit>) {
        let slot_key = arg("--slot").unwrap_or_else(|| "head".into());
        let slot = *SLOTS.iter().find(|s| s.key == slot_key).unwrap();
        let model: u32 = arg("--model").and_then(|m| m.parse().ok()).unwrap_or(10);
        let (race, gender, _) = models().into_iter().find(|m| m.2 == model).unwrap();
        let n: usize = arg("--n").and_then(|v| v.parse().ok()).unwrap_or(32);
        let skip: usize = arg("--skip").and_then(|v| v.parse().ok()).unwrap_or(0);
        let cols: usize = arg("--cols").and_then(|v| v.parse().ok()).unwrap_or(8);
        let out = arg("--out").unwrap_or_else(|| "shots/aa-anim/equip/_raw".into());
        let poses: Vec<String> = arg("--poses").unwrap_or_else(|| "idle,action".into()).split(',').map(str::to_string).collect();
        let width: u32 = arg("--width").and_then(|v| v.parse().ok()).unwrap_or(3200);
        let _ = std::fs::create_dir_all(&out);

        let conn = open_db();
        let all = catalogue(&conn, &slot);
        let picked: Vec<Item> = if let Some(ids) = arg("--items") {
            ids.split(',').filter_map(|id| id.trim().parse::<u32>().ok()).filter_map(|id| all.iter().find(|i| i.id == id).cloned()).collect()
        } else if super::flag("--all") {
            all.iter().filter(|i| !super::is_npc(&i.name)).cloned().collect()
        } else {
            sample(&conn, &all, n + skip)
        };
        let picked: Vec<Item> = picked.into_iter().skip(skip).take(n).collect();
        let rows = picked.len().div_ceil(cols).max(1);
        let (centre, extent, cell_w) = match arg("--frame").map(|f| f.split(',').filter_map(|v| v.parse::<f32>().ok()).collect::<Vec<_>>()) {
            Some(f) if f.len() == 3 => (f[0], f[1], f[2]),
            _ => framing(&slot),
        };
        let frame_w = cell_w * cols as f32;
        let height = ((width as f32) * extent / frame_w).round() as u32;
        let prefix = format!("{}_{}_{}", slot.key, model_label(model), arg("--tag").unwrap_or_else(|| "a".into()));
        info!("equip_sweep: {} {} items for {} ({} rows x {} cols), image {width}x{height}", picked.len(), slot.key, model_label(model), rows, cols);

        let db = GameDb::open().expect("game database");
        let mut cells = Vec::new();
        let mut info_tsv = String::from("row\tcol\titem\tlevel\tstatus\tname\n");
        for (k, item) in picked.iter().enumerate() {
            let (row, col) = (k / cols, k % cols);
            let mut desc = CharacterDesc::default_for(&db, race, gender).expect("default look");
            desc.costume = Costume::None;
            let mut status = "ok".to_string();
            let mut pending = Vec::new();
            let assets = match slot.kind {
                SlotKind::Armor(_) => {
                    desc.costume = Costume::Items(vec![item.id]);
                    let visuals = db.item_visuals(item.id, model).unwrap_or_default();
                    let r = if slot.key == "head" {
                        desc.resolve_with_headgear(&db, visuals.first().map(|v| v.detail))
                    } else {
                        desc.resolve(&db)
                    };
                    match r {
                        Ok(a) if !visuals.is_empty() => a,
                        Ok(a) => {
                            status = "no_visual".into();
                            a
                        }
                        Err(e) => {
                            status = format!("resolve_failed: {e}");
                            warn!("{} {}: {e}", slot.key, item.id);
                            desc.costume = Costume::None;
                            desc.resolve(&db).expect("nude")
                        }
                    }
                }
                SlotKind::Weapon(class) => {
                    let w = item.weapon.as_ref().unwrap();
                    let sheath = analysis::sheath_bone(&w.code, class, w.two_handed, w.staff);
                    let hand = analysis::hand_bone(&w.code, class);
                    let tr = analysis::weapon_transform(w);
                    let (s, r, t) = tr.to_scale_rotation_translation();
                    let mut hand_t = Transform::from_scale(Vec3::splat(if w.drawn_scale > 0.0 { w.drawn_scale } else { 1.0 }));
                    if w.code == "bow" {
                        hand_t.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
                    }
                    pending.push(PendingWeapon { path: w.path.clone(), sheath, sheath_t: Transform { translation: t, rotation: r, scale: s }, hand, hand_t });
                    desc.resolve(&db).expect("nude")
                }
            };
            let x = (col as f32 - (cols as f32 - 1.0) * 0.5) * cell_w;
            let transform = Transform::from_xyz(x, 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
            match spawn_character(&mut commands, &mut cry, &clips, assets, transform) {
                Ok(e) => {
                    // every row is visible (on top of each other) while the pipelines of the materials compile
                    if !pending.is_empty() {
                        commands.entity(e).insert(Pending(pending));
                    }
                    cells.push((e, row));
                }
                Err(e) => {
                    status = format!("spawn_failed: {e}");
                    error!("{} {}: {e}", slot.key, item.id);
                }
            }
            info_tsv.push_str(&format!("{row}\t{col}\t{}\t{}\t{status}\t{}\n", item.id, item.level, item.name));
        }
        let _ = std::fs::write(format!("{out}/{prefix}.tsv"), info_tsv);
        if cells.is_empty() {
            warn!("nothing to draw");
            exit.write(AppExit::Success);
        }

        // showroom: lights like the lineup example, the camera renders into an image
        for (lux, color, from) in [
            (18000.0, Color::srgb(1.0, 0.96, 0.9), Vec3::new(2.0, 5.0, 6.0)),
            (5000.0, Color::srgb(0.6, 0.7, 1.0), Vec3::new(-5.0, 2.5, 4.0)),
            (8000.0, Color::srgb(1.0, 0.9, 0.8), Vec3::new(0.0, 3.0, -6.0)),
        ] {
            commands.spawn((
                DirectionalLight { illuminance: lux, color, shadow_maps_enabled: false, ..default() },
                Transform::from_translation(from).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
            ));
        }
        let image = Image::new_target_texture(width, height, bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb, None);
        let target = cry.images.add(image);
        let dist = 16.0;
        let fov = 2.0 * (extent * 0.5 / dist).atan();
        let mut camera = commands.spawn((
            Camera3d::default(),
            aa_cry::stage::login_color_grading(),
            Projection::Perspective(PerspectiveProjection { fov, ..default() }),
            Transform::from_xyz(0.0, centre, dist).looking_at(Vec3::new(0.0, centre, 0.0), Vec3::Y),
            RenderTarget::Image(target.clone().into()),
        ));
        if let Some(env) = aa_cry::stage::environment_light(&mut cry, aa_cry::stage::DEFAULT_ENVIRONMENT, 3000.0) {
            camera.insert(env);
        }

        let mut shots = Vec::new();
        for pose in &poses {
            for view in views(&slot) {
                for row in 0..rows {
                    shots.push((pose.clone(), view, row));
                }
            }
        }
        commands.insert_resource(Plan {
            slot,
            model,
            target,
            out,
            prefix,
            cells,
            shots,
            next: 0,
            phase: Phase::Settle,
            frames: 0,
            spawned_at: 0.0,
            arranged_at: 0.0,
            drawn: false,
            applied: String::new(),
        });
    }

    /// Hangs a weapon on its sheath bone once the wearer's skeleton exists.
    fn attach_weapons(mut commands: Commands, mut cry: CryAssets, wearers: Query<(Entity, &Animator, &Pending)>) {
        for (entity, animator, pending) in &wearers {
            let bone_of = |name: &str| animator.rig.skeleton.bones.iter().position(|b| b.name == name).and_then(|i| animator.bones.get(i).copied());
            for w in &pending.0 {
                let (Some(sheath), Some(hand)) = (bone_of(w.sheath), bone_of(w.hand)) else {
                    warn!("equip_sweep: bone {} / {} is missing for {}", w.sheath, w.hand, w.path);
                    continue;
                };
                let options = ModelOptions { cry_space: true, shading: Shading::Full, ..default() };
                let Ok(loaded) = cry.load_model(&w.path, &options) else {
                    warn!("equip_sweep: weapon model {} does not load", w.path);
                    continue;
                };
                let model = spawn_loaded(&mut commands, &w.path, &loaded, &options);
                commands.entity(model).insert((w.sheath_t, ChildOf(sheath), Weapon { sheath, hand, sheath_t: w.sheath_t, hand_t: w.hand_t }));
            }
            commands.entity(entity).remove::<Pending>();
        }
    }

    /// CA_BONE object attachments declared by the cdf of a costume part (see aa-client `attachments.rs`).
    fn attach_part_objects(mut commands: Commands, mut cry: CryAssets, characters: Query<(&Character, &Animator), Added<Animator>>) {
        for (character, animator) in &characters {
            for part in &character.assets.parts {
                for a in &part.attachments {
                    let Some((path, material)) = object_model(a) else { continue };
                    let Ok((joint, local)) = a.bone_binding(&animator.rig.skeleton) else { continue };
                    let Some(&parent) = animator.bones.get(joint) else { continue };
                    let options = ModelOptions { material, cry_space: true, ..default() };
                    let Ok(loaded) = cry.load_model(&path, &options) else { continue };
                    let e = spawn_loaded(&mut commands, &path, &loaded, &options);
                    commands.entity(e).insert((Transform::from_matrix(Mat4::from(local)), ChildOf(parent)));
                }
            }
        }
    }

    fn object_model(a: &Attachment) -> Option<(String, Option<String>)> {
        if a.kind != AttachmentKind::Bone || a.binding_kind != BindingKind::Object || a.binding.is_empty() || a.flags & 1 != 0 {
            return None;
        }
        if a.binding.ends_with(".cdf") {
            let cdf = aa_anim::Cdf::load(&a.binding).ok()?;
            Some((cdf.model, cdf.material.or_else(|| a.material.clone())))
        } else {
            Some((a.binding.clone(), a.material.clone()))
        }
    }

    /// Moves the weapons between the sheath and the hand when the plan switches.
    fn place_weapons(mut commands: Commands, plan: Option<Res<Plan>>, weapons: Query<(Entity, &Weapon)>) {
        let Some(plan) = plan else { return };
        if !plan.is_changed() {
            return;
        }
        for (e, w) in &weapons {
            if plan.drawn {
                commands.entity(e).insert((ChildOf(w.hand), w.hand_t));
            } else {
                commands.entity(e).insert((ChildOf(w.sheath), w.sheath_t));
            }
        }
    }

    /// The clips a pose name stands for (first one the model has, fraction of its length, weapon in the hand).
    fn pose_spec(name: &str, slot: &Slot) -> (Vec<&'static str>, f32, bool) {
        let weapon = matches!(slot.kind, SlotKind::Weapon(_));
        match name {
            "idle" => (vec!["fist_ba_relaxed_idle"], 0.0, false),
            "sit" => (vec!["fist_ac_sit_down_loop"], 0.6, false),
            "leap" => (vec!["all_co_sk_leapattack"], 0.4, false),
            "action" if weapon => {
                let clips = match slot.kind {
                    SlotKind::Weapon(Class::TwoHand) => vec!["twohand_ba_combat_idle", "onehand_ba_combat_idle"],
                    SlotKind::Weapon(Class::Bow) => vec!["bow_ba_combat_idle", "onehand_ba_combat_idle"],
                    SlotKind::Weapon(Class::String | Class::Percussion | Class::Tube) => vec!["music_ba_combat_idle", "onehand_ba_combat_idle"],
                    _ => vec!["onehand_ba_combat_idle"],
                };
                (clips, 0.0, true)
            }
            _ => (vec!["onehand_ba_combat_idle"], 0.0, false),
        }
    }

    fn drive(
        mut commands: Commands,
        time: Res<Time>,
        plan: Option<ResMut<Plan>>,
        clips: Res<ClipLibrary>,
        mut animators: Query<&mut Animator>,
        mut transforms: Query<&mut Transform>,
        mut exit: MessageWriter<AppExit>,
    ) {
        let Some(mut plan) = plan else { return };
        let now = time.elapsed_secs();
        match plan.phase {
            Phase::Settle => {
                if plan.spawned_at == 0.0 {
                    plan.spawned_at = now;
                }
                if now - plan.spawned_at >= SETTLE {
                    plan.phase = Phase::Arrange;
                }
            }
            Phase::Arrange => {
                let Some((pose, view, row)) = plan.shots.get(plan.next).cloned() else {
                    info!("equip_sweep: sheet {} done", plan.prefix);
                    exit.write(AppExit::Success);
                    plan.phase = Phase::Done;
                    return;
                };
                if plan.applied != pose {
                    let (names, frac, drawn) = pose_spec(&pose, &plan.slot);
                    let (race, gender) = aa_anim::Race::from_model_id(plan.model).unwrap();
                    let cal_path = format!("objects/characters/{}/{}/nude/{}_{}_base.cal", race.folder(), gender.folder(), race.prefix(), gender.letter());
                    if let Ok(cal) = Cal::shared(&cal_path) {
                        match names.iter().find_map(|n| cal.animation(n).map(|e| (*n, e.clone()))) {
                            Some((name, entry)) => {
                                for (e, _) in plan.cells.clone() {
                                    let Ok(mut a) = animators.get_mut(e) else { continue };
                                    let opts = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: true };
                                    if let Ok(bc) = clips.bound(&a.rig.clone(), &entry.path, opts) {
                                        let dur = bc.clip.duration;
                                        a.play(bc, true, 0.0);
                                        if let Some(c) = a.current.as_mut() {
                                            c.time = dur * frac;
                                        }
                                        a.random_idle = None;
                                        a.paused = true;
                                    }
                                }
                                info!("equip_sweep: pose {pose} = {name} at {:.0}%", frac * 100.0);
                            }
                            None => warn!("equip_sweep: none of {names:?} is in {cal_path}"),
                        }
                    }
                    plan.drawn = drawn;
                    plan.applied = pose.clone();
                }
                // view = rotation of the characters (the camera and the lights stay), row = visibility
                let angle = match view {
                    "front" => std::f32::consts::PI,
                    "side" => std::f32::consts::FRAC_PI_2,
                    _ => 0.0,
                };
                for (e, r) in plan.cells.clone() {
                    if let Ok(mut t) = transforms.get_mut(e) {
                        t.rotation = Quat::from_rotation_y(angle);
                    }
                    commands.entity(e).insert(if r == row { Visibility::Inherited } else { Visibility::Hidden });
                }
                plan.frames = 0;
                plan.arranged_at = now;
                plan.phase = Phase::Wait;
            }
            Phase::Wait => {
                plan.frames += 1;
                // the first picture waits for the pipelines of the new materials to compile
                let min_secs = if plan.next == 0 { 5.0 } else { 0.0 };
                if plan.frames >= 12 && now - plan.arranged_at >= min_secs {
                    let (pose, view, row) = plan.shots[plan.next].clone();
                    let path = format!("{}/{}_{}_{}_r{}.png", plan.out, plan.prefix, pose, view, row);
                    let _ = std::fs::remove_file(&path);
                    commands.spawn(Screenshot::image(plan.target.clone())).observe(save_to_disk(path));
                    plan.frames = 0;
                    plan.phase = Phase::Capture;
                }
            }
            Phase::Capture => {
                plan.frames += 1;
                let (pose, view, row) = plan.shots[plan.next].clone();
                let path = format!("{}/{}_{}_{}_r{}.png", plan.out, plan.prefix, pose, view, row);
                if Path::new(&path).exists() && plan.frames > 3 {
                    plan.next += 1;
                    plan.phase = Phase::Arrange;
                } else if plan.frames > 600 {
                    error!("equip_sweep: screenshot {path} never arrived");
                    plan.next += 1;
                    plan.phase = Phase::Arrange;
                }
            }
            Phase::Done => {}
        }
    }
}
