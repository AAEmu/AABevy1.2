//! Cutting trees down (`SCVegetationCutdowning`, 0x081): the fall and the phase after it.
//!
//! The server's `InteractionEffect` for `WorldInteractionType.Cutdown` (1) runs `doodad.Use` and
//! broadcasts `SCVegetationCutdowningPacket(unitObjId, doodadObjId)` (`aaemu-game`
//! `systems/npcs/doodad/interactions.rs`, CUTDOWN). The 1.2 client's `OnVegetationCutdowning`
//! handler (decompile 368373-368397) resolves the unit and the doodad and calls
//! `FUN_393b7800(unit, doodad, 100)`; that queues a doodad-model effect (`FUN_393b7140` stores
//! `100 * 0.001` s, 773577-773584) and later drives the vegetation model (`FUN_393b6910`,
//! "vegetation cut downed !!" at 773342). The model's own fall animation and its duration are
//! native and were not recovered, so the fall here is our approximation: it tips away from the
//! cutter after the decompile's start delay, over [`CUTDOWN_FALL_SECS`].
//!
//! `SCDoodadPhaseChanged` precedes the cut: the original client's 2026-10-30 capture (old birch
//! template 407, phase 944, skill 15874) logged, in one batch and in this order, the phase change
//! to 6251 (`time_left` 180000), `SCErrorMsg` 181 (`BAG_INVALID_ITEM`, "Invalid item."; see
//! `hud/error_names.txt`) and `SCVegetationCutdowning`, and removed the tree 2367 ms after the
//! cut. [`start`] runs before [`super::doodad_visual::phase_changes`] in the plugin's chain, so
//! the fall starts in the model the tree was standing in and the phase's model swap is deferred.
//!
//! While the model falls, the phase's new model and its growth scale are deferred (the tree
//! keeps the scale it stood with), so the tree does not disappear under the axe or pop in size;
//! a cut phase that keeps the model (e.g. group 1553) leaves the tree lying down until
//! `SCDoodadRemoved`, and a phase with a different explicit model (a stump) swaps it upright
//! after the fall. An empty phase model falls back to the doodad template in
//! `Db::doodad_phase_visual`, which is usually the same tree, so it also stays lying (the
//! captured birch: phase 944 and template 407 share one model and phase 6251 has none). That
//! split is a guess about the native model swap order: the capture exposed no vegetation entity,
//! so neither the fall transform nor the model swap could be sampled.
//!
//! A `SCVegetationCutdowning` that arrives a frame or more *after* its phase model swap is not
//! repaired: the previous standing model is not kept, so the fall starts from whatever model is
//! already drawn. The capture never showed that gap (both packets arrived in one batch, phase
//! first); the path is kept as a defensive fallback.

use aa_net::state::{Doodads, Units};
use aa_net::{Event, NetEvent};
use aa_world::coords::cry_rot_to_bevy;
use bevy::prelude::*;

use super::doodad_visual::DoodadVisual;
use super::player::{LocalPlayer, Mover};
use super::units::{RemoteDoodad, doodad_world_in};

/// Delay before the model effect starts: the handler passes 100 to `FUN_393b7140`, which stores
/// `100 * 0.001` s (decompile 368391 and 773577-773584).
const CUTDOWN_START_DELAY: f32 = 0.1;

/// How long the tree takes to fall. The native `VegetationDoodadModel` animation was not
/// recovered; 2.2 s stays inside the database's 2-3 s cutdown-to-final windows (groups 2178 and
/// 3159 use `DoodadFuncFinal` delays of 2000 and 3000 ms). The 2026-10-30 capture removed the cut
/// tree 2367 ms after the cut, 67 ms after this fall ends, so the removal does not shorten it;
/// the duration and curve remain a guess (the capture had no transform samples).
const CUTDOWN_FALL_SECS: f32 = 2.2;

/// A tree that is falling now: the phase model swap and the phase's growth scale are deferred until
/// it lands.
#[derive(Component)]
pub struct Falling {
    /// The rotation the tree stood with (Bevy space).
    base: Quat,
    /// The horizontal axis it turns about (game space).
    axis: Vec3,
    /// Seconds since the cutdown packet, the start delay included.
    elapsed: f32,
    /// The model path at cutdown; a later swap of this path means the phase changed.
    path: String,
}

/// A tree that landed and stays fallen while its phase keeps the same model.
#[derive(Component)]
pub struct Fallen {
    base: Quat,
    path: String,
}

/// `SCVegetationCutdowning`: start the fall of the doodad away from the unit that cut it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn start(
    mut events: MessageReader<NetEvent>,
    mut commands: Commands,
    doodads: Res<Doodads>,
    units: Res<Units>,
    local: Query<(&LocalPlayer, &Mover)>,
    houses: Option<Res<super::housing::render::DrawnHouses>>,
    roots: Query<(
        Entity,
        &RemoteDoodad,
        &Transform,
        &DoodadVisual,
        Option<&Falling>,
        Option<&Fallen>,
    )>,
) {
    for NetEvent(event) in events.read() {
        let Event::VegetationCutdown {
            unit_obj_id,
            doodad_obj_id,
        } = event
        else {
            continue;
        };
        let Some(doodad) = doodads.by_id.get(doodad_obj_id) else {
            continue;
        };
        let Some((entity, _, transform, visual, falling, fallen)) =
            roots.iter().find(|root| root.1.obj_id == *doodad_obj_id)
        else {
            continue;
        };
        // a repeated broadcast must not restart a fall that is already running or lying
        if falling.is_some() || fallen.is_some() {
            debug!("cutdown: doodad {doodad_obj_id} is already cut down");
            continue;
        }
        let Some(tree) = doodad_world_in(doodad, &units, houses.as_deref()) else {
            continue;
        };
        let own = local.iter().next().map(|(p, m)| (p.obj_id, m.pos));
        let direction = match cutter_position(*unit_obj_id, &units, own) {
            Some(cutter) => fall_direction(tree, cutter),
            // nobody stands near: north is the fallback
            None => Vec3::Y,
        };
        commands.entity(entity).insert(Falling {
            base: transform.rotation,
            axis: fall_axis(direction),
            elapsed: 0.0,
            path: visual.path.clone(),
        });
        debug!(
            "cutdown: doodad {} by unit {} falls ({:.2}, {:.2})",
            doodad_obj_id, unit_obj_id, direction.x, direction.y
        );
    }
}

/// The cutter's position in game space: a remote unit when it is drawn, else the local player
/// (the player is not in `Units`; `Mover.pos` is its live position).
fn cutter_position(unit_obj_id: u32, units: &Units, own: Option<(u32, Vec3)>) -> Option<Vec3> {
    let remote = units
        .by_id
        .get(&unit_obj_id)
        .map(|u| Vec3::new(u.pos.x, u.pos.y, u.pos.z));
    pick_cutter(unit_obj_id, remote, own)
}

fn pick_cutter(unit_obj_id: u32, remote: Option<Vec3>, own: Option<(u32, Vec3)>) -> Option<Vec3> {
    remote.or_else(|| own.filter(|(id, _)| *id == unit_obj_id).map(|(_, pos)| pos))
}

/// The horizontal direction from the cutter to the tree (`+Y` when they stand on top of each
/// other).
fn fall_direction(tree: Vec3, cutter: Vec3) -> Vec3 {
    let d = Vec3::new(tree.x - cutter.x, tree.y - cutter.y, 0.0);
    if d.length_squared() > 1e-4 {
        d.normalize()
    } else {
        Vec3::Y
    }
}

/// The game-space axis that tips the tree's up vector towards `direction`.
fn fall_axis(direction: Vec3) -> Vec3 {
    Vec3::Z.cross(direction).normalize()
}

/// The fall angle at `elapsed` seconds: still during the native start delay, then a quadratic
/// ease-in to a quarter turn.
fn fall_angle(elapsed: f32) -> f32 {
    let progress = ((elapsed - CUTDOWN_START_DELAY) / CUTDOWN_FALL_SECS).clamp(0.0, 1.0);
    std::f32::consts::FRAC_PI_2 * progress * progress
}

/// Tips the falling trees over (slowly at first, then the weight takes them).
pub fn fall(time: Res<Time>, mut trees: Query<(&mut Falling, &mut Transform)>) {
    for (mut tree, mut transform) in &mut trees {
        tree.elapsed += time.delta_secs();
        transform.rotation =
            cry_rot_to_bevy(Quat::from_axis_angle(tree.axis, fall_angle(tree.elapsed))) * tree.base;
    }
}

/// Ends an active fall. The tree keeps lying (`Fallen`) after landing; a phase model swap that
/// was deferred while it fell (with the phase's growth scale) is drawn by
/// [`super::doodad_visual::phase_changes`] and [`settle`] puts the root upright in that same
/// frame, so there is no upright frame with the old model.
pub fn land(mut commands: Commands, mut trees: Query<(Entity, &Falling)>) {
    for (entity, tree) in &mut trees {
        if tree.elapsed < CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS {
            continue;
        }
        commands.entity(entity).remove::<Falling>().insert(Fallen {
            base: tree.base,
            path: tree.path.clone(),
        });
    }
}

/// Puts a lying tree back upright when its model was actually replaced (respawn, stump, ...).
pub fn settle(
    mut commands: Commands,
    mut trees: Query<(Entity, &Fallen, &DoodadVisual, &mut Transform)>,
) {
    for (entity, fallen, visual, mut transform) in &mut trees {
        if visual.path == fallen.path {
            continue;
        }
        transform.rotation = fallen.base;
        commands.entity(entity).remove::<Fallen>();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use aa_cry::scene::CryCache;
    use aa_cry::shade::illum::IllumMaterial;
    use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
    use rusqlite::Connection;

    use super::super::data::{Db, DoodadGrowth};
    use super::super::doodad_visual;
    use super::super::units::{Views, receive_moves, sync_views};
    use super::*;

    fn rotation_error(a: Quat, b: Quat) -> f32 {
        (a * Vec3::X - b * Vec3::X).length()
            + (a * Vec3::Y - b * Vec3::Y).length()
            + (a * Vec3::Z - b * Vec3::Z).length()
    }

    fn tilt(rotation: Quat) -> f32 {
        (rotation * Vec3::Y - Vec3::Y).length()
    }

    fn visual(path: &str, func_group_id: u32, rendered_func_group_id: u32) -> DoodadVisual {
        DoodadVisual {
            path: path.into(),
            model: None,
            func_group_id,
            rendered_func_group_id,
            bond_helpers: HashMap::new(),
            growth: None,
            remaining_ms: 0.0,
            total_ms: 0.0,
            base_scale: 1.0,
        }
    }

    #[test]
    fn fall_tips_the_tree_away_from_the_cutter() {
        let tree = Vec3::new(10.0, 0.0, 0.0);
        let cutter = Vec3::ZERO;
        let direction = fall_direction(tree, cutter);
        assert!((direction - Vec3::X).length() < 1e-5);
        let rotation = Quat::from_axis_angle(fall_axis(direction), std::f32::consts::FRAC_PI_2);
        assert!((rotation * Vec3::Z - Vec3::X).length() < 1e-5);
        // the same tree cut from the other side falls the other way
        assert!((fall_direction(tree, Vec3::new(20.0, 0.0, 0.0)) + Vec3::X).length() < 1e-5);
    }

    #[test]
    fn the_local_player_is_a_cutter_even_though_units_has_no_player() {
        let own = Some((7, Vec3::new(1.0, 2.0, 3.0)));
        assert_eq!(pick_cutter(7, None, own), Some(Vec3::new(1.0, 2.0, 3.0)));
        // a remote unit is preferred, and an unrelated local player is not the cutter
        assert_eq!(pick_cutter(7, Some(Vec3::X), own), Some(Vec3::X));
        assert_eq!(pick_cutter(8, None, own), None);
        assert_eq!(cutter_position(7, &Units::default(), None), None);
        assert_eq!(
            cutter_position(7, &Units::default(), own),
            Some(Vec3::new(1.0, 2.0, 3.0))
        );
    }

    #[test]
    fn the_fall_waits_for_the_native_start_delay() {
        assert_eq!(fall_angle(0.0), 0.0);
        assert_eq!(fall_angle(CUTDOWN_START_DELAY), 0.0);
        assert!(fall_angle(CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS / 2.0) > 0.0);
        let end = fall_angle(CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS);
        assert!((end - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn the_fall_starts_after_the_delay_and_ends_at_a_quarter_turn() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .add_systems(Update, fall);
        let tree = app
            .world_mut()
            .spawn((
                Falling {
                    base: Quat::IDENTITY,
                    axis: fall_axis(Vec3::X),
                    elapsed: 0.0,
                    path: "tree".into(),
                },
                Transform::default(),
            ))
            .id();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(0.05));
        app.update();
        assert!(
            rotation_error(
                app.world().get::<Transform>(tree).unwrap().rotation,
                Quat::IDENTITY
            ) < 1e-5
        );

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(
                CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS,
            ));
        app.update();
        let rotation = app.world().get::<Transform>(tree).unwrap().rotation;
        assert!(tilt(rotation) > 1.0);
        // the local +Y (Bevy up) ends along the game-space fall direction, converted to Bevy
        let up = rotation * Vec3::Y;
        assert!((up - Vec3::X).length() < 1e-4);
    }

    fn doodad_at(obj_id: u32, x: f32, y: f32) -> aa_net::model::Doodad {
        aa_net::model::Doodad {
            obj_id,
            template_id: 407,
            owner_obj_id: 0,
            parent_obj_id: 0,
            attach_point: 0,
            pos: aa_net::Pos::new(x, y, 0.0),
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            has_loot_item: false,
            func_group_id: 1,
            owner_id: 0,
            ucc_id: 0,
            item_template_id: 0,
            type2: 0,
            time_left: 0,
            plant_time: 0,
            quest_glow: 0,
            family: 0,
            puzzle_group: 0,
            owner_type: 0,
            owner_db_id: 0,
            data: 0,
        }
    }

    fn cut_event(unit_obj_id: u32, doodad_obj_id: u32) -> NetEvent {
        NetEvent(Event::VegetationCutdown {
            unit_obj_id,
            doodad_obj_id,
        })
    }

    fn start_app() -> App {
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .insert_resource(Doodads::default())
            .insert_resource(Units::default())
            .add_systems(Update, start);
        app
    }

    fn tree_in(app: &mut App, visual: DoodadVisual) -> Entity {
        let entity = app
            .world_mut()
            .spawn((RemoteDoodad { obj_id: 2 }, Transform::default(), visual))
            .id();
        app.world_mut()
            .resource_mut::<Doodads>()
            .by_id
            .insert(2, doodad_at(2, 10.0, 0.0));
        entity
    }

    #[test]
    fn a_second_cutdown_while_falling_does_not_restart_the_fall() {
        let mut app = start_app();
        let tree = tree_in(&mut app, visual("tree", 1, 1));
        app.world_mut().write_message(cut_event(1, 2));
        app.update();
        app.world_mut().get_mut::<Falling>(tree).unwrap().elapsed = 1.0;

        app.world_mut().write_message(cut_event(1, 2));
        app.update();
        assert_eq!(app.world().get::<Falling>(tree).unwrap().elapsed, 1.0);
    }

    #[test]
    fn a_second_cutdown_while_fallen_stays_lying() {
        let mut app = start_app();
        let base = Quat::from_rotation_y(0.4);
        let tree = tree_in(&mut app, visual("tree", 1, 1));
        app.world_mut().entity_mut(tree).insert(Fallen {
            base,
            path: "tree".into(),
        });
        app.world_mut().write_message(cut_event(1, 2));
        app.update();
        assert!(app.world().get::<Falling>(tree).is_none());
        assert!(app.world().get::<Fallen>(tree).is_some());
    }

    #[test]
    fn a_phase_change_one_frame_before_the_cut_still_falls_the_drawn_model() {
        // Not captured: the observed batch sends the phase change before the cut. If a cutdown
        // ever arrives after its phase model swap, the standing model is not restored and the
        // fall uses whatever model is already drawn.
        let mut app = start_app();
        let tree = tree_in(&mut app, visual("cut", 3169, 3169));
        app.world_mut().write_message(cut_event(1, 2));
        app.update();
        assert_eq!(app.world().get::<Falling>(tree).unwrap().path, "cut");
    }

    /// The captured old birch (template 407): its phase 944 draws this model, and the cut phase
    /// 6251 has none of its own, so it falls back to the template's: the same tree.
    const BIRCH: &str = "objects/env/05_nature/03_temp/white_birch_do_moss.cgf";

    /// Deletes a test database once the app holding it is gone (declare it before the app).
    struct TempDb(std::path::PathBuf);

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// The registered plugin chain (`doodad_visual::plugin`) over a database with the birch's rows
    /// and the `extra` SQL, with everything its systems read.
    fn chain_app(name: &str, extra: &str) -> (TempDb, App) {
        let path =
            std::env::temp_dir().join(format!("aa-cutdown-{}-{name}.sqlite3", std::process::id()));
        let _ = std::fs::remove_file(&path);
        Connection::open(&path)
            .unwrap()
            .execute_batch(&format!(
                "CREATE TABLE doodad_almighties(id INTEGER PRIMARY KEY, model TEXT);
                 CREATE TABLE doodad_func_groups(id INTEGER PRIMARY KEY, doodad_almighty_id INTEGER,
                                                 model TEXT);
                 CREATE TABLE doodad_phase_funcs(id INTEGER PRIMARY KEY,
                     doodad_func_group_id INTEGER, actual_func_type TEXT, actual_func_id INTEGER);
                 CREATE TABLE doodad_func_growths(id INTEGER PRIMARY KEY, delay INTEGER,
                     start_scale INTEGER, end_scale INTEGER);
                 INSERT INTO doodad_almighties VALUES (407, 'vegetation://{BIRCH}');
                 INSERT INTO doodad_func_groups VALUES (944, 407, 'vegetation://{BIRCH}');
                 INSERT INTO doodad_func_groups VALUES (6251, 407, '');
                 {extra}"
            ))
            .unwrap();
        let db = Db::open_path(path.to_str().unwrap()).expect("the test database opens");
        let mut app = App::new();
        app.add_message::<NetEvent>()
            .insert_resource(State::new(crate::Screen::InGame))
            .insert_resource(Time::<()>::default())
            .insert_resource(db)
            .insert_resource(Doodads::default())
            .insert_resource(Units::default())
            .insert_resource(Views::default())
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<IllumMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<Assets<SkinnedMeshInverseBindposes>>()
            .init_resource::<CryCache>()
            .add_plugins(doodad_visual::plugin);
        (TempDb(path), app)
    }

    /// The standing birch (doodad 2, 10 m from the cutter): phase 944 with its growth done at
    /// `scale`, and a model child.
    fn standing_birch(app: &mut App, scale: f32) -> (Entity, Entity) {
        let model = app.world_mut().spawn_empty().id();
        let mut standing = visual(BIRCH, 944, 944);
        standing.model = Some(model);
        standing.growth = Some(DoodadGrowth {
            delay_ms: 180_000,
            start_scale: 0.6,
            end_scale: scale,
        });
        standing.total_ms = 180_000.0;
        let root = app
            .world_mut()
            .spawn((
                RemoteDoodad { obj_id: 2 },
                Transform::from_scale(Vec3::splat(scale)),
                standing,
            ))
            .id();
        app.world_mut().entity_mut(model).insert(ChildOf(root));
        app.world_mut()
            .resource_mut::<Doodads>()
            .by_id
            .insert(2, doodad_at(2, 10.0, 0.0));
        app.world_mut()
            .resource_mut::<Views>()
            .fixture_doodad(2, root);
        (root, model)
    }

    /// The captured batch in its order: the phase change, `SCErrorMsg` 181, then the cut.
    fn send_cut_batch(app: &mut App, phase: u32) {
        app.world_mut()
            .write_message(NetEvent(Event::DoodadPhaseChanged {
                obj_id: 2,
                func_group_id: phase,
                time_left: 180_000,
                puzzle_group: -1,
                item_template_id: 0,
            }));
        app.world_mut()
            .write_message(NetEvent(Event::ServerError { code: 181 }));
        app.world_mut().write_message(cut_event(1, 2));
    }

    /// One 60 Hz frame.
    fn step(app: &mut App) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(1.0 / 60.0));
        app.update();
    }

    fn count<C: Component>(app: &mut App) -> usize {
        app.world_mut().query::<&C>().iter(app.world()).count()
    }

    #[test]
    fn the_captured_phase_before_the_cut_keeps_the_standing_model_falling_until_it_lands() {
        // 2026-10-30 capture, old birch 407 phase 944: SCDoodadPhaseChanged 6251, SCErrorMsg 181
        // and SCVegetationCutdowning arrive in one batch. The registered chain runs `start` before
        // `phase_changes`, so the fall starts in the standing model and the cut phase is drawn only
        // after the landing (its model is the template's, the same tree: the phase counts as drawn
        // when `rendered_func_group_id` reaches it). In the other order it is drawn in the first
        // frame.
        let (_db, mut app) = chain_app("order", "");
        let (tree, model) = standing_birch(&mut app, 1.0);
        send_cut_batch(&mut app, 6251);
        step(&mut app);
        assert_eq!(
            app.world()
                .get::<Falling>(tree)
                .expect("the fall starts")
                .path,
            BIRCH
        );

        let mut frames = 1;
        while app.world().get::<Falling>(tree).is_some() {
            let drawn = app.world().get::<DoodadVisual>(tree).unwrap();
            assert_eq!(
                drawn.func_group_id, 6251,
                "the phase change is known at once"
            );
            assert_eq!(
                drawn.rendered_func_group_id, 944,
                "frame {frames}: the cut phase waits for the landing"
            );
            assert_eq!((drawn.path.as_str(), drawn.model), (BIRCH, Some(model)));
            assert!(
                app.world().get_entity(model).is_ok(),
                "the standing model keeps falling"
            );
            assert!(frames < 200, "the tree never lands");
            step(&mut app);
            frames += 1;
        }
        assert!(frames as f32 / 60.0 > CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS - 0.1);

        // landed: the cut phase is drawn, and the tree stays down (the same model)
        step(&mut app);
        assert_eq!(
            app.world()
                .get::<DoodadVisual>(tree)
                .unwrap()
                .rendered_func_group_id,
            6251
        );
        assert!(app.world().get::<Fallen>(tree).is_some());
        assert!(tilt(app.world().get::<Transform>(tree).unwrap().rotation) > 1.0);
    }

    #[test]
    fn a_falling_tree_keeps_its_scale_until_the_cut_phase_is_drawn() {
        // The birch stands fully grown at 1.4. The cut phase 6251 does not grow (scale 1.0) and the
        // test phase 6252 grows from 0.5: neither may rescale the tree before the phase is drawn
        // (after the landing, with its model), and the scale is the phase's from that frame on.
        let extra = "INSERT INTO doodad_func_groups VALUES (6252, 407, '');
                     INSERT INTO doodad_func_growths VALUES (1, 180000, 500, 1000);
                     INSERT INTO doodad_phase_funcs VALUES (1, 6252, 'DoodadFuncGrowth', 1);";
        for (phase, after_landing) in [(6251, 1.0..=1.0), (6252, 0.5..=0.52)] {
            let (_db, mut app) = chain_app(&format!("scale-{phase}"), extra);
            let (tree, _) = standing_birch(&mut app, 1.4);
            send_cut_batch(&mut app, phase);
            let mut drawn_at = None;
            for frame in 1..200 {
                step(&mut app);
                let drawn = app.world().get::<DoodadVisual>(tree).unwrap();
                let scale = app.world().get::<Transform>(tree).unwrap().scale.x;
                if drawn.rendered_func_group_id == phase {
                    assert!(after_landing.contains(&scale), "phase {phase}: {scale}");
                    drawn_at = Some(frame);
                    break;
                }
                assert_eq!(scale, 1.4, "phase {phase}, frame {frame}");
            }
            let fall_secs = CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS;
            assert!(
                drawn_at.is_some_and(|frame| frame as f32 / 60.0 > fall_secs - 0.1),
                "phase {phase}: drawn at frame {drawn_at:?}"
            );
        }
    }

    #[test]
    fn the_removal_ends_a_falling_or_fallen_tree_and_leaves_no_swap_behind() {
        // 2026-10-30 capture (tree 407 phase 944): SCDoodadRemoved arrived 2367 ms after the
        // cutdown broadcast. The full fall (0.1 s delay + 2.2 s tip) ends 67 ms before that, so the
        // real removal meets a lying tree; one second in it meets a falling tree whose cut phase
        // is still waiting for the landing (a native fall that outlasts the removal).
        for (removal_secs, landed) in [(2.367, true), (1.0, false)] {
            assert_eq!(
                CUTDOWN_START_DELAY + CUTDOWN_FALL_SECS < removal_secs,
                landed
            );
            let (_db, mut app) = chain_app(&format!("removal-{removal_secs}"), "");
            // the app removes doodads in `receive_moves`, before the doodad chain
            app.add_systems(Update, receive_moves.before(sync_views));
            let (tree, model) = standing_birch(&mut app, 1.0);
            send_cut_batch(&mut app, 6251);
            for _ in 0..(removal_secs * 60.0_f32).round() as usize {
                step(&mut app);
            }
            let rendered = app
                .world()
                .get::<DoodadVisual>(tree)
                .unwrap()
                .rendered_func_group_id;
            assert_eq!(
                rendered,
                if landed { 6251 } else { 944 },
                "{removal_secs} s"
            );
            assert_eq!(
                app.world().get::<Falling>(tree).is_none(),
                landed,
                "{removal_secs} s"
            );
            assert_eq!(
                app.world().get::<Fallen>(tree).is_some(),
                landed,
                "{removal_secs} s"
            );
            assert!(
                tilt(app.world().get::<Transform>(tree).unwrap().rotation)
                    > if landed { 1.0 } else { 0.1 }
            );

            app.world_mut()
                .write_message(NetEvent(Event::DoodadsRemoved(vec![2])));
            for _ in 0..5 {
                step(&mut app);
            }
            assert!(
                app.world().get_entity(tree).is_err(),
                "{removal_secs} s: the root is gone"
            );
            assert!(
                app.world().get_entity(model).is_err(),
                "{removal_secs} s: and its model"
            );
            assert!(app.world().resource::<Views>().doodad_entity(2).is_none());
            assert_eq!(
                count::<Falling>(&mut app) + count::<Fallen>(&mut app),
                0,
                "{removal_secs} s"
            );
            assert_eq!(
                count::<DoodadVisual>(&mut app) + count::<Mesh3d>(&mut app),
                0,
                "nothing is drawn"
            );

            // a stale swap would be applied to a doodad that is drawn again under the same id
            let again = app
                .world_mut()
                .spawn((
                    RemoteDoodad { obj_id: 2 },
                    Transform::default(),
                    visual(BIRCH, 944, 0),
                ))
                .id();
            app.world_mut()
                .resource_mut::<Views>()
                .fixture_doodad(2, again);
            for _ in 0..5 {
                step(&mut app);
            }
            let drawn = app.world().get::<DoodadVisual>(again).unwrap();
            assert_eq!(
                drawn.rendered_func_group_id, 0,
                "{removal_secs} s: no swap was left queued"
            );
        }
    }

    #[test]
    fn landing_keeps_the_tree_lying_until_the_deferred_model_is_drawn() {
        let mut app = App::new();
        app.add_systems(Update, (land, settle));
        let base = Quat::from_rotation_y(0.4);
        let lying = Quat::from_rotation_x(1.0);
        let tree = app
            .world_mut()
            .spawn((
                RemoteDoodad { obj_id: 1 },
                Falling {
                    base,
                    axis: Vec3::X,
                    elapsed: 10.0,
                    path: "tree".into(),
                },
                visual("tree", 3, 2),
                Transform::from_rotation(lying),
            ))
            .id();
        app.update();
        assert!(app.world().get::<Falling>(tree).is_none());
        assert!(app.world().get::<Fallen>(tree).is_some());
        // no upright frame while the queued phase model has not been drawn yet
        assert!(rotation_error(app.world().get::<Transform>(tree).unwrap().rotation, lying) < 1e-5);

        // phase_changes drew the deferred model: the root goes upright in the same frame
        app.world_mut().get_mut::<DoodadVisual>(tree).unwrap().path = "stump".into();
        app.update();
        assert!(app.world().get::<Fallen>(tree).is_none());
        assert!(rotation_error(app.world().get::<Transform>(tree).unwrap().rotation, base) < 1e-5);
    }

    #[test]
    fn a_same_model_cut_phase_keeps_the_tree_down_until_the_model_changes() {
        let mut app = App::new();
        app.add_systems(Update, (land, settle));
        let base = Quat::from_rotation_y(0.4);
        let tree = app
            .world_mut()
            .spawn((
                RemoteDoodad { obj_id: 1 },
                Falling {
                    base,
                    axis: Vec3::X,
                    elapsed: 10.0,
                    path: "tree".into(),
                },
                visual("tree", 2, 2),
                Transform::from_rotation(Quat::from_rotation_x(1.0)),
            ))
            .id();
        app.update();
        assert!(app.world().get::<Falling>(tree).is_none());
        assert!(app.world().get::<Fallen>(tree).is_some());
        assert!(tilt(app.world().get::<Transform>(tree).unwrap().rotation) > 0.5);

        // the later phase replaces the model: the new model must not inherit the fallen root
        app.world_mut().get_mut::<DoodadVisual>(tree).unwrap().path = "stump".into();
        app.update();
        assert!(app.world().get::<Fallen>(tree).is_none());
        assert!(rotation_error(app.world().get::<Transform>(tree).unwrap().rotation, base) < 1e-5);
    }
}
