//! Stealth: what a unit under a `buffs.stealth` buff looks like.
//!
//! The real client draws the player's own character as a translucent pink-red hologram (a rim glow, the shape of the body still
//! readable) and does not draw stealthed enemies at all. The server only says which buffs a unit has (`aa_net::state::Buffs`), so
//! this reconciles the look with that list: the meshes of a unit that is friendly to the player swap to one shared translucent
//! material (and get their own back when the buff goes), the meshes of a stealthed enemy are hidden. Friendly is the rule of the
//! target frame's colours (`hud::data::GameData::relation`, private to the hud: `system_factions` / `system_faction_relations`),
//! so party members and allies stay visible; the local player always is.

use std::collections::{HashMap, HashSet};

use aa_cry::shade::backlight::CharMaterial;
use aa_net::state::{Buffs, Player, Units};
use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags};

use super::player::LocalPlayer;
use super::units::RemoteUnit;

/// The seconds between two looks at the meshes of a stealthed unit (equipment changes bring new ones).
const RESCAN: f64 = 0.5;

/// `buffs` rows with `stealth`, the factions, and the material a visible stealthed unit is drawn with.
#[derive(Resource)]
struct Stealth {
    buffs: HashSet<u32>,
    factions: Factions,
    ghost: Handle<StandardMaterial>,
}

/// How a stealthed unit is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    /// The translucent ghost: the local player and the units friendly to it.
    Ghost,
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
}

/// On a mesh of a hidden unit: its visibility before.
#[derive(Component)]
struct Cloaked(Visibility);

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, load).add_systems(Update, (reconcile, shimmer).chain().run_if(resource_exists::<Stealth>));
}

fn load(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
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
    let ghost = materials.add(StandardMaterial {
        base_color: Color::srgba(0.45, 0.01, 0.1, 0.55),
        emissive: LinearRgba::rgb(0.3, 0.03, 0.12),
        perceptual_roughness: 0.4,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.insert_resource(Stealth { buffs: ids, factions, ghost });
}

/// The ghost flickers a little.
fn shimmer(time: Res<Time>, stealth: Res<Stealth>, mut materials: ResMut<Assets<StandardMaterial>>, units: Query<&Stealthed>) {
    if !units.iter().any(|s| s.look == Look::Ghost) {
        return;
    }
    let t = time.elapsed_secs();
    let wave = (t * 9.0).sin() * 0.5 + (t * 23.0).sin() * 0.25;
    if let Some(mut m) = materials.get_mut(&stealth.ghost) {
        m.base_color = Color::srgba(0.45, 0.01, 0.1, 0.55 + 0.07 * wave);
        let glow = 0.25 + 0.1 * wave;
        // the real cloak crackles with blue sparks: short blue flashes now and then
        let spark = ((t * 47.0).sin() * (t * 31.0).sin() - 0.75).max(0.0) * 6.0;
        m.emissive = LinearRgba::rgb(glow, 0.1 * glow, 0.4 * glow + spark);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    time: Res<Time>,
    stealth: Res<Stealth>,
    buffs: Option<Res<Buffs>>,
    player: Option<Res<Player>>,
    net_units: Option<Res<Units>>,
    units: Query<(Entity, Option<&RemoteUnit>, Option<&LocalPlayer>, Option<&Stealthed>)>,
    children: Query<&Children>,
    meshes: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<CharMaterial>>, Option<&Swapped>, Option<&Cloaked>, Option<&Visibility>), With<Mesh3d>>,
) {
    let Some(buffs) = buffs else { return };
    let own_faction = player.as_ref().and_then(|p| p.spawn.as_ref()).map_or(0, |s| s.character().faction_id);
    let now = time.elapsed_secs_f64();
    for (unit, remote, local, state) in &units {
        let Some(obj) = remote.map(|r| r.obj_id).or(local.map(|l| l.obj_id)) else { continue };
        let stealthed = buffs.of(obj).iter().any(|b| stealth.buffs.contains(&b.buff.buff_id));
        let look = stealthed.then(|| {
            let friendly = remote.is_none() || net_units.as_ref().and_then(|u| u.by_id.get(&obj)).is_some_and(|u| stealth.factions.friendly(own_faction, u.faction_id));
            if friendly { Look::Ghost } else { Look::Hidden }
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
        for mesh in children.iter_descendants(unit) {
            let Ok((standard, char, swapped, cloaked, visibility)) = meshes.get(mesh) else { continue };
            let mut e = commands.entity(mesh);
            match (look == Some(Look::Ghost), swapped) {
                (true, None) => {
                    // a material of the character: anything else (an effect) keeps its look
                    if let Some(m) = standard {
                        e.insert((Swapped::Standard(m.0.clone()), MeshMaterial3d(stealth.ghost.clone())));
                    } else if let Some(m) = char {
                        e.remove::<MeshMaterial3d<CharMaterial>>().insert((Swapped::Char(m.0.clone()), MeshMaterial3d(stealth.ghost.clone())));
                    }
                }
                (false, Some(saved)) => {
                    match saved {
                        Swapped::Standard(h) => e.insert(MeshMaterial3d(h.clone())),
                        Swapped::Char(h) => e.remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(h.clone())),
                    }
                    .remove::<Swapped>();
                }
                _ => {}
            }
            match (look == Some(Look::Hidden), cloaked) {
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
