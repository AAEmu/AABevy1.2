//! The app clock audit: does game time run at wall rate?
//!
//! Bevy's [`Time`] is [`Time<Virtual>`](bevy::time::Virtual), normally advanced from `Time<Real>` (the wall clock), so
//! `game/wall` is 1 and a timing read off the app clock is a real duration. `TimeUpdateStrategy::ManualDuration` (the
//! `--record` frame capture and replay examples that need a deterministic step) advances the app clock by a fixed
//! amount per frame instead; a loop that runs faster than that step makes game time outrun wall time. This module
//! measures the ratio every frame and writes it into the `aa-dev` log line; `--require-wall-clock` turns a comparison
//! run into an error when it is not 1.

use std::fmt;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;

/// How far `game/wall` may drift from 1 in a run that claims wall time.
pub const TOLERANCE: f64 = 0.05;
/// Wall seconds measured before [`ClockAudit::check`] judges the ratio (asset loading must not decide it).
pub const WARMUP: f64 = 1.0;

/// What advances the app clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockMode {
    /// `TimeUpdateStrategy::Automatic`: the render/wall clock. Game time is wall time.
    Wall,
    /// A manual strategy (`ManualDuration`, `ManualInstant`, `FixedTimesteps`): game time is the tool's step.
    Manual,
}

impl fmt::Display for ClockMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Wall => "wall",
            Self::Manual => "manual",
        })
    }
}

/// Game seconds per wall second, measured since the current strategy began.
#[derive(Resource, Debug, Default)]
pub struct ClockAudit {
    mode: Option<ClockMode>,
    start: Option<(web_time::Instant, f64)>,
    wall: Duration,
    game: f64,
    frames: u64,
}

impl ClockAudit {
    pub fn mode(&self) -> Option<ClockMode> {
        self.mode
    }

    pub fn mode_name(&self) -> &'static str {
        match self.mode {
            Some(ClockMode::Wall) => "wall",
            Some(ClockMode::Manual) => "manual",
            None => "unknown",
        }
    }

    pub fn wall_secs(&self) -> f64 {
        self.wall.as_secs_f64()
    }

    pub fn game_secs(&self) -> f64 {
        self.game
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Game seconds per wall second in the current mode (1 before the first sample).
    pub fn ratio(&self) -> f64 {
        let wall = self.wall.as_secs_f64();
        if wall > 0.0 { self.game / wall } else { 1.0 }
    }

    /// One observation. A change of mode starts a new measurement window, so `--record` does not mix its fixed
    /// step with the wall-clock loading period before it.
    pub fn sample(&mut self, mode: ClockMode, wall_now: web_time::Instant, game_now: f64) -> f64 {
        match (self.start, self.mode) {
            (Some((wall_start, game_start)), Some(previous)) if previous == mode => {
                self.wall = wall_now.duration_since(wall_start);
                self.game = game_now - game_start;
                self.frames += 1;
            }
            _ => {
                self.mode = Some(mode);
                self.start = Some((wall_now, game_now));
                self.wall = Duration::ZERO;
                self.game = 0.0;
                self.frames = 0;
            }
        }
        self.ratio()
    }

    /// `Ok` when game time is wall time within `tolerance`; `Err` names the measured clock and rate.
    pub fn check(&self, tolerance: f64) -> Result<(), String> {
        if self.wall < Duration::from_secs_f64(WARMUP) {
            return Err(format!(
                "only {:.2} s of wall time measured",
                self.wall_secs()
            ));
        }
        let ratio = self.ratio();
        if (ratio - 1.0).abs() > tolerance {
            return Err(format!(
                "game time runs {ratio:.3}x wall time (clock={}, {:.2} game s in {:.2} wall s)",
                self.mode_name(),
                self.game,
                self.wall_secs()
            ));
        }
        Ok(())
    }
}

pub(crate) fn plugin(app: &mut App, require_wall_clock: bool) {
    app.init_resource::<ClockAudit>()
        .add_systems(First, sample_clock.after(bevy::time::TimeSystems));
    if require_wall_clock {
        app.add_systems(Update, check_wall_clock);
    }
}

fn sample_clock(mut audit: ResMut<ClockAudit>, time: Res<Time>, strategy: Res<TimeUpdateStrategy>) {
    let mode = match &*strategy {
        TimeUpdateStrategy::Automatic => ClockMode::Wall,
        _ => ClockMode::Manual,
    };
    audit.sample(mode, web_time::Instant::now(), time.elapsed_secs_f64());
}

/// `--require-wall-clock`: a comparison run that claims seconds must not be on a fixed step.
fn check_wall_clock(
    audit: Res<ClockAudit>,
    mut checked: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    if *checked || audit.wall_secs() < WARMUP {
        return;
    }
    *checked = true;
    match audit.check(TOLERANCE) {
        Ok(()) => info!("aa-dev: wall clock ok (game/wall {:.3})", audit.ratio()),
        Err(why) => {
            error!("aa-dev: --require-wall-clock: {why}");
            exit.write(AppExit::error());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_app(strategy: TimeUpdateStrategy) -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(strategy)
            .init_resource::<ClockAudit>()
            .add_systems(First, sample_clock.after(bevy::time::TimeSystems));
        app
    }

    #[test]
    fn manual_duration_at_an_uncapped_rate_fails_the_wall_clock_check() {
        // The 2026-10-26 combat side-by-side: 1/60 s of game time per frame while the loop ran at about 94 fps.
        let mut app = sample_app(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        // Keep the loop uncapped for the audit's 1 s warmup: the clock check must reject the rate, not the short window.
        let start = web_time::Instant::now();
        while start.elapsed() < Duration::from_millis(1100) {
            app.update();
            std::thread::sleep(Duration::from_millis(2));
        }
        let audit = app.world().resource::<ClockAudit>();
        assert_eq!(audit.mode(), Some(ClockMode::Manual));
        assert!(
            audit.ratio() > 2.0,
            "uncapped 1/60 s frames: game/wall {:.3}",
            audit.ratio()
        );
        assert!(
            audit.check(TOLERANCE).is_err(),
            "a manual clock must not pass --require-wall-clock"
        );
    }

    #[test]
    fn real_time_advances_game_time_at_wall_rate() {
        let mut app = sample_app(TimeUpdateStrategy::Automatic);
        let start = web_time::Instant::now();
        while start.elapsed() < Duration::from_millis(1100) {
            app.update();
            std::thread::sleep(Duration::from_millis(2));
        }
        let audit = app.world().resource::<ClockAudit>();
        assert_eq!(audit.mode(), Some(ClockMode::Wall));
        assert!(
            audit.check(TOLERANCE).is_ok(),
            "game/wall {:.3}: {audit:?}",
            audit.ratio()
        );
    }

    #[test]
    fn switching_to_a_fixed_step_starts_a_new_window() {
        let mut audit = ClockAudit::default();
        let start = web_time::Instant::now();
        audit.sample(ClockMode::Wall, start, 10.0);
        audit.sample(ClockMode::Wall, start + Duration::from_secs(2), 12.0);
        assert!((audit.ratio() - 1.0).abs() < 1e-9);
        // `--record` starts here: the 10 s of loading before it must not dilute the fixed step.
        audit.sample(ClockMode::Manual, start + Duration::from_secs(2), 20.0);
        assert_eq!(audit.mode(), Some(ClockMode::Manual));
        assert_eq!(audit.wall_secs(), 0.0);
        audit.sample(ClockMode::Manual, start + Duration::from_secs(3), 30.0);
        assert!((audit.ratio() - 10.0).abs() < 1e-9, "{}", audit.ratio());
    }
}
