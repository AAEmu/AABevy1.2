//! Native allocated/active tab count and independent selection/visibility lifetime.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/fake/guild_sponsors.rs"]
mod guild_sponsors;

use aa_ui::{UiRuntime, input, lua};
fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@tab_active", code).unwrap();
}
fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        tab=UIParent:CreateWidget('tab','nativeActive','UIParent')
        tab:SetExtent(400,180);tab:AddAnchor('TOPLEFT',UIParent,100,100);tab:Show(true)
        assert(tab:GetActivateTabCount()==0 and tab:GetSelectedTab()==1)
        for i=1,7 do
            tab:AddSimpleTab('sponsor'..i)
            tab.selectedButton[i]:SetExtent(40,24)
            tab.unselectedButton[i]:SetExtent(40,24)
        end
        changed={};tab:SetHandler('OnTabChanged',function(self,i) changed[#changed+1]=i end)
    "#,
    );
    lua::tick(&ui.lua, 0.0);
    ui
}

#[test]
fn count_only_updates_signed_upper_bounded_getter_until_selection() {
    let ui = setup();
    run(
        &ui,
        r#"
        tab:SelectTab(7);changed={}
        oldX=tab.unselectedButton[7]:GetOffset()
        tab:SetActivateTabCount(5.9)
        assert(tab:GetActivateTabCount()==5 and #tab.window==7)
        assert(tab:GetSelectedTab()==7 and tab.window[7]:IsVisible())
        assert(tab.selectedButton[7]:IsVisible() and #changed==0)
        assert(tab.unselectedButton[7]:GetOffset()==oldX)
        tab:SetActivateTabCount('3');tab:SetActivateTabCount(false);tab:SetActivateTabCount()
        tab:SelectTab('2');assert(tab:GetActivateTabCount()==5 and tab:GetSelectedTab()==7 and #changed==0)
        tab:SetActivateTabCount(100);assert(tab:GetActivateTabCount()==7)
        tab:SetActivateTabCount(-1.9);assert(tab:GetActivateTabCount()==-1)
        tab:SelectTab(1);assert(tab:GetSelectedTab()==7 and #changed==0)
        tab:SetActivateTabCount(0);assert(tab:GetActivateTabCount()==0)
        assert(tab.window[7]:IsVisible())
    "#,
    );
}

#[test]
fn selection_hides_inactive_rows_and_preserves_allocated_positions_and_panes() {
    let ui = setup();
    run(
        &ui,
        r#"
        x7=tab.unselectedButton[7]:GetOffset()
        pane7=tab.window[7]
        tab:SetActivateTabCount(5);tab:SelectTab(1)
        assert(tab:GetSelectedTab()==1 and #changed==1)
        assert(tab.selectedButton[1]:IsVisible() and tab.window[1]:IsVisible())
        for i=2,5 do assert(tab.unselectedButton[i]:IsVisible() and not tab.window[i]:IsVisible()) end
        for i=6,7 do
            assert(not tab.unselectedButton[i]:IsVisible() and not tab.selectedButton[i]:IsVisible())
            assert(not tab.window[i]:IsVisible())
        end
        assert(tab.unselectedButton[7]:GetOffset()==x7 and tab.window[7]==pane7)
        tab:SetActivateTabCount(7);tab:SelectTab(7)
        assert(tab.window[7]==pane7 and pane7:IsVisible())
    "#,
    );
}

#[test]
fn repeated_none_and_out_of_range_selection_always_notifies_without_selecting_pane() {
    let ui = setup();
    run(
        &ui,
        r#"
        tab:SetActivateTabCount(5)
        tab:SelectTab(3);tab:SelectTab(3);tab:SelectTab(0);tab:SelectTab(7);tab:SelectTab(99)
        assert(#changed==5 and changed[1]==3 and changed[2]==3 and changed[3]==0 and changed[4]==7 and changed[5]==99)
        assert(tab:GetSelectedTab()==99)
        for i=1,7 do
            assert(not tab.window[i]:IsVisible() and not tab.selectedButton[i]:IsVisible())
            assert(tab.unselectedButton[i]:IsVisible()==(i<=5))
        end
        tab:SelectTab(-2.9);assert(tab:GetSelectedTab()==-2 and changed[6]==-2)
    "#,
    );
}

#[test]
fn insertion_resets_active_allocation_and_first_selection_but_clear_retains_selection() {
    let ui = setup();
    run(
        &ui,
        r#"
        tab:SetActivateTabCount(2);tab:SelectTab(2);changed={}
        tab:AddSimpleTab('extra')
        assert(tab:GetActivateTabCount()==8 and tab:GetSelectedTab()==1 and changed[1]==1)
        assert(tab.unselectedButton[8]:IsVisible())
        tab:SelectTab(5);tab:RemoveAllTabs()
        assert(tab:GetActivateTabCount()==0 and tab:GetSelectedTab()==5)
        tab:AddSimpleTab('reused')
        assert(tab:GetActivateTabCount()==1 and tab:GetSelectedTab()==1)
        sel=tab:CreateChildWidget('button','customSelected',0,true)
        unsel=tab:CreateChildWidget('button','customUnselected',0,true)
        pane=tab:CreateChildWidget('window','customPane',0,true)
        tab:SetActivateTabCount(0);tab:AddNewTab('borrowed',sel,unsel,pane)
        assert(tab:GetActivateTabCount()==2 and tab:GetSelectedTab()==1 and unsel:IsVisible())
    "#,
    );
}

#[test]
fn selected_button_click_does_not_repeat_the_explicit_selection_callback() {
    let ui = setup();
    let button = eval::<mlua::Value>(&ui, "tab.selectedButton[1]");
    let button = lua::value_id(&ui.lua, &button).unwrap();
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, button)
    };
    let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
    assert_eq!(eval::<i64>(&ui, "#changed"), 0);
    run(&ui, "tab:SelectTab(1);assert(#changed==1)");
}

#[test]
fn original_creation_has_six_or_five_pickable_sponsors_and_zero_clears_panes() {
    for (alliance, count) in [(148, 6), (149, 5)] {
        let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        guild_sponsors::install(&mut ui, alliance);
        for i in 1..=7 {
            let value = eval::<mlua::Value>(&ui, &format!("sponsorTab.unselectedButton[{i}]"));
            let button = lua::value_id(&ui.lua, &value).unwrap();
            let r = {
                let u = lua::ui(&ui.lua);
                u.animations.widget_rect(&u.tree, button)
            };
            let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
            if i == count {
                assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
                let mut mouse = Default::default();
                input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
                input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
            } else if i > count {
                assert_ne!(input::pick(&ui.lua, p.0, p.1), Some(button));
            }
        }
        assert_eq!(eval::<i64>(&ui, "sponsorTab:GetSelectedTab()"), count);
        run(
            &ui,
            "sponsorTab:SelectTab(0);for i=1,7 do assert(not sponsorTab.window[i]:IsVisible()) end",
        );
        assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    }
}
