//! `FxChr` effect items: a skinned `.chr` model that plays its own `start` / `loop` / `end` animations (Summon Crows' crows, the pet
//! net, Golem's hand). The model and clips are loaded through aa-anim / aa-cry; the effect is one entity with a [`ChrEffect`].
//!
//! ```ignore
//! commands.spawn((ChrEffect::new("game/objects/effects/crow/crow.chr"), Transform::from_translation(pos)));
//! ```
//!
//! The `start` animation plays once, then `loop` until the effect ends (`ending`, or `life` seconds), then `end` plays once and the
//! entity is despawned. A model without `loop` holds the last frame of `start`; one without `end` is despawned at once.

use std::sync::Arc;

use std::collections::HashMap;

use aa_anim::animator::{Animator, spawn_bones};
use aa_anim::pose::{BoundClip, Rig, SampleOptions, shared_rig};
use aa_anim::{Cal, Clip, Track};
use aa_cry::CRY_TO_BEVY;
use aa_cry::chunk::{ChunkFile, ty};
use aa_cry::scene::{CryAssets, ModelOptions, SkeletonBinding, spawn_loaded};
use bevy::prelude::*;

/// A character-model effect (`fx_items` of the type `FxChr`). Spawn it with a `Transform` (parent it for a bone-bound effect).
#[derive(Component, Clone, Debug)]
#[require(Transform, Visibility)]
pub struct ChrEffect {
    /// The `.chr` in the pak (`game/objects/effects/crow/crow.chr`); its `.cal` next to it names the animations.
    pub path: String,
    /// Uniform scale of the model.
    pub scale: f32,
    /// Seconds until the effect ends by itself (`None`: it runs until `ending` is set).
    pub life: Option<f32>,
    /// Set to play the `end` animation and despawn.
    pub ending: bool,
}

impl ChrEffect {
    pub fn new(path: impl Into<String>) -> ChrEffect {
        ChrEffect { path: path.into(), scale: 1.0, life: None, ending: false }
    }
}

/// Decoded clips of character effects, by pak path.
#[derive(Resource, Default)]
struct ChrClips(HashMap<String, Option<Arc<Clip>>>);

impl ChrClips {
    fn bound(&mut self, rig: &Rig, path: &str, options: SampleOptions) -> Option<BoundClip> {
        let clip = self.0.entry(path.to_string()).or_insert_with(|| load_clip(path).map(Arc::new)).clone()?;
        Some(BoundClip::new(rig, clip, options))
    }
}

/// [`Clip::load`] plus the v0x827 controllers that aa-anim skips (see [`decode_827`]): the clips of Summon Crows, the pet net and
/// Golem's hand use them.
fn load_clip(path: &str) -> Option<Clip> {
    let bytes = aa_cry::read(path).ok()?;
    let mut clip = Clip::parse(&bytes).ok()?;
    if clip.skipped_controllers > 0 {
        let file = ChunkFile::parse(&bytes).ok()?;
        let (ticks_per_frame, start_frame) = file.first(ty::TIMING).and_then(|t| {
            let mut r = t.reader();
            r.f32().ok()?;
            let ticks = r.i32().ok()?;
            r.fixed_str(32).ok()?;
            Some((ticks as f32, r.i32().ok()? as f32))
        })?;
        for c in file.of_type(ty::CONTROLLER).filter(|c| c.version == 0x827) {
            if let Some(track) = decode_827(c.data, ticks_per_frame, start_frame, clip.fps) {
                clip.tracks.push(track);
                clip.skipped_controllers -= 1;
            }
        }
        clip.tracks.sort_by_key(|t| t.controller);
    }
    Some(clip)
}

/// Controller chunk v0x827: `u32 num_keys, u32 controller_id`, then per key (`CryKeyPQLog`) `i32 time (ticks), f32 x3 position (centimetres, absolute like the rotation),
/// f32 x3 rotation log` (the rotation is `exp(v)`: the axis `v / |v|` with the half angle `|v|`).
fn decode_827(data: &[u8], ticks_per_frame: f32, start_frame: f32, fps: f32) -> Option<Track> {
    let mut r = aa_cry::chunk::Reader::new(data);
    let keys = r.u32().ok()? as usize;
    let mut track = Track { controller: r.u32().ok()?, ..Default::default() };
    for _ in 0..keys {
        let time = (r.i32().ok()? as f32 / ticks_per_frame - start_frame) / fps;
        let pos = r.vec3().ok()? * 0.01;
        let log = r.vec3().ok()?;
        let angle = log.length();
        let rot = if angle > 1e-9 { Quat::from_axis_angle(log / angle, angle * 2.0) } else { Quat::IDENTITY };
        track.rot_times.push(time);
        track.rot.push(rot);
        track.pos_times.push(time);
        track.pos.push(pos);
    }
    Some(track)
}

/// The state of a built [`ChrEffect`].
#[derive(Component)]
struct ChrRun {
    age: f32,
    ended: bool,
    end: Option<BoundClip>,
}

/// Builds the model and animator of new [`ChrEffect`]s.
fn build(mut commands: Commands, mut cry: CryAssets, mut clips: ResMut<ChrClips>, new: Query<(Entity, &ChrEffect), Without<ChrRun>>) {
    for (entity, effect) in &new {
        let path = effect.path.trim().replace(char::from(92), "/").to_ascii_lowercase();
        let Some(run) = build_one(&mut commands, &mut cry, &mut clips, entity, effect, &path) else {
            warn!("aa-fx: character effect {path} could not be built");
            commands.entity(entity).remove::<ChrEffect>();
            continue;
        };
        commands.entity(entity).insert(run);
    }
}

fn build_one(commands: &mut Commands, cry: &mut CryAssets, clips: &mut ChrClips, entity: Entity, effect: &ChrEffect, path: &str) -> Option<ChrRun> {
    let rig = shared_rig(path).ok().filter(|r| !r.is_empty())?;
    let model_root = commands
        .spawn((Name::new("model"), Transform::from_rotation(CRY_TO_BEVY).with_scale(Vec3::splat(effect.scale)), Visibility::default(), ChildOf(entity)))
        .id();
    let bones = spawn_bones(commands, model_root, &rig);
    let options = ModelOptions { skeleton: Some(SkeletonBinding { skeleton: rig.skeleton.clone(), joints: bones.clone() }), cry_space: true, ..default() };
    let loaded = cry.load_model(path, &options).ok()?;
    let part = spawn_loaded(commands, path, &loaded, &options);
    commands.entity(part).insert(ChildOf(model_root));

    let cal = Cal::shared(&format!("{}.cal", path.trim_end_matches(".chr"))).ok();
    let mut bind = |name: &str| -> Option<BoundClip> {
        let cal: &Arc<Cal> = cal.as_ref()?;
        let entry = cal.animation(name)?;
        let options = SampleOptions { position_scale: entry.position_scale(cal.base_height), lock_root_motion: false };
        clips.bound(&rig, &entry.path, options)
    };
    let mut animator = Animator::new(rig.clone(), bones);
    match (bind("start"), bind("loop")) {
        (Some(start), Some(looping)) => animator.play_once_then(start, looping, 0.0),
        (Some(start), None) => animator.play(start, false, 0.0),
        (None, Some(looping)) => animator.play(looping, true, 0.0),
        (None, None) => {}
    }
    commands.entity(entity).insert(animator);
    Some(ChrRun { age: 0.0, ended: false, end: bind("end") })
}

/// Ends effects (`life`, `ending`) and despawns them when the `end` animation is over.
fn drive(mut commands: Commands, time: Res<Time>, mut effects: Query<(Entity, &ChrEffect, &mut ChrRun, &mut Animator)>) {
    for (entity, effect, mut run, mut animator) in &mut effects {
        run.age += time.delta_secs();
        if !run.ended && (effect.ending || effect.life.is_some_and(|l| run.age >= l)) {
            run.ended = true;
            match run.end.take() {
                Some(end) => animator.play(end, false, 0.1),
                None => {
                    commands.entity(entity).despawn();
                    continue;
                }
            }
        }
        if run.ended && animator.current.as_ref().is_none_or(|p| p.finished()) {
            commands.entity(entity).despawn();
        }
    }
}

pub(crate) fn build_plugin(app: &mut App) {
    app.init_resource::<aa_cry::scene::CryCache>().init_resource::<ChrClips>();
    if !app.is_plugin_added::<aa_anim::AnimPlugin>() {
        app.add_plugins(aa_anim::AnimPlugin);
    }
    app.add_systems(Update, (build, drive).chain());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v827_keys_are_centimetres_and_log_rotations() {
        let mut data = Vec::new();
        data.extend(2u32.to_le_bytes());
        data.extend(0xABCDu32.to_le_bytes());
        for (ticks, pos, log) in [(160i32 * 10, [100.0f32, 0.0, 50.0], [0.0f32, 0.0, std::f32::consts::FRAC_PI_4]), (160 * 40, [0.0, 0.0, 0.0], [0.0, 0.0, 0.0])] {
            data.extend(ticks.to_le_bytes());
            data.extend(pos.iter().chain(&log).flat_map(|v| v.to_le_bytes()));
        }
        let t = decode_827(&data, 160.0, 10.0, 30.0).unwrap();
        assert_eq!(t.controller, 0xABCD);
        assert_eq!(t.rot_times, vec![0.0, 1.0]);
        assert!((t.pos[0] - Vec3::new(1.0, 0.0, 0.5)).length() < 1e-6);
        // half angle pi/4: a quarter turn about Z
        assert!(t.rot[0].angle_between(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)) < 1e-3);
        assert_eq!(t.rot[1], Quat::IDENTITY);
        assert!(decode_827(&data[..20], 160.0, 10.0, 30.0).is_none());
    }

    #[test]
    fn the_crow_clips_decode() {
        if !aa_cry::pak_available() {
            return;
        }
        for name in ["start", "loop", "end"] {
            let clip = load_clip(&format!("game/objects/effects/crow/crow_{name}.caf")).unwrap();
            assert_eq!(clip.skipped_controllers, 0, "{name}");
            assert!(clip.tracks.len() >= 53 && clip.duration > 0.1, "{name}: {} tracks {} s", clip.tracks.len(), clip.duration);
        }
    }
}
