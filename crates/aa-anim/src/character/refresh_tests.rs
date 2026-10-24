use super::*;
use crate::facial::{
    FacialAnimator, FacialPlugin, FacialSequence, disable_character_face, enable_character_face,
};
use bevy::ecs::system::SystemState;
use bevy::mesh::morph::MorphWeights;

pub(crate) fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        bevy::asset::AssetPlugin::default(),
        aa_cry::scene::CryPlugin,
        crate::AnimPlugin,
        FacialPlugin,
    ));
    app.init_asset::<Mesh>();
    app.init_asset::<Image>();
    app.init_asset::<StandardMaterial>();
    app.init_asset::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>();
    app
}

pub(crate) fn appearance(items: &[u32]) -> CharacterAssets {
    let db = GameDb::open().unwrap();
    let mut desc = CharacterDesc::default_for(&db, Race::Nuian, Gender::Female).unwrap();
    desc.hair = Some(24024);
    desc.hair_color = None;
    desc.costume = Costume::Items(items.to_vec());
    let detail = items
        .iter()
        .filter_map(|id| db.item_visuals(*id, 11).ok())
        .flatten()
        .find(|v| matches!((v.detail >> 13) & 7, 3 | 7))
        .map(|v| v.detail);
    desc.resolve_with_headgear(&db, detail).unwrap()
}

pub(crate) fn spawn(app: &mut App, assets: CharacterAssets, facial: bool) -> Entity {
    let mut state = SystemState::<(Commands, CryAssets, Res<ClipLibrary>)>::new(app.world_mut());
    let root = {
        let (mut commands, mut cry, clips) = state.get_mut(app.world_mut()).unwrap();
        let spawn = if facial {
            spawn_character_with_facial
        } else {
            spawn_character
        };
        spawn(
            &mut commands,
            &mut cry,
            &clips,
            assets,
            Transform::from_xyz(2.0, 3.0, 4.0),
        )
        .unwrap()
    };
    state.apply(app.world_mut());
    root
}

pub(crate) fn replace(world: &mut World, root: Entity, assets: CharacterAssets) -> Result<()> {
    let animator = world.get::<Animator>(root).unwrap();
    let (rig, bones) = (animator.rig.clone(), animator.bones.clone());
    replace_character_parts(world, root, &rig, &bones, assets)
}

pub(crate) fn subsets(world: &World, root: Entity) -> Vec<Entity> {
    let character = world.get::<Character>(root).unwrap();
    let face = character
        .assets
        .parts
        .iter()
        .position(|p| p.kind == PartKind::Face)
        .unwrap();
    world
        .get::<Children>(character.parts[face])
        .unwrap()
        .iter()
        .filter(|&e| world.get::<Mesh3d>(e).is_some())
        .collect()
}

fn freeze_speech(world: &mut World, root: Entity) {
    let sequence =
        FacialSequence::load("animations/nuian/female/facial/fist_ac_talk_11.fsq").unwrap();
    let mut face = world.get_mut::<FacialAnimator>(root).unwrap();
    face.play(sequence, true);
    face.time = 0.51;
    face.paused = true;
    face.fade_seconds = 0.4;
}

#[derive(Component, Debug, PartialEq)]
struct Gameplay([u64; 6]);

#[test]
fn replacement_preserves_paused_fading_layered_and_parked_animation() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let root = spawn(&mut app, appearance(&[]), true);
    app.update();
    app.world_mut()
        .entity_mut(root)
        .insert(Gameplay([7, 11, 19, 31, 53, 89]));
    let (rig, bones) = {
        let a = app.world().get::<Animator>(root).unwrap();
        (a.rig.clone(), a.bones.clone())
    };
    let clips = app.world().resource::<ClipLibrary>();
    let cal = Cal::shared(&app.world().get::<Character>(root).unwrap().assets.cal).unwrap();
    let bind = |name| {
        let entry = cal.animation(name).unwrap();
        clips
            .bound(
                &rig,
                &entry.path,
                SampleOptions {
                    position_scale: entry.position_scale(cal.base_height),
                    lock_root_motion: true,
                },
            )
            .unwrap()
    };
    let idle = bind(STAGE_IDLE);
    let sit = bind("fist_ac_sit_down_loop");
    let cast = bind("bow_co_sk_cast_ub");
    let configure = |a: &mut Animator| {
        a.random_idle = None;
        a.play(idle.clone(), true, 0.0);
        a.update(0.37);
        a.play_once_then(sit.clone(), idle.clone(), 0.8);
        a.play_layer(
            crate::BoneMask::upper_body(&rig).unwrap(),
            cast.clone(),
            true,
            0.5,
        );
        a.update(0.13);
        a.stop_layer(0.6);
        a.speed = 0.73;
        a.interval = 0.1;
        a.accum = 0.03;
        a.paused = true;
    };
    configure(&mut app.world_mut().get_mut::<Animator>(root).unwrap());
    let mut control = Animator::new(rig.clone(), bones.clone());
    configure(&mut control);
    freeze_speech(app.world_mut(), root);
    let before = app
        .world()
        .get::<Animator>(root)
        .unwrap()
        .playback_snapshot();
    let face_before = app
        .world()
        .get::<FacialAnimator>(root)
        .unwrap()
        .playback_snapshot();
    let weights = app
        .world()
        .get::<MorphWeights>(root)
        .unwrap()
        .weights()
        .to_vec();
    let actor_transform = *app.world().get::<Transform>(root).unwrap();
    let model_root = app.world().get::<Character>(root).unwrap().model_root;
    let rope = app.world().get::<crate::RopeSim>(root).unwrap() as *const _;
    let transforms = bones
        .iter()
        .map(|&e| *app.world().get::<Transform>(e).unwrap())
        .collect::<Vec<_>>();
    replace(app.world_mut(), root, appearance(&[18230, 18180])).unwrap();
    assert_eq!(
        before,
        app.world()
            .get::<Animator>(root)
            .unwrap()
            .playback_snapshot()
    );
    assert_eq!(
        face_before,
        app.world()
            .get::<FacialAnimator>(root)
            .unwrap()
            .playback_snapshot()
    );
    assert_eq!(
        weights,
        app.world().get::<MorphWeights>(root).unwrap().weights()
    );
    assert_eq!(
        actor_transform,
        *app.world().get::<Transform>(root).unwrap()
    );
    assert_eq!(
        model_root,
        app.world().get::<Character>(root).unwrap().model_root
    );
    assert_eq!(
        rope,
        app.world().get::<crate::RopeSim>(root).unwrap() as *const _
    );
    assert_eq!(
        app.world().get::<Gameplay>(root).unwrap(),
        &Gameplay([7, 11, 19, 31, 53, 89])
    );
    for (&e, transform) in bones.iter().zip(&transforms) {
        assert_eq!(*app.world().get::<Transform>(e).unwrap(), *transform);
    }
    app.world_mut().get_mut::<Animator>(root).unwrap().paused = false;
    control.paused = false;
    for dt in [0.04, 0.19, 0.4, 0.3] {
        app.world_mut()
            .get_mut::<Animator>(root)
            .unwrap()
            .update(dt);
        control.update(dt);
        assert_eq!(
            control.playback_snapshot(),
            app.world()
                .get::<Animator>(root)
                .unwrap()
                .playback_snapshot()
        );
    }
    #[derive(Component)]
    struct Parked(Animator);
    let body = app.world_mut().entity_mut(root).take::<Animator>().unwrap();
    let parked_snapshot = body.playback_snapshot();
    app.world_mut().entity_mut(root).insert(Parked(body));
    replace_character_parts(app.world_mut(), root, &rig, &bones, appearance(&[22874])).unwrap();
    assert!(app.world().get::<Animator>(root).is_none());
    assert_eq!(
        parked_snapshot,
        app.world()
            .get::<Parked>(root)
            .unwrap()
            .0
            .playback_snapshot()
    );
    assert_eq!(
        face_before,
        app.world()
            .get::<FacialAnimator>(root)
            .unwrap()
            .playback_snapshot()
    );
}

#[test]
fn failed_replacements_leave_complete_old_actor_and_face_intact() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let root = spawn(&mut app, appearance(&[22874]), false);
    enable_character_face(app.world_mut(), root).unwrap();
    freeze_speech(app.world_mut(), root);
    app.update();
    let character = app.world().get::<Character>(root).unwrap().clone();
    let before = app
        .world()
        .get::<Animator>(root)
        .unwrap()
        .playback_snapshot();
    let face = app
        .world()
        .get::<FacialAnimator>(root)
        .unwrap()
        .playback_snapshot();
    let transform = *app.world().get::<Transform>(character.model_root).unwrap();
    let count = app.world().entities().len();
    let mut failures = Vec::new();
    let mut missing = appearance(&[18230]);
    missing.parts.last_mut().unwrap().path = "objects/missing_refresh.chr".into();
    failures.push(missing);
    let mut material = appearance(&[22874]);
    material.parts.last_mut().unwrap().material = Some("objects/missing_refresh.mtl".into());
    failures.push(material);
    let mut cal = appearance(&[]);
    cal.cal.push_str(".wrong");
    failures.push(cal);
    let mut model = appearance(&[]);
    model.model_id = 10;
    failures.push(model);
    let mut race = appearance(&[]);
    race.desc.race = Race::Elf;
    failures.push(race);
    let mut gender = appearance(&[]);
    gender.desc.gender = Gender::Male;
    failures.push(gender);
    let mut skeleton = appearance(&[]);
    skeleton.skeleton.push_str(".wrong");
    failures.push(skeleton);
    let mut heel = appearance(&[]);
    heel.heel = f32::NAN;
    failures.push(heel);
    let mut no_face = appearance(&[]);
    no_face.parts.retain(|p| p.kind != PartKind::Face);
    failures.push(no_face);
    let mut bad_decal = appearance(&[]);
    bad_decal.face_layers[0].path = "objects/missing_refresh.dds".into();
    failures.push(bad_decal);
    for (case, assets) in failures.into_iter().enumerate() {
        assert!(
            replace(app.world_mut(), root, assets).is_err(),
            "failure case {case}"
        );
        let old = app.world().get::<Character>(root).unwrap();
        assert!(Arc::ptr_eq(&old.assets, &character.assets));
        assert_eq!(old.parts, character.parts);
        assert_eq!(count, app.world().entities().len());
        assert_eq!(
            before,
            app.world()
                .get::<Animator>(root)
                .unwrap()
                .playback_snapshot()
        );
        assert_eq!(
            face,
            app.world()
                .get::<FacialAnimator>(root)
                .unwrap()
                .playback_snapshot()
        );
        assert_eq!(
            transform,
            *app.world().get::<Transform>(old.model_root).unwrap()
        );
    }
    assert!(disable_character_face(app.world_mut(), root));
}

#[test]
fn original_appearance_hair_costume_and_heel_refresh_in_both_directions() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let root = spawn(&mut app, appearance(&[]), false);
    let model = app.world().get::<Character>(root).unwrap().model_root;
    app.world_mut()
        .get_mut::<Transform>(model)
        .unwrap()
        .translation += Vec3::new(0.12, 0.23, 0.34);
    let baseline = *app.world().get::<Transform>(model).unwrap();
    for (items, heel, hair, body) in [
        (vec![22874], 0.0, true, true),
        (vec![18230, 18180], 0.09, false, true),
        (vec![18180, 8000028], 0.08, true, false),
        (vec![28439], 0.0, true, false),
        (vec![30721], 0.0, true, true),
        (vec![], 0.0, true, true),
    ] {
        let expected = appearance(&items);
        let old_parts = app.world().get::<Character>(root).unwrap().parts.clone();
        replace(app.world_mut(), root, expected.clone()).unwrap();
        let character = app.world().get::<Character>(root).unwrap();
        assert_eq!(character.assets.parts, expected.parts);
        assert_eq!(character.parts.len(), expected.parts.len());
        assert_eq!(
            character
                .assets
                .parts
                .iter()
                .any(|p| p.kind == PartKind::Hair),
            hair
        );
        assert_eq!(
            character
                .assets
                .parts
                .iter()
                .any(|p| p.kind == PartKind::Body),
            body
        );
        assert_eq!(character.assets.underwear, expected.underwear);
        let actual = *app.world().get::<Transform>(model).unwrap();
        assert!((actual.translation.y - baseline.translation.y - heel).abs() < 1e-6);
        assert_eq!(
            (
                actual.translation.x,
                actual.translation.z,
                actual.rotation,
                actual.scale
            ),
            (
                baseline.translation.x,
                baseline.translation.z,
                baseline.rotation,
                baseline.scale
            )
        );
        assert!(
            old_parts
                .iter()
                .all(|&e| app.world().get_entity(e).is_err())
        );
    }
}
