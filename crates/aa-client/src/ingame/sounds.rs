//! Sounds of the units in the world: skill sounds, voices, footsteps and the sounds of animations.
//!
//! `aa-audio` plays events (`sounds/x2skills:wild:fire_arrow_form`); this module decides which, when and where. Everything is played
//! in 3D, as a [`Sound`] entity under the bone it comes from, so it follows the unit and stops with it:
//!
//! * **Skill sounds.** The `FxSound` items of an effect group are events that start at a moment of the skill ([`super::combat`]
//!   handles them like particle effects: caster or target, bone, end moment); a particle effect with a sound of its own
//!   (`fx_particles.sound_id`, and the `Sound=""` attribute of the particle library's emitters, [`SoundBook::particle_sounds`]) plays
//!   it when the effect starts.
//! * **Voices.** An `FxVoice` item names an item of the caster's sound pack (`sv_fight_triple_slash_1_launch` in `nuian_male`):
//!   the pack comes from the caster's model (`models.sound_pack_id`), which is what makes the voice follow race and gender.
//! * **Animation sounds** ([`anim_sounds`]): the clip a unit plays has events in the `$SoundEventDatabase` of its `.cal`: `sound`
//!   (an event path), `sound_pack` (an item of the unit's pack: the grunt of a hit) and `footstep` (a row of
//!   `materialeffects.xml`; the column is the water, object or terrain surface under the foot
//!   ([`Audio::surface`]), the cell gives the sound of `fxlibs/<row>.xml`). The run and walk clips have none of their own: the
//!   steps are phases (0.25, 0.75 of a cycle) in the `.lmg` of their group ([`SoundBook::clip_events`]). A clip on the upper body of
//!   a moving unit is invisible to `anim_sounds` (it is the animator's layer): [`LayerSounds`] plays its events by the clock.
//! * **Blows.** A received weapon impact plays the weapon's sound on the target's material (`combat_sounds`, [`SoundBook::blow`]).
//!
//! Every unit has at most [`UNIT_CAP`] sounds at a time (a new one replaces the oldest of the same or a lower [`Class`], else it is
//! dropped), sounds far from the camera are not started at all ([`Class::reach`]), and nothing is read or decoded in a run that is
//! muted (`--shot`, `--exit-after`, `--mute`, `AA_MUTE=1`; `AA_AUDIO_LOG=1` makes the databases load and logs each sound).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use aa_anim::Animator;
use aa_audio::{Engine, Sound, SoundPacks};
use aa_net::state::{Player, Units};
use aa_world::TerrainQuery;
use aa_world::heightmap::GRID;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags, params};

mod weapon;

use super::anim::{Locomotion, clip_file};
use super::camera::OrbitCamera;
use super::combat::{Dead, Sfx};
use super::player::LocalPlayer;
use super::units::RemoteUnit;
use crate::Screen;
use crate::sound::{Packs, log_enabled};

/// Most sounds a unit has playing at once.
pub const UNIT_CAP: usize = 4;

/// Units further from the camera than this (m) do not play their animation sounds at all.
const ANIM_REACH: f32 = 45.0;

/// What a sound is for: a higher class replaces a lower one when a unit is at its cap, and carries further.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    /// Footsteps and the sounds of animations.
    Foley,
    /// The voice of a character.
    Voice,
    /// Skill effects.
    Skill,
}

impl Class {
    /// Distance (m) from the camera beyond which a sound of this class is not started.
    fn reach(self) -> f32 {
        match self {
            Class::Foley => 32.0,
            Class::Voice => 55.0,
            Class::Skill => 75.0,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        ["foley", "voice", "skill"][self.index()]
    }
}

/// A sound entity this module made (the cap of a unit counts the ones that are still in the world).
#[derive(Component)]
pub struct Mark;

/// One sound to start.
pub struct Play<'a> {
    /// The event path (`sounds/x2skills:wild:fire_arrow_form`).
    pub path: &'a str,
    pub class: Class,
    /// The unit it belongs to (its cap), and the entity it hangs under (a bone of the unit, the unit, an effect, a projectile).
    pub unit: Entity,
    pub parent: Entity,
    /// Where it is now (Bevy world), for the distance to the camera.
    pub at: Vec3,
}

/// The sound database of the client and the bookkeeping of the sounds in the world.
#[derive(Resource)]
pub struct SoundBook(Mutex<Inner>);

struct Inner {
    conn: Option<Connection>,
    /// `models.id` -> the name of its sound pack.
    model_packs: HashMap<u32, Option<Arc<str>>>,
    /// `$SoundEventDatabase` path -> the clips' sound events (read on a thread).
    tables: HashMap<String, Arc<OnceLock<ClipTable>>>,
    /// Particle library -> the sounds of its effects (read on a thread).
    particles: HashMap<String, Arc<OnceLock<ParticleSounds>>>,
    footsteps: Arc<OnceLock<Footsteps>>,
    fxlibs: HashMap<String, Arc<HashMap<String, String>>>,
    /// `combat_sounds`: (the weapon's sound material, the target's) -> the sound of a blow.
    blows: HashMap<(u32, u32), Arc<str>>,
    weapon: weapon::Catalog,
    surfaces: Surfaces,
    /// The sounds each unit has playing.
    live: HashMap<Entity, Vec<(Entity, Class, f64)>>,
    counters: [Counter; 3],
}

#[derive(Default, Clone, Copy)]
struct Counter {
    started: u64,
    culled: u64,
    capped: u64,
}

impl SoundBook {
    fn open() -> SoundBook {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).ok();
        let footsteps: Arc<OnceLock<Footsteps>> = Arc::default();
        let cell = footsteps.clone();
        std::thread::spawn(move || {
            let _ = cell.set(Footsteps::load());
        });
        let blows = conn.as_ref().map(read_blows).unwrap_or_default();
        SoundBook(Mutex::new(Inner {
            conn,
            blows,
            weapon: weapon::Catalog::default(),
            model_packs: HashMap::new(),
            tables: HashMap::new(),
            particles: HashMap::new(),
            footsteps,
            fxlibs: HashMap::new(),
            surfaces: Surfaces::default(),
            live: HashMap::new(),
            counters: [Counter::default(); 3],
        }))
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The name of the sound pack of a model (`nuian_male`): the voices and the `sound_pack` events of its units.
    pub fn pack_of_model(&self, model: u32) -> Option<Arc<str>> {
        let mut inner = self.inner();
        if let Some(p) = inner.model_packs.get(&model) {
            return p.clone();
        }
        let name: Option<Arc<str>> = inner
            .conn
            .as_ref()
            .and_then(|c| {
                c.query_row("SELECT p.name FROM models m JOIN sound_packs p ON p.id = m.sound_pack_id WHERE m.id = ?1", params![model], |r| r.get::<_, Option<String>>(0))
                    .ok()
            })
            .flatten()
            .map(|n| Arc::from(n.as_str()));
        inner.model_packs.insert(model, name.clone());
        name
    }

    /// Resolve the impact before spawning its Sound, including all authored initial parameters.
    fn blow(&self, impact: &aa_net::skill::UnitDamage, attacker: &aa_net::Unit, target: &aa_net::Unit) -> Option<Sound> {
        let mut inner = self.inner();
        let Inner { conn, blows, weapon, .. } = &mut *inner;
        let mainhand = attacker.equipment.get(aa_net::model::slot::MAINHAND).map_or(0, |item| item.template_id);
        weapon.sound(conn.as_ref()?, blows, impact, mainhand, target.into())
    }

    /// The sound events of the clips of a model's `$SoundEventDatabase`; `None` while it is being read.
    pub fn clip_sounds(&self, database: &str, clip_name: &str) -> Option<Arc<[AnimSound]>> {
        let slot = {
            let mut inner = self.inner();
            let key = database.to_ascii_lowercase().replace('\\', "/");
            inner
                .tables
                .entry(key.clone())
                .or_insert_with(|| {
                    let slot: Arc<OnceLock<ClipTable>> = Arc::default();
                    let cell = slot.clone();
                    std::thread::spawn(move || {
                        let table = aa_cry::read(&key).ok().map(|b| parse_clip_sounds(&String::from_utf8_lossy(&b))).unwrap_or_default();
                        let _ = cell.set(table);
                    });
                    slot
                })
                .clone()
        };
        let table = slot.get()?;
        Some(table.get(&file_name(clip_name)).cloned().unwrap_or_else(|| Arc::from(Vec::new())))
    }

    /// The sound events of a clip as the unit plays it: its own, else (a direction of a locomotion group, `fist_mo_normal_run_fl`) the
    /// ones of the group's `.lmg`, whose times are phases of the cycle (`duration` is the length of the clip).
    pub fn clip_events(&self, database: &str, file: &str, duration: f32) -> Option<Arc<[AnimSound]>> {
        let own = self.clip_sounds(database, file)?;
        if !own.is_empty() {
            return Some(own);
        }
        let Some(group) = locomotion_group(file) else { return Some(own) };
        let phases = self.clip_sounds(database, &group)?;
        Some(phases.iter().map(|e| AnimSound { time: e.time * duration, ..e.clone() }).collect())
    }

    /// The `Sound=""` events of the emitters of a particle effect (`X2_EFFECTS.p_effect.st1000b`); empty while the library is being
    /// read (the first use) and for effects without sounds.
    pub fn particle_sounds(&self, effect: &str) -> Arc<[Arc<str>]> {
        let Some((lib, name)) = effect.split_once('.') else { return Arc::from(Vec::new()) };
        let lib = lib.to_ascii_lowercase();
        let slot = {
            let mut inner = self.inner();
            inner
                .particles
                .entry(lib.clone())
                .or_insert_with(|| {
                    let slot: Arc<OnceLock<ParticleSounds>> = Arc::default();
                    let cell = slot.clone();
                    std::thread::spawn(move || {
                        let table = aa_cry::read(&format!("libs/particles/{lib}.xml")).ok().map(|b| scan_particle_sounds(&String::from_utf8_lossy(&b))).unwrap_or_default();
                        let _ = cell.set(table);
                    });
                    slot
                })
                .clone()
        };
        slot.get().and_then(|t| t.get(&name.to_ascii_lowercase()).cloned()).unwrap_or_else(|| Arc::from(Vec::new()))
    }

    /// The sound of a footstep row of `materialeffects.xml` on a surface (`mat_dirt`).
    fn footstep(&self, row: &str, surface: &str) -> Option<Arc<str>> {
        let mut inner = self.inner();
        let sheet = inner.footsteps.clone();
        let cell = sheet.get()?.cell(row, surface)?;
        let (lib, effect) = cell.split_once(':')?;
        let effects = inner.fxlibs.entry(lib.to_string()).or_insert_with(|| Arc::new(read_fxlib(lib))).clone();
        effects.get(&effect.to_ascii_lowercase()).map(|p| Arc::from(p.as_str()))
    }

    /// The surface (`mat_dirt`) of the terrain under a world position, or `mat_default`.
    fn surface(&self, terrain: &TerrainQuery, at: Vec3) -> Arc<str> {
        let (x, y) = (at.x, -at.z);
        let default = || Arc::from("mat_default");
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return default();
        }
        let size = aa_world::CELL_SIZE;
        let (cx, cy) = ((x / size).floor() as i32, (y / size).floor() as i32);
        let Some(cell) = terrain.cell(cx, cy) else { return default() };
        let (lx, ly) = (x - cx as f32 * size, y - cy as f32 * size);
        let (ix, iy) = (((lx / 2.0).round() as usize).min(GRID - 1), ((ly / 2.0).round() as usize).min(GRID - 1));
        // below the sea
        if at.y < cell.ocean_level - 0.05 {
            return Arc::from("mat_water");
        }
        let index = cell.surface.get(iy * GRID + ix).copied().unwrap_or(0) as usize & 15;
        let table = {
            let mut inner = self.inner();
            inner.surfaces.cell(terrain.world(), cx, cy)
        };
        table.get().and_then(|t| t[index].clone()).unwrap_or_else(default)
    }

    /// Starts a sound under `play.parent`. `None` when it was not (too far, the unit is at its cap with louder sounds, or the engine is
    /// off).
    pub fn start(&self, commands: &mut Commands, listener: Option<Vec3>, marks: &Query<(), With<Mark>>, now: f64, play: &Play) -> Option<Entity> {
        self.start_sound(commands, listener, marks, now, play, Sound::event(play.path))
    }

    fn start_sound(&self, commands: &mut Commands, listener: Option<Vec3>, marks: &Query<(), With<Mark>>, now: f64, play: &Play, sound: Sound) -> Option<Entity> {
        let class = play.class;
        let mut inner = self.inner();
        if listener.is_some_and(|l| l.distance(play.at) > class.reach()) {
            inner.counters[class.index()].culled += 1;
            return None;
        }
        // what the unit still has playing (a sound spawned a moment ago is not in the world yet)
        let live = inner.live.entry(play.unit).or_default();
        live.retain(|(e, _, born)| now - born < 0.2 || marks.contains(*e));
        let mut victim = None;
        if live.len() >= UNIT_CAP {
            // the oldest of this class or a lower one makes room
            let i = live.iter().enumerate().filter(|(_, (_, c, _))| *c <= class).min_by(|a, b| a.1.2.total_cmp(&b.1.2)).map(|(i, _)| i);
            match i {
                Some(i) => victim = Some(live.remove(i).0),
                None => {
                    inner.counters[class.index()].capped += 1;
                    return None;
                }
            }
        }
        if let Some(old) = victim {
            commands.entity(old).try_despawn();
        }
        let entity = commands
            .spawn((Name::new(format!("sound {}", play.path)), sound.despawn_when_done(), Transform::IDENTITY, ChildOf(play.parent), Mark))
            .id();
        inner.live.entry(play.unit).or_default().push((entity, class, now));
        inner.counters[class.index()].started += 1;
        if log_enabled() {
            info!("sound: {} {} on {:?}", class.name(), play.path, play.unit);
        }
        Some(entity)
    }

    /// How many sounds of each class were started, left out for the distance and left out for the cap.
    pub fn counts(&self) -> [(u64, u64, u64); 3] {
        let inner = self.inner();
        std::array::from_fn(|i| (inner.counters[i].started, inner.counters[i].culled, inner.counters[i].capped))
    }
}

/// Everything a system needs to start sounds. `None` parts mean the run is silent (no engine, or muted without logging).
#[derive(SystemParam)]
pub struct Audio<'w, 's> {
    book: Option<Res<'w, SoundBook>>,
    packs: Option<Res<'w, Packs>>,
    listener: Query<'w, 's, &'static GlobalTransform, With<OrbitCamera>>,
    marks: Query<'w, 's, (), With<Mark>>,
    time: Res<'w, Time>,
    units: Option<Res<'w, Units>>,
    player: Option<Res<'w, Player>>,
    collision: Option<Res<'w, aa_world::WorldCollision>>,
    water: Option<Res<'w, aa_world::WaterQuery>>,
}

impl Audio<'_, '_> {
    /// The sound database, once it is loaded.
    pub fn book(&self) -> Option<&SoundBook> {
        self.book.as_deref()
    }

    pub fn packs(&self) -> Option<&SoundPacks> {
        self.packs.as_ref().and_then(|p| p.get())
    }

    /// Sounds are wanted and can be resolved.
    pub fn active(&self) -> bool {
        self.book.is_some() && self.packs().is_some()
    }

    /// The camera, for the distance of sounds.
    fn camera(&self) -> Option<Vec3> {
        self.listener.iter().next().map(|g| g.translation())
    }

    /// Query the actual supporting material, so a wooden deck does not sound like the terrain underneath it.
    fn surface(&self, book: &SoundBook, terrain: &TerrainQuery, at: Vec3) -> Arc<str> {
        let pos = aa_world::bevy_to_cry(at);
        if self.water.as_ref().and_then(|w| w.water_at(pos)).is_some_and(|(height, _)| height > pos.z + 0.05) {
            return Arc::from("mat_water");
        }
        if let Some(standing) = self.collision.as_ref().and_then(|c| c.surface_below(pos.x, pos.y, pos.z, 0.5))
            && (!standing.terrain || standing.surface != aa_world::collide::Surface::DEFAULT)
        {
            return Arc::from(standing.surface.name());
        }
        book.surface(terrain, at)
    }

    /// Starts the event `play.path`.
    pub fn start(&self, commands: &mut Commands, play: &Play) -> Option<Entity> {
        self.book()?.start(commands, self.camera(), &self.marks, self.time.elapsed_secs_f64(), play)
    }

    /// Starts a skill sound item at a unit: an event, or an item of the unit's sound pack (the voice).
    #[allow(clippy::too_many_arguments)]
    pub fn sfx(&self, commands: &mut Commands, sfx: &Sfx, class: Class, unit: Entity, obj_id: Option<u32>, parent: Entity, at: Vec3) -> Option<Entity> {
        let packs = self.packs()?;
        let path = match sfx {
            Sfx::Event(path) => path.to_string(),
            Sfx::Voice(item) => match obj_id.and_then(|id| self.pack_path(id, item)) {
                Some(path) => path,
                None => {
                    if log_enabled() {
                        info!("sound: no voice {item} for unit {obj_id:?}");
                    }
                    return None;
                }
            },
        };
        let _ = packs;
        self.start(commands, &Play { path: &path, class, unit, parent, at })
    }

    /// A unit by object id: the units in view, else the local player.
    fn unit(&self, obj_id: u32) -> Option<&aa_net::model::Unit> {
        let seen = self.units.as_ref().and_then(|u| u.get(obj_id));
        seen.or_else(|| self.player.as_ref()?.spawn.as_ref().map(|s| &s.unit).filter(|u| u.obj_id == obj_id))
    }

    /// The model of a unit by object id.
    fn model_of(&self, obj_id: u32) -> Option<u32> {
        self.unit(obj_id).map(|u| u.model_id)
    }

    /// A received weapon impact, resolved against equipment at the scheduled impact moment.
    pub fn blow(&self, commands: &mut Commands, impact: &aa_net::skill::UnitDamage, attacker: u32, target: (Entity, u32), parent: Entity, at: Vec3) -> Option<Entity> {
        self.packs()?;
        let book = self.book()?;
        let sound = book.blow(impact, self.unit(attacker)?, self.unit(target.1)?)?;
        let path = sound.event.clone();
        book.start_sound(commands, self.camera(), &self.marks, self.time.elapsed_secs_f64(), &Play { path: &path, class: Class::Skill, unit: target.0, parent, at }, sound)
    }

    /// The sound pack item `item` of the pack of the unit with object id `obj_id`.
    fn pack_path(&self, obj_id: u32, item: &str) -> Option<String> {
        let model = self.model_of(obj_id)?;
        let pack = self.book()?.pack_of_model(model)?;
        self.packs()?.path(&pack, item).map(str::to_string)
    }
}

/// `combat_sounds`: the sound of every pair of sound materials (weapon, target).
fn read_blows(conn: &Connection) -> HashMap<(u32, u32), Arc<str>> {
    let mut blows = HashMap::new();
    let Ok(mut q) = conn.prepare(
        "SELECT c.source_sound_material_id, c.target_sound_material_id, s.path FROM combat_sounds c JOIN sounds s ON s.id = c.sound_id WHERE s.path IS NOT NULL AND s.path <> '' ORDER BY c.id",
    ) else {
        return blows;
    };
    let rows = q.query_map([], |r| Ok((r.get::<_, Option<u32>>(0)?, r.get::<_, Option<u32>>(1)?, r.get::<_, String>(2)?)));
    for (source, target, path) in rows.into_iter().flatten().flatten() {
        if let (Some(s), Some(t)) = (source, target) {
            blows.entry((s, t)).or_insert_with(|| Arc::from(path.trim()));
        }
    }
    blows
}

// ---- the sounds of animations

/// What an animation event plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `sound`: the parameter is an event path.
    Sound,
    /// `sound_pack`: the parameter is an item of the unit's sound pack.
    Pack,
    /// `footstep`: the parameter is a row of `materialeffects.xml`.
    Footstep,
}

/// One sound event of a clip.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimSound {
    /// Seconds into the clip.
    pub time: f32,
    pub kind: Kind,
    pub param: Arc<str>,
    /// The bone it plays at (`Bip01 L Foot`); empty for the unit's origin.
    pub bone: Arc<str>,
}

type ClipTable = HashMap<String, Arc<[AnimSound]>>;

/// The `Sound=""` events of the emitters of every effect of a particle library, by effect name.
type ParticleSounds = HashMap<String, Arc<[Arc<str>]>>;

/// The surface name (`mat_dirt`) of each of the 16 surface types of a terrain cell.
type SurfaceNames = [Option<Arc<str>>; 16];

/// The direction suffixes of the clips of a locomotion group.
const DIRECTIONS: [&str; 8] = ["f", "fl", "fr", "l", "r", "b", "bl", "br"];

/// The `.lmg` (locomotion group) a direction clip belongs to: `fist_mo_normal_run_fl.caf` -> `fist_mo_normal_run.lmg`.
fn locomotion_group(file: &str) -> Option<String> {
    let (group, direction) = file.strip_suffix(".caf")?.rsplit_once('_')?;
    // only the movement clips (`fist_mo_normal_run_fl`), not a swing that ends in `_r` (`fist_co_attack_r`)
    (DIRECTIONS.contains(&direction) && group.contains("_mo_")).then(|| format!("{group}.lmg"))
}

fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_ascii_lowercase()
}

/// Parses a `sound_event_database.animevents`: the `sound`, `sound_pack` and `footstep` events of every clip, by file name, in time
/// order.
fn parse_clip_sounds(xml: &str) -> ClipTable {
    let mut table = ClipTable::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return table };
    for animation in doc.descendants().filter(|n| n.has_tag_name("animation")) {
        let Some(name) = animation.attribute("name") else { continue };
        let mut events: Vec<AnimSound> = Vec::new();
        for event in animation.children().filter(|n| n.has_tag_name("event")) {
            let Some(time) = event.attribute("time").and_then(|t| t.trim().parse::<f32>().ok()) else { continue };
            let param = event.attribute("parameter").unwrap_or("").trim();
            if param.is_empty() {
                continue;
            }
            let kind = match event.attribute("name").map(str::trim) {
                // a few `sound` events carry a footstep row (the authors' slip)
                Some("sound") if param.to_ascii_lowercase().starts_with("footstep") => Kind::Footstep,
                Some("sound") => Kind::Sound,
                Some("sound_pack") => Kind::Pack,
                Some("footstep") => Kind::Footstep,
                _ => continue,
            };
            events.push(AnimSound { time, kind, param: Arc::from(param), bone: Arc::from(event.attribute("bone").unwrap_or("").trim()) });
        }
        if !events.is_empty() {
            events.sort_by(|a, b| a.time.total_cmp(&b.time));
            table.insert(file_name(name), Arc::from(events));
        }
    }
    table
}

/// The events of a clip that a playhead passed going from `before` to `now` (seconds into the clip; `before` is negative for a clip that
/// starts, `wrapped` when it looped between the two): in clip order.
pub fn passed(events: &[AnimSound], before: f32, now: f32, wrapped: bool) -> impl Iterator<Item = &AnimSound> {
    events.iter().filter(move |e| if wrapped { e.time > before || e.time <= now } else { e.time > before && e.time <= now })
}

// ---- footsteps

/// `materialeffects.xml`: for every footstep row (`footstep_nuian_walk`) the effect (`footstep_nuian_walk:dirt`) of each surface
/// (`mat_dirt`).
#[derive(Default)]
struct Footsteps {
    rows: HashMap<String, HashMap<String, String>>,
}

impl Footsteps {
    fn load() -> Footsteps {
        let xml = aa_cry::read("libs/materialeffects/materialeffects.xml").ok().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        Footsteps { rows: parse_sheet(&xml) }
    }

    /// The `lib:effect` of a row on a surface; the default column for surfaces the sheet does not know.
    fn cell(&self, row: &str, surface: &str) -> Option<String> {
        let row = self.rows.get(&row.to_ascii_lowercase())?;
        row.get(&surface.to_ascii_lowercase()).or_else(|| row.get("mat_default")).filter(|c| !c.is_empty()).cloned()
    }
}

const SHEET: &str = "urn:schemas-microsoft-com:office:spreadsheet";

/// The Excel XML sheet of `materialeffects.xml`: the first row names the columns (surfaces), the first cell of every other row the
/// row (an fxlib), the cells the `fxlib:effect`.
fn parse_sheet(xml: &str) -> HashMap<String, HashMap<String, String>> {
    let mut rows = HashMap::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return rows };
    let mut header: HashMap<usize, String> = HashMap::new();
    for (n, row) in doc.descendants().filter(|n| n.has_tag_name("Row")).enumerate() {
        let mut col = 0usize;
        let mut cells: Vec<(usize, String)> = Vec::new();
        for cell in row.children().filter(|n| n.has_tag_name("Cell")) {
            col = match cell.attribute((SHEET, "Index")).and_then(|i| i.parse::<usize>().ok()) {
                Some(i) => i,
                None => col + 1,
            };
            let text = cell.children().find(|d| d.has_tag_name("Data")).and_then(|d| d.text()).unwrap_or("").trim().to_string();
            cells.push((col, text));
        }
        if n == 0 {
            header = cells.into_iter().map(|(c, t)| (c, t.to_ascii_lowercase())).collect();
            continue;
        }
        let Some(name) = cells.iter().find(|(c, _)| *c == 1).map(|(_, t)| t.to_ascii_lowercase()).filter(|t| !t.is_empty()) else { continue };
        let by_surface: HashMap<String, String> = cells.into_iter().filter(|(c, _)| *c > 1).filter_map(|(c, t)| header.get(&c).map(|h| (h.clone(), t))).collect();
        rows.insert(name, by_surface);
    }
    rows
}

/// `libs/materialeffects/fxlibs/<lib>.xml`: effect name (lower case) -> the sound it plays.
fn read_fxlib(lib: &str) -> HashMap<String, String> {
    let Ok(bytes) = aa_cry::read(&format!("libs/materialeffects/fxlibs/{}.xml", lib.to_ascii_lowercase())) else { return HashMap::new() };
    parse_fxlib(&String::from_utf8_lossy(&bytes))
}

fn parse_fxlib(xml: &str) -> HashMap<String, String> {
    let mut effects = HashMap::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return effects };
    for effect in doc.descendants().filter(|n| n.has_tag_name("Effect")) {
        let Some(name) = effect.attribute("name") else { continue };
        let sound = effect.children().find(|n| n.has_tag_name("Sound")).and_then(|s| s.children().find(|n| n.has_tag_name("Name"))).and_then(|n| n.text()).map(str::trim).filter(|s| !s.is_empty());
        if let Some(sound) = sound {
            effects.insert(name.trim().to_ascii_lowercase(), sound.to_string());
        }
    }
    effects
}

/// The terrain surface of every cell, read when a footstep first lands in it.
#[derive(Default)]
struct Surfaces {
    cells: HashMap<(i32, i32), Arc<OnceLock<SurfaceNames>>>,
    /// `.mtl` path -> its `SurfaceType`.
    materials: Arc<Mutex<HashMap<String, Option<Arc<str>>>>>,
}

impl Surfaces {
    /// The 16 surface names of a cell (`None` entries are unknown), `get()` is empty while the cell is being read.
    fn cell(&mut self, world: &str, cx: i32, cy: i32) -> Arc<OnceLock<[Option<Arc<str>>; 16]>> {
        let materials = self.materials.clone();
        self.cells
            .entry((cx, cy))
            .or_insert_with(|| {
                let slot: Arc<OnceLock<[Option<Arc<str>>; 16]>> = Arc::default();
                let cell = slot.clone();
                let path = format!("{}/client/leveldata.xml", aa_world::world::cell_dir(world, cx, cy));
                std::thread::spawn(move || {
                    let xml = aa_cry::read(&path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                    let mut names: [Option<Arc<str>>; 16] = Default::default();
                    for (i, entry) in aa_world::terrain_layers::parse_surface_types(&xml).into_iter().take(16).enumerate() {
                        if entry.material.is_empty() {
                            continue;
                        }
                        let path = format!("{}.mtl", entry.material.replace('\\', "/").to_ascii_lowercase());
                        let cached = materials.lock().unwrap_or_else(|e| e.into_inner()).get(&path).cloned();
                        names[i] = match cached {
                            Some(name) => name,
                            None => {
                                let name = aa_cry::read(&path).ok().and_then(|b| material_surface(&String::from_utf8_lossy(&b))).map(|s| Arc::from(s.as_str()));
                                materials.lock().unwrap_or_else(|e| e.into_inner()).insert(path, name.clone());
                                name
                            }
                        };
                    }
                    let _ = cell.set(names);
                });
                slot
            })
            .clone()
    }
}

/// The `SurfaceType` of the first `<Material>` tag of an `.mtl`.
fn material_surface(mtl: &str) -> Option<String> {
    let tag = &mtl[mtl.find("<Material")?..];
    let tag = &tag[..tag.find('>')?];
    let at = tag.find("SurfaceType=\"")? + "SurfaceType=\"".len();
    let value = &tag[at..];
    Some(value[..value.find('"')?].to_ascii_lowercase()).filter(|s| !s.is_empty())
}

// ---- the sounds of particle libraries

/// The `Sound=""` attributes of the emitters of every effect of a particle library, by effect name (lower case): the effects are the
/// top level `<Particles Name="...">` elements, the emitters their `<Params>` (children included). Scans the text; a library is
/// several MB.
fn scan_particle_sounds(xml: &str) -> HashMap<String, Arc<[Arc<str>]>> {
    let mut out = HashMap::new();
    let (mut at, mut depth) = (0usize, 0i32);
    let mut current: Option<(String, Vec<Arc<str>>)> = None;
    while let Some(open) = xml[at..].find('<').map(|p| p + at) {
        let Some(end) = xml[open..].find('>').map(|p| p + open) else { break };
        let tag = &xml[open + 1..end];
        at = end + 1;
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(closing) = tag.strip_prefix('/') {
            if closing.trim() == "Particles" {
                depth -= 1;
                if depth == 0 {
                    finish(&mut current, &mut out);
                }
            }
            continue;
        }
        let self_closing = tag.ends_with('/');
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match name {
            "Particles" => {
                if depth == 0 {
                    current = attribute(tag, "Name").map(|n| (n.to_ascii_lowercase(), Vec::new()));
                }
                if self_closing {
                    if depth == 0 {
                        finish(&mut current, &mut out);
                    }
                } else {
                    depth += 1;
                }
            }
            "Params" => {
                if let (Some((_, sounds)), Some(sound)) = (current.as_mut(), attribute(tag, "Sound")) {
                    let sound = sound.trim();
                    if !sound.is_empty() && !sounds.iter().any(|s| s.eq_ignore_ascii_case(sound)) {
                        sounds.push(Arc::from(sound));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn finish(current: &mut Option<(String, Vec<Arc<str>>)>, out: &mut HashMap<String, Arc<[Arc<str>]>>) {
    if let Some((name, sounds)) = current.take()
        && !sounds.is_empty()
    {
        out.insert(name, Arc::from(sounds));
    }
}

/// The value of attribute `name` in the text of a tag (`Sound="..."`, not `SoundFXParam`).
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(p) = tag[from..].find(&key).map(|p| p + from) {
        if p == 0 || tag.as_bytes()[p - 1].is_ascii_whitespace() {
            let value = &tag[p + key.len()..];
            return Some(&value[..value.find('"')?]);
        }
        from = p + key.len();
    }
    None
}

// ---- the system

/// Where an animation of a unit was when the sounds were last looked at.
struct Cursor {
    /// Which clip (the address of its data).
    clip: usize,
    time: f32,
    loops: u32,
    events: Arc<[AnimSound]>,
}

/// Plays the sound events of the clips the units play (footsteps, grunts, swings), for the units near the camera.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn anim_sounds(
    mut commands: Commands,
    time: Res<Time>,
    audio: Audio,
    terrain: Res<TerrainQuery>,
    units: Query<(Entity, &Animator, &Locomotion, &GlobalTransform, Option<&RemoteUnit>, Option<&LocalPlayer>), Without<Dead>>,
    xforms: Query<&GlobalTransform>,
    mut cursors: Local<HashMap<Entity, Cursor>>,
    mut next_prune: Local<f32>,
) {
    if !audio.active() {
        return;
    }
    let Some(book) = audio.book() else { return };
    let Some(camera) = audio.camera() else { return };
    let now = time.elapsed_secs();
    if now > *next_prune {
        *next_prune = now + 3.0;
        cursors.retain(|e, _| units.contains(*e));
    }
    for (entity, animator, loco, gt, remote, local) in &units {
        let Some(playing) = animator.current.as_ref() else { continue };
        if gt.translation().distance_squared(camera) > ANIM_REACH * ANIM_REACH {
            cursors.remove(&entity);
            continue;
        }
        let Some(database) = loco.clips.cal.sound_events.as_deref() else { continue };
        let clip = Arc::as_ptr(&playing.clip.clip) as usize;
        let t = playing.time;
        let (before, wrapped) = match cursors.get(&entity) {
            Some(c) if c.clip == clip => (c.time, playing.loops != c.loops || t < c.time),
            _ => (-1.0, false),
        };
        let events = match cursors.get(&entity) {
            Some(c) if c.clip == clip => c.events.clone(),
            _ => match clip_file(&playing.clip) {
                Some(file) => match book.clip_events(database, &file, playing.clip.clip.duration) {
                    Some(events) => events,
                    // the database is being read: look again next frame
                    None => continue,
                },
                // a clip the client did not bind (none so far)
                None => Arc::from(Vec::new()),
            },
        };
        cursors.insert(entity, Cursor { clip, time: t, loops: playing.loops, events: events.clone() });
        if events.is_empty() {
            continue;
        }
        let obj_id = remote.map(|r| r.obj_id).or(local.map(|l| l.obj_id));
        for event in passed(&events, before, t, wrapped) {
            play_event(&mut commands, &audio, book, &terrain, &xforms, (entity, gt.translation()), animator, obj_id, event);
        }
    }
}

/// Plays one sound event of a clip of a unit: at the bone the event names, else at the unit.
#[allow(clippy::too_many_arguments)]
fn play_event(
    commands: &mut Commands,
    audio: &Audio,
    book: &SoundBook,
    terrain: &TerrainQuery,
    xforms: &Query<&GlobalTransform>,
    unit: (Entity, Vec3),
    animator: &Animator,
    obj_id: Option<u32>,
    event: &AnimSound,
) {
    let bone = (!event.bone.is_empty())
        .then(|| animator.rig.skeleton.bones.iter().position(|b| *b.name == *event.bone))
        .flatten()
        .and_then(|i| animator.bones.get(i).copied());
    let (parent, at) = match bone.and_then(|b| xforms.get(b).ok().map(|g| (b, g.translation()))) {
        Some(found) => found,
        None => (unit.0, unit.1),
    };
    let path: Option<String> = match event.kind {
        Kind::Sound => Some(event.param.to_string()),
        Kind::Pack => obj_id.and_then(|id| audio.pack_path(id, &event.param)),
        Kind::Footstep => {
            let surface = audio.surface(book, terrain, at);
            book.footstep(&event.param, &surface).map(|p| p.to_string())
        }
    };
    if let Some(path) = path {
        audio.start(commands, &Play { path: &path, class: Class::Foley, unit: unit.0, parent, at });
    }
}

/// The sound events of a clip that plays on the upper body of a moving unit (an animator layer, which `anim_sounds` cannot see):
/// they play by the clock from when the clip started.
#[derive(Component)]
pub struct LayerSounds {
    events: Arc<[AnimSound]>,
    /// When the clip started (seconds on the app clock) and how many events have played.
    start: f64,
    next: usize,
}

impl LayerSounds {
    pub fn new(events: Arc<[AnimSound]>, start: f64) -> LayerSounds {
        LayerSounds { events, start, next: 0 }
    }
}

/// Plays the events of the layered clips when their time comes.
#[allow(clippy::type_complexity)]
fn layer_sounds(
    mut commands: Commands,
    time: Res<Time>,
    audio: Audio,
    terrain: Res<TerrainQuery>,
    mut units: Query<(Entity, &Animator, &GlobalTransform, &mut LayerSounds, Option<&RemoteUnit>, Option<&LocalPlayer>)>,
    xforms: Query<&GlobalTransform>,
) {
    let Some(book) = audio.book() else { return };
    let now = time.elapsed_secs_f64();
    for (entity, animator, gt, mut layer, remote, local) in &mut units {
        let t = (now - layer.start) as f32;
        let obj_id = remote.map(|r| r.obj_id).or(local.map(|l| l.obj_id));
        while let Some(event) = layer.events.get(layer.next).cloned().filter(|e| e.time <= t) {
            layer.next += 1;
            play_event(&mut commands, &audio, book, &terrain, &xforms, (entity, gt.translation()), animator, obj_id, &event);
        }
        if layer.next >= layer.events.len() {
            commands.entity(entity).remove::<LayerSounds>();
        }
    }
}

/// `AA_AUDIO_LOG=1`: how many sounds each class started, was too far for and was dropped for the cap, every 5 seconds.
fn log_counts(time: Res<Time<Real>>, book: Res<SoundBook>, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since < 5.0 {
        return;
    }
    *since = 0.0;
    let [foley, voice, skill] = book.counts();
    if foley.0 + voice.0 + skill.0 > 0 {
        info!(
            "sound: started foley {} voice {} skill {}; too far {} / {} / {}; over the cap {} / {} / {}",
            foley.0, voice.0, skill.0, foley.1, voice.1, skill.1, foley.2, voice.2, skill.2
        );
    }
}

/// The sound database exists when there is an engine that is audible or logged (muted runs read nothing).
fn init(mut commands: Commands, engine: Option<Res<Engine>>) {
    let Some(engine) = engine else { return };
    if engine.stats().audible || log_enabled() {
        commands.insert_resource(SoundBook::open());
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(Startup, init)
        .add_systems(
            Update,
            (anim_sounds.run_if(resource_exists::<TerrainQuery>), layer_sounds.run_if(resource_exists::<TerrainQuery>), log_counts.run_if(log_enabled))
                .run_if(resource_exists::<SoundBook>)
                .run_if(in_state(Screen::InGame)),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_parameters_are_on_the_submitted_sound_entity() {
        fn submit(mut commands: Commands, book: Res<SoundBook>, marks: Query<(), With<Mark>>) {
            let parent = commands.spawn_empty().id();
            let sound = Sound::event("sounds/x2weapons_sword:sword:sword_wind")
                .with_param("pierce",0.3).with_param("miss",0.0)
                .with_param("critical",1.0).with_param("twohand",1.0);
            let path = sound.event.clone();
            book.start_sound(&mut commands,None,&marks,0.0,&Play { path: &path, class: Class::Skill, unit: parent, parent, at: Vec3::ZERO },sound).unwrap();
        }
        let mut world = World::new();
        world.insert_resource(SoundBook::open());
        let mut schedule = Schedule::default();
        schedule.add_systems(submit);
        schedule.run(&mut world);
        let mut sounds = world.query::<(&Sound,&ChildOf,&Mark)>();
        let (sound,_,_) = sounds.single(&world).unwrap();
        assert_eq!(sound.params,[("pierce".into(),0.3),("miss".into(),0.0),("critical".into(),1.0),("twohand".into(),1.0)]);
        assert!(sound.despawn_when_done);
        assert_eq!(world.resource::<SoundBook>().counts()[Class::Skill.index()].0,1);
    }

    #[test]
    fn clip_sounds_by_file_name_in_time_order() {
        let xml = r#"<anim_event_list>
 <animation name="animations/nuian/male/fist/fist_mo_normal_run_f.caf">
  <event name="footstep" time="0.61" parameter="footstep_nuian" bone="Bip01 R Foot" offset="0,0,0" dir="0,0,0"/>
  <event name="footstep" time="0.12" parameter="footstep_nuian" bone="Bip01 L Foot" offset="0,0,0" dir="0,0,0"/>
  <event name="skill_effect" time="0.3" parameter="launch" bone="" offset="0,0,0" dir="0,0,0"/>
 </animation>
 <animation name="animations/nuian/male/fist/fist_ac_punishment_hit.caf">
  <event name="sound_pack" time="0" parameter="Sounds/x2character:nuian_male:dmg" bone="" offset="0,0,0" dir="0,0,0"/>
  <event name="sound" time="0.2" parameter="footstep_ferre_walk" bone="" offset="0,0,0" dir="0,0,0"/>
  <event name="sound" time="0.4" parameter="Sounds/x2weapons:x2weapons:shield_flesh" bone="" offset="0,0,0" dir="0,0,0"/>
  <event name="sound" time="0.5" parameter="" bone="" offset="0,0,0" dir="0,0,0"/>
 </animation>
</anim_event_list>"#;
        let table = parse_clip_sounds(xml);
        let run = &table["fist_mo_normal_run_f.caf"];
        assert_eq!(run.iter().map(|e| (e.time, e.kind)).collect::<Vec<_>>(), vec![(0.12, Kind::Footstep), (0.61, Kind::Footstep)]);
        assert_eq!(&*run[0].bone, "Bip01 L Foot");
        let hit = &table["fist_ac_punishment_hit.caf"];
        assert_eq!(hit.iter().map(|e| e.kind).collect::<Vec<_>>(), vec![Kind::Pack, Kind::Footstep, Kind::Sound]);
    }

    #[test]
    fn a_direction_clip_belongs_to_its_locomotion_group() {
        assert_eq!(locomotion_group("fist_mo_normal_run_fl.caf").as_deref(), Some("fist_mo_normal_run.lmg"));
        assert_eq!(locomotion_group("onehand_mo_normal_walk_b.caf").as_deref(), Some("onehand_mo_normal_walk.lmg"));
        assert_eq!(locomotion_group("fist_ba_relaxed_idle.caf"), None);
        assert_eq!(locomotion_group("fist_co_attack_r.caf"), None);
    }

    #[test]
    fn a_playhead_passes_each_event_once() {
        let ev = |time| AnimSound { time, kind: Kind::Footstep, param: Arc::from("f"), bone: Arc::from("") };
        let events = [ev(0.0), ev(0.3), ev(0.7)];
        let times = |before, now, wrapped| passed(&events, before, now, wrapped).map(|e| e.time).collect::<Vec<_>>();
        // a clip that starts: the event at 0 counts
        assert_eq!(times(-1.0, 0.05, false), vec![0.0]);
        assert_eq!(times(0.05, 0.2, false), Vec::<f32>::new());
        assert_eq!(times(0.2, 0.4, false), vec![0.3]);
        // the loop closes: the end of the clip, then the start again
        assert_eq!(times(0.6, 0.1, true), vec![0.0, 0.7]);
        // a long frame skips two
        assert_eq!(times(0.1, 0.8, false), vec![0.3, 0.7]);
    }

    #[test]
    fn the_sheet_gives_an_effect_per_row_and_surface() {
        let xml = r#"<Workbook xmlns="urn:schemas-microsoft-com:office:spreadsheet" xmlns:ss="urn:schemas-microsoft-com:office:spreadsheet"><Worksheet><Table>
<Row><Cell ss:Index="2"><Data ss:Type="String">mat_metal</Data></Cell><Cell><Data ss:Type="String">mat_dirt</Data></Cell><Cell><Data ss:Type="String">mat_default</Data></Cell></Row>
<Row><Cell><Data ss:Type="String">footstep_nuian_walk</Data></Cell><Cell><Data ss:Type="String">footstep_nuian_walk:metal_thick</Data></Cell><Cell><Data ss:Type="String">footstep_nuian_walk:dirt</Data></Cell><Cell ss:Index="4"><Data ss:Type="String">footstep_nuian_walk:default</Data></Cell></Row>
</Table></Worksheet></Workbook>"#;
        let sheet = Footsteps { rows: parse_sheet(xml) };
        assert_eq!(sheet.cell("Footstep_Nuian_Walk", "mat_dirt").as_deref(), Some("footstep_nuian_walk:dirt"));
        assert_eq!(sheet.cell("footstep_nuian_walk", "mat_metal").as_deref(), Some("footstep_nuian_walk:metal_thick"));
        // an unknown surface takes the default column
        assert_eq!(sheet.cell("footstep_nuian_walk", "mat_lava").as_deref(), Some("footstep_nuian_walk:default"));
        assert_eq!(sheet.cell("footstep_horse", "mat_dirt"), None);
    }

    #[test]
    fn fxlibs_name_the_sound_of_an_effect() {
        let xml = r#"<FXLib><Effect name="Dirt"><Sound><Name>sounds/x2physics:mid_fs:dirt_walk</Name></Sound></Effect><Effect name="ice"><Particle><Name>a.b</Name></Particle></Effect></FXLib>"#;
        let lib = parse_fxlib(xml);
        assert_eq!(lib.get("dirt").map(String::as_str), Some("sounds/x2physics:mid_fs:dirt_walk"));
        assert!(!lib.contains_key("ice"));
    }

    #[test]
    fn particle_sounds_belong_to_the_top_level_effect() {
        let xml = r#"<?xml version="1.0"?><ParticleLibrary>
 <Particles Name="p_effect.a"><Params Sound="Sounds/x2skills:wild:one" SoundFXParam="1"/><Childs><Particles Name="child"><Params Sound="sounds/x2skills:wild:two"/><Childs><Particles Name="deep"><Params Count="2"/></Particles></Childs></Particles></Childs></Particles>
 <Particles Name="p_effect.b"><Params Count="1"/></Particles>
 <Particles Name="p_effect.c"><Params Sound="Sounds/x2skills:wild:one"/></Particles>
 <Particles Name="P_Effect.D"><Params Sound="sounds/x2skills:wild:three"/><Childs><Particles Name="x"><Params Sound="Sounds/X2Skills:Wild:Three"/></Particles></Childs></Particles>
</ParticleLibrary>"#;
        let t = scan_particle_sounds(xml);
        let names = |k: &str| t.get(k).map(|s| s.iter().map(|p| p.to_string()).collect::<Vec<_>>());
        assert_eq!(names("p_effect.a"), Some(vec!["Sounds/x2skills:wild:one".to_string(), "sounds/x2skills:wild:two".to_string()]));
        assert_eq!(names("p_effect.b"), None);
        assert_eq!(names("p_effect.c"), Some(vec!["Sounds/x2skills:wild:one".to_string()]));
        // the same event twice (any case) once
        assert_eq!(names("p_effect.d").map(|v| v.len()), Some(1));
    }

    #[test]
    fn material_surface_is_read_from_the_first_tag() {
        let mtl = r#"<Material MtlFlags="45" Shader="Terrain.Layer" SurfaceType="mat_Dirt" Diffuse="1,1,1"><Textures/></Material>"#;
        assert_eq!(material_surface(mtl).as_deref(), Some("mat_dirt"));
        assert_eq!(material_surface("<Material Shader=\"x\"/>"), None);
    }

    #[test]
    fn real_data() {
        if !aa_cry::pak_available() {
            return;
        }
        // the sheet and the lib of the walking nuian
        let sheet = Footsteps::load();
        let cell = sheet.cell("footstep_nuian_walk", "mat_dirt").expect("dirt cell");
        let (lib, effect) = cell.split_once(':').unwrap();
        let sound = read_fxlib(lib).get(effect).cloned();
        assert_eq!(sound.as_deref(), Some("sounds/x2physics:mid_fs:dirt_walk"));
        // the clips of the nuian male
        let bytes = aa_cry::read("animations/nuian/male/sound_event_database.animevents").unwrap();
        let table = parse_clip_sounds(&String::from_utf8_lossy(&bytes));
        assert!(table.values().flat_map(|e| e.iter()).filter(|e| e.kind == Kind::Footstep).count() > 100);
        // the particle library of the arrow has sounds somewhere
        let bytes = aa_cry::read("libs/particles/x2_sounds.xml").unwrap();
        assert!(!scan_particle_sounds(&String::from_utf8_lossy(&bytes)).is_empty());
        // the surface of the dirt material
        let mtl = aa_cry::read("materials/aa_terrain/field/tr_field_dirt_001c.mtl").unwrap();
        assert_eq!(material_surface(&String::from_utf8_lossy(&mtl)).as_deref(), Some("mat_dirt"));
    }
}
