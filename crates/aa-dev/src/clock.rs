//! The app clock audit: does game time run at wall rate?
//!
//! Bevy's [`Time`] is [`Time<Virtual>`](bevy::time::Virtual), normally advanced from `Time<Real>` (the wall clock), so
//! `game/wall` is 1 and a timing read off the app clock is a real duration. Two things break that, and both can be
//! installed long after start-up: `TimeUpdateStrategy::ManualDuration` (the `--record` frame capture, a replay example
//! that needs a deterministic step, and the 2026-10-26 combat side-by-side) replaces the wall clock with a fixed step,
//! and `Time<Virtual>::set_relative_speed` (`AA_COMBAT_SLOWMO`) scales it. The audit samples every frame,
//! `--require-wall-clock` judges every frame once [`WARMUP`] has passed, and the ratio is judged once more at
//! [`AppExit`](bevy::app::AppExit): a comparison run cannot report inflated seconds.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bevy::prelude::*;
use bevy::time::{Real, TimeUpdateStrategy, Virtual};

/// How far a rate may drift from 1 in a run that claims wall time.
pub const TOLERANCE: f64 = 0.05;
/// Wall seconds measured before [`ClockAudit::assess`] judges the clock (asset loading must not decide it).
pub const WARMUP: f64 = 1.0;
/// Wall seconds over which the rates are measured. Frame deltas are compared, never cumulative time from frame 0, so a
/// warm-up offset cannot distort the ratio; a clamped frame (a hitch) is left out entirely.
pub const WINDOW: f64 = 3.0;
/// A `Time<Real>` delta that differs from the wall delta by more than this is the render thread's one-frame lag (or its
/// catch-up) after a main-thread stall, not a rate; the frame is left out of both windows. 5 % of the shortest judged
/// window ([`WARMUP`]), so a stall right after the warm-up cannot decide the run.
const STALL: Duration = Duration::from_millis(50);

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

/// The wall clock the audit samples. The default reads the system clock; the tests step it by hand so no test sleeps.
#[derive(Resource, Clone)]
pub struct WallClock(Arc<dyn Fn() -> web_time::Instant + Send + Sync>);

impl Default for WallClock {
    fn default() -> Self {
        Self(Arc::new(web_time::Instant::now))
    }
}

impl WallClock {
    pub fn new(now: impl Fn() -> web_time::Instant + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    pub fn now(&self) -> web_time::Instant {
        (self.0)()
    }
}

/// A verdict on the app clock.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Assessment {
    /// Less than [`WARMUP`] wall seconds measured: nothing judged yet.
    Warmup,
    /// Game time is wall time within the tolerance.
    Ok,
    /// Game time runs slower than the wall clock; reported once, not a failure (the comparison-inflating bug is faster).
    Slow(String),
    /// The app clock is not the wall clock, or game time runs faster than it.
    Fail(String),
}

/// Game seconds per wall second over the last [`WINDOW`] seconds.
#[derive(Debug, Default)]
struct RateWindow {
    samples: VecDeque<(f64, f64)>,
    wall: f64,
    game: f64,
}

impl RateWindow {
    fn push(&mut self, wall: f64, game: f64) {
        if wall <= 0.0 {
            return;
        }
        self.samples.push_back((wall, game));
        self.wall += wall;
        self.game += game;
        while self.wall > WINDOW {
            let Some((wall, game)) = self.samples.pop_front() else {
                break;
            };
            self.wall -= wall;
            self.game -= game;
        }
    }

    fn clear(&mut self) {
        self.samples.clear();
        self.wall = 0.0;
        self.game = 0.0;
    }

    fn ratio(&self) -> f64 {
        if self.wall > 0.0 {
            self.game / self.wall
        } else {
            1.0
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    wall: web_time::Instant,
    virtual_elapsed: Duration,
    real_elapsed: Duration,
}

/// The measured app clock: total wall time (for the warm-up), and rolling `Virtual/wall` and `Real/wall` rates.
#[derive(Resource, Debug, Default)]
pub struct ClockAudit {
    mode: Option<ClockMode>,
    last: Option<Sample>,
    wall: Duration,
    frames: u64,
    virtual_window: RateWindow,
    real_window: RateWindow,
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

    /// Wall seconds measured since the first frame (not since the last mode change: a manual clock must fail at once).
    pub fn wall_secs(&self) -> f64 {
        self.wall.as_secs_f64()
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// `Time<Virtual>` game seconds per wall second over [`WINDOW`] (1 before the first sample).
    pub fn ratio(&self) -> f64 {
        self.virtual_window.ratio()
    }

    /// `Time<Real>` seconds per wall second over [`WINDOW`] (1 before the first sample).
    pub fn real_ratio(&self) -> f64 {
        self.real_window.ratio()
    }

    /// One observation: one frame's elapsed times. A mode change starts a new measurement window, so `--record` does
    /// not mix its fixed step with the wall-clock loading period before it; the warm-up clock never restarts.
    fn sample(
        &mut self,
        mode: ClockMode,
        wall_now: web_time::Instant,
        virtual_elapsed: Duration,
        real_elapsed: Duration,
        max_delta: Duration,
    ) {
        let changed = self.mode != Some(mode);
        if changed {
            self.mode = Some(mode);
            self.virtual_window.clear();
            self.real_window.clear();
        }
        let Some(last) = self.last.filter(|_| !changed) else {
            self.last = Some(Sample {
                wall: wall_now,
                virtual_elapsed,
                real_elapsed,
            });
            return;
        };
        self.last = Some(Sample {
            wall: wall_now,
            virtual_elapsed,
            real_elapsed,
        });
        let wall = wall_now.saturating_duration_since(last.wall);
        if wall.is_zero() {
            return;
        }
        self.wall += wall;
        self.frames += 1;
        let real_delta = real_elapsed.saturating_sub(last.real_elapsed);
        // `Time<Real>` takes the instant the render thread sent when the previous frame's render finished
        // (`vendor/bevy_render/src/lib.rs` `send_time`). A main-thread stall makes Real lag the wall clock for one
        // sample and the next frame catches up; a difference beyond the shortest window's tolerance is that stall, not
        // a rate. Leave the frame out of both windows. A manual strategy drives Real by its step, so its ratio stays
        // measurable (the strategy check is what fails it).
        if mode == ClockMode::Wall && real_delta.abs_diff(wall) > STALL {
            return;
        }
        let wall = wall.as_secs_f64();
        // `Time<Real>` follows the wall clock even when `Time<Virtual>` is clamped.
        self.real_window.push(wall, real_delta.as_secs_f64());
        // A virtual delta of `max_delta` was clamped (a hitch, or a slow manual clock): not a rate sample. The wall
        // interval goes with it, or the window would count a frame of game time against more than its own wall time.
        let virtual_delta = virtual_elapsed.saturating_sub(last.virtual_elapsed);
        if virtual_delta < max_delta {
            self.virtual_window.push(wall, virtual_delta.as_secs_f64());
        }
    }

    /// Judge the clock that is running now; `strategy`, `relative_speed` and `paused` are read live, not from the
    /// sample, so a clock installed after the last sample still fails on the next frame.
    fn assess(
        &self,
        strategy: &TimeUpdateStrategy,
        relative_speed: f64,
        paused: bool,
        tolerance: f64,
    ) -> Assessment {
        if self.wall < Duration::from_secs_f64(WARMUP) {
            return Assessment::Warmup;
        }
        if !matches!(strategy, TimeUpdateStrategy::Automatic) {
            return Assessment::Fail(format!(
                "TimeUpdateStrategy is {} (not Automatic), so game time is not wall time",
                strategy_name(strategy)
            ));
        }
        if (relative_speed - 1.0).abs() > 1e-9 {
            return Assessment::Fail(format!(
                "Time<Virtual> relative speed is {relative_speed}, so game time is not wall time"
            ));
        }
        if paused {
            return Assessment::Fail(
                "Time<Virtual> is paused, so game time is not wall time".into(),
            );
        }
        let real = self.real_ratio();
        if self.real_window.wall > 0.0 && real > 1.0 + tolerance {
            return Assessment::Fail(format!(
                "Time<Real> runs {real:.3}x the wall clock over the last {:.1} s (faster than wall)",
                self.real_window.wall
            ));
        }
        let game = self.ratio();
        if self.virtual_window.wall > 0.0 {
            if game > 1.0 + tolerance {
                return Assessment::Fail(format!(
                    "game time runs {game:.3}x wall time over the last {:.1} s (clock={})",
                    self.virtual_window.wall,
                    self.mode_name()
                ));
            }
            if game < 1.0 - tolerance {
                return Assessment::Slow(format!(
                    "game time runs {game:.3}x wall time over the last {:.1} s",
                    self.virtual_window.wall
                ));
            }
        }
        Assessment::Ok
    }
}

fn strategy_name(strategy: &TimeUpdateStrategy) -> &'static str {
    match strategy {
        TimeUpdateStrategy::Automatic => "Automatic",
        TimeUpdateStrategy::ManualInstant(_) => "ManualInstant",
        TimeUpdateStrategy::ManualDuration(_) => "ManualDuration",
        TimeUpdateStrategy::FixedTimesteps(_) => "FixedTimesteps",
    }
}

pub(crate) fn plugin(app: &mut App, require_wall_clock: bool) {
    app.init_resource::<WallClock>()
        .init_resource::<ClockAudit>()
        .add_systems(First, sample_clock.after(bevy::time::TimeSystems));
    if require_wall_clock {
        app.add_systems(Update, check_wall_clock);
        // After the window-close exits of `Last`, so a run that ends normally still gets its final ratio judged.
        app.add_systems(Last, judge_at_exit.after(bevy::app::OnAppExitSystems));
    }
}

fn sample_clock(
    mut audit: ResMut<ClockAudit>,
    virtual_time: Res<Time<Virtual>>,
    real_time: Res<Time<Real>>,
    strategy: Res<TimeUpdateStrategy>,
    wall: Res<WallClock>,
) {
    let mode = match &*strategy {
        TimeUpdateStrategy::Automatic => ClockMode::Wall,
        _ => ClockMode::Manual,
    };
    audit.sample(
        mode,
        wall.now(),
        virtual_time.elapsed(),
        real_time.elapsed(),
        virtual_time.max_delta(),
    );
}

/// `--require-wall-clock`: every frame after the warm-up, the app clock must be the wall clock.
fn check_wall_clock(
    audit: Res<ClockAudit>,
    strategy: Res<TimeUpdateStrategy>,
    virtual_time: Res<Time<Virtual>>,
    mut state: Local<CheckState>,
    mut exit: MessageWriter<AppExit>,
) {
    match audit.assess(
        &strategy,
        virtual_time.relative_speed_f64(),
        virtual_time.is_paused(),
        TOLERANCE,
    ) {
        Assessment::Warmup => {}
        Assessment::Ok => {
            if !state.ok {
                state.ok = true;
                info!(
                    "aa-dev: wall clock ok (game/wall {:.3}, real/wall {:.3})",
                    audit.ratio(),
                    audit.real_ratio()
                );
            }
        }
        Assessment::Slow(why) => {
            if !state.warned {
                state.warned = true;
                warn!("aa-dev: --require-wall-clock: {why}");
            }
        }
        Assessment::Fail(why) => {
            if !state.failed {
                state.failed = true;
                error!("aa-dev: --require-wall-clock: {why}");
                exit.write(AppExit::error());
            }
        }
    }
}

#[derive(Default)]
struct CheckState {
    ok: bool,
    warned: bool,
    failed: bool,
}

/// Judge the ratio once more when the run exits: a drift that built up after the last frame check still fails.
fn judge_at_exit(
    audit: Res<ClockAudit>,
    strategy: Res<TimeUpdateStrategy>,
    virtual_time: Res<Time<Virtual>>,
    mut exits: MessageMutator<AppExit>,
    mut judged: Local<bool>,
) {
    if *judged || exits.is_empty() {
        return;
    }
    *judged = true;
    match audit.assess(
        &strategy,
        virtual_time.relative_speed_f64(),
        virtual_time.is_paused(),
        TOLERANCE,
    ) {
        Assessment::Fail(why) => {
            error!("aa-dev: --require-wall-clock at exit: {why}");
            exits.write(AppExit::error());
        }
        Assessment::Slow(why) => warn!("aa-dev: --require-wall-clock at exit: {why}"),
        Assessment::Warmup => {
            warn!("aa-dev: --require-wall-clock: the run ended before {WARMUP} s of wall time")
        }
        Assessment::Ok => info!(
            "aa-dev: wall clock ok at exit (game/wall {:.3}, real/wall {:.3})",
            audit.ratio(),
            audit.real_ratio()
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use bevy::time::{TimeReceiver, TimeSender, create_time_channels};

    use super::*;

    /// An app whose wall clock and sent render time are stepped by hand, so no test sleeps or reads the system clock.
    struct Harness {
        app: App,
        wall: Arc<AtomicU64>,
        sent: TimeSender,
        sent_base: web_time::Instant,
        sent_elapsed: Duration,
        wall_elapsed: Duration,
    }

    impl Harness {
        fn new(per_frame_check: bool) -> Self {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            let (sent, receiver) = create_time_channels();
            app.insert_resource(TimeReceiver(receiver.0));
            let wall = Arc::new(AtomicU64::new(0));
            let wall_base = web_time::Instant::now();
            let clock = WallClock::new({
                let wall = wall.clone();
                move || wall_base + Duration::from_nanos(wall.load(Ordering::Relaxed))
            });
            app.insert_resource(clock)
                .init_resource::<ClockAudit>()
                .init_resource::<ExitRequest>()
                .add_systems(First, sample_clock.after(bevy::time::TimeSystems))
                .add_systems(Update, request_exit)
                .add_systems(Last, judge_at_exit.after(bevy::app::OnAppExitSystems));
            if per_frame_check {
                app.add_systems(Update, check_wall_clock);
            }
            Self {
                app,
                wall,
                sent,
                sent_base: web_time::Instant::now(),
                sent_elapsed: Duration::ZERO,
                wall_elapsed: Duration::ZERO,
            }
        }

        /// One frame: the wall clock and the render clock both advance.
        fn step(&mut self, wall: Duration, sent: Duration) {
            self.wall_elapsed += wall;
            self.wall
                .store(self.wall_elapsed.as_nanos() as u64, Ordering::Relaxed);
            self.sent_elapsed += sent;
            let _ = self.sent.0.try_send(self.sent_base + self.sent_elapsed);
            self.app.update();
        }

        /// Frames, stopping like the runner does at the end of the frame that requested the exit.
        fn run(&mut self, frames: usize, wall: Duration, sent: Duration) {
            for _ in 0..frames {
                self.step(wall, sent);
                if self.exit().is_some() {
                    break;
                }
            }
        }

        fn audit(&self) -> &ClockAudit {
            self.app.world().resource::<ClockAudit>()
        }

        fn exit(&self) -> Option<AppExit> {
            self.app.should_exit()
        }

        fn request_exit(&mut self) {
            self.app.world_mut().resource_mut::<ExitRequest>().0 = true;
        }
    }

    #[derive(Resource, Default)]
    struct ExitRequest(bool);

    fn request_exit(request: ResMut<ExitRequest>, mut exit: MessageWriter<AppExit>) {
        if request.0 {
            exit.write(AppExit::Success);
        }
    }

    #[test]
    fn a_fixed_step_installed_after_the_warmup_fails_on_the_next_frame() {
        let mut h = Harness::new(true);
        h.run(75, Duration::from_millis(16), Duration::from_millis(16));
        assert!(h.audit().wall_secs() > WARMUP);
        assert_eq!(h.exit(), None, "wall time is fine before the switch");
        h.app
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / 60.0,
            )));
        h.step(Duration::from_millis(16), Duration::from_millis(16));
        assert!(
            matches!(h.exit(), Some(AppExit::Error(_))),
            "a manual clock must fail at once"
        );
    }

    #[test]
    fn a_relative_speed_set_after_the_warmup_fails_on_the_next_frame() {
        let mut h = Harness::new(true);
        h.run(75, Duration::from_millis(16), Duration::from_millis(16));
        assert_eq!(h.exit(), None);
        h.app
            .world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_relative_speed(2.0);
        h.step(Duration::from_millis(16), Duration::from_millis(16));
        assert!(
            matches!(h.exit(), Some(AppExit::Error(_))),
            "a scaled clock must fail at once"
        );
    }

    #[test]
    fn the_original_1_60_step_at_an_uncapped_rate_fails() {
        // The 2026-10-26 combat side-by-side: `--headless-fps 120` with a 1/60 s fixed step (561ebd1b).
        let mut h = Harness::new(true);
        h.app
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / 60.0,
            )));
        h.run(
            150,
            Duration::from_secs_f64(1.0 / 120.0),
            Duration::from_secs_f64(1.0 / 120.0),
        );
        assert!(
            h.audit().ratio() > 1.5,
            "game time runs {:.3}x wall time",
            h.audit().ratio()
        );
        assert!(
            matches!(h.exit(), Some(AppExit::Error(_))),
            "a 1/60 s step under a 120 fps loop must fail"
        );
    }

    #[test]
    fn a_delayed_render_instant_is_not_a_rate_sample() {
        let mut h = Harness::new(true);
        h.run(75, Duration::from_millis(16), Duration::from_millis(16));
        assert_eq!(h.exit(), None);
        // A 150 ms main-thread stall: the wall clock advances, but `Time<Real>` still carries the render thread's
        // instant from before the stall, so its delta is one normal frame (the old symmetric check read 0.89x).
        h.step(Duration::from_millis(166), Duration::from_millis(16));
        assert_eq!(h.exit(), None, "a Real lag is not a slower clock");
        assert!(
            (h.audit().real_ratio() - 1.0).abs() <= TOLERANCE,
            "the stall frame is not a rate sample: real/wall {:.3}",
            h.audit().real_ratio()
        );
        // The render thread's next instant covers the stall: Real catches up with a large delta. That is the same
        // stall, not a fast clock, and must not fail either.
        h.step(Duration::from_millis(16), Duration::from_millis(166));
        assert_eq!(h.exit(), None, "the catch-up frame is not a fast clock");
        assert!(
            (h.audit().ratio() - 1.0).abs() <= TOLERANCE,
            "the catch-up frame is not a rate sample: game/wall {:.3}",
            h.audit().ratio()
        );
        h.run(75, Duration::from_millis(16), Duration::from_millis(16));
        assert_eq!(h.exit(), None);
    }

    #[test]
    fn a_hitch_that_clamps_game_time_does_not_fail_the_check() {
        let mut h = Harness::new(true);
        h.run(75, Duration::from_millis(16), Duration::from_millis(16));
        assert_eq!(h.exit(), None);
        // A 400 ms stall: `Time<Virtual>` advances only `max_delta` (250 ms) and must not be counted as a rate sample.
        h.step(Duration::from_millis(400), Duration::from_millis(400));
        h.run(75, Duration::from_millis(16), Duration::from_millis(16));
        assert_eq!(
            h.exit(),
            None,
            "a slow machine with a hitch must not fail: {h:?}",
            h = h.audit()
        );
        assert!(
            (h.audit().ratio() - 1.0).abs() <= TOLERANCE,
            "game/wall {:.3}",
            h.audit().ratio()
        );
        assert!(
            (h.audit().real_ratio() - 1.0).abs() <= TOLERANCE,
            "real/wall {:.3}",
            h.audit().real_ratio()
        );
    }

    /// An audit fed by hand: `wall`/`game`/`real` per frame, enough frames to pass the warm-up.
    fn pure_audit(wall: Duration, game: Duration, real: Duration) -> ClockAudit {
        let mut audit = ClockAudit::default();
        let base = web_time::Instant::now();
        let (mut at_wall, mut at_game, mut at_real) =
            (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        for _ in 0..120 {
            audit.sample(
                ClockMode::Wall,
                base + at_wall,
                at_game,
                at_real,
                Duration::from_millis(250),
            );
            at_wall += wall;
            at_game += game;
            at_real += real;
        }
        audit
    }

    #[test]
    fn a_slower_game_clock_is_reported_but_not_failed() {
        let audit = pure_audit(
            Duration::from_millis(10),
            Duration::from_millis(5),
            Duration::from_millis(10),
        );
        assert!(audit.wall_secs() > WARMUP);
        match audit.assess(&TimeUpdateStrategy::Automatic, 1.0, false, TOLERANCE) {
            Assessment::Slow(why) => assert!(why.contains("0.500"), "{why}"),
            other => panic!("a slow game clock is a warning, not {other:?}"),
        }
    }

    #[test]
    fn a_scaled_manual_or_paused_clock_fails_even_when_the_measured_ratio_is_fine() {
        let audit = pure_audit(
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(10),
        );
        assert!(audit.wall_secs() > WARMUP);
        assert!(matches!(
            audit.assess(&TimeUpdateStrategy::Automatic, 2.0, false, TOLERANCE),
            Assessment::Fail(_)
        ));
        assert!(matches!(
            audit.assess(
                &TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)),
                1.0,
                false,
                TOLERANCE
            ),
            Assessment::Fail(_)
        ));
        assert!(matches!(
            audit.assess(&TimeUpdateStrategy::Automatic, 1.0, true, TOLERANCE),
            Assessment::Fail(_)
        ));
    }

    #[test]
    fn a_real_clock_that_does_not_follow_the_wall_clock_fails() {
        // ManualDuration drives Time<Real> too. Keep the virtual ratio at 1 so only the Real check can see it.
        let audit = pure_audit(
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(15),
        );
        match audit.assess(&TimeUpdateStrategy::Automatic, 1.0, false, TOLERANCE) {
            Assessment::Fail(why) => assert!(why.contains("Time<Real>"), "{why}"),
            other => panic!("a non-wall Time<Real> must fail, got {other:?}"),
        }
    }

    #[test]
    fn a_slower_real_clock_is_not_a_failure() {
        // Only a Real clock that runs faster than wall can inflate a timing; a lag is a stall (see the harness test).
        let audit = pure_audit(
            Duration::from_millis(10),
            Duration::from_millis(10),
            Duration::from_millis(5),
        );
        assert!(matches!(
            audit.assess(&TimeUpdateStrategy::Automatic, 1.0, false, TOLERANCE),
            Assessment::Ok
        ));
    }

    #[test]
    fn a_manual_clock_fails_at_app_exit_without_a_per_frame_check() {
        let mut h = Harness::new(false);
        h.app
            .world_mut()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
                1.0 / 60.0,
            )));
        h.run(80, Duration::from_millis(16), Duration::from_millis(16));
        assert!(h.audit().wall_secs() > WARMUP);
        assert_eq!(h.exit(), None, "the per-frame check is off");
        h.request_exit();
        h.step(Duration::from_millis(16), Duration::from_millis(16));
        assert!(
            matches!(h.exit(), Some(AppExit::Error(_))),
            "the final ratio is judged at AppExit"
        );
    }

    #[test]
    fn switching_to_a_fixed_step_starts_a_new_window_without_restarting_the_warmup() {
        let mut audit = ClockAudit::default();
        let start = web_time::Instant::now();
        let max = Duration::MAX;
        audit.sample(ClockMode::Wall, start, Duration::ZERO, Duration::ZERO, max);
        audit.sample(
            ClockMode::Wall,
            start + Duration::from_secs(2),
            Duration::from_secs(2),
            Duration::from_secs(2),
            max,
        );
        assert!((audit.ratio() - 1.0).abs() < 1e-9);
        assert!((audit.wall_secs() - 2.0).abs() < 1e-9);
        // `--record` starts here: the loading before it must not dilute the fixed step, but the warm-up stays 2 s.
        audit.sample(
            ClockMode::Manual,
            start + Duration::from_secs(2),
            Duration::from_secs(20),
            Duration::from_secs(20),
            max,
        );
        assert_eq!(audit.mode(), Some(ClockMode::Manual));
        assert!((audit.wall_secs() - 2.0).abs() < 1e-9);
        audit.sample(
            ClockMode::Manual,
            start + Duration::from_secs(3),
            Duration::from_secs(30),
            Duration::from_secs(30),
            max,
        );
        assert!((audit.ratio() - 10.0).abs() < 1e-9, "{}", audit.ratio());
    }
}
