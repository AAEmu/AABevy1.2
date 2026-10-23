//! Lazy slot tooltip providers: original callers, result semantics and lifetime.
#[path = "../examples/native_slot_tooltip.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, input, lua, tree::Id};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@slot_tooltip_test", code).unwrap();
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        calls=0
        content={type='item',id=28051,extraInfo={marker=1}}
        AAUI.SlotContent=function() return content end
        slot=UIParent:CreateWidget('slot','tooltipSlot','UIParent')
        slot:EstablishSlot(ISLOT_ACTION,1)
    "#,
    );
    ui
}

#[test]
fn existing_table_identity_callable_tables_and_nil_arity_are_preserved() {
    let ui = setup();
    run(
        &ui,
        r#"
        tip=setmetatable({name='kept'},{__call=function() error('table invoked') end})
        content.tooltip=tip
        assert(rawequal(slot:GetTooltip(),tip))
        assert(select('#',slot:GetTooltip())==1)
        tip.name='mutated';assert(slot:GetTooltip().name=='mutated')
        content.tooltip=nil
        assert(slot:GetTooltip()==nil and select('#',slot:GetTooltip())==1)
    "#,
    );
}

#[test]
fn binding_copy_refresh_and_frames_do_not_invoke_any_source_provider() {
    let ui = setup();
    run(
        &ui,
        r#"
        function provider(...) assert(select('#',...)==0);calls=calls+1;return {name='fresh'} end
        content.tooltip=provider
        function copy()
            local c={};for k,v in pairs(content) do c[k]=v end;return c
        end
        AAUI.SlotContent=copy;AAUI.SkillContent=copy;AAUI.ItemContent=copy
        skill=UIParent:CreateWidget('slot','tooltipSkill','UIParent')
        item=UIParent:CreateWidget('slot','tooltipItem','UIParent')
        slot:EstablishSlot(ISLOT_ACTION,1);skill:EstablishSkill(7);item:EstablishItem(28051,1)
        assert(slot:GetBindedType()=='item' and slot:GetExtraInfo().marker==1)
        slot:InheritAnimationData(item)
        assert(calls==0)
    "#,
    );
    ui.refresh_slots().unwrap();
    for _ in 0..100 {
        lua::tick(&ui.lua, 16.);
    }
    run(
        &ui,
        r#"
        assert(calls==0)
        assert(slot:GetTooltip().name=='fresh' and skill:GetTooltip().name=='fresh' and item:GetTooltip().name=='fresh')
        assert(calls==3)
        assert(not rawequal(slot:GetTooltip(),slot:GetTooltip()) and calls==5)
    "#,
    );
}

#[test]
fn provider_nil_zero_results_errors_and_invalid_results_do_not_poison_next_query() {
    let ui = setup();
    run(
        &ui,
        r#"
        content.tooltip=function(...) assert(select('#',...)==0);return nil end
        assert(slot:GetTooltip()==nil and select('#',slot:GetTooltip())==1)
        content.tooltip=function() end
        assert(slot:GetTooltip()==nil and select('#',slot:GetTooltip())==1)
        content.tooltip=function() error('score provider failed') end
        local ok,err=pcall(function() return slot:GetTooltip() end)
        assert(not ok and string.find(tostring(err),'score provider failed',1,true))
        for _,bad in ipairs({false,true,0,'wrong',function() error('returned function invoked') end}) do
            content.tooltip=function() return bad end
            local ok=pcall(function() return slot:GetTooltip() end)
            assert(not ok)
        end
        tip={name='recovered'};content.tooltip=function() return tip,'ignored' end
        assert(rawequal(slot:GetTooltip(),tip) and select('#',slot:GetTooltip())==1)
    "#,
    );
}

#[test]
fn provider_can_reenter_the_getter_and_rebind_the_same_slot() {
    let ui = setup();
    run(
        &ui,
        r#"
        local depth=0
        content.tooltip=function()
            assert(slot:GetBindedType()=='item')
            calls=calls+1
            if depth==0 then
                depth=1;local inner=slot:GetTooltip();assert(inner.name=='inner')
                AAUI.ItemContent=function()return {type='item',id=7,tooltip=function()return {name='next'}end}end
                slot:EstablishItem(7,1);slot:SetExtent(42,42)
                return {name='outer'}
            end
            return {name='inner'}
        end
        assert(slot:GetTooltip().name=='outer' and calls==2)
        assert(slot:GetTooltip().name=='next' and calls==2)
    "#,
    );
}

#[test]
fn retirement_inside_provider_preserves_current_result_and_suppresses_future_calls() {
    for result in [
        "return {name='last'}",
        "return nil",
        "error('retired provider error')",
    ] {
        let ui = setup();
        ui.lua
            .globals()
            .set(
                "retireChats",
                ui.lua
                    .create_function(|lua, ()| Ok(lua::chat::reset_windows(lua)))
                    .unwrap(),
            )
            .unwrap();
        run(
            &ui,
            &format!(
                r#"
            owner=UIParent:CreateWidget('chatwindow','tooltipOwner','UIParent')
            slot=owner:CreateChildWidget('slot','retiredScore',0,true)
            content.tooltip=function()
                calls=calls+1;assert(retireChats()==1);{result}
            end
            slot:EstablishSlot(ISLOT_ACTION,1)
            ok,value=pcall(function()return slot:GetTooltip()end)
            assert(calls==1)
            assert(select('#',slot:GetTooltip())==0 and calls==1)
        "#
            ),
        );
        run(
            &ui,
            match result {
                "return {name='last'}" => "assert(ok and value.name=='last')",
                "return nil" => "assert(ok and value==nil)",
                _ => {
                    "assert(not ok and string.find(tostring(value),'retired provider error',1,true))"
                }
            },
        );
        ui.refresh_slots().unwrap();
        lua::tick(&ui.lua, 16.);
        run(&ui, "assert(calls==1)");
    }
}

fn point(ui: &UiRuntime, mouse: &mut input::MouseState, pos: (f32, f32)) {
    input::update(&ui.lua, mouse, Some(pos), false, false, false, 0.);
}

fn text(ui: &UiRuntime) -> Vec<String> {
    let value = ui
        .lua
        .load("tooltip.tooltipWindows[1].tooltipItemAndSkill")
        .eval()
        .unwrap();
    let id: Id = lua::value_id(&ui.lua, &value).unwrap();
    lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .tip
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .map(|line| line.left.raw.clone())
        .collect()
}

#[test]
fn physical_original_action_slot_keeps_open_tip_until_rehover_then_fetches_score() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let id = original::install(&mut ui, false);
    let r = lua::ui(&ui.lua).tree.rect(id);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    let away = (p.0 + 200., p.1 - 100.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(id));
    run(&ui, "assert(SCORE_TOOLTIP_CALLS==0)");
    let mut mouse = Default::default();
    point(&ui, &mut mouse, p);
    lua::tick(&ui.lua, 1200.);
    let before = text(&ui);
    assert!(before.iter().any(|line| line.contains("Music Paper")));
    run(&ui, "assert(SCORE_TOOLTIP_CALLS==1);SCORE_RECEIVED=true");
    for _ in 0..100 {
        point(&ui, &mut mouse, p);
        lua::tick(&ui.lua, 16.);
    }
    assert_eq!(text(&ui), before);
    run(&ui, "assert(SCORE_TOOLTIP_CALLS==1)");
    point(&ui, &mut mouse, away);
    point(&ui, &mut mouse, p);
    lua::tick(&ui.lua, 1200.);
    run(&ui, "assert(SCORE_TOOLTIP_CALLS==2)");
    let after = text(&ui);
    assert!(
        after.iter().any(|line| line.contains("Harbor")),
        "lines={after:?}"
    );
    assert!(!before.iter().any(|line| line.contains("Harbor")));
}
