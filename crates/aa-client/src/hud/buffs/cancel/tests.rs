use super::super::super::{
    api,
    data::{BuffInfo, GameData, Named},
    panels,
};
use super::*;
use aa_net::{
    buff::Buff,
    state::{NativeBuffRule, Player},
};
use aa_ui::UiRuntime;
use std::{cell::RefCell, rc::Rc};

fn rule(kind: BuffKind) -> NativeBuffRule {
    NativeBuffRule {
        kind,
        per_unit_creation: false,
        stack_rule_id: 4,
        max_stack: 5,
        max_charge: 0,
    }
}
fn buff(index: u32, template: u32, kind: BuffKind) -> Buff {
    Buff {
        index,
        buff_id: template,
        kind,
        stacks: 1,
        ..default()
    }
}
fn shown(data: &mut HudData, buffs: &Buffs, spawn: &PlayerSpawn) {
    refresh(data, Some(spawn), Some(buffs));
    data.panels.buffs = panels::buff_lists(
        data.game.get(),
        buffs.native.of(spawn.obj_id).unwrap_or_else(|| buffs.of(spawn.obj_id)),
        data.start,
    );
    data.panels_pushed = Some(data.panels.clone());
}
fn setup() -> Result<(UiRuntime, Shared, Buffs, PlayerSpawn), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1920., 1080.)?;
    rt.lua.load("X2Ability={};X2Locale={LocalizeUiText=function(_,_,key) return key end};ERROR_MSG=1;BUFF_ERRORS={};local w=UIParent:CreateWidget('emptywidget','buffErrorProbe','UIParent');w:RegisterEvent('SYSMSG');w:SetHandler('OnEvent',function(_,_,text) BUFF_ERRORS[#BUFF_ERRORS+1]=text end)").exec()?;
    let mut game = GameData::default();
    for (id, kind, system) in [(77, 1, false), (78, 1, true), (79, 2, false)] {
        game.buffs.insert(
            id,
            BuffInfo {
                named: Named {
                    name: format!("Buff{id}"),
                    icon: String::new(),
                },
                desc: String::new(),
                kind,
                system,
                stack_rule: 4,
            },
        );
        game.native_buffs
            .insert(id, rule(if kind == 1 { BuffKind::Good } else { BuffKind::Bad }));
    }
    let mut data = HudData::default();
    data.character_id = 47;
    assert!(data.game.0.set(game).is_ok());
    let mut spawn = super::super::super::expansion::fixture::spawn();
    spawn.unit.buffs = vec![
        buff(0, 77, BuffKind::Good),
        buff(u32::MAX, 78, BuffKind::Good),
        buff(4, 79, BuffKind::Bad),
    ];
    let mut buffs = Buffs::default();
    buffs.native.set_catalog(data.game.get().unwrap().native_buffs.clone());
    buffs.apply(&Event::PlayerSpawn(Box::new(spawn.clone())));
    shown(&mut data, &buffs, &spawn);
    let state = Rc::new(RefCell::new(data));
    install(&rt.lua, &state)?;
    Ok((rt, state, buffs, spawn))
}
fn commands(data: &mut HudData) -> Vec<(u32, u32, u32)> {
    flush(data)
        .into_iter()
        .map(|command| match command {
            Command::RemoveBuff {
                character_id,
                obj_id,
                buff_index,
            } => (character_id, obj_id, buff_index),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn native_numeric_ordinal_system_descriptor_and_zero_result_contract() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, _buffs, _spawn) = setup()?;
    rt.lua
        .load(
            r#"
        assert(select('#',X2Ability:CancelPlayerBuff(1.9))==0)
        for _,value in ipairs({'1',true,false,{},0,-1,4,1/0,-1/0,0/0,-2147483648,2147483648}) do
            assert(select('#',X2Ability:CancelPlayerBuff(value))==0)
        end
        assert(select('#',X2Ability:CancelPlayerBuff())==0)
        assert(select('#',X2Ability:CancelPlayerBuff(2))==0)
        assert(#BUFF_ERRORS==1 and BUFF_ERRORS[1]=='CANNOT_REMOVE_SYSTEM_BUFF')
    "#,
        )
        .exec()?;
    assert_eq!(commands(&mut state.borrow_mut()), [(47, 100, 0)], "instance zero is valid");
    {
        let mut data = state.borrow_mut();
        let mut missing = data.buff_cancel.good.as_ref().unwrap()[0];
        missing.template = 999;
        data.buff_cancel.good.as_mut().unwrap().push(missing);
        data.panels_pushed.as_mut().unwrap().buffs.good.push(panels::BuffSnap {
            id: 999,
            instance_index: 0,
            ..default()
        });
    }
    rt.lua.load("X2Ability:CancelPlayerBuff(3);assert(#BUFF_ERRORS==1)").exec()?;
    assert!(commands(&mut state.borrow_mut()).is_empty());
    Ok(())
}

#[test]
fn native_folded_display_and_captured_instance_survive_without_optimistic_removal() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, mut buffs, mut spawn) = setup()?;
    spawn.unit.buffs.clear();
    buffs.apply(&Event::PlayerSpawn(Box::new(spawn.clone())));
    for index in [9, 10] {
        buffs.apply(&Event::BuffCreated {
            obj_id: 100,
            buff: buff(index, 77, BuffKind::Unknown),
        });
    }
    buffs.apply(&Event::BuffCreated {
        obj_id: 100,
        buff: buff(11, 78, BuffKind::Unknown),
    });
    assert_eq!(buffs.of(100).len(), 3);
    assert_eq!(buffs.native.of(100).unwrap().len(), 2);
    let mut world = World::new();
    world.insert_resource(buffs.clone());
    {
        let mut data = state.borrow_mut();
        data.panels = panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels_pushed = Some(data.panels.clone());
        refresh(&mut data, Some(&spawn), Some(&buffs));
        assert_eq!(
            data.panels
                .buffs
                .good
                .iter()
                .map(|buff| (buff.id, buff.instance_index, buff.stacks))
                .collect::<Vec<_>>(),
            [(77, 10, Some(2.)), (78, 11, Some(1.))]
        );
    }
    rt.lua.load("X2Ability:CancelPlayerBuff(1)").exec()?;
    assert_eq!(commands(&mut state.borrow_mut()), [(47, 100, 10)]);
    assert_eq!(state.borrow().panels.buffs.good.len(), 2);
    assert_eq!(buffs.native.of(100).unwrap().len(), 2);
    buffs.apply(&Event::BuffRemovalRequested {
        character_id: 47,
        obj_id: 100,
        buff_index: 10,
    });
    assert_eq!(
        buffs.native.of(100).unwrap().len(),
        2,
        "a successful write is no removal acknowledgement"
    );
    buffs.apply(&Event::BuffRemoved { obj_id: 100, index: 10 });
    shown(&mut state.borrow_mut(), &buffs, &spawn);
    assert_eq!(state.borrow().panels.buffs.good.len(), 1);
    Ok(())
}

#[test]
fn unknown_native_history_stale_display_rekey_and_actor_change_refuse_requests() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, mut buffs, spawn) = setup()?;
    rt.lua.load("X2Ability:CancelPlayerBuff(1)").exec()?;
    buffs.apply(&Event::BuffCreated {
        obj_id: 100,
        buff: buff(19, 77, BuffKind::Unknown),
    });
    refresh(&mut state.borrow_mut(), Some(&spawn), Some(&buffs));
    assert!(
        commands(&mut state.borrow_mut()).is_empty(),
        "rekeyed lifetime drops an already captured request"
    );
    rt.lua.load("X2Ability:CancelPlayerBuff(1)").exec()?;
    assert!(
        commands(&mut state.borrow_mut()).is_empty(),
        "old displayed ordinal cannot cancel a replacement"
    );
    shown(&mut state.borrow_mut(), &buffs, &spawn);
    rt.lua.load("X2Ability:CancelPlayerBuff(1)").exec()?;
    assert_eq!(commands(&mut state.borrow_mut()), [(47, 100, 19)]);
    let mut incomplete = Buffs::default();
    incomplete.apply(&Event::BuffCreated {
        obj_id: 100,
        buff: buff(20, 77, BuffKind::Unknown),
    });
    refresh(&mut state.borrow_mut(), Some(&spawn), Some(&incomplete));
    rt.lua.load("X2Ability:CancelPlayerBuff(1)").exec()?;
    assert!(commands(&mut state.borrow_mut()).is_empty());
    shown(&mut state.borrow_mut(), &buffs, &spawn);
    rt.lua.load("X2Ability:CancelPlayerBuff(1)").exec()?;
    state.borrow_mut().character_id = 48;
    refresh(&mut state.borrow_mut(), Some(&spawn), Some(&buffs));
    assert!(commands(&mut state.borrow_mut()).is_empty());
    Ok(())
}

#[test]
fn actual_hud_schedule_uses_fresh_native_identity_and_discards_disconnect_capture() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, buffs, spawn) = setup()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(Player {
            spawn: Some(spawn.clone()),
        })
        .insert_resource(buffs)
        .add_message::<NetEvent>()
        .add_message::<NetCommand>();
    plugin(&mut app);
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Ability:CancelPlayerBuff(1)")
        .exec()?;
    app.update();
    let sent = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|command| command.0)
        .collect::<Vec<_>>();
    assert!(matches!(
        sent.as_slice(),
        [Command::RemoveBuff {
            character_id: 47,
            obj_id: 100,
            buff_index: 0
        }]
    ));
    // Even an unchanged stale Player resource cannot reopen cancellation after selection.
    app.world_mut()
        .write_message(NetEvent(Event::CharacterSelected(Box::new(spawn.state.clone()))));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Ability:CancelPlayerBuff(1)")
        .exec()?;
    app.update();
    assert!(state.borrow().buff_cancel.good.is_none());
    assert!(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().next().is_none());
    let receipt = Event::PlayerSpawn(Box::new(spawn.clone()));
    app.world_mut().resource_mut::<Buffs>().apply(&receipt);
    app.world_mut().resource_mut::<Player>().apply(&receipt);
    app.world_mut().write_message(NetEvent(receipt));
    app.update();
    assert!(state.borrow().buff_cancel.good.is_some());
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("X2Ability:CancelPlayerBuff(1)")
        .exec()?;
    let disconnected = Event::Disconnected { message: "fixture".into() };
    app.world_mut().resource_mut::<Buffs>().apply(&disconnected);
    app.world_mut().resource_mut::<Player>().apply(&disconnected);
    app.world_mut().write_message(NetEvent(disconnected));
    app.update();
    assert!(app.world_mut().resource_mut::<Messages<NetCommand>>().drain().next().is_none());
    assert!(state.borrow().buff_cancel.good.is_none());
    Ok(())
}

#[test]
fn original_good_icons_right_click_the_folded_instance_and_preserve_system_buffs() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path, mut buffs, spawn) = fixture::runtime(1920., 1080.)?;
    assert_eq!(buffs.of(100).len(), 4);
    assert_eq!(buffs.native.of(100).unwrap().len(), 3);
    rt.lua.load("BUFF_ERRORS={};local w=UIParent:CreateWidget('emptywidget','originalBuffErrors','UIParent');w:RegisterEvent('SYSMSG');w:SetHandler('OnEvent',function(_,_,text) BUFF_ERRORS[#BUFF_ERRORS+1]=text end)").exec()?;
    super::super::super::expansion::fixture::click(&rt, "BUFF_PLAYER_FRAME.buffWindow.button[1]")?;
    assert!(commands(&mut state.borrow_mut()).is_empty(), "original left-click does not cancel");
    fixture::right_click(&rt, "BUFF_PLAYER_FRAME.buffWindow.button[1]")?;
    assert_eq!(commands(&mut state.borrow_mut()), [(47, 100, 13)]);
    assert_eq!(state.borrow().panels.buffs.good.len(), 3);
    fixture::right_click(&rt, "BUFF_PLAYER_FRAME.buffWindow.button[2]")?;
    assert!(commands(&mut state.borrow_mut()).is_empty());
    rt.lua.load("assert(#BUFF_ERRORS==1);assert(BUFF_ERRORS[1]==X2Locale:LocalizeUiText(ERROR_MSG,'CANNOT_REMOVE_SYSTEM_BUFF'));assert(X2Unit:UnitBuffCount('player')==3)").exec()?;
    buffs.apply(&Event::BuffRemoved { obj_id: 100, index: 13 });
    fixture::present(&mut state.borrow_mut(), &buffs, &spawn);
    api::push_changes(&rt, &state);
    aa_ui::lua::tick(&rt.lua, 100.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua.load("assert(X2Unit:UnitBuffCount('player')==2);assert(BUFF_PLAYER_FRAME.buffWindow.button[1].t.buff_id==1029);assert(BUFF_PLAYER_FRAME.buffWindow.button[2].t.buff_id==1000);assert(not BUFF_PLAYER_FRAME.buffWindow.button[3]:IsVisible())").exec()?;
    fixture::right_click(&rt, "BUFF_PLAYER_FRAME.buffWindow.button[2]")?;
    assert_eq!(commands(&mut state.borrow_mut()), [(47, 100, 14)]);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_count_badges_use_charges_and_follow_received_signed_updates_without_removing_icons() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path, mut buffs, spawn) = fixture::runtime(1920., 1080.)?;
    fixture::charges(&rt, &state, &mut buffs, &spawn)?;
    let received = buffs.native.of(100).unwrap()[0].received;
    for (index, stacks, charged) in [(21, 999, 0), (22, 3, -99), (23, 999, 999)] {
        buffs.apply(&Event::BuffUpdated {
            obj_id: 100,
            index,
            stacks,
            charged,
        });
    }
    assert_eq!(buffs.native.of(100).unwrap()[0].received, received);
    fixture::present(&mut state.borrow_mut(), &buffs, &spawn);
    api::push_changes(&rt, &state);
    aa_ui::lua::tick(&rt.lua, 100.);
    aa_ui::lua::flush(&rt.lua);
    rt.lua.load("assert(X2Unit:UnitBuffCount('player')==3);assert(X2Unit:UnitBuff('player',1).stack==nil);assert(BUFF_PLAYER_FRAME.buffWindow.button[1].stack:GetText()=='');assert(X2Unit:UnitBuff('player',2).stack==3);assert(BUFF_PLAYER_FRAME.buffWindow.button[2].stack:GetText()=='x3');assert(X2Unit:UnitBuff('player',3).stack==1);assert(BUFF_PLAYER_FRAME.buffWindow.button[3].stack:GetText()=='')").exec()?;
    buffs.apply(&Event::BuffUpdated {
        obj_id: 100,
        index: 22,
        stacks: -3,
        charged: 99,
    });
    fixture::present(&mut state.borrow_mut(), &buffs, &spawn);
    api::push_changes(&rt, &state);
    aa_ui::lua::tick(&rt.lua, 100.);
    rt.lua.load("assert(X2Unit:UnitBuffCount('player')==3);assert(X2Unit:UnitBuff('player',2).stack==nil);assert(BUFF_PLAYER_FRAME.buffWindow.button[2].stack:GetText()=='')").exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
