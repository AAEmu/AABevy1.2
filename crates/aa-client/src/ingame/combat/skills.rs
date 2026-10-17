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
//!   (special effect 34 `Anim`), effect group (35 / 36) and projectile (37 / 38).
//!
//! Everything is read on first use and cached by id, so the frame never touches the database after a skill was seen once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags, params};

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
}

/// What an effect group holds besides particle effects. The client draws none of it (sounds, voices, camera shakes, material
/// flashes, models and ropes); counted so that `examples/skill_coverage.rs` can tell how many skills lose something.
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
    /// `FxChr`, `FxRope`, `FxCamFov`, `FxMotionBlur`, ...
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

/// One particle effect of an effect group.
#[derive(Clone, Debug, PartialEq)]
pub struct FxItem {
    /// `library.group.name` for aa-fx (`X2_EFFECTS.p_effect.p0000`).
    pub effect: String,
    /// [`ev`] it starts at and ends at (an effect that has played out by itself needs no end).
    pub start: u8,
    pub end: u8,
    /// Plays on the target (`fx_location_id` 2) and not on the caster (1).
    pub on_target: bool,
    /// `fx_items.bone_id`, see [`bone_names`].
    pub bone: u8,
    /// Offset from the bone in cry axes (m).
    pub offset: Vec3,
    /// `fx_items.offset_axis_id`: 4 turns a hit effect towards the attacker.
    pub axis: u8,
    /// `fx_particles.scale`.
    pub scale: f32,
}

/// A thing that flies from the caster to the target (`projectiles`).
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub id: u32,
    /// What it looks like in flight (an effect group: the items start at [`ev::START`] and ride the projectile).
    pub items: Arc<Vec<FxItem>>,
    pub extras: Extras,
    /// Where it starts and ends (`fx_items.bone_id` numbering; 0 = the caster's launch bone from the effect item).
    pub src_bone: u8,
    pub dest_bone: u8,
}

/// The looks of one skill.
#[derive(Clone, Debug, Default)]
pub struct SkillVisual {
    /// Clip names of the caster's `.cal`.
    pub start_anim: Option<Arc<str>>,
    pub fire_anim: Option<Arc<str>>,
    pub twohand_fire_anim: Option<Arc<str>>,
    pub dual_fire_anim: Option<Arc<str>>,
    pub fx: Arc<Vec<FxItem>>,
    /// Not drawn by the client; counted by `examples/skill_coverage.rs`.
    #[allow(dead_code)]
    pub extras: Extras,
    pub projectile: Option<Projectile>,
    /// `skills.channeling_anim_id` (not played yet; checked by `examples/skill_coverage.rs`).
    #[allow(dead_code)]
    pub channeling_anim: Option<Arc<str>>,
    /// `skills.effect_delay`: how long after the fire the shot leaves (the launch moment of the clip), ms.
    pub effect_delay_ms: u32,
    /// The skill is a plot: its steps carry the clips and effects ([`PlotVisual`]).
    pub plot: bool,
}

impl SkillVisual {
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
    pub extras: Extras,
    pub projectile: Option<Projectile>,
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
    groups: HashMap<u32, Arc<Vec<FxItem>>>,
    extras: HashMap<u32, Extras>,
    projectiles: HashMap<u32, Option<Projectile>>,
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
            groups: HashMap::new(),
            extras: HashMap::new(),
            projectiles: HashMap::new(),
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

    /// The particle effects of an effect group (sounds, voices, camera shakes, materials and models are left out).
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
                        i.offset_axis_id, p.scale \
                 FROM fx_group_fx_items g JOIN fx_items i ON i.id = g.fx_item_id \
                 LEFT JOIN fx_particles p ON p.id = i.fx_detail_id \
                 WHERE g.fx_group_id = ?1 AND i.fx_detail_type = 'FxParticle' ORDER BY g.rowid",
            )
            .and_then(|mut q| {
                q.query_map(params![id], |r| {
                    let effect = r.get::<_, Option<String>>(0)?.unwrap_or_default().trim().to_string();
                    let scale = num(r, 9)?;
                    Ok(FxItem {
                        effect,
                        start: int(r, 1)? as u8,
                        end: int(r, 2)? as u8,
                        on_target: int(r, 3)? == 2,
                        bone: int(r, 4)? as u8,
                        offset: Vec3::new(num(r, 5)?, num(r, 6)?, num(r, 7)?),
                        axis: int(r, 8)? as u8,
                        scale: if scale > 0.0 { scale } else { 1.0 },
                    })
                })
                .map(|rows| rows.flatten().filter(|i| i.effect.contains('.')).collect::<Vec<_>>())
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
            .query_row("SELECT fx_group_id, src_bone_id, dest_bone_id FROM projectiles WHERE id = ?1", params![id], |r| {
                Ok((int(r, 0)? as u32, int(r, 1)? as u8, int(r, 2)? as u8))
            })
            .ok();
        let projectile = row.map(|(group, src_bone, dest_bone)| Projectile { id, items: self.group(group), extras: self.extras_of(group), src_bone, dest_bone });
        self.projectiles.insert(id, projectile.clone());
        projectile
    }

    fn skill(&mut self, id: u32) -> Option<Arc<SkillVisual>> {
        if let Some(s) = self.skills.get(&id) {
            return s.clone();
        }
        let row = self
            .conn
            .query_row(
                "SELECT start_anim_id, fire_anim_id, twohand_fire_anim_id, dual_wield_fire_anim_id, fx_group_id, projectile_id, effect_delay, plot_id, channeling_anim_id \
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
                    ))
                },
            )
            .ok();
        let visual = row.map(|(anims, group, projectile, effect_delay_ms, plot, channeling)| {
            let [start, fire, twohand, dual] = anims;
            Arc::new(SkillVisual {
                start_anim: self.anim(start),
                fire_anim: self.anim(fire),
                twohand_fire_anim: self.anim(twohand),
                dual_fire_anim: self.anim(dual),
                fx: self.group(group),
                extras: self.extras_of(group),
                projectile: self.projectile(projectile),
                channeling_anim: self.anim(channeling),
                effect_delay_ms,
                plot,
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
        let effects: Vec<(i64, u32)> = self
            .conn
            .prepare(
                "SELECT s.special_effect_type_id, s.value1 FROM plot_effects e JOIN special_effects s ON s.id = e.actual_id \
                 WHERE e.event_id = ?1 AND e.actual_type = 'SpecialEffect' AND s.special_effect_type_id IN (34, 35, 36, 37, 38) ORDER BY e.position, e.id",
            )
            .and_then(|mut q| q.query_map(params![event_id], |r| Ok((int(r, 0)?, int(r, 1)? as u32))).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        let mut visual = PlotVisual::default();
        let mut fx: Vec<FxItem> = Vec::new();
        for (kind, value) in effects {
            match kind {
                34 => visual.anims.extend(self.anim(value)),
                35 | 36 => {
                    fx.extend(self.group(value).iter().cloned());
                    visual.extras = visual.extras.plus(self.extras_of(value));
                }
                _ => {
                    let first = visual.projectile.is_none();
                    visual.projectile = visual.projectile.take().or_else(|| self.projectile(value));
                    if first && let Some(p) = &visual.projectile {
                        visual.extras = visual.extras.plus(p.extras);
                    }
                }
            }
        }
        // 450 steps play two anims: `all_co_sk_stop` (end the clip before) and then the clip itself; the step is the latter
        visual.anim = visual.anims.iter().rev().find(|a| !is_stop_clip(a)).or(visual.anims.first()).cloned();
        visual.fx = Arc::new(fx);
        let plot = (visual.anim.is_some() || !visual.fx.is_empty() || visual.projectile.is_some()).then(|| Arc::new(visual));
        self.plots.insert(event_id, plot.clone());
        plot
    }
}

/// The bone names (of the player and creature rigs, tried in order) of an `fx_items.bone_id`. The ids 6 / 7 (feet), 8 / 9 (the
/// hands: the weapon bones) and 19 (`Bip01 Hit`, where a blow lands) are certain from the anim events of the clips; 1 to 5 are
/// the body, head and pelvis by their effects. 0 is the model's origin (the feet).
pub fn bone_names(id: u8) -> &'static [&'static str] {
    match id {
        1 => &["Bip01 Spine1", "Bip01 Spine"],
        2 => &["Bip01 Head", "Bip01 Spine2"],
        3 => &["Bip01 Spine2", "Bip01 Spine1"],
        4 => &["Bip01 Pelvis", "Bip01 Spine"],
        5 => &["item_hand_r", "Bip01 R Hand"],
        6 => &["Bip01 L Foot"],
        7 => &["Bip01 R Foot"],
        8 => &["item_hand_l", "Bip01 L Hand", "Bip01 L Forearm"],
        9 => &["item_hand_r", "Bip01 R Hand", "Bip01 R Forearm"],
        19 => &["Bip01 Hit", "Bip01 Spine1", "Bip01 Spine"],
        24 => &["Bip01 Spine1"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let hit = s.fx.iter().find(|i| i.start == ev::HIT && i.on_target).expect("a hit effect");
        assert_eq!((hit.effect.as_str(), hit.bone), ("X2_EFFECTS.p_effect.st1000c", 19));
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
}
