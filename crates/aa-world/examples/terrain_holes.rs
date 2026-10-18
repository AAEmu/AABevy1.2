//! Native original hole leaf: numeric two-view controls, or --world [--filled-control]
//! [--terrain-only] [--unload-check]. --lod 0..4 checks the native LOD-centre rule.
use std::collections::HashSet;
use aa_world::{heightmap::CellHeights, terrain::build_patch_mesh, stream::{Streamer, TerrainPatch}};
use bevy::{asset::AssetId, camera::Viewport,
    core_pipeline::tonemapping::{DebandDither, Tonemapping}, prelude::*,
    render::view::screenshot::ScreenshotCaptured};

fn flag(s: &str) -> bool { std::env::args().any(|a| a == s) }
fn lod() -> usize {
    let args: Vec<_> = std::env::args().collect();
    args.iter().position(|a| a == "--lod").and_then(|i| args.get(i+1))
        .and_then(|s| s.parse().ok()).unwrap_or(0).min(4)
}
fn heights() -> CellHeights {
    CellHeights::parse(&aa_pak::Pak::global().read("worlds/main_world/cells/020_008/client/terrain/heightmap.dat").unwrap()).unwrap()
}
#[derive(Resource, Default)]
struct State {
    entities: HashSet<Entity>, meshes: HashSet<AssetId<Mesh>>,
    moved: bool, verified: bool, last: f32,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("Native terrain holes"),aa_dev::DevPlugin));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 180.0; config.unload_radius = 300.0;
        config.brush_distance = 280.0; config.vegetation = false;
        config.objects = !flag("--terrain-only");
        config.lod_updates_per_frame = 0; // fixed inspection: controls replace only the initial meshes
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock { hour: 10.0,..default() })
            .init_resource::<State>()
            .add_systems(Startup,world_camera).add_systems(Update,world_check);
    } else {
        app.insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup,numeric).add_observer(pixels);
    }
    app.run();
}
fn numeric(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>, windows: Query<&Window>) {
    let window = windows.single().unwrap();
    let size = UVec2::new(window.physical_width(),window.physical_height());
    let mut h = heights();
    let red = materials.add(StandardMaterial { base_color: Color::srgb_u8(210,40,40),unlit:true,..default() });
    let green = materials.add(StandardMaterial { base_color: Color::srgb_u8(30,180,50),unlit:true,..default() });
    for i in 0..2 {
        if i == 1 { h.hole_sectors.fill(false); }
        let x = i as f32 * 1000.0;
        commands.spawn((Mesh3d(meshes.add(build_patch_mesh(&h,2,3,lod()))),MeshMaterial3d(red.clone()),Transform::from_xyz(x,0.0,0.0)));
        commands.spawn((Mesh3d(meshes.add(Plane3d::default().mesh().size(256.0,256.0))),MeshMaterial3d(green.clone()),Transform::from_xyz(x+128.0,0.0,-128.0)));
        commands.spawn((Camera3d::default(),Camera { order:i as isize,
            viewport:Some(Viewport {physical_position:UVec2::new(i*size.x/2,0),physical_size:UVec2::new(size.x/2,size.y),..default()}),..default()},
            Tonemapping::None,DebandDither::Disabled,Msaa::Off,
            Projection::Orthographic(OrthographicProjection { scaling_mode:bevy::camera::ScalingMode::FixedVertical {viewport_height:16.0},..OrthographicProjection::default_3d() }),
            Transform::from_xyz(x+212.25,800.0,-66.5).looking_at(Vec3::new(x+212.25,0.0,-66.5),Vec3::NEG_Z)));
    }
    info!("Original 20,8 leaf, native/control LOD{}: expected native hole={}",lod(),lod()<3);
}
fn pixels(shot: On<ScreenshotCaptured>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgba8();
    let (w,h) = image.dimensions();
    for i in 0..2 {
        let actual = image.get_pixel(w/4+i*w/2,h/2).0;
        let expected = if i==0 && lod()<3 {[30,180,50]} else {[210,40,40]};
        for c in 0..3 { assert!((actual[c] as i32-expected[c] as i32).abs()<=2,"LOD{} view{i}: GPU {actual:?} expected {expected:?}",lod()); }
        info!("Native terrain-hole pixel LOD{} view{i}: {actual:?} expected {expected:?}",lod());
    }
}
fn vector(name: &str) -> Option<Vec3> {
    let a:Vec<_>=std::env::args().collect();
    let s=a.get(a.iter().position(|s|s==name)?+1)?;
    let p:Vec<f32>=s.split(',').map(str::parse).collect::<Result<_,_>>().ok()?;
    (p.len()==3 && p.iter().all(|v|v.is_finite())).then(||Vec3::from_slice(&p))
}
fn world_camera(mut commands:Commands) {
    let eye=vector("--pos").unwrap_or(Vec3::new(21207.0,9000.0,470.0));
    let target=vector("--look").unwrap_or(Vec3::new(21207.0,9029.0,453.0));
    commands.spawn((Camera3d::default(),Transform::from_translation(aa_world::cry_to_bevy(eye))
        .looking_at(aa_world::cry_to_bevy(target),Vec3::Y),
        Projection::Perspective(PerspectiveProjection {near:0.1,far:20000.0,fov:60_f32.to_radians(),..default()})));
}
fn world_check(time:Res<Time>,mut state:ResMut<State>,terrain:Res<aa_world::TerrainQuery>,
    streamer:Res<Streamer>,mut patches:Query<(Entity,&TerrainPatch,&mut Mesh3d)>,
    mut meshes:ResMut<Assets<Mesh>>,mut camera:Query<&mut Transform,With<Camera3d>>) {
    for (entity,patch,mut mesh) in &mut patches {
        state.meshes.insert(mesh.id());
        if state.entities.insert(entity) && flag("--filled-control") {
            let mut h=(*terrain.cell(patch.cell.0,patch.cell.1).unwrap()).clone();
            h.hole_sectors.fill(false);
            mesh.0=meshes.add(build_patch_mesh(&h,patch.index%4,patch.index/4,patch.lod));
            state.meshes.insert(mesh.id());
        }
    }
    if time.elapsed_secs()>state.last+2.0 {
        state.last=time.elapsed_secs();
        info!("Terrain holes: loaded={} patches={} mesh_assets={}",streamer.loaded_count(),patches.iter().count(),state.meshes.len());
    }
    if flag("--unload-check") && !state.moved && time.elapsed_secs()>12.0 {
        assert!(!state.entities.is_empty());
        assert_eq!(terrain.is_hole_at(21204.25,9026.5),Some(true));
        assert!(terrain.ground_at(21204.25,9026.5).is_none());
        camera.single_mut().unwrap().translation=aa_world::cry_to_bevy(Vec3::new(-100000.0,-100000.0,1000.0));
        state.moved=true;
    }
    if state.moved && !state.verified && time.elapsed_secs()>17.0 {
        assert_eq!(streamer.loaded_count(),0); assert!(!streamer.is_loading());
        assert!(patches.is_empty());
        assert!(state.meshes.iter().all(|id|meshes.get(*id).is_none()));
        state.verified=true;
        info!("Terrain-hole unload PASSED: cells/tasks/patches/owned mesh assets = 0");
    }
}
