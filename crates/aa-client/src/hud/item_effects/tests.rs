use super::*;
use aa_net::{
    item_effect::{GradeEnchantBroadcast, GradeEnchantResult},
    items::{ItemTask, ItemTasks, container},
};
use std::{cell::RefCell, rc::Rc};

fn state() -> Shared {
    Rc::new(RefCell::new(super::super::api::HudData::default()))
}
fn change(grade: u8) -> Event {
    Event::ItemsChanged(ItemTasks {
        tasks: vec![ItemTask::Grade {
            container: container::INVENTORY,
            slot: 0,
            item_id: fixture::item().id,
            grade,
        }],
        ..default()
    })
}
fn grade(code: u8) -> GradeEnchantResult {
    GradeEnchantResult {
        raw_result: code,
        item: Some(fixture::item()),
        item_wire: Vec::new(),
        before_grade: 1,
        after_grade: 2,
    }
}
fn socket() -> Event {
    Event::LunagemSocketingResult {
        raw_result: 1,
        item_id: fixture::item().id,
        template_id: 5,
        raw_install: 1,
    }
}

#[test]
fn native_grade_helpers_keep_raw_low_byte_strict_types_and_zero_result_misses() -> Result<(), Box<dyn std::error::Error>> {
    let lua = mlua::Lua::new();
    lua.globals().set("X2Item", lua.create_table()?)?;
    let state = state();
    let mut game = super::super::data::GameData::default();
    game.grades.insert(1, ("Basic".into(), "FFFFFFFF".into()));
    game.grades.insert(0, ("Crude".into(), "FF888888".into()));
    state.borrow().game.0.set(game).ok().unwrap();
    install(&lua, &state)?;
    lua.load(
        r#"
      assert(X2Item:GradeName(0)=='Basic' and X2Item:GradeName(1)=='Crude')
      assert(X2Item:GradeColor(0)=='FFFFFFFF' and X2Item:GradeColor(1)=='FF888888')
      assert(X2Item:GradeName(256)=='Basic' and X2Item:GradeName(257.9)=='Crude')
      assert(X2Item:GradeName(-255)=='Crude' and X2Item:GradeName(0/0)=='Basic' and X2Item:GradeName(1/0)=='Basic')
      assert(select('#',X2Item:GradeName(2))==0 and select('#',X2Item:GradeColor(255))==0)
      assert(select('#',X2Item:GradeName())==0 and select('#',X2Item:GradeColor(nil))==0)
      for _,value in ipairs({'0',true,false,{}}) do
        assert(select('#',X2Item:GradeName(value))==0 and select('#',X2Item:GradeColor(value))==0)
      end
    "#,
    )
    .exec()?;
    Ok(())
}

#[test]
fn native_current_lookup_requires_actual_spawn_equipment_first_and_bag_only() {
    let state = state();
    fixture::seed(&state);
    let mut data = state.borrow_mut();
    let view = &mut data.item_effects;
    assert!(view.receive(&socket()).is_some());
    let mut equipped = fixture::item();
    equipped.grade = 4;
    view.inventory.equipment.slots[0] = Some(equipped.clone());
    assert!(
        matches!(view.receive(&socket()), Some(Notice::Socket {link, ..}) if link == super::super::itemlinks::link(&equipped).unwrap())
    );
    view.inventory.equipment.slots[0] = None;
    view.inventory.bank[0] = view.inventory.bag[0].take();
    assert!(view.receive(&socket()).is_none(), "bank is outside native397ccfb0 lookup");
    view.inventory.bag[0] = view.inventory.bank[0].take();
    view.inventory.bag[0].as_mut().unwrap().id = 0;
    assert!(
        view.current(0).is_none(),
        "first empty equipment ID0 cannot fall through to a bag item"
    );
    view.inventory.equipment.slots[0] = view.inventory.bag[0].clone();
    assert!(view.current(0).is_some(), "a genuine first equipment ID0 is not rejected by value");
    view.inventory.equipment.slots[0] = None;
    view.inventory.bag[0].as_mut().unwrap().id = fixture::item().id;
    assert!(
        view.receive(&Event::UnhandledGamePacket {
            level: 2,
            opcode: 0x3ff,
            len: 1
        })
        .is_none()
    );
    assert!(view.receive(&socket()).is_some());
    view.receive(&Event::UnhandledGamePacket {
        level: 4,
        opcode: 0x3ff,
        len: 1,
    });
    assert!(view.receive(&socket()).is_none());
    drop(data);
    fixture::seed(&state);
    let mut data = state.borrow_mut();
    let view = &mut data.item_effects;
    assert!(view.receive(&socket()).is_some());
    view.receive(&Event::CharacterSelected(Box::new(super::super::expansion::fixture::spawn().state)));
    assert!(view.receive(&socket()).is_none(), "selection cannot retain an old active Game");
    assert!(view.pending.is_empty());
}

#[test]
fn actual_receipt_order_formats_pre_or_post_change_item_without_mutating_received_inventory() {
    let state = state();
    fixture::seed(&state);
    let mut data = state.borrow_mut();
    let view = &mut data.item_effects;
    let old = super::super::itemlinks::link(&fixture::item()).unwrap();
    let magical = Event::LunastoneSocketingResult {
        raw_result: 1,
        item_id: fixture::item().id,
        template_id: 7,
    };
    assert!(matches!(view.receive(&magical), Some(Notice::Magical { link, .. }) if link == old));
    assert_eq!(view.inventory.bag[0].as_ref().unwrap().grade, 1);
    view.receive(&change(3));
    let mut new = fixture::item();
    new.grade = 3;
    let updated = super::super::itemlinks::link(&new).unwrap();
    assert_ne!(updated, old);
    assert!(matches!(view.receive(&socket()), Some(Notice::Socket { link, .. }) if link == updated));
    assert_eq!(view.inventory.bag[0].as_ref().unwrap().grade, 3);
    let inline = Event::GradeEnchantResult(grade(0));
    assert!(
        matches!(view.receive(&inline), Some(Notice::Grade { link, .. }) if link == old),
        "inline item wins over the different current item"
    );
    assert_eq!(view.inventory.bag[0].as_ref().unwrap().grade, 3);
}

#[test]
fn raw_result_domains_invalid_items_numeric_width_and_cstring_boundaries_remain_distinct() {
    let mut view = View::default();
    for code in 0..=4 {
        assert!(view.receive(&Event::GradeEnchantResult(grade(code))).is_some());
    }
    assert!(view.receive(&Event::GradeEnchantResult(grade(5))).is_none());
    let mut invalid = grade(0);
    invalid.item.as_mut().unwrap().template_id = 0;
    assert!(view.receive(&Event::GradeEnchantResult(invalid)).is_none());
    let mut invalid = grade(0);
    invalid.item.as_mut().unwrap().detail = aa_net::model::ItemDetail::Other(2);
    assert!(view.receive(&Event::GradeEnchantResult(invalid)).is_none());
    let binary = GradeEnchantBroadcast {
        raw_name: vec![255, b'A', 0, b'B'],
        result: grade(255),
    };
    assert!(
        matches!(view.receive(&Event::GradeEnchantBroadcast(binary)), Some(Notice::Broadcast {name, result:255, ..}) if name == [255,b'A'])
    );
    assert!(
        view.receive(&Event::GradeEnchantBroadcast(GradeEnchantBroadcast {
            raw_name: vec![b'A'; 129],
            result: grade(255)
        }))
        .is_none()
    );
    let state = state();
    fixture::seed(&state);
    let mut data = state.borrow_mut();
    let view = &mut data.item_effects;
    assert!(matches!(
        view.receive(&Event::LunagemSocketingResult {
            raw_result: 255,
            item_id: fixture::item().id,
            template_id: u32::MAX,
            raw_install: 0
        }),
        Some(Notice::Socket {
            success: false,
            template: 4294967296.,
            install: false,
            ..
        })
    ));
    assert!(
        view.receive(&Event::LunagemSocketingResult {
            raw_result: 1,
            item_id: fixture::item().id,
            template_id: 1,
            raw_install: 2
        })
        .is_none()
    );
    assert!(
        view.receive(&Event::LunastoneSocketingResult {
            raw_result: 2,
            item_id: fixture::item().id,
            template_id: 1
        })
        .is_none()
    );
    view.receive(&Event::Disconnected { message: "fixture".into() });
    assert!(view.receive(&socket()).is_none());
    assert!(
        view.receive(&Event::GradeEnchantResult(grade(0))).is_some(),
        "native inline handler has no Game lookup"
    );
}

#[test]
fn lua_callback_types_arity_binary_names_and_deferred_lifecycle_follow_actual_hud_schedule() -> Result<(), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 800., 600.)?;
    rt.exec("@item_observer", r#"
      seen={};local w=UIParent:CreateWidget('emptywidget','itemObserver','UIParent')
      for _,event in ipairs({'GRADE_ENCHANT_RESULT','GRADE_ENCHANT_BROADCAST','ITEM_SOCKETING_RESULT','ITEM_ENCHANT_MAGICAL_RESULT'}) do w:RegisterEvent(event) end
      w:SetHandler('OnEvent',function(_,event,...) seen[#seen+1]={event=event,n=select('#',...),...} end)
    "#)?;
    let state = state();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt::default());
    plugin(&mut app);
    app.world_mut().write_message(NetEvent(Event::GradeEnchantResult(grade(4))));
    app.update();
    assert_eq!(state.borrow().item_effects.pending.len(), 1);
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = true;
    app.update();
    app.world_mut()
        .write_message(NetEvent(Event::GradeEnchantBroadcast(GradeEnchantBroadcast {
            raw_name: vec![255, b'A', 0, b'B'],
            result: grade(255),
        })));
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(#seen==2);assert(seen[1].event=='GRADE_ENCHANT_RESULT' and seen[1].n==4 and seen[1][1]==4 and type(seen[1][2])=='string');assert(seen[2].event=='GRADE_ENCHANT_BROADCAST' and seen[2].n==5 and #seen[2][1]==2 and string.byte(seen[2][1])==255 and seen[2][2]==255)").exec()?;
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = false;
    app.world_mut().write_message(NetEvent(Event::GradeEnchantResult(grade(0))));
    app.world_mut()
        .write_message(NetEvent(Event::Disconnected { message: "fixture".into() }));
    app.update();
    assert!(state.borrow().item_effects.pending.is_empty());
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = true;
    fixture::seed(&state);
    app.world_mut().write_message(NetEvent(socket()));
    app.world_mut().write_message(NetEvent(Event::LunastoneSocketingResult {
        raw_result: 0,
        item_id: fixture::item().id,
        template_id: 9,
    }));
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(#seen==4);assert(seen[3].event=='ITEM_SOCKETING_RESULT' and seen[3].n==4 and seen[3][1]==true and seen[3][3]==5 and seen[3][4]==true);assert(seen[4].event=='ITEM_ENCHANT_MAGICAL_RESULT' and seen[4].n==3 and seen[4][1]==false)").exec()?;
    Ok(())
}

#[test]
fn original_grade_and_socket_alarm_scripts_accept_received_callbacks() -> Result<(), Box<dyn std::error::Error>> {
    for mode in [
        "grade",
        "grade-break",
        "grade-downgrade",
        "grade-fail",
        "grade-great",
        "socket",
        "socket-fail",
        "magical",
    ] {
        let (rt, state, path) = fixture::runtime(1920., 1080.)?;
        rt.lua.globals().set(
            "RESULT_ITEM_NAME",
            state.borrow().game.get().unwrap().items[&fixture::item().template_id]
                .named
                .name
                .clone(),
        )?;
        dispatch(Some(&rt), &state, &fixture::result(&state, mode));
        aa_ui::lua::tick(&rt.lua, 600.);
        aa_ui::lua::flush(&rt.lua);
        rt.lua
            .load(if mode.starts_with("grade") {
                "assert(gradeEnchantAlarm:IsVisible());local text=gradeEnchantAlarm.squareFrame.itemName:GetText();assert(string.find(text,RESULT_ITEM_NAME,1,true),'grade item name='..tostring(text)..' expected='..tostring(RESULT_ITEM_NAME))"
            } else {
                "assert(etcEnchantAlarm:IsVisible());assert(etcEnchantAlarm.topText:GetText()~='' and etcEnchantAlarm:GetWidth()>55)"
            })
            .exec()?;
        if mode.starts_with("grade") {
            fixture::confirm(&rt)?;
            rt.lua.load("assert(not gradeEnchantAlarm:IsVisible())").exec()?;
        }
        assert!(rt.errors.is_empty(), "{mode}: {:?}", rt.errors);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
