use super::*;
use crate::{library::Library, sim::EffectSim};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::time::{Duration, Instant};

struct Allocator;
thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}
fn allocations(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|n| n.set(0));
    COUNTING.with(|n| n.set(true));
    f();
    COUNTING.with(|n| n.set(false));
    ALLOCATIONS.with(Cell::get)
}

const SOURCE: &str = r#"<ParticleLibrary><Particles Name="burst"><Params Enabled="false"/>
<Childs><Particles Name="wind"><Params ForceGeneration="Wind" Count="0" Size="0" Speed="78.4"
ParticleLifeTime="0.4" EmitAngle="180,1" FocusGravityDir="true"/></Particles></Childs></Particles></ParticleLibrary>"#;

fn entity(i: u32) -> Entity {
    Entity::from_raw_u32(i).unwrap()
}
fn field(owner: u32, center: Vec3) -> Field {
    Field {
        owner: entity(owner),
        ordinal: 0,
        center,
        inv_radius: 7.84f32.recip(),
        strength: 78.4,
    }
}
fn close(a: Vec3, b: Vec3) {
    for (a, b) in a.to_array().into_iter().zip(b.to_array()) {
        assert!((a - b).abs() <= (b.abs() * 3e-6).max(1e-14), "{a} != {b}");
    }
}

#[test]
fn executed_native_external_and_distinct_cache_goldens() {
    let mut count = 0;
    for line in include_str!("../../evidence/wind_kernels.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let mut columns = line.split('\t');
        let name = columns.next().unwrap();
        let v: Vec<f32> = columns
            .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
            .collect();
        let vector = |i| Vec3::new(v[i], v[i + 1], v[i + 2]);
        let f = Field {
            owner: entity(1),
            ordinal: 0,
            center: vector(2),
            inv_radius: v[0].recip(),
            strength: v[1],
        };
        close(f.external(vector(5)), vector(8));
        close(f.cached(vector(5)), vector(11));
        if name == "zero" {
            assert_eq!(f.external(vector(5)), Vec3::Z * 78.4);
            assert_eq!(f.cached(vector(5)), Vec3::ZERO);
        }
        if name.starts_with("exact-distance2-equality") {
            assert_eq!(f.external(vector(5)), Vec3::Z * 78.4);
        }
        count += 1;
    }
    assert_eq!(count, 148);
}

#[test]
fn external_sum_owner_exclusion_native_axes_and_invalid_points() {
    let fields = FxWindFields {
        fields: vec![field(1, Vec3::ZERO), field(2, Vec3::new(-3.92, 0., 0.))],
    };
    close(fields.sample(Vec3::ZERO), Vec3::new(39.2, 78.4, 0.));
    close(
        fields.receiver(entity(1)).unwrap().sample(Vec3::ZERO),
        Vec3::new(39.2, 0., 0.),
    );
    assert_eq!(
        fields.receiver(entity(2)).unwrap().sample(Vec3::ZERO),
        Vec3::ZERO
    );
    // Identity carries Entity generation, rather than raw index or actor ownership.
    let mut world = World::new();
    let first = world.spawn_empty().id();
    world.despawn(first);
    let second = world.spawn_empty().id();
    assert_ne!(first, second);
    let mut one = FxWindFields {
        fields: vec![Field {
            owner: first,
            ..field(1, Vec3::ZERO)
        }],
    };
    assert_eq!(one.receiver(first).unwrap().sample(Vec3::X), Vec3::ZERO);
    assert!(one.receiver(second).unwrap().sample(Vec3::X).length() > 0.);
    for p in [
        Vec3::splat(f32::NAN),
        Vec3::splat(f32::INFINITY),
        Vec3::splat(100.),
    ] {
        assert_eq!(one.sample(p), Vec3::ZERO);
    }
    one.fields.clear();
    assert!(one.is_empty());
    assert!(one.receiver(first).is_none());
    assert_eq!(one.sample(Vec3::ZERO), Vec3::ZERO);
}

fn setup() -> (App, Entity) {
    setup_cap(64)
}
fn setup_cap(cap: usize) -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<FxSettings>()
        .init_resource::<FxWindSettings>()
        .init_resource::<FxWindFields>()
        .add_systems(PostUpdate, (observe, publish).chain());
    app.world_mut().resource_mut::<FxWindSettings>().enabled = true;
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = cap;
    let def = Library::parse(SOURCE)
        .unwrap()
        .get("burst")
        .unwrap()
        .clone();
    let run = FxRun {
        def: Some(def),
        sim: Some(EffectSim::new(Vec::new(), 1., 1., 2.)),
        radius: 100.,
        ..Default::default()
    };
    let id = app
        .world_mut()
        .spawn((
            Effect {
                prime: false,
                ..Effect::new("test.burst")
            },
            run,
            GlobalTransform::IDENTITY,
        ))
        .id();
    app.update();
    (app, id)
}
fn tick(app: &mut App, dt: f64) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f64(dt));
    app.update();
}

#[test]
fn real_management_holds_count_zero_until_native_deadline_and_loops_once() {
    let (mut app, id) = setup();
    app.init_resource::<crate::FxAssets>()
        .init_resource::<crate::FxStats>()
        .init_resource::<crate::FxView>();
    app.world_mut()
        .resource_mut::<crate::FxSettings>()
        .max_lights = 0;
    app.world_mut().resource_mut::<crate::FxView>().0 = Some(crate::sim::View {
        pos: Vec3::Z,
        right: Vec3::X,
        up: Vec3::Y,
        forward: Vec3::NEG_Z,
    });
    app.world_mut().entity_mut(id).insert(crate::FxActive);
    app.world_mut()
        .get_mut::<FxRun>(id)
        .unwrap()
        .sim
        .as_mut()
        .unwrap()
        .primed = true;
    tick(&mut app, 0.1);
    app.world_mut().run_system_cached(crate::simulate).unwrap();
    assert!(!app.world().get::<FxRun>(id).unwrap().finished);
    assert_eq!(app.world().get::<FxRun>(id).unwrap().particles(), 0);
    tick(&mut app, 0.301);
    app.world_mut().run_system_cached(crate::simulate).unwrap();
    assert!(app.world().get::<FxRun>(id).unwrap().finished);
    app.world_mut().get_mut::<Effect>(id).unwrap().force_loop = true;
    app.world_mut().run_system_cached(crate::simulate).unwrap();
    tick(&mut app, 0.1);
    assert!(!app.world().resource::<FxWindFields>().is_empty());
    app.world_mut().get_mut::<FxRun>(id).unwrap().sim = None;
    tick(&mut app, 0.1);
    assert!(app.world().resource::<FxWindFields>().is_empty());
}

#[test]
fn count_zero_uses_unwrapped_clock_through_disable_cap_wait_sleep_and_cleanup() {
    let (mut app, id) = setup();
    assert!(app.world().resource::<FxWindFields>().is_empty());
    tick(&mut app, 0.1);
    assert_eq!(app.world().resource::<FxWindFields>().fields.len(), 1);
    assert_eq!(app.world().get::<FxRun>(id).unwrap().particles(), 0);
    assert!(app.world().get::<FxRun>(id).unwrap().wind.holding(0.1, 2));
    app.world_mut().resource_mut::<FxSettings>().enabled = false;
    tick(&mut app, 0.1);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    app.world_mut().resource_mut::<FxSettings>().enabled = true;
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = 0;
    tick(&mut app, 0.1);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = 64;
    tick(&mut app, 0.1);
    assert_eq!(app.world().resource::<FxWindFields>().fields.len(), 1); // inclusive 0.4
    tick(&mut app, 0.001);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    // A renderer/resource wait or deactivation does not restart the source.
    app.world_mut().get_mut::<FxRun>(id).unwrap().sim = None;
    tick(&mut app, 3601.);
    app.world_mut().get_mut::<FxRun>(id).unwrap().sim =
        Some(EffectSim::new(Vec::new(), 1., 1., 2.));
    tick(&mut app, 0.1);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    // An actual loop/reset owns a fresh epoch, even after wrapped game time.
    let now = app.world().resource::<Time>().elapsed_secs_f64();
    app.world_mut()
        .get_mut::<FxRun>(id)
        .unwrap()
        .wind
        .restart(now);
    tick(&mut app, 0.1);
    assert!(!app.world().resource::<FxWindFields>().is_empty());
    app.world_mut().entity_mut(id).remove::<Effect>();
    tick(&mut app, 0.);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    app.world_mut().despawn(id);
    tick(&mut app, 0.);
    assert!(app.world().resource::<FxWindFields>().is_empty());
}

#[test]
fn late_library_and_late_texture_readiness_cannot_replay_spent_burst() {
    let (mut app, id) = setup();
    let definition = app.world().get::<FxRun>(id).unwrap().def.clone();
    {
        let mut run = app.world_mut().get_mut::<FxRun>(id).unwrap();
        run.def = None;
        run.sim = None;
    }
    tick(&mut app, 2.);
    {
        let mut run = app.world_mut().get_mut::<FxRun>(id).unwrap();
        run.def = definition;
        run.sim = Some(EffectSim::new(Vec::new(), 1., 1., 2.));
    }
    tick(&mut app, 0.1);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    // Replacing FxRun is a fresh instance, not a stale registration.
    {
        let mut run = app.world_mut().get_mut::<FxRun>(id).unwrap();
        run.wind = Run::default();
    }
    tick(&mut app, 0.);
    tick(&mut app, 0.1);
    assert!(!app.world().resource::<FxWindFields>().is_empty());
}

#[test]
fn admission_caps_and_source_semantics_are_not_effect_name_substitutions() {
    let (mut app, _) = setup();
    let definition = Library::parse(SOURCE)
        .unwrap()
        .get("burst")
        .unwrap()
        .clone();
    for i in 0..300 {
        app.world_mut().spawn((
            Effect {
                prime: false,
                ..Effect::new(format!("arbitrary.{i}"))
            },
            FxRun {
                def: Some(definition.clone()),
                sim: Some(EffectSim::new(Vec::new(), 1., 1., 2.)),
                ..Default::default()
            },
            GlobalTransform::IDENTITY,
        ));
    }
    tick(&mut app, 0.);
    tick(&mut app, 0.1);
    assert_eq!(app.world().resource::<FxWindFields>().fields.len(), 64);
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = usize::MAX;
    tick(&mut app, 0.);
    let fields = app.world().resource::<FxWindFields>();
    assert_eq!(fields.fields.len(), 256);
    assert!(fields.fields.windows(2).all(|p| p[0].key() < p[1].key()));
    for alteration in [
        "Continuous=\"true\"",
        "SpawnIndirection=\"ParentStart\"",
        "Speed=\"78.4,0.1\"",
        "AirResistance=\"1\"",
        "Size=\"1\"",
        "PulsePeriod=\"1\"",
    ] {
        let xml = format!(
            "<ParticleLibrary><Particles Name=\"a\"><Params ForceGeneration=\"Wind\" ParticleLifeTime=\"0.4\" EmitAngle=\"180,1\" {} {}/></Particles></ParticleLibrary>",
            if alteration.starts_with("Size=") {
                ""
            } else {
                "Size=\"0\""
            },
            alteration
        );
        assert!(
            !Library::parse(&xml)
                .unwrap()
                .get("a")
                .unwrap()
                .root
                .def
                .wind_source,
            "{alteration}"
        );
    }
}

#[test]
fn original_three_producers_count_independence_and_source_transforms() {
    let library = Library::load("X2_EFFECTS").unwrap();
    for name in [
        "doodad_effect.quest_small_obj_explosion",
        "doodad_effect.quest_woodbox_exp",
        "doodad_effect.quest_pot_exp",
    ] {
        let mut run = Run::default();
        run.resolve(library.get(name).unwrap());
        assert_eq!(run.sources.len(), 1);
        let source = &run.sources[0];
        assert_eq!(source.count.base, 0.);
        let effect = Effect {
            prime: false,
            ..Effect::new(name)
        };
        let f = source_field(
            source,
            entity(1),
            0,
            &effect,
            &GlobalTransform::IDENTITY,
            0.1,
        )
        .unwrap();
        assert_eq!(f.inv_radius, 7.84f32.recip());
        assert_eq!(f.strength, 78.4);
        assert!(
            source_field(
                source,
                entity(1),
                0,
                &effect,
                &GlobalTransform::IDENTITY,
                0.401
            )
            .is_none()
        );
        let mut other = (**source).clone();
        other.count = Var::constant(100.);
        assert!(eligible(&other));
        other.position_offset = Vec3::new(1., 2., 3.);
        let gt = GlobalTransform::from(
            Transform::from_xyz(2., 4., -3.)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(2.)),
        );
        let f = source_field(&other, entity(1), 0, &effect, &gt, 0.1).unwrap();
        close(
            crate::c2b(f.center),
            gt.transform_point(crate::c2b(other.position_offset)),
        );
        close(Vec3::splat(f.strength), Vec3::splat(156.8));
        assert!(
            source_field(
                source,
                entity(1),
                0,
                &effect,
                &GlobalTransform::from(Transform::from_scale(Vec3::new(1., 2., 1.))),
                0.1
            )
            .is_none()
        );
    }
}

#[test]
#[ignore = "explicit CPU measurement; run with --ignored --nocapture --test-threads=1"]
fn scalar_cost() {
    for count in [0, 1, 16, 64] {
        for dense in [false, true] {
            let fields = FxWindFields {
                fields: (0..count)
                    .map(|i| {
                        field(
                            i + 1,
                            Vec3::new(if dense { 0. } else { i as f32 * 32. }, 0., 0.),
                        )
                    })
                    .collect(),
            };
            for queries in [0, 1000, 10000] {
                let mut samples = Vec::new();
                let mut sum = Vec3::ZERO;
                for _ in 0..25 {
                    let start = Instant::now();
                    for i in 0..queries {
                        sum += std::hint::black_box(fields.sample(Vec3::new(
                            (i % 19) as f32 * 0.1,
                            0.,
                            1.,
                        )));
                    }
                    samples.push(start.elapsed().as_secs_f64() * 1000.);
                }
                samples.sort_by(f64::total_cmp);
                let allocs = allocations(|| {
                    for i in 0..queries {
                        std::hint::black_box(fields.sample(Vec3::new(
                            (i % 19) as f32 * 0.1,
                            0.,
                            1.,
                        )));
                    }
                });
                assert_eq!(allocs, 0);
                println!(
                    "wind-cost fields={count} dense={dense} queries={queries} allocations={allocs} p50_ms={:.6} p95_ms={:.6} max_ms={:.6} checksum={sum:?}",
                    samples[12], samples[23], samples[24]
                );
            }
        }
    }
    for count in [1, 16, 64] {
        let (mut app, _) = setup();
        let definition = Library::parse(SOURCE)
            .unwrap()
            .get("burst")
            .unwrap()
            .clone();
        for _ in 1..count {
            app.world_mut().spawn((
                Effect {
                    prime: false,
                    ..Effect::new("bench.burst")
                },
                FxRun {
                    def: Some(definition.clone()),
                    sim: Some(EffectSim::new(Vec::new(), 1., 1., 2.)),
                    ..Default::default()
                },
                GlobalTransform::IDENTITY,
            ));
        }
        tick(&mut app, 0.);
        tick(&mut app, 0.1);
        for _ in 0..5 {
            app.world_mut().run_system_cached(publish).unwrap();
        }
        let mut samples = Vec::new();
        for _ in 0..25 {
            let start = Instant::now();
            app.world_mut().run_system_cached(publish).unwrap();
            samples.push(start.elapsed().as_secs_f64() * 1000.);
        }
        let allocs = allocations(|| {
            for _ in 0..25 {
                app.world_mut().run_system_cached(publish).unwrap();
            }
        });
        samples.sort_by(f64::total_cmp);
        let capacity = app.world().resource::<FxWindFields>().fields.capacity();
        println!(
            "wind-publish roots={count} allocations_25_passes={allocs} capacity={capacity} p50_ms={:.6} p95_ms={:.6} max_ms={:.6}",
            samples[12], samples[23], samples[24]
        );
        // Includes Bevy cached-system invocation overhead, reported rather than hidden.
        app.world_mut().resource_mut::<FxWindSettings>().enabled = false;
        app.world_mut().run_system_cached(publish).unwrap();
        assert!(app.world().resource::<FxWindFields>().is_empty());
        assert_eq!(
            app.world().resource::<FxWindFields>().fields.capacity(),
            capacity
        );
    }
}

#[test]
fn original_other_root_particle_reception_and_same_owner_outside_no_force_controls() {
    use crate::sim::{EmitterState, StepCtx, View};
    use std::sync::atomic::AtomicI32;
    let library = Library::load("X2_EFFECTS").unwrap();
    let receiver = library
        .get("e_effect.siegetank_fire_proj")
        .unwrap()
        .emitters
        .iter()
        .find(|e| e.def.name == "ship_canon_proj_brown_smoke")
        .unwrap()
        .def
        .clone();
    assert!(receiver.wind > 0.);
    let noforce = library
        .get("e_effect.smoke")
        .unwrap()
        .emitters
        .iter()
        .find(|e| e.def.draws())
        .unwrap()
        .def
        .clone();
    assert_eq!(noforce.wind, 0.);
    fn run(d: Arc<EmitterDef>, fields: &FxWindFields, owner: u32) -> Vec<[f32; 3]> {
        let budget = AtomicI32::new(10000);
        let ctx = StepCtx {
            dt: 1.0 / 60.0,
            view: View {
                pos: Vec3::new(0., 2., 10.),
                right: Vec3::X,
                up: Vec3::Y,
                forward: Vec3::NEG_Z,
            },
            global: bevy::math::Affine3A::IDENTITY,
            rot: Mat3::IDENTITY,
            scale: 1.,
            axis: Vec3::ONE,
            speed_scale: 1.,
            count_scale: 1.,
            lod: 1.,
            size_scale: 1.,
            sprite_half: 1.,
            colour_gamma: 2.2,
            count_over_life: true,
            tint: Vec3::ONE,
            sun: Vec3::Y,
            sun_light: 0.,
            sun_facing: 0.,
            emitter_vel: Vec3::ZERO,
            ground: None,
            wind: Vec3::ZERO,
            body: None,
            no_body: true,
            budget: &budget,
            fade: 1.,
        };
        let mut sim = EffectSim::new(
            vec![EmitterState::new(d, None, 0, 1., 42, Entity::PLACEHOLDER)],
            1.,
            1.,
            2.,
        );
        for frame in 0..48 {
            sim.step_wind(
                &ctx,
                if frame < 24 {
                    fields.receiver(entity(owner))
                } else {
                    None
                },
            );
        }
        sim.emitters[0].pos.clone()
    }
    let none = FxWindFields::default();
    let fields = FxWindFields {
        fields: vec![field(1, Vec3::new(-3.92, 0., 0.))],
    };
    let baseline = run(receiver.clone(), &none, 2);
    let external = run(receiver.clone(), &fields, 2);
    assert!(!baseline.is_empty());
    assert_eq!(baseline.len(), external.len());
    let displacement: f32 = baseline
        .iter()
        .zip(&external)
        .map(|(a, b)| (Vec3::from(*a) - Vec3::from(*b)).length())
        .sum();
    assert!(
        displacement > 1.,
        "external original particles did not respond: {displacement}"
    );
    assert_eq!(run(receiver.clone(), &fields, 1), baseline);
    let outside = FxWindFields {
        fields: vec![field(1, Vec3::splat(100.))],
    };
    assert_eq!(run(receiver, &outside, 2), baseline);
    assert_eq!(run(noforce.clone(), &fields, 2), run(noforce, &none, 2));
}

#[test]
fn source_frames_and_scales_match_executed_native_burst_producer() {
    let definition = Library::parse(SOURCE)
        .unwrap()
        .get("burst")
        .unwrap()
        .clone();
    let mut clock = Run::default();
    clock.resolve(&definition);
    let mut count = 0;
    for line in include_str!("../../evidence/wind_transforms.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let mut cols = line.split('\t');
        let _name = cols.next().unwrap();
        let v: Vec<f32> = cols
            .map(|n| f32::from_bits(u32::from_str_radix(n, 16).unwrap()))
            .collect();
        let vector = |i| Vec3::new(v[i], v[i + 1], v[i + 2]);
        let gt = GlobalTransform::from(Transform {
            translation: crate::c2b(vector(0)),
            rotation: Quat::from_xyzw(v[3], v[5], -v[4], v[6]),
            scale: Vec3::splat(v[7]),
        });
        let mut source = (*clock.sources[0]).clone();
        source.position_offset = vector(9);
        let effect = Effect {
            scale: v[8],
            prime: false,
            ..Effect::new("native.transforms")
        };
        let got = source_field(&source, entity(1), 0, &effect, &gt, v[12]).unwrap();
        close(got.center, vector(13));
        close(Vec3::splat(got.inv_radius), Vec3::splat(v[16].recip()));
        close(Vec3::splat(got.strength), Vec3::splat(v[17]));
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
fn initial_zero_cap_retains_count_zero_source_lifetime_without_replaying_age() {
    let (mut app, id) = setup_cap(0);
    assert!(app.world().resource::<FxWindFields>().is_empty());
    assert!(app.world().get::<FxRun>(id).unwrap().wind.holding(0.1, 2));
    tick(&mut app, 0.3);
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = 64;
    tick(&mut app, 0.);
    let fields = app.world().resource::<FxWindFields>();
    assert_eq!(fields.fields.len(), 1);
    close(
        Vec3::splat(fields.fields[0].inv_radius),
        Vec3::splat((78.4f32 * 0.3).recip()),
    );
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = 0;
    tick(&mut app, 0.101);
    app.world_mut().resource_mut::<FxWindSettings>().max_fields = 64;
    tick(&mut app, 0.);
    assert!(app.world().resource::<FxWindFields>().is_empty());
}
