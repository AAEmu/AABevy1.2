//! What the game database says a skill looks like: its clips, its effect groups, its projectile.
//!
//! The 1.2 data links a skill to its visuals in four places (see `docs/aa-client-combat.md`):
//!
//! * `skills`: `start_anim_id` (the looping cast clip), `fire_anim_id` / `twohand_fire_anim_id` /
//!   `dual_wield_fire_anim_id` (the one-shot clip when it fires), `fx_group_id` (particle effects),
//!   `projectile_id`, `effect_delay` (the flight time and the cast time come from the server's packets);
//! * `anims`: id -> the clip name of the model's `.cal` (`bow_co_sk_launch`);
//! * `fx_groups` / `fx_group_fx_items` / `fx_items`: the particle effects of a group, each with the moment of the skill it
//!   starts and ends at ([`ev`]), where it plays (caster or target, which bone) and the library name of the effect;
//! * `plot_effects` -> `special_effects`: most player skills are plots, and every step of a plot plays its own clip
//!   (special effect 34 `Anim`), effect group (35 / 36) and projectile (37 / 38);
//! * `skill_visual_groups`: the effect group and projectile a skill has from a skill level on; `skill_controllers`: the clip of
//!   a dash or leap; the `*_instrument_*_anim_id` columns: the clips of a song by instrument; `anims.loop`: a clip that
//!   loops until the skill ends or the next step plays; `fx_shake_cameras`: what the camera does when the effect lands.
//!
//! Everything is read on first use and cached by id, so the frame never touches the database after a skill was seen once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags, params};

/// A tool a skill or an NPC posture puts in a hand (`item_tools` + its `item_assets` row): the shovel of uproot, a watering
/// pot, the flowerpot of a florist.
#[derive(Clone, Debug, PartialEq)]
pub struct Tool {
    pub path: String,
    /// Offset in the hand bone's frame (m) and rotation (degrees about x, y, z), as `item_assets` has them for weapons.
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
}

/// The `item_assets` row of the tool item template `item_id` (`item_tools.item_id`): its model and attachment offsets. Shared by
/// [`SkillBook`] and the game database lookups (the in-game `Db`), which each hold their own connection.
pub(crate) fn tool_from_conn(conn: &Connection, item_id: u32) -> Option<Tool> {
    if item_id == 0 {
        return None;
    }
    conn.query_row(
        "SELECT ia.path, ia.attachment_offset_pos_x, ia.attachment_offset_pos_y, ia.attachment_offset_pos_z, \
                ia.attachment_offset_rot_x, ia.attachment_offset_rot_y, ia.attachment_offset_rot_z \
         FROM item_tools t JOIN item_assets ia ON ia.id = t.asset_id WHERE t.item_id = ?1",
        params![item_id],
        |r| {
            let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
            let path = r.get::<_, String>(0)?.replace('\\', "/").to_ascii_lowercase();
            Ok(Tool { path, offset: [f(1)?, f(2)?, f(3)?], rotation: [f(4)?, f(5)?, f(6)?] })
        },
    )
    .ok()
    .filter(|t| !t.path.is_empty())
}

/// The moments of a skill an effect starts and ends at (`fx_items.fx_event_start_id` / `fx_event_end_id`; the numbering is
/// read off the effect names: `_form` starts at 1, `_launch` at 3, `_hit` at 4, `_dot` at 0 and everything ends at 5).
pub mod ev {
    /// The skill begins (the server accepted it).
    pub const START: u8 = 0;
    /// The cast begins ("form": the charge-up around the hands).
    pub const FORM: u8 = 1;
    /// The cast completes, just before the skill fires ("throw": drawing the arrow, raising the hand).
    pub const THROW: u8 = 2;
    /// The skill fires ("launch": the swing, the shot).
    pub const LAUNCH: u8 = 3;
    /// The effect lands on the target.
    pub const HIT: u8 = 4;
    /// The skill ends.
    pub const END: u8 = 5;
    /// The voice line of the launch (`FxVoice` items; the clip's `voice_launch` anim event says when).
    pub const VOICE: u8 = 7;
    /// `fx_items.offset_axis_id` 3: the effect has the orientation of its bone (the blade trails on the weapon hand); 4 turns a
    /// hit effect towards the attacker.
    pub const BONE_AXIS: u8 = 3;
}

/// What a sound of an effect group plays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sfx {
    /// An event path (`FxSound`, `fx_particles.sound_id`).
    Event(Arc<str>),
    /// An item of the caster's sound pack (`FxVoice`: `sv_fight_triple_slash_1_launch`).
    Voice(Arc<str>),
}

/// What an effect group holds besides particle effects. The client plays the sounds and voices (see [`FxItem::sound`]) and the camera
/// shakes ([`ShakeItem`]) and none of the rest (material flashes, models, ropes); all counted so that `examples/skill_coverage.rs` can
/// tell how many skills lose something.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    /// `FxSound` items.
    pub sound: u16,
    /// Particle effects that carry a sound of their own (`fx_particles.sound_id` / `sound_pack_id`).
    pub particle_sound: u16,
    /// `FxVoice`: the character's voice line.
    pub voice: u16,
    /// `FxShakeCamera`.
    pub shake: u16,
    /// `FxMaterial`: a material flash on the unit.
    pub material: u16,
    /// `FxRope`, `FxCamFov`, `FxMotionBlur`, ... (`FxChr` is played: [`FxItem::chr`])
    pub other: u16,
}

impl Extras {
    /// Read by `examples/skill_coverage.rs`.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        *self == Extras::default()
    }

    /// The two added up.
    pub fn plus(self, o: Extras) -> Extras {
        Extras {
            sound: self.sound + o.sound,
            particle_sound: self.particle_sound + o.particle_sound,
            voice: self.voice + o.voice,
            shake: self.shake + o.shake,
            material: self.material + o.material,
            other: self.other + o.other,
        }
    }
}

/// What the camera does (`fx_shake_cameras`): angles in degrees (Cry order: x pitch, y roll, z yaw), shifts in metres in the camera's
/// own axes (x right, y forward, z up), a new random pose every `frequency` seconds, `randomness` 0 (one smooth swing) to 1
/// (noise), for `duration` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shake {
    pub ang: Vec3,
    pub shift: Vec3,
    pub frequency: f32,
    pub randomness: f32,
    pub duration: f32,
}

/// A camera shake of an effect group (`FxShakeCamera` items): it starts at an [`ev`] moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShakeItem {
    pub start: u8,
    pub on_target: bool,
    pub shake: Shake,
}

/// One particle effect of an effect group, or one sound of it (`effect` is empty then and `sound` the whole item).
#[derive(Clone, Debug, PartialEq)]
pub struct FxItem {
    /// `library.group.name` for aa-fx (`X2_EFFECTS.p_effect.p0000`); the `.chr` path of a character effect ([`FxItem::chr`]); empty for a
    /// sound item.
    pub effect: String,
    /// `FxChr`: `effect` is a skinned model that plays its own start / loop / end clips (Summon Crows' crows, the pet net).
    pub chr: bool,
    /// A sound item (`FxSound`, `FxVoice`), or the sound a particle effect carries in its database row (`fx_particles.sound_id`).
    pub sound: Option<Sfx>,
    /// [`ev`] it starts at and ends at (an effect that has played out by itself needs no end).
    pub start: u8,
    pub end: u8,
    /// Plays on the target (`fx_location_id` 2) and not on the caster (1).
    pub on_target: bool,
    /// `fx_items.bone_id`, see [`bone_names`].
    pub bone: u8,
    /// Offset from the bone in cry axes (m).
    pub offset: Vec3,
    /// `fx_items.offset_axis_id`: [`ev::BONE_AXIS`] keeps the bone's orientation, 4 turns a hit effect towards the attacker.
    pub axis: u8,
    /// `fx_particles.scale`.
    pub scale: f32,
    /// A song's sound (`fx_particles.sound_pack_id`: a pack whose items are `$holdable_21` / `_22` / `_23`, one per kind of instrument
    /// the caster holds: string, percussion, tube), by kind.
    pub song: [Option<Arc<str>>; 3],
    /// `FxMaterial`: a flash of a dual material over the unit the item is on (the hit of a normal attack), see [`super::flash`].
    pub flash: Option<super::flash::Flash>,
}

impl FxItem {
    /// The sound this item plays for a caster that holds the instrument of `kind` (see [`FxItem::song`]), else its own.
    pub fn sound_for(&self, instrument: Option<usize>) -> Option<Sfx> {
        instrument.and_then(|k| self.song.get(k)).and_then(|s| s.clone()).map(Sfx::Event).or_else(|| self.sound.clone())
    }

    /// A burst that is drawn flat: Crippling Mire's `psychokinesis_launch_form_end` (THROW, axis 0, on the hand). The effect is a ring
    /// (cracks and a ring mesh, `Facing = Velocity`) whose emitters' axis must point up: with the caster's forward it stands upright
    /// around the caster, which is not what the real client shows (a faint dark ellipse flat on the ground, once). It plays once:
    /// the real client's first frame has it, the next ones do not.
    pub fn flat(&self) -> bool {
        self.start == ev::THROW && self.axis == 0 && !self.chr && self.effect.ends_with("_form_end")
    }

    /// A buff visual or sound (damage over time, auras): it belongs to the buff, not to the skill.
    pub fn is_dot(&self) -> bool {
        self.effect.to_ascii_lowercase().contains("_dot") || matches!(&self.sound, Some(Sfx::Event(p)) if self.effect.is_empty() && p.to_ascii_lowercase().contains("_dot"))
    }
}

/// A thing that flies from the caster to the target (`projectiles`).
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub id: u32,
    /// What it looks like in flight (an effect group: the items start at [`ev::START`] and ride the projectile).
    pub items: Arc<Vec<FxItem>>,
    /// The camera shakes of its group.
    pub shakes: Arc<Vec<ShakeItem>>,
    pub extras: Extras,
    /// Where it starts and ends (`fx_items.bone_id` numbering; 0 = the caster's launch bone from the effect item).
    pub src_bone: u8,
    pub dest_bone: u8,
    /// `projectiles.proj_physic_id`: 0 and 1 fly from the caster to the target (a bolt, an arrow), 2 is a link between them (the
    /// beams of Arc Lightning, Telekinesis, Lasso: the effect's mesh is stretched from the launch bone to the target), 3 is an effect area standing on
    /// the ground at the target (Meteor Strike, Sunder Earth, the magic circles).
    pub physic: u8,
}

impl Projectile {
    /// A link between the caster and the target, not a thing that flies.
    pub fn is_beam(&self) -> bool {
        self.physic == 2
    }

    /// An effect area (`proj_physic_id` 3): stands on the ground at the target.
    pub fn is_zone(&self) -> bool {
        self.physic == 3
    }
}

/// What a skill shows besides its clips: the effects, the camera shakes and the projectile of one of its visual groups.
#[derive(Clone, Debug, Default)]
pub struct Look {
    pub fx: Arc<Vec<FxItem>>,
    pub shakes: Arc<Vec<ShakeItem>>,
    pub projectile: Option<Projectile>,
}

/// What cancels a cast of the player besides moving, which always ends the cast time (live game behaviour, not a flag): the skill's
/// flags say whether moving also ends its channel (`channeling_cancelable`: Invincibility, Twilight, Absorb Lifeforce, the tooltips say
/// "canceled if you move") and whether turning ends the cast (`stop_casting_by_turn`). `casting_cancelable` is not read: it is
/// most likely the player's own cancel of the cast.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CancelRules {
    /// `channeling_cancelable`: moving ends the channel.
    pub channel_on_move: bool,
    /// `stop_casting_by_turn`: turning the character ends the cast.
    pub on_turn: bool,
    /// `source_not_swim`: the skill cannot be used swimming.
    pub not_swim: bool,
}

/// The looks of one skill.
#[derive(Clone, Debug, Default)]
pub struct SkillVisual {
    pub cancel: CancelRules,
    /// Clip names of the caster's `.cal`.
    pub start_anim: Option<Arc<str>>,
    pub fire_anim: Option<Arc<str>>,
    pub twohand_fire_anim: Option<Arc<str>>,
    pub dual_fire_anim: Option<Arc<str>>,
    /// The clip of the skill's controller (a dash, a leap, a back step): all the skill has when it has no clips of its own.
    pub controller_anim: Option<Arc<str>>,
    /// The clips of a song by the instrument the caster holds (string, percussion, tube): the cast loop and the fire clip.
    pub instrument_start: [Option<Arc<str>>; 3],
    pub instrument_fire: [Option<Arc<str>>; 3],
    /// The tools the skill's clips put in the hands (`skills.mainhand_tool_id` / `offhand_tool_id` through `item_tools`):
    /// the uproot shovel, the watering pot. Main hand first; `None` when the skill uses none.
    pub tools: [Option<Tool>; 2],
    pub fx: Arc<Vec<FxItem>>,
    /// The camera shakes of the skill's effect group (the base look).
    pub shakes: Arc<Vec<ShakeItem>>,
    /// The groups a skill has from a skill level on (`skill_visual_groups`), ascending by level.
    pub levels: Vec<(u8, Look)>,
    /// Not drawn by the client; counted by `examples/skill_coverage.rs`.
    #[allow(dead_code)]
    pub extras: Extras,
    pub projectile: Option<Projectile>,
    /// The projectiles of the clouts of the doodad the channel spawns (`skills.channeling_doodad_id`, Protective Wings' doodad 879:
    /// `doodad_func_clouts.projectile_id`, an effect area on the ground under the caster for as long as the channel lasts). The
    /// buff of a clout reaches the units from the server (the area trigger applies it), the projectile is the client's.
    pub clout_projectiles: Vec<Projectile>,
    /// `skills.channeling_anim_id` (not played yet; checked by `examples/skill_coverage.rs`).
    #[allow(dead_code)]
    pub channeling_anim: Option<Arc<str>>,
    /// `skills.effect_delay`: how long after the fire the shot leaves (the launch moment of the clip), ms.
    pub effect_delay_ms: u32,
    /// The skill is a plot: its steps carry the clips and effects ([`PlotVisual`]).
    pub plot: bool,
    /// `skills.casting_time` / `channeling_time` (ms): what the dev sweep (`AA_COMBAT_SWEEP`) plays the skill with.
    pub cast_ms: u32,
    pub channel_ms: u32,
}

impl SkillVisual {
    /// The tool the skill holds in its main hand (`skills.mainhand_tool_id`), if any.
    #[allow(dead_code)]
    pub fn mainhand_tool(&self) -> Option<&Tool> {
        self.tools[0].as_ref()
    }

    /// The tool the skill holds in its off hand (`skills.offhand_tool_id`), if any.
    #[allow(dead_code)]
    pub fn offhand_tool(&self) -> Option<&Tool> {
        self.tools[1].as_ref()
    }

    /// The effects, shakes and projectile at skill `level`: the last visual group at or below it, else the skill's own.
    pub fn look(&self, level: u8) -> Look {
        match self.levels.iter().rev().find(|(from, _)| *from <= level) {
            Some((_, look)) => look.clone(),
            None => Look { fx: self.fx.clone(), shakes: self.shakes.clone(), projectile: self.projectile.clone() },
        }
    }

    /// The cast clip of a song for the instrument kind the caster holds (0 string, 1 percussion, 2 tube).
    pub fn instrument_start_clip(&self, kind: Option<usize>) -> Option<&Arc<str>> {
        self.instrument_start.get(kind?)?.as_ref()
    }

    /// The fire clip of a song for the instrument kind the caster holds.
    pub fn instrument_fire_clip(&self, kind: Option<usize>) -> Option<&Arc<str>> {
        self.instrument_fire.get(kind?)?.as_ref()
    }

    /// The fire clip for a caster with (`two_handed`) or without a two handed weapon, `dual` wielding or neither.
    pub fn fire_clip(&self, two_handed: bool, dual: bool) -> Option<&Arc<str>> {
        let pick = if dual {
            self.dual_fire_anim.as_ref()
        } else if two_handed {
            self.twohand_fire_anim.as_ref()
        } else {
            None
        };
        pick.or(self.fire_anim.as_ref())
    }
}

/// What one step of a plot does (`SCPlotEvent::event_id`).
#[derive(Clone, Debug, Default)]
pub struct PlotVisual {
    /// The clip the step plays: the last of its anims that is not a stop (see [`is_stop_clip`]).
    pub anim: Option<Arc<str>>,
    /// Every anim of the step, in order (most steps with two start with a stop).
    pub anims: Vec<Arc<str>>,
    pub fx: Arc<Vec<FxItem>>,
    /// The camera shakes of the step's effect groups.
    pub shakes: Arc<Vec<ShakeItem>>,
    pub extras: Extras,
    pub projectile: Option<Projectile>,
    /// `special_effects.value3` of the step's projectile effect: its speed (m/s; 0 = the default flight time).
    pub projectile_speed: f32,
    /// `special_effects.value2` of the step's projectile effect (`ProjectileAnim`): how long a beam lives, in ms (Lasso 700, Crippling
    /// Mire 4500, Absorb Lifeforce 6500; 0 for Arc Lightning and Mana Force, whose effects play out by themselves).
    pub projectile_ms: u32,
}

impl PlotVisual {
    /// The step plays a stop marker: it ends the clip that loops before it.
    pub fn stops(&self) -> bool {
        self.anims.iter().any(|a| is_stop_clip(a))
    }

    /// The step does nothing but end a clip (a stop marker and no effects).
    pub fn stops_only(&self) -> bool {
        self.stops() && self.clip().is_none() && self.fx.is_empty() && self.shakes.is_empty() && self.projectile.is_none()
    }

    /// The clip the step plays (`None` for a step that only stops).
    pub fn clip(&self) -> Option<&Arc<str>> {
        self.anim.as_ref().filter(|a| !is_stop_clip(a))
    }
}

/// What a buff looks like on the unit that carries it (`buffs`): its aura (the effect group; the `_dot` effects of the skills that
/// apply it), the pose it holds the unit in (`anim_start_id`: stun, struggle, an emote, a song) and what freezes it.
#[derive(Clone, Debug, Default)]
pub struct BuffLook {
    /// The particle effects and sounds of the buff's group: the ones that end at [`ev::END`] live as long as the buff, `start == END`
    /// ones play when it goes.
    pub fx: Arc<Vec<FxItem>>,
    /// `anim_start_id`: the clip the unit plays while the buff lasts (`None` for none and for the stop markers).
    pub start_anim: Option<Arc<str>>,
    /// `anim_end_id`: the clip when it ends (`None` for a stop marker, which only ends the pose).
    pub end_anim: Option<Arc<str>>,
    /// The cast pose of a song by the instrument held (string, percussion, tube).
    pub instrument_start: [Option<Arc<str>>; 3],
    /// `framehold`: the animation freezes (ice, stone, a trap).
    pub framehold: bool,
    /// A debuff (`buffs.kind_id` 2): the dev sweep puts it on the target, the good ones on the caster.
    pub bad: bool,
    /// `ragdoll`: the unit falls over (the client has no ragdoll: it plays the death clip and gets up again).
    pub ragdoll: bool,
    /// `knock_down` (163 buffs, none with a clip): the unit is thrown on its back (or its front) and gets up when the buff goes.
    pub knock_down: bool,
}

impl BuffLook {
    /// The buff changes nothing the client draws.
    pub fn is_empty(&self) -> bool {
        self.fx.is_empty() && self.start_anim.is_none() && self.end_anim.is_none() && self.instrument_start.iter().all(Option::is_none) && !self.framehold && !self.knock_down && !self.ragdoll
    }
}

/// `all_co_sk_stop` / `all_ac_stop`: the anim a plot step plays first to end the clip before it, followed by the clip it wants.
pub fn is_stop_clip(name: &str) -> bool {
    name.ends_with("_stop")
}

/// The clip names to try for the upper body variant `upper` (`anims.move_ub`): the `.cal` of most models calls
/// `bow_co_sk_launch_ub` `bow_co_sk_launch_mub` ("move, upper body"), the others as given.
pub fn upper_candidates(upper: &str) -> impl Iterator<Item = String> {
    upper.strip_suffix("_ub").map(|stem| format!("{stem}_mub")).into_iter().chain(std::iter::once(upper.to_string()))
}

/// The skill database, cached by id. A resource: lookups take `&self` and a short lock.
#[derive(Resource)]
pub struct SkillBook(Mutex<Inner>);

struct Inner {
    conn: Connection,
    skills: HashMap<u32, Option<Arc<SkillVisual>>>,
    plots: HashMap<u32, Option<Arc<PlotVisual>>>,
    anims: HashMap<u32, Option<Arc<str>>>,
    /// Clip name -> the `move_ub` clip of its `anims` row.
    upper: HashMap<String, Option<Arc<str>>>,
    /// Clip name -> `anims.loop`.
    loops: HashMap<String, bool>,
    groups: HashMap<u32, Arc<Vec<FxItem>>>,
    shakes: HashMap<u32, Arc<Vec<ShakeItem>>>,
    extras: HashMap<u32, Extras>,
    projectiles: HashMap<u32, Option<Projectile>>,
    buffs: HashMap<u32, Option<Arc<BuffLook>>>,
    tools: HashMap<u32, Option<Tool>>,
}

impl SkillBook {
    /// Opens the game database (`AA_DB`, else the server's copy) read only.
    pub fn open() -> Option<SkillBook> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).ok()?;
        Some(SkillBook(Mutex::new(Inner {
            conn,
            skills: HashMap::new(),
            plots: HashMap::new(),
            anims: HashMap::new(),
            upper: HashMap::new(),
            loops: HashMap::new(),
            groups: HashMap::new(),
            shakes: HashMap::new(),
            extras: HashMap::new(),
            projectiles: HashMap::new(),
            buffs: HashMap::new(),
            tools: HashMap::new(),
        })))
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The looks of a skill; `None` when the database does not know it.
    pub fn skill(&self, id: u32) -> Option<Arc<SkillVisual>> {
        self.inner().skill(id)
    }

    /// What a plot step plays; `None` when it has no clip, effect or projectile.
    pub fn plot_event(&self, event_id: u32) -> Option<Arc<PlotVisual>> {
        self.inner().plot_event(event_id)
    }

    /// The clip name of an `anims` row (`fire_anim_id` of `SCSkillFired`).
    pub fn anim(&self, id: u32) -> Option<Arc<str>> {
        self.inner().anim(id)
    }

    /// The upper body variant of clip `name` for a unit that moves (`move_ub` of its `anims` row), when it has one.
    pub fn upper_clip(&self, name: &str) -> Option<Arc<str>> {
        self.inner().upper_clip(name)
    }

    /// The clip loops (`anims.loop`): the cast and channel poses.
    pub fn clip_loops(&self, name: &str) -> bool {
        self.inner().clip_loops(name)
    }

    /// The buffs a skill applies (`skill_effects` -> `BuffEffect`, its channeling and toggle buff): the ones whose visuals its casts bring.
    pub fn skill_buffs(&self, skill_id: u32) -> Vec<u32> {
        let inner = self.inner();
        inner
            .conn
            .prepare(
                "SELECT be.buff_id FROM skill_effects s JOIN effects e ON e.id = s.effect_id AND e.actual_type = 'BuffEffect' JOIN buff_effects be ON be.id = e.actual_id WHERE s.skill_id = ?1 UNION SELECT channeling_buff_id FROM skills WHERE id = ?1 AND channeling_buff_id <> 0 UNION SELECT toggle_buff_id FROM skills WHERE id = ?1 AND toggle_buff_id <> 0",
            )
            .and_then(|mut q| q.query_map(params![skill_id], |r| Ok(int(r, 0)? as u32)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default()
    }

    /// What buff `id` looks like on its bearer; `None` when it shows nothing.
    pub fn buff(&self, id: u32) -> Option<Arc<BuffLook>> {
        self.inner().buff(id)
    }
}

/// Entry points for `examples/skill_coverage.rs`, which resolves every skill offline through the code above.
#[allow(dead_code)]
impl SkillBook {
    /// The `plot_events` ids of a skill's plot, in order (what `SCPlotEvent::event_id` will name).
    pub(crate) fn plot_steps(&self, skill_id: u32) -> Vec<u32> {
        let inner = self.inner();
        inner
            .conn
            .prepare("SELECT e.id FROM plot_events e JOIN skills s ON s.plot_id = e.plot_id WHERE s.id = ?1 ORDER BY e.position, e.id")
            .and_then(|mut q| q.query_map(params![skill_id], |r| Ok(int(r, 0)? as u32)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default()
    }

    /// The particle effects of an effect group.
    pub(crate) fn fx_group(&self, id: u32) -> Arc<Vec<FxItem>> {
        self.inner().group(id)
    }

    /// What an effect group holds besides particle effects.
    pub(crate) fn fx_extras(&self, id: u32) -> Extras {
        self.inner().extras_of(id)
    }

    /// A projectile by id.
    pub(crate) fn projectile(&self, id: u32) -> Option<Projectile> {
        self.inner().projectile(id)
    }
}

fn int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    Ok(r.get::<_, Option<i64>>(i)?.unwrap_or(0))
}

fn num(r: &rusqlite::Row, i: usize) -> rusqlite::Result<f32> {
    Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32)
}

impl Inner {
    fn anim(&mut self, id: u32) -> Option<Arc<str>> {
        if id == 0 {
            return None;
        }
        if let Some(a) = self.anims.get(&id) {
            return a.clone();
        }
        let name: Option<Arc<str>> = self
            .conn
            .query_row("SELECT name FROM anims WHERE id = ?1", params![id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            // two rows (345, 435) have a leading space
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty() && n != "none")
            .map(Arc::from);
        self.anims.insert(id, name.clone());
        name
    }

    /// The tool model of an item template (`item_tools` -> `item_assets`), cached.
    fn tool(&mut self, item_id: u32) -> Option<Tool> {
        if item_id == 0 {
            return None;
        }
        if let Some(t) = self.tools.get(&item_id) {
            return t.clone();
        }
        let tool = tool_from_conn(&self.conn, item_id);
        self.tools.insert(item_id, tool.clone());
        tool
    }

    fn upper_clip(&mut self, name: &str) -> Option<Arc<str>> {
        if let Some(u) = self.upper.get(name) {
            return u.clone();
        }
        let upper: Option<Arc<str>> = self
            .conn
            .query_row("SELECT move_ub FROM anims WHERE name = ?1", params![name], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .filter(|n| !n.is_empty() && n != "none")
            .map(Arc::from);
        self.upper.insert(name.to_string(), upper.clone());
        upper
    }

    fn clip_loops(&mut self, name: &str) -> bool {
        if let Some(l) = self.loops.get(name) {
            return *l;
        }
        let looping = self
            .conn
            .query_row("SELECT loop FROM anims WHERE name = ?1", params![name], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .is_some_and(|v| v == "t");
        self.loops.insert(name.to_string(), looping);
        looping
    }

    /// The camera shakes of an effect group.
    fn shakes_of(&mut self, id: u32) -> Arc<Vec<ShakeItem>> {
        if id == 0 {
            return Arc::default();
        }
        if let Some(s) = self.shakes.get(&id) {
            return s.clone();
        }
        let items = self
            .conn
            .prepare(
                "SELECT i.fx_event_start_id, i.fx_location_id, c.ang_x, c.ang_y, c.ang_z, c.shift_x, c.shift_y, c.shift_z, c.frequency, c.randomness, c.duration \
                 FROM fx_group_fx_items g JOIN fx_items i ON i.id = g.fx_item_id JOIN fx_shake_cameras c ON c.id = i.fx_detail_id \
                 WHERE g.fx_group_id = ?1 AND i.fx_detail_type = 'FxShakeCamera' ORDER BY g.rowid",
            )
            .and_then(|mut q| {
                q.query_map(params![id], |r| {
                    Ok(ShakeItem {
                        start: int(r, 0)? as u8,
                        on_target: int(r, 1)? == 2,
                        shake: Shake {
                            ang: Vec3::new(num(r, 2)?, num(r, 3)?, num(r, 4)?),
                            shift: Vec3::new(num(r, 5)?, num(r, 6)?, num(r, 7)?),
                            frequency: num(r, 8)?,
                            randomness: num(r, 9)?,
                            duration: num(r, 10)?,
                        },
                    })
                })
                .map(|rows| rows.flatten().collect::<Vec<_>>())
            })
            .unwrap_or_default();
        let items = Arc::new(items);
        self.shakes.insert(id, items.clone());
        items
    }

    /// The groups of `skill_visual_groups`, ascending by level.
    fn levels_of(&mut self, skill: u32) -> Vec<(u8, Look)> {
        let rows: Vec<(u8, u32, u32)> = self
            .conn
            .prepare("SELECT LEVEL, fx_group_id, projectile_id FROM skill_visual_groups WHERE owner_type = 'Skill' AND owner_id = ?1 ORDER BY LEVEL")
            .and_then(|mut q| q.query_map(params![skill], |r| Ok((int(r, 0)?.clamp(0, 255) as u8, int(r, 1)? as u32, int(r, 2)? as u32))).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        rows.into_iter().map(|(level, group, projectile)| (level, Look { fx: self.group(group), shakes: self.shakes_of(group), projectile: self.projectile(projectile) })).collect()
    }

    /// The particle effects, character effects and sounds of an effect group (camera shakes and materials are left out).
    fn group(&mut self, id: u32) -> Arc<Vec<FxItem>> {
        if id == 0 {
            return Arc::default();
        }
        if let Some(g) = self.groups.get(&id) {
            return g.clone();
        }
        let items = self
            .conn
            .prepare(
                "SELECT i.asset_name, i.fx_event_start_id, i.fx_event_end_id, i.fx_location_id, i.bone_id, i.offset_x, i.offset_y, i.offset_z, \
                        i.offset_axis_id, p.scale, i.fx_detail_type, s.path, v.sound_pack_item_name, \
                        (SELECT x.path FROM sound_pack_items it JOIN sounds x ON x.id = it.sound_id WHERE it.sound_pack_id = p.sound_pack_id AND it.name = '$holdable_21'), \
                        (SELECT x.path FROM sound_pack_items it JOIN sounds x ON x.id = it.sound_id WHERE it.sound_pack_id = p.sound_pack_id AND it.name = '$holdable_22'), \
                        (SELECT x.path FROM sound_pack_items it JOIN sounds x ON x.id = it.sound_id WHERE it.sound_pack_id = p.sound_pack_id AND it.name = '$holdable_23'), \
                        d.filename, m.custom_dual_material_fade_time \
                 FROM fx_group_fx_items g JOIN fx_items i ON i.id = g.fx_item_id \
                 LEFT JOIN fx_particles p ON p.id = i.fx_detail_id AND i.fx_detail_type = 'FxParticle' \
                 LEFT JOIN sounds s ON s.id = p.sound_id \
                 LEFT JOIN fx_voices v ON v.id = i.fx_detail_id AND i.fx_detail_type = 'FxVoice' \
                 LEFT JOIN fx_materials m ON m.id = i.fx_detail_id AND i.fx_detail_type = 'FxMaterial' \
                 LEFT JOIN custom_dual_materials d ON d.id = m.custom_dual_material_id \
                 WHERE g.fx_group_id = ?1 AND i.fx_detail_type IN ('FxParticle', 'FxSound', 'FxVoice', 'FxChr', 'FxMaterial') ORDER BY g.rowid",
            )
            .and_then(|mut q| {
                q.query_map(params![id], |r| {
                    let asset = r.get::<_, Option<String>>(0)?.unwrap_or_default().trim().to_string();
                    let scale = num(r, 9)?;
                    let kind = r.get::<_, Option<String>>(10)?.unwrap_or_default();
                    let path = r.get::<_, Option<String>>(11)?.map(|p| p.trim().to_string()).filter(|p| p.contains(':')).map(|p| Sfx::Event(Arc::from(p.as_str())));
                    let (effect, sound) = match kind.as_str() {
                        // the asset of a sound item is the event path (a few rows hold a bare name: no sound)
                        "FxSound" => {
                            let sound = asset.contains(':').then(|| Sfx::Event(Arc::from(asset.as_str())));
                            (String::new(), sound)
                        }
                        "FxMaterial" => (String::new(), None),
                        "FxVoice" => (String::new(), r.get::<_, Option<String>>(12)?.filter(|n| !n.trim().is_empty()).map(|n| Sfx::Voice(Arc::from(n.trim())))),
                        _ => (asset, path),
                    };
                    let song = |i: usize| -> rusqlite::Result<Option<Arc<str>>> {
                        Ok(r.get::<_, Option<String>>(i)?.map(|p| p.trim().to_string()).filter(|p| p.contains(':')).map(|p| Arc::from(p.as_str())))
                    };
                    Ok(FxItem {
                        song: [song(13)?, song(14)?, song(15)?],
                        flash: match (kind.as_str(), r.get::<_, Option<String>>(16)?) {
                            ("FxMaterial", Some(mtl)) => super::flash::Flash::load(&mtl, num(r, 17)?),
                            _ => None,
                        },
                        chr: kind == "FxChr",
                        effect,
                        sound,
                        start: int(r, 1)? as u8,
                        end: int(r, 2)? as u8,
                        on_target: int(r, 3)? == 2,
                        bone: int(r, 4)? as u8,
                        offset: Vec3::new(num(r, 5)?, num(r, 6)?, num(r, 7)?),
                        axis: int(r, 8)? as u8,
                        scale: if scale > 0.0 { scale } else { 1.0 },
                    })
                })
                .map(|rows| rows.flatten().filter(|i| if i.effect.is_empty() { i.sound.is_some() || i.flash.is_some() } else { i.effect.contains('.') }).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        let items = Arc::new(items);
        self.groups.insert(id, items.clone());
        items
    }

    /// What the group holds that is not a particle effect (see [`Extras`]).
    fn extras_of(&mut self, id: u32) -> Extras {
        if id == 0 {
            return Extras::default();
        }
        if let Some(e) = self.extras.get(&id) {
            return *e;
        }
        let mut extras = Extras::default();
        let rows: Vec<(String, bool)> = self
            .conn
            .prepare(
                "SELECT i.fx_detail_type, COALESCE(p.sound_id, 0) <> 0 OR COALESCE(p.sound_pack_id, 0) <> 0 \
                 FROM fx_group_fx_items g JOIN fx_items i ON i.id = g.fx_item_id \
                 LEFT JOIN fx_particles p ON p.id = i.fx_detail_id AND i.fx_detail_type = 'FxParticle' WHERE g.fx_group_id = ?1",
            )
            .and_then(|mut q| {
                q.query_map(params![id], |r| Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<bool>>(1)?.unwrap_or(false))))
                    .map(|rows| rows.flatten().collect())
            })
            .unwrap_or_default();
        for (kind, with_sound) in rows {
            match kind.as_str() {
                "FxParticle" => extras.particle_sound += u16::from(with_sound),
                "FxSound" => extras.sound += 1,
                "FxVoice" => extras.voice += 1,
                "FxShakeCamera" => extras.shake += 1,
                "FxChr" => {}
                "FxMaterial" => extras.material += 1,
                _ => extras.other += 1,
            }
        }
        self.extras.insert(id, extras);
        extras
    }

    fn projectile(&mut self, id: u32) -> Option<Projectile> {
        if id == 0 {
            return None;
        }
        if let Some(p) = self.projectiles.get(&id) {
            return p.clone();
        }
        let row = self
            .conn
            .query_row("SELECT fx_group_id, src_bone_id, dest_bone_id, proj_physic_id FROM projectiles WHERE id = ?1", params![id], |r| {
                Ok((int(r, 0)? as u32, int(r, 1)? as u8, int(r, 2)? as u8, int(r, 3)? as u8))
            })
            .ok();
        let projectile = row.map(|(group, src_bone, dest_bone, physic)| Projectile { id, items: self.group(group), shakes: self.shakes_of(group), extras: self.extras_of(group), src_bone, dest_bone, physic });
        self.projectiles.insert(id, projectile.clone());
        projectile
    }

    fn buff(&mut self, id: u32) -> Option<Arc<BuffLook>> {
        if let Some(b) = self.buffs.get(&id) {
            return b.clone();
        }
        let row = self
            .conn
            .query_row(
                "SELECT fx_group_id, anim_start_id, anim_end_id, framehold = 't', ragdoll = 't', kind_id = 2,                         string_instrument_start_anim_id, percussion_instrument_start_anim_id, tube_instrument_start_anim_id, knock_down = 't'                  FROM buffs WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        int(r, 0)? as u32,
                        int(r, 1)? as u32,
                        int(r, 2)? as u32,
                        int(r, 3)? != 0,
                        int(r, 4)? != 0,
                        std::array::from_fn::<u32, 3, _>(|i| int(r, 6 + i).unwrap_or(0) as u32),
                        int(r, 5)? != 0,
                        int(r, 9)? != 0,
                    ))
                },
            )
            .ok();
        let look = row
            .map(|(group, start, end, framehold, ragdoll, instruments, bad, knock_down)| BuffLook {
                bad,
                fx: self.group(group),
                // the stop markers only end the pose: they are not clips
                start_anim: self.anim(start).filter(|a| !is_stop_clip(a)),
                end_anim: self.anim(end).filter(|a| !is_stop_clip(a)),
                instrument_start: instruments.map(|i| self.anim(i)),
                framehold,
                ragdoll,
                knock_down,
            })
            .filter(|l| !l.is_empty())
            .map(Arc::new);
        self.buffs.insert(id, look.clone());
        look
    }

    /// The projectiles of the clouts of the doodad skill `skill` channels (`channeling_doodad_id`; the clouts of its first phase).
    fn clout_projectiles(&mut self, skill: u32) -> Vec<Projectile> {
        let ids: Vec<u32> = self
            .conn
            .prepare(
                "SELECT c.projectile_id FROM skills s JOIN doodad_func_groups g ON g.doodad_almighty_id = s.channeling_doodad_id AND g.doodad_func_group_kind_id = 1                  JOIN doodad_phase_funcs p ON p.doodad_func_group_id = g.id AND p.actual_func_type = 'DoodadFuncClout'                  JOIN doodad_func_clouts c ON c.id = p.actual_func_id WHERE s.id = ?1 AND c.projectile_id IS NOT NULL AND c.projectile_id > 0 ORDER BY p.id",
            )
            .and_then(|mut q| q.query_map(params![skill], |r| Ok(int(r, 0)? as u32)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        ids.into_iter().filter_map(|id| self.projectile(id)).collect()
    }

    fn skill(&mut self, id: u32) -> Option<Arc<SkillVisual>> {
        if let Some(s) = self.skills.get(&id) {
            return s.clone();
        }
        let row = self
            .conn
            .query_row(
                "SELECT start_anim_id, fire_anim_id, twohand_fire_anim_id, dual_wield_fire_anim_id, fx_group_id, projectile_id, effect_delay, plot_id, channeling_anim_id, \
                        (SELECT start_anim_id FROM skill_controllers c WHERE c.id = skills.skill_controller_id), \
                        string_instrument_start_anim_id, percussion_instrument_start_anim_id, tube_instrument_start_anim_id, \
                        string_instrument_fire_anim_id, percussion_instrument_fire_anim_id, tube_instrument_fire_anim_id, \
                        channeling_cancelable = 't', stop_casting_by_turn = 't', casting_time, channeling_time, \
                        mainhand_tool_id, offhand_tool_id, source_not_swim = 't' \
                 FROM skills WHERE id = ?1",
                params![id],
                |r| {
                    Ok((
                        [int(r, 0)? as u32, int(r, 1)? as u32, int(r, 2)? as u32, int(r, 3)? as u32],
                        int(r, 4)? as u32,
                        int(r, 5)? as u32,
                        int(r, 6)?.max(0) as u32,
                        int(r, 7)? != 0,
                        int(r, 8)? as u32,
                        int(r, 9)? as u32,
                        std::array::from_fn(|i| int(r, 10 + i).unwrap_or(0) as u32),
                        std::array::from_fn(|i| int(r, 13 + i).unwrap_or(0) as u32),
                        CancelRules { channel_on_move: int(r, 16).unwrap_or(0) != 0, on_turn: int(r, 17).unwrap_or(0) != 0, not_swim: int(r, 22).unwrap_or(0) != 0 },
                        (int(r, 18)?.max(0) as u32, int(r, 19)?.max(0) as u32),
                        (int(r, 20)?.max(0) as u32, int(r, 21)?.max(0) as u32),
                    ))
                },
            )
            .ok();
        let visual = row.map(|(anims, group, projectile, effect_delay_ms, plot, channeling, controller, instrument_start, instrument_fire, cancel, (cast_ms, channel_ms), (main_tool, off_tool))| {
            let [start, fire, twohand, dual] = anims;
            let tools = [self.tool(main_tool), self.tool(off_tool)];
            Arc::new(SkillVisual {
                cancel,
                start_anim: self.anim(start),
                fire_anim: self.anim(fire),
                twohand_fire_anim: self.anim(twohand),
                dual_fire_anim: self.anim(dual),
                controller_anim: self.anim(controller),
                instrument_start: instrument_start.map(|id: u32| self.anim(id)),
                instrument_fire: instrument_fire.map(|id: u32| self.anim(id)),
                tools,
                fx: self.group(group),
                shakes: self.shakes_of(group),
                levels: self.levels_of(id),
                extras: self.extras_of(group),
                projectile: self.projectile(projectile),
                clout_projectiles: self.clout_projectiles(id),
                channeling_anim: self.anim(channeling),
                effect_delay_ms,
                plot,
                cast_ms,
                channel_ms,
            })
        });
        self.skills.insert(id, visual.clone());
        visual
    }

    fn plot_event(&mut self, event_id: u32) -> Option<Arc<PlotVisual>> {
        if let Some(p) = self.plots.get(&event_id) {
            return p.clone();
        }
        // special effects of the event: 34 Anim (anim id), 35 / 36 FxGroup(Anim) (group id), 37 / 38 Projectile(Anim)
        let effects: Vec<(i64, u32, f32, u32)> = self
            .conn
            .prepare(
                "SELECT s.special_effect_type_id, s.value1, s.value3, s.value2 FROM plot_effects e JOIN special_effects s ON s.id = e.actual_id \
                 WHERE e.event_id = ?1 AND e.actual_type = 'SpecialEffect' AND s.special_effect_type_id IN (34, 35, 36, 37, 38) ORDER BY e.position, e.id",
            )
            .and_then(|mut q| q.query_map(params![event_id], |r| Ok((int(r, 0)?, int(r, 1)? as u32, int(r, 2)? as f32, int(r, 3)?.max(0) as u32))).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        let mut visual = PlotVisual::default();
        let mut fx: Vec<FxItem> = Vec::new();
        let mut shakes: Vec<ShakeItem> = Vec::new();
        for (kind, value, speed, ms) in effects {
            match kind {
                34 => visual.anims.extend(self.anim(value)),
                35 | 36 => {
                    fx.extend(self.group(value).iter().cloned());
                    shakes.extend(self.shakes_of(value).iter().copied());
                    visual.extras = visual.extras.plus(self.extras_of(value));
                }
                _ => {
                    let first = visual.projectile.is_none();
                    visual.projectile = visual.projectile.take().or_else(|| self.projectile(value));
                    if first && let Some(p) = &visual.projectile {
                        visual.extras = visual.extras.plus(p.extras);
                        visual.projectile_speed = speed;
                        visual.projectile_ms = ms;
                    }
                }
            }
        }
        // 450 steps play two anims: `all_co_sk_stop` (end the clip before) and then the clip itself; the step is the latter
        visual.anim = visual.anims.iter().rev().find(|a| !is_stop_clip(a)).or(visual.anims.first()).cloned();
        visual.fx = Arc::new(fx);
        visual.shakes = Arc::new(shakes);
        let plot = (visual.anim.is_some() || !visual.fx.is_empty() || !visual.shakes.is_empty() || visual.projectile.is_some()).then(|| Arc::new(visual));
        self.plots.insert(event_id, plot.clone());
        plot
    }
}

/// The joint names (tried in order, case-insensitively, as the client does) of an `fx_items.bone_id` and of a projectile's
/// `dest_bone_id`: the table of x2game `393E7C40` (`docs/research/re-skills.md` 4.4). Unknown ids and 0 are `bip01`, which every clip
/// moves to the ground under the unit; 14 to 18 are an empty name (the client takes `bip01 spine` for it); 19 is `Bip01 Hit`, the point a
/// blow lands on (the client picks the hit joint closest to the attacker; a rig without one gets its spine). A rig without the joint
/// has the model's origin (the caller's fallback).
pub fn bone_names(id: u8) -> &'static [&'static str] {
    match id {
        1 | 14..=18 => &["bip01 spine"],
        2 => &["bip01 head"],
        3 => &["bip01 neck"],
        4 => &["bip01 L Hand"],
        5 => &["bip01 R Hand"],
        6 => &["bip01 L Foot"],
        7 => &["bip01 R Foot"],
        8 => &["item_hand_l"],
        9 => &["item_hand_r"],
        10 => &["effect_1"],
        11 => &["bone_center"],
        12 => &["bone_string"],
        13 => &["bone_string_b8"],
        19 => &["Bip01 Hit", "bip01 spine1", "bip01 spine"],
        20 => &["attach_form"],
        21 => &["attach_launch"],
        22 => &["bip01 spine1"],
        23 => &["bip01 spine2"],
        24 => &["bip01 spine3"],
        _ => &["bip01"],
    }
}

/// The joint names of a projectile's `src_bone_id` (x2game `393E7D50`): the same as [`bone_names`], except that 14 to 18 and the unknown
/// ids are `bip01`, 25 to 28 are the `effect1` / `effect2` joints of the main / off hand weapon (the rigs have none: the caller falls
/// back to the hand that holds it) and 29 / 30 the sword hangers `item_side_l` / `item_side_r`.
///
/// `bip01` (0, the source of 23 of 30 beams and of nearly every flying shot) is the chest here, not the joint: a real capture of
/// Arc Lightning, Absorb Lifeforce and Flamebolt shows the beam's launch disc and the fireball leaving the caster at waist to
/// shoulder height (about 1.2 m), while every clip puts `Bip01` on the ground (`docs/research/re-skills.md` 4.4). The hit joint
/// stands for it (it follows the pose like the real one: the disc sits ahead of the torso in a lunge).
pub fn src_bone_names(id: u8) -> &'static [&'static str] {
    match id {
        1..=13 | 19..=24 => bone_names(id),
        25 | 27 => &["effect1"],
        26 | 28 => &["effect2"],
        29 => &["item_side_l"],
        30 => &["item_side_r"],
        _ => &["Bip01 Hit", "bip01 spine1", "bip01 spine", "bip01"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A song's particle item plays the sound of the instrument the caster holds (`$holdable_21` string, `_22` percussion, `_23` tube).
    #[test]
    fn a_song_item_plays_the_sound_of_the_instrument_held() {
        let event = |p: &str| Some(Arc::<str>::from(p));
        let item = FxItem {
            effect: "x.y.z".into(),
            chr: false,
            sound: Some(Sfx::Event(Arc::from("sounds/own"))),
            start: 0,
            end: 0,
            on_target: false,
            bone: 0,
            offset: Vec3::ZERO,
            axis: 0,
            scale: 1.0,
            song: [event("sounds/guitar"), None, event("sounds/flute")],
            flash: None,
        };
        assert_eq!(item.sound_for(Some(0)), Some(Sfx::Event(Arc::from("sounds/guitar"))));
        assert_eq!(item.sound_for(Some(2)), Some(Sfx::Event(Arc::from("sounds/flute"))));
        // no sound for the drum in the pack, or no instrument: the item's own sound
        assert_eq!(item.sound_for(Some(1)), Some(Sfx::Event(Arc::from("sounds/own"))));
        assert_eq!(item.sound_for(None), Some(Sfx::Event(Arc::from("sounds/own"))));
    }

    /// The bone table of x2game (`393E7C40` / `393E7D50`): the ids, the defaults and the source-only ids.
    #[test]
    fn the_bone_tables_follow_x2game() {
        let names = |id: u8| bone_names(id)[0];
        assert_eq!((names(0), names(200)), ("bip01", "bip01"));
        assert_eq!((names(1), names(3), names(4), names(5)), ("bip01 spine", "bip01 neck", "bip01 L Hand", "bip01 R Hand"));
        assert_eq!((names(8), names(9), names(10), names(13)), ("item_hand_l", "item_hand_r", "effect_1", "bone_string_b8"));
        assert_eq!((names(16), names(19), names(22), names(24)), ("bip01 spine", "Bip01 Hit", "bip01 spine1", "bip01 spine3"));
        // the source table adds the weapon and hanger joints and sends the unknown ids to the chest
        let src = |id: u8| src_bone_names(id)[0];
        assert_eq!((src(25), src(28), src(29), src(9)), ("effect1", "effect2", "item_side_l", "item_hand_r"));
        assert_eq!((src(0), src(15)), ("Bip01 Hit", "Bip01 Hit"));
    }

    fn book() -> Option<SkillBook> {
        std::path::Path::new(aa_anim::db::DEFAULT_DB).exists().then(|| SkillBook::open().expect("database"))
    }

    /// Shoot Arrow: cast clip, launch clip, arrow projectile on the left hand, hit effect on the target.
    #[test]
    fn shoot_arrow_links_clips_effects_and_the_arrow() {
        let Some(book) = book() else { return };
        let s = book.skill(16064).expect("Shoot Arrow");
        assert_eq!(s.start_anim.as_deref(), Some("bow_co_sk_cast"));
        assert_eq!(s.fire_anim.as_deref(), Some("bow_co_sk_launch"));
        assert_eq!((s.effect_delay_ms, s.plot), (358, false));
        let hit = s.fx.iter().find(|i| i.start == ev::HIT && i.on_target && !i.effect.is_empty()).expect("a hit particle effect");
        assert_eq!((hit.effect.as_str(), hit.bone), ("X2_EFFECTS.p_effect.st1000c", 19));
        assert!(s.fx.iter().any(|i| i.start == ev::HIT && i.on_target && i.flash.is_some()), "the material flash accompanies the particles");
        let arrow = s.projectile.as_ref().expect("the arrow");
        assert_eq!((arrow.id, arrow.dest_bone), (9, 19));
        let item = &arrow.items[0];
        assert_eq!((item.effect.as_str(), item.start, item.bone, item.on_target), ("X2_EFFECTS.p_effect.st1000b", ev::START, 8, false));
        // the SCSkillFired fire_anim_id 32 is the same clip
        assert_eq!(book.anim(32).as_deref(), Some("bow_co_sk_launch"));
        assert!(book.skill(16064).is_some(), "second lookup is cached");
    }

    /// The three slashes of 18132 are plot steps; each plays its own clip and effect group.
    #[test]
    fn plot_steps_carry_their_clips_and_effects() {
        let Some(book) = book() else { return };
        let s = book.skill(18132).expect("3-hit slash");
        assert!(s.plot);
        assert_eq!(s.fire_clip(false, false).map(|a| &**a), Some("onehand_co_sk_streakattack_1"));
        assert_eq!(s.fire_clip(true, false).map(|a| &**a), Some("twohand_co_sk_streakattack_1"));
        assert_eq!(s.fire_clip(false, true).map(|a| &**a), Some("twoswords_co_sk_streakattack_1"));
        // 12130 = stage 1, one hand; 12132 = stage 2; 12133 = stage 3
        let first = book.plot_event(12130).expect("stage 1");
        assert_eq!(first.anim.as_deref(), Some("onehand_co_sk_streakattack_1"));
        assert!(first.fx.iter().any(|i| i.start == ev::HIT && i.effect.ends_with("fight.hitup")), "{:?}", first.fx);
        assert_eq!(book.plot_event(12132).unwrap().anim.as_deref(), Some("onehand_co_sk_streakattack_2"));
        assert_eq!(book.plot_event(12133).unwrap().anim.as_deref(), Some("onehand_co_sk_streakattack_3"));
        assert!(book.plot_event(12144).is_none(), "the hit check step has nothing to show");
    }

    /// The Angry Rat bites with skill 2 and clip id 1 (`fist_co_attack_r`); the plain attack has a hit spark on the target.
    #[test]
    fn melee_and_monster_attacks() {
        let Some(book) = book() else { return };
        assert_eq!(book.anim(1).as_deref(), Some("fist_co_attack_r"));
        assert_eq!(book.anim(2).as_deref(), Some("fist_co_attack_r_2"));
        assert_eq!(book.anim(0), None);
        let s = book.skill(2).expect("melee attack");
        assert!(s.fx.iter().any(|i| i.start == ev::HIT && i.on_target && i.effect.starts_with("X2_EFFECTS.p_effect")));
        assert!(book.skill(0xFF_FFFF).is_none());
    }

    /// 450 plot steps play two anims, `all_co_sk_stop` first and then the clip: the step is the clip.
    #[test]
    fn a_step_that_stops_first_plays_the_clip_after_it() {
        let Some(book) = book() else { return };
        let step = book.plot_event(73).expect("step 73");
        assert_eq!(step.anims.iter().map(|a| &**a).collect::<Vec<_>>(), ["all_co_sk_stop", "all_co_sk_whirlwind_launch"]);
        assert_eq!(step.anim.as_deref(), Some("all_co_sk_whirlwind_launch"));
        // a step that only stops keeps the stop
        assert_eq!(book.plot_event(32).and_then(|p| p.anim.clone()).as_deref(), Some("all_co_sk_stop"));
        assert!(is_stop_clip("all_ac_stop") && !is_stop_clip("all_co_sk_stop_2"));
    }

    /// Two `anims` rows have a leading space; the `.cal` does not.
    #[test]
    fn clip_names_are_trimmed() {
        let Some(book) = book() else { return };
        assert_eq!(book.anim(345).as_deref(), Some("fist_ac_guide_01"));
        assert_eq!(book.anim(435).as_deref(), Some("fist_pos_stn_handsback_idle"));
    }

    /// The upper body names the client tries: `_mub` first, then the name as given.
    #[test]
    fn upper_body_names() {
        assert_eq!(upper_candidates("bow_co_sk_launch_ub").collect::<Vec<_>>(), ["bow_co_sk_launch_mub", "bow_co_sk_launch_ub"]);
        assert_eq!(upper_candidates("fist_ac_x").collect::<Vec<_>>(), ["fist_ac_x"]);
    }

    /// Shoot Arrow's groups hold a voice and a material flash that the client does not draw.
    #[test]
    fn counts_what_is_not_a_particle() {
        let Some(book) = book() else { return };
        let s = book.skill(16064).expect("Shoot Arrow");
        let all = s.extras.plus(s.projectile.as_ref().map(|p| p.extras).unwrap_or_default());
        assert!(!all.is_empty(), "{all:?}");
        assert_eq!(book.skill(0xFF_FFFF).map(|s| s.extras), None);
    }

    /// A step that only plays `all_co_sk_stop` ends the cast loop and has no clip of its own; one with a clip after it has both.
    #[test]
    fn a_stop_only_step_ends_the_loop() {
        let Some(book) = book() else { return };
        let stop = book.plot_event(32).expect("step 32");
        assert!(stop.stops());
        assert_eq!(stop.clip(), None);
        let both = book.plot_event(73).expect("step 73");
        assert!(both.stops());
        assert_eq!(both.clip().map(|a| &**a), Some("all_co_sk_whirlwind_launch"));
        let plain = book.plot_event(12130).expect("stage 1");
        assert!(!plain.stops());
        assert_eq!(plain.clip().map(|a| &**a), Some("onehand_co_sk_streakattack_1"));
    }

    /// The cast and channel poses (`anims.loop`) loop; the strikes do not.
    #[test]
    fn looping_clips_are_the_ones_the_data_says() {
        let Some(book) = book() else { return };
        for name in ["all_co_sk_spell_cast_d", "all_co_sk_buff_channel_force", "bow_co_sk_cast_3", "all_co_sk_spell_channel_sorb"] {
            assert!(book.clip_loops(name), "{name}");
        }
        for name in ["bow_co_sk_launch", "onehand_co_sk_streakattack_1", "all_co_sk_stop", "no_such_clip"] {
            assert!(!book.clip_loops(name), "{name}");
        }
        // Flame Bolt casts with a looping pose
        assert!(book.clip_loops(book.skill(10107).unwrap().start_anim.as_deref().unwrap()));
    }

    /// Dash (buff 2675): its group holds the held particle effect, the rush sound that lasts as long as the buff, and the end burst.
    #[test]
    fn dash_holds_a_particle_and_a_rush_sound_until_it_ends() {
        let Some(book) = book() else { return };
        let look = book.buff(2675).expect("Dash");
        // item 2584: an FxSound from the start to the end of the buff, with no particle effect
        let rush: Vec<_> = look.fx.iter().filter(|i| i.effect.is_empty()).collect();
        assert_eq!(rush.len(), 1, "{:?}", look.fx);
        assert_eq!((rush[0].start, rush[0].end), (ev::START, ev::END));
        assert!(matches!(&rush[0].sound, Some(Sfx::Event(p)) if p.contains("skill_rush_speed")));
        // item 1450 is held as well (its own sound is the launch), item 2586 plays when the buff goes
        assert!(look.fx.iter().any(|i| i.effect.ends_with("system_skill.speed_full_pc") && i.end == ev::END));
        assert!(look.fx.iter().any(|i| i.effect.ends_with("speed_full_pc_end") && i.start == ev::END));
    }

    /// Invincibility has its aura as the buff of its channel, not as a skill effect.
    #[test]
    fn channel_buffs_are_the_skills_buffs() {
        let Some(book) = book() else { return };
        assert!(book.skill_buffs(10372).contains(&131), "{:?}", book.skill_buffs(10372));
        assert!(book.skill_buffs(1).is_empty());
    }

    /// `proj_physic_id`: Arc Lightning's projectile is a beam, Meteor Strike's is an effect area, Shoot Arrow's flies.
    #[test]
    fn projectile_physics() {
        let Some(book) = book() else { return };
        let beam = book.projectile(66).expect("arc lightning");
        assert!(beam.is_beam() && !beam.is_zone());
        let meteor = book.projectile(8).expect("meteor");
        assert!(meteor.is_zone() && !meteor.is_beam());
        let arrow = book.skill(16064).and_then(|v| v.projectile.clone()).expect("arrow");
        assert!(!arrow.is_beam() && !arrow.is_zone());
        // a plot step carries the speed of its projectile (Missile Rain: 25 m/s)
        assert_eq!(book.plot_event(212).map(|p| p.projectile_speed), Some(25.0));
    }

    /// Uproot (13789) holds the off hand shovel, watering (13625) the main hand pot: `skills.*_tool_id` through
    /// `item_tools` -> `item_assets`.
    #[test]
    fn tool_skills_resolve_their_hand_models() {
        let Some(book) = book() else { return };
        let uproot = book.skill(13789).expect("uproot");
        assert_eq!(uproot.mainhand_tool(), None);
        let shovel = uproot.offhand_tool().expect("the shovel");
        assert!(shovel.path.ends_with("shovel_general.cgf"), "{}", shovel.path);
        assert_eq!(shovel.offset, [-0.01, 0.16, 0.0]);
        assert_eq!(shovel.rotation, [10.0, 88.88, 0.0]);
        let water = book.skill(13625).expect("watering");
        let pot = water.mainhand_tool().expect("the pot");
        assert!(pot.path.ends_with("ndeco_quest_pot_a.cgf"), "{}", pot.path);
        assert_eq!(water.offhand_tool(), None);
        // a skill without tools holds none
        assert_eq!(book.skill(16064).unwrap().tools, [None, None]);
    }

    /// A dash or a leap has nothing but its controller's clip.
    #[test]
    fn controller_clips() {
        let Some(book) = book() else { return };
        let dash = book.skill(11918).expect("dash");
        assert_eq!((dash.start_anim.as_deref(), dash.fire_anim.as_deref()), (None, None));
        assert_eq!(dash.controller_anim.as_deref(), Some("all_co_sk_dashattack"));
        assert_eq!(book.skill(12049).unwrap().controller_anim.as_deref(), Some("all_co_sk_backstep"));
        assert_eq!(book.skill(16064).unwrap().controller_anim, None);
    }

    /// A song has a clip per instrument family (string, percussion, tube); the other skills have none.
    #[test]
    fn instrument_clips() {
        let Some(book) = book() else { return };
        let song = book.skill(11377).expect("a song");
        for kind in 0..3 {
            assert!(song.instrument_start_clip(Some(kind)).is_some(), "start {kind}");
            assert!(song.instrument_fire_clip(Some(kind)).is_some(), "fire {kind}");
        }
        assert_eq!(song.instrument_start_clip(None), None);
        let other = book.skill(16064).unwrap();
        assert_eq!(other.instrument_start_clip(Some(0)), None);
        assert_eq!(other.instrument_fire_clip(Some(0)), None);
    }

    /// Summon Crows' group holds a character effect: the crow model, on the caster's bone 2, from the start to the end.
    #[test]
    fn a_character_effect_is_an_item() {
        let Some(book) = book() else { return };
        let items = book.fx_group(401);
        let crow = items.iter().find(|i| i.chr).expect("the crow model");
        assert_eq!((crow.effect.as_str(), crow.start, crow.end, crow.bone), ("game/objects/effects/crow/crow.chr", ev::START, ev::END, 2));
        assert!(items.iter().any(|i| !i.chr && !i.effect.is_empty()), "the particle effect stays");
    }

    /// A beam lives as long as the `ProjectileAnim` of its plot step says (ms): Absorb Lifeforce 6.5 s, Crippling Mire 4.5 s, Lasso 0.7 s;
    /// Arc Lightning's and Mana Force's say 0 (their effects play out).
    #[test]
    fn a_beam_lives_as_long_as_its_step_says() {
        let Some(book) = book() else { return };
        let ms = |event: u32| book.plot_event(event).map(|p| (p.projectile.as_ref().is_some_and(|p| p.is_beam()), p.projectile_ms));
        assert_eq!(ms(2499), Some((true, 6500)));
        assert_eq!(ms(2479), Some((true, 4500)));
        assert_eq!(ms(2657), Some((true, 700)));
        assert_eq!(ms(2325), Some((true, 0)));
        assert_eq!(ms(5088), Some((true, 0)));
    }

    /// Only Crippling Mire's ring is a flat burst; the other `_form_end` effects and the rest of its group are not.
    #[test]
    fn the_ring_of_crippling_mire_is_flat() {
        let Some(book) = book() else { return };
        let items = book.fx_group(229);
        let flat: Vec<_> = items.iter().filter(|i| i.flat()).collect();
        assert_eq!(flat.len(), 1, "{items:?}");
        assert!(flat[0].effect.ends_with("psychokinesis_launch_form_end"));
        assert!(book.fx_group(229).iter().any(|i| !i.flat()));
    }

    /// A skill whose effects grow with its level has a group per level range: the caster's level picks one.
    #[test]
    fn the_level_picks_the_visual_group() {
        let Some(book) = book() else { return };
        let s = book.skill(10159).expect("a tree skill");
        assert_eq!(s.levels.iter().map(|(l, _)| *l).collect::<Vec<_>>(), [1, 6, 11]);
        let (low, mid, high) = (s.look(1), s.look(7), s.look(30));
        assert!(!low.fx.is_empty() && !mid.fx.is_empty() && !high.fx.is_empty());
        assert_eq!(s.look(1).fx, s.look(5).fx, "levels 1 to 5 share the first group");
        assert_ne!(low.fx, mid.fx);
        assert_eq!(s.look(11).fx, s.look(60).fx);
        // a skill without levels keeps its own group
        let arrow = book.skill(16064).unwrap();
        assert_eq!(arrow.look(1).fx, arrow.fx);
    }

    /// Camera shakes of an effect group: when they start and on whom, and how long.
    #[test]
    fn camera_shakes() {
        let Some(book) = book() else { return };
        let s = book.skill(18384).expect("a heavy blow");
        let shake = s.shakes.first().expect("a shake");
        assert_eq!(shake.start, ev::HIT);
        assert!(!shake.on_target);
        assert!((shake.shake.duration - 0.1).abs() < 1e-6, "{shake:?}");
        assert!(book.skill(16064).unwrap().shakes.is_empty());
    }

    /// Moving always ends a cast time; the flags add the channels that moving ends (Invincibility, Telekinesis) and turning.
    #[test]
    fn cancel_flags() {
        let Some(book) = book() else { return };
        let rules = |id| book.skill(id).expect("skill").cancel;
        assert_eq!(rules(10372), CancelRules { channel_on_move: true, on_turn: false, not_swim: false });
        assert_eq!(rules(11504), CancelRules { channel_on_move: true, on_turn: false, not_swim: false });
        assert!(rules(26860).on_turn);
        // Return and the combat spells have no flag: only their cast time is cancelled by moving
        for id in [10594, 10752, 10546, 16064, 18132, 21959] {
            assert_eq!(rules(id), CancelRules::default(), "{id}");
        }
    }
}
