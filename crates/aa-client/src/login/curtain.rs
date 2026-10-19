//! The last stage of the loading screen: it stays on the window while the game screen does its own first-use work.
//!
//! The world is built behind the loading screen (`loading`), but some of the game screen exists only in game: the HUD stage
//! is swapped in on `Screen::InGame` and drawn for the first time (its textures, the HDR variants of the UI pipelines), and the
//! name plates, the quest marks over the NPCs and their light beams (`hud::plates`, `hud::marks`, aa-fx) are spawned by the
//! HUD's systems. Measured (`AA_ENTER_TRACE=1`, `ingame::trace`): 25 images and 9 to 11 pipelines in the first frames of the
//! game, the HUD missing on the first frame, the quest beams showing 0.25 to 1 s later, a 75 to 100 ms first frame.
//!
//! So when the loading gauge is full the window is captured (a screenshot of the loading screen at 100 %), the capture is
//! drawn over everything by the UI camera (the curtain, a sprite at the top of the UI's depth range), and then the screen
//! switches to the game underneath it. The curtain is lifted when the game screen is complete: the HUD is up, the world
//! camera is on the window, the plates and marks had their first looks ([`MIN_FRAMES`]), and no pipeline, image or unit has
//! been waiting or new for [`QUIET`] seconds; [`MAX_SECS`] is the safety net. To the player it is the loading screen that
//! stays a moment longer at 100 %, like the original.
//!
//! The curtain's own pipelines must be ready on both sides of the switch (the UI camera is plain on the loading screen and HDR
//! in game): a transparent sprite on the UI layer and a small HDR UI camera that draws into an image ([`Warm`]) compile both
//! variants while the loading screen is up, together with the HDR variants of the UI's materials.

use aa_ui::UiBackgroundCamera;
use aa_ui::render::UiCamera;
use aa_world::pipelines::PipelineStatus;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CompositingSpace, Hdr, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::Msaa;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::Screen;

/// Frames and seconds the game screen runs under the curtain at least (the quest marks look every 30 frames).
const MIN_FRAMES: u32 = 40;
const MIN_SECS: f32 = 0.4;
/// Seconds without a new or waiting pipeline, a new image or an outstanding unit before the curtain is lifted.
const QUIET: f32 = 0.3;
/// The curtain goes after this many seconds whatever is still going on.
const MAX_SECS: f32 = 6.0;
/// Seconds to wait for the capture of the window before the game starts without a curtain.
const CAPTURE_TIMEOUT: f32 = 1.0;
/// Depth of the curtain on the UI camera (the UI's batches are at 0 and up by 0.01, the dragged icon at 100).
const Z: f32 = 500.0;

#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Phase {
    #[default]
    Off,
    /// The capture of the window is on its way.
    Capturing,
    /// Drawn over the window; the game screen comes up underneath.
    Up,
}

#[derive(Resource, Default)]
pub struct Curtain {
    pub phase: Phase,
    /// Seconds in the current phase, frames under the curtain.
    secs: f32,
    frames: u32,
    sprite: Option<Entity>,
    /// Seconds nothing new has come up, and what was looked at.
    quiet: f32,
    pipelines: usize,
    worst: f32,
    pipelines_at_up: usize,
}

impl Curtain {
    /// The curtain covers the window (the game screen is not seen yet).
    pub fn is_up(&self) -> bool {
        self.phase != Phase::Off
    }

    /// Ready to switch to the game screen: drawn, or the capture did not come.
    pub fn ready(&self) -> bool {
        (self.phase == Phase::Up && self.frames >= 2) || (self.phase == Phase::Capturing && self.secs > CAPTURE_TIMEOUT)
    }
}

/// The small HDR UI camera and the transparent sprite that compile the curtain's pipelines on the loading screen.
#[derive(Component)]
struct Warm;

#[derive(Component)]
struct CurtainSprite;

pub fn plugin(app: &mut App) {
    app.init_resource::<Curtain>()
        .add_systems(OnEnter(Screen::Loading), (reset, spawn_warm))
        .add_systems(Update, follow_ui_camera.run_if(any_with_component::<Warm>))
        .add_systems(Update, (count, lift).chain().run_if(in_state(Screen::InGame)))
        .add_systems(OnExit(Screen::InGame), drop_all)
        .add_systems(OnEnter(Screen::CharacterSelect), drop_all)
        .add_systems(OnEnter(Screen::WorldSelect), drop_all);
}

fn reset(mut curtain: ResMut<Curtain>) {
    *curtain = Curtain::default();
}

fn spawn_warm(mut commands: Commands, mut images: ResMut<Assets<Image>>, existing: Query<(), With<Warm>>) {
    if !existing.is_empty() {
        return;
    }
    let target = Image::new_target_texture(64, 36, TextureFormat::Rgba8UnormSrgb, None);
    // what the UI cameras are in game (`hud::cameras::match_world_camera`)
    commands.spawn((
        Name::new("curtain warm camera"),
        Warm,
        Camera2d,
        Camera { order: -20, clear_color: ClearColorConfig::None, ..default() },
        RenderTarget::Image(images.add(target).into()),
        Hdr,
        Msaa::Off,
        CompositingSpace::Linear,
    ));
    // drawn by the loading screen's UI camera (plain) and by the one above (HDR); invisible
    commands.spawn((
        Name::new("curtain warm sprite"),
        Warm,
        Sprite { color: Color::NONE, custom_size: Some(Vec2::splat(4.0)), ..default() },
        Transform::from_xyz(8.0, -8.0, Z),
        RenderLayers::layer(0),
    ));
}

/// The warm camera looks where the UI camera looks (so it draws what the UI draws, in HDR).
#[allow(clippy::type_complexity)]
fn follow_ui_camera(
    ui: Query<(&Transform, &Projection), (With<UiCamera>, Without<UiBackgroundCamera>, Without<Warm>)>,
    mut warm: Query<(&mut Transform, &mut Projection), (With<Warm>, With<Camera>)>,
) {
    let Some((t, p)) = ui.iter().next() else { return };
    for (mut wt, mut wp) in &mut warm {
        if *wt != *t {
            *wt = *t;
        }
        if let (Projection::Orthographic(a), Projection::Orthographic(b)) = (p, &mut *wp) {
            b.scaling_mode = a.scaling_mode;
            b.scale = a.scale;
        }
    }
}

/// Captures the window and puts the capture over it (`loading` calls this when the gauge is full).
pub fn lower(commands: &mut Commands, curtain: &mut Curtain) {
    if curtain.phase != Phase::Off {
        return;
    }
    curtain.phase = Phase::Capturing;
    curtain.secs = 0.0;
    commands.spawn(Screenshot::primary_window()).observe(captured);
}

fn captured(
    shot: On<ScreenshotCaptured>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut curtain: ResMut<Curtain>,
    screen: Res<State<Screen>>,
) {
    if curtain.phase != Phase::Capturing || !matches!(screen.get(), Screen::Loading | Screen::InGame) {
        return;
    }
    let mut image = shot.image.clone();
    // the swap chain holds display (sRGB) values whatever its texture says
    image.texture_descriptor.format = image.texture_descriptor.format.add_srgb_suffix();
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    let size = Vec2::new(image.width() as f32, image.height() as f32);
    let sprite = commands
        .spawn((
            Name::new("loading curtain"),
            CurtainSprite,
            Sprite { image: images.add(image), custom_size: Some(size), ..default() },
            // the UI camera's frame: physical pixels, y down from the top left corner
            Transform::from_xyz(size.x / 2.0, -size.y / 2.0, Z),
            RenderLayers::layer(0),
        ))
        .id();
    curtain.sprite = Some(sprite);
    curtain.phase = Phase::Up;
    curtain.secs = 0.0;
    curtain.frames = 0;
    debug!("curtain: the loading screen captured ({}x{})", size.x, size.y);
}

/// Seconds and frames of the phase (the loading screen's `progress` reads them).
pub fn tick(curtain: &mut Curtain, dt: f32) {
    if curtain.phase != Phase::Off {
        curtain.secs += dt;
        curtain.frames += 1;
    }
}

/// Under the curtain: what is still coming up.
fn count(
    mut curtain: ResMut<Curtain>,
    time: Res<Time<Real>>,
    pipelines: Res<PipelineStatus>,
    warm: Res<crate::ingame::warm::Warm>,
    mut image_events: MessageReader<AssetEvent<Image>>,
) {
    let added = image_events.read().any(|e| matches!(e, AssetEvent::Added { .. }));
    if curtain.phase != Phase::Up {
        return;
    }
    let dt = time.delta_secs();
    tick(&mut curtain, dt);
    curtain.worst = curtain.worst.max(dt * 1000.0);
    if curtain.pipelines_at_up == 0 {
        curtain.pipelines_at_up = pipelines.total();
    }
    let total = pipelines.total();
    let busy = added || total != curtain.pipelines || pipelines.waiting() > 0 || warm.outstanding > 0;
    curtain.pipelines = total;
    curtain.quiet = if busy { 0.0 } else { curtain.quiet + dt };
}

/// Lifts the curtain once the game screen is complete.
#[allow(clippy::too_many_arguments)]
fn lift(
    mut commands: Commands,
    mut curtain: ResMut<Curtain>,
    hud: Res<crate::hud::HudBuilt>,
    options: Res<crate::hud::HudOptions>,
    behind: Query<(), With<crate::ingame::camera::BehindLoading>>,
    warm: Query<Entity, With<Warm>>,
    pipelines: Res<PipelineStatus>,
) {
    if curtain.phase != Phase::Up {
        if curtain.phase == Phase::Capturing {
            // a capture on its way (a teleport lowers the curtain in game, `ingame::teleport`): wait for it
            if curtain.secs <= CAPTURE_TIMEOUT {
                return;
            }
            // the capture never came: nothing to lift
            curtain.phase = Phase::Off;
        }
        for e in &warm {
            commands.entity(e).despawn();
        }
        return;
    }
    let hud_up = hud.up || !options.enabled;
    let complete = hud_up && behind.is_empty() && curtain.frames >= MIN_FRAMES && curtain.secs >= MIN_SECS && curtain.quiet >= QUIET;
    if !complete && curtain.secs < MAX_SECS {
        return;
    }
    if complete {
        info!(
            "curtain: lifted after {:.2} s ({} frames, worst {:.0} ms, {} pipelines compiled under it)",
            curtain.secs,
            curtain.frames,
            curtain.worst,
            pipelines.total().saturating_sub(curtain.pipelines_at_up)
        );
    } else {
        warn!(
            "curtain: lifted after {MAX_SECS} s with the game screen still busy (HUD up {hud_up}, world on the window {}, {} pipelines waiting, quiet {:.2} s)",
            behind.is_empty(),
            pipelines.waiting(),
            curtain.quiet
        );
    }
    if let Some(e) = curtain.sprite.take() {
        commands.entity(e).despawn();
    }
    for e in &warm {
        commands.entity(e).despawn();
    }
    curtain.phase = Phase::Off;
}

#[allow(clippy::type_complexity)]
fn drop_all(mut commands: Commands, mut curtain: ResMut<Curtain>, doomed: Query<Entity, Or<(With<Warm>, With<CurtainSprite>)>>) {
    for e in &doomed {
        commands.entity(e).despawn();
    }
    *curtain = Curtain::default();
}
