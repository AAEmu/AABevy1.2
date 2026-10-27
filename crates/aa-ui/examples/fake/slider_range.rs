//! Complete unchanged original upgrade notifier; host data uses the existing 16064 fixture.
use aa_ui::{UiRuntime, lua};

#[path = "mod.rs"]
#[allow(dead_code)]
mod fake;

pub fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.load_ui_texts(std::path::Path::new(&std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into())))
    .expect("ADMISSION: original UI texts");
    ui.load_base_libs();
    assert!(
        ui.errors.is_empty(),
        "ADMISSION: base scripts {:?}",
        ui.errors
    );
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/skill/skill_info_notify.alb")
        .expect("ADMISSION: complete original notifier ALB");
    lua::flush(&ui.lua);
    ui.exec(
        "@range_supplier_admission",
        r#"
        UIParent:SetUIScale(1)
        assert(type(locale.skill.GetSkillChangedText)=='function')
        assert(type(CreateActionSlot)=='function' and type(ResetScroll)=='function')
        local tip=X2Skill:GetSkillTooltip(16064,0)
        assert(tip.type==16064 and type(tip.name)=='string' and tip.name~='')
        assert(tip.skillLevel==1)
    "#,
    )
    .expect("ADMISSION: original locale/action-slot/known-skill suppliers");
}

pub fn upgrade(ui: &UiRuntime) {
    // This is exactly route_book's SKILL_UPGRADED sink shape, not SKILL_CHANGED's tuple.
    ui.fire_event("SKILL_UPGRADED", 16064_i64);
}

pub fn expose_original(ui: &UiRuntime) {
    let id = lua::ui(&ui.lua)
        .tree
        .find("skillInfoNotifier")
        .expect("ADMISSION: original ShowNotifyer created the named singleton");
    let object = lua::object_table(&ui.lua, id)
        .expect("ADMISSION: original object lookup")
        .expect("ADMISSION: original object table");
    ui.lua.globals().set("rangeNotifier", object).unwrap();
    ui.exec(
        "@range_original_observation_aliases",
        r#"
        assert(rangeNotifier:IsVisible())
        assert(type(rangeNotifier.AddChild)=='function' and type(rangeNotifier.UpdateScroll)=='function')
        rangeScroll=rangeNotifier.scrollWnd
        rangeContent=rangeScroll.content
        rangeSlider=rangeScroll.scroll.vs
        local child=rangeNotifier.children[#rangeNotifier.children]
        assert(child and child.skillBtn and child.skillBtn:GetBindedType()=='skill')
        assert(child:GetText()~='')
    "#,
    )
    .expect("ADMISSION: real original child, action slot, scroll and handlers");
}

#[cfg(not(test))]
pub(crate) use fake::{config, physical_pixels};

fn scene_eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn scene_bounds(ui: &UiRuntime) -> [f32; 5] {
    scene_eval(
        ui,
        r#"
        local f=rangeScroll;local last=rangeNotifier.children[#rangeNotifier.children]
        local _,fy=f:GetOffset();local _,fh=f:GetExtent()
        local _,ly=last:GetOffset();local _,lh=last:GetExtent()
        local _,cy=f.content:GetOffset();local _,ch=f.content:GetExtent()
        local _,hi=f.scroll.vs:GetMinMaxValues()
        return {hi,math.max(0,math.floor(ly+lh-fy+10-fh)),ly+lh-cy,ch,#rangeNotifier.children}
    "#,
    )
}
fn scene_wheel(ui: &UiRuntime, mouse: &mut aa_ui::input::MouseState) {
    let r: [f32; 4] = scene_eval(
        ui,
        "local x,y=rangeContent:GetEffectiveOffset();local w,h=rangeContent:GetEffectiveExtent();return {x,y,w,h}",
    );
    let scale = lua::ui(&ui.lua).ui_scale;
    aa_ui::input::update(
        &ui.lua,
        mouse,
        Some(((r[0] + r[2] / 2.) / scale, (r[1] + r[3] / 2.) / scale)),
        false,
        false,
        false,
        -1.,
    );
}

/// Used by the copied visual example and its CPU control; original methods remain unchanged.
pub fn visible_range_update(ui: &UiRuntime) -> [f32; 5] {
    for _ in 0..12 {
        upgrade(ui);
        expose_original(ui);
        let b = scene_bounds(ui);
        if b[4] >= 8. && b[0] >= 80. {
            break;
        }
    }
    let b = scene_bounds(ui);
    assert!(b[4] >= 8. && b[0] >= 80.);
    let mut mouse = aa_ui::input::MouseState::default();
    scene_wheel(ui, &mut mouse);
    let old: f32 = scene_eval(ui, "rangeSlider:GetValue()");
    assert!(old > 0.);
    upgrade(ui);
    expose_original(ui);
    let after = scene_bounds(ui);
    assert!((after[0] - after[1]).abs() < 0.01);
    for _ in 0..64 {
        let value: f32 = scene_eval(ui, "rangeSlider:GetValue()");
        if value >= after[0] - 0.01 {
            break;
        }
        scene_wheel(ui, &mut mouse);
    }
    let end = scene_bounds(ui);
    assert!(end[2] <= end[3] + 1.);
    [old, after[0], after[1], end[2], end[3]]
}
