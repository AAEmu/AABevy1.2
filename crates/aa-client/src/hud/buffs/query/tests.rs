use super::super::super::{api, panels};
use super::*;
use aa_net::{
    Event,
    buff::{Buff, BuffKind},
    state::Buffs,
};
use aa_ui::UiRuntime;

fn show(rt: &UiRuntime, state: &Shared, buffs: &Buffs) {
    {
        let mut data = state.borrow_mut();
        data.panels.buffs = panels::buff_lists(data.game.get(), buffs.native.of(100).unwrap(), data.start);
    }
    api::push_changes(rt, state);
}

#[test]
fn original_getter_strict_arguments_and_default_explicit_flag_shapes() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path, mut buffs, mut spawn) = super::super::cancel::fixture::runtime(1920., 1080.)?;
    spawn.unit.buffs = buffs.native.of(100).unwrap().iter().map(|entry| entry.buff.clone()).collect();
    spawn.unit.buffs[0].caster_char_id = 47;
    buffs.apply(&Event::PlayerSpawn(Box::new(spawn)));
    show(&rt, &state, &buffs);
    rt.lua
        .load(
            r#"
        local function size(t) local n=0;for _ in pairs(t) do n=n+1 end;return n end
        local all=X2Unit:UnitBuff('player',1)
        assert(size(all)==10 and all.buff_id==127 and all.name=='Inspire' and all.path~='')
        assert(all.tipType=='buff' and all.category==X2Locale:LocalizeUiText(33,'buff'))
        assert(all.mine==true and all.duration==60000 and all.timeLeft>0 and all.timeLeft<=60000 and all.stack==2)
        for _,flag in ipairs({0,'-1',true,false,{},1/0,-1/0,0/0,4294967295}) do assert(size(X2Unit:UnitBuff('player',1,flag))==0) end
        assert(size(X2Unit:UnitBuff('player',1,nil))==0)
        local s=X2Unit:UnitBuff('player',1,16);assert(size(s)==1 and s.stack==2)
        local d=X2Unit:UnitBuff('player',1,1);assert(size(d)==1 and d.description==all.description)
        local m=X2Unit:UnitBuff('player',1,2);assert(size(m)==1 and m.mine==true)
        local m2=X2Unit:UnitBuff('player',2,2);assert(size(m2)==1 and m2.mine==false)
        local t=X2Unit:UnitBuff('player',1,4);assert(size(t)==1 and t.duration==60000)
        local l=X2Unit:UnitBuff('player',1,8);assert(size(l)==1 and l.timeLeft>0)
        assert(size(X2Unit:UnitBuff('player',1,31))==5)
        assert(size(X2Unit:UnitBuff('player',1,-1.9))==10)
        assert(X2Unit:UnitBuff('player',1.9,16).stack==2)
        assert(select('#',X2Unit:UnitBuff('missing',1))==0)
        assert(select('#',X2Unit:UnitBuff(100,1))==0)
        assert(select('#',X2Unit:UnitBuff(nil,1))==0)
        assert(select('#',X2Unit:UnitBuff({},1))==0)
        assert(select('#',X2Unit:UnitBuff('player'))==0)
        for _,value in ipairs({'1',true,false,{},0,-1,99,1/0,-1/0,0/0,-2147483648,2147483648}) do
            assert(select('#',X2Unit:UnitBuff('player',value))==0)
        end
    "#,
        )
        .exec()?;
    let mut mouse = super::super::cancel::fixture::hover(&rt)?;
    aa_ui::lua::tick(&rt.lua, 600.);
    rt.lua.load("local t=tooltip.tooltipWindows[7];assert(t:IsVisible() and t.tipType=='buff','original buff tooltip must be visible');assert(t.buffInfo.buff_id==127 and t.buffInfo.name=='Inspire' and t.buffInfo.duration==60000 and t.buffInfo.timeLeft>0,'full metadata must survive the original time-only refresh')").exec()?;
    aa_ui::input::update(&rt.lua, &mut mouse, Some((0., 0.)), false, false, false, 0.);
    rt.lua.load("assert(not tooltip.tooltipWindows[7]:IsVisible())").exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn physical_count_preserves_missing_descriptors_and_static_category_is_independent_of_wire_group() -> Result<(), Box<dyn std::error::Error>>
{
    let (rt, state, path, mut buffs, mut spawn) = super::super::cancel::fixture::runtime(1920., 1080.)?;
    let bad_id = state
        .borrow()
        .game
        .get()
        .unwrap()
        .buffs
        .iter()
        .filter(|(_, info)| info.kind == 2)
        .map(|(&id, _)| id)
        .min()
        .unwrap();
    spawn.unit.buffs = vec![
        Buff {
            index: 21,
            buff_id: u32::MAX,
            kind: BuffKind::Good,
            ..Default::default()
        },
        Buff {
            index: 22,
            buff_id: bad_id,
            kind: BuffKind::Good,
            ..Default::default()
        },
        Buff {
            index: 23,
            buff_id: bad_id,
            kind: BuffKind::Bad,
            ..Default::default()
        },
    ];
    buffs.apply(&Event::PlayerSpawn(Box::new(spawn)));
    show(&rt, &state, &buffs);
    rt.lua
        .load(
            r#"
        assert(X2Unit:UnitBuffCount('player')==2 and X2Unit:UnitDeBuffCount('player')==1)
        assert(select('#',X2Unit:UnitBuff('player',1))==0)
        local good=X2Unit:UnitBuff('player',2)
        assert(good.tipType=='debuff' and good.category==X2Locale:LocalizeUiText(33,'debuff'))
        local bad=X2Unit:UnitDeBuff('player',1)
        assert(bad.tipType=='debuff' and bad.category==good.category)
        local selected=X2Unit:UnitDeBuff('player',1,16)
        for key in pairs(selected) do assert(key=='stack') end
        assert(select('#',X2Unit:UnitDeBuff('player','1'))==0)
    "#,
        )
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn unsigned_duration_zero_time_omission_and_f32_projection_preserve_received_bits() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path, mut buffs, mut spawn) = super::super::cancel::fixture::runtime(1920., 1080.)?;
    spawn.unit.buffs = vec![
        Buff {
            index: 31,
            buff_id: 1000,
            kind: BuffKind::Good,
            duration_ms: 0,
            ..Default::default()
        },
        Buff {
            index: 32,
            buff_id: 1000,
            kind: BuffKind::Good,
            duration_ms: -1,
            elapsed_ms: 0,
            ..Default::default()
        },
        Buff {
            index: 33,
            buff_id: 127,
            kind: BuffKind::Good,
            duration_ms: 1000,
            elapsed_ms: 300,
            stacks: 0,
            ..Default::default()
        },
    ];
    buffs.apply(&Event::PlayerSpawn(Box::new(spawn)));
    show(&rt, &state, &buffs);
    let now = state.borrow().panels.buffs.good[1].ends_ms.unwrap() - u64::from(u32::MAX);
    rt.lua.globals().set("BUFF_RECEIVED_MS", now)?;
    rt.lua
        .load(
            r#"
        HUD.now_ms=function() return BUFF_RECEIVED_MS end
        local zero=X2Unit:UnitBuff('player',1,12);assert(zero.duration==0 and zero.timeLeft==nil)
        local high=X2Unit:UnitBuff('player',2,12);assert(high.duration==4294967296 and high.timeLeft==4294967296)
        local left=X2Unit:UnitBuff('player',3,8);assert(left.timeLeft==700)
        local noStack=X2Unit:UnitBuff('player',3,16);assert(next(noStack)==nil)
        local full=X2Unit:UnitBuff('player',3);assert(full.stack==nil and full.duration==1000 and full.timeLeft==700)
    "#,
        )
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
