//! Pet actor statistics never reuse the player's attribute snapshot or fake character table.
use super::*;

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    for (name, modifier) in [("pet_info", false), ("pet_modifiers", true)] {
        let state = bridge.clone();
        hud.set(
            name,
            lua.create_function(move |lua, ()| {
                let data = state.borrow();
                let Some(info) = data.pets.owned.current.as_ref().and_then(|pet| pet.info.as_ref()) else {
                    return Ok(MultiValue::new());
                };
                let out = lua.create_table()?;
                if modifier {
                    for ((key, _, _), value) in super::super::super::stats::MATE_MODIFIERS.into_iter().zip(info.modifiers) {
                        out.set(key, f64::from(value))?;
                    }
                } else {
                    for (key, value) in super::super::super::stats::MATE_VALUE_KEYS.into_iter().zip(info.values) {
                        out.set(key, f64::from(value))?;
                    }
                }
                Ok(MultiValue::from_iter([Value::Table(out)]))
            })?,
        )?;
    }
    lua.load(
        r#"
      local info,modifiers=X2Unit.UnitInfo,X2Unit.UnitModifierInfo
      local function pet(key)return key=='playerpet' or(HUD.playerpet and key==HUD.playerpet.unit_id)end
      X2Unit.UnitInfo=function(self,key)
        if pet(key)then return HUD.pet_info()end
        if info then return info(self,key)end
      end
      X2Unit.UnitModifierInfo=function(self,key)
        if pet(key)then return HUD.pet_modifiers()end
        if modifiers then return modifiers(self,key)end
      end
    "#,
    )
    .set_name("@pet_info.lua")
    .exec()
}

pub(super) fn push(rt: &UiRuntime, bridge: &Shared) {
    let known = bridge.borrow().pets.owned.current.as_ref().is_some_and(|pet| pet.info.is_some());
    let result = (|| -> mlua::Result<()> {
        let hud: Table = rt.lua.globals().get("HUD")?;
        hud.set("pet_info_available", known)?;
        rt.lua
            .load(
                r#"
          if petBar then
            if petBar.command and petBar.command.buttons[PET_CMD_EQUIP] then
              petBar.command.buttons[PET_CMD_EQUIP]:Enable(HUD.pet_info_available)
            end
            if petBar.equip and petBar.equip:IsVisible()then
              if HUD.pet_info_available then petBar.equip.attributes:UpdateInfoView()
              else petBar.equip:Show(false)end
            end
          end
        "#,
            )
            .exec()
    })();
    if let Err(error) = result {
        warn!("hud: pet information refresh: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "loads original pet information window and physical equipment/info toggle"]
    fn original_pet_information_panel_uses_own_actor_attributes_and_modifier_keys() -> Result<(), Box<dyn std::error::Error>> {
        let (rt, bridge, path, player, mut units, mut mates) = crate::hud::pets::active_fixture::runtime(1920., 1080.)?;
        rt.lua
            .load("assert(select('#',X2Unit:UnitInfo('playerpet'))==0 and select('#',X2Unit:UnitModifierInfo('playerpet'))==0)")
            .exec()?;
        crate::hud::expansion::fixture::click(&rt, "petBar.command.buttons[PET_CMD_EQUIP]")?;
        rt.lua
            .load("assert(not petBar.equip:IsVisible(),'unknown actor statistics must not show player fallback or open a broken panel')")
            .exec()?;
        let _buffs = crate::hud::pets::active_fixture::native(&rt, &bridge, &player, &mut units, &mut mates);
        rt.lua
            .load(
                r#"
          local info=X2Unit:UnitInfo('playerpet');assert(info and info.armor==0 and info.magic_resist==52 and info.health_regen==10)
          local other=X2Unit:UnitInfo('201');assert(other.melee_dps==info.melee_dps)
          local modifiers=X2Unit:UnitModifierInfo('playerpet');local count=0
          for k,v in pairs(modifiers)do count=count+1;assert(v==0)end;assert(count==17)
        "#,
            )
            .exec()?;
        crate::hud::expansion::fixture::click(&rt, "petBar.command.buttons[PET_CMD_EQUIP]")?;
        aa_ui::lua::tick(&rt.lua, 20.);
        rt.lua
            .load(
                r#"
          assert(petBar.equip:IsVisible())
          local labels=petBar.equip.attributes.labels
          assert(#labels==11 and labels[4].value:GetText()=='29.20')
          assert(labels[7].value:GetText()=='0 (0.00%)')
          assert(labels[8].value:GetText()=='52 (0.97%)')
          assert(labels[9].value:GetText()=='10' and labels[10].value:GetText()=='9')
        "#,
            )
            .exec()?;
        let _buffs = crate::hud::pets::active_fixture::modified(&rt, &bridge, &player, &mut units, &mates);
        rt.lua
            .load(
                r#"
          local labels=petBar.equip.attributes.labels
          assert(labels[7].value:GetText()=='122 (2.25%)')
          assert(labels[9].value:GetText()=='18')
          for _,index in ipairs({7,9})do
            local r,g,b,a=labels[index].value.style:GetColor()
            assert(math.abs(r-FONT_COLOR.GREEN[1])<0.00001 and math.abs(g-FONT_COLOR.GREEN[2])<0.00001 and math.abs(b-FONT_COLOR.GREEN[3])<0.00001)
          end
        "#,
            )
            .exec()?;
        assert!(rt.errors.is_empty());
        drop(rt);
        drop(bridge);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
