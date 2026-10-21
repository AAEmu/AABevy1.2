//! Dash (skill 16287, buff 2675) as 1.2's input code starts and ends it, its speed, and the blur while it runs.
//!
//! What 1.2 does (read in `x2game`):
//!
//! * **Start:** two presses of the forward binding less than 300 ms apart, no key repeat, no other key in between
//!   (`x2game+0x40e6f0`), with option 55 `use_celerity_with_double_forward` on (its default) and buff 2675 absent, use
//!   skill 16287 on the own unit (`x2game+0x409230`). In combat the skill is refused ("Can't be used in combat.": its unit
//!   requirement kind 12); here the server refuses it and the HUD says so.
//! * **End by the keys:** on a key release with forward, back and both strafe keys all up, buff 2675 present and the option
//!   on, the skill is used again (`x2game+0x40d930`), and using a toggle skill whose buff is on removes the buff instead of
//!   casting: cs 0x55 with the buff's index and reason 0 (`skill::use_checked` `x2game+0x3a6e20`) = `Command::RemoveBuff`.
//! * **Every other end is the server's:** the buff's `remove_on_` attack, attacked, damage, damaged, start_skill,
//!   use_skill, interaction, mount and death, and the drain (`tick_level_mana_cost` 0.5 x `level_mana` every 200 ms, no
//!   mana regeneration; the buff ends when a tick cannot be paid). 1.2's client buff tick never reads the drain.
//! * **Speed:** the own unit's buffs' `MoveSpeedMul` rows (`unit_modifiers`, attribute 10): flat ones add per mille,
//!   `value + linear_level_bonus x AbLevel / 100` rounded as the server's `BuffTemplate.Start`, percent ones multiply.
//!   Dash at ability level 1: 1.384 x 5.4 = 7.47 m/s.
//! * **The sprint run:** buff 2675 has `sprint_motion` (the record's +0x193, OR-ed over the unit's buffs into the buff manager's
//!   +0x2104 by `ClientBuffMan::UpdateCache` `x2game+0x397e00`, which also takes the earliest buff's `ag_stance`) and the stance
//!   `sprint`, whose graph state `SPRINT_RUN` plays the run group `_FIST_MO_NORMAL_RUN` (`fist_mo_normal_run.lmg`). That group
//!   lists the run and then a sprint tier (`fist_mo_sprint_run_*`, the run's own back clips); a 1.2 recording shows the
//!   forward-leaning sprint. Who reads +0x2104 was not found, so that `sprint_motion` picks the tier is a guess. The own unit with a
//!   `sprint_motion` buff runs with the sprint tier ([`Sprint::sprint_motion`], `anim::SprintClips`), at the sprint clip's own pace.
//! * **The view, the trail and the bursts:** the wider view is fx group 1034's `FxCamFov` (`super::cam_fov`); the start burst,
//!   the streaks while it runs and the end burst are its two particle items, played by the buff visuals (`combat::buffs`).
//! * **Blur:** the group's `FxMotionBlur` row turns CryEngine's `r_MotionBlur` to 1 for the own unit while the buff lasts and
//!   back after it (`x2game+0x3f2450`, `+0x3f24f0`): the engine's camera motion blur (shutter 0.015 s), which a straight run behind a
//!   following camera hardly shows; 1.2's frames of a Dash are sharp. Ours, a light radial blur of the screen's edges in
//!   `aa_world`'s post pass, is a different picture, so it is off unless the settings file's `s:AoAMotionBlur` (1 on, a fraction
//!   for less) or `AA_MOTION_BLUR` turns it on.

use std::collections::{HashMap, HashSet};

use aa_net::state::{Buffs, Player};
use aa_net::{Command, NetCommand};
use bevy::prelude::*;

use super::player::LocalPlayer;

pub const DASH_SKILL: u32 = 16287;
pub const DASH_BUFF: u32 = 2675;
/// The longest gap between the two presses of a double tap (ms).
pub const DOUBLE_TAP_MS: f64 = 300.0;
/// Seconds before a request that brought no change may be sent again (the skill's own cooldown is 1 s).
const RETRY: f64 = 1.0;
/// The blur's fade in and out (s): the view's (`fx_cam_fovs` 1, `fadeIn` 1, `fadeOut` 1).
pub const BLUR_FADE: f32 = 1.0;
/// How far the blur's samples reach at full strength, as a share of the way to the screen's centre.
pub const BLUR_REACH: f32 = 0.035;

/// 1.2's double-forward gesture.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoubleForward {
    last_forward: Option<f64>,
}

impl DoubleForward {
    /// A key press (no repeat) at `ms`: `forward` the forward binding, else any other key, which breaks the gesture. True on
    /// the second forward press within [`DOUBLE_TAP_MS`] of the first.
    pub fn press(&mut self, ms: f64, forward: bool) -> bool {
        if !forward {
            self.last_forward = None;
            return false;
        }
        let tap = self.last_forward.is_some_and(|t| ms >= t && ms - t < DOUBLE_TAP_MS);
        // a third quick press starts a new pair rather than tapping again
        self.last_forward = if tap { None } else { Some(ms) };
        tap
    }
}

/// What the input asks of Dash this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashAsk {
    Start,
    Stop,
}

/// 1.2's two input rules: a double tap starts Dash when it is off, letting go of the last direction key ends it when it is on;
/// both only with the option on.
pub fn dash_ask(option_on: bool, double_tap: bool, released_all_directions: bool, dashing: bool) -> Option<DashAsk> {
    match (option_on, dashing) {
        (false, _) => None,
        (true, false) if double_tap => Some(DashAsk::Start),
        (true, true) if released_all_directions => Some(DashAsk::Stop),
        _ => None,
    }
}

/// The native option `OPTION_ITEM_USE_CELERITY_WITH_DOUBLE_FORWARD`.
pub const CELERITY_OPTION: u32 = 55;

/// Whether the option is on, from the value the settings file holds for it (`None`: never set, the cvar's default of 1 applies,
/// `settings::native_default`). The Options window used to save 0 for it without being asked (it was shown unchecked), which switched
/// both the double tap and the release off.
pub fn celerity_on(saved: Option<f64>) -> bool {
    saved.or_else(|| crate::settings::native_default(CELERITY_OPTION)).is_some_and(|v| v != 0.0)
}

/// 1.2 looks at a key's release: a direction key was held last frame and none is now. Ours adds a Dash the double tap asked for whose
/// buff came after the keys were let go (`late_start`).
pub fn released_all(held_before: bool, held: bool, late_start: bool) -> bool {
    !held && (held_before || late_start)
}

/// A `MoveSpeedMul` row of a buff: `unit_modifier_type_id` (0 flat per mille, 1 percent), `value`, `linear_level_bonus`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeedRow {
    pub kind: i32,
    pub value: f32,
    pub linear: f32,
}

/// The move speed multiplier of the own unit's buffs, each with its ability level: the server's rounding of
/// `value + linear x level / 100`, flat rows summed on 1000, then the percent rows.
pub fn speed_multiplier<'a>(rows: impl IntoIterator<Item = (&'a SpeedRow, i32)>) -> f32 {
    let bonus = |r: &SpeedRow, level: i32| (r.value + r.linear * level as f32 / 100.0).round();
    let rows: Vec<(&SpeedRow, i32)> = rows.into_iter().collect();
    let mut total = 1000.0 + rows.iter().filter(|(r, _)| r.kind == 0).map(|(r, l)| bonus(r, *l)).sum::<f32>();
    for (r, l) in rows.iter().filter(|(r, _)| r.kind == 1) {
        total += total * bonus(r, *l) / 100.0;
    }
    (total / 1000.0).max(0.0)
}

/// The blur's strength one frame on: towards 1 over [`BLUR_FADE`] while Dash runs, back to 0 over the same from wherever it
/// got to (1.2's fade of the view, `x2game+0x3f2300`).
pub fn blur_step(level: f32, on: bool, dt: f32) -> f32 {
    let step = dt / BLUR_FADE;
    if on { (level + step).min(1.0) } else { (level - step).max(0.0) }
}

/// The eased strength the screen gets (a smoothstep: 1.2's curve object was not read).
pub fn blur_ease(level: f32) -> f32 {
    let x = level.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Dash of the local player, as the movement and the camera need it.
#[derive(Resource, Debug)]
pub struct Sprint {
    /// Buff 2675 is on the own unit.
    pub dashing: bool,
    /// The own buffs' move speed multiplier (1 without any).
    pub speed_mul: f32,
    /// One of the own buffs has `sprint_motion`: the run plays the sprint tier of the run group.
    pub sprint_motion: bool,
    /// The blur's level, 0..1 (before easing and the setting).
    pub blur: f32,
    gesture: DoubleForward,
    /// A direction key was held last frame.
    held_before: bool,
    /// Dash was on last frame.
    was_dashing: bool,
    /// What was asked last and when (s), so a request waits for the server's answer.
    asked: Option<(DashAsk, f64)>,
    rows: Option<HashMap<u32, Vec<SpeedRow>>>,
    /// The buffs with `sprint_motion` (Dash, the speed songs, ...).
    sprint_buffs: Option<HashSet<u32>>,
    /// The blur setting (0..1) and when it was read (s).
    blur_setting: (f32, f64),
}

impl Default for Sprint {
    fn default() -> Self {
        Sprint {
            dashing: false,
            speed_mul: 1.0,
            sprint_motion: false,
            blur: 0.0,
            gesture: DoubleForward::default(),
            held_before: false,
            was_dashing: false,
            asked: None,
            rows: None,
            sprint_buffs: None,
            blur_setting: (0.0, f64::NEG_INFINITY),
        }
    }
}

/// The `MoveSpeedMul` rows of every buff (533 rows in r208022's database).
fn load_rows() -> HashMap<u32, Vec<SpeedRow>> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let mut rows: HashMap<u32, Vec<SpeedRow>> = HashMap::new();
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI) else {
        warn!("sprint: no game database, buffs do not change the run speed");
        return rows;
    };
    let read = conn
        .prepare("SELECT owner_id, unit_modifier_type_id, value, linear_level_bonus FROM unit_modifiers WHERE owner_type = 'Buff' AND unit_attribute_id = 10")
        .and_then(|mut st| {
            st.query_map([], |r| {
                Ok((r.get::<_, i64>(0)? as u32, SpeedRow {
                    kind: r.get::<_, Option<i64>>(1)?.unwrap_or(0) as i32,
                    value: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0) as f32,
                    linear: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0) as f32,
                }))
            })?
            .collect::<Result<Vec<_>, _>>()
        });
    match read {
        Ok(list) => {
            for (id, row) in list {
                rows.entry(id).or_default().push(row);
            }
        }
        Err(e) => warn!("sprint: unit_modifiers: {e}"),
    }
    rows
}

/// The buffs whose `sprint_motion` is set.
fn load_sprint_buffs() -> HashSet<u32> {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let Ok(conn) = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI) else { return HashSet::new() };
    conn.prepare("SELECT id FROM buffs WHERE sprint_motion = 't'")
        .and_then(|mut st| st.query_map([], |r| Ok(r.get::<_, i64>(0)? as u32))?.collect::<Result<HashSet<_>, _>>())
        .unwrap_or_else(|e| {
            warn!("sprint: buffs.sprint_motion: {e}");
            HashSet::new()
        })
}

/// The blur setting: `AA_MOTION_BLUR`, else the settings file's `s:AoAMotionBlur`, else off (1.2's Dash shows none).
fn blur_setting() -> f32 {
    let value = std::env::var("AA_MOTION_BLUR").ok().and_then(|v| v.trim().parse::<f64>().ok()).or_else(|| crate::settings::option_number("s:AoAMotionBlur"));
    value.map_or(0.0, |v| v.clamp(0.0, 1.0) as f32)
}

/// The double tap, the release of the direction keys, the speed of the own buffs and the blur's level.
#[allow(clippy::too_many_arguments)]
pub fn drive_sprint(
    time: Res<Time>,
    actions: Res<crate::settings::Actions>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
    player: Query<&LocalPlayer>,
    spawn: Option<Res<Player>>,
    buffs: Option<Res<Buffs>>,
    mut sprint: ResMut<Sprint>,
    mut out: MessageWriter<NetCommand>,
    mut events: MessageReader<aa_net::NetEvent>,
) {
    let now = time.elapsed_secs_f64();
    for aa_net::NetEvent(event) in events.read() {
        if let aa_net::Event::BuffRemovalRequestRejected { buff_index, message, .. } = event {
            warn!("buff removal of index {buff_index} was not sent: {message}");
        }
    }
    let Ok(local) = player.single() else {
        sprint.dashing = false;
        sprint.speed_mul = 1.0;
        sprint.sprint_motion = false;
        sprint.blur = 0.0;
        return;
    };
    let own: &[aa_net::state::BuffEntry] = buffs.as_deref().map_or(&[], |b| b.of(local.obj_id));
    let dash = own.iter().find(|e| e.buff.buff_id == DASH_BUFF).map(|e| e.buff.index);
    sprint.dashing = dash.is_some();
    // the speed of every own buff, Dash's among them
    if sprint.rows.is_none() {
        sprint.rows = Some(load_rows());
    }
    let rows = sprint.rows.as_ref().expect("loaded above");
    let speed = speed_multiplier(own.iter().flat_map(|e| rows.get(&e.buff.buff_id).into_iter().flatten().map(move |r| (r, e.buff.ability_level as i32))));
    sprint.speed_mul = speed;
    if sprint.sprint_buffs.is_none() {
        sprint.sprint_buffs = Some(load_sprint_buffs());
    }
    let sprinters = sprint.sprint_buffs.as_ref().expect("loaded above");
    sprint.sprint_motion = own.iter().any(|e| sprinters.contains(&e.buff.buff_id));
    // the blur follows the buff
    let on = sprint.dashing;
    sprint.blur = blur_step(sprint.blur, on, time.delta_secs());
    if now - sprint.blur_setting.1 > 1.0 {
        sprint.blur_setting = (blur_setting(), now);
    }

    // the keys: nothing while an edit box has the keyboard
    if typing.is_some_and(|f| f.0) {
        sprint.gesture = DoubleForward::default();
        return;
    }
    let ms = now * 1000.0;
    let mut tap = false;
    for (name, index, button) in actions.hotkeys() {
        if button {
            continue;
        }
        tap |= sprint.gesture.press(ms, name == "moveforward" && index == 0);
    }
    let held = ["moveforward", "moveback", "moveleft", "moveright"].iter().any(|a| actions.pressed(a, 0));
    // 1.2 looks on a key's release; a Dash the double tap asked for whose buff arrives after the keys were let go (a quick
    // double tap and release) ends too, not one started from the action bar
    let started = sprint.dashing && !sprint.was_dashing;
    let tapped_lately = sprint.asked.is_some_and(|(a, t)| a == DashAsk::Start && now - t < RETRY);
    let released = released_all(sprint.held_before, held, started && tapped_lately);
    sprint.held_before = held;
    sprint.was_dashing = sprint.dashing;
    let option_on = celerity_on(crate::settings::option_number("n:55"));
    let Some(ask) = dash_ask(option_on, tap, released, sprint.dashing) else { return };
    if sprint.asked.is_some_and(|(a, t)| a == ask && now - t < RETRY) {
        return;
    }
    sprint.asked = Some((ask, now));
    // the character the session takes a buff removal for: the spawned one's (as the HUD's buff cancel names it)
    let character_id = spawn.as_deref().and_then(|p| p.spawn.as_ref()).map(|s| s.character().id);
    match (ask, dash, character_id) {
        (DashAsk::Start, _, _) => {
            info!("sprint: double forward, Dash asked");
            out.write(NetCommand(Command::UseSkill { skill_id: DASH_SKILL, target_obj_id: 0 }));
        }
        (DashAsk::Stop, Some(index), Some(character_id)) => {
            info!("sprint: every direction key let go, Dash ends");
            out.write(NetCommand(Command::RemoveBuff { character_id, obj_id: local.obj_id, buff_index: index }));
        }
        _ => {}
    }
}

/// The blur's strength on the world camera's post pass (`aa_world::post::PostUniform::motion`).
pub fn apply_blur(sprint: Res<Sprint>, mut cameras: Query<&mut aa_world::post::PostUniform, With<super::camera::OrbitCamera>>) {
    let strength = blur_ease(sprint.blur) * sprint.blur_setting.0;
    for mut u in &mut cameras {
        let wanted = Vec4::new(strength * BLUR_REACH, 0.5, 0.55, 0.0);
        if u.motion != wanted {
            u.motion = wanted;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_forward_presses_within_300_ms_ask_for_dash() {
        let mut g = DoubleForward::default();
        assert!(!g.press(0.0, true));
        assert!(g.press(108.0, true), "1.2's own tap: 0 and 108 ms");
        // too slow
        let mut g = DoubleForward::default();
        g.press(0.0, true);
        assert!(!g.press(300.0, true));
        assert!(g.press(450.0, true), "the late press starts a new pair");
        // another key in between breaks it
        let mut g = DoubleForward::default();
        g.press(0.0, true);
        g.press(50.0, false);
        assert!(!g.press(100.0, true));
        // a third quick press does not tap again
        let mut g = DoubleForward::default();
        g.press(0.0, true);
        assert!(g.press(100.0, true));
        assert!(!g.press(200.0, true));
    }

    #[test]
    fn dash_starts_on_a_double_tap_and_ends_when_every_direction_key_is_up() {
        assert_eq!(dash_ask(true, true, false, false), Some(DashAsk::Start));
        assert_eq!(dash_ask(true, true, false, true), None, "a double tap while it runs does nothing");
        assert_eq!(dash_ask(true, false, true, true), Some(DashAsk::Stop));
        assert_eq!(dash_ask(true, false, true, false), None);
        assert_eq!(dash_ask(false, true, false, false), None, "option 55 off");
        assert_eq!(dash_ask(false, false, true, true), None);
    }

    /// The option is on until a player turns it off: an option nobody set takes the cvar's default of 1, and the 0 the Options window
    /// used to save over it (the box was shown unchecked) is what kept both the double tap and the release from doing anything.
    #[test]
    fn dash_s_option_is_on_unless_a_player_turned_it_off() {
        assert!(celerity_on(None), "never set: 1.2's default is on");
        assert!(celerity_on(Some(1.0)));
        assert!(!celerity_on(Some(0.0)));
        // the Options window shows what the keys use
        assert_eq!(crate::settings::native_default(CELERITY_OPTION), Some(1.0));
    }

    /// The keys as 1.2 reads them, frame by frame: a double tap of forward starts Dash, letting go of every direction key ends it, with
    /// the option on; with it off (the saved 0) neither happens.
    #[test]
    fn a_double_tap_starts_dash_and_letting_go_of_every_key_ends_it() {
        for (saved, works) in [(None, true), (Some(1.0), true), (Some(0.0), false)] {
            let on = celerity_on(saved);
            let (mut gesture, mut held_before, mut dashing) = (DoubleForward::default(), false, false);
            let mut asks = Vec::new();
            // (ms, forward pressed this frame, a direction key held, the buff arrived this frame)
            for (ms, press, held) in [(0.0, true, true), (60.0, false, false), (120.0, true, true), (400.0, false, true), (900.0, false, false), (1000.0, false, false)] {
                let tap = if press { gesture.press(ms, true) } else { false };
                let ask = dash_ask(on, tap, released_all(held_before, held, false), dashing);
                held_before = held;
                match ask {
                    Some(DashAsk::Start) => {
                        asks.push((ms, DashAsk::Start));
                        dashing = true;
                    }
                    Some(DashAsk::Stop) => {
                        asks.push((ms, DashAsk::Stop));
                        dashing = false;
                    }
                    None => {}
                }
            }
            if works {
                assert_eq!(asks, [(120.0, DashAsk::Start), (900.0, DashAsk::Stop)], "{saved:?}");
            } else {
                assert!(asks.is_empty(), "{saved:?}: {asks:?}");
            }
        }
        // standing still with Dash on (started from the action bar) is no release: 1.2 looks at a key's release only
        assert!(!released_all(false, false, false));
        assert!(released_all(true, false, false));
        assert!(!released_all(true, true, false));
        // a Dash the double tap asked for whose buff came after the keys were let go ends at once
        assert!(released_all(false, false, true));
        assert!(!released_all(false, true, true));
    }

    #[test]
    fn dash_at_level_1_runs_at_7_47_metres_a_second() {
        let dash = SpeedRow { kind: 0, value: 380.0, linear: 400.0 };
        let m = speed_multiplier([(&dash, 1)]);
        assert!((m - 1.384).abs() < 1e-4, "{m}");
        assert!((5.4 * m - 7.47).abs() < 0.01);
        assert!((speed_multiplier([(&dash, 50)]) - 1.58).abs() < 1e-4);
        // no buff: as before
        assert_eq!(speed_multiplier(std::iter::empty()), 1.0);
        // a percent row multiplies what the flat ones made
        let slow = SpeedRow { kind: 1, value: -50.0, linear: 0.0 };
        assert!((speed_multiplier([(&dash, 1), (&slow, 1)]) - 0.692).abs() < 1e-4);
    }

    #[test]
    fn the_blur_fades_in_and_out_over_a_second() {
        let mut b = 0.0;
        for _ in 0..30 {
            b = blur_step(b, true, 1.0 / 60.0);
        }
        assert!((b - 0.5).abs() < 1e-4);
        // let go half way: it fades out from there, in half a second
        for _ in 0..30 {
            b = blur_step(b, false, 1.0 / 60.0);
        }
        assert!(b.abs() < 1e-4);
        for _ in 0..120 {
            b = blur_step(b, true, 1.0 / 60.0);
        }
        assert_eq!(b, 1.0);
        assert_eq!(blur_ease(0.0), 0.0);
        assert_eq!(blur_ease(1.0), 1.0);
        assert!((blur_ease(0.5) - 0.5).abs() < 1e-6);
    }

    /// Dash and the speed buffs that share its group have `sprint_motion`; the torch (stance `sprint` only) has not (needs `AA_DB`).
    #[test]
    fn dash_runs_with_the_sprint_motion() {
        let sprinters = load_sprint_buffs();
        if sprinters.is_empty() {
            eprintln!("no game database: skipped");
            return;
        }
        assert!(sprinters.contains(&DASH_BUFF));
        assert!(!sprinters.contains(&3634), "the torch");
    }

    /// The database's Dash row is the one the rule was read from (needs `AA_DB`).
    #[test]
    fn the_database_holds_dash_s_speed_row() {
        let rows = load_rows();
        let Some(dash) = rows.get(&DASH_BUFF) else {
            eprintln!("no game database: skipped");
            return;
        };
        assert_eq!(dash, &vec![SpeedRow { kind: 0, value: 380.0, linear: 400.0 }]);
    }
}
