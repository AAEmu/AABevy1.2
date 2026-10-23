//! Original high-ratio rock / externalLOD3 building, with identical before/after cameras.
//! --lod3 selects the building; --unload-check retires the real streamed cell.

use aa_world::objects::lod::BrushLodPart;
use aa_world::objects::{ObjectSector, ObjectState};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, WorldFocus, cry_to_bevy};
use bevy::prelude::*;

#[derive(Resource)]
struct Fixture {
    cell: (i32, i32),
    index: u32,
    target: Vec3,
    offset: Vec3,
    unload: bool,
    transition: bool,
    dissolve: bool,
    stage: u8,
    checked: bool,
    moved: bool,
    done: bool,
    requested: u8,
    stable_cache: Option<usize>,
}
fn flag(name: &str) -> bool {
    std::env::args().any(|arg| arg == name)
}

fn main() {
    let lod3 = flag("--lod3");
    let dissolve=flag("--dissolve-check");
    let cell = if lod3 { (21, 8) } else { (9, 12) };
    let data = aa_world::placement::load_cell_objects("main_world", cell.0, cell.1);
    let index = if lod3 {
        data.brushes
            .iter()
            .position(|b| {
                data.models[b.model as usize].ends_with("/ma_top_house01.cgf")
                    && (900.0..950.0).contains(&b.max_view_dist)
            })
            .expect("original externalLOD3 house")
    } else if dissolve { 6 } else {
        427
    };
    let brush = &data.brushes[index];
    let ratio = brush.lod_ratio;
    let max_view_dist = brush.max_view_dist;
    let path = &data.models[brush.model as usize];
    if !lod3 {
        assert!(path.ends_with(if dissolve {"/pepples_big_a_rock.cgf"}else{"/cliff_rock_a_small.cgf"}));
    }
    let model = aa_cry::Model::load(path).expect("original fixture model");
    let lods: Vec<_> = (0..=model.max_embedded_lod())
        .map(|lod| {
            let triangles: usize = model
                .lod_nodes(lod)
                .iter()
                .map(|&n| model.meshes[model.nodes[n].mesh.unwrap()].indices.len() / 3)
                .sum();
            (lod, triangles)
        })
        .collect();
    let origin = aa_world::coords::cell_origin(cell.0, cell.1);
    let a = Vec3::from_slice(&brush.bbox[..3]);
    let b = Vec3::from_slice(&brush.bbox[3..]);
    let target = origin + (a + b) * 0.5;
    info!(
        "original LOD fixture cell={cell:?} brush={index} model={path} maxview={} world_bbox={:?}..{:?} embedded={lods:?}",
        brush.max_view_dist,
        origin + a,
        origin + b
    );
    if lod3 {
        assert!(aa_pak::Pak::global().exists(&aa_cry::lod_path(path, 3)));
    } else {
        assert!(
            model.max_embedded_lod() > 0,
            "high-ratio rock needs actual alternative geometry"
        );
    }
    if flag("--asset") {
        asset(
            brush.clone(),
            path.clone(),
            usize::try_from(brush.material)
                .ok()
                .and_then(|i| data.materials.get(i))
                .map(|m| format!("{m}.mtl")),
        );
        return;
    }
    let offset = if lod3 {
        Vec3::new(1.0, -1.0, 0.25).normalize() * 900.0
    } else if dissolve {
        Vec3::new((b.x-a.x)*0.5+10.5,0.0,0.0)
    } else {
        Vec3::new(50.0, -65.0, 20.0)
    };
    let mut cfg = WorldConfig::new("main_world");
    cfg.load_radius = 100.0;
    cfg.unload_radius = 220.0;
    cfg.brush_distance = if lod3 { 1200.0 } else { 220.0 };
    if flag("--transition-check") { cfg.brush_distance = 600.0; }
    cfg.water_distance = 150.0;
    cfg.vegetation = false;
    cfg.shadows = false;
    App::new()
        .add_plugins((aa_dev::default_plugins("Native brush LOD"), aa_dev::DevPlugin))
        .add_plugins(AaWorldPlugin { config: cfg })
        .insert_resource(WorldClock { hour: 12.0, ..default() })
        .insert_resource(Fixture { cell, index:index as u32, target, offset, unload: flag("--unload-check"),
            transition:flag("--transition-check"), dissolve, stage:0, checked:false, moved:false, done:false,
            requested:if lod3 {3} else if dissolve {0} else {1}, stable_cache:None })
        .add_systems(Startup, move |mut commands: Commands| {
            info!("original brush LOD fixture cell={cell:?} brush={index} ratio={} maxview={} embedded={lods:?} camera={:?} target={target:?}",
                ratio, max_view_dist,target+offset);
            // Keep the source cell streamed even for the distant geometry test.
            commands.spawn((WorldFocus, Transform::from_translation(cry_to_bevy(target))));
            commands.spawn((Camera3d::default(),
                Transform::from_translation(cry_to_bevy(target + offset)).looking_at(cry_to_bevy(target), Vec3::Y),
                Projection::Perspective(PerspectiveProjection { fov: 60.0_f32.to_radians(), near: 0.1, far: 20000.0, ..default() })));
        })
        .add_systems(Update, check)
        .run();
}

#[allow(clippy::too_many_arguments,clippy::type_complexity)]
fn check(
    time: Res<Time>,
    objects: Res<ObjectState>,
    sectors: Query<Entity, With<ObjectSector>>,
    parts: Query<(&BrushLodPart, &InheritedVisibility)>,
    render_visible: Query<(&BrushLodPart, &ViewVisibility)>,
    dithers: Query<(&BrushLodPart,&aa_world::objects::lod_transition::MeshLodDither)>,
    mut positions: Query<&mut Transform, Or<(With<Camera3d>, With<WorldFocus>)>>,
    mut fixture: ResMut<Fixture>,
    mut next: Local<f32>,
) {
    if time.elapsed_secs() > *next {
        info!(
            "object LOD scene target={:?} sectors={} parts={} pending={} failed={} spawn_ms={} checks={} tracked={} rebuilds={} cached={}",
            fixture.target,
            objects.stats.sectors,
            objects.stats.entities,
            objects.stats.models_pending,
            objects.stats.models_failed,
            objects.stats.spawn_ms,
            objects.stats.brush_lod_checks,
            objects.stats.brush_lod_instances,
            objects.stats.brush_lod_rebuilds,
            objects.stats.models_cached
        );
        *next = time.elapsed_secs() + 2.0;
    }
    assert!(objects.stats.brush_lod_checks <= aa_world::objects::lod::MAX_CHECKS);
    if !fixture.checked && time.elapsed_secs() > 16.0 {
        let levels: Vec<_> = parts
            .iter()
            .filter(|(p, v)| p.cell == fixture.cell && p.index == fixture.index && v.get())
            .map(|(p, _)| p.requested)
            .collect();
        assert!(!levels.is_empty(), "actual original brush parts must load");
        assert!(
            levels.iter().all(|&lod| lod == fixture.requested),
            "native requestedLOD {:?} expected{}",
            levels,
            fixture.requested
        );
        fixture.stable_cache = Some(objects.stats.models_cached);
        fixture.checked = true;
        info!(
            "actual original brush requestedLOD={} PASSED with {} visible parts; camera and streaming focus are separate",
            fixture.requested,
            levels.len()
        );
    }
    if fixture.checked && !fixture.transition && !fixture.dissolve && !fixture.moved && time.elapsed_secs() > 20.0 {
        assert_eq!(
            fixture.stable_cache,
            Some(objects.stats.models_cached),
            "stationary view must not grow cache"
        );
    }
    if fixture.dissolve {
        let move_camera=|positions:&mut Query<&mut Transform,Or<(With<Camera3d>,With<WorldFocus>)>>,offset:f32| {
            for mut t in positions.iter_mut() {
                if t.translation.distance(cry_to_bevy(fixture.target))>1.0 {
                    t.translation=cry_to_bevy(fixture.target+fixture.offset+Vec3::X*offset);
                }
            }
        };
        if fixture.stage==0 && time.elapsed_secs()>18.0 {
            assert!(fixture.checked);move_camera(&mut positions,1.0);fixture.stage=1;
        } else if fixture.stage==1 && time.elapsed_secs()>22.0 {
            let pair:Vec<_>=dithers.iter().filter(|(p,d)|p.cell==fixture.cell && p.index==fixture.index && d.active).collect();
            assert_eq!(pair.len(),2,"actual adjacent original LODs must form one pair");
            move_camera(&mut positions,3.5);fixture.stage=2;
        } else if fixture.stage==2 && time.elapsed_secs()>26.0 {
            let pair:Vec<_>=dithers.iter().filter(|(p,d)|p.cell==fixture.cell && p.index==fixture.index && d.active).map(|(p,d)|(p.requested,d.level)).collect();
            assert_eq!(pair.len(),2);
            assert!(pair.contains(&(0,8)) && pair.contains(&(1,-8)),"half-pair stationary hold: {pair:?}");
            assert!(render_visible.iter().any(|(p,v)|p.cell==fixture.cell && p.index==fixture.index && v.get()));
            info!("actual streamed native camera-distance LOD pair stationary hold PASSED: {pair:?}; global pairs={} extra_parts={}",objects.stats.brush_lod_pairs,objects.stats.brush_lod_extra_parts);
            fixture.stage=3;
        } else if fixture.stage==3 && time.elapsed_secs()>28.0 {
            move_camera(&mut positions,0.0);fixture.stage=4;
        } else if fixture.stage==4 && time.elapsed_secs()>31.0 {
            let pair:Vec<_>=dithers.iter().filter(|(p,_)|p.cell==fixture.cell && p.index==fixture.index).map(|(p,d)|(p.requested,d.active)).collect();
            assert_eq!(pair,vec![(0,false)],"camera reversal retains exactly the original mesh");
            move_camera(&mut positions,1.0);fixture.stage=5;
        } else if fixture.stage==5 && time.elapsed_secs()>34.0 {
            move_camera(&mut positions,7.0);fixture.stage=6;
        } else if fixture.stage==6 && time.elapsed_secs()>37.0 {
            let pair:Vec<_>=dithers.iter().filter(|(p,_)|p.cell==fixture.cell && p.index==fixture.index).map(|(p,d)|(p.requested,d.active)).collect();
            assert_eq!(pair,vec![(1,false)]);
            info!("actual streamed original mesh pair reversal and completion PASSED; old roots retired");fixture.stage=7;
        }
    }
    if fixture.transition {
        if fixture.stage == 0 && time.elapsed_secs() > 18.0 {
            assert!(fixture.checked);
            for mut t in &mut positions {
                if t.translation.distance(cry_to_bevy(fixture.target)) > 1.0 {
                    t.translation = cry_to_bevy(fixture.target + fixture.offset * 4.0);
                }
            }
            fixture.stage = 1;
        } else if fixture.stage == 1 && time.elapsed_secs() > 26.0 {
            let levels: Vec<_> = parts
                .iter()
                .filter(|(p, v)| p.cell == fixture.cell && p.index == fixture.index && v.get())
                .map(|(p, _)| p.requested)
                .collect();
            assert!(!levels.is_empty());
            assert!(
                levels.iter().all(|&lod| lod == 3),
                "camera-only far transition must request3: {levels:?}"
            );
            assert!(render_visible.iter().any(|(p,v)|p.cell==fixture.cell && p.index==fixture.index && p.requested==3 && v.get()),
                "far original brush must be render-visible as well as selected");
            for mut t in &mut positions {
                if t.translation.distance(cry_to_bevy(fixture.target)) > 1.0 {
                    t.translation = cry_to_bevy(fixture.target + fixture.offset);
                }
            }
            fixture.stage = 2;
        } else if fixture.stage == 2 && time.elapsed_secs() > 34.0 {
            let levels: Vec<_> = parts
                .iter()
                .filter(|(p, v)| p.cell == fixture.cell && p.index == fixture.index && v.get())
                .map(|(p, _)| p.requested)
                .collect();
            assert!(!levels.is_empty());
            assert!(levels.iter().all(|&lod| lod == 1));
            info!(
                "actual camera-only brush requestedLOD1 ->3 ->1 transition PASSED; focus unchanged"
            );
            fixture.stage = 3;
        }
    }
    if !fixture.unload || fixture.done {
        return;
    }
    if fixture.dissolve && fixture.stage<7 { return; }
    if !fixture.moved && time.elapsed_secs() > 22.0 {
        assert!(objects.stats.entities > 0);
        for mut transform in &mut positions {
            transform.translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        }
        fixture.moved = true;
    } else if fixture.moved && time.elapsed_secs() > if fixture.dissolve {43.0}else{29.0} {
        assert_eq!(objects.stats.sectors, 0);
        assert!(sectors.is_empty());
        assert!(parts.is_empty());
        assert_eq!(objects.stats.brush_lod_instances, 0);
        info!("object LOD actual cell/sector unload PASSED");
        fixture.done = true;
    }
}

fn asset(brush: aa_world::placement::Brush, path: String, material: Option<String>) {
    let level = std::env::args()
        .position(|a| a == "--level")
        .and_then(|i| std::env::args().nth(i + 1))
        .map(|s| s.parse::<u32>().unwrap())
        .unwrap_or(0);
    let lo = Vec3::from_slice(&brush.bbox[..3]);
    let hi = Vec3::from_slice(&brush.bbox[3..]);
    let centre = (lo + hi) * 0.5;
    let radius = (hi - lo).length() * 0.5;
    App::new().add_plugins((aa_dev::default_plugins("Original brush LOD geometry control"),aa_dev::DevPlugin,aa_cry::CryPlugin))
        .insert_resource(GlobalAmbientLight {brightness:100.0,..default()})
        .add_systems(Startup,move |mut commands:Commands, mut cry:aa_cry::CryAssets| {
            let options=aa_cry::ModelOptions {lod:level,material:material.clone(),keep_cpu_mesh:true,cry_space:true,..default()};
            let loaded=cry.load_model(&path,&options).unwrap();
            let triangles:usize=loaded.parts.iter().flat_map(|(_,parts)|parts).map(|p|cry.meshes.get(&p.mesh).unwrap().indices().unwrap().len()/3).sum();
            info!("original asset geometry level={level} triangles={triangles} file={} model_nodes={} drawn_nodes={} material={:?}",
                if aa_pak::Pak::global().exists(&aa_cry::lod_path(&path,level)){aa_cry::lod_path(&path,level)}else{path.clone()},loaded.model.nodes.len(),loaded.parts.len(),loaded.material_path);
            let root=cry.spawn_model_with(&mut commands,&path,&options).unwrap();
            let matrix=Mat4::from_translation(-cry_to_bevy(centre))*Mat4::from_quat(aa_cry::CRY_TO_BEVY)*brush.mat4();
            commands.entity(root).insert(Transform::from_matrix(matrix));
            commands.spawn((DirectionalLight {illuminance:12000.0,..default()},Transform::from_xyz(-3.0,8.0,5.0).looking_at(Vec3::ZERO,Vec3::Y)));
            commands.spawn((Camera3d::default(),Transform::from_translation(cry_to_bevy(Vec3::new(-1.2,-1.5,0.7).normalize()*radius*2.6)).looking_at(Vec3::ZERO,Vec3::Y)));
            commands.spawn((Text::new(format!("Original asset: requestedLOD{level}, {triangles} drawn triangles\nSame source transform/material and camera for each level; isolated geometry control")),
                TextFont {font_size:FontSize::Px(18.0),..default()},Node {position_type:PositionType::Absolute,top:px(12.0),left:px(12.0),..default()}));
        }).run();
}
