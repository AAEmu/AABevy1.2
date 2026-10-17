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

/// The tag of a skin material, whose `Diffuse` has the luma `diffuse`: `2 + diffuse`. The skin's base colour carries the
/// lit skin colour (`BSkinColor^1.5 * 1.5` x a gain, see `skin_base_color`) that the engine puts into the sun and
/// ambient light instead, while the back lights reach the skin as plain `colour * N.L * texture * Diffuse`; the term
/// therefore uses `texture * diffuse` as the albedo of skins.
pub fn skin_tag(diffuse: f32) -> f32 {
    2.0 + diffuse.clamp(0.0, 0.99)
}

/// `Some(None)` for a character material, `Some(Some(diffuse))` for a skin, `None` for anything else.
fn decode_tag(exposure: f32) -> Option<Option<f32>> {
    if exposure == TAG {
        Some(None)
    } else if (2.0..3.0).contains(&exposure) {
        Some(Some(exposure - 2.0))
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

/// The uniform of the shader (binding 100).
#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType, Reflect)]
pub struct BackLightParams {
    /// `xyz`: direction towards the light in the character's space, `w`: fall-off (1).
    pub dirs: [Vec4; 2],
    /// `rgb`: illuminance (lux), `w`: specular multiplier.
    pub lights: [Vec4; 2],
}

/// The material extension.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct CharBackExt {
    #[uniform(100)]
    pub params: BackLightParams,
    /// `rgb`: factor on the material's base colour to get the albedo the back lights are multiplied with (1 for
    /// everything but skin, see [`skin_tag`]).
    #[uniform(101)]
    pub albedo: Vec4,
}

impl Default for CharBackExt {
    fn default() -> Self {
        CharBackExt { params: BackLightParams::default(), albedo: Vec4::ONE }
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
            let albedo = skin.map_or(Vec4::ONE, |d| {
                let c = base.base_color.to_linear();
                (Vec3::splat(d) / Vec3::new(c.red, c.green, c.blue).max(Vec3::splat(1e-4))).extend(1.0)
            });
            let strong = extended.add(CharMaterial { base: base.clone(), extension: CharBackExt { params: light.params(), albedo } });
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

/// Pushes the lights to every [`CharMaterial`] when they changed (the time of day moves slowly).
fn upload_lights(
    light: Res<CharBackLight>,
    mut extended: ResMut<Assets<CharMaterial>>,
    mut mirrors: ResMut<Mirrors>,
    mut sent: Local<Option<CharBackLight>>,
) {
    if !light.active() || sent.as_ref().is_some_and(|s| !s.differs(&light)) {
        return;
    }
    *sent = Some(*light);
    let params = light.params();
    for (_, m) in extended.iter_mut() {
        m.extension.params = params;
    }
    // forget the materials nobody uses any more
    mirrors.0.retain(|_, m| extended.contains(*m));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip_and_the_default_is_not_a_tag() {
        assert_eq!(decode_tag(TAG), Some(None));
        assert_eq!(decode_tag(1.0), None);
        assert_eq!(decode_tag(skin_tag(0.086)), Some(Some(skin_tag(0.086) - 2.0)));
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
