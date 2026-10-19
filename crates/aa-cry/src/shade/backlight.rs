//! The character back lights (`Char BackLight` of the time of day), applied to characters only.
//!
//! What the engine does (`cry3dengine.dll` `TimeOfDayManager::UpdateEngineBackLightParameters`, `cryanimation.dll`
//! `CharacterDefinition`, the `illum` / `humanskin` / `hair` / `cloth` / `metal` shaders in the pak; `docs/aa-cry.md`):
//!
//! * They are not scene lights but a shader term, `X2BackLight` under `%_RT_OBJECT_BACKLIGHT`: two unshadowed
//!   directional lights, `cDiffuseAcc += colour * saturate(N.L)` and a Phong highlight, added to the light that is
//!   multiplied by the diffuse map. Only actor models carry the flag.
//! * Colour `i` (1, 2) is the time of day's `"i Color" x "i Diffuse Multiplier"` (engine units, like the sun), the
//!   highlight is scaled by `"i Specular Multiplier"`.
//! * The direction is a property of the character: every player `.cdf` has
//!   `<BackLight><Light Index="1" Position="0.45,-1.1,0.25"/><Light Index="2" Position="-0.35,0.35,0.1"/></BackLight>`
//!   in the character's own space (+Y forward, +X to its right, Z up), so light 1 shines from behind the character, a
//!   little from the right and above, and light 2 from its front left. It turns with the character, not with the
//!   camera. That is why a character seen from behind in the shade is still lit.
//!
//! Here a [`CharMaterial`] (`StandardMaterial` extended by `backlight.wesl`) adds the term after the PBR lighting.
//! [`CharLightPlugin`] swaps the material of every entity that got a `Shading::Full` material (they carry [`TAG`])
//! for one of those while the resource [`CharBackLight`] is active (any light has a colour); the world crate sets the
//! resource from its time of day. Without the plugin or while the lights are black nothing changes.

use std::collections::HashMap;

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// `StandardMaterial::lightmap_exposure` of the materials that belong to characters (`Shading::Full`): the field is
/// only read when a mesh has a lightmap, which nothing here has, and its default is exactly 1.
pub const TAG: f32 = 1.0 + 1.0 / 4096.0;

/// The tag of hair and eye materials: back lights, but none of the world look (see [`world_look`]).
pub const NEUTRAL_TAG: f32 = 1.0 + 2.0 / 4096.0;

/// What a tag says about a material.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Costume,
    /// A skin whose `Diffuse` has this luma.
    Skin(f32),
    Neutral,
}

/// The tag of a skin material, whose `Diffuse` has the luma `diffuse`: `2 + diffuse`. The skin's base colour carries the
/// lit skin colour (`BSkinColor^1.5 * 1.5` x a gain, see `skin_base_color`) that the engine puts into the sun and
/// ambient light instead, while the back lights reach the skin as plain `colour * N.L * texture * Diffuse`; the term
/// therefore uses `texture * diffuse` as the albedo of skins.
pub fn skin_tag(diffuse: f32) -> f32 {
    2.0 + diffuse.clamp(0.0, 0.99)
}

fn decode_tag(exposure: f32) -> Option<Kind> {
    if exposure == TAG {
        Some(Kind::Costume)
    } else if exposure == NEUTRAL_TAG {
        Some(Kind::Neutral)
    } else if (2.0..3.0).contains(&exposure) {
        Some(Kind::Skin(exposure - 2.0))
    } else {
        None
    }
}

/// A `StandardMaterial` of a character, with the back lights.
pub type CharMaterial = ExtendedMaterial<StandardMaterial, CharBackExt>;

/// The `Position`s of the two `<Light>`s in the `.cdf` of a nuian, hariharan or male elf / ferre body. The female elf, the
/// female ferre and a few more differ by a few hundredths.
pub const CDF_POSITIONS: [Vec3; 2] = [Vec3::new(0.45, -1.1, 0.25), Vec3::new(-0.35, 0.35, 0.1)];

/// The two back lights.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct CharBackLight {
    /// Direction towards each light in the character's space (CryEngine: +Y forward, Z up), normalised.
    pub dirs: [Vec3; 2],
    /// `rgb` = illuminance (lux, i.e. colour x multiplier x the world's lux scale), `w` = specular multiplier.
    pub lights: [Vec4; 2],
}

impl Default for CharBackLight {
    fn default() -> Self {
        CharBackLight::from_positions(CDF_POSITIONS, [Vec4::ZERO; 2])
    }
}

impl CharBackLight {
    /// Lights at the `.cdf`'s `Position`s.
    pub fn from_positions(positions: [Vec3; 2], lights: [Vec4; 2]) -> Self {
        CharBackLight { dirs: positions.map(Vec3::normalize_or_zero), lights }
    }

    /// Whether either light has a colour.
    pub fn active(&self) -> bool {
        self.lights.iter().any(|l| l.truncate().max_element() > 0.0)
    }

    fn params(&self) -> BackLightParams {
        BackLightParams { dirs: self.dirs.map(|d| d.extend(1.0)), lights: self.lights }
    }

    /// Whether `other` differs enough from `self` to be worth uploading to the materials (they are all rebuilt then).
    fn differs(&self, other: &CharBackLight) -> bool {
        let scale = 0.004 * self.lights.iter().chain(&other.lights).map(|l| l.max_element()).fold(1.0, f32::max);
        self.lights.iter().zip(&other.lights).any(|(a, b)| (*a - *b).abs().max_element() > scale)
            || self.dirs.iter().zip(&other.dirs).any(|(a, b)| a.distance(*b) > 0.004)
    }
}

/// What the shader reads per material (an element of the bindless data array, binding 101, or the uniform at binding 50).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct BackLightParams {
    /// `xyz`: direction towards the light in the character's space, `w`: fall-off (1).
    pub dirs: [Vec4; 2],
    /// `rgb`: illuminance (lux), `w`: specular multiplier.
    pub lights: [Vec4; 2],
}

/// The GPU form of a [`CharBackExt`].
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct BackData {
    params: BackLightParams,
    /// `rgb`: factor on the material's base colour to get the albedo the back lights are multiplied with (1 for
    /// everything but skin, see [`skin_tag`]).
    albedo: Vec4,
}

/// The material extension.
///
/// It is *bindless*, like `StandardMaterial`: an `ExtendedMaterial` is only bindless if both halves are, and without
/// it every character material gets its own bind group and uniform buffer, the draws stop batching and the render
/// world spends milliseconds on bind groups and buffer writes per frame. The data lives in one array (binding 101)
/// that the shader indexes by the material slot; the extension's index table sits at binding 100 and uses the
/// bindless indices 50.. so that it does not clash with the 0..31 of `StandardMaterial`.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[data(50, BackData, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct CharBackExt {
    pub params: BackLightParams,
    /// See [`BackData::albedo`].
    pub albedo: Vec4,
}

impl Default for CharBackExt {
    fn default() -> Self {
        CharBackExt { params: BackLightParams::default(), albedo: Vec4::ONE }
    }
}

impl From<&CharBackExt> for BackData {
    fn from(e: &CharBackExt) -> Self {
        BackData { params: e.params, albedo: e.albedo }
    }
}

impl MaterialExtension for CharBackExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_cry/shade/backlight.wesl".into()
    }
}

/// Registers [`CharMaterial`], the [`CharBackLight`] resource and the systems that give characters the term.
pub struct CharLightPlugin;

impl Plugin for CharLightPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "backlight.wesl");
        app.add_plugins(MaterialPlugin::<CharMaterial>::default())
            .init_resource::<CharBackLight>()
            .init_resource::<Mirrors>()
            .add_systems(PostUpdate, (swap_materials, upload_lights).chain());
    }
}

/// The `CharMaterial` made for each `StandardMaterial` (ids only: the entities own the handles).
#[derive(Resource, Default)]
struct Mirrors(HashMap<AssetId<StandardMaterial>, AssetId<CharMaterial>>);

#[allow(clippy::too_many_arguments)]
fn swap_materials(
    mut commands: Commands,
    light: Res<CharBackLight>,
    standard: Res<Assets<StandardMaterial>>,
    mut extended: ResMut<Assets<CharMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    fresh: Query<(Entity, &MeshMaterial3d<StandardMaterial>), Changed<MeshMaterial3d<StandardMaterial>>>,
    all: Query<(Entity, &MeshMaterial3d<StandardMaterial>)>,
    mut was_active: Local<bool>,
) {
    let active = light.active();
    let first = active && !*was_active;
    *was_active = active;
    if !active {
        return;
    }
    // everything once when the lights come on, then only what appears
    let mut swap = |entity: Entity, handle: &Handle<StandardMaterial>| {
        let Some(base) = standard.get(handle) else { return };
        let Some(skin) = decode_tag(base.lightmap_exposure) else { return };
        let id = handle.id();
        let shared = mirrors.0.get(&id).and_then(|m| extended.get_strong_handle(*m));
        let mirror = shared.unwrap_or_else(|| {
            let mut world = base.clone();
            world_look(&mut world, skin);
            // the back lights keep their own calibration: the albedo they use is the stage one
            let stage = base.base_color.to_linear();
            let now = world.base_color.to_linear();
            let albedo = match skin {
                Kind::Skin(d) => (Vec3::splat(d) / Vec3::new(now.red, now.green, now.blue).max(Vec3::splat(1e-4))).extend(1.0),
                _ => (Vec3::new(stage.red, stage.green, stage.blue) / Vec3::new(now.red, now.green, now.blue).max(Vec3::splat(1e-4))).extend(1.0),
            };
            let strong = extended.add(CharMaterial { base: world, extension: CharBackExt { params: light.params(), albedo } });
            mirrors.0.insert(id, strong.id());
            strong
        });
        commands.entity(entity).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(mirror));
    };
    if first {
        for (e, m) in &all {
            swap(e, &m.0);
        }
    } else {
        for (e, m) in &fresh {
            swap(e, &m.0);
        }
    }
}

/// How the world's light (nominal engine units) differs from the login stage's (three times that, `LOGIN_EXPOSURE`):
/// the skin and costume colours were calibrated on the stage, so under the world's sun they are gain times too dark
/// (grey-brown faces, dim costumes), and the sky the environment light shows is a bright mirror for the reflections the
/// stage's `beach_blur2` only tints. Applied to the copy the swap makes, so the login stages (no swap) are unchanged.
const WORLD_DIFFUSE_GAIN: f32 = 3.0;
/// Factor on the specular reflectance of costumes in the world (the sky is bright, the stage's environment is not).
const WORLD_REFLECTANCE: f32 = 0.3;

fn world_look(m: &mut StandardMaterial, kind: Kind) {
    if kind == Kind::Neutral {
        return;
    }
    let k = WORLD_DIFFUSE_GAIN;
    let c = m.base_color.to_linear();
    m.base_color = Color::linear_rgba(c.red * k, c.green * k, c.blue * k, c.alpha);
    if matches!(kind, Kind::Costume) {
        m.reflectance *= WORLD_REFLECTANCE;
    }
}

/// The least time between two uploads of the lights, in seconds.
const UPLOAD_INTERVAL: f32 = 0.5;

/// How many materials an upload rewrites per frame. Every changed material asset is re-extracted and re-prepared by the
/// render world (about 10 us each), so rewriting the ~1000 materials of a town at once costs one 10 ms frame; spread over
/// 16 frames it is not noticeable (and a 0.4 % step in the lights, a few frames late, is not either).
const UPLOAD_CHUNK: usize = 64;

/// What [`upload_lights`] remembers.
#[derive(Default)]
struct Upload {
    /// The lights of the last upload that was started, and when.
    sent: Option<(CharBackLight, f32)>,
    /// The materials that still have to get `params`.
    queue: Vec<AssetId<CharMaterial>>,
    params: BackLightParams,
}

/// Pushes the lights to every [`CharMaterial`] when they changed (the time of day moves slowly): by more than a
/// fraction of a percent and not more often than [`UPLOAD_INTERVAL`], a few materials per frame.
fn upload_lights(
    light: Res<CharBackLight>,
    time: Res<Time<Real>>,
    mut extended: ResMut<Assets<CharMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    mut up: Local<Upload>,
) {
    if !light.active() {
        up.queue.clear();
        return;
    }
    let now = time.elapsed_secs();
    if up.queue.is_empty() {
        if up.sent.as_ref().is_some_and(|(s, at)| !s.differs(&light) || now - at < UPLOAD_INTERVAL) {
            return;
        }
        up.sent = Some((*light, now));
        up.params = light.params();
        // forget the materials nobody uses any more
        mirrors.0.retain(|_, m| extended.contains(*m));
        up.queue.extend(extended.ids());
    }
    let params = up.params;
    for _ in 0..UPLOAD_CHUNK {
        let Some(id) = up.queue.pop() else { break };
        if let Some(mut m) = extended.get_mut(id) {
            m.extension.params = params;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_character_material_is_bindless_like_the_standard_material() {
        assert!(<StandardMaterial as AsBindGroup>::bindless_slot_count().is_some());
        assert!(<CharMaterial as AsBindGroup>::bindless_slot_count().is_some());
        let d = <CharMaterial as AsBindGroup>::bindless_descriptor().expect("a bindless descriptor");
        assert_eq!(d.index_tables.len(), 2);
    }

    #[test]
    fn tags_round_trip_and_the_default_is_not_a_tag() {
        assert_eq!(decode_tag(TAG), Some(Kind::Costume));
        assert_eq!(decode_tag(NEUTRAL_TAG), Some(Kind::Neutral));
        assert_eq!(decode_tag(1.0), None);
        assert_eq!(decode_tag(skin_tag(0.086)), Some(Kind::Skin(skin_tag(0.086) - 2.0)));
        assert!((skin_tag(0.086) - 2.086).abs() < 1e-6);
    }

    #[test]
    fn lights_are_inactive_until_they_have_a_colour_and_small_changes_are_not_uploaded() {
        let mut l = CharBackLight::default();
        assert!(!l.active());
        l.lights[0] = Vec4::new(5000.0, 5000.0, 5000.0, 1.0);
        assert!(l.active());
        let mut m = l;
        m.lights[0].x += 5.0;
        assert!(!l.differs(&m));
        m.lights[0].x += 500.0;
        assert!(l.differs(&m));
    }

    #[test]
    fn directions_are_the_normalised_cdf_positions_light_1_behind_the_character() {
        let l = CharBackLight::default();
        assert!(l.dirs.iter().all(|d| (d.length() - 1.0).abs() < 1e-6));
        // +Y is forward: light 1 is behind (and to the right, above), light 2 in front on the left
        assert!(l.dirs[0].y < -0.8 && l.dirs[0].x > 0.0 && l.dirs[0].z > 0.0);
        assert!(l.dirs[1].y > 0.0 && l.dirs[1].x < 0.0);
    }
}
