//! Stealth: what a unit under a `buffs.stealth` buff looks like.
//!
//! The real client draws the player's own character as a translucent pink-red hologram (a rim glow, the shape of the body still
//! readable) and does not draw other stealthed units at all. The server only says which buffs a unit has
//! (`aa_net::state::Buffs`), so this reconciles the look with that list: the meshes of the local player swap to one shared
//! translucent material (and get their own back when the buff goes), the meshes of a stealthed remote unit are hidden.
//! There is no faction data on the client, so a stealthed ally disappears too (the real client shows them to allies).

use std::collections::HashSet;

use aa_cry::shade::backlight::CharMaterial;
use aa_net::state::Buffs;
use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags};

use super::player::LocalPlayer;
use super::units::RemoteUnit;

/// The seconds between two looks at the meshes of a stealthed unit (equipment changes bring new ones).
const RESCAN: f64 = 0.5;

/// `buffs` rows with `stealth`, and the material the local player is drawn with.
#[derive(Resource)]
struct Stealth {
    buffs: HashSet<u32>,
    ghost: Handle<StandardMaterial>,
}

/// On a unit that is drawn stealthed.
#[derive(Component)]
struct Stealthed {
    scanned: f64,
}

/// On a mesh of the local player that wears the ghost material: what it wore before.
#[derive(Component)]
enum Swapped {
    Standard(Handle<StandardMaterial>),
    Char(Handle<CharMaterial>),
}

/// On a mesh of a stealthed remote unit: its visibility before.
#[derive(Component)]
struct Cloaked(Visibility);

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, load).add_systems(Update, (reconcile, shimmer).chain().run_if(resource_exists::<Stealth>));
}

fn load(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let ids = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
        .and_then(|conn| {
            let mut q = conn.prepare("SELECT id FROM buffs WHERE stealth = 't'")?;
            q.query_map([], |r| r.get::<_, i64>(0).map(|id| id as u32))?.collect::<Result<HashSet<_>, _>>()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return;
    }
    let ghost = materials.add(StandardMaterial {
        base_color: Color::srgba(0.9, 0.08, 0.26, 0.45),
        emissive: LinearRgba::rgb(0.3, 0.03, 0.12),
        perceptual_roughness: 0.4,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    commands.insert_resource(Stealth { buffs: ids, ghost });
}

/// The ghost flickers a little.
fn shimmer(time: Res<Time>, stealth: Res<Stealth>, mut materials: ResMut<Assets<StandardMaterial>>, units: Query<(), With<Stealthed>>) {
    if units.is_empty() {
        return;
    }
    let t = time.elapsed_secs();
    let wave = (t * 9.0).sin() * 0.5 + (t * 23.0).sin() * 0.25;
    if let Some(mut m) = materials.get_mut(&stealth.ghost) {
        m.base_color = Color::srgba(0.9, 0.08, 0.26, 0.45 + 0.07 * wave);
        let glow = 0.3 + 0.12 * wave;
        m.emissive = LinearRgba::rgb(glow, 0.1 * glow, 0.4 * glow);
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    time: Res<Time>,
    stealth: Res<Stealth>,
    buffs: Option<Res<Buffs>>,
    units: Query<(Entity, Option<&RemoteUnit>, Option<&LocalPlayer>, Option<&Stealthed>)>,
    children: Query<&Children>,
    meshes: Query<(Option<&MeshMaterial3d<StandardMaterial>>, Option<&MeshMaterial3d<CharMaterial>>, Option<&Swapped>, Option<&Cloaked>, Option<&Visibility>), With<Mesh3d>>,
) {
    let Some(buffs) = buffs else { return };
    let now = time.elapsed_secs_f64();
    for (unit, remote, local, state) in &units {
        let Some(obj) = remote.map(|r| r.obj_id).or(local.map(|l| l.obj_id)) else { continue };
        let wanted = buffs.of(obj).iter().any(|b| stealth.buffs.contains(&b.buff.buff_id));
        match (wanted, state) {
            (false, None) => continue,
            (true, Some(s)) if now - s.scanned < RESCAN => continue,
            _ => {}
        }
        if wanted {
            commands.entity(unit).insert(Stealthed { scanned: now });
        } else {
            commands.entity(unit).remove::<Stealthed>();
        }
        for mesh in children.iter_descendants(unit) {
            let Ok((standard, char, swapped, cloaked, visibility)) = meshes.get(mesh) else { continue };
            let mut e = commands.entity(mesh);
            if local.is_some() {
                match (wanted, swapped) {
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
            } else {
                match (wanted, cloaked) {
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
}
