//! How many skills would look right in the client, from the data alone (no server, no window).
//!
//! Every skill a 1.2 player can learn (the ten ability trees `skills.ability_id` 1..=10, the basic skills and emotes of
//! `default_skills`) plus the skills of the low level monsters that spawn (`npcs.level <= 12`, `base_skill_id` and `np_skills`) is
//! resolved through the client's own `SkillBook` (`src/ingame/combat/skills.rs`, included below, so nothing is duplicated):
//!
//! * **Clips**: `start_anim`, `fire_anim`, `twohand_*`, `dual_wield_*`, `channeling_anim` and every anim of every plot step (the
//!   weapon attack clips of `holdables` for the auto attacks 2, 3 and 4, which the server picks per weapon). Each must be a name of
//!   the `.cal` of all 8 player models (`<race>/<gender>/nude/<pfx>_base.cal`, a monster: its own model), the `.caf` must be in the
//!   pak and must decode and bind to the model's skeleton. The `move_ub` variant is looked up the way the client does
//!   (`upper_candidates`), and the anim event database of the model must hold a `launch` / `combat_sync` event for the fire and
//!   plot clips (`ClipEvents::of`).
//! * **Effects**: every `fx_items.asset_name` of the skill's group, its projectile's group, its plot steps and its per level visual
//!   groups, looked up in the particle libraries with aa-fx's own loader (`Library::load` / `get`). The raw emitter XML of every
//!   effect that exists is scanned for what aa-fx does not do: an attribute `EmitterDef::from_attributes` never reads (found by
//!   feeding it test values: what aa-fx supports is read from its parser), and a collision (the ground has to be known: `GROUND_WIRED`).
//! * **Not particles**: sounds, voices, camera shakes, material flashes and the like in the effect groups.
//!
//! ```text
//! cargo run -p aa-client --example skill_coverage [-- --out data/extract/combat/skill_coverage.tsv]
//! ```
//!
//! Writes one row per skill and prints the summary of `docs/aa-client-combat.md`, "Coverage".

#[allow(dead_code)]
#[path = "../src/ingame/combat/clips.rs"]
mod clips;
#[allow(dead_code)]
#[path = "../src/ingame/combat/flash.rs"]
mod flash;
#[allow(dead_code)]
#[path = "../src/ingame/combat/skills.rs"]
mod skills;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::sync::{Arc, OnceLock};

use aa_anim::db::{Gender, Race};
use aa_anim::pose::{Rig, SampleOptions, shared_rig};
use aa_anim::{Cal, Cdf, ClipLibrary};
use aa_fx::{EmitterDef, Library, Spawn};
use clips::ClipEvents;
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, params};
use skills::{Extras, FxItem, SkillBook, ev, is_stop_clip, upper_candidates};

/// Monsters up to this level count as "near the starting areas".
const MOB_LEVEL: i64 = 12;

/// Features whose absence changes how an effect looks (a mesh, a distortion, a trail, an orbit, a bounce, an emission shape, a
/// decal). The others (a light, a sound, a rendering hint) leave the sprites as they are.
const MAJOR: &[&str] = &["geometry", "refraction", "tail", "rotate", "collision", "attach", "decal"];

/// Whether the client gives aa-fx a ground height (`FxSettings::ground`): until it does, particles that bounce on the ground (and the
/// emitters that start where they land) are reported as an unsupported `collision`. Meshes (`Geometry`), tails, orbits, refraction
/// materials and emission from the body are drawn by aa-fx itself and are not listed any more.
const GROUND_WIRED: bool = true;

/// Attributes that only hint at cost or visibility; the effect is not degraded by an engine that ignores them.
const HINTS: &[&str] = &[
    "FillRateCost",
    "DisableVisClipping",
    "EncodeVelocity",
    "MinPixels",
    "ViewDistanceAdjust",
    "DrawNear",
    "CameraMinDistance",
    // bouncing off objects and characters: the ground is enough for the look, and a particle's first hit is all that is reported
    "MaxCollisionEvents",
    "CollideStaticObjects",
    "CollideDynamicObjects",
    "CameraMinBorder",
    "CameraMaxBorder",
    "VisibleIndoors",
    "VisibleUnderwater",
    "CastShadows",
    "VirtualAge",
    "RemainWhileVisible",
    "NotAffectedByFog",
    "IgnoreAttractor",
    "MotionBlurScale",
    "LandMark",
];

// ------------------------------------------------------------------------------------------------------------------ models

/// A model whose `.cal` a clip must be in.
struct Model {
    tag: String,
    cal: Arc<Cal>,
    rig: Option<Arc<Rig>>,
}

fn clip_library() -> &'static ClipLibrary {
    static CLIPS: OnceLock<ClipLibrary> = OnceLock::new();
    CLIPS.get_or_init(ClipLibrary::default)
}

/// The 8 player models: nuian / elf / hariharan / ferre x male / female, the way `CharacterDesc::resolve` finds the `.cal`.
fn player_models() -> Vec<Model> {
    let mut out = Vec::new();
    for race in Race::ALL {
        for gender in Gender::ALL {
            let tag = format!("{}_{}", race.prefix(), gender.letter());
            let dir = format!("objects/characters/{}/{}/nude", race.folder(), gender.folder());
            let cdf = Cdf::load(&format!("{dir}/{tag}_base.cdf")).unwrap_or_else(|e| panic!("{tag}: {e}"));
            let cal = Cal::shared(&format!("{}.cal", cdf.model.trim_end_matches(".chr"))).unwrap_or_else(|e| panic!("{tag}: {e}"));
            out.push(Model { tag, cal, rig: shared_rig(&cdf.model).ok() });
        }
    }
    out
}

/// A monster's model (`models.id` -> `.cdf` / `.chr` -> `.cal` next to the skeleton, as `creature::spawn` does).
fn mob_model(db: &Connection, cache: &mut HashMap<u32, Option<Arc<Model>>>, model_id: u32) -> Option<Arc<Model>> {
    cache
        .entry(model_id)
        .or_insert_with(|| {
            let file: String = db
                .query_row(
                    "SELECT a.model_file FROM models m JOIN actor_models a ON a.id = m.sub_id AND m.sub_type = 'ActorModel' WHERE m.id = ?1",
                    params![model_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()?;
            let file = file.replace('\\', "/").to_ascii_lowercase();
            let main = if file.ends_with(".cdf") { Cdf::load(&file).ok()?.model } else { file };
            let cal = Cal::shared(&format!("{}.cal", main.trim_end_matches(".chr"))).ok()?;
            Some(Arc::new(Model { tag: format!("m{model_id}"), cal, rig: shared_rig(&main).ok() }))
        })
        .clone()
}

enum Presence {
    Ok,
    /// The model's `.cal` does not name the clip.
    NoCal,
    /// The `.cal` names it but the `.caf` is not in the pak.
    NoFile,
    /// The `.caf` does not decode or bind to the skeleton.
    Bad,
}

fn presence(m: &Model, name: &str) -> Presence {
    let Some(entry) = m.cal.animation(name) else { return Presence::NoCal };
    if !aa_pak::Pak::global().exists(&entry.path) {
        return Presence::NoFile;
    }
    let Some(rig) = &m.rig else { return Presence::Ok };
    let options = SampleOptions { position_scale: entry.position_scale(m.cal.base_height), lock_root_motion: true };
    if clip_library().bound(rig, &entry.path, options).is_ok() { Presence::Ok } else { Presence::Bad }
}

// ------------------------------------------------------------------------------------------------------------------ effects

/// What the particle libraries say about one effect.
#[derive(Default)]
struct FxInfo {
    found: bool,
    /// Why it is missing: "no library" or "not in library".
    why: &'static str,
    /// Emitters aa-fx draws.
    drawn: usize,
    /// Emitters that are switched on (`Enabled`): an effect whose emitters are all off is empty in the real client too.
    enabled: usize,
    /// Unsupported features: collision (no ground), light, sound, decal, geometry (`GeometryInPieces`), `other:<Attribute>`.
    features: BTreeSet<String>,
    /// How many enabled emitters carry each feature.
    feature_emitters: BTreeMap<String, usize>,
    /// Enabled emitters with at least one feature of [`MAJOR`].
    major_emitters: usize,
    /// Ignored attributes that only hint at cost or visibility.
    hints: BTreeSet<String>,
}

impl FxInfo {
    /// Looks different without the feature (see [`MAJOR`]).
    fn major(&self) -> bool {
        self.features.iter().any(|f| MAJOR.contains(&f.as_str()))
    }

    /// aa-fx draws none of its emitters although it has content it cannot show (a mesh, a trail, an orbit): invisible here, not in
    /// the client.
    fn invisible(&self) -> bool {
        self.found && self.drawn == 0 && self.major()
    }

    /// Nothing to draw: every emitter is switched off, has no texture, or only plays a sound or lights a light (an artists'
    /// variant, a sound carrier): empty in the real client too.
    fn empty(&self) -> bool {
        self.found && self.drawn == 0 && !self.major()
    }
}

struct LibData {
    lib: Library,
    /// Lower case effect name -> its `<Particles>` element (the last of equal names wins, as in `Library::parse`).
    index: HashMap<String, roxmltree::Node<'static, 'static>>,
}

#[derive(Default)]
struct Libs {
    libs: HashMap<String, Option<LibData>>,
    handled: HashMap<String, bool>,
    checked: HashMap<String, Arc<FxInfo>>,
}

impl Libs {
    fn load(&mut self, name: &str) {
        if self.libs.contains_key(name) {
            return;
        }
        let data = (|| {
            let lib = Library::load(name).ok()?;
            let text = aa_pak::Pak::global().read_string(&aa_fx::library::library_path(name)).ok()?;
            let text: &'static str = Box::leak(text.trim_start_matches('\u{feff}').to_string().into_boxed_str());
            let doc: &'static roxmltree::Document<'static> = Box::leak(Box::new(roxmltree::Document::parse(text).ok()?));
            let mut index = HashMap::new();
            for p in doc.root_element().children().filter(|c| c.has_tag_name("Particles")) {
                if let Some(n) = p.attribute("Name") {
                    index.insert(n.to_ascii_lowercase(), p);
                }
            }
            Some(LibData { lib, index })
        })();
        self.libs.insert(name.to_string(), data);
    }

    /// Whether `EmitterDef::from_attributes` reads the attribute at all: feeding it test values changes the emitter.
    fn handled(&mut self, attr: &str) -> bool {
        if let Some(h) = self.handled.get(attr) {
            return *h;
        }
        const VALUES: &[&str] = &["1", "2,3", "0.5", "true", "false", "Additive", "ParentStart", "Free", "Velocity", "Medium", "Physics", "Volume", "SimpleCollision", "textures/x.dds", "materials/x", "(0,1;1,2)", "1,2,3", "x.cgf"];
        let build = |value: Option<&str>| {
            let xml = match value {
                Some(v) => format!("<P {attr}=\"{v}\"/>"),
                None => "<P/>".to_string(),
            };
            let doc = roxmltree::Document::parse(&xml).expect("probe xml");
            format!("{:?}", EmitterDef::from_attributes("p", doc.root_element().attributes()))
        };
        let base = build(None);
        let h = VALUES.iter().any(|v| build(Some(v)) != base);
        self.handled.insert(attr.to_string(), h);
        h
    }

    /// The unsupported features of one emitter and the ones below it.
    fn walk(&mut self, el: roxmltree::Node<'static, 'static>, info: &mut FxInfo) {
        if let Some(params) = el.children().find(|c| c.has_tag_name("Params")) {
            let def = EmitterDef::from_attributes(el.attribute("Name").unwrap_or(""), params.attributes());
            if def.enabled {
                info.enabled += 1;
                let mut mine: BTreeSet<String> = BTreeSet::new();
                if !GROUND_WIRED && (def.collides() || matches!(def.spawn, Spawn::ParentCollide)) {
                    mine.insert("collision".into());
                }
                for a in params.attributes() {
                    let name = a.name();
                    if name == "Geometry" || self.handled(name) || trivial(name, a.value()) {
                        continue;
                    }
                    if HINTS.contains(&name) {
                        info.hints.insert(name.to_string());
                    } else {
                        mine.insert(bucket(name));
                    }
                }
                info.major_emitters += usize::from(mine.iter().any(|f| MAJOR.contains(&f.as_str())));
                for f in mine {
                    *info.feature_emitters.entry(f.clone()).or_default() += 1;
                    info.features.insert(f);
                }
            }
        }
        for child in el.children().filter(|c| c.has_tag_name("Childs")).flat_map(|c| c.children().filter(|p| p.has_tag_name("Particles"))) {
            self.walk(child, info);
        }
    }

    fn check(&mut self, effect: &str) -> Arc<FxInfo> {
        let key = effect.to_ascii_lowercase();
        if let Some(i) = self.checked.get(&key) {
            return i.clone();
        }
        let mut info = FxInfo::default();
        match aa_fx::split_name(&key) {
            None => info.why = "not library.name",
            Some((lib, rest)) => {
                self.load(lib);
                let found = self.libs[lib].as_ref().map(|d| (d.lib.get(rest).map(|e| e.drawn()), d.index.get(rest).copied()));
                match found {
                    None => info.why = "no library",
                    Some((None, _)) => info.why = "not in library",
                    Some((Some(drawn), node)) => {
                        info.found = true;
                        info.drawn = drawn;
                        if let Some(node) = node {
                            self.walk(node, &mut info);
                        }
                    }
                }
            }
        }
        let info = Arc::new(info);
        self.checked.insert(key, info.clone());
        info
    }
}

/// An attribute value that changes nothing: empty, `false`, `none`, or zero. A variable value reads `base,variation,curves` and the
/// variation only scales the base (`base * (1 - variation * rand)`), so only the base counts; a vector (`x,y,z`) counts all of it.
fn trivial(name: &str, v: &str) -> bool {
    let t = v.trim();
    if t.is_empty() || matches!(t.to_ascii_lowercase().as_str(), "false" | "none" | "no") {
        return true;
    }
    let head = t.split('(').next().unwrap_or(t);
    let zero = |p: &str| p.trim().is_empty() || p.trim().parse::<f32>().is_ok_and(|f| f == 0.0);
    match name {
        "RotatePosition" => head.split(',').all(zero),
        // an empty base is the default, which is 1
        "SizeZ" => head.split(',').next().is_some_and(|p| !p.trim().is_empty() && zero(p)),
        _ => head.split(',').next().is_none_or(zero),
    }
}

/// The feature an ignored attribute belongs to.
fn bucket(name: &str) -> String {
    match name {
        n if n.starts_with("Tail") => "tail".into(),
        n if n.starts_with("LightSource") || n.starts_with("LightHDR") => "light".into(),
        "PhysicsType" | "Bounciness" | "DynamicFriction" | "MaxCollisionEvents" | "CollideTerrain" | "CollideStaticObjects" | "CollideDynamicObjects" => "collision".into(),
        n if n.starts_with("Rotate") || n == "NormalizeRotateLength" => "rotate".into(),
        "Sound" | "SoundFXParam" => "sound".into(),
        n if n.starts_with("Decal") || n.starts_with("Defer") => "decal".into(),
        "AttachType" | "AttachForm" => "attach".into(),
        "GeometryInPieces" => "geometry".into(),
        other => format!("other:{other}"),
    }
}

// ------------------------------------------------------------------------------------------------------------------ one skill

/// One clip a skill needs, and how the models fare with it.
#[derive(Default)]
struct ClipRow {
    roles: BTreeSet<&'static str>,
    /// Models without a usable clip (no `.cal` entry, no `.caf`, or it does not decode).
    missing: Vec<String>,
    no_cal: usize,
    no_file: usize,
    bad: usize,
    /// `anims.move_ub` of the clip, and the models where none of the names the client tries exists.
    ub: Option<String>,
    ub_missing: Vec<String>,
    /// The data says the clip loops (`anims.loop`): the client loops it until the next clip or the end of the skill.
    loops: bool,
    /// The skill (or the plot step) starts effects or a projectile at the launch of this clip.
    launch_timed: bool,
    /// Models whose anim event database has neither `launch` nor `combat_sync` for a launch timed clip (the client then falls back
    /// to `effect_delay`, 0 in a plot).
    no_release: Vec<String>,
    /// Models that lack the `launch` event of a launch timed clip (`combat_sync` stands in for it).
    no_launch: Vec<String>,
    /// Models without `combat_sync` for an attack clip: a swing that deals damage at once then lands at the fire.
    no_sync: Vec<String>,
}

struct Row {
    id: u32,
    name: String,
    set: String,
    ability: i64,
    shown: bool,
    plot: bool,
    steps: usize,
    steps_with_anim: usize,
    steps_two_anims: usize,
    steps_stop_only: usize,
    cast_ms: i64,
    chan_ms: i64,
    clips: BTreeMap<String, ClipRow>,
    /// `all_co_sk_stop` / `all_ac_stop`: markers that end the clip before, not clips.
    stops: BTreeSet<String>,
    /// Lower case effect names (without the buff visuals the client leaves out).
    fx: BTreeSet<String>,
    /// The ones that fly (projectile groups).
    proj_fx: BTreeSet<String>,
    extras: Extras,
    models: usize,
}

impl Row {
    fn channel_clip(&self) -> bool {
        self.clips.values().any(|c| c.roles.contains("channel"))
    }

    fn channeling(&self) -> bool {
        self.chan_ms > 0 || self.channel_clip()
    }
}

fn flag(r: &rusqlite::Row, i: usize) -> bool {
    match r.get::<_, Value>(i) {
        Ok(Value::Integer(n)) => n != 0,
        Ok(Value::Real(f)) => f != 0.0,
        Ok(Value::Text(t)) => matches!(t.as_str(), "t" | "true" | "1"),
        _ => false,
    }
}

struct Ctx<'a> {
    book: &'a SkillBook,
    db: &'a Connection,
    names: &'a HashMap<u32, String>,
    /// Clips with `anims.loop` set.
    looping: &'a BTreeSet<String>,
}

impl Ctx<'_> {
    /// The weapon attack clips the server picks for the auto attacks (`Skill.GetWeaponAttackAnimId`: skill 2 main hand, 3 off hand,
    /// 4 ranged), from `holdables` of the weapons that exist.
    fn weapon_clips(&self, skill: u32) -> Vec<Arc<str>> {
        let cols = match skill {
            2 => "h.anim_r1_id, h.anim_r2_id, h.anim_r3_id",
            3 => "h.anim_l1_id, h.anim_l2_id, h.anim_l3_id",
            4 => "h.anim_r1_id, h.anim_r2_id, h.anim_r3_id",
            _ => return Vec::new(),
        };
        let filter = if skill == 4 { "AND h.slot_type_id = 18" } else { "" };
        let ids: BTreeSet<u32> = self
            .db
            .prepare(&format!("SELECT {cols} FROM holdables h WHERE h.id IN (SELECT holdable_id FROM item_weapons) {filter}"))
            .and_then(|mut q| {
                q.query_map([], |r| Ok([0, 1, 2].map(|i| r.get::<_, Option<i64>>(i).ok().flatten().unwrap_or(0) as u32))).map(|rows| rows.flatten().flatten().collect())
            })
            .unwrap_or_default();
        // monsters bite with the fist clips 1 / 2 whatever they hold
        ids.into_iter().chain([1, 2].into_iter().filter(|_| skill == 2)).filter_map(|id| self.book.anim(id)).collect::<BTreeSet<_>>().into_iter().collect()
    }

    /// `anims` ids of the instrument clips (`string_` / `percussion_` / `tube_instrument_start_` / `fire_anim_id`) and of the skill
    /// controller (`start`, `end`, two transitions) of a skill.
    fn extra_anims(&self, id: u32) -> Vec<(u32, &'static str)> {
        self.db
            .query_row(
                "SELECT s.string_instrument_start_anim_id, s.percussion_instrument_start_anim_id, s.tube_instrument_start_anim_id,                         s.string_instrument_fire_anim_id, s.percussion_instrument_fire_anim_id, s.tube_instrument_fire_anim_id,                         c.start_anim_id, c.end_anim_id, c.transition_anim_1_id, c.transition_anim_2_id                  FROM skills s LEFT JOIN skill_controllers c ON c.id = s.skill_controller_id WHERE s.id = ?1",
                params![id],
                |r| {
                    Ok((0..10)
                        .map(|i| (match r.get::<_, Value>(i) { Ok(Value::Integer(n)) => n as u32, _ => 0 }, if i < 6 { "instrument" } else { "controller" }))
                        .filter(|(a, _)| *a != 0)
                        .collect())
                },
            )
            .unwrap_or_default()
    }

    fn resolve(&self, id: u32, set: &str, models: &[Arc<Model>]) -> Option<Row> {
        let v = self.book.skill(id)?;
        let (cast_ms, chan_ms, ability, shown, ko) = self
            .db
            .query_row("SELECT casting_time, channeling_time, ability_id, show, name FROM skills WHERE id = ?1", params![id], |r| {
                Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, Option<i64>>(1)?.unwrap_or(0), r.get::<_, Option<i64>>(2)?.unwrap_or(0), flag(r, 3), r.get::<_, Option<String>>(4)?.unwrap_or_default()))
            })
            .ok()?;
        let steps = if v.plot { self.book.plot_steps(id) } else { Vec::new() };
        let mut row = Row {
            id,
            name: self.names.get(&id).cloned().unwrap_or(ko),
            set: set.to_string(),
            ability,
            shown,
            plot: v.plot,
            steps: steps.len(),
            steps_with_anim: 0,
            steps_two_anims: 0,
            steps_stop_only: 0,
            cast_ms,
            chan_ms,
            clips: BTreeMap::new(),
            stops: BTreeSet::new(),
            fx: BTreeSet::new(),
            proj_fx: BTreeSet::new(),
            extras: v.extras,
            models: models.len(),
        };

        // the clips: the cast clip matters when the skill casts (plot steps cast on their own)
        let mut need: BTreeMap<String, BTreeSet<&'static str>> = BTreeMap::new();
        let mut launch_timed: BTreeSet<String> = BTreeSet::new();
        let mut stops: BTreeSet<String> = BTreeSet::new();
        let mut add = |name: &str, role: &'static str| {
            if is_stop_clip(name) {
                stops.insert(name.to_string());
            } else {
                need.entry(name.to_string()).or_default().insert(role);
            }
        };
        if cast_ms > 0 || v.plot {
            v.start_anim.iter().for_each(|n| add(n, "start"));
        }
        v.fire_anim.iter().for_each(|n| add(n, "fire"));
        v.twohand_fire_anim.iter().for_each(|n| add(n, "twohand"));
        v.dual_fire_anim.iter().for_each(|n| add(n, "dual"));
        // a skill that is no plot starts its launch effects and its projectile at the release of the fire clip
        if !v.plot && (v.projectile.is_some() || v.fx.iter().any(|i| i.start == ev::LAUNCH)) {
            launch_timed.extend([&v.fire_anim, &v.twohand_fire_anim, &v.dual_fire_anim].into_iter().flatten().map(|n| n.to_string()));
        }
        v.channeling_anim.iter().for_each(|n| add(n, "channel"));
        self.weapon_clips(id).iter().for_each(|n| add(n, "weapon"));
        // the instruments' (music skills) and the skill controller's (a stance, a charge) anims
        for (anim, role) in self.extra_anims(id) {
            self.book.anim(anim).iter().for_each(|n| add(n, role));
        }

        // the effects
        let mut fx: Vec<Arc<Vec<FxItem>>> = vec![v.fx.clone()];
        let mut flying: Vec<Arc<Vec<FxItem>>> = Vec::new();
        let mut extras = v.extras;
        if let Some(p) = &v.projectile {
            flying.push(p.items.clone());
            extras = extras.plus(p.extras);
        }
        for step in &steps {
            let Some(p) = self.book.plot_event(*step) else { continue };
            row.steps_with_anim += usize::from(!p.anims.is_empty());
            row.steps_two_anims += usize::from(p.anims.len() > 1);
            row.steps_stop_only += usize::from(!p.anims.is_empty() && p.anims.iter().all(|a| is_stop_clip(a)));
            p.anims.iter().for_each(|n| add(n, "step"));
            // a plot step times its launch effects and its projectile by the clip it plays
            if let Some(a) = &p.anim
                && (p.projectile.is_some() || p.fx.iter().any(|i| i.start == ev::LAUNCH))
            {
                launch_timed.insert(a.to_string());
            }
            fx.push(p.fx.clone());
            extras = extras.plus(p.extras);
            if let Some(proj) = &p.projectile {
                flying.push(proj.items.clone());
                extras = extras.plus(proj.extras);
            }
        }
        // alternative visuals by skill level (the client picks one by the caster's level)
        let levels: Vec<(u32, u32)> = self
            .db
            .prepare("SELECT COALESCE(fx_group_id, 0), COALESCE(projectile_id, 0) FROM skill_visual_groups WHERE owner_type = 'Skill' AND owner_id = ?1")
            .and_then(|mut q| q.query_map(params![id], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32))).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        for (group, projectile) in levels {
            fx.push(self.book.fx_group(group));
            extras = extras.plus(self.book.fx_extras(group));
            if let Some(p) = self.book.projectile(projectile) {
                flying.push(p.items.clone());
                extras = extras.plus(p.extras);
            }
        }
        row.extras = extras;
        row.stops = stops;
        // buff visuals (`_dot`) belong to the buff: the client leaves them out
        let names = |g: &[Arc<Vec<FxItem>>]| g.iter().flat_map(|g| g.iter()).map(|i| i.effect.to_ascii_lowercase()).filter(|e| !e.is_empty() && !e.contains("_dot")).collect::<BTreeSet<_>>();
        row.proj_fx = names(&flying);
        row.fx = names(&fx).union(&row.proj_fx).cloned().collect();

        for (name, roles) in need {
            let mut c = ClipRow { roles, ..Default::default() };
            let upper = self.book.upper_clip(&name);
            let attack = c.roles.iter().any(|r| matches!(*r, "fire" | "twohand" | "dual" | "weapon"));
            c.launch_timed = launch_timed.contains(&name);
            c.loops = self.looping.contains(&name);
            for m in models {
                match presence(m, &name) {
                    Presence::Ok => {
                        let e = ClipEvents::of(&m.cal, &name);
                        if c.launch_timed && e.release().is_none() {
                            c.no_release.push(m.tag.clone());
                        }
                        if c.launch_timed && e.launch.is_none() {
                            c.no_launch.push(m.tag.clone());
                        }
                        if attack && e.combat_sync.is_none() {
                            c.no_sync.push(m.tag.clone());
                        }
                    }
                    Presence::NoCal => {
                        c.no_cal += 1;
                        c.missing.push(m.tag.clone());
                    }
                    Presence::NoFile => {
                        c.no_file += 1;
                        c.missing.push(m.tag.clone());
                    }
                    Presence::Bad => {
                        c.bad += 1;
                        c.missing.push(m.tag.clone());
                    }
                }
                if let Some(u) = &upper
                    && !upper_candidates(u).any(|n| matches!(presence(m, &n), Presence::Ok))
                {
                    c.ub_missing.push(m.tag.clone());
                }
            }
            c.ub = upper.map(|u| u.to_string());
            row.clips.insert(name, c);
        }
        Some(row)
    }
}

// ------------------------------------------------------------------------------------------------------------------ statistics

/// How a skill's effects fare.
#[derive(PartialEq, Eq, Clone, Copy)]
enum FxClass {
    /// No particle effects.
    None,
    /// All found, nothing aa-fx ignores.
    Plain,
    /// All found; aa-fx ignores a light, a sound or a rendering detail only.
    Minor,
    /// All found; at least one effect looks different (mesh, refraction, trail, orbit, bounce, shape, decal) or is invisible.
    Degraded,
    /// At least one effect is not in any library.
    Missing,
}

impl FxClass {
    fn label(self) -> &'static str {
        match self {
            FxClass::None => "none",
            FxClass::Plain => "plain",
            FxClass::Minor => "minor",
            FxClass::Degraded => "degraded",
            FxClass::Missing => "missing",
        }
    }
}

type Fx = HashMap<String, Arc<FxInfo>>;

fn fx_class(row: &Row, fx: &Fx) -> FxClass {
    if row.fx.is_empty() {
        FxClass::None
    } else if row.fx.iter().any(|e| !fx[e].found) {
        FxClass::Missing
    } else if row.fx.iter().any(|e| fx[e].major()) {
        FxClass::Degraded
    } else if row.fx.iter().any(|e| !fx[e].features.is_empty()) {
        FxClass::Minor
    } else {
        FxClass::Plain
    }
}

fn features_of(row: &Row, fx: &Fx) -> BTreeSet<String> {
    row.fx.iter().flat_map(|e| fx[e].features.iter().cloned()).collect()
}

/// The counted rows of one column of the summary table.
fn stats(rows: &[&Row], fx: &Fx, tags: &[String], per_model: bool) -> Vec<(String, String)> {
    let n = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count().to_string();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut put = |k: &str, v: String| out.push((k.to_string(), v));
    put("skills", rows.len().to_string());
    put("  with clips", n(&|r| !r.clips.is_empty()));
    put("  with particle effects", n(&|r| !r.fx.is_empty()));
    put("  casting (casting_time > 0)", n(&|r| r.cast_ms > 0));
    put("  channeling (a channel time or clip)", n(&|r| r.channeling()));
    put("    with a real channel clip", n(&|r| r.channel_clip()));
    put("  plot skills", n(&|r| r.plot));
    put("CLIPS: all clips on all models", n(&|r| !r.clips.is_empty() && r.clips.values().all(|c| c.missing.is_empty())));
    put("  >= 1 clip missing on some model", n(&|r| r.clips.values().any(|c| !c.missing.is_empty())));
    put("  >= 1 clip missing on every model", n(&|r| r.clips.values().any(|c| c.missing.len() == r.models && r.models > 0)));
    if per_model {
        for t in tags {
            put(&format!("  missing a clip on {t}"), n(&|r| r.clips.values().any(|c| c.missing.contains(t))));
        }
    }
    put("  uses a stop marker (no clip)", n(&|r| !r.stops.is_empty()));
    put("  >= 1 move_ub variant unresolved", n(&|r| r.clips.values().any(|c| c.ub.is_some() && !c.ub_missing.is_empty())));
    put("  fire/step clip loops in the data", n(&|r| r.clips.values().any(|c| c.loops && c.roles.iter().any(|x| matches!(*x, "fire" | "twohand" | "dual" | "step")))));
    put("ANIM EVENTS: launch timed clips", n(&|r| r.clips.values().any(|c| c.launch_timed)));
    put("  ... without launch or combat_sync event", n(&|r| r.clips.values().any(|c| !c.no_release.is_empty())));
    put("  ... without a launch event (sync stands in)", n(&|r| r.clips.values().any(|c| !c.no_launch.is_empty())));
    put("  attack clip without combat_sync", n(&|r| r.clips.values().any(|c| !c.no_sync.is_empty())));
    put("EFFECTS: skills with effects", n(&|r| !r.fx.is_empty()));
    put("  all found, plain (nothing ignored)", n(&|r| fx_class(r, fx) == FxClass::Plain));
    put("  all found, minor loss (light/sound/detail)", n(&|r| fx_class(r, fx) == FxClass::Minor));
    put("  all found, looks different (see features)", n(&|r| fx_class(r, fx) == FxClass::Degraded));
    put("  >= 1 effect missing", n(&|r| fx_class(r, fx) == FxClass::Missing));
    put("  >= 1 effect invisible in aa-fx (mesh/trail only)", n(&|r| r.fx.iter().any(|e| fx[e].invisible())));
    put("  >= 1 effect has nothing to draw (off/sound/light)", n(&|r| r.fx.iter().any(|e| fx[e].empty())));
    put("NOT PARTICLES: any sound/voice/shake/...", n(&|r| !r.extras.is_empty() || features_of(r, fx).contains("sound")));
    put("  any sound (all three kinds below)", n(&|r| r.extras.sound > 0 || r.extras.particle_sound > 0 || features_of(r, fx).contains("sound")));
    put("    FxSound item", n(&|r| r.extras.sound > 0));
    put("    sound bound to a particle item", n(&|r| r.extras.particle_sound > 0));
    put("    emitter Sound attribute", n(&|r| features_of(r, fx).contains("sound")));
    put("  voice (FxVoice)", n(&|r| r.extras.voice > 0));
    put("  camera shake", n(&|r| r.extras.shake > 0));
    put("  material flash", n(&|r| r.extras.material > 0));
    put("  model / rope / fov / blur", n(&|r| r.extras.other > 0));
    out
}

fn tsv(rows: &[Row], fx: &Fx, tags: &[String]) -> String {
    let mut s = String::new();
    s.push_str("id\tname\tset\tability\tshown\tplot\tsteps\tcast_ms\tchan_ms\tmodels\tclips\tclips_missing_n\tclips_missing");
    for t in tags {
        let _ = write!(s, "\tmiss_{t}");
    }
    s.push_str("\tstops\tub_unresolved\tloops\tno_release\tno_launch\tno_sync\tfx_n\tfx_class\tfx_missing\tfx_major\tfx_features\tfx_hints\tfx_invisible");
    s.push_str("\tsound\tparticle_sound\tvoice\tshake\tmaterial\tother\tfx_names\n");
    for r in rows {
        let join = |it: &mut dyn Iterator<Item = String>| it.collect::<Vec<_>>().join(";");
        let clip_list = join(&mut r.clips.iter().map(|(n, c)| format!("{n}[{}]", c.roles.iter().copied().collect::<Vec<_>>().join(","))));
        let missing = join(&mut r.clips.iter().filter(|(_, c)| !c.missing.is_empty()).map(|(n, c)| {
            if c.missing.len() == r.models { format!("{n}:all") } else { format!("{n}:{}", c.missing.join(",")) }
        }));
        let _ = write!(s, "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}", r.id, r.name, r.set, r.ability, r.shown as u8, r.plot as u8, r.steps, r.cast_ms, r.chan_ms, r.models, clip_list, r.clips.values().filter(|c| !c.missing.is_empty()).count(), missing);
        for t in tags {
            let n = r.clips.values().filter(|c| c.missing.contains(t)).count();
            let _ = write!(s, "\t{n}");
        }
        let stops = join(&mut r.stops.iter().cloned());
        let ub = join(&mut r.clips.iter().filter(|(_, c)| c.ub.is_some() && !c.ub_missing.is_empty()).map(|(n, c)| format!("{n}>{}:{}", c.ub.as_deref().unwrap_or(""), c.ub_missing.len())));
        let no_release = join(&mut r.clips.iter().filter(|(_, c)| !c.no_release.is_empty()).map(|(n, c)| format!("{n}:{}", c.no_release.len())));
        let no_launch = join(&mut r.clips.iter().filter(|(_, c)| !c.no_launch.is_empty()).map(|(n, c)| format!("{n}:{}", c.no_launch.len())));
        let no_sync = join(&mut r.clips.iter().filter(|(_, c)| !c.no_sync.is_empty()).map(|(n, c)| format!("{n}:{}", c.no_sync.len())));
        let loops = join(&mut r.clips.iter().filter(|(_, c)| c.loops).map(|(n, _)| n.clone()));
        let fx_missing = join(&mut r.fx.iter().filter(|e| !fx[*e].found).cloned());
        let major = join(&mut r.fx.iter().filter(|e| fx[*e].major()).cloned());
        let features = join(&mut features_of(r, fx).into_iter());
        let hints = join(&mut r.fx.iter().flat_map(|e| fx[e].hints.iter().cloned()).collect::<BTreeSet<_>>().into_iter());
        let invisible = join(&mut r.fx.iter().filter(|e| fx[*e].invisible()).cloned());
        let _ = write!(s, "\t{stops}\t{ub}\t{loops}\t{no_release}\t{no_launch}\t{no_sync}\t{}\t{}\t{fx_missing}\t{major}\t{features}\t{hints}\t{invisible}", r.fx.len(), fx_class(r, fx).label());
        let e = r.extras;
        let names = join(&mut r.fx.iter().cloned());
        let _ = writeln!(s, "\t{}\t{}\t{}\t{}\t{}\t{}\t{names}", e.sound, e.particle_sound, e.voice, e.shake, e.material, e.other);
    }
    s
}

fn print_columns(cols: &[(&str, Vec<(String, String)>)]) {
    let width = 15;
    let mut head = format!("{:<50}", "");
    for (name, _) in cols {
        let _ = write!(head, "{name:>width$}");
    }
    println!("{head}");
    for (label, _) in &cols[0].1 {
        let mut line = format!("{label:<50}");
        for (_, c) in cols {
            let v = c.iter().find(|(k, _)| k == label).map(|(_, v)| v.as_str()).unwrap_or("-");
            let _ = write!(line, "{v:>width$}");
        }
        println!("{line}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| format!("{}/../../data/extract/combat/skill_coverage.tsv", env!("CARGO_MANIFEST_DIR")));
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("skill_coverage needs the game_pak and the game database");
        return;
    }
    let t0 = std::time::Instant::now();
    let book = SkillBook::open().expect("game database");
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).expect("database");
    let names: HashMap<u32, String> = db
        .prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'skills' AND tbl_column_name = 'name' AND en_us IS NOT NULL AND en_us <> ''")
        .and_then(|mut q| q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?))).map(|rows| rows.flatten().collect()))
        .unwrap_or_default();
    let looping: BTreeSet<String> = db
        .prepare("SELECT name, loop FROM anims")
        .and_then(|mut q| q.query_map([], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default().trim().to_string(), flag(r, 1)))).map(|rows| rows.flatten().filter(|(_, l)| *l).map(|(n, _)| n).collect()))
        .unwrap_or_default();
    let ctx = Ctx { book: &book, db: &db, names: &names, looping: &looping };

    // --- the skill sets
    let mut sets: Vec<(u32, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut q = db.prepare("SELECT id, ability_id FROM skills WHERE ability_id BETWEEN 1 AND 10 ORDER BY ability_id, id").unwrap();
    for (id, ability) in q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)?))).unwrap().flatten() {
        if seen.insert(id) {
            sets.push((id, format!("tree{ability}")));
        }
    }
    let mut q = db.prepare("SELECT skill_id, skill_book_category_id FROM default_skills ORDER BY skill_id").unwrap();
    for (id, category) in q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)?))).unwrap().flatten() {
        if seen.insert(id) {
            sets.push((id, if category == 5 { "emote" } else { "basic" }.to_string()));
        }
    }

    // --- player skills against the 8 player models
    let models: Vec<Arc<Model>> = player_models().into_iter().map(Arc::new).collect();
    let tags: Vec<String> = models.iter().map(|m| m.tag.clone()).collect();
    let mut rows: Vec<Row> = Vec::new();
    let mut unknown = 0;
    for (id, set) in &sets {
        match ctx.resolve(*id, set, &models) {
            Some(r) => rows.push(r),
            None => unknown += 1,
        }
    }
    let player_rows = rows.len();

    // --- monsters: the skills of the spawned low level NPCs, against the models that use them
    let mut mob_cache = HashMap::new();
    let mut mob_skills: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
    {
        let mut q = db
            .prepare(
                "SELECT n.id, n.model_id, COALESCE(n.base_skill_id, 0) FROM npcs n WHERE n.level BETWEEN 1 AND ?1 \
                 AND n.id IN (SELECT member_id FROM npc_spawner_npcs WHERE member_type = 'Npc')",
            )
            .unwrap();
        let npcs: Vec<(u32, u32, u32)> = q.query_map(params![MOB_LEVEL], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32, r.get::<_, i64>(2)? as u32))).unwrap().flatten().collect();
        let mut sq = db.prepare("SELECT skill_id FROM np_skills WHERE owner_type = 'Npc' AND owner_id = ?1").unwrap();
        for (npc, model, base) in npcs {
            let mut skills: BTreeSet<u32> = sq.query_map(params![npc], |r| Ok(r.get::<_, i64>(0)? as u32)).unwrap().flatten().collect();
            skills.insert(base);
            for s in skills.into_iter().filter(|s| *s != 0 && !seen.contains(s)) {
                mob_skills.entry(s).or_default().insert(model);
            }
        }
    }
    for (id, model_ids) in &mob_skills {
        let ms: Vec<Arc<Model>> = model_ids.iter().filter_map(|m| mob_model(&db, &mut mob_cache, *m)).collect();
        if ms.is_empty() {
            continue;
        }
        if let Some(r) = ctx.resolve(*id, "mob", &ms) {
            rows.push(r);
        }
    }

    // --- effects
    let mut libs = Libs::default();
    let mut fx: Fx = HashMap::new();
    for r in &rows {
        for e in &r.fx {
            fx.entry(e.clone()).or_insert_with(|| libs.check(e));
        }
    }

    // --- output
    let path = std::path::Path::new(&out);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).expect("output directory");
    }
    std::fs::write(path, tsv(&rows, &fx, &tags)).expect("write tsv");

    let players: Vec<&Row> = rows[..player_rows].iter().collect();
    let tree: Vec<&Row> = players.iter().copied().filter(|r| r.set.starts_with("tree") && r.shown).collect();
    let basic: Vec<&Row> = players.iter().copied().filter(|r| !r.set.starts_with("tree")).collect();
    let mobs: Vec<&Row> = rows[player_rows..].iter().collect();
    println!("skill_coverage: {} player skills ({} unknown to the database), {} monster skills, {} effects, in {:.1} s", player_rows, unknown, mobs.len(), fx.len(), t0.elapsed().as_secs_f32());
    println!("wrote {} ({} rows)\n", path.display(), rows.len());
    println!("sets: {}", {
        let mut c: BTreeMap<&str, usize> = BTreeMap::new();
        for r in &players {
            *c.entry(r.set.as_str()).or_default() += 1;
        }
        c.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ")
    });
    println!("tree skills hidden from the skill window (sub skills of other skills): {}\n", players.iter().filter(|r| r.set.starts_with("tree") && !r.shown).count());
    print_columns(&[
        ("player (all)", stats(&players, &fx, &tags, true)),
        ("tree, shown", stats(&tree, &fx, &tags, true)),
        ("basic + emote", stats(&basic, &fx, &tags, true)),
        ("monsters", stats(&mobs, &fx, &tags, false)),
    ]);

    // --- clips
    println!("\n== clips of the player skills");
    type ClipUse = (BTreeSet<u32>, usize, usize, usize, usize, BTreeSet<&'static str>);
    let mut clip_use: BTreeMap<&str, ClipUse> = BTreeMap::new();
    for r in &players {
        for (name, c) in &r.clips {
            let e = clip_use.entry(name.as_str()).or_default();
            e.0.insert(r.id);
            e.1 = e.1.max(c.missing.len());
            e.2 += c.no_cal.min(1);
            e.3 += c.no_file.min(1);
            e.4 += c.bad.min(1);
            e.5.extend(c.roles.iter().copied());
        }
    }
    let mut missing_clips: Vec<(&str, usize, usize)> = clip_use.iter().filter(|(_, e)| e.1 > 0).map(|(n, e)| (*n, e.0.len(), e.1)).collect();
    missing_clips.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!(
        "distinct clips needed {}, missing on >= 1 model {} (no .cal entry {}, .caf not in the pak {}, does not decode or bind {}), missing on all {} models {}",
        clip_use.len(),
        missing_clips.len(),
        clip_use.values().filter(|e| e.2 > 0).count(),
        clip_use.values().filter(|e| e.3 > 0).count(),
        clip_use.values().filter(|e| e.4 > 0).count(),
        models.len(),
        missing_clips.iter().filter(|c| c.2 == models.len()).count()
    );
    let mut stop_use: BTreeMap<&str, usize> = BTreeMap::new();
    for r in &players {
        for s in &r.stops {
            *stop_use.entry(s.as_str()).or_default() += 1;
        }
    }
    println!("stop markers (not clips; the .cal has no entry): {} (skills using each)", stop_use.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "));
    let ub_rows: BTreeSet<&str> = players.iter().flat_map(|r| r.clips.iter()).filter(|(_, c)| c.ub.is_some()).map(|(n, _)| n.as_str()).collect();
    let ub_gap: BTreeMap<&str, (String, usize)> = players
        .iter()
        .flat_map(|r| r.clips.iter())
        .filter(|(_, c)| c.ub.is_some() && !c.ub_missing.is_empty())
        .map(|(n, c)| (n.as_str(), (c.ub.clone().unwrap_or_default(), c.ub_missing.len())))
        .collect();
    println!("clips with a move_ub row {}, of which unresolved on >= 1 model {} (the client then plays the clip itself through the upper body mask):", ub_rows.len(), ub_gap.len());
    println!("  {}", ub_gap.iter().map(|(n, (u, m))| format!("{n}>{u}({m}/{} models)", models.len())).collect::<Vec<_>>().join("  "));
    println!("\n== top 20 missing clips (player skills that need them, models missing it, roles)");
    if missing_clips.is_empty() {
        println!("none");
    }
    for (name, skills, models_missing) in missing_clips.iter().take(20) {
        println!("{skills:>4} skills  {models_missing}/{} models  {name}  [{}]", models.len(), clip_use[name].5.iter().copied().collect::<Vec<_>>().join(","));
    }
    let mut mob_missing: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for r in &mobs {
        for (n, _) in r.clips.iter().filter(|(_, c)| !c.missing.is_empty()) {
            mob_missing.entry(n.as_str()).or_default().insert(r.id);
        }
    }
    let mut mob_missing: Vec<(&str, usize)> = mob_missing.iter().map(|(n, s)| (*n, s.len())).collect();
    mob_missing.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("\n== top 20 missing clips of the monster skills (skills, clip; missing on the models that use the skill)");
    for (name, skills) in mob_missing.iter().take(20) {
        println!("{skills:>4} skills  {name}");
    }
    if mob_missing.is_empty() {
        println!("none");
    }
    println!("\n== launch timed clips of the shown tree skills without a launch or combat_sync event (clip: models)");
    for r in &tree {
        let gaps: Vec<String> = r.clips.iter().filter(|(_, c)| !c.no_release.is_empty()).map(|(n, c)| format!("{n}:{}", c.no_release.len())).collect();
        if !gaps.is_empty() {
            println!("{:>6} {:<28} {}", r.id, r.name, gaps.join(" "));
        }
    }
    let loop_clips: BTreeSet<&str> = players.iter().flat_map(|r| r.clips.iter()).filter(|(_, c)| c.loops && c.roles.iter().any(|x| matches!(*x, "fire" | "twohand" | "dual" | "step"))).map(|(n, _)| n.as_str()).collect();
    println!("\n== fire / plot clips that loop in the data but are played once by the client ({}): {}", loop_clips.len(), loop_clips.iter().copied().collect::<Vec<_>>().join(" "));

    // --- effects
    println!("\n== effects of the player skills");
    let mut fx_skills: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    let mut feature_skills: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut feature_tree: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut hint_skills: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut feature_fx: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    let mut geometry_flying: BTreeSet<u32> = BTreeSet::new();
    let mut geometry_other: BTreeSet<u32> = BTreeSet::new();
    let mut invisible: BTreeSet<&str> = BTreeSet::new();
    let mut empty: BTreeSet<&str> = BTreeSet::new();
    for r in &players {
        for e in &r.fx {
            fx_skills.entry(e.as_str()).or_default().insert(r.id);
            let info = &fx[e];
            for f in &info.features {
                feature_skills.entry(f.clone()).or_default().insert(r.id);
                if r.set.starts_with("tree") && r.shown {
                    feature_tree.entry(f.clone()).or_default().insert(r.id);
                }
                feature_fx.entry(f.clone()).or_default().insert(e.as_str());
            }
            if info.features.contains("geometry") {
                if r.proj_fx.contains(e) {
                    geometry_flying.insert(r.id);
                } else {
                    geometry_other.insert(r.id);
                }
            }
            for h in &info.hints {
                hint_skills.entry(h.clone()).or_default().insert(r.id);
            }
            if info.invisible() {
                invisible.insert(e.as_str());
            }
            if info.empty() {
                empty.insert(e.as_str());
            }
        }
    }
    let found = fx_skills.keys().filter(|e| fx[**e].found).count();
    let major = fx_skills.keys().filter(|e| fx[**e].found && fx[**e].major()).count();
    let minor = fx_skills.keys().filter(|e| fx[**e].found && !fx[**e].major() && !fx[**e].features.is_empty()).count();
    let lib_missing = fx_skills.keys().filter(|e| fx[**e].why == "no library").count();
    println!(
        "distinct effects {}: found {} (plain {}, minor loss {}, looks different {} of which invisible {}; {} have nothing to draw by design), missing {} (library missing {}, not in library {})",
        fx_skills.len(),
        found,
        found - major - minor,
        minor,
        major,
        invisible.len(),
        empty.len(),
        fx_skills.len() - found,
        lib_missing,
        fx_skills.len() - found - lib_missing
    );
    let mut missing_fx: Vec<(&str, usize)> = fx_skills.iter().filter(|(e, _)| !fx[**e].found).map(|(e, s)| (*e, s.len())).collect();
    missing_fx.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("\n== missing effects (player skills that use them)");
    for (e, n) in missing_fx.iter().take(20) {
        println!("{n:>4} skills  {e}  ({})", fx[*e].why);
    }
    let mut mob_fx: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for r in &mobs {
        for e in r.fx.iter().filter(|e| !fx[*e].found) {
            mob_fx.entry(e.as_str()).or_default().insert(r.id);
        }
    }
    println!("missing effects of the monster skills: {}", if mob_fx.is_empty() { "none".to_string() } else { mob_fx.iter().map(|(e, s)| format!("{e} ({} skills)", s.len())).collect::<Vec<_>>().join(", ") });
    let mut features: Vec<(&String, usize, usize, usize)> = feature_skills.iter().map(|(f, s)| (f, s.len(), feature_tree.get(f).map_or(0, |t| t.len()), feature_fx[f].len())).collect();
    features.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("\n== unsupported features by player skills affected ({} skills; tree = the {} shown tree skills; * = changes the look)", players.len(), tree.len());
    println!("{:>6} {:>6} {:>8} {:>9} {:>6}  feature", "skills", "tree", "effects", "emitters", "share");
    for (f, skills, in_tree, effects) in &features {
        // the share of the enabled emitters of those effects that carry the feature
        let (hit, all) = feature_fx[*f].iter().fold((0, 0), |(h, a), e| (h + fx[*e].feature_emitters.get(*f).copied().unwrap_or(0), a + fx[*e].enabled));
        println!("{skills:>6} {in_tree:>6} {effects:>8} {hit:>9} {:>5}%  {f}{}", hit * 100 / all.max(1), if MAJOR.contains(&f.as_str()) { " *" } else { "" });
    }
    let (emitters, lost): (usize, usize) = fx_skills.keys().filter(|e| fx[**e].found).fold((0, 0), |(a, l), e| (a + fx[*e].enabled, l + fx[*e].major_emitters));
    println!("enabled emitters in the {found} effects found: {emitters}, of which {lost} carry a feature that changes the look ({}%); the other {} draw as authored (a light, a sound or a rendering detail aside)", lost * 100 / emitters.max(1), emitters - lost);
    println!("geometry (GeometryInPieces, which aa-fx draws whole): {} skills in projectile groups, {} skills in other groups", geometry_flying.len(), geometry_other.len());
    let mut hints: Vec<(&String, usize)> = hint_skills.iter().map(|(f, s)| (f, s.len())).collect();
    hints.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!("\n== ignored cost / visibility hints (not counted as unsupported; skills)");
    println!("{}", hints.iter().map(|(h, n)| format!("{h}={n}")).collect::<Vec<_>>().join(" "));
    println!("\n== effects with a mesh, trail or orbit but no sprite emitter, which aa-fx draws as nothing ({}): name [features]", invisible.len());
    for e in &invisible {
        println!("  {e} [{}]", fx[*e].features.iter().cloned().collect::<Vec<_>>().join(","));
    }

    let mut ignored: Vec<&String> = libs.handled.iter().filter(|(_, h)| !**h).map(|(k, _)| k).collect();
    ignored.sort();
    println!("attributes in these effects that aa-fx's parser never reads: {}", ignored.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "));
    let total = players.iter().fold(Extras::default(), |a, r| a.plus(r.extras));
    println!("non-particle items in the groups of all player skills: {} FxSound, {} sounds bound to particles, {} voices, {} camera shakes, {} material flashes, {} others", total.sound, total.particle_sound, total.voice, total.shake, total.material, total.other);

    // --- plots and channeling
    let steps: usize = players.iter().map(|r| r.steps).sum();
    let steps_anim: usize = players.iter().map(|r| r.steps_with_anim).sum();
    let steps_two: usize = players.iter().map(|r| r.steps_two_anims).sum();
    let steps_stop: usize = players.iter().map(|r| r.steps_stop_only).sum();
    println!("\n== plots: {} skills, {steps} steps, {steps_anim} with an anim, {steps_two} with two anims (stop + clip), {steps_stop} with only a stop marker", players.iter().filter(|r| r.plot).count());
    let channel: Vec<&&Row> = players.iter().filter(|r| r.channeling()).collect();
    println!(
        "channeling skills: {} ({})",
        channel.len(),
        channel.iter().map(|r| format!("{} {}{}", r.id, r.name, if r.channel_clip() { "" } else { " [no clip]" })).collect::<Vec<_>>().join("; ")
    );

    // --- what the client does with them (the same lookups `ingame/combat.rs` makes)
    {
        let tree = |f: &dyn Fn(&Row) -> bool| -> String {
            let all = players.iter().filter(|r| f(r)).count();
            let shown = players.iter().filter(|r| r.shown && f(r)).count();
            format!("{all} skills ({shown} shown tree skills)")
        };
        let plays = |r: &Row, roles: &[&str]| r.clips.values().any(|c| c.roles.iter().any(|x| roles.contains(x)));
        let loops = |r: &Row| r.clips.values().any(|c| c.loops && c.roles.iter().any(|x| matches!(*x, "fire" | "twohand" | "dual" | "step")));
        let shakes = |r: &Row| {
            let Some(v) = book.skill(r.id) else { return false };
            let flying = |p: &Option<skills::Projectile>| p.as_ref().is_some_and(|p| !p.shakes.is_empty());
            !v.shakes.is_empty()
                || flying(&v.projectile)
                || v.levels.iter().any(|(_, l)| !l.shakes.is_empty() || flying(&l.projectile))
                || book.plot_steps(r.id).iter().any(|e| book.plot_event(*e).is_some_and(|p| !p.shakes.is_empty() || flying(&p.projectile)))
        };
        let levels = |r: &Row| book.skill(r.id).is_some_and(|v| !v.levels.is_empty());
        println!("\n== client handling (player skills; each is played by ingame/combat.rs)");
        println!("a plot step that only stops ends the cast loop: {}", tree(&|r| r.steps_stop_only > 0));
        println!("a clip that loops until the next step or the end (anims.loop; fire, weapon variant and plot step clips): {}", tree(&loops));
        println!("  of them channeling skills (channeling time or clip) whose pose loops for the channel: {} of {}", players.iter().filter(|r| r.channeling() && loops(r)).count(), players.iter().filter(|r| r.channeling()).count());
        println!("clip by the weapon held (twohand / dual wield variants): {}", tree(&|r| plays(r, &["twohand", "dual"])));
        println!("controller clip only (dash, leap, back step: no clip of its own): {}", tree(&|r| plays(r, &["controller"]) && !plays(r, &["start", "fire", "twohand", "dual", "step"])));
        println!("clip by the instrument held (songs): {}", tree(&|r| plays(r, &["instrument"])));
        println!("visual group by skill level: {}", tree(&levels));
        println!("camera shake: {}", tree(&shakes));
        let lost: Vec<String> = players.iter().filter(|r| r.extras.shake > 0 && !shakes(r)).map(|r| format!("{} {}", r.id, r.name)).collect();
        println!("  shake items the client does not play: {}", if lost.is_empty() { "none".to_string() } else { lost.join("; ") });
        // the buffs the skills bring (skill effects, channeling and toggle buffs) and what the client shows of them
        let mut buffs: BTreeSet<u32> = BTreeSet::new();
        let mut with_buff = (0, 0);
        for r in players.iter() {
            let ids = book.skill_buffs(r.id);
            let shows = ids.iter().filter(|b| book.buff(**b).is_some()).count();
            with_buff.0 += usize::from(!ids.is_empty());
            with_buff.1 += usize::from(shows > 0);
            buffs.extend(ids);
        }
        let looks: Vec<_> = buffs.iter().filter_map(|b| book.buff(*b)).collect();
        // the buffs that show something, as `b<id>` tokens for `AA_COMBAT_SWEEP` (next to the skill table)
        if let Some(dir) = std::path::Path::new(&out).parent() {
            let ids: Vec<String> = buffs.iter().filter(|b| book.buff(**b).is_some()).map(|b| format!("b{b}")).collect();
            let _ = std::fs::write(dir.join("ids_buffs.txt"), ids.join(","));
        }
        println!(
            "buffs of the player skills: {} distinct, {} show something ({} effect group, {} pose, {} animation freeze, {} fall over); {} skills bring buffs, {} of them with a visible one",
            buffs.len(),
            looks.len(),
            looks.iter().filter(|l| !l.fx.is_empty()).count(),
            looks.iter().filter(|l| l.start_anim.is_some() || l.instrument_start.iter().any(Option::is_some)).count(),
            looks.iter().filter(|l| l.framehold).count(),
            looks.iter().filter(|l| l.ragdoll).count(),
            with_buff.0,
            with_buff.1
        );
    }

    // --- per set
    println!("\n== effect classes per set (player skills)");
    let mut by_set: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for r in &players {
        *by_set.entry(r.set.as_str()).or_default().entry(fx_class(r, &fx).label()).or_default() += 1;
    }
    for (set, c) in &by_set {
        println!("{set:<8} {}", c.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "));
    }
}
