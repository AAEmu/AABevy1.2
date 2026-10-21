//! Which jump clip 1.2 plays, and when it moves on: the on-foot jump states of `game/animations/graphs/player_full_body.xml`
//! and their templates (`game/libs/animationgraphtemplates/jumpmain.xml`, `jumploop.xml`, `jumpend.xml`), read 2026-10-20.
//! Plain Rust, no Bevy type: [`super::anim::drive_locomotion`] plays what these rules choose.
//!
//! * **Take-off** (`JumpMain`, Action `jumpMP01`, `jumpMP02` when walking): `[item1]_mo_[stance]_jump_[dir]_start`, the
//!   direction the keys ask for at the press. It is not left before 0.98 of its length (`stayInStateUntil`), then holds its
//!   last frame. The take-off clip lasts about as long as a flat jump is in the air (0.645 s), so on flat ground the
//!   landing usually follows it straight away.
//! * **In the air** (`JumpLoop`): `[item1]_mo_[stance]_jump_[dir]_loop`, chosen again when the asked direction changes
//!   (`allowSelect`); it plays once and holds its last frame (`LoopAnimation 0`, `RepeatLastKey 1`).
//! * **Landing** (`JumpEnd`), by the direction asked at touchdown: none (`s`) plays `[item1]_mo_[stance]_jump_s_end`, a
//!   standing landing; a direction plays `[item1]_mo_[stance]_jump_[dir]_land`, a landing on the run. Walking lands with
//!   `fist_mo_relaxed_jump_walk_[dir]_land`. It holds until 0.8 of its length (`stayInStateUntil`), or less when the
//!   asked direction changes (`leaveRequestChanged`).
//! * **Diagonals:** unarmed relaxed, fl and fr play the forward clip and bl, br the backward one with the pelvis turned
//!   45 degrees; unarmed combat plays the side clip. The pelvis turn is not drawn (aa-anim has none).
//! * **Dash** (stance `sprint`, buff 2675): `fist_mo_jump_sprint_[dir]_*`, standing `fist_mo_jump_sprint_s_end`; backward
//!   ones take the relaxed backward clips. Its take-off blends in at once (transition 0).

/// The phases of a jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpPhase {
    Start,
    Loop,
    Land,
    /// A long fall (move mode 1, Action `freefall`): the graph's `fall` state, `all_re_combat_fall`, whatever the keys ask.
    Fall,
}

/// `pl_fall_start_velocity` (m/s) and `pl_fall_start_height` (m): a fall faster than the one and higher above the terrain than the
/// other is a freefall (`CActor::UpdateInAir`, move mode 0 to 1); after a jump the height is at least [`FALL_START_HEIGHT_JUMPED`].
pub const FALL_START_VELOCITY: f32 = 10.0;
pub const FALL_START_HEIGHT: f32 = 2.0;
pub const FALL_START_HEIGHT_JUMPED: f32 = 3.0;

/// Whether the unit is in a freefall this frame. `was` is last frame's answer: mode 1 holds until the unit is back on the ground
/// or in water (then `airborne` is false). `fall_speed` is the downward speed (m/s, positive falling), `height` the distance above
/// the **terrain** (objects do not count), `jumped` the air time began with a jump.
pub fn freefall(was: bool, airborne: bool, fall_speed: f32, height: f32, jumped: bool) -> bool {
    if !airborne {
        return false;
    }
    was || (fall_speed > FALL_START_VELOCITY && height > if jumped { FALL_START_HEIGHT_JUMPED } else { FALL_START_HEIGHT })
}

/// The phase the unit plays: in a freefall the air clip gives way to the fall clip (a take-off is held to its end first).
pub fn in_freefall(phase: (JumpPhase, JumpDir), freefall: bool) -> (JumpPhase, JumpDir) {
    match phase {
        (JumpPhase::Loop, _) if freefall => (JumpPhase::Fall, JumpDir::S),
        p => p,
    }
}

/// `RequestedMoveDir4LH`: the direction the keys ask for, relative to the facing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JumpDir {
    S,
    F,
    B,
    L,
    R,
    Fl,
    Fr,
    Bl,
    Br,
}

impl JumpDir {
    /// From a direction (x right, y forward; zero: standing).
    pub fn of(right: f32, forward: f32) -> JumpDir {
        let (f, b, l, r) = (forward > 0.01, forward < -0.01, right < -0.01, right > 0.01);
        match (f, b, l, r) {
            (true, _, true, _) => JumpDir::Fl,
            (true, _, _, true) => JumpDir::Fr,
            (true, _, _, _) => JumpDir::F,
            (_, true, true, _) => JumpDir::Bl,
            (_, true, _, true) => JumpDir::Br,
            (_, true, _, _) => JumpDir::B,
            (_, _, true, _) => JumpDir::L,
            (_, _, _, true) => JumpDir::R,
            _ => JumpDir::S,
        }
    }

    fn code(self) -> &'static str {
        match self {
            JumpDir::S => "s",
            JumpDir::F => "f",
            JumpDir::B => "b",
            JumpDir::L => "l",
            JumpDir::R => "r",
            JumpDir::Fl => "fl",
            JumpDir::Fr => "fr",
            JumpDir::Bl => "bl",
            JumpDir::Br => "br",
        }
    }

    /// Relaxed (and walking): a diagonal plays the forward or backward clip (with the pelvis turned).
    fn relaxed(self) -> &'static str {
        match self {
            JumpDir::Fl | JumpDir::Fr => "f",
            JumpDir::Bl | JumpDir::Br => "b",
            d => d.code(),
        }
    }

    /// Combat: a diagonal plays the side clip.
    fn combat(self) -> &'static str {
        match self {
            JumpDir::Fl | JumpDir::Bl => "l",
            JumpDir::Fr | JumpDir::Br => "r",
            d => d.code(),
        }
    }
}

/// What the character jumps as: the graph's `Stance` and `Item1`, and walking (`jumpMP02`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JumpKind {
    /// Unarmed or the weapon away (`fist`, `relaxed`).
    Relaxed,
    /// Walking (`.` or Shift): relaxed take-off and air, the walking landing.
    Walk,
    /// Dash is on (stance `sprint`).
    Sprint,
    /// The weapon out: `[item1]_mo_combat_*` (`fist`, `onehand`, `twohand`, `staff`, `music`).
    Combat(&'static str),
}

/// The clip names to try for a phase, best first: the graph's choice, then the relaxed unarmed one, then the old single air
/// clip. The names are `.cal` aliases (`fist_mo_relaxed_jump_s_start = fist\fist_mo_jump_s_start.caf` in `global_player.cal`).
pub fn clip_names(phase: JumpPhase, dir: JumpDir, kind: JumpKind) -> Vec<String> {
    if phase == JumpPhase::Fall {
        return vec!["all_re_combat_fall".to_string(), "fist_mo_relaxed_jump_s_loop".to_string()];
    }
    let suffix = |d: &str| match phase {
        JumpPhase::Start => format!("{d}_start"),
        JumpPhase::Loop | JumpPhase::Fall => format!("{d}_loop"),
        JumpPhase::Land if d == "s" => "s_end".to_string(),
        JumpPhase::Land => format!("{d}_land"),
    };
    let relaxed = format!("fist_mo_relaxed_jump_{}", suffix(dir.relaxed()));
    let first = match kind {
        JumpKind::Relaxed => relaxed.clone(),
        JumpKind::Walk if phase == JumpPhase::Land && dir != JumpDir::S => format!("fist_mo_relaxed_jump_walk_{}_land", dir.relaxed()),
        JumpKind::Walk => relaxed.clone(),
        JumpKind::Sprint => match dir {
            JumpDir::B | JumpDir::Bl | JumpDir::Br => format!("fist_mo_relaxed_jump_{}", suffix("b")),
            d => format!("fist_mo_jump_sprint_{}", suffix(d.relaxed())),
        },
        JumpKind::Combat(item) => format!("{item}_mo_combat_jump_{}", suffix(dir.combat())),
    };
    let mut names = vec![first];
    for n in [relaxed, "fist_mo_relaxed_jump_s_loop".to_string()] {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    names
}

/// The cross-fade into a phase (s): the graph's `transition_time` (a Dash take-off has none).
pub fn fade(phase: JumpPhase, kind: JumpKind) -> f32 {
    match (phase, kind) {
        (JumpPhase::Start, JumpKind::Sprint) => 0.0,
        (JumpPhase::Start, _) | (JumpPhase::Land, _) => 0.1,
        (JumpPhase::Loop | JumpPhase::Fall, _) => 0.2,
    }
}

/// `stayInStateUntil` of `JumpMain` and `JumpEnd`: the share of the clip played before it may be left.
pub const START_HOLD: f32 = 0.98;
pub const LAND_HOLD: f32 = 0.8;

/// The jump a unit is in: its phase and the direction that phase was chosen with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JumpTrack {
    pub phase: Option<(JumpPhase, JumpDir)>,
}

impl JumpTrack {
    /// One frame: `airborne` off the ground, `rising` the air time began with a take-off (a fall off a ledge does not),
    /// `dir` the direction asked now, `progress` the share of the phase's clip played (0..1). Returns the phase to play
    /// (`None`: back to standing or running) and stores it; a phase or direction other than the stored one means a new clip.
    pub fn step(&mut self, airborne: bool, rising: bool, dir: JumpDir, progress: f32) -> Option<(JumpPhase, JumpDir)> {
        let next = match self.phase {
            None | Some((JumpPhase::Land, _)) if airborne && rising => Some((JumpPhase::Start, dir)),
            None | Some((JumpPhase::Land, _)) if airborne => Some((JumpPhase::Loop, dir)),
            None => None,
            Some((JumpPhase::Start, d)) if progress < START_HOLD => Some((JumpPhase::Start, d)),
            // (the track itself never holds Fall: the unit's freefall turns its Loop into Fall when played)
            Some((JumpPhase::Start | JumpPhase::Loop | JumpPhase::Fall, _)) if !airborne => Some((JumpPhase::Land, dir)),
            Some((JumpPhase::Start | JumpPhase::Loop | JumpPhase::Fall, _)) => Some((JumpPhase::Loop, dir)),
            Some((JumpPhase::Land, d)) if progress < LAND_HOLD && d == dir => Some((JumpPhase::Land, d)),
            Some((JumpPhase::Land, _)) => None,
        };
        self.phase = next;
        next
    }

    /// An action on the whole body played over the jump (a swing, a cast): its phase is over. In the air the unit goes on in
    /// the air clip; on the ground the jump is done (a landing is not played again after the action).
    pub fn interrupted(&mut self, airborne: bool) {
        self.phase = match self.phase {
            Some((_, d)) if airborne => Some((JumpPhase::Loop, d)),
            _ => None,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_standing_jump_and_a_running_jump_take_their_own_clips() {
        let first = |p, d, k| clip_names(p, d, k)[0].clone();
        assert_eq!(first(JumpPhase::Start, JumpDir::S, JumpKind::Relaxed), "fist_mo_relaxed_jump_s_start");
        assert_eq!(first(JumpPhase::Loop, JumpDir::S, JumpKind::Relaxed), "fist_mo_relaxed_jump_s_loop");
        assert_eq!(first(JumpPhase::Land, JumpDir::S, JumpKind::Relaxed), "fist_mo_relaxed_jump_s_end");
        assert_eq!(first(JumpPhase::Start, JumpDir::F, JumpKind::Relaxed), "fist_mo_relaxed_jump_f_start");
        assert_eq!(first(JumpPhase::Loop, JumpDir::F, JumpKind::Relaxed), "fist_mo_relaxed_jump_f_loop");
        assert_eq!(first(JumpPhase::Land, JumpDir::F, JumpKind::Relaxed), "fist_mo_relaxed_jump_f_land");
        assert_eq!(first(JumpPhase::Land, JumpDir::L, JumpKind::Relaxed), "fist_mo_relaxed_jump_l_land");
        // diagonals: the forward / backward clip relaxed, the side clip in combat
        assert_eq!(first(JumpPhase::Start, JumpDir::Fl, JumpKind::Relaxed), "fist_mo_relaxed_jump_f_start");
        assert_eq!(first(JumpPhase::Land, JumpDir::Br, JumpKind::Relaxed), "fist_mo_relaxed_jump_b_land");
        assert_eq!(first(JumpPhase::Loop, JumpDir::Bl, JumpKind::Combat("fist")), "fist_mo_combat_jump_l_loop");
        assert_eq!(first(JumpPhase::Land, JumpDir::S, JumpKind::Combat("onehand")), "onehand_mo_combat_jump_s_end");
        // walking lands on the walking landing; standing still it is the standing one
        assert_eq!(first(JumpPhase::Start, JumpDir::F, JumpKind::Walk), "fist_mo_relaxed_jump_f_start");
        assert_eq!(first(JumpPhase::Land, JumpDir::F, JumpKind::Walk), "fist_mo_relaxed_jump_walk_f_land");
        assert_eq!(first(JumpPhase::Land, JumpDir::S, JumpKind::Walk), "fist_mo_relaxed_jump_s_end");
        // Dash
        assert_eq!(first(JumpPhase::Start, JumpDir::F, JumpKind::Sprint), "fist_mo_jump_sprint_f_start");
        assert_eq!(first(JumpPhase::Land, JumpDir::Fr, JumpKind::Sprint), "fist_mo_jump_sprint_f_land");
        assert_eq!(first(JumpPhase::Land, JumpDir::S, JumpKind::Sprint), "fist_mo_jump_sprint_s_end");
        assert_eq!(first(JumpPhase::Loop, JumpDir::Bl, JumpKind::Sprint), "fist_mo_relaxed_jump_b_loop");
        // the fallbacks end at the old air clip
        assert_eq!(clip_names(JumpPhase::Land, JumpDir::F, JumpKind::Sprint).last().unwrap(), "fist_mo_relaxed_jump_s_loop");
        assert_eq!(fade(JumpPhase::Start, JumpKind::Sprint), 0.0);
        assert_eq!(fade(JumpPhase::Loop, JumpKind::Relaxed), 0.2);
    }

    #[test]
    fn a_long_fall_plays_its_own_clip_until_the_ground() {
        // too slow or too low: no freefall; a jump asks 3 m, a fall off a ledge 2 m
        assert!(!freefall(false, true, 9.0, 30.0, false));
        assert!(!freefall(false, true, 15.0, 2.0, false));
        assert!(freefall(false, true, 15.0, 2.5, false));
        assert!(!freefall(false, true, 15.0, 2.5, true));
        assert!(freefall(false, true, 15.0, 3.5, true));
        // once begun it holds while the fall slows or nears the ground, and ends on the ground
        assert!(freefall(true, true, 3.0, 0.5, false));
        assert!(!freefall(true, false, 30.0, 0.0, false));
        // the air clip gives way to the fall clip, the take-off is held first
        assert_eq!(in_freefall((JumpPhase::Loop, JumpDir::F), true), (JumpPhase::Fall, JumpDir::S));
        assert_eq!(in_freefall((JumpPhase::Loop, JumpDir::F), false), (JumpPhase::Loop, JumpDir::F));
        assert_eq!(in_freefall((JumpPhase::Start, JumpDir::F), true), (JumpPhase::Start, JumpDir::F));
        assert_eq!(clip_names(JumpPhase::Fall, JumpDir::S, JumpKind::Relaxed)[0], "all_re_combat_fall");
        assert_eq!(fade(JumpPhase::Fall, JumpKind::Combat("fist")), 0.2);
    }

    #[test]
    fn the_keys_give_the_jump_direction() {
        assert_eq!(JumpDir::of(0.0, 0.0), JumpDir::S);
        assert_eq!(JumpDir::of(0.0, 1.0), JumpDir::F);
        assert_eq!(JumpDir::of(-0.7, 0.7), JumpDir::Fl);
        assert_eq!(JumpDir::of(0.7, -0.7), JumpDir::Br);
        assert_eq!(JumpDir::of(1.0, 0.0), JumpDir::R);
    }

    #[test]
    fn a_jump_takes_off_flies_and_lands_standing_or_running() {
        let mut j = JumpTrack::default();
        assert_eq!(j.step(false, false, JumpDir::F, 0.0), None);
        // take-off running forward; it is held to 0.98 even when the ground comes first
        assert_eq!(j.step(true, true, JumpDir::F, 0.0), Some((JumpPhase::Start, JumpDir::F)));
        assert_eq!(j.step(false, false, JumpDir::S, 0.5), Some((JumpPhase::Start, JumpDir::F)));
        // W let go in the air: the landing is the standing one
        assert_eq!(j.step(false, false, JumpDir::S, 0.99), Some((JumpPhase::Land, JumpDir::S)));
        assert_eq!(j.step(false, false, JumpDir::S, 0.5), Some((JumpPhase::Land, JumpDir::S)));
        assert_eq!(j.step(false, false, JumpDir::S, 0.85), None);

        // a long jump: the air clip follows the keys, the landing goes on the run
        let mut j = JumpTrack::default();
        j.step(true, true, JumpDir::S, 0.0);
        assert_eq!(j.step(true, false, JumpDir::S, 0.99), Some((JumpPhase::Loop, JumpDir::S)));
        assert_eq!(j.step(true, false, JumpDir::F, 0.3), Some((JumpPhase::Loop, JumpDir::F)));
        assert_eq!(j.step(false, false, JumpDir::F, 1.0), Some((JumpPhase::Land, JumpDir::F)));
        // a key pressed during the landing leaves it for the run at once
        assert_eq!(j.step(false, false, JumpDir::Fl, 0.2), None);

        // walking off a ledge: no take-off
        let mut j = JumpTrack::default();
        assert_eq!(j.step(true, false, JumpDir::F, 0.0), Some((JumpPhase::Loop, JumpDir::F)));
        // a jump again during a landing
        j.step(false, false, JumpDir::F, 1.0);
        assert_eq!(j.step(true, true, JumpDir::F, 0.1), Some((JumpPhase::Start, JumpDir::F)));
    }

    #[test]
    fn an_action_during_a_jump_does_not_replay_its_phase() {
        // a swing during the standing landing: back on the ground the landing is not played again
        let mut j = JumpTrack::default();
        j.step(true, true, JumpDir::S, 0.0);
        j.step(false, false, JumpDir::S, 0.99);
        assert_eq!(j.phase, Some((JumpPhase::Land, JumpDir::S)));
        j.interrupted(false);
        assert_eq!(j.step(false, false, JumpDir::S, 1.0), None);
        // a cast during the take-off: the air clip follows, not the take-off again
        let mut j = JumpTrack::default();
        j.step(true, true, JumpDir::F, 0.0);
        j.interrupted(true);
        assert_eq!(j.step(true, false, JumpDir::F, 1.0), Some((JumpPhase::Loop, JumpDir::F)));
    }

    /// The names resolve through the Nuian man's `.cal` (and the `global_player.cal` it includes) to clips that load. Needs
    /// the 1.2 `game_pak` (`AA_PAK`).
    #[test]
    fn every_on_foot_jump_clip_resolves_for_a_nuian_man() {
        let Ok(cal) = aa_anim::Cal::load("objects/characters/nuian/male/nude/nu_m_base.cal") else {
            eprintln!("no game_pak: skipped");
            return;
        };
        let library = aa_anim::ClipLibrary::default();
        let dirs = [JumpDir::S, JumpDir::F, JumpDir::B, JumpDir::L, JumpDir::R, JumpDir::Fl, JumpDir::Fr, JumpDir::Bl, JumpDir::Br];
        let mut lengths = Vec::new();
        for kind in [JumpKind::Relaxed, JumpKind::Walk, JumpKind::Sprint, JumpKind::Combat("fist")] {
            for phase in [JumpPhase::Start, JumpPhase::Loop, JumpPhase::Land] {
                for dir in dirs {
                    let name = clip_names(phase, dir, kind)[0].clone();
                    let entry = cal.animation(&name).unwrap_or_else(|| panic!("{name} is not in the .cal"));
                    let clip = library.get(&entry.path).unwrap_or_else(|e| panic!("{name} = {}: {e}", entry.path));
                    assert!(clip.duration > 0.1, "{name}: {} s", clip.duration);
                    if kind == JumpKind::Relaxed && matches!(dir, JumpDir::S | JumpDir::F) {
                        lengths.push(format!("{name} {:.3} s", clip.duration));
                    }
                }
            }
        }
        let fall = cal.animation("all_re_combat_fall").expect("all_re_combat_fall is not in the .cal");
        let clip = library.get(&fall.path).unwrap_or_else(|e| panic!("all_re_combat_fall = {}: {e}", fall.path));
        lengths.push(format!("all_re_combat_fall {:.3} s", clip.duration));
        eprintln!("{}", lengths.join("\n"));
    }
}
