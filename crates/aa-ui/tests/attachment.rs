//! Native lifetime links and the original popup's local upvalue/OnHide contract.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@attachment",
        r#"
        owner=UIParent:CreateWidget('emptywidget','owner','UIParent'); owner:Show(true)
        child=owner:CreateChildWidget('button','child',0,true); child:SetExtent(100,30)
        popup=UIParent:CreateWidget('window','popup','UIParent'); popup:Show(true)
        other=UIParent:CreateWidget('window','other','UIParent'); other:Show(true)
        hides=0
        popup:SetHandler('OnHide',function() hides=hides+1 end)
    "#,
    )
    .unwrap();
    ui
}

#[test]
fn attachment_preserves_parentage_and_anchors_and_detach_does_not_hide() {
    let ui = setup();
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    let content = lua::ui(&ui.lua).tree.content();
    ui.exec("@link", "child:AttachWidget(popup); assert(child:GetAttachedWidget()==popup); child:AttachWidget(popup); assert(popup:GetParent()==UIParent); child:DetachWidget(); assert(child:GetAttachedWidget()==nil and popup:IsVisible()); child:DetachWidget()").unwrap();
    assert_eq!(eval::<u32>(&ui, "hides"), 0);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
}

#[test]
fn replacement_dispatches_old_hide_before_assigning_new_link() {
    let ui = setup();
    ui.exec(
        "@replace",
        r#"
        child:AttachWidget(popup)
        popup:SetHandler('OnHide',function()
            assert(child:GetAttachedWidget()==popup)
            child:DetachWidget(); hides=hides+1
        end)
        child:AttachWidget(other)
        assert(not popup:IsVisible() and other:IsVisible())
        assert(child:GetAttachedWidget()==other)
        child:AttachWidget(nil) -- Native invalid arguments leave the link alone.
        assert(child:GetAttachedWidget()==other)
    "#,
    )
    .unwrap();
    assert_eq!(eval::<u32>(&ui, "hides"), 1);
}

#[test]
fn ancestor_hide_clears_hidden_descendant_links_and_nested_popups_once() {
    let ui = setup();
    ui.exec(
        "@ancestor",
        r#"
        nested=popup:CreateChildWidget('button','nested',0,true)
        nested:AttachWidget(other)
        child:Show(false); child:AttachWidget(popup)
        owner:SetHandler('OnHide',function() assert(child:GetAttachedWidget()==popup) end)
        owner:Show(false)
        assert(not popup:IsVisible() and not other:IsVisible())
        assert(child:GetAttachedWidget()==nil and nested:GetAttachedWidget()==nil)
        owner:Show(false)
    "#,
    )
    .unwrap();
    assert_eq!(eval::<u32>(&ui, "hides"), 1);
}

#[test]
fn cyclic_links_close_once_and_dead_targets_are_not_returned() {
    let ui = setup();
    ui.exec(
        "@rows",
        r#"
        child:AttachWidget(popup); popup:AttachWidget(child)
        owner:Show(false)
        assert(not popup:IsVisible() and child:GetAttachedWidget()==nil and popup:GetAttachedWidget()==nil)
        child:AttachWidget(other)
    "#,
    )
    .unwrap();
    assert_eq!(eval::<u32>(&ui, "hides"), 1);
    let other = lua::value_id(&ui.lua, &eval(&ui, "other")).unwrap();
    lua::ui(&ui.lua).tree.get_mut(other).alive = false;
    assert!(eval::<bool>(&ui, "child:GetAttachedWidget()==nil"));
}

#[test]
fn original_popup_attaches_hides_for_matching_owner_and_reopens_after_ancestor_hide() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@original_popup",
        r#"
        owner=UIParent:CreateWidget('emptywidget','popupOwner','UIParent'); owner:Show(true)
        child=owner:CreateChildWidget('button','child',0,true); child:SetExtent(100,30)
        other=UIParent:CreateWidget('emptywidget','otherOwner','UIParent'); other:Show(true)
        local create=CreateEmptyWindow
        CreateEmptyWindow=function(id,parent) popup=create(id,parent); return popup end
        info=GetDefaultPopupInfoTable(); info:AddInfo('Whisper',function() end)
        ShowPopUpMenu('originalAttachmentPopup',child,info)
        first=popup
        assert(child:GetAttachedWidget()==first and first:IsVisible())
        HidePopUpMenu(other); assert(first:IsVisible())
        HidePopUpMenu(child); assert(not first:IsVisible() and child:GetAttachedWidget()==nil)
        ShowPopUpMenu('originalAttachmentPopup2',child,info)
        second=popup
        assert(second~=first and second:IsVisible())
        owner:Show(false)
        assert(not second:IsVisible() and child:GetAttachedWidget()==nil)
        owner:Show(true)
        ShowPopUpMenu('originalAttachmentPopup3',child,info)
        assert(child:GetAttachedWidget()==popup and popup:IsVisible())
        CreateEmptyWindow=create
    "#,
    )
    .unwrap();
    assert!(
        ui.stub_calls()
            .iter()
            .all(|(name, _)| !name.ends_with(":AttachWidget")
                && !name.ends_with(":DetachWidget")
                && !name.ends_with(":GetAttachedWidget"))
    );
}
