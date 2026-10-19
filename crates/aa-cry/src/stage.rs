//! Lighting for showing characters and props the way the 1.2 client's login stage does, and
//! the environment-map helpers behind it.
//!
//! ## Units
//!
//! CryEngine's HDR lighting works in "light units": a surface with albedo `a` facing a light of
//! intensity `E` shows `a * E`, then the film curve and the eye adaptation bring that to the
//! screen. The materials of the client are authored for it: costume `Diffuse` is about 0.22, so
//! the stage's sun (`Sun color multiplier` ~2 plus ~1.2 of sky light in the login zone) lifts
//! `0.22 * texture` to a normal image. [`crate::shade`] keeps `Diffuse * texture` as the base
//! colour, so the lights here are expressed in the same units: [`LIGHT_UNIT_LUX`] converts one
//! engine unit to Bevy lux for a camera with the default exposure (a white Lambert surface
//! facing a light of 1 unit shows 1.0 before tone mapping).
//!
//! ## Login stage
//!
//! [`StageLighting::login`] is calibrated against the captures in `reference/character_create`
//! and `reference/character_select` (see `docs/aa-cry.md`): a key light high in front of the
//! character slightly to the viewer's left, a blue-ish fill from the right, a warm rim from
//! behind and the sky as environment light. Directions are in the **stage frame**: the frame of
//! the figure, whose front is **-Z** (the figure is turned to the camera with
//! `from_rotation_arc(NEG_Z, to_camera)`), so +Z points from the camera over the figure's head,
//! +Y is up and +X is the viewer's left. Pass the same rotation to [`StageLighting::spawn`].
//! (The first calibration took +Z for the direction to the camera, which put the key light behind
//! the figure and the rim light in front: `docs/aa-cry.md`, "The stage frame".)

use bevy::camera::Exposure;
use bevy::light::{CascadeShadowConfigBuilder, GeneratedEnvironmentMapLight};
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};

use crate::material::Material as CryMaterial;
use crate::scene::CryAssets;

/// Lux of a directional light that gives a white Lambert surface the display value 1.0 with
/// Bevy's default exposure (`pi / exposure`, exposure = `1 / (1.2 * 2^9.7)`).
pub const LIGHT_UNIT_LUX: f32 = std::f32::consts::PI * 998.0;

/// The environment cube map of a material (`<Texture Map="Environment">`), as a pak path.
/// 88% of the costume materials name `textures/cubemaps/beach_blur2.dds`.
pub fn material_environment(m: &CryMaterial) -> Option<String> {
    let t = m.texture(&crate::material::TextureMap::Environment)?;
    let path = t.pak_path();
    (!path.is_empty()).then_some(path)
}

/// The environment cube map the character materials use unless they name another one.
pub const DEFAULT_ENVIRONMENT: &str = "textures/cubemaps/beach_blur2.dds";

/// Environment light (diffuse ambient + specular reflections) from a cube map of the pak; put it
/// on the camera. `intensity` is in cd/m2 as for Bevy's own environment maps (about 1000 =
/// the cube map's own brightness at the default exposure).
pub fn environment_light(cry: &mut CryAssets, cubemap: &str, intensity: f32) -> Option<GeneratedEnvironmentMapLight> {
    let environment_map = cry.cube_texture(cubemap)?;
    Some(GeneratedEnvironmentMapLight { environment_map, intensity, ..default() })
}

/// One directional light of a stage, in the stage frame.
#[derive(Clone, Copy, Debug)]
pub struct StageLight {
    /// Direction from the subject towards the light (need not be normalised).
    pub from: Vec3,
    /// Linear colour.
    pub color: Vec3,
    /// Intensity in engine light units (see the module docs).
    pub intensity: f32,
    pub shadows: bool,
}

/// Shadow map setup of the stage's shadow casting lights. Bevy's default (4 cascades out to 150 m,
/// depth bias 2 cm) is made for a landscape: the first cascade is 10 m deep, so the figure's chin
/// shadow on the neck (3 cm) and the rim of a cutout fall inside the bias and one or two texels.
#[derive(Clone, Copy, Debug)]
pub struct StageShadows {
    pub cascades: usize,
    /// Distances from the camera in metres: the shadowed range and the far bound of the first
    /// cascade (the figure stands 5 - 9 m from the login cameras).
    pub near: f32,
    pub far: f32,
    pub first: f32,
    pub depth_bias: f32,
    pub normal_bias: f32,
}

impl Default for StageShadows {
    fn default() -> Self {
        StageShadows { cascades: 2, near: 1.0, far: 30.0, first: 10.0, depth_bias: 0.02, normal_bias: 1.8 }
    }
}

/// A lighting rig for characters. All intensities are in engine light units.
#[derive(Clone, Debug)]
pub struct StageLighting {
    pub lights: Vec<StageLight>,
    pub shadows: StageShadows,
    /// Environment cube map and its intensity (cd/m2).
    pub environment: Option<(String, f32)>,
    /// Flat ambient light (linear colour, cd/m2) on top of the environment light.
    pub ambient: Option<(Vec3, f32)>,
}

/// The captures show costumes about three times brighter than the engine's nominal light values
/// (sun multiplier 2 + sky 1.2 on a 0.216 albedo) suggest: the client's HDR exposure. Folded into
/// [`StageLighting::login`].
pub const LOGIN_EXPOSURE: f32 = 3.0;

impl StageLighting {
    /// The login stage (character select / create screens).
    pub fn login() -> StageLighting {
        StageLighting {
            lights: vec![
                // key: warm sun, high, in front, slightly to the viewer's left
                StageLight { from: Vec3::new(0.45, 0.60, -0.65), color: Vec3::new(1.0, 0.95, 0.88), intensity: 2.1, shadows: true },
                // fill: sky blue from the right
                StageLight { from: Vec3::new(-0.8, 0.25, -0.5), color: Vec3::new(0.62, 0.72, 1.0), intensity: 0.7, shadows: false },
                // rim: from behind and above
                StageLight { from: Vec3::new(-0.1, 0.5, 0.85), color: Vec3::new(1.0, 0.9, 0.8), intensity: 1.0, shadows: false },
            ],
            shadows: StageShadows::default(),
            environment: Some((DEFAULT_ENVIRONMENT.to_string(), 1500.0)),
            ambient: Some((Vec3::new(0.75, 0.8, 0.9), 150.0)),
        }
        .scaled(LOGIN_EXPOSURE)
    }

    /// Scales every light and the environment (a quick exposure knob).
    pub fn scaled(mut self, k: f32) -> StageLighting {
        for l in &mut self.lights {
            l.intensity *= k;
        }
        if let Some((_, i)) = &mut self.environment {
            *i *= k;
        }
        if let Some((_, i)) = &mut self.ambient {
            *i *= k;
        }
        self
    }

    /// Spawns the directional lights (`frame` rotates the stage frame into the world) and puts
    /// the environment and ambient light on `camera`. Returns the light entities.
    pub fn spawn(&self, commands: &mut Commands, cry: &mut CryAssets, camera: Entity, frame: Quat) -> Vec<Entity> {
        let mut lights = Vec::new();
        for l in &self.lights {
            let at = frame * (l.from.normalize() * 10.0);
            let e = commands
                .spawn((
                    Name::new("stage light"),
                    DirectionalLight {
                        illuminance: l.intensity * LIGHT_UNIT_LUX,
                        color: Color::linear_rgb(l.color.x, l.color.y, l.color.z),
                        shadow_maps_enabled: l.shadows,
                        shadow_depth_bias: self.shadows.depth_bias,
                        shadow_normal_bias: self.shadows.normal_bias,
                        ..default()
                    },
                    CascadeShadowConfigBuilder {
                        num_cascades: self.shadows.cascades,
                        minimum_distance: self.shadows.near,
                        maximum_distance: self.shadows.far,
                        first_cascade_far_bound: self.shadows.first,
                        ..default()
                    }
                    .build(),
                    Transform::from_translation(at).looking_at(Vec3::ZERO, Vec3::Y),
                ))
                .id();
            lights.push(e);
        }
        if let Some((path, intensity)) = &self.environment {
            if let Some(env) = environment_light(cry, path, *intensity) {
                commands.entity(camera).insert(env);
            }
        }
        if let Some((c, brightness)) = self.ambient {
            commands.entity(camera).insert(AmbientLight { color: Color::linear_rgb(c.x, c.y, c.z), brightness, ..default() });
        }
        lights
    }
}

/// The camera exposure the lights above assume (Bevy's default, spelled out).
pub fn stage_exposure() -> Exposure {
    Exposure::default()
}

/// The look of the client's final image (saturated colours, deep shadows): the camera component
/// to put next to [`StageLighting`].
pub fn login_color_grading() -> ColorGrading {
    color_grading(1.6, 1.2)
}

/// A colour grading with the given post-tonemapping saturation and contrast.
pub fn color_grading(saturation: f32, contrast: f32) -> ColorGrading {
    let section = ColorGradingSection { contrast, ..default() };
    ColorGrading {
        global: ColorGradingGlobal { post_saturation: saturation, ..default() },
        shadows: section,
        midtones: section,
        highlights: section,
    }
}
