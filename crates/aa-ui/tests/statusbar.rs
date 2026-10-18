//! Native status-bar image geometry, texture cropping and original casting-flare anchors.

#[path = "../examples/fake/castingbar.rs"]
mod castingbar;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@statusbar_test", code).unwrap();
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    run(
        &ui,
        r#"
        bar=UIParent:CreateWidget('statusbar','nativeStatus','UIParent')
        bar:SetExtent(200,80);bar:AddAnchor('TOPLEFT',UIParent,100,150)
        bar:Show(true)
        flare=bar:CreateImageDrawable('ui/common/hud.dds','overlay')
        flare:SetExtent(21,25)
        bar:AddAnchorChildToBar(flare,'TOPLEFT','TOPRIGHT',-15,-2)
    "#,
    );
    let flare = lua::value_id(&ui.lua, &eval(&ui, "flare")).unwrap();
    assert!(lua::ui(&ui.lua).tree.get(flare).anchors.is_empty());
    run(&ui, "bar:SetBarTexture('ui/common/hud.dds','background')");
    let bar = lua::value_id(&ui.lua, &eval(&ui, "bar")).unwrap();
    let image = lua::ui(&ui.lua)
        .tree
        .widget(bar)
        .unwrap()
        .status
        .as_ref()
        .unwrap()
        .bar
        .unwrap();
    // Inspect the internal image without adding a Lua method absent from the native API.
    ui.lua
        .globals()
        .set("image", lua::new_object_table(&ui.lua, image).unwrap())
        .unwrap();
    ui
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.002, "{a} != {b}");
}

#[test]
fn child_anchors_to_filled_image_and_tracks_value_range_and_owner_extent() {
    let ui = setup();
    let flare = lua::value_id(&ui.lua, &eval(&ui, "flare")).unwrap();
    assert!(lua::ui(&ui.lua).tree.get(flare).anchors.is_empty());
    run(
        &ui,
        r#"
        bar:SetBarTexture('ui/common/hud.dds','background')
        bar:SetMinMaxValues(0,100);bar:SetValue(25)
        bar:AddAnchorChildToBar(flare,'TOPLEFT','TOPRIGHT',-15,-2)
        local w,h=image:GetExtent();assert(w==50 and h==80)
        local x,y=flare:GetOffset();assert(x==135 and y==148)
        bar:SetValue(75)
        x,y=flare:GetOffset();assert(x==235 and y==148)
        bar:SetMinMaxValues(50,150)
        assert(image:GetWidth()==50)
        bar:SetExtent(400,80)
        assert(image:GetWidth()==100)
        x,y=flare:GetOffset();assert(x==185 and y==148)
    "#,
    );
    let bar = lua::value_id(&ui.lua, &eval(&ui, "image")).unwrap();
    assert_eq!(lua::ui(&ui.lua).tree.get(flare).anchors[0].target, bar);
}

#[test]
fn minimum_and_degenerate_ranges_hide_image_and_retain_filled_axis_extent() {
    let ui = setup();
    run(
        &ui,
        r#"
        bar:SetBarTexture('ui/common/hud.dds','background')
        local w,h=image:GetExtent();assert(w==0 and h==0)
        bar:SetMinMaxValues(0,100);bar:SetValue(75)
        bar:AddAnchorChildToBar(flare,'TOPLEFT','TOPRIGHT',-15,-2)
        bar:SetExtent(400,90)
        bar:SetValue(0)
        w,h=image:GetExtent();assert(w==300 and h==90)
        bar:SetExtent(600,100)
        w,h=image:GetExtent();assert(w==300 and h==100)
        local x,y=flare:GetOffset();assert(x==385 and y==148)
        bar:SetValue(25)
        w,h=image:GetExtent();assert(w==150 and h==100)
        bar:SetMinMaxValues(25,25)
        w,h=image:GetExtent();assert(w==150 and h==100)
        bar:SetExtent(800,120)
        w,h=image:GetExtent();assert(w==150 and h==120)
    "#,
    );
    let image = lua::value_id(&ui.lua, &eval(&ui, "image")).unwrap();
    assert!(!lua::ui(&ui.lua).tree.get(image).shown);
    // The independently anchored decoration remains visible in native Lua.
    let flare = lua::value_id(&ui.lua, &eval(&ui, "flare")).unwrap();
    assert!(lua::ui(&ui.lua).tree.get(flare).shown);
}

#[test]
fn renderer_crops_once_and_vertical_fill_starts_at_top() {
    let ui = setup();
    run(
        &ui,
        r#"
        bar:SetBarTexture('ui/common/hud.dds','background')
        bar:SetBarTextureCoords(459,179,200,80)
        bar:SetMinMaxValues(0,100);bar:SetValue(25)
        flare:Show(false)
    "#,
    );
    let (tw, th) = lua::ui(&ui.lua).textures.size("ui/common/hud.dds").unwrap();
    let q = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].pos, [100., 150., 150., 230.]);
    near(q[0].uv[0] * tw as f32, 459.);
    near(q[0].uv[2] * tw as f32, 509.);
    run(&ui, "bar:SetOrientation('VERTICAL')");
    let q = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].pos, [100., 150., 300., 170.]);
    near(q[0].uv[1] * th as f32, 179.);
    near(q[0].uv[3] * th as f32, 259.);
    // The native vertical path writes original height*f into its UV width slot.
    near(q[0].uv[2] * tw as f32, 479.);
    run(&ui, "bar:SetValue(0)");
    assert!(aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
}

#[test]
fn vertical_hidden_bar_keeps_its_height_while_top_anchors_track_owner_width() {
    let ui = setup();
    run(
        &ui,
        r#"
        bar:SetOrientation('VERTICAL')
        bar:SetMinMaxValues(0,100);bar:SetValue(25)
        bar:AddAnchorChildToBar(flare,'TOPLEFT','BOTTOMLEFT',0,0)
        local x,y=flare:GetOffset();assert(x==100 and y==170)
        bar:SetValue(0);bar:SetExtent(400,160)
        local w,h=image:GetExtent();assert(w==400 and h==20)
        x,y=flare:GetOffset();assert(x==100 and y==170)
        bar:SetValue(50)
        w,h=image:GetExtent();assert(w==400 and h==80)
        x,y=flare:GetOffset();assert(x==100 and y==230)
    "#,
    );
}

#[test]
fn unchanged_original_casting_view_anchors_flare_to_each_progress_endpoint() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    castingbar::install(&mut ui);
    run(
        &ui,
        r#"
        for i,frame in ipairs(nativeCastingBars) do
            local bar=frame.statusBar
            local x,y=bar:GetOffset()
            local width=bar:GetWidth()*i*0.25
            local fx,fy=frame.lightDeco:GetOffset()
            assert(math.abs(fx-(x+width-15))<0.001 and fy==y-2)
        end
    "#,
    );
    let u = lua::ui(&ui.lua);
    for i in 1..=3 {
        let id = u.tree.find(&format!("nativeCasting{i}.statusBar")).unwrap();
        let image = u
            .tree
            .widget(id)
            .unwrap()
            .status
            .as_ref()
            .unwrap()
            .bar
            .unwrap();
        near(u.tree.rect(image).w, u.tree.rect(id).w * i as f32 * 0.25);
    }
}
