//! Change saved-option distances after a real cell has finished streaming.
//! --stale records the pre-fix regression; --restore also returns to the initial settings.
use aa_world::objects::{ObjectState, ViewLimits, lod::BrushLodPart};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, WorldFocus, cry_to_bevy};
use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;

#[derive(Resource)]
struct Fixture {
    cell: (i32, i32),
    brushes: Vec<aa_world::placement::Brush>,
    stage: u8,
    changed: f32,
    roots: Vec<Entity>,
    stale: bool,
    restore: bool,
}

fn main() {
    let cell = (13, 10);
    let objects = aa_world::placement::load_cell_objects("main_world", cell.0, cell.1);
    let index = objects.brushes.iter().position(|b| {
        objects.models[b.model as usize].ends_with("/castle_nuia_001.cgf")
    }).expect("original palace castle");
    let b = &objects.brushes[index];
    let target = aa_world::coords::cell_origin(cell.0, cell.1)
        + (Vec3::from_slice(&b.bbox[..3]) + Vec3::from_slice(&b.bbox[3..])) * 0.5;
    let mut cfg = WorldConfig::new("main_world");
    cfg.load_radius = 0.0;
    cfg.unload_radius = 100.0;
    cfg.terrain_detail = false;
    cfg.shadows = false;
    info!("draw-distance fixture cell={cell:?} castle={index} target={target:?}");
    App::new()
        .add_plugins((aa_dev::default_plugins("Live world draw distance"), aa_dev::DevPlugin))
        .add_plugins(AaWorldPlugin { config: cfg })
        .insert_resource(WorldClock { hour: 12.0, ..default() })
        .insert_resource(Fixture {
            cell, brushes: objects.brushes, stage: 0, changed: 0.0, roots: Vec::new(),
            stale: std::env::args().any(|a| a == "--stale"),
            restore: std::env::args().any(|a| a == "--restore"),
        })
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((WorldFocus, Transform::from_translation(cry_to_bevy(target))));
            commands.spawn((Camera3d::default(),
                Transform::from_translation(cry_to_bevy(target + Vec3::new(500.0, -600.0, 60.0)))
                    .looking_at(cry_to_bevy(target), Vec3::Y),
                Projection::Perspective(PerspectiveProjection { fov: 45.0_f32.to_radians(), far: 20000.0, ..default() })));
        })
        .add_systems(Update, check.after(aa_world::objects::stream_objects))
        .run();
}

fn check(
    time: Res<Time>, mut fixture: ResMut<Fixture>, mut cfg: ResMut<WorldConfig>,
    state: Res<ObjectState>, streamer: Res<aa_world::stream::Streamer>,
    roots: Query<Entity, With<aa_world::stream::CellRoot>>,
    parts: Query<(&BrushLodPart, &VisibilityRange)>,
) {
    if fixture.stage == 0 {
        if time.elapsed_secs() < 12.0 || streamer.is_loading()
            || state.stats.cells_pending > 0 || state.stats.models_pending > 0
            || state.stats.sectors_incomplete > 0 { return; }
        assert!(parts.iter().any(|(p, _)| p.cell == fixture.cell), "real streamed brush parts");
        fixture.roots = roots.iter().collect();
        cfg.brush_distance = 600.0;
        cfg.view_dist_scale = 0.75;
        cfg.vegetation_distance = 110.0;
        cfg.tree_distance = 500.0;
        fixture.changed = time.elapsed_secs();
        fixture.stage = 1;
        info!("draw-distance changed already-loaded world to low at {:.2}s roots={:?}", fixture.changed, fixture.roots);
        return;
    }
    if fixture.stage == 1 || fixture.stage == 3 {
        if time.elapsed_secs() - fixture.changed < 3.0 { return; }
        let limits = ViewLimits::new(&cfg);
        let mut checked = 0;
        let mut stale = 0;
        for (p, range) in &parts {
            if p.cell != fixture.cell { continue; }
            checked += 1;
            let expected = aa_world::objects::dissolve::range(limits.brush(&fixture.brushes[p.index as usize]).0);
            stale += usize::from(range.end_margin != expected.end_margin);
        }
        assert!(checked > 0);
        assert!(fixture.roots.iter().all(|&e| roots.contains(e)), "settings retain terrain cells");
        info!("draw-distance loaded-world check stage={} parts={checked} stale={stale} cached={} pending={}/{}",
            fixture.stage, state.stats.models_cached, state.stats.models_pending, state.stats.sectors_incomplete);
        if fixture.stale { assert!(stale > 0, "pre-fix regression must reproduce"); }
        else { assert_eq!(stale, 0, "existing geometry must use current distances"); }
        fixture.stage += 1;
    }
    if fixture.stage == 2 && fixture.restore && time.elapsed_secs() - fixture.changed > 6.0 {
        cfg.brush_distance = 2000.0;
        cfg.view_dist_scale = 1.5;
        cfg.vegetation_distance = 220.0;
        cfg.tree_distance = 1000.0;
        fixture.changed = time.elapsed_secs();
        fixture.stage = 3;
        info!("draw-distance restored already-loaded world to high");
    }
}
