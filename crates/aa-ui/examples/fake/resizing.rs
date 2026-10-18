//! Unchanged quest notifier, with saved quest data and a physical bottom-edge resize.

use aa_ui::{UiRuntime, input, lua};

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.exec(
        "@resize_saved_quests",
        r#"
        X2.GetQuestNotifierListUiData=function()
            return {listOpen=true,openHeight=300,questList={2531,330,250}}
        end
    "#,
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@resize_original_tracker",
        r#"
        GetMergedQuests()
        OpenNotifier()
        questContext.notifierBack:RemoveAllAnchors()
        questContext.notifierBack:AddAnchor('TOPRIGHT',UIParent,-35,180)
        questContext.notifierBack:ShowMoveAndBg()
    "#,
    )
    .unwrap();
}

pub fn drag_bottom(ui: &UiRuntime) -> (f32, f32) {
    let (id, before) = {
        let u = lua::ui(&ui.lua);
        let id = u.tree.find("questContext.notifierBack").unwrap();
        (id, u.tree.rect(id))
    };
    let mut mouse = input::MouseState::default();
    let at = (before.x + before.w * 0.5, before.bottom() - 5.0);
    input::update(&ui.lua, &mut mouse, Some(at), true, false, false, 0.0);
    let end = (at.0, at.1 + 130.0);
    input::update(&ui.lua, &mut mouse, Some(end), false, false, false, 0.0);
    input::update(&ui.lua, &mut mouse, Some(end), false, true, false, 0.0);
    let after = lua::ui(&ui.lua).tree.rect(id);
    assert!(
        (after.h - before.h - 130.0).abs() < 0.01,
        "{before:?} -> {after:?}"
    );
    assert_eq!((after.x, after.y, after.w), (before.x, before.y, before.w));
    (after.x + after.w * 0.5, after.bottom() - 5.0)
}
