//! Stealth: what a unit under a `buffs.stealth` buff looks like (`docs/research/re-skills.md` section 5).
//!
//! `ClientUnit::ApplyStealth` hides a stealthed unit unless the local player sees it: the player itself, a friendly unit
//! (`system_factions` / `system_faction_relations`, the rule of the name plates' colours, `hud::data::GameData::relation`, private to the
//! hud) or one within the detection range (front or back of the player's facing). A visible stealthed character gets CryEngine's
//! cloak material layer, the same for the player, allies and detected enemies (no colour by relation): [`CloakMaterial`], the port
//! of `cloaklayer.cfx` in `stealth.wesl` (a palette colour that runs dark red, magenta, violet, blue, white towards the rim of the
//! body, scanlines and blue sparks). A stealthed NPC has no look: it is only drawn or hidden. The server only says which buffs a
//! unit has (`aa_net::state::Buffs`), so this reconciles with that list: the meshes of a character swap to the cloak material (and
//! get their own back when the buff goes), the meshes of a hidden unit are hidden.

use std::collections::{HashMap, HashSet};

use aa_cry::shade::backlight::CharMaterial;
use aa_cry::shade::hair::{HairFringe, HairMaterial};
use aa_cry::shade::illum::IllumMaterial;
use aa_cry::shade::skin::SkinMaterial;
use aa_net::state::{Buffs, Player, Units};
use bevy::asset::embedded_asset;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use rusqlite::{Connection, OpenFlags};

use super::player::LocalPlayer;
use super::units::RemoteUnit;

/// The seconds between two looks at the meshes of a stealthed unit (equipment changes bring new ones).
const RESCAN: f64 = 0.5;

/// How far (m) the local player sees a stealthed unit in front of it and behind it (`detect_stealth_range_front` / `_back`, a stat
/// the client reads from the unit's attributes; the base values are not in the data we have, so these are guesses).
const DETECT_FRONT: f32 = 8.0;
const DETECT_BACK: f32 = 4.0;

/// The cloak layer of CryEngine (`MTL_LAYER_CLOAK`), see `stealth.wesl`.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct CloakMaterial {
    /// `x` the gain of the palette colour, `y` the gain of the sparks.
    #[uniform(0)]
    params: Vec4,
}

impl Material for CloakMaterial {
    fn fragment_shader() -> ShaderRef {
        concat!("embedded://", env!("CARGO_CRATE_NAME"), "/ingame/stealth.wesl").into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

/// `buffs` rows with `stealth`, the factions, and the material a visible stealthed character is drawn with.
#[derive(Resource)]
struct Stealth {
    buffs: HashSet<u32>,
    factions: Factions,
    cloak: Handle<CloakMaterial>,
}

/// Whether the local player (`me`) sees a stealthed unit at `at` by distance: within [`DETECT_FRONT`] in front of its facing,
/// within [`DETECT_BACK`] behind it.
fn detected(me: &GlobalTransform, at: &GlobalTransform) -> bool {
    let delta = (at.translation() - me.translation()) * Vec3::new(1.0, 0.0, 1.0);
    let ahead = delta.dot(*me.forward()) >= 0.0;
    delta.length() <= if ahead { DETECT_FRONT } else { DETECT_BACK }
}

/// How a stealthed unit is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// The cloak layer: the local player, the units friendly to it and the ones it detects.
    Ghost,
    /// A stealthed NPC the player sees: drawn as it is.
    Seen,
    /// Not at all.
    Hidden,
}

/// On a unit that is drawn stealthed.
#[derive(Component)]
struct Stealthed {
    look: Look,
    scanned: f64,
}

/// `system_factions.mother_id` and `system_faction_relations` (state 1 hostile, 2 neutral, 3 friendly).
#[derive(Default)]
struct Factions {
    mother: HashMap<u32, u32>,
    relations: HashMap<(u32, u32), i64>,
}

impl Factions {
    /// Whether a unit of faction `other` is friendly to one of faction `own` (the server's `SystemFaction.GetRelationState`: a
    /// faction under a root faction 1, 2 or 3 is friendly, neutral or hostile to everyone, the same group is friendly, else the
    /// stored relation of the mother faction, or of the faction, decides).
    fn friendly(&self, own: u32, other: u32) -> bool {
        let (Some(_), Some(_)) = (self.mother.get(&own), self.mother.get(&other)) else { return false };
        let mother = |f: u32| self.mother.get(&f).copied().unwrap_or(0);
        let group = |f: u32| if mother(f) != 0 { mother(f) } else { f };
        let (mine, theirs) = (group(own), group(other));
        for root in [mine, theirs] {
            match root {
                1 => return true,
                2 | 3 => return false,
                _ => {}
            }
        }
        if mine == theirs {
            return true;
        }
        let stored = |a: u32, b: u32| self.relations.get(&(a, b)).or_else(|| self.relations.get(&(b, a))).copied();
        let state = if mother(own) != 0 && self.mother.contains_key(&mother(own)) { stored(mother(own), theirs).or(Some(3)) } else { stored(own, theirs) };
        state == Some(3)
    }
}

/// On a mesh that wears the ghost material: what it wore before.
#[derive(Component)]
enum Swapped {
    Standard(Handle<StandardMaterial>),
    Char(Handle<CharMaterial>),
    Hair(Handle<HairMaterial>),
    Skin(Handle<SkinMaterial>),
    Illum(Handle<IllumMaterial>),
}

/// On a mesh of a hidden unit: its visibility before.
#[derive(Component)]
struct Cloaked(Visibility);

pub fn plugin(app: &mut App) {
    embedded_asset!(app, "stealth.wesl");
    app.add_plugins(MaterialPlugin::<CloakMaterial>::default()).add_systems(Startup, load).add_systems(Update, reconcile.run_if(resource_exists::<Stealth>));
}

fn load(mut commands: Commands, mut materials: ResMut<Assets<CloakMaterial>>) {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI) else { return };
    let ids = conn
        .prepare("SELECT id FROM buffs WHERE stealth = 't'")
        .and_then(|mut q| q.query_map([], |r| r.get::<_, i64>(0).map(|id| id as u32))?.collect::<Result<HashSet<_>, _>>())
        .unwrap_or_default();
    if ids.is_empty() {
        return;
    }
    let mut factions = Factions::default();
    if let Ok(mut q) = conn.prepare("SELECT id, mother_id FROM system_factions")
        && let Ok(rows) = q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0) as u32)))
    {
        factions.mother = rows.flatten().collect();
    }
    if let Ok(mut q) = conn.prepare("SELECT faction1_id, faction2_id, state_id FROM system_faction_relations")
        && let Ok(rows) = q.query_map([], |r| Ok(((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32), r.get::<_, i64>(2)?)))
    {
        factions.relations = rows.flatten().collect();
    }
    // the gains stand for the engine's light amount and HDR range (the renderer fills `CloakParams`; x2game does not)
    let cloak = materials.add(CloakMaterial { params: Vec4::new(4.5, 2.5, 0.0, 0.0) });
    commands.insert_resource(Stealth { buffs: ids, factions, cloak });
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    time: Res<Time>,
    stealth: Res<Stealth>,
    buffs: Option<Res<Buffs>>,
    player: Option<Res<Player>>,
    net_units: Option<Res<Units>>,
    units: Query<(Entity, Option<&RemoteUnit>, Option<&LocalPlayer>, Option<&Stealthed>, &GlobalTransform)>,
    children: Query<&Children>,
    meshes: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<CharMaterial>>, Option<&MeshMaterial3d<HairMaterial>>, Option<&MeshMaterial3d<SkinMaterial>>, Option<&MeshMaterial3d<IllumMaterial>>, Option<&Swapped>, Option<&Cloaked>, Option<&Visibility>), With<Mesh3d>>,
    fringes: Query<&HairFringe>,
) {
    let Some(buffs) = buffs else { return };
    let own_faction = player.as_ref().and_then(|p| p.spawn.as_ref()).map_or(0, |s| s.character().faction_id);
    let now = time.elapsed_secs_f64();
    let me = units.iter().find(|u| u.2.is_some()).map(|u| u.4);
    for (unit, remote, local, state, at) in &units {
        let Some(obj) = remote.map(|r| r.obj_id).or(local.map(|l| l.obj_id)) else { continue };
        let stealthed = buffs.of(obj).iter().any(|b| stealth.buffs.contains(&b.buff.buff_id));
        let look = stealthed.then(|| {
            let friendly = remote.is_none() || net_units.as_ref().and_then(|u| u.by_id.get(&obj)).is_some_and(|u| stealth.factions.friendly(own_faction, u.faction_id));
            let seen = friendly || me.is_some_and(|me| detected(me, at));
            match (seen, remote.is_some_and(|r| r.is_character()) || remote.is_none()) {
                (true, true) => Look::Ghost,
                // a stealthed NPC has no look: it is drawn or hidden
                (true, false) => Look::Seen,
                (false, _) => Look::Hidden,
            }
        });
        match (look, state) {
            (None, None) => continue,
            (Some(l), Some(s)) if s.look == l && now - s.scanned < RESCAN => continue,
            _ => {}
        }
        match look {
            Some(look) => commands.entity(unit).insert(Stealthed { look, scanned: now }),
            None => commands.entity(unit).remove::<Stealthed>(),
        };
        // the blended strand ends of the hair (`HairFringe`, the whole hair mesh again): hidden while the hair is a ghost
        let fringe_meshes: Vec<Entity> = children.iter_descendants(unit).filter_map(|m| fringes.get(m).ok().map(|f| f.0)).collect();
        for mesh in children.iter_descendants(unit) {
            let Ok((standard, char, hair, skin, illum, swapped, cloaked, visibility)) = meshes.get(mesh) else { continue };
            let fringe = fringe_meshes.contains(&mesh);
            let mut e = commands.entity(mesh);
            match (look == Some(Look::Ghost) && !fringe, swapped) {
                (true, None) => {
                    // a material of the character: anything else (an effect) keeps its look
                    if let Some(m) = standard {
                        e.remove::<MeshMaterial3d<StandardMaterial>>().insert((Swapped::Standard(m.0.clone()), MeshMaterial3d(stealth.cloak.clone())));
                    } else if let Some(m) = char {
                        e.remove::<MeshMaterial3d<CharMaterial>>().insert((Swapped::Char(m.0.clone()), MeshMaterial3d(stealth.cloak.clone())));
                    } else if let Some(m) = hair {
                        e.remove::<MeshMaterial3d<HairMaterial>>().insert((Swapped::Hair(m.0.clone()), MeshMaterial3d(stealth.cloak.clone())));
                    } else if let Some(m) = skin {
                        e.remove::<MeshMaterial3d<SkinMaterial>>().insert((Swapped::Skin(m.0.clone()), MeshMaterial3d(stealth.cloak.clone())));
                    } else if let Some(m) = illum {
                        e.remove::<MeshMaterial3d<IllumMaterial>>().insert((Swapped::Illum(m.0.clone()), MeshMaterial3d(stealth.cloak.clone())));
                    }
                }
                (false, Some(saved)) => {
                    e.remove::<MeshMaterial3d<CloakMaterial>>();
                    match saved {
                        Swapped::Standard(h) => e.insert(MeshMaterial3d(h.clone())),
                        Swapped::Char(h) => e.insert(MeshMaterial3d(h.clone())),
                        Swapped::Hair(h) => e.insert(MeshMaterial3d(h.clone())),
                        Swapped::Skin(h) => e.insert(MeshMaterial3d(h.clone())),
                        Swapped::Illum(h) => e.insert(MeshMaterial3d(h.clone())),
                    }
                    .remove::<Swapped>();
                }
                _ => {}
            }
            let hide = look == Some(Look::Hidden) || (look == Some(Look::Ghost) && fringe);
            match (hide, cloaked) {
                (true, None) => {
                    e.insert((Cloaked(visibility.copied().unwrap_or_default()), Visibility::Hidden));
                }
                (false, Some(Cloaked(before))) => {
                    e.insert(*before).remove::<Cloaked>();
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illum_equipment_is_cloaked_and_restored_after_the_received_buff_ends() {
        let mut buffs = Buffs::default();
        buffs.apply(&aa_net::Event::BuffCreated { obj_id: 77, buff: aa_net::buff::Buff { index: 1, buff_id: 5, ..default() } });
        let mut app = App::new();
        app.insert_resource(Time::<()>::default()).insert_resource(buffs)
            .insert_resource(Stealth { buffs: HashSet::from([5]), factions: Factions::default(), cloak: Handle::default() })
            .add_systems(Update, reconcile);
        let unit = app.world_mut().spawn((LocalPlayer { obj_id: 77 }, GlobalTransform::IDENTITY)).id();
        let material = Handle::<IllumMaterial>::default();
        let mesh = app.world_mut().spawn((Mesh3d(Handle::default()), MeshMaterial3d(material.clone()), ChildOf(unit))).id();
        app.update();
        assert!(app.world().get::<MeshMaterial3d<IllumMaterial>>(mesh).is_none());
        assert!(app.world().get::<MeshMaterial3d<CloakMaterial>>(mesh).is_some());
        app.world_mut().resource_mut::<Buffs>().apply(&aa_net::Event::BuffRemoved { obj_id: 77, index: 1 });
        app.update();
        assert_eq!(app.world().get::<MeshMaterial3d<IllumMaterial>>(mesh).unwrap().0, material);
        assert!(app.world().get::<MeshMaterial3d<CloakMaterial>>(mesh).is_none());
        assert!(app.world().get::<Swapped>(mesh).is_none());
    }

    #[test]
    fn friendly_follows_the_server_rule() {
        let mut f = Factions::default();
        // 1 friendly to all, 3 hostile to all, 10 and 11 are nations, 20 a player faction under 10, 21 under 11
        f.mother = HashMap::from([(1, 0), (3, 0), (10, 0), (11, 0), (20, 10), (21, 11), (22, 10)]);
        f.relations.insert((10, 11), 1);
        assert!(f.friendly(20, 22), "the same nation");
        assert!(!f.friendly(20, 21), "hostile nations");
        assert!(f.friendly(20, 1), "a root friendly faction");
        assert!(!f.friendly(20, 3), "a root hostile faction");
        assert!(!f.friendly(20, 99), "an unknown faction is not friendly");
    }
}
