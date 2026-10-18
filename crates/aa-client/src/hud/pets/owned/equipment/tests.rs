use super::*;

#[test]
fn native_slot_pack_requires_known_template_and_only_the_four_physical_slots() {
    let mut tables = Tables::default();
    tables.equipment.insert(1, [true, false, true, true]);
    tables.equipment.insert(2, [true, true, false, true]);
    let mut pet = Pet {
        template: 1,
        ..default()
    };
    assert_eq!(
        (0..=29)
            .filter(|slot| equipable(&tables, &pet, *slot))
            .collect::<Vec<_>>(),
        [1, 4, 7]
    );
    pet.template = 2;
    assert_eq!(
        (0..=29)
            .filter(|slot| equipable(&tables, &pet, *slot))
            .collect::<Vec<_>>(),
        [1, 3, 7]
    );
    pet.template = 99;
    assert!(!equipable(&tables, &pet, 1));
}

#[test]
fn received_equipment_events_keep_duplicate_order_one_based_slots_and_owner_timeline_guards() {
    let mut data = HudData::default();
    data.pets.owned.current = Some(Pet {
        object: 201,
        spawn: MateSpawn {
            tl_id: 12,
            ..default()
        },
        ..default()
    });
    let received = |object| Event::EquipmentChanged {
        obj_id: object,
        items: vec![(0, None), (0, None), (6, None), (28, None)],
    };
    assert_eq!(slots(&data, &received(201)), [1, 1, 7]);
    assert!(slots(&data, &received(100)).is_empty());
    let flags = |tl, container, slot| {
        Event::Mate(aa_net::mate::MateEvent::EquipmentFlagsChanged {
            mate_tl_id: tl,
            container,
            slot,
            flags: 1,
        })
    };
    assert_eq!(slots(&data, &flags(12, 0xfc, 27)), [28]);
    for event in [flags(13, 0xfc, 0), flags(12, 2, 0), flags(12, 0xfc, 28)] {
        assert!(slots(&data, &event).is_empty());
    }
    data.pets.owned.current = None;
    assert!(slots(&data, &received(201)).is_empty());
}

#[test]
fn lua_read_getter_keeps_empty_and_unknown_physical_items_and_releases_player_callback_borrow()
-> Result<(), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1920., 1080.)?;
    rt.lua
        .load("X2Equipment={GetEquippedItemTooltipText=function(self) return {itemType=self:IsMateEquipableSlot('playerpet',1) and 765 or 0} end}")
        .exec()?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
    let mut game = crate::hud::data::GameData::default();
    game.pets
        .owned
        .equipment
        .insert(1, [true, false, true, true]);
    assert!(bridge.borrow().game.0.set(game).is_ok());
    bridge.borrow_mut().snapshot.unit_id = "100".into();
    bridge.borrow_mut().pets.owned.current = Some(Pet {
        object: 201,
        template: 1,
        ..default()
    });
    install(&rt.lua, &bridge)?;
    rt.lua
        .load(
            r#"
      assert(X2Equipment:GetEquippedItemTooltipText('player',1).itemType==765)
      assert(X2Equipment:GetEquippedItemTooltipText('100',1).itemType==765)
      local t=X2Equipment:GetEquippedItemTooltipText('playerpet',1);assert(t.itemType==0 and t.lookType==0)
      assert(select('#',X2Equipment:GetEquippedItemTooltipText('999999',1))==0)
      assert(select('#',X2Equipment:GetEquippedItemTooltipText('playerpet',0))==0)
    "#,
        )
        .exec()?;
    bridge
        .borrow_mut()
        .pets
        .owned
        .current
        .as_mut()
        .unwrap()
        .equipment
        .slots[0] = Some(aa_net::model::Item {
        template_id: 0xdead,
        id: u64::MAX,
        grade: 1,
        count: 1,
        ..default()
    });
    rt.lua
        .load("local t=X2Equipment:GetEquippedItemTooltipText('201',1);assert(t.itemType==57005 and t.lookType==57005 and t.equip)")
        .exec()?;
    bridge.borrow_mut().pets.owned.current = None;
    rt.lua
        .load("assert(select('#',X2Equipment:GetEquippedItemTooltipText('playerpet',1))==0 and not X2Equipment:IsMateEquipableSlot('playerpet',1))")
        .exec()?;
    Ok(())
}

#[test]
#[ignore = "loads original current-pet equipment, slot packs, item tooltips and pointer controls"]
fn original_pet_equipment_reads_its_own_physical_items_and_received_updates()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path, player, mut units, mut mates) =
        super::super::super::active_fixture::runtime(1920., 1080.)?;
    let buffs = super::super::super::active_fixture::equipped(
        &rt, &bridge, &player, &mut units, &mut mates, false,
    );
    super::super::super::super::expansion::fixture::click(
        &rt,
        "petBar.command.buttons[PET_CMD_EQUIP]",
    )?;
    rt.lua
        .load(
            r#"
      assert(petBar.equip:IsVisible())
      assert(X2Equipment:IsMateEquipableSlot('playerpet',1) and X2Equipment:IsMateEquipableSlot('201',4))
      assert(not X2Equipment:IsMateEquipableSlot('playerpet',3) and not X2Equipment:IsMateEquipableSlot('player',1))
      assert(petBar.equip.buttons[1]:IsVisible() and petBar.equip.buttons[4]:IsVisible() and petBar.equip.buttons[7]:IsVisible())
      assert(not petBar.equip.buttons[3]:IsVisible())
      local item=X2Equipment:GetEquippedItemTooltipText('playerpet',1)
      assert(item.itemType==23090 and item.lookType==23090 and item.equip and item.soul_bound==1)
      assert(item.durability==42 and item.name=='Shabby Pet Helm' and item.isPetOnly==true)
      petBar.equip.buttons[1]:OnEnter();HideTooltip()
      assert(X2Equipment:GetEquippedItemTooltipText('201',1).itemType==23090)
      local empty=X2Equipment:GetEquippedItemTooltipText('playerpet',3);assert(empty.itemType==0 and empty.lookType==0)
      assert(select('#',X2Equipment:GetEquippedItemTooltipText('999999',1))==0)
      local old=petBar.equip.OnEquipmentChanged;PET_EQUIPMENT_EVENTS={}
      petBar.equip:SetHandler('OnEvent',function(self,event,slot)
        table.insert(PET_EQUIPMENT_EVENTS,{event,slot});old(self)
      end)
    "#,
        )
        .exec()?;
    let icon =
        aa_ui::lua::value_id(&rt.lua, &rt.lua.load("petBar.equip.buttons[1]").eval()?).unwrap();
    let background = aa_ui::lua::ui(&rt.lua)
        .tree
        .widget(icon)
        .unwrap()
        .button
        .as_ref()
        .unwrap()
        .bg[0]
        .unwrap();
    let before = aa_ui::lua::ui(&rt.lua)
        .tree
        .get(background)
        .drawable()
        .unwrap()
        .texture
        .clone();
    units.by_id.get_mut(&201).unwrap().equipment.slots[0] = None;
    let mut world = World::new();
    world.insert_resource(player.clone());
    world.insert_resource(units.clone());
    world.insert_resource(mates.clone());
    world.insert_resource(buffs);
    super::super::super::refresh(&world, &mut bridge.borrow_mut());
    super::super::super::push(&rt, &bridge);
    let event = Event::EquipmentChanged {
        obj_id: 201,
        items: vec![(0, None), (0, None)],
    };
    let notifications = {
        let data = bridge.borrow();
        slots(&data, &event)
    };
    for slot in notifications {
        rt.fire_event("UNIT_NPC_EQUIPMENT_CHANGED", slot);
    }
    rt.lua.load("assert(#PET_EQUIPMENT_EVENTS==2 and PET_EQUIPMENT_EVENTS[1][2]==1 and PET_EQUIPMENT_EVENTS[2][2]==1);assert(X2Equipment:GetEquippedItemTooltipText('playerpet',1).itemType==0)").exec()?;
    assert_ne!(
        aa_ui::lua::ui(&rt.lua)
            .tree
            .get(background)
            .drawable()
            .unwrap()
            .texture,
        before
    );
    rt.lua
        .load("petBar.equip.buttons[1]:OnEnter();HideTooltip()")
        .exec()?;
    let _ = super::super::super::active_fixture::equipped(
        &rt, &bridge, &player, &mut units, &mut mates, true,
    );
    // The original DISMISS_PET handler closes the previous pet's panel. Reopening runs its
    // original slot-pack layout before checking the newly received battle pet.
    rt.lua.load("assert(not petBar.equip:IsVisible())").exec()?;
    super::super::super::super::expansion::fixture::click(
        &rt,
        "petBar.command.buttons[PET_CMD_EQUIP]",
    )?;
    rt.lua.load("assert(X2Equipment:IsMateEquipableSlot('playerpet',3) and not X2Equipment:IsMateEquipableSlot('playerpet',4));assert(petBar.equip.buttons[3]:IsVisible() and not petBar.equip.buttons[4]:IsVisible())").exec()?;
    assert!(rt.errors.is_empty());
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
