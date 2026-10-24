//! Deterministic regression fixture for review-01's plan/draw readiness race.
//! This manipulates only a labelled stock control in the private example.
use bevy::{
    prelude::*,
    render::{
        render_phase::{CachedRenderPipelinePhaseItem, PhaseItem},
        render_resource::{CachedRenderPipelineId, PipelineCache},
        sync_world::MainEntity,
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

#[derive(Clone, Copy)]
struct Selection {
    queued: CachedRenderPipelineId,
    selected: Option<CachedRenderPipelineId>,
    depth_writes: Option<bool>,
}

#[derive(Resource)]
pub(crate) struct Control {
    enabled: bool,
    trigger: u64,
    epoch: u64,
    members: HashSet<MainEntity>,
    fresh: Option<CachedRenderPipelineId>,
    ready_at_plan: bool,
    ready_before_draw: bool,
    selections: Mutex<HashMap<(Entity, bool), Selection>>,
    submitted: Mutex<HashSet<(Entity, bool)>>,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            enabled: super::flag("--readiness-transition"),
            trigger: super::arg("--capture-frame")
                .and_then(|v| v.parse().ok())
                .unwrap_or(360),
            epoch: 0,
            members: HashSet::new(),
            fresh: None,
            ready_at_plan: false,
            ready_before_draw: false,
            selections: Mutex::default(),
            submitted: Mutex::default(),
        }
    }
}
impl Control {
    pub(crate) fn begin(&mut self, epoch: u64, members: impl Iterator<Item = MainEntity>) {
        self.epoch = epoch;
        if self.enabled {
            self.members = members.collect();
            self.selections.get_mut().unwrap().clear();
            self.submitted.get_mut().unwrap().clear();
        }
    }

    // Return true when the control's retained bin must adopt a new base identity.
    pub(crate) fn prepare(
        &mut self,
        cache: &PipelineCache,
        entity: MainEntity,
        id: &mut CachedRenderPipelineId,
    ) -> bool {
        if !self.enabled || self.epoch < self.trigger || !self.members.contains(&entity) {
            return false;
        }
        let fresh = *self.fresh.get_or_insert_with(|| {
            assert!(
                cache.get_render_pipeline(*id).is_some(),
                "control source must be warm before introducing a new stock pipeline"
            );
            let mut descriptor = cache.get_render_pipeline_descriptor(*id).clone();
            descriptor.label = Some("readiness control: freshly queued unsplit stock base".into());
            cache.queue_render_pipeline(descriptor)
        });
        self.ready_at_plan = cache.get_render_pipeline(fresh).is_some();
        if self.epoch == self.trigger {
            assert!(
                !self.ready_at_plan,
                "fresh control base must be unavailable to plan"
            );
        }
        let changed = *id != fresh;
        *id = fresh;
        changed
    }

    pub(crate) fn fresh(&self) -> Option<CachedRenderPipelineId> {
        self.fresh
    }

    pub(crate) fn finish(
        &mut self,
        cache: &mut PipelineCache,
        split: Option<(CachedRenderPipelineId, CachedRenderPipelineId)>,
    ) {
        if !self.enabled || self.epoch < self.trigger || self.epoch > self.trigger + 1 {
            return;
        }
        let fresh = self.fresh.expect("control queued during plan");
        if self.epoch == self.trigger {
            assert!(!self.ready_at_plan);
            assert!(
                split.is_none(),
                "the first availability transition must have no split plan"
            );
        } else {
            assert!(self.ready_at_plan);
        }
        // Real PipelineCache processing: no fake readiness booleans or GPU
        // pipeline substitution. This deliberately occurs after plan/queue.
        cache.block_on_render_pipeline(fresh);
        assert!(cache.get_render_pipeline(fresh).is_some());
        if let Some((z, motion)) = split {
            cache.block_on_render_pipeline(z);
            cache.block_on_render_pipeline(motion);
            assert!(cache.get_render_pipeline(z).is_some());
            assert!(cache.get_render_pipeline(motion).is_some());
        }
        self.ready_before_draw = true;
        info!(
            "native-pass-probe: readiness-transition epoch={} base={} plan_ready={} draw_ready=true split_planned={}",
            self.epoch,
            fresh.id(),
            self.ready_at_plan,
            split.is_some()
        );
    }

    fn tracks<P: PhaseItem>(&self, item: &P) -> bool {
        self.enabled
            && (self.epoch == self.trigger || self.epoch == self.trigger + 1)
            && self.members.contains(&item.main_entity())
    }

    pub(crate) fn selection<P: CachedRenderPipelinePhaseItem>(
        &self,
        item: &P,
        view: Entity,
        motion: bool,
        selected: Option<CachedRenderPipelineId>,
        cache: &PipelineCache,
    ) {
        if !self.tracks(item) {
            return;
        }
        let depth_writes = selected.map(|id| {
            cache
                .get_render_pipeline_descriptor(id)
                .depth_stencil
                .as_ref()
                .unwrap()
                .depth_write_enabled
                .unwrap_or(false)
        });
        self.selections.lock().unwrap().insert(
            (view, motion),
            Selection {
                queued: item.cached_pipeline(),
                selected,
                depth_writes,
            },
        );
    }

    pub(crate) fn submitted<P: PhaseItem>(&self, item: &P, view: Entity, motion: bool) {
        if self.tracks(item) {
            self.submitted.lock().unwrap().insert((view, motion));
        }
    }

    pub(crate) fn write(
        &self,
        entity: Entity,
        number: u32,
        cache: &PipelineCache,
        split: Option<(CachedRenderPipelineId, CachedRenderPipelineId)>,
    ) {
        if !self.enabled || (self.epoch != self.trigger && self.epoch != self.trigger + 1) {
            return;
        }
        let Some(dir) = super::readback::directory(self.epoch) else {
            return;
        };
        let fresh = self.fresh.unwrap();
        assert!(self.ready_before_draw && cache.get_render_pipeline(fresh).is_some());
        let selections = self.selections.lock().unwrap();
        let submitted = self.submitted.lock().unwrap();
        let mut csv = String::from(
            "stage,base_pipeline,ready_at_plan,ready_before_draw,split_planned,queued_pipeline,selected_pipeline,selected_depth_writes,submitted\n",
        );
        for motion in [false, true] {
            let selection = selections
                .get(&(entity, motion))
                .expect("control must reach ChoosePrepass in both passes");
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{}\n",
                if motion { "motion" } else { "z" },
                fresh.id(),
                self.ready_at_plan,
                self.ready_before_draw,
                split.is_some(),
                selection.queued.id(),
                selection
                    .selected
                    .map_or(String::new(), |id| id.id().to_string()),
                selection
                    .depth_writes
                    .map_or(String::new(), |v| v.to_string()),
                submitted.contains(&(entity, motion))
            ));
            if self.epoch == self.trigger {
                assert_eq!(selection.queued, fresh);
                assert!(selection.selected.is_none() && !submitted.contains(&(entity, motion)));
            } else {
                let (z, m) = split.expect("next frame must plan split variants");
                assert_eq!(
                    selection.queued, z,
                    "future retained queue must use the split Z identity"
                );
                assert_eq!(selection.selected, Some(if motion { m } else { z }));
                assert_eq!(selection.depth_writes, Some(!motion));
                assert!(
                    submitted.contains(&(entity, motion)),
                    "trailing DrawMesh recorder must execute"
                );
            }
        }
        std::fs::write(
            dir.join(format!("v{number}-f{}-readiness.csv", self.epoch)),
            csv,
        )
        .unwrap();
    }
}
