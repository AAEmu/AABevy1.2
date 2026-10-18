//! Actual392c2f50/3928e6c0/392c7360 completion-child goldens.
use aa_ui::{
    UiRuntime, lua,
    render::{Quad, Tex},
    tree::Id,
};
use lua::slot::CooldownSample as Sample;

fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    lua::slot::set_native_cooldown_mode(&ui.lua, true);
    ui.exec("@completion", r#"
      AAUI.SkillContent=function() return {type='skill',id=7,icon='ui/icon/icon_skill_basic01.dds'} end
      parent=UIParent:CreateWidget('window','completionParent','UIParent')
      parent:SetExtent(42,42);parent:Show(true)
      slot=parent:CreateChildWidget('slot','completionSlot',0,true)
      slot:SetExtent(42,42);slot:EstablishSkill(7);slot:Show(true)
      events={};slot:SetHandler('OnContentUpdated',function(self,event) table.insert(events,event) end)
    "#).unwrap();
    let id = lua::value_id(&ui.lua, &ui.lua.load("slot").eval().unwrap()).unwrap();
    (ui, id)
}
fn query(ui: &UiRuntime, id: Id, value: Option<(u32, u32)>) {
    lua::slot::update_native_slot_cooldown(&ui.lua, id, 15, Sample::default(), |_, _| {
        value.map(|(remaining_ms, total_ms)| Sample {
            remaining_ms,
            total_ms,
        })
    });
}
fn start(ui: &UiRuntime, id: Id) {
    query(ui, id, Some((500, 1000)));
    query(ui, id, Some((0, 1000)));
}
fn paint(ui: &UiRuntime) -> Vec<Quad> {
    let path = lua::ui(&ui.lua)
        .textures
        .resolve("ui/common/hud.dds")
        .unwrap();
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| q.tex == Tex::Image(path.clone()))
        .collect()
}
fn frame(ui: &UiRuntime, ms: f32) -> Vec<Quad> {
    lua::tick(&ui.lua, ms);
    paint(ui)
}
fn alpha(quads: &[Quad]) -> u32 {
    quads
        .first()
        .map_or(0, |q| (q.color[3] * 255.).round() as u32)
}

#[test]
fn native_phase_handoff_and_ninepart_vertices_match_executed_goldens() {
    let (ui, id) = setup();
    start(&ui, id);
    for expected in [54, 114, 181, 255, 229, 0, 0] {
        let q = frame(&ui, 100.);
        assert_eq!(alpha(&q), expected);
        if expected > 0 {
            assert_eq!(q.len(), 9);
        }
        if expected == 54 {
            assert_eq!(
                q[0].corners.unwrap(),
                [[12., 12.], [15., 12.], [15., 15.], [12., 15.]]
            );
            assert_eq!(
                q[1].corners.unwrap(),
                [[15., 12.], [27., 12.], [27., 15.], [15., 15.]]
            );
            assert_eq!(
                q[0].uv,
                [743. / 1024., 35. / 512., 749. / 1024., 42. / 512.]
            );
        }
    }
    assert_eq!(
        ui.lua
            .load("table.concat(events,'|')")
            .eval::<String>()
            .unwrap(),
        "skill_cooldown_start|skill_cooldown_done"
    );
}

#[test]
fn overshoot_is_carried_but_next_phase_initial_value_gets_its_own_draw() {
    let (ui, id) = setup();
    start(&ui, id);
    assert_eq!(alpha(&frame(&ui, 450.)), 255);
    assert_eq!(alpha(&frame(&ui, 50.)), 229);
    assert_eq!(alpha(&frame(&ui, 50.)), 71);
}

#[test]
fn hidden_ancestor_pauses_while_alpha_zero_offscreen_and_empty_clip_still_advance() {
    let (ui, id) = setup();
    start(&ui, id);
    ui.exec("@hide", "parent:Show(false)").unwrap();
    assert!(frame(&ui, 5000.).is_empty());
    ui.exec("@show", "parent:Show(true);slot:SetAlpha(0)")
        .unwrap();
    assert!(frame(&ui, 100.).is_empty());
    ui.exec(
        "@alpha",
        "slot:SetAlpha(1);slot:AddAnchor('TOPLEFT',parent,5000,5000)",
    )
    .unwrap();
    assert_eq!(alpha(&frame(&ui, 100.)), 114);
    ui.exec(
        "@clip",
        "slot:RemoveAllAnchors();parent:SetExtent(0,0);parent:EnableScroll(true)",
    )
    .unwrap();
    assert!(frame(&ui, 100.).is_empty());
    ui.exec(
        "@unclip",
        "parent:EnableScroll(false);parent:SetExtent(42,42)",
    )
    .unwrap();
    assert_eq!(alpha(&frame(&ui, 100.)), 255);
}

#[test]
fn mask_and_short_total_pause_existing_flash_without_new_callbacks() {
    let (ui, id) = setup();
    start(&ui, id);
    assert_eq!(alpha(&frame(&ui, 100.)), 54);
    query(&ui, id, Some((800, 1000)));
    lua::tick(&ui.lua, 5000.);
    paint(&ui);
    // ResetState clears flags, preserving the already-running internal effect and draw fields.
    ui.exec("@reset", "slot:ResetState()").unwrap();
    query(&ui, id, Some((0, 199)));
    assert!(frame(&ui, 5000.).is_empty());
    query(&ui, id, Some((0, 1000)));
    assert_eq!(alpha(&frame(&ui, 100.)), 114);
    query(&ui, id, None);
    assert!(frame(&ui, 5000.).is_empty());
    query(&ui, id, Some((0, 1000)));
    assert_eq!(alpha(&frame(&ui, 100.)), 181);
}

#[test]
fn orphaned_subtrees_do_not_advance_outside_the_actual_paint_traversal() {
    let (ui, id) = setup();
    start(&ui, id);
    let parent = lua::ui(&ui.lua).tree.get(id).parent.unwrap();
    lua::ui(&ui.lua)
        .tree
        .widget_mut(parent)
        .unwrap()
        .children
        .retain(|&c| c != id);
    assert!(frame(&ui, 5000.).is_empty());
    lua::ui(&ui.lua)
        .tree
        .widget_mut(parent)
        .unwrap()
        .children
        .push(id);
    assert_eq!(alpha(&frame(&ui, 100.)), 54);
}

#[test]
fn completion_starts_after_lua_callback_and_compatibility_clock_keeps_its_identity() {
    let (ui, id) = setup();
    start(&ui, id);
    for _ in 0..3 {
        frame(&ui, 100.);
    }
    lua::tick(&ui.lua, 0.);
    let observe = ui
        .lua
        .create_function(|lua, ()| {
            let path = lua::ui(lua).textures.resolve("ui/common/hud.dds").unwrap();
            let q = aa_ui::render::build_quads(&mut lua::ui(lua), None);
            Ok(q.into_iter()
                .find(|q| q.tex == Tex::Image(path.clone()))
                .map(|q| (q.color[3] * 255.).round() as u32))
        })
        .unwrap();
    ui.lua.globals().set("ObserveCompletion", observe).unwrap();
    ui.exec("@handler", "slot:SetHandler('OnContentUpdated',function(self,event) if event=='skill_cooldown_done' then observed=ObserveCompletion() end end)").unwrap();
    start(&ui, id);
    assert_eq!(ui.lua.globals().get::<u32>("observed").unwrap(), 181);
    assert!(paint(&ui).is_empty());
    assert_eq!(alpha(&frame(&ui, 100.)), 54);
    lua::slot::set_native_cooldown_mode(&ui.lua, false);
    ui.exec("@legacy", "AAUI.SkillContent=function() return {type='skill',id=7,cooldown={remain=100,total=1000}} end;slot:EstablishSkill(7)").unwrap();
    assert_eq!(alpha(&frame(&ui, 100.)), 54);
    assert!(lua::ui(&ui.lua).tree.widget(id).unwrap().cooldown.is_none());
}

#[test]
fn callback_mutated_timer_controls_admission_before_the_new_child_advances() {
    let (ui, id) = setup();
    let update_short = ui
        .lua
        .create_function(move |lua, ()| {
            lua::slot::update_native_slot_cooldown(lua, id, 15, Sample::default(), |_, _| {
                Some(Sample {
                    remaining_ms: 0,
                    total_ms: 100,
                })
            });
            Ok(())
        })
        .unwrap();
    ui.lua
        .globals()
        .set("UpdateShortTimer", update_short)
        .unwrap();
    ui.exec("@callback_timer", "slot:SetHandler('OnContentUpdated',function(self,event) if event=='skill_cooldown_done' then UpdateShortTimer() end end)").unwrap();
    start(&ui, id);
    assert!(frame(&ui, 5000.).is_empty());
    query(&ui, id, Some((0, 1000)));
    assert_eq!(alpha(&frame(&ui, 100.)), 54);
}
