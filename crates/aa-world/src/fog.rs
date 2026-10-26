//! Static authored FogVolume slots. Native CFogVolumeRenderNode and fogvolume.cfx.
//! Authored local/global RGB, HDRDynamic=0; no FlowGraph density adapter.
use std::collections::{HashMap, HashSet};

use crate::coords::{CRY_TO_BEVY, cell_origin, cry_to_bevy};
use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    core_pipeline::core_3d::Transparent3d,
    ecs::{
        query::ROQueryItem,
        system::{SystemParam, SystemParamItem, lifetimeless::SRes},
    },
    light::NotShadowCaster,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{DrawMaterial, Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    render::render_resource::{
        AsBindGroup, ColorWrites, CompareFunction, Face, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError,
    },
    render::{
        Extract, ExtractSchedule, Render, RenderApp, RenderSystems,
        batching::NoAutomaticBatching,
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, RenderCommand, RenderCommandResult,
            TrackedRenderPass, ViewSortedRenderPhases,
        },
        sync_world::{MainEntity, MainEntityHashMap},
        view::ExtractedView,
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
    /// Initial scene XML render-proxy pass runs after OnInit creates the node.
    /// Native3207B420/3001A990 clamp this byte; 255 means1000 in GetMaxViewDist.
    pub view_dist_ratio: u8,
}
impl FogVolume {
    pub fn supported(&self) -> bool {
        self.hdr_dynamic == 0.0 && (self.global_color || self.color.min_element() >= 0.0)
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
            // A global volume must wait for the native environment input; its
            // authored local colour is ignored by CFogVolume::GetFogColour.
            color: if self.global_color {
                Vec4::ZERO
            } else {
                (self.color * radiance).extend(self.density)
            },
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
        let Some(view_dist_ratio) = e
            .attribute("ViewDistRatio")
            .map(|s| s.parse::<i32>().ok().map(|n| n.clamp(0, 255) as u8))
            .unwrap_or(Some(100))
        else {
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
            view_dist_ratio,
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
    1.0 - (-p.color.w * std::f32::consts::LOG2_E * integral).exp2()
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct FogMaterial {
    #[uniform(0)]
    pub params: FogParams,
    // CPU draw metadata, deliberately absent from the shader bind group.
    distance: FogDistance,
}
impl FogMaterial {
    pub fn new(volume: &FogVolume, cell: (i32, i32), radiance: f32) -> Self {
        Self {
            params: volume.params(cell, radiance),
            distance: FogDistance::new(volume, cell),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct FogDistance {
    /// Cry axes retain the native scalar accumulation order (Bevy swaps Y/Z).
    bounds: [Vec3; 2],
    ratio: u8,
}
impl FogDistance {
    fn new(volume: &FogVolume, cell: (i32, i32)) -> Self {
        let m = volume.matrix;
        let h = volume.size * 0.5;
        // Whole original helper300B54F0, symmetric local bounds. Preserve its
        // abs-matrix product/add grouping rather than enumerating corners.
        let extent = Vec3::new(
            (m.y_axis.x.abs() * h.y + m.x_axis.x.abs() * h.x) + m.z_axis.x.abs() * h.z,
            (m.y_axis.y.abs() * h.y + m.x_axis.y.abs() * h.x) + m.z_axis.y.abs() * h.z,
            (m.y_axis.z.abs() * h.y + m.x_axis.z.abs() * h.x) + m.z_axis.z.abs() * h.z,
        );
        let center = cell_origin(cell.0, cell.1) + m.w_axis.truncate();
        Self {
            bounds: [center - extent, center + extent],
            ratio: volume.view_dist_ratio,
        }
    }

    fn maximum(self, general: f32) -> Option<f32> {
        let [lo, hi] = self.bounds;
        if !lo.is_finite() || !hi.is_finite() || !general.is_finite() {
            return None;
        }
        let radius = if lo.x > hi.x {
            0.0
        } else {
            let d = hi - lo;
            ((d.y * d.y + d.x * d.x) + d.z * d.z).sqrt() * 0.5
        };
        let logical = if self.ratio == 255 {
            1000
        } else {
            i32::from(self.ratio)
        };
        extended_maximum(general, radius, logical)
    }
}

fn extended_maximum(general: f32, radius: f32, ratio: i32) -> Option<f32> {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        // Native300B6A6C..300B6A89 retains the products on x87 until the f32
        // cache store. CSystem36558783 selects extended precision. Save/restore
        // the caller's CW; two pushes and two pops leave the FPU stack balanced.
        // Packed final word contains savedCW low16 / nativeCW high16.
        let mut values = [
            general.to_bits(),
            radius.to_bits(),
            ratio as u32,
            0.01_f32.to_bits(),
            0,
            0x037f_0000,
        ];
        // SAFETY: the six-word local array supplies every referenced address;
        // the assembly neither calls code nor changes the CPU stack pointer.
        unsafe {
            core::arch::asm!(
                "fnstcw word ptr [{v} + 20]",
                "fldcw word ptr [{v} + 22]",
                "fld dword ptr [{v}]",
                "fmul dword ptr [{v} + 4]",
                "fild dword ptr [{v} + 8]",
                "fmul dword ptr [{v} + 12]",
                "fmulp st(1), st(0)",
                "fstp dword ptr [{v} + 16]",
                "fldcw word ptr [{v} + 20]",
                v = in(reg) values.as_mut_ptr(),
                out("st(0)") _, out("st(1)") _,
                options(nostack, preserves_flags),
            );
        }
        Some(f32::from_bits(values[4]).max(0.0)) // proved native minimum0 / engine factor1
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        let _ = (general, radius, ratio);
        None // Preserve the existing material path without a guessed f64 replacement.
    }
}

fn scaled_distance(bounds: [Vec3; 2], eye: Vec3, fov: f32) -> f32 {
    let [lo, hi] = bounds;
    // Original301EDBFA..301EDCBC, scalar Cry X/Y/Z order, no SIMD dot/FMA.
    let delta = (lo - eye).max(Vec3::ZERO).max(eye - hi);
    let distance = ((delta.x * delta.x + delta.y * delta.y) + delta.z * delta.z).sqrt();
    let scale = ((fov * 57.295776 * 0.016666668) * 0.8) + 0.2;
    distance * scale
}

#[derive(Clone, Copy)]
struct FogDraw {
    bounds: [Vec3; 2],
    maximum: Option<f32>,
}
#[derive(Resource, Default)]
struct FogDraws(MainEntityHashMap<FogDraw>);
impl FogDraws {
    fn admitted(
        &self,
        entity: MainEntity,
        view: &ExtractedView,
        projection: Option<&Projection>,
    ) -> bool {
        // These views stay outside the new mapping. Their existing DrawMaterial
        // and fog kernel behavior is retained, including the kernel's ortho gate.
        let Some(Projection::Perspective(p)) = projection else {
            return true;
        };
        let eye = view.world_from_view.translation();
        if view.invert_culling
            || !eye.is_finite()
            || !p.fov.is_finite()
            || p.fov <= 0.0
            || p.fov >= std::f32::consts::PI
        {
            return true;
        }
        let Some(draw) = self.0.get(&entity) else {
            return false;
        }; // retired generation/material
        let Some(maximum) = draw.maximum else {
            return true;
        }; // unproved arithmetic input/platform
        scaled_distance(draw.bounds, crate::coords::bevy_to_cry(eye), p.fov) < maximum
    }
}

struct CheckFogDistance;
type DrawFog = (CheckFogDistance, DrawMaterial);
impl RenderCommand<Transparent3d> for CheckFogDistance {
    type Param = SRes<FogDraws>;
    type ViewQuery = (&'static ExtractedView, Option<&'static Projection>);
    type ItemQuery = ();
    fn render<'w>(
        item: &Transparent3d,
        (view, projection): ROQueryItem<'w, '_, Self::ViewQuery>,
        _: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        draws: SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        if draws
            .into_inner()
            .admitted(item.main_entity(), view, projection)
        {
            RenderCommandResult::Success
        } else {
            RenderCommandResult::Skip
        }
    }
}

fn extract_fog_draws(
    mut draws: ResMut<FogDraws>,
    config: Extract<Option<Res<crate::WorldConfig>>>,
    instances: Extract<Query<(Entity, &MeshMaterial3d<FogMaterial>)>>,
    materials: Extract<Res<Assets<FogMaterial>>>,
) {
    let general = config.as_ref().map_or(60.0, |c| c.view_dist_scale * 40.0);
    draws.0.clear();
    for (entity, handle) in instances.iter().take(MAX_VOLUMES) {
        if let Some(material) = materials.get(&handle.0) {
            draws.0.insert(
                entity.into(),
                FogDraw {
                    bounds: material.distance.bounds,
                    maximum: material.distance.maximum(general),
                },
            );
        }
    }
}

fn install_distance_draw(
    draws: Res<FogDraws>,
    functions: Res<DrawFunctions<Transparent3d>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
) {
    let draw = functions.read().id::<DrawFog>();
    for phase in phases.values_mut() {
        for item in phase.items.values_mut() {
            if draws.0.contains_key(&item.main_entity()) {
                item.draw_function = draw;
            }
        }
    }
}

fn unbatch_fog(
    mut commands: Commands,
    nodes: Query<
        Entity,
        (
            With<MeshMaterial3d<FogMaterial>>,
            Without<NoAutomaticBatching>,
        ),
    >,
) {
    for entity in &nodes {
        commands.entity(entity).insert(NoAutomaticBatching);
    }
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
        app.add_plugins((
            MaterialPlugin::<FogMaterial>::default(),
            ExtractComponentPlugin::<FogWaterPass>::default(),
        ))
        .add_systems(
            Update,
            refresh_global_color.after(crate::sky::update_environment),
        )
        .add_systems(PostUpdate, unbatch_fog);
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render
                .init_resource::<FogDraws>()
                .add_render_command::<Transparent3d, DrawFog>()
                .add_systems(ExtractSchedule, extract_fog_draws)
                .add_systems(
                    Render,
                    install_distance_draw
                        .after(RenderSystems::QueueMeshes)
                        .before(RenderSystems::PhaseSort),
                );
        }
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

/// Static authored node centre for native normal-pass water-half selection.
/// Cry 300B5B60 stores the matrix translation; this is not a bounds corner or falloff base.
#[derive(Component, Clone, Copy, Debug, ExtractComponent)]
#[extract_app(RenderApp)]
#[require(NoAutomaticBatching)]
pub struct FogWaterPass {
    pub center_height: f32,
}

pub(crate) fn after_water(camera_height: f32, ocean_height: f32, center_height: f32) -> bool {
    ((camera_height - ocean_height) * 0.5) * (center_height - ocean_height) > 0.0
}

/// Retains authored density while a global-colour volume waits for environment data.
/// Materials are unique to their volume; streaming owns at most [`MAX_VOLUMES`].
#[derive(Component, Clone, Copy, Debug)]
pub struct GlobalFogColor {
    pub density: f32,
}

fn refresh_global_color(
    env: Option<Res<crate::sky::Environment>>,
    tuning: Option<Res<crate::sky::AtmosphereTuning>>,
    volumes: Query<(&GlobalFogColor, &MeshMaterial3d<FogMaterial>)>,
    mut materials: ResMut<Assets<FogMaterial>>,
) {
    let color = env
        .as_ref()
        .and_then(|e| e.sample.as_ref())
        .zip(tuning.as_ref())
        .map(|(sample, tuning)| sample.native_fog * tuning.radiance);
    for (global, handle) in volumes.iter().take(MAX_VOLUMES) {
        let target = color.map_or(Vec4::ZERO, |c| c.extend(global.density));
        if let Some(mut material) = materials.get_mut(&handle.0) {
            if material.params.color != target {
                material.params.color = target;
            }
        }
    }
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

    /// Like [`CellVolumes::release`] but directly on a [`World`], for a runtime world switch.
    pub(crate) fn release_world(&mut self, world: &mut World) {
        for entity in self.entities.drain(..) {
            if world.get_entity(entity).is_ok() {
                world.despawn(entity);
            }
        }
        for (mesh, material) in &self.assets {
            if let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() {
                meshes.remove(mesh.id());
            }
            if let Some(mut materials) = world.get_resource_mut::<Assets<FogMaterial>>() {
                materials.remove(material.id());
            }
        }
        self.assets.clear();
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
        let params = volume.params(cell, radiance);
        let material = materials.add(FogMaterial::new(volume, cell, radiance));
        let mut entity = commands.spawn((
            Name::new(format!("FogVolume {}", volume.name)),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(cry_to_bevy(cell_origin(cell.0, cell.1))),
            NotShadowCaster,
            FogVolumePart {
                cell,
                guid: volume.guid,
            },
            FogWaterPass {
                center_height: params.center.y,
            },
        ));
        if volume.global_color {
            entity.insert(GlobalFogColor {
                density: volume.density,
            });
        }
        result.entities.push(entity.id());
        result.assets.push((mesh, material));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        app::SubApp,
        camera::CameraProjection,
        ecs::world::CommandQueue,
        render::{
            MainWorld,
            render_phase::PhaseItemExtraIndex,
            render_resource::{CachedRenderPipelineId, TextureFormat},
            view::RetainedViewEntity,
        },
    };

    fn render_view(eye: Vec3, fov: f32) -> ExtractedView {
        ExtractedView {
            retained_view_entity: RetainedViewEntity::new(Entity::PLACEHOLDER.into(), None, 0),
            clip_from_view: PerspectiveProjection {
                fov,
                aspect_ratio: 640.0 / 720.0,
                ..default()
            }
            .get_clip_from_view(),
            world_from_view: GlobalTransform::from_translation(cry_to_bevy(eye)),
            clip_from_world: None,
            target_format: TextureFormat::Rgba8Unorm,
            viewport: UVec4::new(0, 0, 640, 720),
            color_grading: default(),
            invert_culling: false,
        }
    }

    // Real streaming spawn, current Assets, actual extraction system and the
    // real installed DrawFog type. No GPU draw is claimed by these CPU tests.
    fn render_cell(volume: &FogVolume) -> (SubApp, CellVolumes) {
        let mut main = World::new();
        let mut queue = CommandQueue::default();
        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<FogMaterial>::default();
        let cells = spawn_cell(
            &mut Commands::new(&mut queue, &main),
            &mut meshes,
            &mut materials,
            (0, 0),
            std::slice::from_ref(volume),
            1,
            1.0,
        );
        queue.apply(&mut main);
        // The raw CPU World does not install the render plugins' visibility
        // components. Supply the actual inherited state whose preservation is tested.
        for entity in &cells.entities {
            main.entity_mut(*entity).insert(Visibility::Inherited);
        }
        main.insert_resource(meshes);
        main.insert_resource(materials);
        main.insert_resource(crate::WorldConfig::new("main_world"));
        let mut wrapped = MainWorld::default();
        *wrapped = main;
        let mut render = SubApp::new();
        render
            .insert_resource(wrapped)
            .init_resource::<FogDraws>()
            .init_resource::<DrawFunctions<Transparent3d>>()
            .init_resource::<ViewSortedRenderPhases<Transparent3d>>()
            .add_render_command::<Transparent3d, DrawMaterial>()
            .add_render_command::<Transparent3d, DrawFog>()
            .add_schedule(Schedule::new(ExtractSchedule))
            .add_schedule(Schedule::new(Render))
            .add_systems(ExtractSchedule, extract_fog_draws)
            .add_systems(Render, install_distance_draw);
        (render, cells)
    }

    #[test]
    fn initial_xml_ratio_clamp_defaults_reach_real_streamed_draw_data() {
        for (attribute, ratio) in [
            ("", 100),
            ("ViewDistRatio=\"10\"", 10),
            ("ViewDistRatio=\"-2\"", 0),
            ("ViewDistRatio=\"255\"", 255),
            ("ViewDistRatio=\"999\"", 255),
        ] {
            let xml = format!(
                r#"<Mission><Objects><Entity EntityClass="FogVolume" {attribute}><Properties><Size x="60" y="60" z="40"/></Properties></Entity></Objects></Mission>"#
            );
            let volume = parse(&xml).remove(0);
            assert_eq!(volume.view_dist_ratio, ratio);
            let (mut app, cells) = render_cell(&volume);
            app.world_mut().run_schedule(ExtractSchedule);
            let native = app
                .world()
                .resource::<FogDraws>()
                .0
                .get(&MainEntity::from(cells.entities[0]))
                .unwrap();
            let expected: f32 = match ratio {
                0 => 0.0,
                10 => 281.424_93,
                100 => 2814.249_3,
                _ => f32::from_bits(1188814076), // native-r3 sentinel/over-clamp, not rounded decimal
            };
            assert_eq!(native.maximum.unwrap().to_bits(), expected.to_bits());
        }
        for invalid in ["NaN", "10.5", "2147483648"] {
            assert!(parse(&format!(r#"<Mission><Objects><Entity EntityClass="FogVolume" ViewDistRatio="{invalid}"><Properties/></Entity></Objects></Mission>"#)).is_empty());
        }
    }

    #[test]
    fn per_view_installed_draw_live_config_handles_generation_and_cell_release() {
        let mut volume = fixture(1);
        volume.size = Vec3::new(60.0, 60.0, 40.0);
        volume.matrix = Mat4::IDENTITY;
        volume.view_dist_ratio = 10;
        let (mut render, mut cells) = render_cell(&volume);
        let entity = cells.entities[0];
        let (mesh, material) = {
            let main = render.world().resource::<MainWorld>();
            assert!(main.get::<NoAutomaticBatching>(entity).is_some());
            (
                main.get::<Mesh3d>(entity).unwrap().0.id(),
                main.get::<MeshMaterial3d<FogMaterial>>(entity)
                    .unwrap()
                    .0
                    .id(),
            )
        };
        let near = render_view(Vec3::new(0.0, -220.0, 0.0), 50_f32.to_radians());
        let far = render_view(Vec3::new(0.0, -450.0, 0.0), 50_f32.to_radians());
        let projection = Projection::Perspective(PerspectiveProjection {
            fov: 50_f32.to_radians(),
            ..default()
        });
        for (scale, expected) in [
            (1.5, [true, false]),
            (0.15, [false, false]),
            (3.0, [true, true]),
            (1.5, [true, false]),
        ] {
            render
                .world_mut()
                .resource_mut::<MainWorld>()
                .resource_mut::<crate::WorldConfig>()
                .view_dist_scale = scale;
            render.world_mut().run_schedule(ExtractSchedule);
            let draws = render.world().resource::<FogDraws>();
            assert_eq!(
                [
                    draws.admitted(entity.into(), &near, Some(&projection)),
                    draws.admitted(entity.into(), &far, Some(&projection))
                ],
                expected
            );
            let main = render.world().resource::<MainWorld>();
            assert_eq!(main.get::<Mesh3d>(entity).unwrap().0.id(), mesh);
            assert_eq!(
                main.get::<MeshMaterial3d<FogMaterial>>(entity)
                    .unwrap()
                    .0
                    .id(),
                material
            );
            assert!(main.resource::<Assets<Mesh>>().contains(mesh));
            assert!(main.resource::<Assets<FogMaterial>>().contains(material));
            assert_ne!(*main.get::<Visibility>(entity).unwrap(), Visibility::Hidden);
        }
        let draw_id = render
            .world()
            .resource::<DrawFunctions<Transparent3d>>()
            .read()
            .id::<DrawFog>();
        let original_draw = render
            .world()
            .resource::<DrawFunctions<Transparent3d>>()
            .read()
            .id::<DrawMaterial>();
        assert_ne!(original_draw, draw_id);
        for index in 0..2 {
            let key =
                RetainedViewEntity::new(render.world_mut().spawn_empty().id().into(), None, 0);
            let item = Transparent3d {
                sorting_info: bevy::core_pipeline::core_3d::TransparentSortingInfo3d::Sorted {
                    mesh_center: Vec3::ZERO,
                    depth_bias: SORT_BIAS,
                },
                distance: 1.0,
                pipeline: CachedRenderPipelineId::INVALID,
                entity: (Entity::PLACEHOLDER, entity.into()),
                draw_function: original_draw,
                batch_range: 40 + index..41 + index,
                extra_index: PhaseItemExtraIndex::DynamicOffset(1536),
                indexed: true,
            };
            render
                .world_mut()
                .resource_mut::<ViewSortedRenderPhases<Transparent3d>>()
                .entry(key)
                .or_default()
                .add_retained(item);
        }
        render.world_mut().run_schedule(Render);
        for phase in render
            .world()
            .resource::<ViewSortedRenderPhases<Transparent3d>>()
            .values()
        {
            let item = &phase.items[&(Entity::PLACEHOLDER, entity.into())];
            assert_eq!(item.draw_function, draw_id);
            assert_eq!(item.batch_range.len(), 1); // distance never globally suppresses ranges
            assert!(matches!(
                item.extra_index,
                PhaseItemExtraIndex::DynamicOffset(1536)
            ));
        }
        {
            let mut main = render.world_mut().resource_mut::<MainWorld>();
            cells.release_world(&mut main);
            assert!(main.get_entity(entity).is_err());
            assert!(!main.resource::<Assets<Mesh>>().contains(mesh));
            assert!(!main.resource::<Assets<FogMaterial>>().contains(material));
        }
        render.world_mut().run_schedule(ExtractSchedule);
        assert!(render.world().resource::<FogDraws>().0.is_empty());
        assert!(!render.world().resource::<FogDraws>().admitted(
            entity.into(),
            &near,
            Some(&projection)
        ));
        let replacement = {
            let mut main = render.world_mut().resource_mut::<MainWorld>();
            // Bevy0.20 remote_allocator.rs buffers128 freed IDs. Publish that
            // buffer through real World lifecycles, then consume actual reused
            // allocations. No immediate-reuse or fabricated-generation assumption.
            let filler: Vec<_> = (0..128).map(|_| main.spawn_empty().id()).collect();
            for id in filler {
                assert!(main.despawn(id));
            }
            (0..256)
                .map(|_| main.spawn_empty().id())
                .find(|id| id.index() == entity.index())
                .expect("actual freed index must be reused")
        };
        assert_ne!(replacement, entity);
        assert_eq!(replacement.index(), entity.index());
        assert!(
            !render
                .world()
                .resource::<FogDraws>()
                .0
                .contains_key(&MainEntity::from(replacement))
        );
        {
            let mut main = render.world_mut().resource_mut::<MainWorld>();
            let mesh = main
                .resource_mut::<Assets<Mesh>>()
                .add(super::mesh(&volume));
            let material = main
                .resource_mut::<Assets<FogMaterial>>()
                .add(FogMaterial::new(&volume, (0, 0), 1.0));
            main.entity_mut(replacement)
                .insert((Mesh3d(mesh), MeshMaterial3d(material)));
        }
        render.world_mut().run_schedule(ExtractSchedule);
        let draws = render.world().resource::<FogDraws>();
        assert!(draws.admitted(replacement.into(), &near, Some(&projection)));
        assert!(!draws.admitted(entity.into(), &near, Some(&projection)));
    }

    #[test]
    fn strict_native_boundary_actual_fov_and_unmapped_view_pass_through() {
        let mut volume = fixture(1);
        volume.matrix = Mat4::IDENTITY;
        volume.size = Vec3::new(2.0, 0.01, 0.01);
        volume.view_dist_ratio = 100;
        let material = FogMaterial::new(&volume, (0, 0), 1.0);
        let maximum = material.distance.maximum(60.0).unwrap();
        let mut draws = FogDraws::default();
        let id: MainEntity = Entity::PLACEHOLDER.into();
        draws.0.insert(
            id,
            FogDraw {
                bounds: material.distance.bounds,
                maximum: Some(maximum),
            },
        );
        let at = 1.0 + maximum;
        let projection = Projection::Perspective(PerspectiveProjection {
            fov: 60_f32.to_radians(),
            ..default()
        });
        for (bits, expected) in [
            (at.to_bits() - 1, true),
            (at.to_bits(), false),
            (at.to_bits() + 1, false),
        ] {
            let eye = Vec3::new(f32::from_bits(bits), 0.0, 0.0);
            let view = render_view(eye, 60_f32.to_radians());
            assert_eq!(draws.admitted(id, &view, Some(&projection)), expected);
        }
        let mut far = render_view(Vec3::splat(100000.0), 60_f32.to_radians());
        assert!(!draws.admitted(id, &far, Some(&projection)));
        far.invert_culling = true;
        assert!(draws.admitted(id, &far, Some(&projection)));
        far.invert_culling = false;
        assert!(draws.admitted(id, &far, None));
        assert!(draws.admitted(
            id,
            &far,
            Some(&Projection::custom(PerspectiveProjection::default()))
        ));
        assert!(draws.admitted(
            id,
            &far,
            Some(&Projection::Orthographic(
                OrthographicProjection::default_3d()
            ))
        ));
        let bounds = [Vec3::new(-30.0, -30.0, -20.0), Vec3::new(30.0, 30.0, 20.0)];
        // Original native-r3: swapping CryY/Z accumulation loses this boundary bit.
        assert_eq!(
            scaled_distance(bounds, Vec3::new(31.0, 4126.0, 21.5), 60_f32.to_radians()).to_bits(),
            4096_f32.to_bits()
        );
        draws.0.insert(
            id,
            FogDraw {
                bounds,
                maximum: Some(281.424_93),
            },
        );
        for (fov, expected) in [(30.0_f32, true), (80.0_f32, false)] {
            let view = render_view(Vec3::new(0.0, -400.0, 0.0), fov.to_radians());
            let p = Projection::Perspective(PerspectiveProjection {
                fov: fov.to_radians(),
                ..default()
            });
            assert_eq!(draws.admitted(id, &view, Some(&p)), expected);
        }
    }

    #[test]
    fn actual_authored_fog4_final_ratio_and_parent_cell_bounds() {
        if !crate::pak_available() {
            return;
        }
        let xml = aa_pak::Pak::global()
            .read_string("worlds/main_world/cells/021_008/client/entities.xml")
            .unwrap();
        let volume = parse(&xml)
            .into_iter()
            .find(|v| v.guid == 0x4133521fcc7b527e)
            .unwrap();
        let material = FogMaterial::new(&volume, (21, 8), 1.0);
        assert_eq!(volume.view_dist_ratio, 10);
        assert_eq!(
            material.distance.bounds,
            [
                Vec3::new(22034.125, 8346.248, 427.38528),
                Vec3::new(22094.125, 8406.248, 467.38528)
            ]
        );
        assert_eq!(
            material.distance.maximum(60.0).unwrap().to_bits(),
            1133295204
        );
        let child = parse(r#"<Mission><Objects><Entity EntityGuid="A" Pos="10,20,30" Scale="2,3,-4"/><Entity EntityGuid="B" ParentGUID="A" Pos="1,2,3" EntityClass="FogVolume" ViewDistRatio="10"><Properties><Size x="4" y="6" z="8"/></Properties></Entity></Objects></Mission>"#).remove(0);
        let material = FogMaterial::new(&child, (21, 8), 1.0);
        assert_eq!(
            material.distance.bounds,
            [
                Vec3::new(21512.0, 8209.0, 2.0),
                Vec3::new(21520.0, 8227.0, 34.0)
            ]
        );
    }
    #[test]
    fn original_x86_water_halves_use_matrix_translation_and_strict_boundary() {
        // Original 301121C0, normal pass ID zero. Its caller supplies node+208,
        // written directly from the Matrix34 translation by 300B5B60.
        for (eye, center, after) in [
            (90.0, 90.0, true),
            (90.0, 100.0, false),
            (90.0, 110.0, false),
            (100.0, 90.0, false),
            (100.0, 100.0, false),
            (100.0, 110.0, false),
            (110.0, 90.0, false),
            (110.0, 100.0, false),
            (110.0, 110.0, true),
        ] {
            assert_eq!(after_water(eye, 100.0, center), after);
        }
        let mut volume = fixture(1);
        volume.matrix = Mat4::from_translation(Vec3::new(299.0, 308.0, 117.0))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_scale(Vec3::new(-2.0, 3.0, 4.0));
        volume.direction = Vec3::Z;
        volume.shift = -80.0;
        let params = volume.params((12, 10), 1.0);
        assert_eq!(params.center.y, 117.0);
        assert_eq!(params.base.y, 37.0);
        assert!(after_water(130.0, 100.0, params.center.y));
        assert!(!after_water(130.0, 100.0, params.base.y));
    }
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
        assert!(!v[1].active && v[1].supported() && !v[2].supported());
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
        let mut volumes = vec![fixture(0); 40];
        for volume in volumes.iter_mut().step_by(2) {
            volume.global_color = true;
        }
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
        assert_eq!(world.query::<&FogWaterPass>().iter(&world).count(), 32);
        for entity in first.entities.iter().chain(&second.entities) {
            let handle = world.get::<MeshMaterial3d<FogMaterial>>(*entity).unwrap();
            let params = materials.get(handle.id()).unwrap().params;
            assert_eq!(
                world.get::<FogWaterPass>(*entity).unwrap().center_height,
                params.center.y
            );
            assert!(world.get::<NoAutomaticBatching>(*entity).is_some());
        }
        assert_eq!(world.query::<&GlobalFogColor>().iter(&world).count(), 16);
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
        assert_eq!(world.query::<&FogWaterPass>().iter(&world).count(), 0);
        assert_eq!(world.query::<&GlobalFogColor>().iter(&world).count(), 0);
        assert_eq!((meshes.len(), materials.len()), (0, 0));
    }
    #[test]
    fn global_input_absence_restore_recolor_preserves_density_local_color_and_assets() {
        let mut app = App::new();
        app.init_resource::<Assets<FogMaterial>>()
            .insert_resource(crate::sky::AtmosphereTuning {
                radiance: 2.0,
                ..default()
            })
            .add_systems(Update, refresh_global_color);
        let mut volume = fixture(1);
        let local = app
            .world_mut()
            .resource_mut::<Assets<FogMaterial>>()
            .add(FogMaterial::new(&volume, (0, 0), 2.0));
        let local_color = volume.params((0, 0), 2.0).color;
        volume.global_color = true;
        let global = app
            .world_mut()
            .resource_mut::<Assets<FogMaterial>>()
            .add(FogMaterial::new(&volume, (0, 0), 2.0));
        app.world_mut().spawn(MeshMaterial3d(local.clone()));
        let entity = app
            .world_mut()
            .spawn((
                MeshMaterial3d(global.clone()),
                GlobalFogColor {
                    density: volume.density,
                },
                Visibility::Hidden,
            ))
            .id();
        app.update();
        let read = |app: &App| {
            app.world()
                .resource::<Assets<FogMaterial>>()
                .get(&global)
                .unwrap()
                .params
                .color
        };
        assert_eq!(read(&app), Vec4::ZERO);
        let mut sample =
            crate::env::TimeOfDay::default().sample(10.0, crate::env::Lighting::default());
        sample.native_fog = Vec3::new(0.1, 0.3, 0.5);
        app.insert_resource(crate::sky::Environment {
            sample: Some(sample.clone()),
            zone: Some(179),
            hour: 10.0,
            ocean_fog: Vec3::ZERO,
            ocean_density: 0.0,
        });
        app.update();
        assert_eq!(read(&app), Vec4::new(0.2, 0.6, 1.0, 0.2));
        sample.native_fog = Vec3::new(0.2, 0.1, 0.0);
        {
            let mut env = app.world_mut().resource_mut::<crate::sky::Environment>();
            env.zone = Some(180);
            env.sample = Some(sample);
        }
        app.update();
        assert_eq!(read(&app), Vec4::new(0.4, 0.2, 0.0, 0.2));
        let sample = app
            .world_mut()
            .resource_mut::<crate::sky::Environment>()
            .sample
            .take();
        app.update();
        assert_eq!(read(&app), Vec4::ZERO);
        app.world_mut()
            .resource_mut::<crate::sky::Environment>()
            .sample = sample;
        app.update();
        assert_eq!(read(&app), Vec4::new(0.4, 0.2, 0.0, 0.2));
        let materials = app.world().resource::<Assets<FogMaterial>>();
        assert_eq!(materials.len(), 2);
        assert_eq!(materials.get(&local).unwrap().params.color, local_color);
        assert_eq!(
            *app.world().get::<Visibility>(entity).unwrap(),
            Visibility::Hidden
        );
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
        let c = parse(&read("012_010"));
        assert_eq!(c.len(), 1);
        let v = &c[0];
        assert_eq!(v.guid, 0x403db27ffb2191af);
        assert!(v.global_color && v.supported() && v.active && !v.hidden && !v.box_shape);
        assert_eq!(v.size, Vec3::new(200., 300., 30.));
        assert_eq!(v.density, 0.07);
        assert_eq!(v.soft_edges, 1.);
        assert_eq!(v.params((12, 10), 540.).color, Vec4::ZERO);
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn original_bounds_helper_and_x87_maximum_bits() {
        // tools/fog_distance_native.py executes whole300B54F0 and the actual
        // octree virtual getter/store. Native-r3 SHA b025d69c1e381e7b01590a2c64102a1df05a95387267040078a9b9f264f964a2.
        // These are declared post-parent matrix fixtures, not a native parent-loader claim.
        let cases: &[(&str, [[f32; 4]; 3], u8, f32, [u32; 6], u32)] = &[
            (
                "fog4_quarter_turn_world_control",
                [
                    [0.0, 1.0, 0.0, 22064.125],
                    [-1.0, 0.0, 0.0, 8376.24854],
                    [0.0, 0.0, 1.0, 447.38528],
                ],
                10,
                60.0,
                [
                    1185686592, 1174563071, 1138078033, 1185717312, 1174624511, 1139388753,
                ],
                1133295204,
            ),
            (
                "fog4_legacy100_control",
                [
                    [0.0, 1.0, 0.0, 22064.125],
                    [-1.0, 0.0, 0.0, 8376.24854],
                    [0.0, 0.0, 1.0, 447.38528],
                ],
                100,
                60.0,
                [
                    1185686592, 1174563071, 1138078033, 1185717312, 1174624511, 1139388753,
                ],
                1160766461,
            ),
            (
                "origin",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                10,
                60.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                1133295204,
            ),
            (
                "parent_rotation_nonuniform",
                [
                    [0.0, -3.0, 0.0, 100.0],
                    [2.0, 0.0, 0.0, 200.0],
                    [0.0, 0.0, 4.0, 300.0],
                ],
                10,
                60.0,
                [
                    1092616192, 1124859904, 1130102784, 1128136704, 1132593152, 1136525312,
                ],
                1145687530,
            ),
            (
                "parent_mirror_shear",
                [
                    [1.0, 0.25, -0.125, 100.0],
                    [0.5, -2.0, 0.25, 200.0],
                    [0.0, 0.5, 4.0, 300.0],
                ],
                10,
                60.0,
                [
                    1114636288, 1123024896, 1129119744, 1124859904, 1133248512, 1137016832,
                ],
                1145288775,
            ),
            (
                "parent_precise_fraction",
                [
                    [0.73, -1.11, 0.29, 22064.125],
                    [1.43, 0.27, -0.51, 8376.24854],
                    [-0.19, 0.87, 1.33, 447.38528],
                ],
                10,
                60.0,
                [
                    1185670720, 1174531122, 1136819742, 1185733184, 1174656460, 1140647044,
                ],
                1142714496,
            ),
            (
                "zero",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                0,
                60.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                0,
            ),
            (
                "negative_clamp",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                0,
                60.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                0,
            ),
            (
                "sentinel",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                255,
                60.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                1188814076,
            ),
            (
                "over_clamp",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                255,
                60.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                1188814076,
            ),
            (
                "live_general6",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                10,
                6.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                1105273811,
            ),
            (
                "live_general120",
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                10,
                120.0,
                [
                    3253731328, 3253731328, 3248488448, 1106247680, 1106247680, 1101004800,
                ],
                1141683812,
            ),
        ];
        for &(label, m, ratio, general, expected_bounds, expected_max) in cases {
            let mut volume = fixture(1);
            volume.size = Vec3::new(60.0, 60.0, 40.0);
            volume.view_dist_ratio = ratio;
            volume.matrix = Mat4::from_cols(
                Vec4::new(m[0][0], m[1][0], m[2][0], 0.0),
                Vec4::new(m[0][1], m[1][1], m[2][1], 0.0),
                Vec4::new(m[0][2], m[1][2], m[2][2], 0.0),
                Vec4::new(m[0][3], m[1][3], m[2][3], 1.0),
            );
            let distance = FogMaterial::new(&volume, (0, 0), 1.0).distance;
            let [lo, hi] = distance.bounds;
            assert_eq!(
                [lo.x, lo.y, lo.z, hi.x, hi.y, hi.z].map(f32::to_bits),
                expected_bounds,
                "{label}"
            );
            assert_eq!(
                distance.maximum(general).unwrap().to_bits(),
                expected_max,
                "{label}"
            );
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn x87_helper_restores_precision_control_and_stack_top() {
        fn state() -> (u16, u16) {
            let mut cw = 0_u16;
            let mut sw = 0_u16;
            // SAFETY: both pointers address writable two-byte locals.
            unsafe {
                core::arch::asm!("fnstcw word ptr [{c}]","fnstsw word ptr [{s}]",
                c=in(reg) &mut cw,s=in(reg) &mut sw,options(nostack,preserves_flags));
            }
            (cw, sw & 0x3800)
        }
        let (original, _) = state();
        let alternate = (original & !0x0300) | 0x0200;
        // SAFETY: save/restore changes this test thread's x87 precision only.
        unsafe {
            core::arch::asm!("fldcw word ptr [{c}]",c=in(reg) &alternate,options(nostack,preserves_flags));
        }
        let before = state();
        let mut results = Vec::new();
        for _ in 0..64 {
            results.push(extended_maximum(60.0, 46.904_156, 10).unwrap());
        }
        let after = state();
        unsafe {
            core::arch::asm!("fldcw word ptr [{c}]",c=in(reg) &original,options(nostack,preserves_flags));
        }
        assert_eq!(before, after);
        assert!(results.iter().all(|v| v.to_bits() == 1133295204));
    }
}
