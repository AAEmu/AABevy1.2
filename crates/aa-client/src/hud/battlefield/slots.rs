//! Native slot244 identity/tag/enable gates, composed with the client's existing ordinary skill checks.
//! Protected native ordinary eligibility and actor-planner click behavior remain separate open work.
use super::*;

pub(in crate::hud) fn refresh(world: &World, data: &mut HudData) {
    let game = data.game.get();
    let skills = world.get_resource::<aa_net::state::Skills>();
    let inventory = world.get_resource::<aa_net::state::Inventory>();
    let buffs = world
        .get_resource::<aa_net::state::Buffs>()
        .map_or(&[][..], |buffs| buffs.of(skills.map_or(0, |s| s.obj_id)));
    let env = super::super::usable::Env::of(data, skills, buffs, inventory);
    let mut slots = std::collections::HashMap::new();
    if let Some(joined) = data.battlefield.received.joined {
        for skill in joined.killstreak_skills {
            if skill != 0 && game.is_some_and(|game| game.battlefield.killstreak_skills.contains(&skill)) {
                let mut slot = super::super::panels::skill_slot(game, skills, skill, data.start);
                slot.blocked = env.slot_blocked(&slot);
                slots.insert(skill, slot);
            }
        }
    }
    if data.battlefield.ordinary_slots != slots {
        data.battlefield.slots_dirty = true;
        data.battlefield.ordinary_slots = slots;
    }
}

pub(super) fn push(rt: &UiRuntime, bridge: &Shared, force: bool) {
    let changed = std::mem::take(&mut bridge.borrow_mut().battlefield.slots_dirty);
    if (changed || force)
        && let Err(error) = rt.refresh_slots()
    {
        warn!("hud: refresh battlefield slots: {error}");
    }
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    let state = bridge.clone();
    hud.set(
        "battlefield_slot",
        lua.create_function(move |lua, index: i32| {
            let data = state.borrow();
            let received = &data.battlefield.received;
            let Some(joined) = received.joined else { return Ok(None) };
            let Some(index) = index.checked_sub(1).and_then(|i| usize::try_from(i).ok()).filter(|i| *i < 24) else {
                return Ok(None);
            };
            let skill = joined.killstreak_skills[index];
            let Some(game) = data.game.get().filter(|game| game.battlefield.killstreak_skills.contains(&skill)) else {
                return Ok(None);
            };
            if skill == 0 || !game.skills.contains_key(&skill) {
                return Ok(None);
            }
            let fallback = super::super::panels::skill_slot(Some(game), None, skill, data.start);
            let ordinary = data.battlefield.ordinary_slots.get(&skill);
            let out = super::super::panels::slot_table(lua, ordinary.unwrap_or(&fallback))?;
            // Native scans from row0 for the first matching ID, including duplicate skill IDs.
            let enabled = received.in_instant_game()
                && joined
                    .killstreak_skills
                    .iter()
                    .position(|id| *id == skill)
                    .is_some_and(|i| received.skills_enabled[i]);
            out.set("canUse", enabled && ordinary.is_some_and(|slot| slot.blocked.is_none()))?;
            Ok(Some(out))
        })?,
    )?;
    lua.load(
        r#"
        local content=AAUI.SlotContent
        AAUI.SlotContent=function(kind,index)
          if kind==ISLOT_INSTANT_KILL_STREAK then
            local row=HUD.battlefield_slot(index)
            if row then
              local result={type='skill',id=row.id,icon=row.icon,canUse=row.canUse,tooltip=HUD.skill_tip(row.id)}
              if row.cd_end then
                local remaining=row.cd_end-HUD.now_ms()
                if remaining>0 then result.cooldown={remain=remaining,total=math.max(row.cd_total,remaining)} end
              end
              return result
            end
            return nil
          end
          return content(kind,index)
        end
    "#,
    )
    .set_name("@battlefield_slots.lua")
    .exec()
}
