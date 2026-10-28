//! Server combat events for the original UI scripts. The toolkit owns the native
//! `damagedisplay` pool and animation timelines; the app only projects unit anchors.

use aa_net::Event;
use aa_net::skill::hit_type;
use aa_net::state::{Player, Units};
use bevy::prelude::*;
use mlua::Value;

use super::api::HudBridge;

#[allow(dead_code)]
pub(super) mod fixture;
mod positions;

/// Quiet period after combat ends, so packets for the last blow cannot restart it.
const OVER_QUIET: f64 = 1.0;

pub(super) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    positions::install(lua)
}

// -- events -------------------------------------------------------------------------------------------

/// `hitOrMissType` of the scripts for a wire hit type: "HIT", "CRITICAL" or the miss kind.
fn hit_name(h: u8) -> &'static str {
    match h {
        hit_type::MELEE_CRITICAL | hit_type::RANGED_CRITICAL | hit_type::SPELL_CRITICAL => "CRITICAL",
        hit_type::MELEE_MISS | hit_type::RANGED_MISS | hit_type::SPELL_MISS => "MISS",
        hit_type::MELEE_DODGE | hit_type::RANGED_DODGE => "DODGE",
        hit_type::MELEE_BLOCK | hit_type::RANGED_BLOCK => "BLOCK",
        hit_type::MELEE_PARRY | hit_type::RANGED_PARRY => "PARRY",
        hit_type::IMMUNE => "IMMUNE",
        hit_type::SPELL_RESIST => "RESIST",
        _ => "HIT",
    }
}

/// A unit's name for the chat lines and the death window's resurrection caster.
pub(super) fn unit_name(units: &Units, game: Option<&super::data::GameData>, own: &Player, id: u32) -> String {
    if own.spawn.as_ref().is_some_and(|s| s.obj_id == id) {
        return own.spawn.as_ref().map(|s| s.name().to_string()).unwrap_or_default();
    }
    let Some(u) = units.get(id) else { return String::new() };
    if !u.name.is_empty() {
        return u.name.clone();
    }
    super::plates::npc_of(game, u).map(|n| n.name.clone()).unwrap_or_default()
}

/// Native39391720 ->3940a3e0. Presentation is separate from the authoritative points packet.
pub(super) fn environment(rt: &aa_ui::UiRuntime, damage: &aa_net::EnvironmentDamage, name: &str) {
    let source = match damage.source {
        0 => "FALLING",
        2 => "DROWNING",
        _ => return,
    };
    if damage.amount == 0 { return; }
    let name = if name.is_empty() { "unknown" } else { name };
    // Native3940a9dd uses CVTSI2SS on the raw dword, so this presentation path deliberately
    // sign-interprets and rounds to f32. The aa-net report retains the original unsigned value.
    let amount = f64::from(damage.amount as i32 as f32);
    rt.fire_event("COMBAT_MSG", (
        super::unit_id::format(damage.target_obj_id), "ENVIRONMENTAL_DAMAGE", name, name, source, amount,
        "HEALTH", "HIT", 0i64, "idle", 0i64, false,
    ));
}

/// A damage packet whose blow has landed (`ingame::combat::DamageLanded`): the floating number and the combat log line. `now` is the
/// time the combat code took the packet up.
#[allow(clippy::too_many_arguments)]
fn show_damage(
    d: &aa_net::skill::UnitDamage,
    now: f64,
    over_at: f64,
    own: u32,
    own_pos: Vec3,
    rt: &aa_ui::UiRuntime,
    bridge: &HudBridge,
    units: &Units,
    player: &Player,
    game: Option<&super::data::GameData>,
) {
    let skill = |id: u32| game.and_then(|g| g.skills.get(&id));
    let id = |obj: u32| super::unit_id::format(obj);
    if d.caster_obj_id != own && d.target_obj_id != own {
        return;
    }
    // (the packet of the killing blow comes after the one that ends the fight: it is not a new fight)
    if now - over_at > OVER_QUIET {
        bridge.0.borrow_mut().local_combat = true;
    }
    let info = skill(d.action.skill_id);
    let auto = info.is_some_and(|s| s.auto_attack);
    let hit = hit_name(d.hit_type);
    let missed = hit_type::is_miss(d.hit_type);
    if d.target_obj_id == own {
        let source = unit_name(&units, game, &player, d.caster_obj_id);
        let target = unit_name(&units, game, &player, own);
        let spell = info.map_or_else(String::new, |s| s.named.name.clone());
        let (event, with_spell) = match (missed, d.action.kind == 2, auto) {
            (true, _, true) => ("MELEE_MISSED", false),
            (true, _, false) => ("SPELL_MISSED", true),
            (false, true, _) => ("SPELL_DOT_DAMAGE", true),
            (false, false, true) => ("MELEE_DAMAGE", false),
            (false, false, false) => ("SPELL_DAMAGE", true),
        };
        // (targetId, event, source, target, [spellId, spellName, school,] then the event's own values)
        let mut args: Vec<Value> = Vec::new();
        let s = |text: &str| rt.lua.create_string(text).map(Value::String).unwrap_or(Value::Nil);
        args.extend([s(&id(own)), s(event), s(&source), s(&target)]);
        if with_spell {
            args.extend([crate::lua_int(i64::from(d.action.skill_id)), s(&spell), s("physical")]);
        }
        if missed {
            args.push(s(hit));
        } else {
            args.extend([crate::lua_int(i64::from(d.damage)), s("HEALTH"), s(hit), crate::lua_int(i64::from(d.absorbed)), s("slashing"), Value::Integer(0)]);
        }
        rt.fire_event("COMBAT_MSG", mlua::MultiValue::from_iter(args));
    } else {
        let skill_type = if d.action.kind == 2 {
            "DOT"
        } else if auto {
            "SWING"
        } else {
            "SKILL"
        };
        let distance = units.get(d.target_obj_id).map_or(0.0, |u| own_pos.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) as f64);
        debug!("hud: COMBAT_TEXT {} -> {} {} {skill_type} {hit}", d.caster_obj_id, d.target_obj_id, d.damage);
        rt.fire_event(
            "COMBAT_TEXT",
            (id(own), id(d.target_obj_id), -i64::from(d.damage), skill_type, hit, 0i64, false, distance),
        );
    }
}

/// aa-net's combat events as the scripts' events (after `sync_hud` pushed this frame's state).
#[allow(clippy::too_many_arguments)]
pub fn route_combat(
    mut events: MessageReader<aa_net::NetEvent>,
    mut landed: MessageReader<crate::ingame::combat::DamageLanded>,
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Res<Units>,
    player: Res<Player>,
    time: Res<Time>,
    mut over_at: Local<f64>,
) {
    let Some(rt) = rt else {
        events.clear();
        landed.clear();
        return;
    };
    let now = time.elapsed_secs_f64();
    let Some(own) = player.spawn.as_ref().map(|s| s.obj_id) else {
        events.clear();
        landed.clear();
        return;
    };
    // no borrow of the bridge while the events run: the scripts call back into the game API
    let (shared_game, own_pos) = {
        let data = bridge.0.borrow();
        (data.game.clone(), data.pos)
    };
    let game = shared_game.get();
    let skill = |id: u32| game.and_then(|g| g.skills.get(&id));
    let id = |obj: u32| super::unit_id::format(obj);
    for aa_net::NetEvent(event) in events.read() {
        match event {
            Event::EnvDamage(damage) => {
                let name = unit_name(&units, game, &player, damage.target_obj_id);
                environment(&rt, damage, &name);
            }
            // (the damage packets come with the blow: `DamageLanded`, below)
            Event::UnitDamaged(_) => {}
            Event::UnitHealed(h) => {
                if h.target_obj_id != own && h.caster.obj_id != own {
                    continue;
                }
                let critical = matches!(h.hit_type, hit_type::RANGED_CRITICAL | hit_type::SPELL_CRITICAL);
                let hit = if critical { "CRITICAL" } else { "HIT" };
                if h.target_obj_id == own {
                    let name = skill(h.action.skill_id).map_or_else(String::new, |s| s.named.name.clone());
                    let (event, power) = if h.heal_type == 1 { ("SPELL_ENERGIZE", "MANA") } else { ("SPELL_HEALED", "HEALTH") };
                    let source = unit_name(&units, game, &player, h.caster.obj_id);
                    let target = unit_name(&units, game, &player, own);
                    if h.heal_type == 1 {
                        rt.fire_event("COMBAT_MSG", (id(own), event, source, target, i64::from(h.action.skill_id), name, "physical", i64::from(h.value), power));
                    } else {
                        rt.fire_event("COMBAT_MSG", (id(own), event, source, target, i64::from(h.action.skill_id), name, "physical", i64::from(h.value), hit));
                    }
                } else {
                    let skill_type = if h.heal_type == 1 { "MANA" } else { "HEAL" };
                    rt.fire_event("COMBAT_TEXT", (id(own), id(h.target_obj_id), i64::from(h.value), skill_type, hit, 0i64, false, 0.0f64));
                }
            }
            // attacking something starts the fight the moment the skill does (the weapon comes out)
            Event::SkillStarted(s) if s.caster.obj_id == own && s.result == 0 && s.target.kind == aa_net::skill::target_kind::UNIT && s.target.obj_id != 0 && s.target.obj_id != own => {
                if now - *over_at > OVER_QUIET {
                    bridge.0.borrow_mut().local_combat = true;
                }
            }
            // the server clears the combat state of the player when the fight is over (the kill); a death ends it too
            Event::CombatChanged { obj_id, engaged: false } | Event::UnitDeath { obj_id } | Event::Resurrected { obj_id, .. } if *obj_id == own => {
                debug!("hud: the combat of the player is over ({event:?})");
                bridge.0.borrow_mut().local_combat = false;
                *over_at = now;
            }
            Event::CombatChanged { obj_id, engaged } => debug!("hud: combat state of unit {obj_id}: {engaged}"),
            Event::ExpChanged { obj_id, delta, .. } if *obj_id == own => {
                rt.fire_event("EXP_CHANGED", (id(own), i64::from(*delta), delta.abs().to_string()));
            }
            _ => {}
        }
    }
    // the blows: a number shows when the blow lands, not when the packet came
    for crate::ingame::combat::DamageLanded { damage, received } in landed.read() {
        show_damage(damage, *received, *over_at, own, own_pos, &rt, &bridge, &units, &player, game);
    }
}

pub fn plugin(app: &mut App) {
    positions::plugin(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_hit_types() {
        assert_eq!(hit_name(hit_type::MELEE_HIT), "HIT");
        assert_eq!(hit_name(hit_type::SPELL_CRITICAL), "CRITICAL");
        assert_eq!(hit_name(hit_type::RANGED_DODGE), "DODGE");
        assert_eq!(hit_name(hit_type::SPELL_RESIST), "RESIST");
    }

    #[test]
    fn environment_reports_keep_native_arguments_and_numeric_conversion() -> Result<(), Box<dyn std::error::Error>> {
        let rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.lua.load(r#"
            received={}; text_count=0
            UIParent:SetEventHandler('COMBAT_TEXT',function() text_count=text_count+1 end)
            UIParent:SetEventHandler('COMBAT_MSG',function(...) received[#received+1]={n=select('#',...),...} end)
        "#).exec()?;
        for (source,amount,name) in [(0,17,"Faller"),(2,16_777_217,""),(0,u32::MAX,"High"),(0,0,"Zero"),(1,7,"Gimmick"),(3,9,"Ship"),(255,9,"Other")] {
            environment(&rt, &aa_net::EnvironmentDamage { source,target_obj_id:100,amount,gimmick_id:(source==1).then_some(8) }, name);
        }
        rt.lua.load(r#"
            assert(#received==3 and text_count==0)
            for _,v in ipairs(received) do
              assert(v.n==12 and v[1]=='64' and v[2]=='ENVIRONMENTAL_DAMAGE' and v[3]==v[4])
              assert(v[7]=='HEALTH' and v[8]=='HIT' and v[9]==0 and v[10]=='idle' and v[11]==0 and v[12]==false)
            end
            assert(received[1][3]=='Faller' and received[1][5]=='FALLING' and received[1][6]==17)
            assert(received[2][3]=='unknown' and received[2][5]=='DROWNING' and received[2][6]==16777216)
            assert(received[3][6]==-1)
        "#).exec()?;
        Ok(())
    }
}
