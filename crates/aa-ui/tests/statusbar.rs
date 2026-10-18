//! Native status-bar image geometry, texture cropping and original casting-flare anchors.

#[path = "../examples/fake/castingbar.rs"]
mod castingbar;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/fake/healthbar.rs"]
mod healthbar;

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

fn status(ui: &UiRuntime, name: &str) -> aa_ui::lua::statusbar::StatusBarData {
    let id = lua::value_id(&ui.lua, &eval(ui, name)).unwrap();
    lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .status
        .clone()
        .unwrap()
}

#[test]
fn target_is_immediate_and_native_seconds_drive_the_separate_fill() {
    let ui = setup();
    run(
        &ui,
        "bar:SetMinMaxValues(0,100);bar:SetValue(20);bar:SetValue(80,true,0.2);assert(bar:GetValue()==80)",
    );
    assert_eq!(status(&ui, "bar").displayed, 20.0);
    lua::tick(&ui.lua, 50.0);
    assert_eq!(status(&ui, "bar").displayed, 35.0);
    lua::tick(&ui.lua, 50.0);
    assert_eq!(status(&ui, "bar").displayed, 50.0);
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 80.0);
    assert!(!status(&ui, "bar").running);
    // A completed (or hidden) gauge must not invalidate the layout every frame.
    let generation = lua::ui(&ui.lua).tree.generation;
    lua::tick(&ui.lua, 100.0);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
}

#[test]
fn same_target_keeps_animation_and_interruption_uses_remaining_duration() {
    let ui = setup();
    run(
        &ui,
        "bar:SetMinMaxValues(0,100);bar:SetValue(100);bar:SetValue(40,true,0.3)",
    );
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 80.0);
    run(&ui, "bar:SetValue(40,false,0)");
    assert!(status(&ui, "bar").animate);
    // Actual native oracle: new duration 1s is ignored here; remaining old duration is ~0.2s.
    run(&ui, "bar:SetValue(20,true,1)");
    assert_eq!(status(&ui, "bar").speed.to_bits(), 299.99997_f32.to_bits());
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 50.0);
    lua::tick(&ui.lua, 100.0);
    assert_eq!(
        status(&ui, "bar").displayed.to_bits(),
        20.000002_f32.to_bits()
    );
    lua::tick(&ui.lua, 1.0);
    assert_eq!(status(&ui, "bar").displayed, 20.0);
    assert!(!status(&ui, "bar").running);
}

#[test]
fn zero_and_hidden_owner_preserve_animation_state_then_resume() {
    let ui = setup();
    run(
        &ui,
        "bar:SetMinMaxValues(0,100);bar:SetValue(100);bar:SetValue(0,true,0.3)",
    );
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 100.0);
    assert!(!status(&ui, "bar").running);
    run(&ui, "bar:SetValue(50,true,0.2)");
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 75.0);
    run(&ui, "bar:Show(false)");
    let generation = lua::ui(&ui.lua).tree.generation;
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 75.0);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    run(&ui, "bar:Show(true)");
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "bar").displayed, 50.0);
}

#[test]
fn range_setter_preserves_target_ignores_reversed_ranges_and_starts_at_zero() {
    let ui = setup();
    run(
        &ui,
        r#"
        local lo,hi=bar:GetMinMaxValues();assert(lo==0 and hi==0)
        bar:SetValue(50);assert(bar:GetValue()==0)
        bar:SetMinMaxValues(0,100);bar:SetValue(80)
        bar:SetMinMaxValues(20,60);assert(bar:GetValue()==80)
        bar:SetMinMaxValues(50,10)
        lo,hi=bar:GetMinMaxValues();assert(lo==20 and hi==60)
        bar:SetValue(100);assert(bar:GetValue()==60)
    "#,
    );
}

#[test]
fn native_optional_animation_arguments_require_boolean_and_numeric_seconds() {
    let ui = setup();
    run(
        &ui,
        r#"
        bar:SetMinMaxValues(0,100);bar:SetValue(20)
        bar:SetValue(40,1,0.2)
    "#,
    );
    assert_eq!(status(&ui, "bar").displayed, 40.0);
    run(&ui, "bar:SetValue(60,true,'0.2')");
    assert_eq!(status(&ui, "bar").displayed, 60.0);
    run(&ui, "bar:SetValue(80,true,0)");
    assert_eq!(status(&ui, "bar").displayed, 80.0);
}

#[test]
fn unchanged_player_hp_method_animates_gain_and_after_image_loss() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    healthbar::install(&mut ui);
    run(&ui, "playerFrame=nativeHealthFrame");
    healthbar::lose_health(&ui);
    assert_eq!(status(&ui, "playerFrame.hpBar.statusBar").displayed, 40.0);
    assert_eq!(
        status(&ui, "playerFrame.hpBar.statusBarAfterImage").displayed,
        100.0
    );
    lua::tick(&ui.lua, 100.0);
    assert_eq!(
        status(&ui, "playerFrame.hpBar.statusBarAfterImage").displayed,
        80.0
    );
    lua::tick(&ui.lua, 201.0);
    run(&ui, "FAKE.player.hp=80;playerFrame:SetHp(80,100)");
    assert_eq!(status(&ui, "playerFrame.hpBar.statusBar").value, 80);
    assert_eq!(status(&ui, "playerFrame.hpBar.statusBar").displayed, 40.0);
    assert_eq!(
        status(&ui, "playerFrame.hpBar.statusBarAfterImage").displayed,
        80.0
    );
    lua::tick(&ui.lua, 100.0);
    assert_eq!(status(&ui, "playerFrame.hpBar.statusBar").displayed, 60.0);
}
