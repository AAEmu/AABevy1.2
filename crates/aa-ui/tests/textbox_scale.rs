//! Eager auto-height invalidation for populated original TextBoxes under effective scale changes.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@textbox_scale", code).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(1)
        parent=UIParent:CreateWidget('emptywidget','boxParent','UIParent')
        parent:SetExtent(600,500);parent:Show(true)
        box=parent:CreateChildWidget('textbox','box',0,true)
        box:SetWidth(225);box.style:SetFont('ui/font/yd_ygo540.ttf',15)
        box:SetLineSpace(3);box:SetAutoResize(true)
        box:SetText('Old forests conceal distant roads and hidden valleys')
    "#,
    );
    ui
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
// Read the stored tree directly: neither GetHeight nor a measurement getter may repair it.
fn stored(ui: &UiRuntime, name: &str) -> f32 {
    let box_id = id(ui, name);
    lua::ui(&ui.lua).tree.get(box_id).height
}
fn stored_tree(ui: &UiRuntime, name: &str) -> f32 {
    let u = lua::ui(&ui.lua);
    let id = u
        .tree
        .find(name)
        .unwrap_or_else(|| panic!("no widget {name}"));
    u.tree.get(id).height
}
fn height(ui: &UiRuntime, expected: f32) {
    let actual = stored(ui, "box");
    assert!(
        (actual - expected).abs() < 0.001,
        "stored={actual}, native={expected}"
    );
}

#[test]
fn populated_auto_textbox_global_scale_refits_before_any_measurement_query() {
    let ui = setup();
    height(&ui, 39.);
    for (scale, expected) in [(0.85, 40.647057), (1., 39.)] {
        run(&ui, &format!("UIParent:SetUIScale({scale})"));
        height(&ui, expected);
    }
}

#[test]
fn populated_auto_textbox_ancestor_scale_refits_before_on_scale_and_round_trips() {
    let ui = setup();
    run(
        &ui,
        r#"
        expectedHeight=40.647057;scaleCalls=0;scaleSawHeight=true
        parent:SetHandler('OnScale',function()
            scaleCalls=scaleCalls+1
            scaleSawHeight=scaleSawHeight and math.abs(box:GetHeight()-expectedHeight)<.001
        end)
        parent:SetScale(.85)
    "#,
    );
    height(&ui, 40.647057);
    run(
        &ui,
        "expectedHeight=39;parent:SetScale(1);assert(scaleCalls==2 and scaleSawHeight)",
    );
    height(&ui, 39.);
}

#[test]
fn opted_out_root_and_frozen_parent_ignore_global_changes_but_follow_effective_scale() {
    let ui = setup();
    run(
        &ui,
        r#"
        frozen=UIParent:CreateWidget('textbox','frozenRoot','UIParent')
        frozen:SetWidth(225);frozen.style:SetFont('ui/font/yd_ygo540.ttf',15)
        frozen:SetLineSpace(3);frozen:SetAutoResize(true);frozen:SetText(box:GetText())
        frozen:ApplyUIScale(false)
        parent:ApplyUIScale(false)
        box:SetHeight(777);frozen:SetHeight(888)
    "#,
    );
    for scale in [0.85, 1.] {
        run(&ui, &format!("UIParent:SetUIScale({scale})"));
        height(&ui, 777.);
        assert_eq!(stored(&ui, "frozen"), 888.);
    }
    run(&ui, "parent:SetScale(.85)");
    height(&ui, 40.647057);
    assert_eq!(stored(&ui, "frozen"), 888.);
    run(&ui, "parent:SetScale(1)");
    height(&ui, 39.);
}

#[test]
fn hidden_auto_textbox_refits_but_manual_height_and_same_scale_are_preserved() {
    let ui = setup();
    run(&ui, "box:Show(false);UIParent:SetUIScale(.85)");
    height(&ui, 40.647057);
    run(
        &ui,
        "UIParent:SetUIScale(1);box:SetHeight(777);UIParent:SetUIScale(1);parent:SetScale(1)",
    );
    height(&ui, 777.);
    run(&ui, "box:SetAutoResize(false);box:SetHeight(444)");
    for command in [
        "UIParent:SetUIScale(.85)",
        "parent:SetScale(.85)",
        "UIParent:SetUIScale(1)",
        "parent:SetScale(1)",
    ] {
        run(&ui, command);
        height(&ui, 444.);
    }
    run(&ui, "box:SetAutoResize(true);parent:SetScale(.85)");
    height(&ui, 40.647057);
}

#[test]
fn empty_and_dead_textboxes_keep_their_stored_height_on_scale_changes() {
    let ui = setup();
    run(
        &ui,
        "box:SetText('');box:SetHeight(321);UIParent:SetUIScale(.85);parent:SetScale(.85)",
    );
    height(&ui, 321.);
    run(
        &ui,
        "UIParent:SetUIScale(1);parent:SetScale(1);box:SetText('Old forests conceal distant roads and hidden valleys')",
    );
    let box_id = id(&ui, "box");
    {
        let mut u = lua::ui(&ui.lua);
        u.tree.get_mut(box_id).height = 333.;
        u.tree.get_mut(box_id).alive = false;
    }
    // A stale enrolled id must be discarded before touching its font, rows, or height.
    run(
        &ui,
        "UIParent:SetUIScale(.85);parent:SetScale(.85);UIParent:SetUIScale(1);parent:SetScale(1)",
    );
    assert_eq!(lua::ui(&ui.lua).tree.get(box_id).height, 333.);
}

#[test]
fn on_scale_reentry_observes_refit_and_can_disable_auto_height_before_nested_scale() {
    let ui = setup();
    run(
        &ui,
        r#"
        entered=false;observed=false;calls=0
        parent:SetHandler('OnScale',function()
            calls=calls+1
            if entered then return end
            entered=true
            observed=math.abs(box:GetHeight()-40.647057)<.001
            parent:SetScale(.85)
            box:SetAutoResize(false);box:SetHeight(909)
            UIParent:SetUIScale(.85)
        end)
        parent:SetScale(.85)
        assert(entered and observed and calls==2)
    "#,
    );
    height(&ui, 909.);
    run(&ui, "UIParent:SetUIScale(1);parent:SetScale(1)");
    height(&ui, 909.);
}

#[test]
fn scaled_anchor_callback_copies_current_auto_height_into_partner() {
    let ui = setup();
    run(
        &ui,
        r#"
        partner=UIParent:CreateWidget('emptywidget','anchorHeightPartner','UIParent')
        partner:SetExtent(100,10)
        anchorHeight=0;anchorCalls=0
        box:SetHandler('OnChangedAnchor',function()
            anchorCalls=anchorCalls+1
            anchorHeight=box:GetHeight()
            partner:SetHeight(anchorHeight)
        end)
    "#,
    );
    for (scale, expected) in [(0.85, 40.647057), (1., 39.)] {
        run(&ui, &format!("parent:SetScale({scale})"));
        let observed: f32 = eval(&ui, "anchorHeight");
        let copied = stored(&ui, "partner");
        assert!(
            (observed - expected).abs() < 0.001 && (copied - expected).abs() < 0.001,
            "scale{scale}: OnChangedAnchor observed={observed}, copied={copied}, stored={}, native={expected}",
            stored(&ui, "box")
        );
    }
    assert_eq!(eval::<usize>(&ui, "anchorCalls"), 2);
}

#[test]
fn enabled_auto_anchor_reentry_refreshes_retained_height_after_nested_global_scale() {
    let ui = setup();
    run(
        &ui,
        r#"
        partner=UIParent:CreateWidget('emptywidget','nestedHeightPartner','UIParent')
        partner:SetExtent(100,10)
        entered=false;anchorCalls=0;anchorHeight=0;nestedScale=.85
        box:SetHandler('OnChangedAnchor',function()
            anchorCalls=anchorCalls+1
            if entered then return end
            entered=true
            UIParent:SetUIScale(nestedScale)
            anchorHeight=box:GetHeight()
            partner:SetHeight(anchorHeight)
        end)
    "#,
    );
    for (scale, expected) in [(0.85, 40.647057), (1., 39.)] {
        run(
            &ui,
            &format!("entered=false;nestedScale={scale};parent:SetScale(UIParent:GetUIScale())"),
        );
        let observed: f32 = eval(&ui, "anchorHeight");
        let copied = stored(&ui, "partner");
        assert!(
            (observed - expected).abs() < 0.001 && (copied - expected).abs() < 0.001,
            "nested scale{scale}: OnChangedAnchor observed={observed}, copied={copied}, stored={}, native={expected}",
            stored(&ui, "box")
        );
    }
    assert_eq!(
        eval::<usize>(&ui, "anchorCalls"),
        2,
        "height refresh must not introduce callback events"
    );
}

#[test]
fn nested_auto_height_refit_refreshes_retained_anchor_dependents_without_extra_callbacks() {
    let ui = setup();
    run(
        &ui,
        r#"
        dependent=UIParent:CreateWidget('emptywidget','heightDependent','UIParent')
        dependent:AddAnchor('TOPLEFT',box,0,0);dependent:AddAnchor('BOTTOMRIGHT',box,0,0)
        leaf=UIParent:CreateWidget('emptywidget','heightLeaf','UIParent')
        leaf:AddAnchor('TOPLEFT',dependent,0,0);leaf:AddAnchor('BOTTOMRIGHT',dependent,0,0)
        partner=UIParent:CreateWidget('emptywidget','dependentHeightPartner','UIParent')
        partner:SetExtent(100,10)
        leafPartner=UIParent:CreateWidget('emptywidget','leafHeightPartner','UIParent')
        leafPartner:SetExtent(100,10)
        entered=false;trace='';nestedScale=.85;dependentHeight=0;leafHeight=0
        dependent:SetHandler('OnChangedAnchor',function() trace=trace..'D' end)
        leaf:SetHandler('OnChangedAnchor',function() trace=trace..'L' end)
        box:SetHandler('OnChangedAnchor',function()
            trace=trace..'B'
            if entered then return end
            entered=true
            UIParent:SetUIScale(nestedScale)
            dependentHeight=dependent:GetHeight();leafHeight=leaf:GetHeight()
            partner:SetHeight(dependentHeight);leafPartner:SetHeight(leafHeight)
        end)
    "#,
    );
    for (scale, expected) in [(0.85, 40.647057), (1., 39.)] {
        run(
            &ui,
            &format!("entered=false;nestedScale={scale};parent:SetScale(UIParent:GetUIScale())"),
        );
        let observed: f32 = eval(&ui, "dependentHeight");
        let leaf: f32 = eval(&ui, "leafHeight");
        let copied = stored(&ui, "partner");
        let copied_leaf = stored(&ui, "leafPartner");
        assert!(
            [observed, leaf, copied, copied_leaf]
                .iter()
                .all(|h| (*h - expected).abs() < 0.001),
            "nested scale{scale}: dependent={observed}, leaf={leaf}, copied={copied}, leafCopy={copied_leaf}, rawbox={}, native={expected}",
            stored(&ui, "box")
        );
    }
    assert_eq!(
        eval::<String>(&ui, "trace"),
        "LDBLDB",
        "preserve depth-first callback order and count"
    );
}

#[test]
fn anchored_sibling_textbox_refits_before_its_first_changed_anchor() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(1)
        parent=UIParent:CreateWidget('emptywidget','scaleParent','UIParent')
        parent:SetExtent(600,500);parent:Show(true)
        sibling=parent:CreateChildWidget('emptywidget','scaleSibling',0,true)
        sibling:SetExtent(10,10)
        box=parent:CreateChildWidget('textbox','box',0,true)
        box:SetWidth(225);box.style:SetFont('ui/font/yd_ygo540.ttf',15)
        box:SetLineSpace(3);box:SetAutoResize(true)
        box:AddAnchor('TOPLEFT',sibling,0,0)
        box:SetText('Old forests conceal distant roads and hidden valleys')
        firstBoxHeight=0;boxCalls=0;siblingSawBox=0
        box:SetHandler('OnChangedAnchor',function()
            boxCalls=boxCalls+1
            if boxCalls==1 then firstBoxHeight=box:GetHeight() end
        end)
        sibling:SetHandler('OnChangedAnchor',function()
            siblingSawBox=box:GetHeight()
        end)
        "#,
    );
    run(&ui, "parent:SetScale(.85)");
    let first: f32 = eval(&ui, "firstBoxHeight");
    let seen: f32 = eval(&ui, "siblingSawBox");
    assert!(
        (first - 40.647057).abs() < 0.001,
        "first OnChangedAnchor saw {first}, native 40.647057"
    );
    assert!(
        (seen - 40.647057).abs() < 0.001,
        "anchored sibling saw {seen}, native 40.647057"
    );
}

#[test]
fn many_textboxes_are_checked_once_per_scale_change() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(1)
        parent=UIParent:CreateWidget('emptywidget','manyParent','UIParent')
        parent:SetExtent(600,500);parent:Show(true)
        for i=1,24 do
            local b=parent:CreateChildWidget('textbox','manyBox'..i,0,true)
            b:SetWidth(225);b.style:SetFont('ui/font/yd_ygo540.ttf',15)
            b:SetLineSpace(3);b:SetAutoResize(true)
            b:SetText('Old forests conceal distant roads and hidden valleys')
        end
        "#,
    );
    lua::ui(&ui.lua).animations.reset_textbox_refit_checks();
    run(&ui, "parent:SetScale(.85)");
    let checks = lua::ui(&ui.lua).animations.textbox_refit_checks();
    assert!(
        checks <= 24,
        "refit checked {checks} TextBoxes for 24 siblings"
    );
    for i in 1..=24 {
        let h = stored_tree(&ui, &format!("manyBox{i}"));
        assert!(
            (h - 40.647057).abs() < 0.001,
            "manyBox{i} height {h}, native 40.647057"
        );
    }
}

#[test]
fn removing_an_earlier_sibling_during_the_scale_walk_still_refits_the_rest() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.lua
        .globals()
        .set(
            "zz_remove",
            ui.lua
                .create_function(|lua, v: mlua::Value| -> mlua::Result<()> {
                    let Some(id) = aa_ui::lua::value_id(lua, &v) else {
                        return Ok(());
                    };
                    let mut u = aa_ui::lua::ui(lua);
                    if !u.tree.get(id).alive {
                        return Ok(());
                    }
                    let parent = u.tree.get(id).parent;
                    u.tree.get_mut(id).alive = false;
                    if let Some(parent) = parent
                        && let Some(w) = u.tree.widget_mut(parent)
                    {
                        w.children.retain(|&c| c != id);
                    }
                    u.animations.remove(id);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        UIParent:SetUIScale(1)
        parent=UIParent:CreateWidget('emptywidget','removeParent','UIParent')
        parent:SetExtent(600,500);parent:Show(true)
        text0=parent:CreateChildWidget('textbox','removeText0',0,true)
        text0:SetWidth(225);text0.style:SetFont('ui/font/yd_ygo540.ttf',15)
        text0:SetLineSpace(3);text0:SetAutoResize(true)
        text0:SetText('Old forests conceal distant roads and hidden valleys')
        remover=parent:CreateChildWidget('emptywidget','removeCaller',0,true)
        remover:SetExtent(10,10)
        text1=parent:CreateChildWidget('textbox','removeText1',0,true)
        text1:SetWidth(225);text1.style:SetFont('ui/font/yd_ygo540.ttf',15)
        text1:SetLineSpace(3);text1:SetAutoResize(true)
        text1:SetText('Old forests conceal distant roads and hidden valleys')
        text2=parent:CreateChildWidget('textbox','removeText2',0,true)
        text2:SetWidth(225);text2.style:SetFont('ui/font/yd_ygo540.ttf',15)
        text2:SetLineSpace(3);text2:SetAutoResize(true)
        text2:SetText('Old forests conceal distant roads and hidden valleys')
        remover:SetHandler('OnChangedAnchor',function() zz_remove(text0) end)
        "#,
    );
    run(&ui, "parent:SetScale(.85)");
    for name in ["removeText1", "removeText2"] {
        let h = stored_tree(&ui, name);
        assert!(
            (h - 40.647057).abs() < 0.001,
            "{name} height {h}, native 40.647057"
        );
    }
}
