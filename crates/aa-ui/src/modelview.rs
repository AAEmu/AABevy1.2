//! Widget-owned handoff to an application's preview renderer. No scene or target allocation.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::prelude::{Handle, Image};

use crate::tree::{Id, Tree};

/// Unique widget incarnation, never reused, including across runtimes. Clear keeps this ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelViewId(u64);

#[derive(Clone, Debug, PartialEq)]
pub struct ModelViewCommand {
    pub view: ModelViewId,
    pub op: ModelViewOp,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ModelViewOp {
    SetTextureSize([i32; 2]),
    Init {
        unit: String,
        create_model: bool,
    },
    SetIngameShopMode(bool),
    /// Add these f32 offsets in order, in the native camera's coordinate frame.
    AdjustCameraPos([f32; 3]),
    EquipItem(u32),
    ResetEquips,
    PlayAnimation {
        name: String,
        looping: bool,
    },
    StopAnimation,
    /// Accumulated radians. Lua's SetRotation/AddRotation take degrees.
    SetRotation(f32),
    /// Release presented content; retain the host's target allocation identity.
    ClearModel,
    Destroy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelViewSurface {
    pub view: ModelViewId,
    /// Raw valid request, before host renderer screen limits / allocation admission.
    pub requested_texture_size: [i32; 2],
    pub model_extent: [i32; 2],
    pub source_xywh: [i32; 4],
    /// Inherited visibility/fade paint admission, independent of host model readiness.
    pub paint_admitted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelViewImage {
    pub image: Handle<Image>,
    /// Actual allocation dimensions, used for pixel-crop UV normalization.
    pub extent_px: [u32; 2],
}

#[derive(Debug)]
pub struct ModelViewWork {
    pub commands: Vec<ModelViewCommand>,
    /// All live surfaces, including hidden ones. Absence means host retirement.
    pub surfaces: Vec<ModelViewSurface>,
}

// One process-wide monotonic namespace for both trees and widget incarnations. Fail at
// exhaustion instead of wrapping and admitting an old image into a new widget/runtime.
fn identity() -> ModelViewId {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    ModelViewId(
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .expect("modelview identity space exhausted"),
    )
}

pub(crate) struct View {
    pub token: ModelViewId,
    pub requested: [i32; 2],
    pub extent: [i32; 2],
    pub crop: [i32; 4],
    pub rotation: f32,
    pub drawable: Option<Id>,
    pub image: Option<ModelViewImage>,
}

pub(crate) struct Views {
    pub epoch: ModelViewId,
    pub live: BTreeMap<Id, View>,
    pub commands: Vec<ModelViewCommand>,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            epoch: identity(),
            live: BTreeMap::new(),
            commands: Vec::new(),
        }
    }
}

impl Views {
    pub fn create(&mut self, widget: Id) {
        self.live.insert(
            widget,
            View {
                token: identity(),
                requested: [128, 128],
                extent: [64, 64],
                crop: [0, 0, 128, 128],
                rotation: 0.,
                drawable: None,
                image: None,
            },
        );
    }

    pub fn push(&mut self, widget: Id, op: ModelViewOp) {
        if let Some(view) = self.live.get(&widget) {
            self.commands.push(ModelViewCommand {
                view: view.token,
                op,
            });
        }
    }

    pub fn retire(&mut self, widget: Id) {
        if let Some(view) = self.live.remove(&widget) {
            self.commands.push(ModelViewCommand {
                view: view.token,
                op: ModelViewOp::Destroy,
            });
        }
    }
}

pub(crate) fn take(u: &mut crate::lua::Ui) -> ModelViewWork {
    // Direct Tree mutations are supported by the existing public tree API too.
    reconcile(&mut u.tree);
    let surfaces = u
        .tree
        .modelviews
        .live
        .iter()
        .map(|(&id, v)| ModelViewSurface {
            view: v.token,
            requested_texture_size: v.requested,
            model_extent: v.extent,
            source_xywh: v.crop,
            paint_admitted: admitted(u, id),
        })
        .collect();
    ModelViewWork {
        commands: std::mem::take(&mut u.tree.modelviews.commands),
        surfaces,
    }
}

// Same live/shown-or-fading admission as Tree's paint traversal, walking only
// this surface's ancestors instead of allocating the whole UI paint order each frame.
fn admitted(u: &crate::lua::Ui, id: Id) -> bool {
    let mut owner = Some(id);
    while let Some(id) = owner {
        let o = u.tree.get(id);
        if !o.alive || !(o.shown || o.widget().is_some_and(|w| w.fading)) {
            return false;
        }
        owner = o.parent;
    }
    crate::render::effective_alpha(u, id) > 0.
}

pub(crate) fn reconcile(tree: &mut Tree) {
    let dead: Vec<_> = tree
        .modelviews
        .live
        .keys()
        .copied()
        .filter(|&id| !tree.objects.get(id as usize).is_some_and(|o| o.alive))
        .collect();
    if !dead.is_empty() {
        tree.repaint();
    }
    for id in dead {
        tree.modelviews.retire(id);
    }
}

impl crate::UiRuntime {
    /// Drain exact ordered native calls and snapshot all live modelview surfaces, once per
    /// host frame in [`crate::UiModelViews`]. No UI borrow survives this call.
    pub fn take_modelview_work(&self) -> ModelViewWork {
        take(&mut crate::lua::ui(&self.lua))
    }

    /// Associate a root-owned image. None withdraws content. Reject a retired/foreign token
    /// (or a zero allocation extent). Host async job revisions remain the host's responsibility.
    pub fn set_modelview_image(&self, view: ModelViewId, image: Option<ModelViewImage>) -> bool {
        let mut u = crate::lua::ui(&self.lua);
        reconcile(&mut u.tree);
        if image.as_ref().is_some_and(|i| i.extent_px.contains(&0)) {
            return false;
        }
        let Some(v) = u
            .tree
            .modelviews
            .live
            .values_mut()
            .find(|v| v.token == view)
        else {
            return false;
        };
        if v.image != image {
            v.image = image;
            u.tree.repaint();
        }
        true
    }
}

#[cfg(test)]
mod tests;
