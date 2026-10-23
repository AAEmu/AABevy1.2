//! Material flashes: the `FxMaterial` items of effect groups (`fx_items.fx_detail_type`). Each names a "dual material" of
//! `custom_dual_materials` (`objects/effects/nu_multi_hit.mtl`) and a fade time (`fx_materials.custom_dual_material_fade_time`): the
//! engine draws that material over the unit's own for the fade time. The normal attack's hit has one (`nu_multi_hit`: an orange,
//! `Diffuse 1 0.66 0.45`, `Opacity 0.2`, 0.15 s): 32 groups of 24 skills use it on the target at the blow; the rest of the table (ice,
//! blood, poison ... 35 materials) has no item in a group of a player skill.
//!
//! A flash is a second copy of every mesh of the unit (the same skin, the same joints) drawn with a blended [`StandardMaterial`] of
//! the material's diffuse colour at its opacity, which fades to nothing over the fade time. The `.mtl` textures (the ice's frost) are
//! not drawn: only the colour.

use aa_cry::shade::backlight::CharMaterial;
use aa_cry::shade::hair::HairMaterial;
use aa_cry::shade::illum::IllumMaterial;
use aa_cry::shade::skin::SkinMaterial;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::skinning::SkinnedMesh;
use bevy::prelude::*;

/// What a flash looks like: the dual material's colour and opacity at its strongest, and how long it takes to fade out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flash {
    pub color: Vec3,
    pub opacity: f32,
    pub fade: f32,
}

impl Flash {
    /// The flash of the dual material `mtl` (a pak path without the extension, as `custom_dual_materials.filename`) fading over `fade` s;
    /// `None` when the material is not there or the fade time is 0 (a permanent overlay: the data has one, the soot of Blacken, which
    /// belongs to its buff).
    pub fn load(mtl: &str, fade: f32) -> Option<Flash> {
        if fade <= 0.0 {
            return None;
        }
        let m = aa_cry::material::Material::load(&format!("{}.mtl", mtl.trim().trim_end_matches(".mtl"))).ok()?;
        Some(Flash { color: m.diffuse, opacity: m.opacity.clamp(0.0, 1.0), fade })
    }

    /// The opacity of the overlay `age` seconds after it started: the material's own at once, none after the fade time.
    pub fn alpha_at(&self, age: f32) -> f32 {
        if age >= self.fade { 0.0 } else { self.opacity * (1.0 - age / self.fade) }
    }
}

/// Put on a unit to flash it (the combat code's request; [`start_flashes`] turns it into overlays).
#[derive(Component)]
pub struct FlashRequest(pub Flash);

/// On an overlay mesh: the flash it plays, since when (`Time::elapsed_secs`), and its material.
#[derive(Component)]
struct Overlay {
    flash: Flash,
    since: f32,
    material: Handle<StandardMaterial>,
}

#[allow(clippy::type_complexity)]
fn start_flashes(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    requests: Query<(Entity, &FlashRequest)>,
    children: Query<&Children>,
    meshes: Query<
        (&Mesh3d, Option<&SkinnedMesh>, Option<&Visibility>),
        (Without<Overlay>, Or<(With<MeshMaterial3d<StandardMaterial>>, With<MeshMaterial3d<CharMaterial>>, With<MeshMaterial3d<HairMaterial>>, With<MeshMaterial3d<SkinMaterial>>, With<MeshMaterial3d<IllumMaterial>>)>),
    >,
) {
    for (unit, request) in &requests {
        commands.entity(unit).remove::<FlashRequest>();
        let flash = request.0;
        // one material for the whole unit
        let [r, g, b] = flash.color.to_array();
        let material = materials.add(StandardMaterial {
            base_color: Color::linear_rgba(r, g, b, flash.opacity),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.8,
            reflectance: 0.0,
            ..default()
        });
        for mesh in children.iter_descendants(unit) {
            let Ok((handle, skin, visibility)) = meshes.get(mesh) else { continue };
            if visibility.is_some_and(|v| *v == Visibility::Hidden) {
                continue;
            }
            let mut overlay = commands.spawn((
                Name::new("flash"),
                handle.clone(),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                Visibility::Inherited,
                Overlay { flash, since: time.elapsed_secs(), material: material.clone() },
                ChildOf(mesh),
            ));
            if let Some(skin) = skin {
                overlay.insert((skin.clone(), NoFrustumCulling));
            }
        }
    }
}

/// Fades the overlays and removes them when the flash is over.
fn fade_flashes(mut commands: Commands, time: Res<Time>, mut materials: ResMut<Assets<StandardMaterial>>, overlays: Query<(Entity, &Overlay)>) {
    let now = time.elapsed_secs();
    for (entity, overlay) in &overlays {
        let alpha = overlay.flash.alpha_at(now - overlay.since);
        if alpha <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if let Some(mut m) = materials.get_mut(&overlay.material) {
            m.base_color.set_alpha(alpha);
        }
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(Update, (start_flashes, fade_flashes).chain());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illum_meshes_receive_the_hit_overlay_without_replacing_their_material() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default()).init_resource::<Assets<StandardMaterial>>()
            .add_systems(Update, start_flashes);
        let unit = app.world_mut().spawn(FlashRequest(Flash { color: Vec3::ONE, opacity: 0.2, fade: 0.15 })).id();
        let mesh = app.world_mut().spawn((Mesh3d(Handle::default()), MeshMaterial3d::<IllumMaterial>(Handle::default()), ChildOf(unit))).id();
        app.update();
        assert!(app.world().get::<FlashRequest>(unit).is_none());
        assert!(app.world().get::<MeshMaterial3d<IllumMaterial>>(mesh).is_some());
        let mut query = app.world_mut().query_filtered::<&ChildOf, With<Overlay>>();
        let overlays: Vec<_> = query.iter(app.world()).map(|p| p.parent()).collect();
        assert_eq!(overlays, [mesh]);
    }

    #[test]
    fn a_flash_starts_at_its_opacity_and_fades_out_linearly() {
        let f = Flash { color: Vec3::new(1.0, 0.66, 0.45), opacity: 0.2, fade: 0.15 };
        assert!((f.alpha_at(0.0) - 0.2).abs() < 1e-6);
        assert!((f.alpha_at(0.075) - 0.1).abs() < 1e-6);
        assert_eq!(f.alpha_at(0.15), 0.0);
        assert_eq!(f.alpha_at(2.0), 0.0);
    }

    #[test]
    fn the_hit_flash_is_the_orange_dual_material() {
        if !aa_cry::pak_available() {
            return;
        }
        let f = Flash::load("objects/effects/nu_multi_hit", 0.15).expect("nu_multi_hit");
        assert!((f.opacity - 0.2).abs() < 1e-3 && f.color.x > f.color.y && f.color.y > f.color.z, "{f:?}");
        // a fade time of 0 is a permanent overlay, which belongs to a buff
        assert!(Flash::load("objects/effects/nu_multi_hit", 0.0).is_none());
        assert!(Flash::load("objects/effects/does_not_exist", 1.0).is_none());
    }
}
