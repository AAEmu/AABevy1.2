//! Icon overloads, state geometry and original Achievement category decoration.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{
    UiRuntime, lua,
    render::{Quad, Tex},
};

fn exec(ui: &UiRuntime, s: &str) {
    ui.exec("@listbox_art", s).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, s: &str) -> T {
    ui.lua.load(s).eval().unwrap()
}
fn quads(ui: &UiRuntime) -> Vec<Quad> {
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
}
fn image<'a>(q: &'a [Quad], path: &str) -> Vec<&'a Quad> {
    q.iter()
        .filter(|q| matches!(&q.tex,Tex::Image(p) if p.ends_with(path)))
        .collect()
}
fn click(ui: &UiRuntime, mouse: &mut aa_ui::input::MouseState, name: &str, row: usize) {
    let p = eval(
        ui,
        &format!(
            "local x,y={name}:GetOffset();local l,t={name}:GetInset();return x+l+12,y+t+{name}:GetHeight()*({row}+.5)"
        ),
    );
    aa_ui::input::update(&ui.lua, mouse, Some(p), true, false, false, 0.);
    aa_ui::input::update(&ui.lua, mouse, Some(p), false, true, false, 0.);
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    exec(
        &ui,
        r#"
        list=UIParent:CreateWidget('listbox','nativeArtTest','UIParent')
        list:AddAnchor('TOPLEFT',UIParent,60,70);list:SetExtent(320,150)
        list:SetInset(5,7,5,7);list:SetHeight(8);list.itemStyle:SetFontSize(14);list:Show(true)
    "#,
    );
    ui
}

#[test]
fn exact_icon_overloads_cached_sources_native_anchor_and_tree_suppression() {
    let ui = setup();
    exec(
        &ui,
        r#"
        list:AppendItem('Controls',11,'ui/button/screenshot_mode.dds')
        list:AppendItem('Period',22,1,0,0,1,'ui/ranking/period.dds')
        list:AppendItem('Invalid icon type',88,false)
        list:AppendItem('Invalid RGBA',99,{},0,0,1,'ui/ranking/period.dds')
        list:AppendItem('Native rejected source size',33,'ui/icon/icon_item_potion01.dds')
        assert(list:ItemCount()==3);list:Select(1);assert(list:GetSelectedValue()==22)
        list:UseConsistentTextColor(false)
    "#,
    );
    let h = eval::<f32>(&ui, "list:GetHeight()");
    let q = quads(&ui);
    let a = image(&q, "screenshot_mode.dds");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].uv, [0., 0., 1., 1.]);
    assert_eq!(a[0].pos, [75., 77. + h * 0.5, 75. + h, 77. + h * 1.5]);
    assert_eq!(image(&q, "period.dds").len(), 1);
    assert!(image(&q, "icon_item_potion01.dds").is_empty());
    assert!(q.iter().any(|q| q.color == [1., 0., 0., 1.]));
    lua::ui(&ui.lua).tree.settle();
    quads(&ui);
    assert!(!lua::ui(&ui.lua).tree.changed());
    exec(
        &ui,
        r#"
        list:SetItemTrees({{text='Root',iconPath='ui/button/screenshot_mode.dds',opened=true,
            child={{text='Leaf',value=42,iconPath='ui/ranking/period.dds'}}}})
        assert(list:IsItemOpened(0) and not list:IsItemOpened(1) and not list:IsItemOpened(-1))
    "#,
    );
    let q = quads(&ui);
    assert!(image(&q, "screenshot_mode.dds").is_empty());
    assert!(image(&q, "period.dds").is_empty());
    exec(
        &ui,
        "list:ClearItem();list:AppendItem('Still indented',44);list:Select(0)",
    );
    let q = quads(&ui);
    let glyphs: Vec<_> = q
        .iter()
        .filter(|q| matches!(q.tex, Tex::Atlas) && q.color == [1., 1., 1., 1.])
        .collect();
    assert!(!glyphs.is_empty());
    assert!(glyphs.iter().all(|q| q.pos[0] >= 70. + h));
}

#[test]
fn state_texture_uvs_require_texture_and_expand_inside_widget_clip() {
    let ui = setup();
    exec(
        &ui,
        r#"
        list:SetSelectedItemCoord(9,8,7,6) -- no texture: native ignores this setter
        list:SetListItemStateTexture('ui/button/grid.dds')
        list:SetListItemStateTextureInset(-15,-2,-10,-1)
        list:SetSelectedItemColor(1,0,0,1);list:SetOveredItemColor(0,1,0,1)
        list:AppendItem('Selected',11);list:AppendItem('Hover',22);list:Select(0)
    "#,
    );
    let q = quads(&ui);
    let a = image(&q, "grid.dds");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].pos[0], 60.);
    assert_eq!(a[0].pos[1], 75.);
    assert_eq!(a[0].pos[2], 380.);
    // Clipping preserves the original UV interpolation across the expanded left edge.
    assert!(a[0].uv[0] > 0. && a[0].uv[0] < 0.1);
    assert_eq!(a[0].uv[2], 1.);
    exec(
        &ui,
        "list:SetSelectedItemCoord(0,0,21,14);list:SetOveredItemCoord(44,0,13,21)",
    );
    let mut mouse = Default::default();
    click(&ui, &mut mouse, "list", 1);
    exec(&ui, "list:Select(0)");
    let q = quads(&ui);
    let a = image(&q, "grid.dds");
    assert_eq!(a.len(), 2);
    let red = a.iter().find(|q| q.color == [1., 0., 0., 1.]).unwrap();
    let green = a.iter().find(|q| q.color == [0., 1., 0., 1.]).unwrap();
    assert!(red.uv[2] < green.uv[0]);
    assert!(
        q.iter()
            .all(|q| q.pos[0] >= 60. && q.pos[1] >= 70. && q.pos[2] <= 380. && q.pos[3] <= 220.)
    );
}

#[test]
fn original_achievement_separator_toggle_counts_scroll_and_flat_wrapper_icons() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    fake::enter_world(&mut ui);
    exec(&ui, include_str!("../examples/fixtures/listbox_art.lua"));
    // Exercise a shorter viewport through ordinary window sizing, then finish its authored
    // opening animation before comparing physical quad coordinates.
    exec(&ui, "achievement:SetExtent(850,330)");
    aa_ui::lua::tick(&ui.lua, 500.0);
    exec(
        &ui,
        "assert(tree:ItemCount()==8 and tree:GetOpenedItemCount()==8 and not tree:IsItemOpened(3));assert(icons.list:ItemCount()==3)",
    );
    let mut mouse = Default::default();
    click(&ui, &mut mouse, "tree", 3);
    exec(
        &ui,
        "assert(tree:IsItemOpened(3) and tree:GetOpenedItemCount()==12 and counts:ItemCount()==12)",
    );
    let q = quads(&ui);
    let (x, y, h): (f32, f32, f32) = eval(
        &ui,
        "local x,y=tree:GetOffset();local l,t=tree:GetInset();return x+l+10,y+t,tree:GetHeight()",
    );
    let lines: Vec<_> = q
        .iter()
        .filter(|q| matches!(q.tex, Tex::Image(_)) && q.color == [1., 1., 1., 0.8])
        .collect();
    assert!(!lines.is_empty());
    assert!(
        lines
            .iter()
            .any(|q| (q.pos[0] - x).abs() < 0.01 && (q.pos[1] - (y + h + 2. - 3.)).abs() < 0.01),
        "expected {x},{y}+{h}-1, lines={lines:?}"
    );
    assert_eq!(image(&q, "screenshot_mode.dds").len(), 1);
    assert_eq!(image(&q, "period.dds").len(), 1);
    exec(&ui, r#"
        icons:AppendItem('Healing potion',44,'ui/icon/icon_item_potion01.dds')
        icons:AppendItem('Mana potion',55,'ui/icon/icon_item_potion02.dds')
        assert(icons.list:ItemCount()==5)
    "#);
    click(&ui, &mut mouse, "icons.list", 3);
    exec(&ui, "assert(icons.list:GetSelectedValue()==44 and icons.list:GetSelectedText()=='Healing potion')");
    let q = quads(&ui);
    assert!(image(&q, "icon_item_potion01.dds").is_empty());
    assert!(image(&q, "icon_item_potion02.dds").is_empty());
    assert_eq!(image(&q, "screenshot_mode.dds").len(), 1);
    // A root at viewport index zero never receives a separator, even after scrolling.
    exec(
        &ui,
        "category.scroll.vs:SetValue(1,true);assert(tree:GetTop()==1 and counts:GetTop()==1)",
    );
    let q = quads(&ui);
    assert!(!q.iter().any(|q| q.color == [1., 1., 1., 0.8]
        && (q.pos[0] - x).abs() < 0.01
        && (q.pos[1] - (y - 1.)).abs() < 0.01));
    exec(&ui, "category.scroll.vs:SetValue(0,true)");
    click(&ui, &mut mouse, "tree", 3);
    exec(
        &ui,
        "assert(not tree:IsItemOpened(3) and tree:GetOpenedItemCount()==8 and counts:ItemCount()==8)",
    );
}
