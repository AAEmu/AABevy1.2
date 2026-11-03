//! `SCShowQuestArea` (`0x1af`): the native `OnShowQuestArea` (`FUN_391df7b0` -> `FUN_393840b0`)
//! appends `{type, position, radius}` to `DAT_3a173d80` (a `FUN_393833f0` push_back, no replace).
//! `FUN_393822f0` then draws, every frame and without a debug cvar gate, a red (204, 49, 49, 255)
//! sphere of that radius and a `"stype: %u"` label (`FUN_3963b590(type)`). Bevy keeps the same
//! list (capped) and draws the sphere and label in the world.

use aa_net::quest::QuestArea;
use aa_net::{Event, NetEvent};
use bevy::prelude::*;

use super::api::HudBridge;

/// `FUN_3961de30`'s scale: fixed-point X/Y times `_DAT_399f9b80` (2^-44) gives metres.
const CELL: f64 = 1.0 / 17_592_186_044_416.0;
/// The native list grows until the world unloads; keep a bound.
const MAX_AREAS: usize = 256;
const RED: Color = Color::srgb(204.0 / 255.0, 49.0 / 255.0, 49.0 / 255.0);

/// One area as the world draw sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    pub pos: Vec3,
    pub radius: f32,
    pub ty: u32,
}

fn marker(area: &QuestArea) -> Marker {
    Marker {
        pos: Vec3::new((area.x as f64 * CELL) as f32, (area.y as f64 * CELL) as f32, area.z),
        radius: area.radius.max(0.0),
        ty: area.ty,
    }
}

/// The areas as world markers (native order).
fn markers(areas: &[QuestArea]) -> Vec<Marker> {
    areas.iter().map(marker).filter(|m| m.pos.is_finite() && m.radius.is_finite()).collect()
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
}

/// The native per-frame draw (`FUN_393822f0`): a red sphere and the `stype` label per area.
fn draw_areas(
    bridge: NonSend<HudBridge>,
    mut shown: ResMut<Shown>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (version, areas) = {
        let d = bridge.0.borrow();
        (d.quest_area_version, d.quest_areas.clone())
    };
    if shown.seen && shown.version == version {
        return;
    }
    shown.seen = true;
    shown.version = version;
    if let Some(root) = shown.root.take() {
        commands.entity(root).despawn();
    }
    if areas.is_empty() {
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
                alpha_mode: AlphaMode::Blend,
                ..default()
            })
        })
        .clone();
    let root = commands.spawn((Name::new("quest areas"), Transform::default(), Visibility::default())).id();
    shown.root = Some(root);
    for m in markers(&areas) {
        commands.spawn((
            Mesh3d(sphere.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(m.pos).with_scale(Vec3::splat(m.radius)),
            ChildOf(root),
        ));
        commands.spawn((
            Text2d::new(format!("stype: {}", m.ty)),
            TextFont { font_size: bevy::text::FontSize::Px(16.0), ..default() },
            TextColor(RED),
            Transform::from_translation(m.pos + Vec3::Y * (m.radius + 1.0)),
            ChildOf(root),
        ));
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Shown>().add_systems(Update, (track_areas, draw_areas).chain());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::hud::api::HudData;

    #[test]
    fn areas_are_world_markers_at_the_fixed_point_position() {
        let areas = [QuestArea { ty: 9, x: (12.0 * 2f64.powi(44)) as i64, y: (-3.0 * 2f64.powi(44)) as i64, z: 7.5, radius: 30.0 }];
        let m = markers(&areas);
        assert_eq!(m.len(), 1);
        assert!((m[0].pos - Vec3::new(12.0, -3.0, 7.5)).length() < 1e-4);
        assert_eq!((m[0].radius, m[0].ty), (30.0, 9));
        assert!(markers(&[QuestArea { radius: -1.0, ..areas[0] }])[0].radius == 0.0);
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
