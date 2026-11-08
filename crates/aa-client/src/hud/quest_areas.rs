//! `SCShowQuestArea` (`0x1af`): the native `OnShowQuestArea` (`FUN_391df7b0` -> `FUN_393840b0`)
//! appends `{type, position, radius}` to `DAT_3a173d80` (a `FUN_393833f0` push_back, no replace).
//! `FUN_393822f0` then draws, every frame and without a debug-cvar gate, a red (204, 49, 49, 255)
//! sphere of that radius plus a white `stype: %u` label: `FUN_3963b590(type)` looks the type up in
//! the `spheres` table (`+0x9db4`) and only a found record names the first field.
//!
//! The server never sends 0x1af, so the drawing is a dev flag (`--quest-area-geometry`), off by
//! default; the list is still kept. Labels sit on the HUD layer in screen coordinates like the
//! other world labels (`plates::LAYER`), the sphere is world geometry with `cry_to_bevy`.

use std::collections::HashSet;

use aa_net::quest::QuestArea;
use aa_net::{Event, NetEvent};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use super::api::HudBridge;

/// `FUN_3961de30`'s scale: fixed-point X/Y times `_DAT_399f9b80` (2^-44) gives metres.
const CELL: f64 = 1.0 / 17_592_186_044_416.0;
/// The native list grows until the world unloads; keep a bound.
const MAX_AREAS: usize = 256;
const RED: Color = Color::srgb(204.0 / 255.0, 49.0 / 255.0, 49.0 / 255.0);

/// One area as the native list holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    /// Cry-space position converted with [`aa_world::cry_to_bevy`].
    pub pos: Vec3,
    pub radius: f32,
    pub ty: u32,
    /// `stype: N` only when `FUN_3963b590(ty)` finds a `spheres` row.
    pub label: Option<String>,
}

fn marker(area: &QuestArea, known: &HashSet<u32>) -> Marker {
    let cry = Vec3::new((area.x as f64 * CELL) as f32, (area.y as f64 * CELL) as f32, area.z);
    Marker {
        pos: aa_world::cry_to_bevy(cry),
        radius: area.radius.max(0.0),
        ty: area.ty,
        label: known.contains(&area.ty).then(|| format!("stype: {}", area.ty)),
    }
}

/// The areas as markers (native order).
fn markers(areas: &[QuestArea], known: &HashSet<u32>) -> Vec<Marker> {
    areas.iter().map(|a| marker(a, known)).filter(|m| m.pos.is_finite() && m.radius.is_finite()).collect()
}

/// `OnShowQuestArea`: append, never replace (`FUN_393833f0`).
fn track_areas(mut events: MessageReader<NetEvent>, bridge: NonSend<HudBridge>) {
    for NetEvent(event) in events.read() {
        let Event::QuestArea(area) = event else { continue };
        let mut d = bridge.0.borrow_mut();
        if d.quest_areas.len() >= MAX_AREAS {
            d.quest_areas.remove(0);
        }
        d.quest_areas.push(*area);
        d.quest_area_version = d.quest_area_version.wrapping_add(1);
    }
}

#[derive(Resource, Default)]
struct Shown {
    version: u64,
    seen: bool,
    root: Option<Entity>,
    sphere: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
    /// World labels and their cry-to-bevy world anchor.
    labels: Vec<(Entity, Vec3)>,
}

/// Read once: `aa_dev::args` collects the command line (on the web it parses the page URL).
fn area_geometry_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| aa_dev::args().iter().any(|a| a == "--quest-area-geometry"))
}

/// The native per-frame draw (`FUN_393822f0`): a red sphere and the `stype` label per area.
#[allow(clippy::type_complexity)]
fn draw_areas(
    bridge: NonSend<HudBridge>,
    mut shown: ResMut<Shown>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    font: Res<super::plates::PlateFont>,
    cameras: Query<(&Camera, &Transform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
) {
    let version = bridge.0.borrow().quest_area_version;
    if !shown.seen || shown.version != version {
        shown.seen = true;
        shown.version = version;
        if let Some(root) = shown.root.take() {
            commands.entity(root).despawn();
        }
        shown.labels.clear();
        // only on a change: the markers are built inside the borrow, nothing is cloned per frame
        let list = {
            let d = bridge.0.borrow();
            let none = HashSet::new();
            markers(&d.quest_areas, d.game.get().map_or(&none, |g| &g.spheres))
        };
        if list.is_empty() {
            return;
        }
        let sphere = shown.sphere.get_or_insert_with(|| meshes.add(Sphere::new(1.0).mesh().uv(24, 16))).clone();
        let material = shown
            .material
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: RED,
                    unlit: true,
                    cull_mode: None,
                    ..default()
                })
            })
            .clone();
        let root = commands.spawn((Name::new("quest areas"), Transform::default(), Visibility::default())).id();
        shown.root = Some(root);
        for m in list {
            commands.spawn((
                Mesh3d(sphere.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(m.pos).with_scale(Vec3::splat(m.radius)),
                ChildOf(root),
            ));
            if let Some(text) = m.label {
                let e = commands
                    .spawn((
                        Text2d::new(text),
                        TextFont { font: FontSource::Handle(font.0.clone()), font_size: bevy::text::FontSize::Px(16.0), ..default() },
                        TextColor(Color::WHITE),
                        Transform::default(),
                        Visibility::Hidden,
                        RenderLayers::layer(super::plates::LAYER),
                        ChildOf(root),
                    ))
                    .id();
                shown.labels.push((e, m.pos + Vec3::Y * (m.radius + 1.0)));
            }
        }
    }
    // The labels live in screen coordinates on the HUD layer (`plates.rs` / `loot.rs`).
    let Some((camera, camera_transform)) = cameras.iter().find(|(c, _)| c.is_active) else { return };
    let Some(viewport) = camera.logical_viewport_size() else { return };
    let camera_transform = GlobalTransform::from(*camera_transform);
    for (e, at) in &shown.labels {
        match camera.world_to_viewport(&camera_transform, *at) {
            Ok(screen) => {
                let x = (screen.x - viewport.x / 2.0).round();
                let y = (viewport.y / 2.0 - screen.y).round();
                commands.entity(*e).insert((Transform::from_xyz(x, y, 1.0), Visibility::Visible));
            }
            Err(_) => {
                commands.entity(*e).insert(Visibility::Hidden);
            }
        }
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Shown>()
        .add_systems(Update, (track_areas, draw_areas.run_if(area_geometry_enabled)).chain());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::hud::api::HudData;

    #[test]
    fn areas_are_bevy_markers_at_the_fixed_point_position() {
        let areas = [QuestArea { ty: 9, x: (12.0 * 2f64.powi(44)) as i64, y: (-3.0 * 2f64.powi(44)) as i64, z: 7.5, radius: 30.0 }];
        let m = markers(&areas, &HashSet::from([9]));
        assert_eq!(m.len(), 1);
        assert!((m[0].pos - aa_world::cry_to_bevy(Vec3::new(12.0, -3.0, 7.5))).length() < 1e-4);
        assert_eq!((m[0].radius, m[0].ty), (30.0, 9));
        assert_eq!(m[0].label.as_deref(), Some("stype: 9"));
        assert_eq!(markers(&areas, &HashSet::new())[0].label, None);
        assert!(markers(&[QuestArea { radius: -1.0, ..areas[0] }], &HashSet::new())[0].radius == 0.0);
    }

    #[test]
    fn repeated_areas_are_appended_not_replaced() {
        let shared = Rc::new(RefCell::new(HudData::default()));
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .insert_non_send(HudBridge(shared.clone()))
            .add_systems(Update, track_areas);
        let area = QuestArea { ty: 4, x: 1, y: 2, z: 3.0, radius: 5.0 };
        app.world_mut().write_message(NetEvent(Event::QuestArea(area)));
        app.update();
        app.world_mut().write_message(NetEvent(Event::QuestArea(area)));
        app.update();
        let d = shared.borrow();
        assert_eq!(d.quest_areas.len(), 2, "FUN_393833f0 push_back");
        assert_eq!(d.quest_area_version, 2);
    }
}
