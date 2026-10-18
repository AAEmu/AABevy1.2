//! Static authored FogVolume slots. Native CFogVolumeRenderNode and fogvolume.cfx.
//! Local RGB / HDRDynamic=0 only; no global-colour or FlowGraph density adapter.
use std::collections::{HashMap, HashSet};

use crate::coords::{CRY_TO_BEVY, cell_origin, cry_to_bevy};
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    ecs::system::SystemParam,
    light::NotShadowCaster,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::render_resource::{
        AsBindGroup, ColorWrites, CompareFunction, Face, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

pub const MAX_VOLUMES: usize = 32;
// CPU transparency order only, after sky/voxel/decal overlays and before ordinary translucency.
const SORT_BIAS: f32 = -300_000.0;

#[derive(Clone, Debug)]
pub struct FogVolume {
    pub name: String,
    pub guid: u64,
    pub matrix: Mat4,
    pub active: bool,
    pub hidden: bool,
    pub box_shape: bool,
    pub size: Vec3,
    pub color: Vec3,
    pub density: f32,
    pub soft_edges: f32,
    pub direction: Vec3,
    pub shift: f32,
    pub falloff: f32,
    pub global_color: bool,
    pub hdr_dynamic: f32,
}
impl FogVolume {
    pub fn supported(&self) -> bool {
        !self.global_color && self.hdr_dynamic == 0.0 && self.color.min_element() >= 0.0
    }
    pub fn params(&self, cell: (i32, i32), radiance: f32) -> FogParams {
        let origin = cell_origin(cell.0, cell.1);
        let mut linear = self.matrix;
        linear.w_axis = Vec4::W;
        let from_local = Mat4::from_quat(CRY_TO_BEVY) * linear * Mat4::from_scale(self.size * 0.5);
        let center = origin + self.matrix.w_axis.truncate();
        FogParams {
            local_from_world: from_local.inverse(),
            center: cry_to_bevy(center).extend(0.0),
            color: (self.color * radiance).extend(self.density),
            base: cry_to_bevy(center + self.direction * self.shift).extend(self.soft_edges),
            gradient: cry_to_bevy(self.direction * self.falloff)
                .extend(self.box_shape as u8 as f32),
        }
    }
}

/// Uses FogVolume.lua class defaults and the exact 1.2 script binding clamps.
/// Inactive/hidden and unsupported variants remain inspectable data.
pub fn parse(xml: &str) -> Vec<FogVolume> {
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let Some(objects) = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("Objects"))
    else {
        return Vec::new();
    };
    let nodes: HashMap<_, _> = objects
        .children()
        .filter(|n| n.has_tag_name("Entity"))
        .filter_map(|n| Some((crate::entities::guid(n.attribute("EntityGuid")?)?, n)))
        .collect();
    let mut result = Vec::new();
    for e in objects
        .children()
        .filter(|n| n.attribute("EntityClass") == Some("FogVolume"))
    {
        let Some((matrix, hidden)) = crate::entities::transform(e, &nodes, &mut HashSet::new())
        else {
            continue;
        };
        let Some(p) = e.children().find(|n| n.has_tag_name("Properties")) else {
            continue;
        };
        let number = |name, default| {
            p.attribute(name)
                .map(|v| v.parse::<f32>().ok().filter(|v| v.is_finite()))
                .unwrap_or(Some(default))
        };
        let Some(active) = number("bActive", 1.0) else {
            continue;
        };
        let Some([kind, density, soft, hdr, long, lat, shift, falloff]) = (|| {
            Some([
                number("VolumeType", 0.0)?,
                number("GlobalDensity", 1.0)?,
                number("SoftEdges", 1.0)?,
                number("fHDRDynamic", 0.0)?,
                number("FallOffDirLong", 0.0)?,
                number("FallOffDirLati", 90.0)?,
                number("FallOffShift", 0.0)?,
                number("FallOffScale", 1.0)?,
            ])
        })() else {
            continue;
        };
        let color = p
            .attribute("color_Color")
            .map(crate::entities::vector::<3>)
            .unwrap_or(Some([1.0; 3]));
        let Some(color) = color.map(Vec3::from) else {
            continue;
        };
        let size = p.children().find(|n| n.has_tag_name("Size"));
        let mut dims = [1.0; 3];
        let mut valid = true;
        for (i, name) in ["x", "y", "z"].into_iter().enumerate() {
            match size
                .and_then(|n| n.attribute(name))
                .map(|v| v.parse::<f32>().ok().filter(|v| v.is_finite()))
                .unwrap_or(Some(1.0))
            {
                Some(v) => dims[i] = if v <= 0.0 { 0.01 } else { v },
                None => valid = false,
            }
        }
        if !valid {
            continue;
        }
        let longitude = long.to_radians();
        let polar = (90.0 - lat).to_radians();
        let direction = Vec3::new(
            longitude.cos() * polar.sin(),
            longitude.sin() * polar.sin(),
            polar.cos(),
        );
        let v = FogVolume {
            name: e.attribute("Name").unwrap_or("").to_string(),
            guid: e
                .attribute("EntityGuid")
                .and_then(crate::entities::guid)
                .unwrap_or(0),
            matrix,
            hidden,
            active: active == 1.0,
            box_shape: (kind as i32).clamp(0, 1) == 1,
            size: Vec3::from_array(dims),
            color,
            density: density.max(0.0),
            soft_edges: soft.clamp(0.0, 1.0),
            direction,
            shift,
            falloff,
            global_color: p.attribute("bUseGlobalFogColor").is_some_and(|v| v != "0"),
            hdr_dynamic: hdr,
        };
        if v.params((0, 0), 1.0).local_from_world.is_finite() {
            result.push(v);
        }
    }
    result
}

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct FogParams {
    pub local_from_world: Mat4,
    /// Separate translation avoids cancellation for small volumes at large cell coordinates.
    pub center: Vec4,
    /// Exposed by the shader per view; RGB is raw authored linear input times existing radiance adapter.
    pub color: Vec4,
    /// World-space height-falloff base point; w=authored soft edges.
    pub base: Vec4,
    /// World-space height gradient (not rotated/scaled with entity); w=box rather than ellipsoid.
    pub gradient: Vec4,
}

/// Native analytic ray integration. Direction is normalized world-space, scene_distance in metres.
/// Returns alpha, including the high-quality ellipsoid soft-edge branch.
pub fn opacity(p: &FogParams, eye: Vec3, direction: Vec3, scene_distance: f32) -> f32 {
    if p.color.w == 0.0 {
        return 0.0;
    }
    let e = p
        .local_from_world
        .transform_vector3(eye - p.center.truncate());
    let d = p.local_from_world.transform_vector3(direction);
    let (start, end, discr) = if p.gradient.w > 0.5 {
        let length = d.length();
        let n = d / length;
        let a = (-Vec3::ONE - e) / n;
        let b = (Vec3::ONE - e) / n;
        let lo = a
            .min(b)
            .clamp(Vec3::splat(-10_000_000.0), Vec3::splat(10_000_000.0));
        let hi = a
            .max(b)
            .clamp(Vec3::splat(-10_000_000.0), Vec3::splat(10_000_000.0));
        (
            lo.max_element().max(0.0) / length,
            hi.min_element().max(0.0) / length,
            0.0,
        )
    } else {
        let length = d.length();
        let n = d / length;
        let b = 2.0 * e.dot(n);
        let c = 4.0 * (e.length_squared() - 1.0);
        let discr = b * b - c;
        if discr < 0.0 {
            return 0.0;
        }
        (
            ((-b - discr.sqrt()) * 0.5).max(0.0) / length,
            ((-b + discr.sqrt()) * 0.5).max(0.0) / length,
            discr,
        )
    };
    let end = scene_distance.min(end).max(start);
    let distance = end - start;
    if distance <= 0.0 {
        return 0.0;
    }
    let front = eye + direction * start;
    let gradient = p.gradient.truncate();
    let mut integral = distance * (-(front - p.base.truncate()).dot(gradient)).exp();
    let height = direction.dot(gradient) * distance;
    if height.abs() > 0.001 {
        integral *= (1.0 - (-height).exp()) / height;
    }
    if p.gradient.w < 0.5 {
        let soft = p.base.w * ((e.length() - 0.95) * 20.0).clamp(0.0, 1.0);
        let arg = (discr * soft + 1.0 - soft).clamp(0.0, 1.0);
        integral *= arg * (2.0 - arg);
    }
    1.0 - (-p.color.w * 1.442_695 * integral).exp2()
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FogMaterial {
    #[uniform(0)]
    pub params: FogParams,
}
impl Material for FogMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/fog.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/fog.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        SORT_BIAS
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _p: &MaterialPipeline,
        d: &mut RenderPipelineDescriptor,
        l: &MeshVertexBufferLayoutRef,
        _k: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        d.vertex.buffers = vec![l.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])?];
        d.primitive.cull_mode = Some(Face::Front);
        if let Some(depth) = d.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(false);
            depth.depth_compare = Some(CompareFunction::Always);
            depth.bias.constant = 0;
        }
        for target in d
            .fragment
            .as_mut()
            .into_iter()
            .flat_map(|f| f.targets.iter_mut().flatten())
        {
            target.write_mask = ColorWrites::RED | ColorWrites::GREEN | ColorWrites::BLUE;
        }
        Ok(())
    }
}
pub struct FogVolumePlugin;
impl Plugin for FogVolumePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "fog.wesl");
        bevy::shader::load_shader_library!(app, "fog_math.wesl");
        app.add_plugins(MaterialPlugin::<FogMaterial>::default());
    }
}
#[derive(SystemParam)]
#[doc(hidden)]
pub struct FogAssets<'w> {
    pub materials: ResMut<'w, Assets<FogMaterial>>,
    pub atmosphere: Res<'w, crate::sky::AtmosphereTuning>,
}
#[derive(Component, Clone, Copy, Debug)]
pub struct FogVolumePart {
    pub cell: (i32, i32),
    pub guid: u64,
}

#[derive(Default)]
pub(crate) struct CellVolumes {
    pub entities: Vec<Entity>,
    assets: Vec<(Handle<Mesh>, Handle<FogMaterial>)>,
    pub unsupported: usize,
    pub over_budget: usize,
}
impl CellVolumes {
    pub fn release(
        self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<FogMaterial>,
    ) {
        for entity in self.entities {
            commands.entity(entity).despawn();
        }
        for (mesh, material) in self.assets {
            meshes.remove(mesh.id());
            materials.remove(material.id());
        }
    }
}

/// Native renderer uses the eight bounds corners for either analytic shape.
pub fn mesh(volume: &FogVolume) -> Mesh {
    let half = volume.size * 0.5;
    let positions: Vec<[f32; 3]> = (0..8)
        .map(|i| {
            let v = half
                * Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                );
            cry_to_bevy(volume.matrix.transform_point3(v)).to_array()
        })
        .collect();
    let mut indices = vec![
        0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6,
        1, 3, 5, 3, 7, 5,
    ];
    if volume.matrix.determinant() < 0.0 {
        for t in indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}
pub(crate) fn spawn_cell(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<FogMaterial>,
    cell: (i32, i32),
    volumes: &[FogVolume],
    available: usize,
    radiance: f32,
) -> CellVolumes {
    let mut result = CellVolumes::default();
    for volume in volumes.iter().filter(|v| v.active && !v.hidden) {
        if !volume.supported() {
            result.unsupported += 1;
            continue;
        }
        if result.entities.len() >= available.min(MAX_VOLUMES) {
            result.over_budget += 1;
            continue;
        }
        let mesh = meshes.add(mesh(volume));
        let material = materials.add(FogMaterial {
            params: volume.params(cell, radiance),
        });
        let entity = commands
            .spawn((
                Name::new(format!("FogVolume {}", volume.name)),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(cry_to_bevy(cell_origin(cell.0, cell.1))),
                NotShadowCaster,
                FogVolumePart {
                    cell,
                    guid: volume.guid,
                },
            ))
            .id();
        result.entities.push(entity);
        result.assets.push((mesh, material));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(kind: u8) -> FogVolume {
        parse(&format!(r#"<Mission><Objects><Entity EntityClass="FogVolume" EntityGuid="A" Pos="0,8,0"><Properties VolumeType="{kind}" GlobalDensity="0.2" FallOffScale="0"><Size x="4" y="4" z="4"/></Properties></Entity></Objects></Mission>"#)).remove(0)
    }
    #[test]
    fn native_script_clamps_parent_transform_and_explicit_unsupported_inputs() {
        let xml = r#"<Mission><Objects>
          <Entity EntityGuid="A" Pos="10,20,30" Scale="2,3,4" HiddenInGame="1"/>
          <Entity EntityClass="FogVolume" EntityGuid="B" ParentGUID="A" Pos="1,2,3"><Properties VolumeType="9" GlobalDensity="-1" SoftEdges="100" FallOffDirLati="0"><Size x="0" y="-1" z="4"/></Properties></Entity>
          <Entity EntityClass="FogVolume"><Properties bActive="0" bUseGlobalFogColor="1"/></Entity>
          <Entity EntityClass="FogVolume"><Properties fHDRDynamic="1"/></Entity>
          <Entity EntityClass="FogVolume"><Properties GlobalDensity="NaN"/></Entity>
        </Objects></Mission>"#;
        let v = parse(xml);
        assert_eq!(v.len(), 3);
        assert!(v[0].hidden && v[0].active && v[0].box_shape && v[0].supported());
        assert_eq!(v[0].matrix.w_axis.truncate(), Vec3::new(12.0, 26.0, 42.0));
        assert_eq!(v[0].size, Vec3::new(0.01, 0.01, 4.0));
        assert_eq!(v[0].density, 0.0);
        assert_eq!(v[0].soft_edges, 1.0);
        assert!(v[0].direction.distance(Vec3::X) < 1e-6);
        assert!(!v[1].active && !v[1].supported() && !v[2].supported());
        let p = v[0].params((21, 8), 1.0);
        let center = cry_to_bevy(cell_origin(21, 8) + Vec3::new(12.0, 26.0, 42.0));
        assert!(
            p.local_from_world
                .transform_vector3(center - p.center.truncate())
                .length()
                < 0.001
        );
    }
    #[test]
    fn native_box_and_ellipsoid_depth_inside_miss_and_zero_density() {
        for kind in [0, 1] {
            let v = fixture(kind);
            let mut p = v.params((0, 0), 1.0);
            for (eye, depth, path) in [
                (Vec3::ZERO, 20.0, 4.0),
                (Vec3::ZERO, 6.0, 0.0),
                (Vec3::ZERO, 8.0, 2.0),
                (Vec3::new(0., 0., -8.), 20., 2.),
            ] {
                let alpha = opacity(&p, eye, Vec3::NEG_Z, depth);
                assert!(
                    (alpha - (1.0 - (-0.2f32 * path).exp())).abs() < 2e-6,
                    "{kind} {eye} {depth}: {alpha}"
                );
            }
            assert_eq!(opacity(&p, Vec3::new(8., 0., 0.), Vec3::NEG_Z, 20.), 0.);
            p.color.w = 0.;
            assert_eq!(opacity(&p, Vec3::ZERO, Vec3::NEG_Z, 20.), 0.);
        }
    }
    #[test]
    fn native_gradient_is_world_space_and_integrates_both_signs() {
        let mut v = fixture(1);
        v.matrix = Mat4::from_translation(Vec3::new(0., 8., 0.)) * Mat4::from_rotation_y(1.0);
        v.direction = Vec3::Y;
        v.falloff = 0.2;
        v.shift = 1.;
        let p = v.params((0, 0), 1.);
        assert!(p.gradient.truncate().distance(Vec3::NEG_Z * 0.2) < 1e-6);
        // Independent numerical density integration along each ray, no native closed-form reuse.
        for (eye, d) in [
            (Vec3::ZERO, Vec3::NEG_Z),
            (Vec3::new(0., 0., -16.), Vec3::Z),
        ] {
            let steps = 100_000;
            let dt = 16. / steps as f32;
            let mut integral = 0f64;
            for i in 0..steps {
                let pos = eye + d * ((i as f32 + 0.5) * dt);
                let q = p
                    .local_from_world
                    .transform_vector3(pos - p.center.truncate());
                if q.abs().max_element() <= 1. {
                    integral += f64::from(dt)
                        * f64::from(-(pos - p.base.truncate()).dot(p.gradient.truncate())).exp();
                }
            }
            let cpu = 1. - (-0.2 * integral).exp();
            assert!((f64::from(opacity(&p, eye, d, 16.)) - cpu).abs() < 3e-5);
        }
    }
    #[test]
    fn total_cap_and_unload_release_every_owned_entity_mesh_and_material() {
        use bevy::ecs::world::CommandQueue;
        let mut world = World::new();
        let mut commands = CommandQueue::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<FogMaterial>::default();
        let volumes = vec![fixture(0); 40];
        let first = spawn_cell(
            &mut Commands::new(&mut commands, &world),
            &mut meshes,
            &mut materials,
            (0, 0),
            &volumes,
            20,
            1.,
        );
        let second = spawn_cell(
            &mut Commands::new(&mut commands, &world),
            &mut meshes,
            &mut materials,
            (1, 0),
            &volumes,
            MAX_VOLUMES - first.entities.len(),
            1.,
        );
        assert_eq!(first.entities.len() + second.entities.len(), 32);
        assert_eq!(first.over_budget + second.over_budget, 48);
        commands.apply(&mut world);
        assert_eq!(world.query::<&FogVolumePart>().iter(&world).count(), 32);
        first.release(
            &mut Commands::new(&mut commands, &world),
            &mut meshes,
            &mut materials,
        );
        second.release(
            &mut Commands::new(&mut commands, &world),
            &mut meshes,
            &mut materials,
        );
        commands.apply(&mut world);
        assert_eq!(world.query::<&FogVolumePart>().iter(&world).count(), 0);
        assert_eq!((meshes.len(), materials.len()), (0, 0));
    }
    #[test]
    fn original_cave_shapes_and_nonlinear_soft_edge_clamp_are_authored() {
        if !crate::pak_available() {
            return;
        }
        let read = |cell| {
            aa_pak::Pak::global()
                .read_string(&format!(
                    "worlds/main_world/cells/{cell}/client/entities.xml"
                ))
                .unwrap()
        };
        let a = parse(&read("021_008"));
        assert_eq!(a.len(), 2);
        assert!(a.iter().all(|v| v.active && !v.hidden && v.supported()));
        assert!(a[0].box_shape);
        assert_eq!(a[0].size, Vec3::new(60., 60., 40.));
        assert_eq!(a[0].falloff, 0.);
        assert_eq!(a[0].density, 0.01);
        assert!(!a[1].box_shape);
        assert_eq!(a[1].size, Vec3::new(100., 100., 20.));
        assert_eq!(a[1].falloff, 1.);
        let b = parse(&read("020_008"));
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].soft_edges, 1.);
        assert_eq!(b[0].density, 0.0001);
    }
}
