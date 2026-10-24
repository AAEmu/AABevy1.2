//! Original assets through the production remote receipt/refresh path; no socket.
use super::*;
use aa_net::{
    Event,
    model::{EquipDetail, Item, slot},
};

#[derive(Resource, Default)]
struct Fixture {
    before: bool,
    changed: bool,
    parked: bool,
    verified: bool,
    actors: Vec<(Entity, Entity)>,
}
fn item(template: u32) -> Item {
    Item {
        id: u64::from(template) + 9_007_199_254_740_993,
        template_id: template,
        count: 1,
        detail: ItemDetail::Equip(EquipDetail::default()),
        ..default()
    }
}
pub(crate) fn run() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Received character gear refresh"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_fx::FxPlugin,
        ))
        .init_state::<crate::Screen>()
        .insert_state(crate::Screen::InGame)
        .insert_resource(Fixture {
            before: std::env::args().any(|a| a == "--before"),
            ..default()
        })
        .insert_resource(Db::open().unwrap())
        .init_resource::<Units>()
        .add_plugins((plugin, super::super::attachments::plugin))
        .configure_sets(
            Update,
            Refresh.run_if(|fixture: Res<Fixture>| !fixture.before),
        )
        .insert_resource(GlobalAmbientLight {
            brightness: 600.0,
            ..default()
        })
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                receipts.before(refresh),
                weapons::attach_carried.in_set(weapons::InGameCarried),
                weapons::hold_weapons,
            ),
        )
        .add_systems(Last, verify)
        .run();
}
fn setup(world: &mut World) {
    world.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.25, 5.5).looking_at(Vec3::new(0.0, 0.9, 0.0), Vec3::Y),
    ));
    world.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, -0.5, 0.0)),
    ));
    let db = world.resource::<Db>().clone();
    for i in 0..3u32 {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = 201 + i;
        unit.kind = aa_net::model::UnitKind::Character { char_id: 301 + i };
        unit.model_id = 11;
        unit.pos = aa_net::Pos::new((i as f32 - 1.0) * 1.25, 0.0, 0.0);
        unit.rotation[2] = std::f32::consts::PI;
        for (s, template) in [
            (slot::HAIR, 24024),
            (slot::CHEST, 23387),
            (slot::LEGS, 23388),
            (slot::FEET, 23390),
            (slot::MAINHAND, 26060),
        ] {
            unit.equipment.slots[s] = Some(item(template));
        }
        if i == 1 {
            unit.equipment.slots[slot::COSPLAY] = Some(item(14359));
        }
        if i == 2 {
            unit.equipment.slots[slot::COSPLAY] = Some(item(34276));
        }
        let mut state = SystemState::<(Commands, CryAssets, Res<aa_anim::ClipLibrary>)>::new(world);
        let root = {
            let (mut commands, mut cry, clips) = state.get_mut(world).unwrap();
            let build = super::super::prepare::prepare_unit(&db, &cry.loader(), &clips, &unit);
            super::super::units::spawn_unit(&mut commands, &mut cry, &clips, &unit, build).unwrap()
        };
        state.apply(world);
        {
            // Sample the exact same skeletal pose before parking or replacing
            // parts. Pausing first would retain a frame-dependent prior pose.
            let mut animator = world.get_mut::<Animator>(root).unwrap();
            animator.speed = 0.0;
            animator.random_idle = None;
            if let Some(current) = animator.current.as_mut() {
                current.time = 0.4;
            }
        }
        aa_anim::facial::enable_character_face(world, root).unwrap();
        world.entity_mut(root).insert(weapons::Drawn::after_action(
            1000.0,
            weapons::Out::Blade { dual: false },
        ));
        world
            .resource_mut::<Units>()
            .by_id
            .insert(unit.obj_id, unit);
        let model = world.get::<Character>(root).unwrap().model_root;
        world.resource_mut::<Fixture>().actors.push((root, model));
    }
    world.spawn((Text::new("Provided remote receipts: hood + weapon image | received dye | parked actor + CDF removal"),
        TextFont { font_size: FontSize::Px(19.0), ..default() }, Node { position_type: PositionType::Absolute,
            left: Val::Px(18.0), bottom: Val::Px(18.0), ..default() }));
}
fn receipts(world: &mut World) {
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed > 1.0 && !world.resource::<Fixture>().parked {
        let actors = world.resource::<Fixture>().actors.clone();
        for (i, (root, _)) in actors.iter().enumerate() {
            world.get_mut::<Animator>(*root).unwrap().paused = true;
            if i == 2 {
                let animator = world.entity_mut(*root).take::<Animator>().unwrap();
                world.entity_mut(*root).insert(Asleep::parked(animator));
            }
        }
        world.resource_mut::<Fixture>().parked = true;
    }
    if elapsed <= 3.0 || world.resource::<Fixture>().changed {
        return;
    }
    let mut blade = item(26060);
    let ItemDetail::Equip(detail) = &mut blade.detail else {
        unreachable!()
    };
    detail.image_item_template_id = 26118;
    let mut dyed = item(14359);
    let ItemDetail::Equip(detail) = &mut dyed.detail else {
        unreachable!()
    };
    detail.dye_item_id = 29383;
    for (obj_id, items) in [
        (
            201,
            vec![
                (slot::HEAD as u8, Some(item(18230))),
                (slot::MAINHAND as u8, Some(blade)),
            ],
        ),
        (202, vec![(slot::COSPLAY as u8, Some(dyed))]),
        (203, vec![(slot::COSPLAY as u8, None)]),
    ] {
        assert!(
            world
                .resource_mut::<Units>()
                .apply(&Event::EquipmentChanged { obj_id, items })
        );
    }
    world.resource_mut::<Fixture>().changed = true;
    info!("gear fixture: three explicit received equipment deltas; no socket or server mutation");
}
fn verify(world: &mut World) {
    if world.resource::<Time>().elapsed_secs() < 7.0 || world.resource::<Fixture>().verified {
        return;
    }
    let actors = world.resource::<Fixture>().actors.clone();
    for (i, (root, model)) in actors.iter().enumerate() {
        let current = source(world, *root).unwrap();
        if world.resource::<Fixture>().before {
            assert_ne!(
                world.get::<Applied>(*root).unwrap().key,
                current.key,
                "refresh-disabled control receives the same newer source without acknowledging it"
            );
        } else {
            assert_eq!(world.get::<Applied>(*root).unwrap().key, current.key);
        }
        assert!(world.get::<Applied>(*root).unwrap().failed.is_none());
        assert_eq!(world.get::<Character>(*root).unwrap().model_root, *model);
        assert_eq!(world.get::<Asleep>(*root).is_some(), i == 2);
        assert!(
            world
                .get::<aa_anim::facial::FacialAnimator>(*root)
                .is_some()
        );
    }
    world.resource_mut::<Fixture>().verified = true;
    info!(
        "gear fixture: PASS same three received deltas, refresh disabled={}; unchanged actors/model roots, parked and facial state retained",
        world.resource::<Fixture>().before
    );
}
