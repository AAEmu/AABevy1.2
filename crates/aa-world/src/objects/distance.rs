//! Refresh distance-derived data without rereading placement or rebuilding static meshes.
use super::{CellObjectData, Kind, ObjectState, ViewLimits, dissolve};
use crate::stream::WorldConfig;
use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;

/// Authored inputs retained by drawn parts; the range itself is derived from current settings.
#[derive(Component, Clone, Copy)]
pub(crate) enum ViewDistance {
    Brush(f32),
    Vegetation { compiled: f32, mesh_until: f32 },
    Near,
    Road { compiled: f32, radius: f32 },
    Voxel(f32),
}

impl ViewDistance {
    pub(super) fn range(self, limits: &ViewLimits) -> VisibilityRange {
        match self {
            Self::Brush(compiled) => dissolve::range(limits.brush_view(compiled).0),
            Self::Vegetation { compiled, mesh_until } => {
                dissolve::range(limits.vegetation_view(compiled).0.min(mesh_until))
            }
            Self::Near => dissolve::range(limits.near),
            Self::Road { compiled, radius } => {
                let end = compiled.min(limits.far).max(0.0) + radius;
                VisibilityRange { start_margin: 0.0..0.0, end_margin: end..end + 20.0, use_aabb: true }
            }
            Self::Voxel(compiled) => {
                let end = if compiled.is_finite() && compiled > 1.0 { compiled.min(limits.far) } else { limits.far };
                VisibilityRange { start_margin: 0.0..0.0, end_margin: end..end, use_aabb: true }
            }
        }
    }
}

impl CellObjectData {
    /// Keep sector/bin membership fixed for the cell lifetime. Moving the draw limit must not
    /// invalidate in-progress instance cursors or native brush LOD-pair bin indices.
    pub(super) fn refresh_limits(&mut self, limits: &ViewLimits) {
        if self.limits == *limits { return; }
        self.limits = *limits;
        for sector in 0..self.views.len() {
            for kind in [Kind::Brush, Kind::FarBrush, Kind::Entity, Kind::Animated, Kind::Vegetation, Kind::Trees] {
                let view = super::bin_of(self, kind, sector).iter().map(|&index| match kind {
                    Kind::Brush | Kind::FarBrush => limits.brush(&self.objects.brushes[index as usize]).0,
                    Kind::Vegetation | Kind::Trees => limits.vegetation(&self.objects.vegetation[index as usize]).0,
                    Kind::Entity | Kind::Animated => limits.near,
                }).fold(0.0_f32, f32::max);
                self.views[sector][kind as usize] = view;
            }
        }
    }
}

pub(crate) fn refresh(
    cfg: Res<WorldConfig>, mut state: ResMut<ObjectState>,
    mut parts: Query<(&ViewDistance, &mut VisibilityRange)>,
    mut previous: Local<Option<ViewLimits>>,
) {
    let limits = ViewLimits::new(&cfg);
    if *previous == Some(limits) { return; }
    *previous = Some(limits);
    for data in state.cells.values_mut() {
        // Normally uniquely owned between streaming frames; callers holding a temporary snapshot
        // must keep that snapshot intact. No placement files or model assets are loaded here.
        std::sync::Arc::make_mut(data).refresh_limits(&limits);
    }
    for (distance, mut range) in &mut parts {
        let next = distance.range(&limits);
        if range.start_margin != next.start_margin || range.end_margin != next.end_margin || range.use_aabb != next.use_aabb {
            *range = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::{Brush, CellObjects, Vegetation};
    use std::sync::Arc;

    fn cell(limits: &ViewLimits) -> CellObjectData {
        let brush = |distance| Brush {
            matrix: [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 10.0, 0.0, 0.0, 1.0, 0.0],
            model: 0, material: -1, flags: 0, max_view_dist: distance, lod_ratio: 100, bbox: [0.0; 6],
        };
        CellObjectData::new(CellObjects {
            brushes: vec![brush(600.0), brush(9112.0)],
            vegetation: vec![Vegetation {
                pos: Vec3::splat(10.0), scale: 1.0, group: 0,
                angle_x: 0, angle_y: 0, angle_z: 0, bright: 0,
                color_variation: [255; 3], flags: 0, max_view_dist: 160.0,
            }],
            ..default()
        }, limits)
    }

    #[test]
    fn already_loaded_settings_refresh_ranges_and_sector_reach_without_replacing_geometry() {
        let cfg = WorldConfig::new("main_world");
        let high = ViewLimits::new(&cfg);
        let mut state = ObjectState::new();
        state.cells.insert((13, 10), Arc::new(cell(&high)));
        let mut app = App::new();
        app.insert_resource(cfg).insert_resource(state).init_resource::<Assets<Mesh>>().add_systems(Update, refresh);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(Cuboid::default());
        let root = app.world_mut().spawn(Transform::IDENTITY).id();
        let sources = [
            ViewDistance::Brush(600.0), ViewDistance::Brush(9112.0),
            ViewDistance::Vegetation { compiled: 160.0, mesh_until: 180.0 },
            ViewDistance::Vegetation { compiled: 160.0, mesh_until: f32::INFINITY },
            ViewDistance::Near, ViewDistance::Road { compiled: 1500.0, radius: 12.0 },
            ViewDistance::Voxel(1400.0),
        ];
        let parts: Vec<_> = sources.iter().map(|&source| app.world_mut().spawn((
            source, source.range(&high), Mesh3d(mesh.clone()), Transform::IDENTITY, ChildOf(root),
        )).id()).collect();
        app.update();
        let data = &app.world().resource::<ObjectState>().cells[&(13, 10)];
        let bin_address = data.brush_bins[0].as_ptr();
        let data_address = Arc::as_ptr(data);
        assert_eq!(data.views[0][Kind::Brush as usize], 900.0);
        assert_eq!(data.views[0][Kind::Trees as usize], 240.0);

        {
            let mut cfg = app.world_mut().resource_mut::<WorldConfig>();
            cfg.brush_distance = 600.0;
            cfg.view_dist_scale = 0.75;
            cfg.vegetation_distance = 110.0;
            cfg.tree_distance = 500.0;
        }
        app.update();
        let ends: Vec<_> = parts.iter().map(|&e| app.world().get::<VisibilityRange>(e).unwrap().end_margin.clone()).collect();
        assert_eq!(ends, [442.0..450.0, 592.0..600.0, 117.0..120.0, 117.0..120.0,
            592.0..600.0, 612.0..632.0, 600.0..600.0]);
        let data = &app.world().resource::<ObjectState>().cells[&(13, 10)];
        assert_eq!(data.views[0][Kind::Brush as usize], 450.0);
        assert_eq!(data.views[0][Kind::FarBrush as usize], 600.0);
        assert_eq!(data.views[0][Kind::Trees as usize], 120.0);
        assert_eq!(Arc::as_ptr(data), data_address);
        assert_eq!(data.brush_bins[0].as_ptr(), bin_address);
        for &e in &parts {
            assert_eq!(app.world().get::<ChildOf>(e).unwrap().parent(), root);
            assert_eq!(app.world().get::<Mesh3d>(e).unwrap().0, mesh);
        }

        *app.world_mut().resource_mut::<WorldConfig>() = WorldConfig::new("main_world");
        app.update();
        for (&e, source) in parts.iter().zip(sources) {
            assert_eq!(app.world().get::<VisibilityRange>(e).unwrap().end_margin, source.range(&high).end_margin);
        }
        app.update();
        assert_eq!(Arc::as_ptr(&app.world().resource::<ObjectState>().cells[&(13, 10)]), data_address);
    }

    #[test]
    fn placement_finished_after_option_save_uses_current_limits_without_rebinning() {
        let mut data = cell(&ViewLimits::default());
        let old_bin = data.brush_bins.clone();
        let mut cfg = WorldConfig::new("main_world");
        cfg.brush_distance = 150.0;
        cfg.view_dist_scale = 0.75;
        let limits = ViewLimits::new(&cfg);
        data.refresh_limits(&limits);
        assert_eq!(data.brush_bins, old_bin, "in-flight indices stay valid across saves");
        assert_eq!(data.limits, limits);
        assert_eq!(data.views[0][Kind::Brush as usize], 150.0);
        assert_eq!(data.views[0][Kind::FarBrush as usize], 150.0);
    }
}
