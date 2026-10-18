//! Ordinary native DLight HDR colour, after the existing style luminance.
//!
//! Engine30131C8A initializes the base to2;30121B70 returns it. An external
//! override through30123720 remains unresolved, so no guessed state input is exposed.

use bevy::prelude::*;

const NATIVE_DEFAULT_HDR_BASE: f32 = 2.0;

pub(super) fn color(base: Vec3, style: f32, hdr: f32) -> Option<Vec3> {
    if !hdr.is_finite() { return None; }
    let factor = NATIVE_DEFAULT_HDR_BASE.powf(hdr);
    // Keep both float32 multiplies in native38179800's order, including HDR0.
    let rgb = (base * style) * factor;
    (factor.is_finite() && rgb.is_finite()).then_some(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_default_hdr_negative_fractional_zero_and_style_order() {
        let base = Vec3::new(0.7, 0.4, 0.2);
        // Original38179800, engine30121B70 and imported x86MSVCR100 powf:
        //100 executed RGBA uploads plus4 native HDR-disable gates, bit-exact.
        for (hdr, factor) in [(-4.0, 0.0625), (-1.0, 0.5), (-0.5, 0.70710677),
            (0.0, 1.0), (0.5, 1.4142135), (0.75, 1.6817929), (0.8, 1.7411011),
            (1.0, 2.0), (1.5, 2.828427), (4.0, 16.0)] {
            for style in [1.0, 0.58999997, 0.125, 0.5, 0.775] {
                let actual = color(base, style, hdr).unwrap();
                let native = (base * style) * factor;
                assert!(actual.abs_diff_eq(native, 2e-6), "HDR{hdr} style{style}: {actual:?} vs{native:?}");
                assert_eq!(color(base, style, 0.0), Some(base * style), "HDR0 preserves prior bits");
            }
        }
        // This exact source-upload case distinguishes the two multiplication orders.
        let factor = 2.0_f32.powf(-0.5);
        assert_ne!((base * 0.33625) * factor, base * (0.33625 * factor));
        assert_eq!(color(base, 0.33625, -0.5), Some((base * 0.33625) * factor));
        assert_eq!(color(base, 1.0, f32::NAN), None);
        assert_eq!(color(base, 1.0, f32::INFINITY), None);
        assert_eq!(color(base, 1.0, 128.0), None);
        assert_eq!(color(Vec3::splat(f32::MAX), 1.0, 1.0), None);
        assert_eq!(color(base, 1.0, -200.0), Some(Vec3::ZERO));
    }

    #[test]
    fn original_dock_hdr_admission_updates_and_releases_point_components() {
        if !crate::pak_available() { return; }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(crate::WorldConfig::new("main_world"));
        app.add_plugins(super::super::LocalLightPlugin);
        let data = crate::placement::load_cell_objects("main_world", 18, 7);
        let lamp = data.lights.iter().find(|p| p.guid == 0x405c2f3b9f41a176).unwrap().clone();
        assert_eq!(lamp.hdr_dynamic, 1.0);
        let expected = lamp.diffuse * 2.0;
        let position = crate::cry_to_bevy(crate::coords::cell_origin(18, 7) + lamp.matrix.w_axis.truncate());
        app.world_mut().spawn((crate::WorldFocus, GlobalTransform::from_translation(position)));
        app.world_mut().commands().queue(move |world: &mut World| {
            super::super::spawn_cell(&mut world.commands(), (18, 7), &[lamp]);
        });
        app.update(); app.update();
        let world = app.world_mut();
        let actual = world.query::<&PointLight>().single(world).unwrap().color.to_linear().to_vec3();
        assert_eq!(actual, expected);
        // Invalid out-of-range diagnostics must not upload infinities to PBR.
        world.query::<&mut super::super::LocalLight>().single_mut(world).unwrap().placement.hdr_dynamic = 128.0;
        world.resource_mut::<super::super::LocalLightSettings>().distance = 401.0;
        app.update();
        assert_eq!(app.world().resource::<super::super::LocalLightStats>().unsupported, 1);
        let world = app.world_mut();
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 0);
        world.resource_mut::<super::super::LocalLightSettings>().enabled = false;
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 0);
    }
}
