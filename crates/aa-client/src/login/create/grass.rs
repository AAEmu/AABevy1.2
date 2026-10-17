//! The grass material of the step 3 field: a `StandardMaterial` with a tone curve on the lit colour.
//!
//! The captures' near grass is far more contrasty than anything the stage rig makes of it: its darkest quarter is
//! about 0.15 x and its brightest few percent 3 - 4 x the radiance of ours, in all four races, i.e. over the whole
//! range the captured radiance is about a power of 1.8 - 2.4 of ours (the quantile map of the near field of the
//! captures against ours, `docs/aa-client-create.md`). That is the engine's tone mapping and light model at work (HDR
//! sun, a toe in the film curve), which the stage, built for the lit figure, does not have. The field's materials are
//! lit by the same lights as the figure, so the curve is applied to the grass alone, after the lighting and before the
//! fog: `rgb' = rgb * (floor + (c / reference) ^ (gamma - 1))` with `c` = the colour channels (or their luminance, see
//! `saturation`): a linear part for the dark gaps (the captures'
//! darkest quarter is not black) and the power for the sunlit blades. Like the character back lights this is a bindless material extension (one
//! bind group for all the grass, see `aa_cry::shade::backlight`).

use bevy::asset::embedded_asset;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// A grass `StandardMaterial` with the curve.
pub type GrassMaterial = ExtendedMaterial<StandardMaterial, GrassExt>;

/// The curve of a material.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[data(50, GrassData, binding_array(101))]
#[bindless(index_table(range(50..51), binding(100)))]
pub struct GrassExt {
    /// The exponent (1 = no curve).
    pub gamma: f32,
    /// The lit radiance (scene referred, times the camera's exposure) from which the power takes over.
    pub reference: f32,
    /// The linear part: `rgb' = rgb * (floor + (rgb / reference) ^ (gamma - 1))`.
    pub floor: f32,
    /// How much of the power acts on every colour channel (1) and how much on the luminance (0, the hue is kept).
    pub saturation: f32,
}

/// The GPU form of [`GrassExt`].
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct GrassData {
    /// `x` = exponent, `y` = reference, `z` = floor, `w` = saturation.
    curve: Vec4,
}

impl From<&GrassExt> for GrassData {
    fn from(e: &GrassExt) -> Self {
        GrassData { curve: Vec4::new(e.gamma, e.reference, e.floor, e.saturation) }
    }
}

impl MaterialExtension for GrassExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(bevy::asset::AssetPath::from(format!("embedded://{}/login/create/grass.wesl", env!("CARGO_CRATE_NAME"))))
    }
}

/// Registers [`GrassMaterial`].
pub fn plugin(app: &mut App) {
    embedded_asset!(app, "grass.wesl");
    app.add_plugins(MaterialPlugin::<GrassMaterial>::default());
}
