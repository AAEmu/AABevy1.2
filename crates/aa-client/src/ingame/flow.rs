//! The way into the game and back, timed, and a scripted round trip for unattended runs.
//!
//! `flow:` lines in the log (normal login flow; `--enter-world` has no loading screen):
//!
//! * `loading started`: Start was pressed (`OnEnter(Screen::Loading)`).
//! * `loading took S s`: how long the loading screen was up, with its worst frame.
//! * `first in-game frame S s after Start`: from pressing Start to the first frame shown in game.
//! * `in game: worst frame ... of the first 5 s`: the hitches the player sees after the loading screen.
//!
//! Flags (see also `dev`): `--leave-world S` asks the server to go back to the character list S seconds after the
//! game screen came up (the HUD's leave flow does the rest on `LeftWorld`), `--again` presses Start on the character
//! list when it is back, so a run goes round and round (`--exit-after` ends it).

use aa_net::{Command, LeaveTarget, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;

use super::dev::DevOptions;
use crate::Screen;
use crate::login::UiBuilt;

/// How long the first frames in game are watched.
const WATCH: f32 = 5.0;

#[derive(Resource, Default)]
pub struct Flow {
    clicked: Option<std::time::Instant>,
    /// Real seconds since Start was pressed when the game screen came up.
    loading_secs: f32,
    loading_worst: f32,
    /// Seconds on the game screen and frames since it came up.
    in_game: f32,
    frames: u32,
    worst: (f32, u32),
    over_33: u32,
    over_100: u32,
    reported: bool,
    /// Left the game screen and not pressed Start again yet (`--again`).
    left: bool,
    leave_sent: bool,
    list_secs: f32,
}

pub fn plugin(app: &mut App) {
    app.init_resource::<Flow>()
        .add_systems(OnEnter(Screen::Loading), loading_started)
        .add_systems(OnEnter(Screen::InGame), game_started)
        .add_systems(OnExit(Screen::InGame), game_left)
        .add_systems(
            Update,
            (
                watch.run_if(super::world_screens),
                leave_world.run_if(in_state(Screen::InGame)),
                press_start_again.run_if(in_state(Screen::CharacterSelect)),
            ),
        );
}

fn loading_started(mut flow: ResMut<Flow>) {
    *flow = Flow { clicked: Some(std::time::Instant::now()), left: flow.left, ..default() };
    info!("flow: loading started");
}

fn game_started(mut flow: ResMut<Flow>) {
    flow.loading_secs = flow.clicked.map_or(0.0, |c| c.elapsed().as_secs_f32());
    if flow.clicked.is_some() {
        info!("flow: loading took {:.2} s (worst frame {:.0} ms)", flow.loading_secs, flow.loading_worst);
    }
    flow.in_game = 0.0;
    flow.frames = 0;
    flow.worst = (0.0, 0);
    flow.over_33 = 0;
    flow.over_100 = 0;
    flow.reported = false;
    flow.leave_sent = false;
}

fn game_left(mut flow: ResMut<Flow>) {
    flow.left = true;
    flow.list_secs = 0.0;
}

/// Frame times on the way in: the delta the frame sees is the length of the one before it, so index 1 is the frame the
/// switch to the game screen happened in (the spawn and the HUD build land there), index 2 shows its picture.
fn watch(time: Res<Time<Real>>, screen: Res<State<Screen>>, mut flow: ResMut<Flow>) {
    let ms = time.delta_secs() * 1000.0;
    if *screen.get() == Screen::Loading {
        flow.loading_worst = flow.loading_worst.max(ms);
        return;
    }
    let idx = flow.frames;
    flow.frames += 1;
    flow.in_game += time.delta_secs();
    if idx == 2
        && let Some(c) = flow.clicked
    {
        info!("flow: first in-game frame {:.2} s after Start (loading {:.2} s)", c.elapsed().as_secs_f32(), flow.loading_secs);
    }
    if idx >= 1 && flow.in_game <= WATCH {
        if ms > 33.4 {
            flow.over_33 += 1;
        }
        if ms > 100.0 {
            flow.over_100 += 1;
        }
        if ms > flow.worst.0 {
            flow.worst = (ms, idx);
        }
        if ms > 50.0 {
            info!("flow: frame {idx} in game took {ms:.0} ms");
        }
    }
    if !flow.reported && flow.in_game > WATCH {
        flow.reported = true;
        info!(
            "flow: in game: worst frame {:.0} ms (frame {}) of the first {WATCH:.0} s, {} frames over 33 ms, {} over 100 ms, {} frames",
            flow.worst.0, flow.worst.1, flow.over_33, flow.over_100, flow.frames
        );
    }
}

/// `--leave-world S`: ask the server to go back to the character list.
fn leave_world(dev: Res<DevOptions>, mut flow: ResMut<Flow>, mut out: MessageWriter<NetCommand>) {
    let Some(after) = dev.leave_world else { return };
    if !flow.leave_sent && flow.in_game >= after {
        flow.leave_sent = true;
        info!("flow: leaving the world (character list)");
        out.write(NetCommand(Command::LeaveWorld(LeaveTarget::CharacterSelect)));
    }
}

/// `--again`: press Start (the original script's button) when the character list is up after leaving the game.
fn press_start_again(
    time: Res<Time>,
    dev: Res<DevOptions>,
    built: Res<UiBuilt>,
    rt: Option<NonSend<UiRuntime>>,
    mut flow: ResMut<Flow>,
) {
    if !dev.again || !flow.left || built.0 != Some(Screen::CharacterSelect) {
        return;
    }
    let Some(rt) = rt else { return };
    flow.list_secs += time.delta_secs();
    if flow.list_secs < 2.5 {
        return;
    }
    flow.left = false;
    info!("flow: character list is back, pressing Start again");
    if let Err(e) = rt.exec("again", "characterSelectWindow.bottomPanel.startButton:OnClick('LeftButton')") {
        warn!("flow: --again: {e}");
    }
}
