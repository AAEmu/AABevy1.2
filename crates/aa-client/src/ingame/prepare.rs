//! Everything about a unit or doodad that does not need the ECS, done on the worker threads of [`aa_cry::workers`] before
//! the main thread builds the entity.
//!
//! Spawning a character from scratch costs tens of milliseconds of CPU (database rows, pak files, model parsing, texture
//! decoding, the composed face texture, the clip bindings), which showed as 50-300 ms frames whenever a town filled in or
//! a crowd came into range. A job here does all of it for one object and hands the results over:
//!
//! * [`plan_unit`] + [`build_unit`]: the [`CharacterAssets`] (database + pak), the composed face
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
    Character {
        assets: Box<CharacterAssets>,
        models: Vec<PreparedModel>,
        carried: Vec<CarriedItem>,
        applied: super::gear_refresh::Applied,
        wield: weapons::Wield,
        clips: Option<Arc<ClipSet>>,
        pose: Option<BoundClip>,
        /// The tools the pose holds (`anim_actions`).
        tools: [Option<super::data::Tool>; 2],
    },
    /// A monster / NPC model of its own (`.cdf`, `.chr`, `.cgf`).
    Creature { file: String, cdf: Option<Box<aa_anim::Cdf>>, models: Vec<PreparedModel> },
    /// A vehicle or ship: its body parts and wheels ([`super::vehicle`]).
    Vehicle(Vec<super::vehicle::Piece>),
    /// Particle effects only (a prefab model without a model, the summoned portal gates), positions relative to the unit.
    Effects(Vec<aa_world::placement::PlacedEffect>),
    /// Nothing we can draw, and why (logged once by [`super::units`]).
    None(String),
}

/// A doodad that is ready to be spawned.
pub struct DoodadBuild {
    pub path: String,
    pub model: Option<super::doodad_model::Prepared>,
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

/// What a unit is made of, decided from the database before any model is read ([`plan_unit`]).
pub enum UnitPlan {
    Character(Box<CharacterAssets>),
    Creature { file: String, cdf: Option<Box<aa_anim::Cdf>> },
    Vehicle(super::data::VehicleLook),
    /// Nothing to load.
    Done(UnitBuild),
}

/// What `unit` is made of, from the database; [`build_unit`] then reads the models. Callable from any thread.
pub fn plan_unit(db: &Db, unit: &Unit) -> UnitPlan {
    if let Some(look) = db.vehicle_look(unit.model_id) {
        return UnitPlan::Vehicle(look);
    }
    let file = db.actor_model_file(unit.model_id);
    let humanoid = match unit.kind {
        UnitKind::Character { .. } => appearance::race_gender(unit.model_id, unit.race(), unit.gender()),
        _ => file.as_deref().and_then(humanoid_of_file),
    };
    if let Some((race, gender)) = humanoid {
        let game = db.game.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut desc) = appearance::describe(&game, race, gender, &unit.equipment, &unit.model_params) else {
            return UnitPlan::Done(UnitBuild::None(format!("no appearance for {race:?} {gender:?} (model {})", unit.model_id)));
        };
        if matches!(desc.costume, Costume::Items(ref i) if i.is_empty()) && !unit.is_character() {
            // an NPC without armour data: the default outfit beats a naked body
            desc.costume = Costume::Login;
        }
        match appearance::resolve(&game, &desc, &unit.equipment) {
            Ok(a) => UnitPlan::Character(Box::new(a)),
            Err(e) => UnitPlan::Done(UnitBuild::None(format!("character assets of {race:?} {gender:?} (model {}): {e}", unit.model_id))),
        }
    } else if let Some(file) = file {
        let cdf = if file.ends_with(".cdf") {
            let game = db.game.lock().unwrap_or_else(|e| e.into_inner());
            match creature::configured_cdf(&game, &file, unit.model_id, &unit.equipment) {
                Ok(cdf) => Some(Box::new(cdf)),
                Err(e) => return UnitPlan::Done(UnitBuild::None(format!("cannot prepare {file}: {e}"))),
            }
        } else { None };
        UnitPlan::Creature { file, cdf }
    } else {
        let effects = db.prefab_effects(unit.model_id);
        if !effects.is_empty() {
            return UnitPlan::Done(UnitBuild::Effects(effects));
        }
        UnitPlan::Done(UnitBuild::None(format!("model {} has no actor model file", unit.model_id)))
    }
}

impl UnitPlan {
    /// The model and material files [`build_unit`] reads (the browser downloads them first).
    pub fn files(&self, db: &Db, unit: &Unit) -> Vec<String> {
        let mut models: Vec<(String, Option<String>)> = Vec::new();
        match self {
            UnitPlan::Character(assets) => {
                for part in &assets.parts {
                    models.push((part.path.clone(), part.material.clone()));
                    models.extend(part.attachments.iter().filter_map(super::attachments::model));
                }
                for item in weapons::carried(db, &unit.equipment) {
                    models.extend(weapons::model_files(&item.path));
                }
            }
            UnitPlan::Creature { file, cdf } => models.extend(creature::model_files(file, cdf.as_deref())),
            UnitPlan::Vehicle(look) => models.extend(super::vehicle::model_files(look)),
            UnitPlan::Done(_) => {}
        }
        let mut files: Vec<String> = models.iter().flat_map(|(p, m)| aa_cry::deps::model_files(p, m.as_deref())).collect();
        files.sort();
        files.dedup();
        files
    }
}

/// The pak half of a unit's preparation: reads the models, the face and the clips `plan` names.
pub fn build_unit(db: &Db, loader: &CryLoader, clips: &ClipLibrary, unit: &Unit, plan: UnitPlan) -> UnitBuild {
    let _t = aa_cry::prof::scope("unit.prepare");
    match plan {
        UnitPlan::Done(build) => build,
        UnitPlan::Character(assets) => build_character(db, loader, clips, unit, assets),
        UnitPlan::Creature { file, cdf } => build_creature(loader, unit, file, cdf),
        UnitPlan::Vehicle(look) => {
            // Warm the deck floor and its bounds on this worker so `board_deck`
            // never loads a hull mesh in the frame.
            db.ship_floor(unit.model_id);
            match super::vehicle::prepare(loader, &look) {
                pieces if pieces.is_empty() => UnitBuild::None(format!("no part of vehicle {} could be loaded (model {})", look.normal, unit.model_id)),
                pieces => UnitBuild::Vehicle(pieces),
            }
        }
    }
}

/// Synchronous preparation for native callers that already have all source files available.
pub fn prepare_unit(db: &Db, loader: &CryLoader, clips: &ClipLibrary, unit: &Unit) -> UnitBuild {
    build_unit(db, loader, clips, unit, plan_unit(db, unit))
}

fn build_character(db: &Db, loader: &CryLoader, clips: &ClipLibrary, unit: &Unit, assets: Box<CharacterAssets>) -> UnitBuild {
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
    let (race, gender) = (assets.desc.race, assets.desc.gender);
    let tools = db.anim_action_tools(unit.anim_action);
    UnitBuild::Character { assets, models, carried, applied: super::gear_refresh::Applied::new(unit.model_id, race, gender, &unit.equipment, &unit.model_params), wield: weapons::base_wield(db, &unit.equipment), clips: set, pose, tools }
}

fn build_creature(loader: &CryLoader, unit: &Unit, file: String, cdf: Option<Box<aa_anim::Cdf>>) -> UnitBuild {
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
    let Some(path) = db.doodad_phase_visual(template_id, func_group_id) else {
        return DoodadBuild { path: String::new(), model: None, func_group_id, growth };
    };
    let model = super::doodad_model::prepare(loader, &path);
    DoodadBuild { path, model, func_group_id, growth }
}
