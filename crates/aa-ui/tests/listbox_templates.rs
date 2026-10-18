//! List-owned image visibility, native frame animation and fallback state images.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{
    UiRuntime, lua,
    render::{Quad, Tex},
};

fn exec(ui: &UiRuntime, s: &str) {
    ui.exec("@listbox_templates", s).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, s: &str) -> T {
    ui.lua.load(s).eval().unwrap()
}
fn quads(ui: &UiRuntime) -> Vec<Quad> {
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
}
fn image(q: &[Quad], path: &str) -> Vec<Quad> {
    q.iter()
        .filter(|q| matches!(&q.tex, Tex::Image(p) if p.ends_with(path)))
        .cloned()
        .collect()
}
fn same(a: &[Quad], b: &[Quad]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(
            (&a.tex, a.pos, a.uv, a.color, a.corners, a.uvs, a.vcolors),
            (&b.tex, b.pos, b.uv, b.color, b.corners, b.uvs, b.vcolors)
        );
    }
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    exec(
        &ui,
        r#"
        list=UIParent:CreateWidget('listbox','templateTest','UIParent')
        list:AddAnchor('TOPLEFT',UIParent,60,70);list:SetExtent(320,160)
        list:SetInset(20,7,5,7);list:SetHeight(8);list.itemStyle:SetFontSize(14);list:Show(true)
        list:SetItemTrees({{text='A',opened=true,child={{text='a'}}},
            {text='B',opened=true,child={{text='b'}}},{text='C',child={{text='c'}}}})
        opened=list:CreateOpenedImageDrawable('ui/button/grid.dds')
        opened:SetCoords(0,0,21,14);opened:SetExtent(21,14)
        closed=list:CreateClosedImageDrawable('ui/ranking/period.dds')
        closed:SetExtent(13,21)
        separator=list:CreateSeparatorImageDrawable('ui/button/screenshot_mode.dds')
        separator:SetExtent(80,3)
    "#,
    );
    ui
}

#[test]
fn templates_keep_independent_visibility_and_never_draw_at_owner_anchors() {
    let ui = setup();
    exec(
        &ui,
        r#"
        assert(opened:IsVisible() and closed:IsVisible() and separator:IsVisible())
        opened:AddAnchor('TOPLEFT',UIParent,800,800)
    "#,
    );
    let a = image(&quads(&ui), "grid.dds");
    assert_eq!(a.len(), 2);
    assert!(a.iter().all(|q| q.pos[0] < 100. && q.pos[1] < 200.));
    assert_eq!(image(&quads(&ui), "period.dds").len(), 1);
    assert_eq!(image(&quads(&ui), "screenshot_mode.dds").len(), 2);
    exec(&ui, "opened:SetVisible(false);separator:SetVisible(false)");
    let q = quads(&ui);
    assert!(image(&q, "grid.dds").is_empty());
    assert!(image(&q, "screenshot_mode.dds").is_empty());
    assert_eq!(image(&q, "period.dds").len(), 1);
    exec(
        &ui,
        "opened:SetVisible(true);opened:SetColor(1,1,1,.4);closed:SetVisible(false)",
    );
    let q = quads(&ui);
    assert_eq!(image(&q, "grid.dds").len(), 2);
    assert!(image(&q, "grid.dds").iter().all(|q| q.color[3] == 0.4));
    assert!(image(&q, "period.dds").is_empty());
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    same(&quads(&ui), &q);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    assert!(!lua::ui(&ui.lua).tree.changed());
}

#[test]
fn shared_template_frame_sample_is_applied_per_row_and_finishes_hidden() {
    let ui = setup();
    exec(
        &ui,
        r#"
        opened:SetAnimFrameInfo({
            {x=0,y=0,w=21,h=14,time=100,scale=1},
            {x=44,y=0,w=13,h=21,time=100,scale=2,animType=4,alpha=.2}})
        opened:Animation(true,false)
    "#,
    );
    let a = image(&quads(&ui), "grid.dds");
    assert_eq!(a.len(), 2);
    let generation = lua::ui(&ui.lua).tree.generation;
    lua::tick(&ui.lua, 50.);
    same(&image(&quads(&ui), "grid.dds"), &a);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    lua::tick(&ui.lua, 100.);
    let b = image(&quads(&ui), "grid.dds");
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].uv[0], 44. / 128.);
    assert_eq!(b[0].pos[2] - b[0].pos[0], 42.);
    assert!((b[0].color[3] - 0.6).abs() < 0.001);
    for (a, b) in a.iter().zip(&b) {
        assert!((a.pos[0] + a.pos[2] - b.pos[0] - b.pos[2]).abs() < 0.001);
        assert!((a.pos[1] + a.pos[3] - b.pos[1] - b.pos[3]).abs() < 0.001);
    }
    lua::tick(&ui.lua, 50.);
    exec(&ui, "assert(not opened:IsVisible())");
    assert!(image(&quads(&ui), "grid.dds").is_empty());
    exec(&ui, "opened:Animation(true,true)");
    lua::tick(&ui.lua, 250.);
    exec(&ui, "assert(opened:IsVisible())");
    same(&image(&quads(&ui), "grid.dds"), &a);
    exec(&ui, "list:RemoveAllDrawables()");
    lua::tick(&ui.lua, 500.);
    assert!(image(&quads(&ui), "grid.dds").is_empty());
}

#[test]
fn original_achievement_templates_mouse_toggle_and_slider_fallback_textures() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    fake::enter_world(&mut ui);
    exec(
        &ui,
        include_str!("../examples/fixtures/listbox_templates.lua"),
    );
    lua::tick(&ui.lua, 500.);
    exec(
        &ui,
        "opened:Animation(false,false);opened:SetVisible(true);closed:SetVisible(false)",
    );
    let mut mouse = Default::default();
    let p = eval(
        &ui,
        "local x,y=tree:GetOffset();local l,t=tree:GetInset();return x+l+12,y+t+tree:GetHeight()*3.5",
    );
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    exec(
        &ui,
        "assert(tree:IsItemOpened(3) and counts:ItemCount()==12)",
    );
    assert!(!image(&quads(&ui), "grid.dds").is_empty());
    exec(&ui, "opened:SetVisible(false);rows.list:Select(0)");
    let p = eval(
        &ui,
        "local x,y=rows.list:GetOffset();local l,t=rows.list:GetInset();return x+l+12,y+t+rows.list:GetHeight()*1.5",
    );
    aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, false, false, 0.);
    let q = quads(&ui);
    assert!(image(&q, "grid.dds").is_empty());
    assert!(!image(&q, "textures/defaults/white.dds").is_empty());
    assert_eq!(image(&q, "textures/defaults/grey.dds").len(), 1);
    exec(
        &ui,
        "rows.list:SetListItemStateTexture('ui/ranking/period.dds')",
    );
    let q = quads(&ui);
    assert!(image(&q, "textures/defaults/grey.dds").is_empty());
    assert_eq!(image(&q, "period.dds").len(), 2);
}
