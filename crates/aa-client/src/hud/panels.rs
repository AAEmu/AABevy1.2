//! The lists the scripts read besides the player's numbers: the action bars, the bag, the buffs on
//! the player and the target, and what the player targets. [`Panels`] is computed from aa-net's
//! resources every frame, compared with what the Lua state has seen, and pushed as the tables
//! `HUD.actions`, `HUD.bag`, `HUD.buffs`, `HUD.target` only when it changed (`ingame.lua` answers
//! `AAUI.SlotContent`, `X2Unit:UnitBuff`, `X2Unit:UnitName("target")` ... from them).
//!
//! Times that count down on their own (buffs, cooldowns) are pushed as absolute end times in the
//! HUD's clock (`HUD.now_ms()`: milliseconds since the plugin started), so a running countdown is not
//! a change.

use web_time::Instant;

use aa_net::buff::BuffKind;
use aa_net::items::ActionKind;
use aa_net::model::UnitKind;
use aa_net::state::{ActionBar, Buffs, Inventory, Player, Skills, Target, Units};
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::HudKeyboardFocus;
use super::alerts::UseFailed;
use super::api::HudBridge;
use super::data::{GameData, Snapshot};
use super::usable::{Env, InFlight};
use crate::ingame::ground_target::SkillPressed;

/// One action bar slot (or bag slot).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotSnap {
    /// `"skill"` or `"item"`.
    pub kind: &'static str,
    /// Skill id, or item template id.
    pub id: u32,
    /// Real item instance identity, never cast through a Lua number.
    pub uid: u64,
    pub name: String,
    pub icon: String,
    /// Items: how many (the bag total for a template slot).
    pub count: i64,
    /// Whether the count is shown on the icon (stackable items).
    pub stack: bool,
    pub grade: u8,
    /// Skills: when the cooldown ends (HUD clock, ms) and how long it is.
    pub cooldown: Option<(u64, u32)>,
    /// Items: description and level requirement (tooltips).
    pub desc: String,
    pub level: i64,
    /// Items: `item_groups.id` (bag tabs).
    pub group: u32,
    pub category_order: i64,
    /// Worn items and bag items: the instance's durability (`None` for an item that does not wear out or is not
    /// an instance), temper values and item flags (the tooltip's durability, stats and "Untradeable").
    pub dur: Option<u8>,
    pub temper: (u16, u16),
    pub flags: Option<u8>,
    /// Gems, enchant, dye, look and lock of the instance (the tooltip's sockets and bonuses).
    pub extra: super::tips::ItemExtra,
    /// Action bar slots that cannot be used now (the icon goes dark, the hotkey red): the skill result that refuses them
    /// (`usable::code`), `None` when the slot is lit.
    pub blocked: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuffSnap {
    pub id: u32,
    pub descriptor: bool,
    pub kind: i64,
    pub caster_character_id: u32,
    pub duration_ms: u32,
    /// Received instance identity; the original click callback still passes its one-based ordinal.
    pub instance_index: u32,
    pub name: String,
    pub icon: String,
    pub desc: String,
    pub stacks: Option<f64>,
    /// The ability level the buff was applied with (its description's numbers scale with it).
    pub ab_level: i16,
    /// When it ends (HUD clock, ms); `None` for a permanent buff.
    pub ends_ms: Option<u64>,
}

/// The buffs of a unit, good and bad.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuffLists {
    pub good: Vec<BuffSnap>,
    pub bad: Vec<BuffSnap>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetSnap {
    pub obj_id: u32,
    /// Original GetNpcTypeIndex accepts raw Npc only, excluding mates and other NPC facades.
    pub npc_type: Option<u32>,
    pub character_id: u32,
    pub name: String,
    pub level: u8,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,
    /// Honor gained in combat and hostile faction kills (unit tooltip `|p10;`, `|p11;`).
    pub pvp_honor: i32,
    pub pvp_kills: i32,
    /// `GetUnitTypeString`: `"character"` / `"npc"`, and the detail (`"slave"`, `"housing"`, ...).
    pub unit_type: &'static str,
    pub detail: &'static str,
    pub faction_name: String,
    /// `"friendly"`, `"neutral"` or `"hostile"` towards the player.
    pub relation: &'static str,
    /// `CanAutoAttackTargetUnit`'s attackable bit (`FUN_39838880` 0x100): every unit is attackable
    /// except a friendly one and one refused by the reasons below; the type-3 `+0x40` and
    /// the force-attack/bloodlust flags are not recovered.
    pub attackable: bool,
    /// `FUN_39838880` reasons 5/6: the level-10 protection (`skill_protected_level`) applies to this
    /// hostile or neutral/friendly target, because both owning-character ids (`+0x24`) are valid and the
    /// attacker or the target is level 10 or below in a zone protected for its faction.
    pub protected_level: bool,
    /// `FUN_39838880` reasons 3/4 (`FUN_39837b40` / `FUN_398377c0`): on the neutral/friendly branch the
    /// same owning character or the same party is refused as `INVALID_TARGET` before reasons 5/6.
    pub invalid_attack: bool,
    /// The unit's faction id, for the `PROTECTED_FACTION` refusal detail.
    pub faction_id: i64,
    pub grade: &'static str,
    pub race: u8,
    pub gender: u8,
    /// The active abilities (1..=10) of a character target, as its unit state carried them (the target frame's job icon);
    /// empty for NPCs and while the server has not said.
    pub abilities: Vec<u8>,
    pub buffs: BuffLists,
}

/// Everything in this module, as of this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panels {
    /// Action bar by slot index (0 based); `None` = empty. Empty when the server has not sent the
    /// bars yet.
    pub actions: Vec<Option<SlotSnap>>,
    pub actions_known: bool,
    pub bag: Vec<Option<SlotSnap>>,
    pub bag_slots: u32,
    pub bank: Vec<Option<SlotSnap>>,
    pub bank_slots: u32,
    /// The open coffer's slots (`coffer.lua`).
    pub coffer: Vec<Option<SlotSnap>>,
    /// Worn items by aa-net slot index (`model::slot`, the scripts' `ES_*` minus one); body parts
    /// are not items to the window.
    pub equipment: Vec<Option<SlotSnap>>,
    pub buffs: BuffLists,
    pub target: Option<TargetSnap>,
}

/// Milliseconds on the HUD clock.
pub fn now_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

pub(in crate::hud) fn item_slot(game: Option<&GameData>, template: u32, count: i64, grade: u8) -> SlotSnap {
    let info = game.and_then(|g| g.items.get(&template));
    SlotSnap {
        kind: "item",
        id: template,
        uid: 0,
        name: info.map(|i| i.named.name.clone()).unwrap_or_default(),
        icon: info.map(|i| i.named.icon.clone()).unwrap_or_default(),
        count,
        stack: info.is_some_and(|i| i.max_stack > 1),
        grade: super::store::ui_grade(grade),
        cooldown: None,
        desc: info.map(|i| i.desc.clone()).unwrap_or_default(),
        level: info.map_or(0, |i| i.level),
        group: info.map_or(0, |i| i.group),
        category_order: info.map_or(0, |i| i.category_order),
        dur: None,
        temper: (0, 0),
        flags: None,
        extra: Default::default(),
        blocked: None,
    }
}

impl SlotSnap {
    /// The slot of an item instance: what its tooltip reads of it besides the template.
    pub(in crate::hud) fn with_instance(mut self, it: &aa_net::model::Item) -> SlotSnap {
        self.uid = it.id;
        if let aa_net::model::ItemDetail::Equip(d) = &it.detail {
            self.dur = Some(d.durability);
            self.temper = (d.temper_physical, d.temper_magical);
        }
        self.flags = Some(it.flags);
        self.extra = super::tips::ItemExtra::of_item(it);
        self
    }
}

fn cooldown_slot(cooldown: Option<(u32, u32)>, start: Instant) -> Option<(u64, u32)> {
    // quantized to a quarter second: the end time of a running cooldown must not jitter
    let (left, total) = cooldown?;
    if left == 0 {
        return None;
    }
    let end = now_ms(start) + u64::from(left);
    Some(((end + 125) / 250 * 250, total))
}

fn skill_slot_of(game: Option<&GameData>, skill: u32, cooldown: Option<(u64, u32)>) -> SlotSnap {
    let info = game.and_then(|g| g.skills.get(&skill));
    SlotSnap {
        kind: "skill",
        id: skill,
        uid: 0,
        name: info.map(|i| i.named.name.clone()).unwrap_or_default(),
        icon: info.map(|i| i.named.icon.clone()).unwrap_or_default(),
        count: 0,
        stack: false,
        grade: 0,
        cooldown,
        desc: String::new(),
        level: 0,
        group: 0,
        category_order: 0,
        dur: None,
        temper: (0, 0),
        flags: None,
        extra: Default::default(),
        blocked: None,
    }
}

/// An action-bar skill slot: a linked skill (`FUN_393925c0`) shows the converted skill's cooldown, and with no matching
/// item it is dark (`FUN_3943fa10` sets `skillUsable=0`, which `refresh` applies through [`Env::slot_blocked`]). The name
/// and icon stay the pressed skill's.
fn action_skill_slot(
    game: Option<&GameData>,
    skills: Option<&Skills>,
    inventory: Option<&Inventory>,
    skill: u32,
    start: Instant,
) -> SlotSnap {
    let effective = match crate::ingame::linked_skill::resolve_with(skill, inventory, game) {
        crate::ingame::linked_skill::LinkedUse::Converted { skill, .. } => skill,
        _ => skill,
    };
    let cooldown = game.and_then(|g| {
        let skills = skills?;
        cooldown_slot(super::cooldown::display_at(g, skills, effective, Instant::now()), start)
    });
    skill_slot_of(game, skill, cooldown)
}

pub(in crate::hud) fn skill_slot(game: Option<&GameData>, skills: Option<&Skills>, skill: u32, start: Instant) -> SlotSnap {
    // the skill's own cooldown, or the global cooldown the last cast started (every skill that does not
    // ignore it waits for it), whichever ends later
    let cooldown = game.and_then(|g| {
        let skills = skills?;
        cooldown_slot(
            super::cooldown::display_at(g, skills, skill, Instant::now()),
            start,
        )
    });
    skill_slot_of(game, skill, cooldown)
}

/// A slot of the driven slave's bar: its own timer and global cooldown come from the slave's
/// `Mount` casts, not the player's `Skills` (the server sends no cooldown packet for a slave).
pub(in crate::hud) fn slave_skill_slot(
    game: Option<&GameData>,
    skills: Option<&Skills>,
    caster: u32,
    skill: u32,
    start: Instant,
) -> SlotSnap {
    let cooldown = game.and_then(|g| {
        let skills = skills?;
        cooldown_slot(
            super::cooldown::mount_display_at(g, skills, caster, skill, Instant::now()),
            start,
        )
    });
    skill_slot_of(game, skill, cooldown)
}

pub(super) fn buff_lists(game: Option<&GameData>, entries: &[aa_net::state::BuffEntry], start: Instant) -> BuffLists {
    let mut lists = BuffLists::default();
    for e in entries {
        let b = &e.buff;
        let info = game.and_then(|g| g.buffs.get(&b.buff_id));
        let good = match b.kind {
            BuffKind::Good => true,
            BuffKind::Bad => false,
            BuffKind::Hidden => continue,
            BuffKind::Unknown => match info.map(|i| i.kind) {
                Some(2) => false,
                Some(3) => continue,
                _ => true,
            },
        };
        let duration_ms = b.duration_ms as u32;
        let ends_ms = (duration_ms != 0).then(|| {
            let received = e.received.saturating_duration_since(start).as_millis() as u64;
            received + u64::from(duration_ms.saturating_sub(b.elapsed_ms))
        });
        let snap = BuffSnap {
            id: b.buff_id,
            descriptor: info.is_some(),
            kind: info.map_or(0, |info| info.kind),
            caster_character_id: b.caster_char_id,
            duration_ms,
            instance_index: b.index,
            name: info.map(|i| i.named.name.clone()).unwrap_or_default(),
            icon: info.map(|i| i.named.icon.clone()).unwrap_or_default(),
            desc: info.map(|i| i.desc.clone()).unwrap_or_default(),
            stacks: info.and_then(|info| super::buffs::stack_count(info.stack_rule, b.stacks, b.charged)),
            ab_level: b.ability_level,
            ends_ms,
        };
        if good { lists.good.push(snap) } else { lists.bad.push(snap) }
    }
    lists
}

/// The original manager folds repeats in place. Keep raw lane icons only while its history is unknown.
pub(super) fn unit_buff_lists(game: Option<&GameData>, buffs: &Buffs, obj_id: u32, start: Instant) -> BuffLists {
    buff_lists(game, buffs.native.of(obj_id).unwrap_or_else(|| buffs.of(obj_id)), start)
}

fn unit_race_gender(race_gender: u8) -> (u8, u8) {
    (race_gender & 0x0f, race_gender >> 4)
}

/// Native `+0x24` (`FUN_39356b60`, `x2game.dll.c:690617`): the owning character id. A character owns
/// itself; a slave its summoner, a mate its owner, a house or shipyard the received owner. NPCs (native
/// kind 1), transfers and ownerless units return `None`. The reason 2-6 block of `FUN_39838880` only
/// runs when both units have a valid id, so the level protection is PvP-only.
fn owning_character(world: &World, unit: &aa_net::model::Unit) -> Option<u32> {
    match unit.kind {
        UnitKind::Character { char_id } => (char_id != 0).then_some(char_id),
        UnitKind::Slave { summoner_id, .. } => (summoner_id != 0).then_some(summoner_id),
        UnitKind::Mate { owner_id, .. } => (owner_id != 0).then_some(owner_id),
        UnitKind::Housing { tl_id, .. } => world
            .get_resource::<aa_net::state::Houses>()?
            .by_tl_id
            .get(&tl_id)
            .map(|h| h.snapshot.owner_id)
            .filter(|id| *id != 0),
        UnitKind::Shipyard { id, .. } => world
            .get_resource::<aa_net::state::Shipyards>()?
            .by_id
            .get(&id)
            .map(|s| s.kind2)
            .filter(|id| *id != 0),
        UnitKind::Npc { .. } | UnitKind::Transfer { .. } => None,
    }
}

/// `FUN_39837b90` (`x2game.dll.c:1832566`): whether a zone value protects a unit of `faction`.
/// The value is the `zones.faction_id` the HUD keeps. `DAT_39acb68c` = 5 protects every owned unit;
/// `DAT_39acb680` = 4 protects a unit friendly to `DAT_39acb684` = 148 or `DAT_39acb688` = 149; any
/// other value is a faction id the unit must be friendly to (`+0x4` relation state 3).
fn zone_protects(game: Option<&GameData>, zone_faction: u32, faction: u32) -> bool {
    match zone_faction {
        0 => false,
        4 => game.is_some_and(|g| g.relation(faction, 148) == "friendly" || g.relation(faction, 149) == "friendly"),
        5 => true,
        zone => game.is_some_and(|g| g.relation(faction, zone) == "friendly"),
    }
}

/// `FUN_39027eb0` (`x2game.dll.c:39098`): the zone's protection value. A group with a static
/// `conflict_zones` row (`FUN_397f2080`) is a conflict zone: `closed` means 0, and otherwise the row's
/// `peace_protected_faction_id` applies only while the received runtime state is 7 (peace); any other
/// state or no state means 0 (`FUN_390298d0`, `40895`). A group without a row is a home zone and uses
/// `zones.faction_id`; the timed controller override that also returns 5 there is not modeled
/// (capture-pending).
fn zone_protection_value(world: &World, game: Option<&GameData>, me: &Snapshot) -> u32 {
    if let Some((closed, peace_faction)) = game.and_then(|g| g.conflict_zones.get(&me.zone_group)).copied() {
        if closed {
            return 0;
        }
        let peace = world
            .get_resource::<aa_net::state::ConflictZones>()
            .and_then(|z| z.get(me.zone_group as u16))
            .is_some_and(|s| s.state == 7);
        return if peace { peace_faction } else { 0 };
    }
    me.zone_faction
}

/// Compute the panels from aa-net's state. `own_faction` is the player's faction id, `me` and `pos` the player's numbers and
/// game position (which action bar slots are dark depends on them); also returns the game position of the target.
pub fn refresh(world: &World, game: Option<&GameData>, own_faction: u32, own_obj_id: u32, start: Instant, me: &Snapshot, pos: Vec3) -> (Panels, Option<Vec3>, f32) {
    let mut p = Panels::default();
    let skills = world.get_resource::<Skills>();
    let inventory = world.get_resource::<Inventory>();
    if let Some(bar) = world.get_resource::<ActionBar>() {
        p.actions_known = !bar.slots.is_empty();
        p.actions = bar
            .slots
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let slot = *slot;
                if slot.is_empty() {
                    return None;
                }
                match slot.kind {
                    ActionKind::Spell | ActionKind::RidePetSpell => {
                        Some(action_skill_slot(game, skills, inventory, slot.id as u32, start))
                    }
                    ActionKind::ItemType | ActionKind::ItemId => {
                        let inv = inventory?;
                        let (template, grade) = match slot.item_template_id() {
                            Some(t) => (t, inv.find_template(t).map_or_else(|| game.and_then(|g| g.items.get(&t)).map_or(0, |i| i.fixed_grade.max(0) as u8), |(_, it)| it.grade)),
                            None => {
                                let (_, _, it) = inv.find_by_id(slot.id)?;
                                (it.template_id, it.grade)
                            }
                        };
                        Some(item_slot(game, template, bar.item_count(i, inv).unwrap_or(0), grade))
                    }
                    _ => None,
                }
            })
            .collect();
    }
    if let Some(inv) = inventory {
        p.bag_slots = inv.bag_slots;
        p.bank_slots = inv.bank_slots;
        p.bank = inv.bank.iter().map(|it| it.as_ref().map(|it| item_slot(game, it.template_id, i64::from(it.count), it.grade).with_instance(it))).collect();
        p.coffer = inv.coffer.slots.iter().map(|it| it.as_ref().map(|it| item_slot(game, it.template_id, i64::from(it.count), it.grade).with_instance(it))).collect();
        p.bag = inv.bag.iter().map(|it| it.as_ref().map(|it| item_slot(game, it.template_id, i64::from(it.count), it.grade).with_instance(it))).collect();
        p.equipment = inv
            .equipment
            .slots
            .iter()
            .enumerate()
            .map(|(i, it)| it.as_ref().filter(|_| !aa_net::model::slot::is_body_part(i)).map(|it| item_slot(game, it.template_id, 1, it.grade).with_instance(it)))
            .collect();
    }
    if let Some(buffs) = world.get_resource::<Buffs>() {
        p.buffs = unit_buff_lists(game, buffs, own_obj_id, start);
    }
    let mut target_pos = None;
    let mut target_radius = 0.0;
    let target_obj_id = world.get_resource::<Target>().and_then(|t| t.obj_id);
    // the local player is not in `Units` (aa-net keeps it in `Player`): targeting self resolves its
    // own unit here, with the HUD's current points.
    let own_target = (target_obj_id == Some(own_obj_id))
        .then(|| {
            let spawn = world.get_resource::<Player>()?.spawn.as_ref()?;
            let mut unit = spawn.unit.clone();
            unit.hp = me.hp;
            unit.max_hp = me.max_hp;
            unit.mp = me.mp;
            unit.max_mp = me.max_mp;
            unit.level = me.level;
            unit.dead = me.dead;
            Some(unit)
        })
        .flatten();
    let picked = world
        .get_resource::<Units>()
        .and_then(|units| world.get_resource::<Target>()?.unit(units));
    if let Some(u) = picked.or(own_target.as_ref()) {
        let npc = super::plates::npc_of(game, u);
        let faction = if u.faction_id != 0 { u.faction_id } else { npc.map_or(0, |n| n.faction_id as u32) };
        let (race, gender) = unit_race_gender(u.race_gender);
        let (unit_type, detail) = match u.kind {
            UnitKind::Character { .. } => ("character", "character"),
            UnitKind::Npc { .. } => ("npc", "npc"),
            UnitKind::Slave { .. } => ("npc", "slave"),
            UnitKind::Housing { .. } => ("npc", "housing"),
            UnitKind::Transfer { .. } => ("npc", "transfer"),
            UnitKind::Mate { .. } => ("npc", "mate"),
            UnitKind::Shipyard { .. } => ("npc", "shipyard"),
        };
        let name = if !u.name.is_empty() { u.name.clone() } else { npc.map(|n| n.name.clone()).unwrap_or_default() };
        // the maxima come from the database's formulas; the highest seen is the fallback
        let unit_buffs = world.get_resource::<Buffs>().map_or(&[][..], |b| b.of(u.obj_id));
        let (max_hp, max_mp) = game.and_then(|g| super::stats::unit_max(g, u, unit_buffs)).unwrap_or((u.max_hp, u.max_mp));
        let relation = game.map_or("neutral", |g| g.relation(own_faction, faction));
        // 1.2 has no recovered attackability test here (`FUN_39837800` only calls the type-3 vtable's
        // `+0x40`, whose source is not decoded): neutral units, NPC or character, stay attackable as
        // before; only a FRIENDLY one is refused.
        // `FUN_39838880` reasons 2-6 need both owning-character ids (`+0x24`) valid (`FUN_39356b60`):
        // character/slave/house/mate/shipyard, never an NPC or a transfer. Reason 2 (the attacker's
        // unresolved `+0x30` flag) is not modeled. On relation 2/3, reasons 3/4 (same owner, same party)
        // come first and refuse as INVALID_TARGET (0x22); reasons 5/6 then use `FUN_39837a90`'s level <= 10
        // and the zone value of `FUN_39027eb0`/`FUN_39837b90`. The native code reads each unit's own zone;
        // the HUD has only the player's (`Snapshot::zone_faction`), so both sides use it (inferred,
        // capture-pending). The home-zone timed controller override is not modeled (capture-pending).
        let own_character = world
            .get_resource::<Player>()
            .and_then(|p| p.spawn.as_ref())
            .map(|spawn| spawn.character().id)
            .filter(|id| *id != 0);
        let target_character = owning_character(world, u);
        let both_owned = own_character.zip(target_character);
        let same_owner = both_owned.is_some_and(|(a, b)| a == b);
        let same_party = both_owned.is_some_and(|(a, b)| {
            world
                .get_resource::<aa_net::state::Party>()
                .is_some_and(|p| p.id != 0 && p.member(a).is_some() && p.member(b).is_some())
        });
        let invalid_attack = relation != "hostile" && both_owned.is_some() && (same_owner || same_party);
        let zone_value = zone_protection_value(world, game, me);
        let zone_protected = both_owned.is_some()
            && zone_value != 0
            && game.is_some_and(|g| {
                let own = me.level <= 10 && zone_protects(Some(g), zone_value, own_faction);
                let target = u.level <= 10 && zone_protects(Some(g), zone_value, faction);
                own || target
            });
        let protected_level = zone_protected && !invalid_attack;
        let attackable = relation != "friendly" && !protected_level && !invalid_attack;
        p.target = Some(TargetSnap {
            obj_id: u.obj_id,
            npc_type: match u.kind { UnitKind::Npc { template_id, .. } => Some(template_id), _ => None },
            character_id: match u.kind { UnitKind::Character { char_id }=>char_id,_=>0 },
            name,
            level: u.level.max(1),
            hp: u.hp,
            max_hp: max_hp.max(u.hp).max(1),
            mp: u.mp,
            max_mp: max_mp.max(u.mp),
            pvp_honor: u.pvp_honor,
            pvp_kills: u.pvp_kills,
            unit_type,
            detail,
            faction_name: game.and_then(|g| g.factions.get(&faction).cloned()).unwrap_or_default(),
            relation,
            attackable,
            protected_level,
            invalid_attack,
            faction_id: i64::from(faction),
            grade: GameData::grade_name(npc.map_or(1, |n| n.grade)),
            race,
            gender,
            abilities: u.active_abilities.iter().copied().filter(|a| (1..aa_net::model::NO_ABILITY).contains(a)).collect(),
            buffs: world.get_resource::<Buffs>().map(|b| unit_buff_lists(game, b, u.obj_id, start)).unwrap_or_default(),
        });
        target_pos = if Some(u.obj_id) == Some(own_obj_id) {
            Some(pos)
        } else {
            world
                .get_resource::<crate::ingame::units::Views>()
                .and_then(|views| {
                    let drawn = views
                        .unit_entity(u.obj_id)
                        .and_then(|e| crate::ingame::units::current_pose_world(world, e));
                    drawn.or_else(|| {
                        let units = world.get_resource::<Units>()?;
                        crate::ingame::units::unit_world_pose_in(u, units)
                    })
                })
                .map(|(pos, _)| pos)
        };
        target_radius = game.map_or(0.0, |g| g.model_size(u.model_id, u.scale));
    }
    // the dark slots: what the original's `CanUseSkill` says about each (mana, range, target, state ...)
    let own_buffs = world.get_resource::<Buffs>().map_or(&[][..], |b| b.of(own_obj_id));
    let env = Env { game, me, pos, target: p.target.as_ref(), target_pos, skills, buffs: own_buffs, inventory };
    let blocked: Vec<Option<u8>> = p.actions.iter().map(|s| s.as_ref().and_then(|s| env.slot_blocked(s))).collect();
    for (slot, b) in p.actions.iter_mut().zip(blocked) {
        if let Some(s) = slot {
            s.blocked = b;
        }
    }
    (p, target_pos, target_radius)
}

pub(in crate::hud) fn slot_table(lua: &Lua, s: &SlotSnap) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("kind", s.kind)?;
    t.raw_set("id", i64::from(s.id))?;
    t.raw_set("name", s.name.as_str())?;
    t.raw_set("icon", s.icon.as_str())?;
    t.raw_set("count", s.count)?;
    t.raw_set("stack", s.stack)?;
    t.raw_set("grade", i64::from(s.grade))?;
    t.raw_set("uid", s.uid.to_string())?;
    t.raw_set("desc", s.desc.as_str())?;
    t.raw_set("level", s.level)?;
    t.raw_set("group", i64::from(s.group))?;
    t.raw_set("category_order", s.category_order)?;
    if let Some(d) = s.dur {
        t.raw_set("dur", i64::from(d))?;
    }
    t.raw_set("tp", i64::from(s.temper.0))?;
    t.raw_set("tm", i64::from(s.temper.1))?;
    if let Some(f) = s.flags {
        t.raw_set("fl", i64::from(f))?;
        s.extra.write(&t, lua)?;
    }
    if let Some(b) = s.blocked {
        t.raw_set("blocked", i64::from(b))?;
    }
    if let Some((end, total)) = s.cooldown {
        t.raw_set("cd_end", end as f64)?;
        t.raw_set("cd_total", i64::from(total))?;
    }
    Ok(t)
}

fn buff_table(lua: &Lua, b: &BuffSnap) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("buff_id", i64::from(b.id))?;
    t.raw_set("descriptor", b.descriptor)?;
    t.raw_set("buff_kind", b.kind)?;
    t.raw_set("caster_character_id", i64::from(b.caster_character_id))?;
    t.raw_set("duration", f64::from(b.duration_ms as f32))?;
    t.raw_set("name", b.name.as_str())?;
    t.raw_set("path", b.icon.as_str())?;
    t.raw_set("description", b.desc.as_str())?;
    if let Some(count) = b.stacks {
        t.raw_set("stack", count)?;
    }
    t.raw_set("ab_level", i64::from(b.ab_level))?;
    if let Some(end) = b.ends_ms {
        t.raw_set("ends", end as f64)?;
    }
    Ok(t)
}

pub(super) fn buffs_table(lua: &Lua, l: &BuffLists) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("good", lua.create_sequence_from(l.good.iter().map(|b| buff_table(lua, b)).collect::<mlua::Result<Vec<_>>>()?)?)?;
    t.raw_set("bad", lua.create_sequence_from(l.bad.iter().map(|b| buff_table(lua, b)).collect::<mlua::Result<Vec<_>>>()?)?)?;
    Ok(t)
}

fn slots_table(lua: &Lua, slots: &[Option<SlotSnap>]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, s) in slots.iter().enumerate() {
        if let Some(s) = s {
            t.raw_set(i as i64 + 1, slot_table(lua, s)?)?;
        }
    }
    Ok(t)
}

/// `HUD.actions` is keyed by the global packet slot (1 based; slot 0 is unused): the main 12
/// cells follow the page, the sub bars are 13..24.
fn action_table(lua: &Lua, slots: &[Option<SlotSnap>]) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    for (i, s) in slots.iter().enumerate() {
        if let Some(s) = s {
            t.raw_set(i as i64, slot_table(lua, s)?)?;
        }
    }
    Ok(t)
}

/// Put the changed parts of `new` into the `HUD` table (all of it when `old` is `None`); returns
/// what changed.
pub fn push(lua: &Lua, hud: &Table, new: &Panels, old: Option<&Panels>) -> mlua::Result<Changed> {
    let mut c = Changed::default();
    if old.is_none_or(|o| o.actions != new.actions || o.actions_known != new.actions_known) {
        hud.raw_set("actions", action_table(lua, &new.actions)?)?;
        hud.raw_set("actions_known", new.actions_known)?;
        c.slots = true;
    }
    if old.is_none_or(|o| o.bag != new.bag || o.bag_slots != new.bag_slots) {
        hud.raw_set("bag", slots_table(lua, &new.bag)?)?;
        hud.raw_set("bag_slots", i64::from(new.bag_slots))?;
        c.slots = true;
        c.bag = true;
    }
    if old.is_none_or(|o| o.bank != new.bank || o.bank_slots != new.bank_slots) {
        hud.raw_set("bank", slots_table(lua, &new.bank)?)?;
        hud.raw_set("bank_slots", i64::from(new.bank_slots))?;
        c.bank = true;
    }
    if old.is_none_or(|o| o.coffer != new.coffer) {
        hud.raw_set("coffer", slots_table(lua, &new.coffer)?)?;
        hud.raw_set("coffer_slots", new.coffer.len() as i64)?;
    }
    if old.is_none_or(|o| o.equipment != new.equipment) {
        hud.raw_set("equipment", slots_table(lua, &new.equipment)?)?;
        c.equipment = true;
    }
    if old.is_none_or(|o| o.buffs != new.buffs) {
        hud.raw_set("buffs", buffs_table(lua, &new.buffs)?)?;
        c.buffs = true;
    }
    if old.is_none_or(|o| o.target != new.target) {
        match &new.target {
            None => hud.raw_set("target", mlua::Value::Nil)?,
            Some(t) => {
                let x = lua.create_table()?;
                x.raw_set("unit_id", t.obj_id.to_string())?;
                x.raw_set("character_id",t.character_id)?;
                x.raw_set("name", t.name.as_str())?;
                x.raw_set("level", i64::from(t.level))?;
                x.raw_set("hp", i64::from(t.hp))?;
                x.raw_set("max_hp", i64::from(t.max_hp))?;
                x.raw_set("mp", i64::from(t.mp))?;
                x.raw_set("max_mp", i64::from(t.max_mp))?;
                x.raw_set("pvp_honor", i64::from(t.pvp_honor))?;
                x.raw_set("pvp_kills", i64::from(t.pvp_kills))?;
                x.raw_set("dead", t.hp <= 0)?;
                x.raw_set("in_combat", false)?;
                x.raw_set("unit_type", t.unit_type)?;
                x.raw_set("detail", t.detail)?;
                x.raw_set("npc_type", t.npc_type)?;
                x.raw_set("faction", t.faction_name.as_str())?;
                x.raw_set("relation", t.relation)?;
                x.raw_set("grade", t.grade)?;
                x.raw_set("race", i64::from(t.race))?;
                x.raw_set("gender", i64::from(t.gender))?;
                x.raw_set("abilities", lua.create_sequence_from(t.abilities.iter().map(|a| i64::from(*a)))?)?;
                x.raw_set("buffs", buffs_table(lua, &t.buffs)?)?;
                hud.raw_set("target", x)?;
            }
        }
        c.target = true;
    }
    Ok(c)
}

/// Which parts of the panels changed in a [`push`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Changed {
    /// Action bar or bag: the slot widgets should ask again.
    pub slots: bool,
    pub bag: bool,
    pub bank: bool,
    pub equipment: bool,
    pub buffs: bool,
    pub target: bool,
}

/// Use what is in global action bar slot `slot` (1 based): cast the skill, use the item. What the client already knows is
/// refused (no target, too far, no mana ...) is not sent: the alert says why instead ([`UseFailed`]). A press that waits for
/// the global cooldown, the cast it repeats or the answer to the last press is dropped without an alert ([`Env::held_back`]).
#[allow(clippy::too_many_arguments)]
pub fn use_slot(slot: usize, env: &Env, flight: &InFlight, bar: &ActionBar, inventory: &Inventory, target: &Target, net: &mut MessageWriter<SkillPressed>, failed: &mut MessageWriter<UseFailed>) {
    let Some(action) = bar.get(slot) else { return };
    let target_obj_id = target.obj_id.unwrap_or(0);
    match action.kind {
        ActionKind::Spell | ActionKind::RidePetSpell => {
            let skill = action.id as u32;
            if env.held_back(skill, flight) {
                return;
            }
            // a basic attack checks its target and then range-checks (`FUN_39397f90`): an out-of-range press is sent unless the
            // `DAT_39d1fba4` setting returns 1, which then takes the click-attack path (`FUN_39393920`) without a send; the K2
            // client sent `CSStartSkill 2` at 11.2 m, so its setting was not 1 (the default is not provable). The client's own
            // refusal shows the same alert as any skill.
            let refusal = if env.is_auto_attack(skill) { env.precheck_auto_attack(skill) } else { env.precheck(skill) };
            if let Some(f) = refusal {
                failed.write(UseFailed { skill, result: f.result, value: f.value, server: false });
                return;
            }
            // `ground_target::route` sends the command, or aims first when the skill takes a spot of the ground
            net.write(SkillPressed { skill, item: None, target_obj_id, gcd: env.uses_gcd(skill) });
        }
        ActionKind::ItemType | ActionKind::ItemId => {
            let found = match action.item_template_id() {
                Some(t) => inventory.find_template(t).map(|(_, it)| it),
                None => inventory.find_by_id(action.id).map(|(_, _, it)| it),
            };
            let Some(item) = found else {
                failed.write(UseFailed { skill: 0, result: super::usable::code::NEED_REAGENT, value: action.item_template_id().unwrap_or(0), server: false });
                return;
            };
            use_item(item, env, flight, target_obj_id, net, failed);
        }
        _ => {}
    }
}

/// Use a bag item (a right click in the bag window): its skill, on the target.
pub fn use_bag_slot(slot: usize, env: &Env, flight: &InFlight, inventory: &Inventory, target: &Target, net: &mut MessageWriter<SkillPressed>, failed: &mut MessageWriter<UseFailed>) {
    if let Some(Some(item)) = inventory.bag.get(slot) {
        use_item(item, env, flight, target.obj_id.unwrap_or(0), net, failed);
    }
}

fn use_item(item: &aa_net::model::Item, env: &Env, flight: &InFlight, target_obj_id: u32, net: &mut MessageWriter<SkillPressed>, failed: &mut MessageWriter<UseFailed>) {
    let skill_id = env.game.and_then(|g| g.items.get(&item.template_id)).map_or(0, |i| i.use_skill);
    if skill_id != 0 && env.held_back(skill_id, flight) {
        return;
    }
    if let Some((skill, f)) = env.precheck_item(item.template_id) {
        failed.write(UseFailed { skill, result: f.result, value: f.value, server: false });
        return;
    }
    net.write(SkillPressed { skill: skill_id, item: Some((item.id, item.template_id)), target_obj_id, gcd: env.uses_gcd(skill_id) });
}

/// The action bar's number keys.
#[allow(clippy::too_many_arguments)]
pub fn action_keys(
    actions: Res<crate::settings::Actions>,
    focus: Res<HudKeyboardFocus>,
    bridge: NonSend<HudBridge>,
    mut bar: ResMut<ActionBar>,
    inventory: Res<Inventory>,
    target: Res<Target>,
    skills: Res<Skills>,
    buffs: Res<Buffs>,
    mut flight: ResMut<InFlight>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    mut script: ResMut<super::chat::ScriptedKeys>,
    mut net: MessageWriter<SkillPressed>,
    mut cast: MessageWriter<aa_net::NetCommand>,
    mut failed: MessageWriter<UseFailed>,
) {
    if let Some(slot) = script.take_slot() {
        // `take_slot` is the 0-based display cell (`slotN` in the script is 1 based).
        let page = bridge.0.borrow().action_page;
        if let Some(slot) = ActionBar::display_slot_on(page, slot) {
            let data = bridge.0.borrow();
            let env = Env::of(&data, Some(&skills), buffs.of(skills.obj_id), Some(&inventory));
            use_slot(slot, &env, &flight, &bar, &inventory, &target, &mut net, &mut failed);
        }
    }
    if focus.0 {
        return;
    }
    for (action, slot, button) in actions.hotkeys() {
        if button {
            continue;
        }
        if action == "action_bar_page_prev" || action == "action_bar_page_next" {
            let current = bridge.0.borrow().action_page;
            let page = if action == "action_bar_page_prev" {
                (current + aa_net::items::ACTION_PAGES - 1) % aa_net::items::ACTION_PAGES
            } else {
                (current + 1) % aa_net::items::ACTION_PAGES
            };
            if bar.set_page(page) {
                bridge.0.borrow_mut().action_page = page;
                if let Some(rt) = rt.as_ref() {
                    // The cells show the new page's slots: `HUD.actions` is keyed by global slot and
                    // does not change, so nothing else redraws them.
                    if let Err(e) = rt.refresh_slots() {
                        warn!("hud: refresh slots: {e}");
                    }
                    rt.fire_event("ACTION_BAR_PAGE_CHANGED", (i64::from(page) + 1,));
                }
            }
            continue;
        }
        if action == "action_bar_button" && (1..=58).contains(&slot) {
            let page = bridge.0.borrow().action_page;
            let Some(slot) = ActionBar::display_slot_on(page, slot as usize - 1) else { continue };
            let data = bridge.0.borrow();
            let env = Env::of(&data, Some(&skills), buffs.of(skills.obj_id), Some(&inventory));
            use_slot(slot, &env, &flight, &bar, &inventory, &target, &mut net, &mut failed);
        }
        if action == "pet_action_bar_button" && (1..=5).contains(&slot) {
            bridge.0.borrow_mut().requests.push(super::api::Request::UsePetAction { slot: slot as usize });
        }
    if !button && action == "mode_action_bar_button" && (1..=15).contains(&slot) {
            let mut data = bridge.0.borrow_mut();
            let index = slot as usize - 1;
            if let Some(function) = super::mode_actions::function_at(&data, index) {
                super::mode_actions::run_function(
                    &mut data,
                    function,
                    web_time::Instant::now(),
                );
                continue;
            }
            let env = Env::of(
                &data,
                Some(&skills),
                buffs.of(skills.obj_id),
                Some(&inventory),
            );
            super::mode_actions::use_slot(
                &data,
                index,
                &env,
                &mut flight,
                &target,
                &mut net,
                &mut cast,
                &mut failed,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use aa_net::state::NativeCooldown;

    #[test]
    fn neutral_npcs_stay_attackable_and_friendly_npcs_are_not() {
        use aa_net::model::UnitKind;
        use crate::hud::data::NpcInfo;
        let mut game = GameData::default();
        game.factions.insert(101, "Mine".into());
        game.factions.insert(2, "Neutral".into());
        game.npcs.insert(
            42,
            NpcInfo {
                name: "Amutar".into(),
                level: 15,
                grade: 1,
                kind: 14,
                template: 9,
                faction_id: 2,
                show_name_tag: true,
            },
        );
        game.npcs.insert(
            43,
            NpcInfo {
                name: "Cocky Trainee".into(),
                level: 32,
                grade: 1,
                kind: 1,
                template: 9,
                faction_id: 2,
                show_name_tag: true,
            },
        );
        game.npcs.insert(
            44,
            NpcInfo {
                name: "Friendly Guard".into(),
                level: 32,
                grade: 1,
                kind: 1,
                template: 9,
                faction_id: 101,
                show_name_tag: true,
            },
        );
        let mut units = Units::default();
        for (obj_id, template, faction) in [(7, 42, 2), (8, 43, 2), (9, 44, 101)] {
            let mut unit = crate::hud::fixture_spawn().unit;
            unit.obj_id = obj_id;
            unit.kind = UnitKind::Npc {
                template_id: template,
                owner_id: 0,
            };
            unit.faction_id = faction;
            units.by_id.insert(obj_id, unit);
        }
        let mut app = App::new();
        app.insert_resource(units).init_resource::<Target>().init_resource::<Buffs>();
        let attackable = |app: &mut App, obj_id| {
            app.world_mut().resource_mut::<Target>().obj_id = Some(obj_id);
            let (panels, _, _) = refresh(
                app.world(),
                Some(&game),
                101,
                1,
                Instant::now(),
                &Snapshot::default(),
                Vec3::ZERO,
            );
            panels.target.unwrap().attackable
        };
        assert!(attackable(&mut app, 7), "a neutral NPC (Amutar-like) is attackable");
        assert!(attackable(&mut app, 8), "another neutral NPC is attackable");
        assert!(!attackable(&mut app, 9), "a friendly NPC is not attackable");
    }

    /// `FUN_39838880` reasons 2-6 need both owning-character ids (`+0x24`): a character target is
    /// protected while the attacker or the target is level 10 or below (`FUN_39837a90`) and the zone is
    /// protected for that side's faction (`FUN_39027eb0` + `FUN_39837b90`). An NPC has no `+0x24`, so no
    /// level protection. On neutral/friendly relation reasons 3/4 (same owner, same party) come first.
    /// Both sides use the player's zone here (inferred).
    #[test]
    fn the_low_level_zone_protection_gates_owned_targets() {
        use aa_net::conflict::ConflictZoneState;
        use aa_net::model::UnitKind;
        use aa_net::party::PartyMember;
        use aa_net::state::{ConflictZones, Party};
        let spawn = crate::hud::fixture_spawn();
        let mut game = GameData::default();
        for faction in [101, 102, 103, 148] {
            game.factions.insert(faction, format!("F{faction}"));
        }
        game.relations.insert((101, 102), 1);
        let mut units = Units::default();
        let mut unit = spawn.unit.clone();
        unit.obj_id = 7;
        unit.kind = UnitKind::Character { char_id: 102 };
        unit.faction_id = 102;
        unit.level = 32;
        units.by_id.insert(7, unit);
        let mut app = App::new();
        app.insert_resource(units)
            .insert_resource(Player { spawn: Some(spawn) })
            .init_resource::<Target>()
            .init_resource::<Buffs>();
        let check = |app: &mut App, game: &GameData, level: u8, zone: u32, target_level: u8, group: u32| {
            app.world_mut().resource_mut::<Target>().obj_id = Some(7);
            app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().level = target_level;
            let me = Snapshot {
                level,
                zone_faction: zone,
                zone_group: group,
                ..Snapshot::default()
            };
            let (panels, _, _) = refresh(app.world(), Some(game), 101, 100, Instant::now(), &me, Vec3::ZERO);
            let t = panels.target.unwrap();
            (t.protected_level, t.attackable, t.invalid_attack)
        };
        // a hostile character target in a home zone: the attacker is level 10 in a zone its own faction
        // holds (reason 5), using `zones.faction_id` because group 0 has no conflict row
        assert_eq!(check(&mut app, &game, 10, 101, 32, 0), (true, false, false));
        // level 11 is outside `FUN_39837a90`'s level <= 10
        assert_eq!(check(&mut app, &game, 11, 101, 32, 0), (false, true, false));
        // the target is level 10 in a home zone its own faction holds (reason 6)
        assert_eq!(check(&mut app, &game, 32, 102, 10, 0), (true, false, false));
        assert_eq!(check(&mut app, &game, 32, 102, 11, 0), (false, true, false));
        // no zone faction and no conflict row: no protection
        assert_eq!(check(&mut app, &game, 10, 0, 10, 0), (false, true, false));
        // an NPC target has no owning character, so the level protection never applies to it
        app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().kind = UnitKind::Npc { template_id: 0, owner_id: 0 };
        assert_eq!(check(&mut app, &game, 5, 101, 1, 0), (false, true, false));
        // a neutral character target: reason 6 still applies ...
        app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().kind = UnitKind::Character { char_id: 103 };
        app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().faction_id = 103;
        assert_eq!(check(&mut app, &game, 32, 103, 10, 0), (true, false, false));
        // ... unless the same party makes reasons 3/4 come first
        app.insert_resource(Party {
            id: 9,
            members: vec![PartyMember { id: 47, ..Default::default() }, PartyMember { id: 103, ..Default::default() }],
            ..Default::default()
        });
        assert_eq!(check(&mut app, &game, 32, 103, 10, 0), (false, false, true));
        app.world_mut().resource_mut::<Party>().id = 0;
        app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().kind = UnitKind::Character { char_id: 102 };
        app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().faction_id = 102;
        // a conflict group (row 14, peace_protected_faction_id 5) protects every owned unit in peace;
        // its `zones.faction_id` is NULL, so the value comes from the row, not from the snapshot
        game.conflict_zones.insert(14, (false, 5));
        app.insert_resource(ConflictZones {
            by_group: [(14, ConflictZoneState { zone_group_id: 14, state: 7, end_time: 0 })].into(),
        });
        assert_eq!(check(&mut app, &game, 5, 0, 32, 14), (true, false, false));
        // any other received state, no state, and a closed row are no protection
        app.insert_resource(ConflictZones {
            by_group: [(14, ConflictZoneState { zone_group_id: 14, state: 6, end_time: 0 })].into(),
        });
        assert_eq!(check(&mut app, &game, 5, 0, 32, 14), (false, true, false));
        app.world_mut().remove_resource::<ConflictZones>();
        assert_eq!(check(&mut app, &game, 5, 0, 32, 14), (false, true, false));
        game.conflict_zones.insert(14, (true, 5));
        app.insert_resource(ConflictZones {
            by_group: [(14, ConflictZoneState { zone_group_id: 14, state: 7, end_time: 0 })].into(),
        });
        assert_eq!(check(&mut app, &game, 5, 0, 32, 14), (false, true, false));
        game.conflict_zones.insert(14, (false, 5));
        // a home zone still uses `zones.faction_id`; sentinel 5 protects every owned unit and 4 one
        // friendly to faction 148/149
        assert_eq!(check(&mut app, &game, 10, 5, 32, 0), (true, false, false));
        app.world_mut().resource_mut::<Units>().by_id.get_mut(&7).unwrap().faction_id = 148;
        assert_eq!(check(&mut app, &game, 32, 4, 10, 0), (true, false, false));
    }

    #[test]
    fn action_bar_page_keys_rotate_the_four_pages() {
        let bridge = HudBridge(Rc::new(RefCell::new(crate::hud::api::HudData::default())));
        let mut actions = crate::settings::Actions::default();
        actions.tap_for_test("action_bar_page_next");
        let mut app = App::new();
        app.insert_resource(actions)
            .insert_resource(HudKeyboardFocus(false))
            .insert_non_send(bridge)
            .init_resource::<ActionBar>()
            .init_resource::<Inventory>()
            .init_resource::<Target>()
            .init_resource::<Skills>()
            .init_resource::<Buffs>()
            .init_resource::<InFlight>()
            .init_resource::<super::super::chat::ScriptedKeys>()
            .add_message::<SkillPressed>()
            .add_message::<aa_net::NetCommand>()
            .add_message::<UseFailed>()
            .add_systems(Update, action_keys);
        app.update();
        assert_eq!(app.world().resource::<ActionBar>().page(), 1);
        let mut prev = crate::settings::Actions::default();
        prev.tap_for_test("action_bar_page_prev");
        app.world_mut().insert_resource(prev);
        app.update();
        assert_eq!(app.world().resource::<ActionBar>().page(), 0);
        let mut prev = crate::settings::Actions::default();
        prev.tap_for_test("action_bar_page_prev");
        app.world_mut().insert_resource(prev);
        app.update();
        assert_eq!(app.world().resource::<ActionBar>().page(), 3);
    }

    #[test]
    fn pet_action_bar_keys_queue_the_pet_slot() {
        let data = Rc::new(RefCell::new(crate::hud::api::HudData::default()));
        let mut actions = crate::settings::Actions::default();
        actions.tap_for_test_at("pet_action_bar_button", 1);
        let mut app = App::new();
        app.insert_resource(actions)
            .insert_resource(HudKeyboardFocus(false))
            .insert_non_send(HudBridge(data.clone()))
            .init_resource::<ActionBar>()
            .init_resource::<Inventory>()
            .init_resource::<Target>()
            .init_resource::<Skills>()
            .init_resource::<Buffs>()
            .init_resource::<InFlight>()
            .init_resource::<super::super::chat::ScriptedKeys>()
            .add_message::<SkillPressed>()
            .add_message::<aa_net::NetCommand>()
            .add_message::<UseFailed>()
            .add_systems(Update, action_keys);
        app.update();
        assert!(matches!(
            &data.borrow().requests[..],
            [crate::hud::api::Request::UsePetAction { slot: 1 }]
        ));
    }

    #[test]
    fn scripted_slot_uses_the_page_cell_the_scripts_see() {
        let bridge = HudBridge(Rc::new(RefCell::new(crate::hud::api::HudData {
            action_page: 1,
            ..Default::default()
        })));
        let mut script = super::super::chat::ScriptedKeys::default();
        script.press_slot_for_test(2);
        let mut bar = ActionBar::default();
        bar.slots = vec![aa_net::items::ActionSlot::default(); aa_net::items::ACTION_SLOTS];
        bar.slots[50] = aa_net::items::ActionSlot { kind: ActionKind::Spell, id: 18132 };
        assert!(bar.set_page(1));
        let mut app = App::new();
        app.insert_resource(crate::settings::Actions::default())
            .insert_resource(HudKeyboardFocus(false))
            .insert_non_send(bridge)
            .insert_resource(bar)
            .insert_resource(script)
            .init_resource::<Inventory>()
            .init_resource::<Target>()
            .init_resource::<Skills>()
            .init_resource::<Buffs>()
            .init_resource::<InFlight>()
            .add_message::<SkillPressed>()
            .add_message::<aa_net::NetCommand>()
            .add_message::<UseFailed>()
            .add_systems(Update, action_keys);
        app.update();
        let pressed: Vec<_> = app.world_mut()
            .resource_mut::<Messages<SkillPressed>>()
            .drain()
            .collect();
        assert!(matches!(&pressed[..], [skill] if skill.skill == 18132), "{pressed:?}");
    }

    #[test]
    fn hud_actions_use_the_global_slots() {
        let lua = mlua::Lua::new();
        let hud = lua.create_table().unwrap();
        let mut panels = Panels::default();
        panels.actions = vec![None; aa_net::items::ACTION_SLOTS];
        panels.actions[1] = Some(SlotSnap { kind: "skill", id: 18132, name: "Triple Slash".into(), ..SlotSnap::default() });
        panels.actions[49] = Some(SlotSnap { kind: "skill", id: 18132, name: "Triple Slash".into(), ..SlotSnap::default() });
        push(&lua, &hud, &panels, None).unwrap();
        lua.globals().set("HUD", hud).unwrap();
        assert!(lua.load("return HUD.actions[1] ~= nil and HUD.actions[49] ~= nil and HUD.actions[0] == nil").eval::<bool>().unwrap());
    }

    /// F3: a linked action slot shows the converted skill's cooldown and is dark when the item is not equipped. This
    /// drives `refresh`, whose `Env::slot_blocked` pass is the path that sets `SlotSnap::blocked` for the bar.
    #[test]
    fn a_linked_action_slot_shows_the_converted_cooldown_and_goes_dark_without_the_item() {
        use aa_net::items::{ActionKind, ActionSlot};

        let mut game = GameData::default();
        game.skills.insert(
            14900,
            super::super::data::SkillInfo {
                named: super::super::data::Named { name: "Use Instrument".into(), ..Default::default() },
                ..Default::default()
            },
        );
        game.skills.insert(11951, super::super::data::SkillInfo { cooldown_ms: 45_000, ..Default::default() });
        game.skill_links.insert(
            14900,
            crate::ingame::data::SkillLink { slot: aa_net::model::slot::MUSICAL, backpack_type: 1 },
        );
        game.items.insert(20480, super::super::data::ItemInfo { use_skill: 11951, ..Default::default() });
        let mut slots = vec![ActionSlot::default(); aa_net::items::ACTION_SLOTS];
        slots[1] = ActionSlot {
            kind: ActionKind::Spell,
            id: 14900,
        };
        let bar = ActionBar {
            slots,
            ..Default::default()
        };
        let mut skills = Skills::default();
        skills
            .last_used
            .insert(11951, Instant::now() - std::time::Duration::from_secs(1));
        let mut inventory = Inventory::default();
        inventory.equipment.slots[aa_net::model::slot::MUSICAL] =
            Some(aa_net::model::Item { id: 5, template_id: 20480, ..Default::default() });
        let start = Instant::now();
        let me = super::super::data::Snapshot::default();
        let world = |inventory| {
            let mut world = World::new();
            world.insert_resource(bar.clone());
            world.insert_resource(skills.clone());
            world.insert_resource(inventory);
            world
        };
        let (panels, _, _) = refresh(&world(inventory), Some(&game), 0, 0, start, &me, Vec3::ZERO);
        let slot = panels.actions[1].as_ref().expect("the 14900 slot");
        assert_eq!(slot.cooldown.map(|(_, total)| total), Some(45_000));
        assert_eq!(slot.name, "Use Instrument");
        assert_eq!(slot.blocked, None);
        let (bare, _, _) = refresh(
            &world(Inventory::default()),
            Some(&game),
            0,
            0,
            start,
            &me,
            Vec3::ZERO,
        );
        assert_eq!(
            bare.actions[1].as_ref().and_then(|s| s.blocked),
            Some(super::super::usable::code::URK_EQUIP_ITEM)
        );
    }

    #[test]
    fn skill_sweep_keeps_the_winning_received_duration_including_zero() {
        let mut game = GameData::default();
        game.skills.insert(7, super::super::data::SkillInfo {
            tag: 9, tags: vec![9], cooldown_ms: 10_000, ignore_gcd: true, ..default()
        });
        let start = Instant::now();
        let mut skills = Skills::default();
        skills.native_tag_cooldowns.push(NativeCooldown {
            cooldown: aa_net::Cooldown { id: 9, total_ms: 25_000, remaining_ms: 5_000 },
            updated_at: start, origin: aa_net::state::NativeCooldownOrigin::Snapshot, reset: false,
        });
        let snap = skill_slot(Some(&game), Some(&skills), 7, start);
        assert_eq!(snap.cooldown.map(|(_, total)| total), Some(25_000));
        assert!((4_500..=5_250).contains(&snap.cooldown.unwrap().0));

        skills.native_tag_cooldowns[0].cooldown.total_ms = 0;
        assert_eq!(skill_slot(Some(&game), Some(&skills), 7, start).cooldown.map(|(_, total)| total), Some(0));

        // A longer observed cast still wins, with its own DB duration, as before.
        skills.last_used.insert(7, Instant::now());
        assert_eq!(skill_slot(Some(&game), Some(&skills), 7, start).cooldown.map(|(_, total)| total), Some(10_000));
        skills.last_used.clear();
        skills.native_tag_cooldowns[0].reset = true;
        assert!(skill_slot(Some(&game), Some(&skills), 7, start).cooldown.is_none());
    }
}

#[cfg(test)]
mod attached_target_tests {
    use super::*;
    use aa_net::model::{UnitKind, UnitStateAttachments};
    use crate::ingame::units::Views;

    fn slave(id: u32, pos: Vec3, parent: Option<u32>) -> aa_net::model::Unit {
        let mut unit = crate::hud::fixture_spawn().unit;
        unit.obj_id = id;
        unit.kind = UnitKind::Slave {
            id: 1,
            tl_id: 1,
            template_id: 1,
            summoner_id: 0,
        };
        unit.pos = aa_net::Pos::new(pos.x, pos.y, pos.z);
        if let Some(parent) = parent {
            unit.state_attachments = Some(UnitStateAttachments {
                first_point: 3,
                first_obj_id: Some(parent),
                bonding_obj_id: None,
            });
        }
        unit
    }

    #[test]
    fn an_attached_child_target_pos_is_its_world_point() {
        let ship = Vec3::new(12000.0, 9000.0, 0.0);
        let local = Vec3::new(2.0, 3.0, 1.0);
        let mut app = App::new();
        app.add_plugins(bevy::transform::TransformPlugin)
            .init_resource::<Units>()
            .init_resource::<Views>()
            .init_resource::<Target>();
        let ship_unit = slave(60, ship, None);
        let child_unit = slave(7, local, Some(60));
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .insert(60, ship_unit.clone());
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .insert(7, child_unit.clone());
        let ship_entity = app
            .world_mut()
            .spawn(Transform::from_translation(aa_world::cry_to_bevy(ship)))
            .id();
        let child_entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(aa_world::cry_to_bevy(local)),
                ChildOf(ship_entity),
            ))
            .id();
        {
            let mut views = app.world_mut().resource_mut::<Views>();
            views.fixture_actor(&ship_unit, Some(ship_entity));
            views.fixture_actor(&child_unit, Some(child_entity));
        }
        app.world_mut().resource_mut::<Target>().obj_id = Some(7);
        app.update();
        let (_, target_pos, _) = refresh(
            app.world(),
            None,
            0,
            1,
            Instant::now(),
            &Snapshot::default(),
            Vec3::ZERO,
        );
        let expected = ship + local;
        let target_pos = target_pos.expect("the target is in view");
        assert!(
            target_pos.distance(expected) < 1e-3,
            "target_pos {target_pos:?} != {expected:?} (local {local:?})"
        );
        assert!(target_pos.distance(local) > 1.0, "the local offset was used");
    }
}
