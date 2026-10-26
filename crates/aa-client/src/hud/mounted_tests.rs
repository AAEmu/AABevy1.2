//! Original hidden information-window consumer over explicit loaded/bound receipts.
//! This is a controlled public-method case, not a claim about the mode-action producer.

use aa_net::{Event, model::UnitKind, slave::SlaveEvent};

struct BindingsFile(std::path::PathBuf);

impl Drop for BindingsFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
#[ignore = "requires the original World UI, game pak and read-only game database"]
fn original_bound_slave_header_uses_received_binding_and_hex_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::breath::fixture::runtime(1280., 720.)?;
    let _bindings = BindingsFile(path);
    let own = super::expansion::fixture::spawn();
    let mut bound = own.unit.clone();
    bound.obj_id = 0x9abc;
    bound.name = "Received bound catapult".into();
    bound.kind = UnitKind::Slave {
        id: 700,
        tl_id: 8,
        template_id: 1, // Original slaves row: Catapult, siege_weapon.
        summoner_id: 100,
    };
    let mut owned = bound.clone();
    owned.obj_id = 0x1234;
    owned.name = "Owned announcement is a different object".into();
    owned.kind = UnitKind::Slave {
        id: 701,
        tl_id: 9,
        template_id: 2,
        summoner_id: 100,
    };
    for event in [
        Event::CharacterSelected(Box::new(own.state.clone())),
        Event::PlayerSpawn(Box::new(own)),
        Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        Event::UnitSpawned(Box::new(owned)),
        Event::Slave(SlaveEvent::Created(aa_net::slave::SlaveCreated {
            owner_obj_id: 100,
            tl_id: 9,
            obj_id: 0x1234,
            hide_spawn_effect: true,
            raw_id: 0,
            creator_name: String::new(),
        })),
        Event::UnitSpawned(Box::new(bound)),
        Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id: 0x9abc,
        }),
    ] {
        super::enchant::dispatch(Some(&rt), &state, &event, None);
    }
    rt.lua
        .load(
            r#"
        assert(slaveInfoFrame and not slaveInfoFrame:IsVisible(), 'original hidden window')
        local original = slaveInfoFrame.UpdateSlaveDefaultInfo
        local source = debug.getinfo(original, 'S')
        assert(source.what == 'Lua' and source.linedefined == 7, 'original Lua consumer')
        original(slaveInfoFrame)
        assert(slaveInfoFrame.UpdateSlaveDefaultInfo == original)
        assert(slaveInfoFrame.unitId == '9abc', 'received bound reference, not owned announcement')
        assert(slaveInfoFrame.name:GetText() == 'Received bound catapult')
        assert(slaveInfoFrame.class:GetText() == 'Catapult')
        assert(slaveInfoFrame.kind:GetText() == locale.map.slaveKind('siege_weapon'))
        assert(not slaveInfoFrame:IsVisible(), 'method case does not invent SHOW_SLAVE_INFO')
    "#,
        )
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok(())
}

fn received_part(uid: u64, grade: u8) -> aa_net::model::Item {
    let mut wire = aa_net::wire::Writer::new();
    wire.u32(29689)
        .u64(uid)
        .u8(grade)
        .u8(0)
        .i32(1)
        .u8(9)
        .u32(0)
        .i64(0)
        .i32(0)
        .u32(0)
        .u8(0)
        .i64(0)
        .i64(0);
    let bytes = wire.finish();
    let mut reader = aa_net::wire::Reader::new(&bytes);
    let item = aa_net::model::Item::read(&mut reader).unwrap().unwrap();
    assert!(reader.is_empty());
    assert_eq!(item.id, uid);
    item
}

#[test]
#[ignore = "requires original World/slave UI, game pak and read-only game database"]
fn original_equipment_rows_read_receipts_and_refresh_even_after_failed_reports()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::breath::fixture::runtime(1280., 720.)?;
    let _bindings = BindingsFile(path);
    let own = super::expansion::fixture::spawn();
    let mut slave = own.unit.clone();
    slave.obj_id = 0x9abc;
    slave.name = "Received clipper".into();
    slave.kind = UnitKind::Slave {
        id: 700,
        tl_id: 8,
        template_id: 14,
        summoner_id: 100,
    };
    // All are typed equipment values, not a claim about the currently held
    // original-kind2 Unit::read exception or a live packet/gameplay run.
    slave.equipment.slots[13] = Some(received_part(9_007_199_254_740_993, 4));
    slave.equipment.slots[23] = Some(received_part(9_007_199_254_740_994, 3));
    slave.equipment.slots[4] = Some(received_part(9_007_199_254_740_995, 2));
    let mut features = vec![0; 7];
    features[6] = 1;
    let original_config = aa_net::InitialConfig {
        host: Vec::new(),
        feature_bytes: features,
        raw_candidate_count: 0,
        candidate_codes: Vec::new(),
        initial_lp: 0,
        service_flags: [0; 7],
        second_price_type: 0,
        second_password_max_fail_count: 0,
        idle_kick_time: 0,
    };
    for event in [
        Event::InitialConfig(original_config.clone()),
        Event::CharacterSelected(Box::new(own.state.clone())),
        Event::PlayerSpawn(Box::new(own)),
        Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        Event::UnitSpawned(Box::new(slave)),
        Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id: 0x9abc,
        }),
    ] {
        super::enchant::dispatch(Some(&rt), &state, &event, None);
    }
    rt.lua.load(r#"
        assert(not slaveInfoFrame:IsVisible())
        local original=slaveInfoFrame.UpdateSlaveDefaultInfo
        original(slaveInfoFrame)
        assert(slaveInfoFrame.UpdateSlaveDefaultInfo==original)
        assert(slaveInfoFrame.customizingType==2,'received feature and descriptor select original equipment style')
        assert(slaveInfoFrame.equipmentWnd:GetHeight()>0)
        function MOUNTED_ROW(slot)
          for _,w in ipairs(slaveInfoFrame.equipmentWnd.equipWnd.slots) do
            if w.slotType==slot then return w end
          end
          error('missing original equipment row '..tostring(slot))
        end
        local lamp=MOUNTED_ROW(13)
        local source=debug.getinfo(lamp.Update,'S')
        assert(source.what=='Lua' and source.linedefined==127,'original equipment consumer')
        MOUNTED_ORIGINAL_UPDATE=lamp.Update
        assert(lamp.info.itemType==29689 and lamp.info.itemGrade==4)
        assert(MOUNTED_ROW(23).info.itemGrade==3)
        local unconfigured=X2SiegeWeapon:GetSlaveEquipSlotInfo(4)
        assert(unconfigured.has==false and unconfigured.itemInfo.itemType==29689)
        assert(MOUNTED_ROW(4).info==nil,'original row rejects unconfigured storage')
        MOUNTED_REPORTS={}
        local observer=CreateEmptyWindow('mountedReportObserver','UIParent')
        local events={SLAVE_UNIT_EQUIPMENT_CHANGED=function(a,b)
          MOUNTED_REPORTS[#MOUNTED_REPORTS+1]={a,b}
        end}
        observer:SetHandler('OnEvent',function(_,event,...) events[event](...) end)
        RegistUIEvent(observer,events)
    "#).exec()?;
    super::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::EquipmentChanged {
            obj_id: 0x9abc,
            items: vec![
                (13, None),
                (23, Some(received_part(9_007_199_254_740_996, 5))),
            ],
        },
        None,
    );
    rt.lua
        .load("assert(MOUNTED_ROW(13).info.itemGrade==4 and MOUNTED_ROW(23).info.itemGrade==3)")
        .exec()?;
    // Native394E1DA0 neither checks these header identities nor gates its event
    // on success. Its report items must not overwrite the selected slave's gear.
    let change = aa_net::SlaveEquipmentChange {
        character_id: 999,
        slave_tl_id: 999,
        db_slave_id: 999,
        bts: false,
        first_item: Some(received_part(1, 1)),
        second_item: Some(received_part(2, 1)),
        first_location: aa_net::SlaveItemLocation {
            container: 2,
            slot: 33,
        },
        second_location: aa_net::SlaveItemLocation {
            container: 0xf2,
            slot: 13,
        },
        success: false,
    };
    super::enchant::dispatch(
        Some(&rt),
        &state,
        &Event::Slave(SlaveEvent::EquipmentChanged(change.clone())),
        None,
    );
    rt.lua.load(r#"
        assert(#MOUNTED_REPORTS==1 and MOUNTED_REPORTS[1][1]==-1 and MOUNTED_REPORTS[1][2]==13)
        assert(MOUNTED_ROW(13).info==nil,'original failed-report handler rereads authoritative empty slot')
        assert(MOUNTED_ROW(23).info.itemGrade==3,'unmentioned original row remains unchanged')
        assert(X2SiegeWeapon:GetSlaveEquipSlotInfo(23).itemInfo.itemGrade==5)
        assert(MOUNTED_ROW(13).Update==MOUNTED_ORIGINAL_UPDATE)
    "#).exec()?;
    let mut change = change;
    change.first_location = aa_net::SlaveItemLocation {
        container: 0xf2,
        slot: 23,
    };
    change.second_location = aa_net::SlaveItemLocation {
        container: 2,
        slot: 4,
    };
    for _ in 0..2 {
        super::enchant::dispatch(
            Some(&rt),
            &state,
            &Event::Slave(SlaveEvent::EquipmentChanged(change.clone())),
            None,
        );
    }
    rt.lua
        .load(
            r#"
        assert(#MOUNTED_REPORTS==3)
        assert(MOUNTED_REPORTS[2][1]==23 and MOUNTED_REPORTS[2][2]==-1)
        assert(MOUNTED_REPORTS[3][1]==23 and MOUNTED_REPORTS[3][2]==-1)
        assert(MOUNTED_ROW(23).info.itemGrade==5,'report payload did not fabricate gear settlement')
        assert(not slaveInfoFrame:IsVisible(),'no renderer/show/motion claim')
    "#,
        )
        .exec()?;
    let mut short = original_config;
    short.feature_bytes.truncate(6);
    super::enchant::dispatch(Some(&rt), &state, &Event::InitialConfig(short), None);
    rt.lua.load(r#"
        slaveInfoFrame:UpdateSlaveDefaultInfo()
        assert(slaveInfoFrame.customizingType==nil and slaveInfoFrame.equipmentWnd:GetHeight()==0)
        assert(X2SiegeWeapon:GetSlaveEquipSlotInfo(23).itemInfo.itemGrade==5,'slot query itself has no feature gate')
    "#).exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok(())
}
