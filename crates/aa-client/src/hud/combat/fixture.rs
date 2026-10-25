//! An isolated visual check of the same projection system used by the client.

use std::cell::RefCell;
use std::rc::Rc;

use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::super::{api, data::GameData, plates::PlateState};

#[derive(Component)]
struct Actor(bool);

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let mut rt = UiRuntime::new("en_us", width as f32, height as f32).unwrap();
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB)).unwrap();
    let bridge = Rc::new(RefCell::new(api::HudData::default()));
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Combat fixture".into();
        d.snapshot.level = 1;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        assert!(d.game.0.set(GameData::load(aa_anim::db::DEFAULT_DB).unwrap()).is_ok());
    }
    api::install(&mut rt, &bridge).unwrap();
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    rt.lua
        .load("assert(HUD.combat_text_start==nil); assert(rawget(combatTextFrame.combatTexts[1],'SetUnitId')==nil)")
        .exec()
        .unwrap();
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original combat HUD"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(api::HudBridge(bridge))
        .init_resource::<aa_net::state::Units>()
        .init_resource::<PlateState>()
        .add_systems(Startup, scene)
        .add_systems(
            Update,
            (motion, burst, super::positions::update, verify_projection).chain().before(aa_ui::UiFrame),
        )
        .run();
}

fn scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut plates: ResMut<PlateState>,
    bridge: NonSend<api::HudBridge>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    for mut window in &mut windows {
        let (width, height) = aa_dev::DevArgs::from_env().size;
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 4.0, 11.0).looking_at(Vec3::Y, Vec3::Y)));
    let mesh = meshes.add(Capsule3d::new(0.35, 1.1));
    for (own, id, x, color) in [(true, 100, -2.0, Color::srgb(0.2, 0.5, 0.8)), (false, 200, 2.0, Color::srgb(0.7, 0.3, 0.2))] {
        let entity = commands
            .spawn((Actor(own), Transform::from_xyz(x, 0.0, 0.0), Visibility::default()))
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: color,
                        unlit: true,
                        ..default()
                    })),
                    Transform::from_xyz(0.0, 0.9, 0.0),
                ));
            })
            .id();
        plates.entities.insert(id, entity);
        plates.heights.insert(id, (1.8, 0));
        if own {
            bridge.0.borrow_mut().player_entity = Some(entity);
            plates.entities.insert(201, entity);
            plates.heights.insert(201, (1.8, 0));
        }
    }
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.08, 0.12, 0.14),
            unlit: true,
            ..default()
        })),
    ));
}

fn motion(time: Res<Time>, mut actors: Query<(&Actor, &mut Transform), Without<Camera3d>>, mut cameras: Query<&mut Transform, With<Camera3d>>) {
    let t = time.elapsed_secs();
    for (actor, mut transform) in &mut actors {
        transform.translation.x = if actor.0 { -2.0 } else { 2.0 + (t * 1.2).sin() };
        transform.translation.z = if actor.0 { 0.0 } else { (t * 0.7).sin() };
    }
    for mut transform in &mut cameras {
        *transform = Transform::from_xyz((t * 0.4).sin(), 4.0, 11.0).looking_at(Vec3::Y, Vec3::Y);
    }
}

fn burst(rt: NonSend<UiRuntime>, time: Res<Time>, mut previous: Local<u32>) {
    let beat = (time.elapsed_secs() / 2.0) as u32;
    if beat == 0 || beat == *previous {
        return;
    }
    *previous = beat;
    rt.fire_event("COMBAT_TEXT", ("64", "c8", -128, "SWING", "CRITICAL", 1, false, 4.0));
    rt.fire_event("COMBAT_TEXT", ("64", "c9", -22, "SWING", "HIT", 0, false, 0.0));
    rt.fire_event("UNIT_COMBAT_STATE_CHANGED", (beat % 2 == 1, "64"));
    let visible: usize = rt
        .lua
        .load("local n=0; for _,w in ipairs(combatTextFrame.combatTexts) do if w:IsVisible() then n=n+1 end end; return n")
        .eval()
        .unwrap();
    info!("combat fixture: original displays active={visible}; errors={}", rt.errors.len());
    assert!(visible >= 2);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
}

fn verify_projection(rt: NonSend<UiRuntime>, time: Res<Time>, mut previous: Local<Option<Vec2>>, mut last: Local<f32>) {
    // Keep two fixture IDs requested so we can check current transforms independently of a burst.
    let points: (Option<mlua::Table>, Option<mlua::Table>) = rt
        .lua
        .load("assert(AAUI.CombatTextPosition('64','f423f')==nil); return AAUI.CombatTextPosition('64','c8'), AAUI.CombatTextPosition('64','c9')")
        .eval()
        .unwrap();
    let (Some(target), Some(own)) = points else { return };
    let point = Vec2::new(target.get("x").unwrap(), target.get("y").unwrap());
    assert!(point.x > own.get::<f32>("x").unwrap());
    if time.elapsed_secs() - *last >= 2.0 {
        if let Some(old) = *previous {
            assert!(old.distance(point) > 1.0, "moving unit/camera projection stayed fixed");
        }
        *previous = Some(point);
        *last = time.elapsed_secs();
        info!("combat fixture: moving target projected at {point:?}");
    }
}
