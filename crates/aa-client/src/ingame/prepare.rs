//! Everything about a unit or doodad that does not need the ECS, done on the worker threads of [`aa_cry::workers`] before
//! the main thread builds the entity.
//!
//! Spawning a character from scratch costs tens of milliseconds of CPU (database rows, pak files, model parsing, texture
//! decoding, the composed face texture, the clip bindings), which showed as 50-300 ms frames whenever a town filled in or
//! a crowd came into range. A job here does all of it for one object and hands the results over:
//!
//! * [`prepare_unit`]: the [`CharacterAssets`] (database + pak), the composed face
//!   ([`aa_anim::character::prewarm_character_face`]), the part models as [`PreparedModel`]s (their caches are filled when
//!   the main thread calls `finish_model`), the weapons it carries and its locomotion clips;
//! * [`prepare_doodad`]: the doodad's model.
//!
//! The main thread then only inserts assets and spawns entities; every lookup it still does hits a cache.

use std::sync::Arc;

use aa_anim::pose::BoundClip;
use aa_anim::{CharacterAssets, ClipLibrary, Costume, Gender, Race};
use aa_cry::scene::{ModelOptions, Shading};
use aa_cry::{CryLoader, PreparedModel};
use aa_net::model::{Unit, UnitKind};
use bevy::prelude::*;

use super::anim::{ClipSet, shared_clip_set};
use super::appearance;
use super::creature;
use super::data::{Db, DoodadGrowth};
use super::weapons::{self, CarriedItem};

/// A unit that is ready to be spawned.
pub enum UnitBuild {
    /// Drawn like a player character.
    Character { assets: Box<CharacterAssets>, models: Vec<PreparedModel>, carried: Vec<CarriedItem>, clips: Option<Arc<ClipSet>>, pose: Option<BoundClip> },
    /// A monster / NPC model of its own (`.cdf`, `.chr`, `.cgf`).
    Creature { file: String, cdf: Option<Box<aa_anim::Cdf>>, models: Vec<PreparedModel> },
    /// Particle effects only (a prefab model without a model, the summoned portal gates), positions relative to the unit.
    Effects(Vec<aa_world::placement::PlacedEffect>),
    /// Nothing we can draw, and why (logged once by [`super::units`]).
    None(String),
}

/// A doodad that is ready to be spawned.
pub struct DoodadBuild {
    pub path: String,
    pub model: Option<PreparedModel>,
    /// The phase this model belongs to: a phase change while the worker ran makes the build stale.
    pub func_group_id: u32,
    pub growth: Option<DoodadGrowth>,
}

/// Race and gender of a humanoid model file (`objects/characters/<race>/<gender>/nude/...`).
pub fn humanoid_of_file(file: &str) -> Option<(Race, Gender)> {
    let rest = file.split("characters/").nth(1)?;
    let mut it = rest.split('/');
    let folder = it.next()?;
    let race = Race::ALL.into_iter().find(|r| r.folder() == folder)?;
    let gender = match it.next()? {
        "male" => Gender::Male,
        "female" => Gender::Female,
        _ => return None,
    };
    (it.next()? == "nude").then_some((race, gender))
}

/// Prepares the models of `unit`; callable from any thread.
pub fn prepare_unit(db: &Db, loader: &CryLoader, clips: &ClipLibrary, unit: &Unit) -> UnitBuild {
    let _t = aa_cry::prof::scope("unit.prepare");
    let file = db.actor_model_file(unit.model_id);
    let humanoid = match unit.kind {
        UnitKind::Character { .. } => appearance::race_gender(unit.model_id, unit.race(), unit.gender()),
        _ => file.as_deref().and_then(humanoid_of_file),
    };
    if let Some((race, gender)) = humanoid {
        let assets = {
            let game = db.game.lock().unwrap_or_else(|e| e.into_inner());
            let Some(mut desc) = appearance::describe(&game, race, gender, &unit.equipment, &unit.model_params) else {
                return UnitBuild::None(format!("no appearance for {race:?} {gender:?} (model {})", unit.model_id));
            };
            if matches!(desc.costume, Costume::Items(ref i) if i.is_empty()) && !unit.is_character() {
                // an NPC without armour data: the default outfit beats a naked body
                desc.costume = Costume::Login;
            }
            match appearance::resolve(&game, &desc, &unit.equipment) {
                Ok(a) => a,
                Err(e) => return UnitBuild::None(format!("character assets of {race:?} {gender:?} (model {}): {e}", unit.model_id)),
            }
        };
        if let Err(e) = aa_anim::character::prewarm_character_face(&assets) {
            debug!("unit {} face: {e}", unit.obj_id);
        }
        let mut models = Vec::new();
        for part in &assets.parts {
            let options = ModelOptions { material: part.material.clone(), cry_space: true, shading: Shading::Full, ..default() };
            if let Ok(m) = loader.prepare_model(&part.path, &options) {
                models.push(m);
            }
            for attachment in &part.attachments {
                if let Some((path, material)) = super::attachments::model(attachment) {
                    let options = ModelOptions { material, cry_space: true, shading: Shading::Full, ..default() };
                    if let Ok(model) = loader.prepare_model(&path, &options) { models.push(model); }
                }
            }
        }
        let carried = weapons::carried(db, &unit.equipment);
        for (path, material) in carried.iter().flat_map(|item| weapons::model_files(&item.path)) {
            let options = ModelOptions { material, cry_space: true, shading: Shading::Full, ..default() };
            if let Ok(m) = loader.prepare_model(&path, &options) {
                models.push(m);
            }
        }
        let set = shared_clip_set(&assets.skeleton, &assets.cal, clips);
        // an NPC that stands in a pose (a merchant's, a guard's stance)
        let pose = super::idle::pose_clip(db, unit.anim_action, &assets.skeleton, &assets.cal, clips);
        UnitBuild::Character { assets: Box::new(assets), models, carried, clips: set, pose }
    } else if let Some(file) = file {
        let cdf = if file.ends_with(".cdf") {
            let game = db.game.lock().unwrap_or_else(|e| e.into_inner());
            match creature::configured_cdf(&game, &file, unit.model_id, &unit.equipment) {
                Ok(cdf) => Some(Box::new(cdf)),
                Err(e) => return UnitBuild::None(format!("cannot prepare {file}: {e}")),
            }
        } else { None };
        let mut models = Vec::new();
        for (path, material) in creature::model_files(&file, cdf.as_deref()) {
            let options = ModelOptions { material, cry_space: true, ..default() };
            if let Ok(m) = loader.prepare_model(&path, &options) {
                models.push(m);
            }
        }
        if models.is_empty() {
            return UnitBuild::None(format!("no model could be loaded from {file} (model {})", unit.model_id));
        }
        UnitBuild::Creature { file, cdf, models }
    } else {
        let effects = db.prefab_effects(unit.model_id);
        if !effects.is_empty() {
            return UnitBuild::Effects(effects);
        }
        UnitBuild::None(format!("model {} has no actor model file", unit.model_id))
    }
}

/// A character of the character list (what the server saved: race, gender, equipment, looks) ready to be spawned like
/// the player of the game screen; callable from any thread. Used by `prewarm`.
pub fn prepare_listed(db: &Db, loader: &CryLoader, c: &aa_net::model::Character) -> Option<(Box<CharacterAssets>, Vec<PreparedModel>)> {
    let (race, gender) = appearance::race_gender(0, c.race, c.gender)?;
    let assets = {
        let game = db.game.lock().unwrap_or_else(|e| e.into_inner());
        let desc = appearance::describe(&game, race, gender, &c.equipment, &c.model_params)?;
        appearance::resolve(&game, &desc, &c.equipment).ok()?
    };
    if let Err(e) = aa_anim::character::prewarm_character_face(&assets) {
        debug!("listed character {} face: {e}", c.name);
    }
    let mut models = Vec::new();
    for part in &assets.parts {
        let options = ModelOptions { material: part.material.clone(), cry_space: true, shading: Shading::Full, ..default() };
        if let Ok(m) = loader.prepare_model(&part.path, &options) {
            models.push(m);
        }
        for attachment in &part.attachments {
            if let Some((path, material)) = super::attachments::model(attachment) {
                let options = ModelOptions { material, cry_space: true, shading: Shading::Full, ..default() };
                if let Ok(model) = loader.prepare_model(&path, &options) { models.push(model); }
            }
        }
    }
    Some((Box::new(assets), models))
}

/// Prepares the model of the doodad template `template_id` in its current phase `func_group_id`; callable from any
/// thread.
pub fn prepare_doodad(db: &Db, loader: &CryLoader, template_id: u32, func_group_id: u32) -> DoodadBuild {
    let _t = aa_cry::prof::scope("doodad.prepare");
    let growth = db.doodad_growth(func_group_id);
    let Some(path) = db.doodad_phase_model(template_id, func_group_id) else {
        return DoodadBuild { path: String::new(), model: None, func_group_id, growth };
    };
    let model = match loader.prepare_model(&path, &ModelOptions::default()) {
        Ok(m) => Some(m),
        Err(e) => {
            debug!("doodad {template_id} phase {func_group_id} model {path}: {e}");
            None
        }
    };
    DoodadBuild { path, model, func_group_id, growth }
}
