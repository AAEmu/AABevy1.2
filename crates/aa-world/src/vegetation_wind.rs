//! Steady global wind and main bending from the original 1.2 producer/renderer.
//! Detail RGB, physical wind areas and complex/skeletal vegetation are separate paths.
use aa_cry::scene::LoadedModel;
use bevy::prelude::*;

/// A supported ordinary vegetation part; follows its existing cell/sector entity lifetime.
#[derive(Component, Clone, Copy, Debug)]
pub struct VegetationWindPart {
    pub cell: (i32, i32),
    pub group: u16,
    pub bending: f32,
    pub half_height: f32,
}

/// 30142970 reads EnvState/WindVector with fallback 1,0,0; malformed input becomes zero.
pub fn environment(xml: &str) -> Vec3 {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        return Vec3::X;
    };
    let text = doc
        .descendants()
        .find(|n| n.has_tag_name("EnvState"))
        .and_then(|n| n.attribute("WindVector"))
        .unwrap_or("1,0,0");
    let mut values = text.split(',').map(str::trim).map(str::parse::<f32>);
    match (values.next(), values.next(), values.next()) {
        (Some(Ok(x)), Some(Ok(y)), Some(Ok(z))) if Vec3::new(x, y, z).is_finite() => {
            Vec3::new(x, y, z)
        }
        _ => Vec3::ZERO,
    }
}

/// Native CStatObj +180 is vertical half-height (3001BA80), not sphere radius +172.
/// Compound/transformed nodes need a separate native subobject-frame proof.
pub(crate) fn model_half_height(loaded: &LoadedModel) -> Option<f32> {
    let [(index, parts)] = loaded.parts.as_slice() else {
        return None;
    };
    if parts.iter().any(|p| p.skinned) {
        return None;
    }
    let node = &loaded.model.nodes[*index];
    if !Mat4::from(aa_cry::scene::render_transform(&loaded.model, *index))
        .abs_diff_eq(Mat4::IDENTITY, 1e-6)
    {
        return None;
    }
    let mesh = loaded.model.meshes.get(node.mesh?)?;
    let h = (mesh.bbox_max.z - mesh.bbox_min.z) * 0.5;
    (h.is_finite() && h >= 0.0).then_some(h)
}

/// SetupBending3003E560: steady base XY; group scalar is applied after SI_BendInfo's waves.
pub fn parameters(wind: Vec3, bending: f32, half_height: f32) -> Vec4 {
    let q = wind.truncate() * 0.25;
    let base = q * (2.0 / (q.length() + 2.0));
    Vec4::new(base.x, base.y, bending * 0.1, half_height)
}

/// Renderer380C8050: all 1024 entries include both endpoints (360/1023 degrees).
pub fn lookup(index: i32) -> f32 {
    let angle = (index & 1023) as f32
        * f32::from_bits(0x3eb42d0b)
        * std::f32::consts::PI
        * f32::from_bits(0x3bb60b61);
    angle.sin()
}

/// SI_BendInfo38072920. Truncate before masking, including negative phases.
pub fn bend_info(p: Vec4, translation: Vec3, seconds: f32) -> Vec4 {
    let f = 0.4 / (p.w + 1.0) + 0.2;
    let ix = ((f * seconds + translation.x * 0.125) * 1024.0) as i32;
    let iy = ((f * 1.125 * seconds + translation.y * 0.125) * 1024.0) as i32;
    let wave = Vec2::new(
        lookup(ix) * (p.x * 0.008 - p.y * 0.002),
        lookup(iy) * (p.y * 0.008 - p.x * 0.002),
    );
    let base = p.truncate().truncate();
    let xy = (base + wave * 50.0) * p.z;
    Vec4::new(
        xy.x,
        xy.y,
        p.z * base.length() * 2.0,
        (base + wave).length() * 0.3,
    )
}

/// Original modificatorvt.cfi main branch, high quality; input/output remain in Cry local axes.
/// `matrix` maps native local coordinates to native world coordinates, including uniform scale.
pub fn bend(
    position: Vec3,
    matrix: Mat4,
    p: Vec4,
    seconds: f32,
    grass: bool,
    high_quality: bool,
) -> Vec3 {
    if p.z == 0.0 {
        return position;
    }
    let world = matrix.w_axis.truncate();
    let info = bend_info(p, world, seconds);
    let transformed = matrix.transpose().transform_vector3(info.truncate());
    let mut xy = transformed.truncate();
    if high_quality {
        let speed = info.w * if grass { position.z } else { 1.0 };
        let phase = (seconds + world.length() * 2.0) * 2.0;
        let waves = [0.95, 0.45793, 0.913, 0.5793].map(|f| {
            let v = ((phase * f * 0.1).fract_gl() * 2.0 - 1.0) * 0.7 * speed;
            let triangle = ((v + 0.5).fract_gl() * 2.0 - 1.0).abs();
            triangle * triangle * (3.0 - 2.0 * triangle)
        });
        xy += Vec2::new(waves[0] + waves[1], waves[2] + waves[3])
            * (0.3333 * (xy.x + xy.y).clamp(0.0, 1.0));
    }
    xy *= 0.015;
    let factor = (position.z * info.z + 1.0).powi(2);
    let offset = xy * (factor * factor - factor);
    let moved = position + offset.extend(0.0);
    moved.normalize_or_zero() * position.length()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_environment_and_lookup_endpoints() {
        assert_eq!(
            environment("<Environment><EnvState/></Environment>"),
            Vec3::X
        );
        assert_eq!(
            environment("<Environment><EnvState WindVector=\"1,1,0\"/></Environment>"),
            Vec3::new(1.0, 1.0, 0.0)
        );
        assert_eq!(
            environment("<Environment><EnvState WindVector=\"1,no,0\"/></Environment>"),
            Vec3::ZERO
        );
        assert_eq!(lookup(-1), lookup(1023));
        assert!(lookup(1023).abs() < 3e-7);
        assert!(lookup(512) < -0.003); // not the zero of a 1024-spaced analytic cycle.
    }
    #[test]
    fn main_branch_keeps_origin_and_radius_and_uses_native_matrix() {
        let p = parameters(Vec3::new(1.0, 1.0, 0.0), 2.0, 4.0);
        let m = Mat4::from_scale_rotation_translation(
            Vec3::splat(1.3),
            Quat::from_euler(EulerRot::XYZ, 0.1, 0.2, 0.7),
            Vec3::new(-100.25, 200.5, 10.0),
        );
        assert_eq!(bend(Vec3::ZERO, m, p, 3.25, true, true), Vec3::ZERO);
        let v = Vec3::new(0.4, 0.2, 5.0);
        let a = bend(v, m, p, 3.25, true, true);
        let b = bend(v, m, p, 4.25, true, true);
        assert!((a.length() - v.length()).abs() < 1e-5);
        assert!((a - b).length() > 0.0001);
        assert_eq!(bend(v, m, Vec4::ZERO, 4.25, true, true), v);
    }
    #[test]
    fn original_flower_root_bounds_and_identity_qualification() {
        if !crate::pak_available() {
            return;
        }
        use aa_cry::{Model, mesh::NodeKind, scene::Part};
        use std::sync::Arc;
        let model = Model::load("objects/env/05_nature/00_common/flower_a.cgf").unwrap();
        let parts = model
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.kind == NodeKind::Mesh && n.mesh.is_some())
            .map(|(i, _)| {
                (
                    i,
                    vec![Part {
                        mesh: Handle::default(),
                        material: Handle::default(),
                        aabb: default(),
                        skinned: false,
                    }],
                )
            })
            .collect();
        let mut loaded = LoadedModel {
            model: Arc::new(model),
            skeleton: None,
            inverse_bindposes: None,
            material_path: None,
            parts,
        };
        let height =
            model_half_height(&loaded).expect("authored flower has one identity mesh node");
        eprintln!("original flower native vertical half-height={height:?}");
        assert!(height > 0.1 && height < 1.0);
        let index = loaded.parts[0].0;
        Arc::make_mut(&mut loaded.model).nodes[index]
            .local
            .translation
            .x = 1.0;
        assert_eq!(model_half_height(&loaded), None);
        Arc::make_mut(&mut loaded.model).nodes[index]
            .local
            .translation
            .x = 0.0;
        loaded.parts.push(loaded.parts[0].clone());
        assert_eq!(model_half_height(&loaded), None);
    }
}
