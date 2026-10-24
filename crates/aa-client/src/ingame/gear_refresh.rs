//! Receipt-driven visual replacement on the existing actor, bones and Animator.
use super::{
    appearance,
    data::Db,
    player::LocalPlayer,
    units::{Asleep, RemoteUnit},
    weapons,
};
use aa_anim::{Animator, Character, CharacterAssets, Gender, Race};
use aa_cry::{
    CryLoader, PreparedModel,
    scene::{CryAssets, ModelOptions, Shading},
};
use aa_net::{
    model::{Equipment, ItemDetail, ModelParams},
    state::{Player, Units},
};
use bevy::{ecs::system::SystemState, prelude::*};
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        mpsc::{Receiver, Sender, channel},
    },
};

#[derive(Clone, Debug, PartialEq)]
struct Key {
    model: u32,
    race: Race,
    gender: Gender,
    params: ModelParams,
    // IDs/count/grade/durability do not change these render assets. Image and
    // dye do. Keeping this key small avoids a rebuild for ordinary wear damage.
    equipment: Vec<(u32, u32, u32)>,
}
impl Key {
    fn new(
        model: u32,
        race: Race,
        gender: Gender,
        equipment: &Equipment,
        params: &ModelParams,
    ) -> Self {
        Self {
            model,
            race,
            gender,
            params: params.clone(),
            equipment: equipment
                .slots
                .iter()
                .map(|item| {
                    item.as_ref().map_or((0, 0, 0), |item| {
                        (
                            item.template_id,
                            appearance::effective_look(item),
                            match &item.detail {
                                ItemDetail::Equip(d) => d.dye_item_id,
                                _ => 0,
                            },
                        )
                    })
                })
                .collect(),
        }
    }
}
#[derive(Component)]
pub(super) struct Applied {
    key: Key,
    failed: Option<Key>,
}
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Refresh;
impl Applied {
    pub(super) fn new(
        model: u32,
        race: Race,
        gender: Gender,
        equipment: &Equipment,
        params: &ModelParams,
    ) -> Self {
        Self {
            key: Key::new(model, race, gender, equipment, params),
            failed: None,
        }
    }
}
struct Source {
    key: Key,
    equipment: Equipment,
}
struct Prepared {
    assets: CharacterAssets,
    models: Vec<PreparedModel>,
    carried: Vec<weapons::CarriedItem>,
    wield: weapons::Wield,
}
struct Ready {
    root: Entity,
    key: Key,
    result: Result<Prepared, String>,
}
#[derive(Resource)]
struct Jobs {
    tx: Sender<Ready>,
    rx: Mutex<Receiver<Ready>>,
    pending: HashMap<Entity, Key>,
}
impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tx,
            rx: Mutex::new(rx),
            pending: HashMap::new(),
        }
    }
}

fn source(world: &World, root: Entity) -> Option<Source> {
    if let Some(local) = world.get::<LocalPlayer>(root) {
        let spawn = world.get_resource::<Player>()?.spawn.as_ref()?;
        if local.obj_id != spawn.obj_id {
            return None;
        }
        let character = spawn.character();
        let equipment =
            crate::hud::received_equipment(world.get_non_send::<crate::hud::api::HudBridge>()?)?;
        let mut params = if spawn.unit.model_params.kind > 0 {
            spawn.unit.model_params.clone()
        } else {
            character.model_params.clone()
        };
        if let Some(color) = world
            .get_resource::<super::dev::DevOptions>()
            .and_then(|dev| dev.hair_color)
        {
            params.hair_color = color;
            params.kind = params.kind.max(ModelParams::HAIR);
        }
        let (race, gender) =
            appearance::race_gender(spawn.unit.model_id, character.race, character.gender)?;
        Some(Source {
            key: Key::new(spawn.unit.model_id, race, gender, &equipment, &params),
            equipment,
        })
    } else {
        let remote = world.get::<RemoteUnit>(root)?;
        let unit = world.get_resource::<Units>()?.by_id.get(&remote.obj_id)?;
        if !unit.is_character() {
            return None;
        }
        let (race, gender) = appearance::race_gender(unit.model_id, unit.race(), unit.gender())?;
        Some(Source {
            key: Key::new(
                unit.model_id,
                race,
                gender,
                &unit.equipment,
                &unit.model_params,
            ),
            equipment: unit.equipment.clone(),
        })
    }
}
fn prepare(db: &Db, loader: &CryLoader, source: &Source) -> Result<Prepared, String> {
    let assets = {
        let game = db.game.lock().map_err(|e| e.to_string())?;
        let desc = appearance::describe(
            &game,
            source.key.race,
            source.key.gender,
            &source.equipment,
            &source.key.params,
        )
        .ok_or("cannot describe received gear")?;
        appearance::resolve(&game, &desc, &source.equipment).map_err(|e| e.to_string())?
    };
    aa_anim::character::prewarm_character_face(&assets).map_err(|e| e.to_string())?;
    let mut models = Vec::new();
    for part in &assets.parts {
        let options = ModelOptions {
            material: part.material.clone(),
            cry_space: true,
            shading: Shading::Full,
            ..Default::default()
        };
        models.push(
            loader
                .prepare_model(&part.path, &options)
                .map_err(|e| e.to_string())?,
        );
        for attachment in &part.attachments {
            if let Some((path, material)) = super::attachments::model(attachment) {
                let options = ModelOptions {
                    material,
                    cry_space: true,
                    shading: Shading::Full,
                    ..Default::default()
                };
                models.push(
                    loader
                        .prepare_model(&path, &options)
                        .map_err(|e| e.to_string())?,
                );
            }
        }
    }
    let carried = weapons::carried(db, &source.equipment);
    for (path, material) in carried
        .iter()
        .flat_map(|item| weapons::model_files(&item.path))
    {
        let options = ModelOptions {
            material,
            cry_space: true,
            shading: Shading::Full,
            ..Default::default()
        };
        models.push(
            loader
                .prepare_model(&path, &options)
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(Prepared {
        assets,
        models,
        carried,
        wield: weapons::base_wield(db, &source.equipment),
    })
}

fn apply(world: &mut World, ready: Ready) {
    let Some(current) = source(world, ready.root) else {
        return;
    };
    // Recompute immediately before any finish_model, visual retirement or applied
    // marker mutation. Entity generation additionally fences removal/reuse.
    let Some(applied) = world.get::<Applied>(ready.root) else {
        return;
    };
    if current.key != ready.key || applied.key == ready.key {
        return;
    }
    let result = ready.result.and_then(|prepared| {
        let (rig, bones) = world
            .get::<Animator>(ready.root)
            .or_else(|| world.get::<Asleep>(ready.root).map(Asleep::animator))
            .map(|a| (a.rig.clone(), a.bones.clone()))
            .ok_or("missing current rig")?;
        let mut state = SystemState::<CryAssets>::new(world);
        {
            let mut cry = state.get_mut(world).map_err(|e| e.to_string())?;
            for model in prepared.models {
                cry.finish_model(model);
            }
        }
        state.apply(world);
        aa_anim::character::replace_character_parts(
            world,
            ready.root,
            &rig,
            &bones,
            prepared.assets,
        )
        .map_err(|e| e.to_string())?;
        if world
            .get::<RemoteUnit>(ready.root)
            .is_some_and(|remote| !remote.casts_shadow())
        {
            let model = world.get::<Character>(ready.root).unwrap().model_root;
            super::units::no_shadows(world, model);
        }
        let old: Vec<Entity> = world
            .query::<(Entity, &weapons::CarriedModelOwner)>()
            .iter(world)
            .filter_map(|(entity, owner)| (owner.0 == ready.root).then_some(entity))
            .collect();
        for entity in old {
            world.despawn(entity);
        }
        if !prepared.carried.iter().any(|item| item.staff) {
            world.entity_mut(ready.root).remove::<weapons::StaffSide>();
        }
        world
            .entity_mut(ready.root)
            .insert((weapons::Carried(prepared.carried), prepared.wield));
        Ok(())
    });
    match result {
        Ok(()) => {
            let mut applied = world.get_mut::<Applied>(ready.root).unwrap();
            applied.key = ready.key;
            applied.failed = None;
        }
        Err(error) => {
            warn!(
                "gear refresh {:?}: previous appearance retained: {error}",
                ready.root
            );
            if let Some(mut applied) = world.get_mut::<Applied>(ready.root) {
                applied.failed = Some(ready.key);
            }
        }
    }
}
pub(super) fn refresh(world: &mut World) {
    let Some(db) = world.get_resource::<Db>().cloned() else {
        return;
    };
    world.resource_scope(|world, mut jobs: Mut<Jobs>| {
        let ready: Vec<Ready> = {
            let rx = jobs.rx.lock().unwrap();
            rx.try_iter().collect()
        };
        for ready in ready {
            jobs.pending.remove(&ready.root);
            apply(world, ready);
        }
        // Keep a despawned actor's entry until its worker returns: retirement
        // must not create new permits while old work is still running.
        let roots: Vec<Entity> = world
            .query_filtered::<Entity, (With<Character>, With<Applied>)>()
            .iter(world)
            .collect();
        for root in roots {
            if jobs.pending.len() >= 2 {
                break;
            } // Application CPU-work bound.
            if jobs.pending.contains_key(&root) {
                continue;
            }
            let Some(source) = source(world, root) else {
                continue;
            };
            let applied = world.get::<Applied>(root).unwrap();
            if applied.key == source.key || applied.failed.as_ref() == Some(&source.key) {
                continue;
            }
            let mut state = SystemState::<CryAssets>::new(world);
            let loader = {
                let Ok(mut cry) = state.get_mut(world) else {
                    continue;
                };
                cry.loader()
            };
            state.apply(world);
            jobs.pending.insert(root, source.key.clone());
            let (db, tx) = (db.clone(), jobs.tx.clone());
            aa_cry::workers::Workers::global().spawn(0.0, move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    prepare(&db, &loader, &source)
                }))
                .unwrap_or_else(|_| Err("gear preparation panicked".into()));
                let _ = tx.send(Ready {
                    root,
                    key: source.key,
                    result,
                });
            });
        }
    });
}
pub(super) fn plugin(app: &mut App) {
    app.init_resource::<Jobs>()
        .configure_sets(Update, weapons::InGameCarried.before(weapons::hold_weapons))
        .add_systems(
            Update,
            refresh
                .in_set(Refresh)
                .after(crate::hud::HudInputReady)
                .after(super::units::sync_views)
                .after(super::player::spawn_player)
                .before(weapons::InGameCarried)
                .before(super::anim::drive_locomotion)
                .run_if(super::world_screens),
        );
}
pub(super) mod fixture;
#[cfg(test)]
mod tests;
