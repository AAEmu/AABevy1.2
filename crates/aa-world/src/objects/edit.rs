//! Hooks for editors: which placed object a drawn part belongs to, and replacing a loaded cell's placement data.
//!
//! Indices are those of [`CellObjects`] (`brushes`, `vegetation`, `entity_props`). An editor keeps them stable by
//! never removing entries: removed objects are left out of the bins with the `removed` filter of
//! [`ObjectState::replace_cell_objects`], copies are appended.

use std::sync::Arc;

use bevy::prelude::*;

use super::{CellObjectData, Kind, ObjectSector, ObjectState, lod};
use crate::placement::CellObjects;
use crate::vegetation::VegGroups;

/// What a placed object is, for [`ObjectPart`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Brush,
    Vegetation,
    /// A static `entities.xml` render slot.
    Entity,
    /// A playing animated `entities.xml` slot.
    Animated,
}

impl ObjectKind {
    fn of(kind: Kind) -> Self {
        match kind {
            Kind::Brush | Kind::FarBrush => ObjectKind::Brush,
            Kind::Vegetation | Kind::Trees => ObjectKind::Vegetation,
            Kind::Entity => ObjectKind::Entity,
            Kind::Animated => ObjectKind::Animated,
        }
    }
}

/// On every drawn mesh part of a brush, vegetation instance or entity slot: the object it belongs to.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectPart {
    pub cell: (i32, i32),
    pub kind: ObjectKind,
    /// Index into the cell's [`CellObjects`] list of that kind.
    pub index: u32,
}

impl ObjectPart {
    pub(super) fn new(cell: (i32, i32), kind: Kind, index: usize) -> Self {
        ObjectPart { cell, kind: ObjectKind::of(kind), index: index as u32 }
    }
}

const KINDS: [Kind; 6] = [Kind::Brush, Kind::FarBrush, Kind::Entity, Kind::Animated, Kind::Vegetation, Kind::Trees];

impl ObjectState {
    /// The placement data of a loaded cell.
    pub fn cell_data(&self, cell: (i32, i32)) -> Option<&Arc<CellObjectData>> {
        self.cells.get(&cell)
    }

    /// Cells whose placement data is loaded.
    pub fn object_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.cells.keys().copied()
    }

    /// The world's vegetation groups, once loaded.
    pub fn vegetation_groups(&self) -> Option<&Arc<VegGroups>> {
        self.groups.as_ref()
    }

    /// Replaces the placement data of a loaded cell (an editor's edited copy). Objects for which `removed` is true
    /// are left out. Sectors in `respawn` (`sx * 16 + sy`) and those whose instance lists changed are rebuilt: the
    /// new parts are built hidden next to the old ones and swapped in when complete, so nothing blinks.
    ///
    /// Returns the sectors that could not be rebuilt yet (a level of detail transition is running); call again
    /// later for those. Returns `None` when the cell is not loaded.
    pub fn replace_cell_objects(
        &mut self,
        commands: &mut Commands,
        cell: (i32, i32),
        objects: CellObjects,
        removed: impl Fn(ObjectKind, u32) -> bool,
        respawn: &[u8],
    ) -> Option<Vec<u8>> {
        let old = self.cells.get(&cell)?.clone();
        let mut data = CellObjectData::new(objects, &old.limits);
        for kind in KINDS {
            let k = ObjectKind::of(kind);
            for sector in 0..256 {
                bin_mut(&mut data, kind, sector).retain(|&i| !removed(k, i));
            }
        }
        let mut deferred = Vec::new();
        for kind in KINDS {
            for sector in 0..=255u8 {
                let changed = super::bin_of(&old, kind, sector as usize) != super::bin_of(&data, kind, sector as usize);
                if !changed && !respawn.contains(&sector) {
                    continue;
                }
                let Some(s) = self.sectors.get_mut(&(cell.0, cell.1, sector, kind)) else { continue };
                if s.transitioning {
                    deferred.push(sector);
                    // keep the old lists for this sector until it can be rebuilt
                    *bin_mut(&mut data, kind, sector as usize) = super::bin_of(&old, kind, sector as usize).to_vec();
                    continue;
                }
                let root = commands
                    .spawn((ObjectSector { cell, sector, kind }, Transform::from_translation(s.translation), Visibility::Hidden))
                    .id();
                s.retired.push(std::mem::replace(&mut s.entity, root));
                s.hidden = true;
                s.next = 0;
                s.spawned = 0;
                s.waiting.clear();
                s.sprite_models.clear();
                s.drawn_sprites = 0;
                (s.refresh_below, s.refresh_above) = (f32::MIN, f32::MAX);
                if s.brush_lods.is_some() {
                    s.brush_lods = Some(lod::BrushLods::default());
                }
            }
        }
        deferred.sort_unstable();
        deferred.dedup();
        self.cells.insert(cell, Arc::new(data));
        Some(deferred)
    }
}

fn bin_mut(data: &mut CellObjectData, kind: Kind, sector: usize) -> &mut Vec<u32> {
    match kind {
        Kind::Brush => &mut data.brush_bins[sector],
        Kind::FarBrush => &mut data.far_brush_bins[sector],
        Kind::Entity => &mut data.entity_bins[sector],
        Kind::Animated => &mut data.animated_bins[sector],
        Kind::Vegetation => &mut data.veg_bins[sector],
        Kind::Trees => &mut data.tree_bins[sector],
    }
}
