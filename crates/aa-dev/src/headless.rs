//! `--headless` / `--offscreen-shot`: see the crate docs.
//!
//! Without the window plugin nothing draws into the primary window, so `--offscreen-shot` gives every camera that
//! targets it an image of the window's size to draw into instead, and saves that image (the same pixels a window
//! screenshot would have). Systems that read the primary `Window` component (the UI's size) keep working.

use bevy::camera::{RenderTarget, ImageRenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PrimaryWindow, WindowRef};

use crate::DevArgs;

/// The image the window's cameras draw into with `--offscreen-shot`.
#[derive(Resource, Clone)]
pub struct OffscreenTarget(pub Handle<Image>);

#[derive(Resource, Default)]
struct ShotState {
    requested: bool,
    wait: f32,
}

pub fn plugin(app: &mut App) {
    let args = app.world().resource::<DevArgs>().clone();
    if args.offscreen_shot.is_none() {
        return;
    }
    let mut image = Image::new_target_texture(args.size.0, args.size.1, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_SRC;
    let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
    app.insert_resource(OffscreenTarget(handle)).init_resource::<ShotState>().add_systems(Last, (retarget_cameras, offscreen_shot).chain());
}

/// A camera that would draw into the (not existing) primary window draws into the offscreen image.
fn retarget_cameras(target: Res<OffscreenTarget>, primary: Query<Entity, With<PrimaryWindow>>, mut cameras: Query<&mut RenderTarget>) {
    let primary = primary.single().ok();
    for mut t in &mut cameras {
        let to_window = matches!(&*t, RenderTarget::Window(WindowRef::Primary)) || matches!(&*t, RenderTarget::Window(WindowRef::Entity(e)) if Some(*e) == primary);
        if to_window {
            *t = RenderTarget::Image(ImageRenderTarget::from(target.0.clone()));
        }
    }
}

/// Saves the offscreen image `--shot-after` seconds in; without a script the run ends right after (like `--shot`).
fn offscreen_shot(mut commands: Commands, time: Res<Time>, args: Res<DevArgs>, target: Res<OffscreenTarget>, mut state: ResMut<ShotState>, mut exit: MessageWriter<AppExit>) {
    let Some(path) = args.offscreen_shot.clone() else { return };
    if !state.requested && time.elapsed_secs() >= args.shot_after {
        take(&mut commands, &target, &path);
        state.requested = true;
    } else if state.requested && !SCRIPTED.load(std::sync::atomic::Ordering::Relaxed) {
        state.wait += time.delta_secs();
        if state.wait > 1.5 && std::path::Path::new(&path).exists() {
            exit.write(AppExit::Success);
        }
    }
}

/// Set by a script runner: `--offscreen-shot` then leaves the exit to the script.
pub static SCRIPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn take(commands: &mut Commands, target: &OffscreenTarget, path: &str) {
    if let Some(dir) = std::path::Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    commands.spawn(Screenshot::image(target.0.clone())).observe(save_to_disk(path.to_string()));
    info!("aa-dev: offscreen screenshot -> {path}");
}

/// A script's `shot <file>`: save the offscreen image now. `false` when this run has no offscreen target.
pub fn take_offscreen_shot(commands: &mut Commands, target: Option<&OffscreenTarget>, path: &str) -> bool {
    match target {
        Some(t) => {
            take(commands, t, path);
            true
        }
        None => false,
    }
}
