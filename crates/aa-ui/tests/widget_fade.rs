//! Native Show/SetAlpha wrappers and the independent manager factor (87 executed controls).
use aa_ui::{UiRuntime, input, lua, render};

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec("@fade_setup", r#"
        w=UIParent:CreateWidget('window','nativeFade','UIParent')
        w:SetExtent(100,100);w:AddAnchor('TOPLEFT',UIParent,20,20);w:Show(true)
        art=w:CreateColorDrawable(1,1,1,1,'background')
        art:AddAnchor('TOPLEFT',w,0,0);art:AddAnchor('BOTTOMRIGHT',w,0,0)
        log={}
        for _,event in ipairs({'OnShow','OnHide','OnEndFadeIn','OnEndFadeOut'}) do
            w:SetHandler(event,function(self) table.insert(log,event..':'..tostring(self:IsVisible())) end)
        end
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@clear_setup", "log={}").unwrap();
    ui
}

fn factor(ui: &UiRuntime) -> f32 {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "w"))
        .unwrap()
        .fade_alpha
}

fn alpha(ui: &UiRuntime) -> f32 {
    render::build_quads(&mut lua::ui(&ui.lua), None)[0].color[3]
}

#[test]
fn strict_wrappers_preserve_arity_and_unclamped_negative_or_nan_alpha() {
    let ui = setup();
    ui.exec(
        "@strict",
        r#"
        assert(select('#',w:Show())==0 and w:IsVisible())
        for _,v in ipairs({'false',{},true}) do
            if type(v)~='boolean' then w:Show(v);assert(w:IsVisible()) end
            assert(select('#',w:SetAlpha(v))==0 and w:GetAlpha()==1)
        end
        assert(select('#',w:SetAlpha())==0 and w:GetAlpha()==1)
        assert(select('#',w.SetAlpha(art,.1))==0)
        assert(select('#',w.Show(art,false))==0)
        w:SetAlpha(2);assert(w:GetAlpha()==1)
        w:SetAlpha(-2);assert(w:GetAlpha()==-2)
        w:SetAlpha(0/0);assert(w:GetAlpha()~=w:GetAlpha())
        w:SetAlpha(math.huge);assert(w:GetAlpha()==1)
        w:SetAlpha(-math.huge);assert(w:GetAlpha()==-math.huge)
        assert(select('#',w:Show(0,'100'))==0 and not w:IsVisible())
    "#,
    )
    .unwrap();
    let art = id(&ui, "art");
    assert_eq!(lua::ui(&ui.lua).tree.get(art).alpha, 1.);
    assert!(lua::ui(&ui.lua).tree.get(art).shown);
    let fade = lua::ui(&ui.lua).fades[&id(&ui, "w")];
    assert_eq!(fade.start, fade.end);
    lua::tick(&ui.lua, 0.);
    ui.exec("@third_arg", "w:Show(1,100,.3)").unwrap();
    lua::tick(&ui.lua, 100.);
    assert_eq!(factor(&ui), 1.);
    ui.exec(
        "@numeric_bool",
        "w:Show(0);w:Show(0/0);assert(w:IsVisible())",
    )
    .unwrap();
}

#[test]
fn hidden_fade_paints_but_cannot_receive_pointer_and_base_alpha_survives() {
    let ui = setup();
    ui.exec("@hide", "w:SetAlpha(.4);w:Show(false,1000)")
        .unwrap();
    assert!(!eval::<bool>(&ui, "w:IsVisible()"));
    assert!(!lua::ui(&ui.lua).tree.visible(id(&ui, "w")));
    assert_ne!(input::pick(&ui.lua, 30., 30.), Some(id(&ui, "w")));
    lua::tick(&ui.lua, 500.);
    assert_eq!(factor(&ui), 0.5);
    assert_eq!(alpha(&ui), 0.4 * 0.5);
    assert_eq!(eval::<f32>(&ui, "w:GetAlpha()"), 0.4);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "OnHide:false");
    lua::tick(&ui.lua, 500.);
    assert!(render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "OnHide:false,OnEndFadeOut:false"
    );
    assert_eq!(eval::<f32>(&ui, "w:GetAlpha()"), 0.4);
}

#[test]
fn repeated_show_keeps_record_and_immediate_reversal_uses_current_factor() {
    let ui = setup();
    let w = id(&ui, "w");
    ui.exec("@fade", "w:SetAlpha(.4);w:Show(false,1000)")
        .unwrap();
    lua::tick(&ui.lua, 250.);
    let old = lua::ui(&ui.lua).fades[&w];
    ui.exec("@same", "w:Show(false,50)").unwrap();
    assert_eq!(lua::ui(&ui.lua).fades[&w].end, old.end);
    assert_eq!(factor(&ui), 0.75);
    ui.exec("@reverse", "w:Show(true)").unwrap();
    let new = lua::ui(&ui.lua).fades[&w];
    assert_eq!(new.from, 0.75);
    assert_eq!(new.start, new.end);
    assert_eq!(factor(&ui), 0.75);
    lua::tick(&ui.lua, 0.);
    assert_eq!(factor(&ui), 1.);
    assert_eq!(alpha(&ui), 0.4);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "OnHide:false,OnShow:true,OnEndFadeIn:true"
    );
}

#[test]
fn show_and_set_alpha_preserve_internal_running_alpha_and_scale() {
    let ui = setup();
    ui.exec(
        "@animate",
        r#"
        w:SetAlphaAnimation(.5,1,.1,0)
        w:SetScaleAnimation(1.5,1,.1,0,'CENTER')
        w:SetStartAnimation(true,true);w:Show(false,1000);w:Show(true,1000)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    let w = id(&ui, "w");
    let before = lua::ui(&ui.lua).animations.widget_sample(w);
    assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
    ui.exec("@override", "w:SetAlpha(.1);w:Show(true)").unwrap();
    assert_eq!(eval::<f32>(&ui, "w:GetAlpha()"), 0.1);
    assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
    assert_eq!(
        lua::ui(&ui.lua).animations.widget_sample(w).scale,
        before.scale
    );
    lua::tick(&ui.lua, 20.);
    let after = lua::ui(&ui.lua).animations.widget_sample(w);
    assert!((after.alpha.unwrap() - 0.7).abs() < 0.00001);
    assert!(after.scale < before.scale);
}

#[test]
fn fade_frame_handles_negative_nan_infinite_and_absolute_f32_clock() {
    let early = setup();
    early.exec("@early", "w:Show(false,1000)").unwrap();
    lua::tick(&early.lua, -250.);
    assert_eq!(factor(&early), 1.25); // Native frame does not lower-clamp progress.
    for duration in ["-100", "0/0", "-math.huge"] {
        let ui = setup();
        ui.exec("@duration", &format!("w:Show(false,{duration})"))
            .unwrap();
        lua::tick(&ui.lua, 0.);
        assert_eq!(factor(&ui), 0., "{duration}");
        assert!(!lua::ui(&ui.lua).tree.widget(id(&ui, "w")).unwrap().fading);
    }
    let ui = setup();
    lua::tick(&ui.lua, 16_777_216.);
    ui.exec("@clock", "w:Show(false,3)").unwrap();
    let w = id(&ui, "w");
    let f = lua::ui(&ui.lua).fades[&w];
    assert_eq!(f.end - f.start, 4.);
    lua::tick(&ui.lua, 1.); // f32 accumulated clock cannot advance by one here.
    assert_eq!(factor(&ui), 1.);
    lua::tick(&ui.lua, 2.);
    assert_eq!(factor(&ui), 0.5);
    ui.exec("@infinity", "w:Show(true,math.huge)").unwrap();
    lua::tick(&ui.lua, 1000.);
    assert_eq!(factor(&ui), 0.5);
}

#[test]
fn fading_parent_keeps_child_clip_layer_and_parent_first_alpha_product() {
    let ui = setup();
    ui.exec(
        "@nested",
        r#"
        w:SetUILayer('background');w:SetAlpha(.83);w:EnableScroll(true)
        c=w:CreateChildWidget('button','nested',0,true)
        c:SetExtent(100,100);c:AddAnchor('TOPLEFT',w,50,50);c:SetAlpha(.67)
        local d=c:CreateColorDrawable(1,1,1,1,'artwork')
        d:AddAnchor('TOPLEFT',c,0,0);d:AddAnchor('BOTTOMRIGHT',c,0,0)
        c:Show(false,1000);w:Show(false,1000)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 250.);
    let (quads, background) = render::build_quads_split(&mut lua::ui(&ui.lua), None);
    assert_eq!(background, quads.len());
    assert_eq!(quads.len(), 2);
    assert_eq!(
        quads[1].color[3].to_bits(),
        ((0.75f32 * 0.83) * (0.75f32 * 0.67)).to_bits()
    );
    assert_eq!(quads[1].pos, [70., 70., 120., 120.]);
    assert_ne!(input::pick(&ui.lua, 80., 80.), Some(id(&ui, "c")));
    lua::tick(&ui.lua, 750.);
    assert!(render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
}

#[test]
fn fade_out_completion_keeps_widgets_valid_for_callback_then_retires_descendants() {
    let ui = setup();
    ui.exec(
        "@retire",
        r#"
        c=w:CreateChildWidget('editbox','retiredEdit',0,true)
        c:SetExtent(50,20);c:SetFocus()
        w:EnableHidingIsRemove(true)
        w:SetHandler('OnEndFadeOut',function(self)
            assert(not self:IsVisible() and self:GetAlpha()==1 and c:GetId()~=nil)
            finished=true
        end)
        w:Show(false,100)
    "#,
    )
    .unwrap();
    let w = id(&ui, "w");
    let c = id(&ui, "c");
    lua::tick(&ui.lua, 50.);
    assert!(lua::ui(&ui.lua).tree.get(w).alive);
    lua::tick(&ui.lua, 50.);
    assert!(eval::<bool>(&ui, "finished"));
    let u = lua::ui(&ui.lua);
    assert!(!u.tree.get(w).alive && !u.tree.get(c).alive);
    assert!(!u.fades.contains_key(&w));
    assert!(u.focus.is_none());
}

#[test]
fn completion_callback_can_replace_its_manager_record_without_rollback() {
    let ui = setup();
    ui.exec(
        "@reenter",
        r#"
        w:SetHandler('OnEndFadeOut',function(self) self:Show(true,100) end)
        w:Show(false,100)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 100.);
    assert!(eval::<bool>(&ui, "w:IsVisible()"));
    assert_eq!(factor(&ui), 0.);
    assert!(lua::ui(&ui.lua).fades.contains_key(&id(&ui, "w")));
    lua::tick(&ui.lua, 50.);
    assert_eq!(factor(&ui), 0.5);
}

#[test]
fn manager_completion_precedes_the_same_frame_widget_channel_step() {
    let ui = setup();
    ui.exec("@hide_before_order", "w:Show(false)").unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec(
        "@native_frame_order",
        r#"
        w:SetAlphaAnimation(0,1,.1,0);w:SetStartAnimation(true,false)
        w:SetHandler('OnEndFadeIn',function(self)
            observed=self:GetAlpha()
            self:SetAlphaAnimation(.2,1,.1,0);self:SetStartAnimation(true,false)
        end)
        w:Show(true,20)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<f32>(&ui, "observed"), 0.);
    assert!((eval::<f32>(&ui, "w:GetAlpha()") - 0.36).abs() < 0.00001);
}

#[test]
fn manager_registration_order_keeps_replacement_position_and_observes_callback_mutation() {
    let ui = setup();
    ui.exec(
        "@manager_order",
        r#"
        a=UIParent:CreateWidget('window','earlyAllocated','UIParent')
        b=UIParent:CreateWidget('window','laterAllocated','UIParent')
        order={}
        b:SetHandler('OnEndFadeIn',function()
            table.insert(order,'b');assert(a:GetAlpha()==1)
            -- A later record is changed before its frame evaluation.
            a:Show(false,200)
        end)
        a:SetHandler('OnEndFadeOut',function() table.insert(order,'a') end)
        b:Show(true,20);a:Show(true,20)
        -- Replacement retains b's earlier manager position, despite allocation order.
        b:Show(false,20);b:Show(true,20)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<String>(&ui, "table.concat(order,',')"), "b");
    let a = id(&ui, "a");
    assert_eq!(lua::ui(&ui.lua).tree.widget(a).unwrap().fade_alpha, 1.);
    lua::tick(&ui.lua, 200.);
    assert_eq!(eval::<String>(&ui, "table.concat(order,',')"), "b,a");
}

#[test]
fn completion_callback_appends_an_immediate_record_which_finishes_in_the_same_frame() {
    let ui = setup();
    ui.exec(
        "@append_order",
        r#"
        w:SetHandler('OnEndFadeOut',function(self) self:Show(true) end)
        w:Show(false,20)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(factor(&ui), 1.);
    assert!(!lua::ui(&ui.lua).fades.contains_key(&id(&ui, "w")));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "OnHide:false,OnShow:true,OnEndFadeIn:true"
    );
}

#[test]
fn unchanged_pending_factor_still_repaints_admitted_hidden_animation() {
    let ui = setup();
    ui.exec(
        "@constant_factor",
        r#"
        w:SetAlphaAnimation(0,1,.1,0);w:SetStartAnimation(true,false)
        w:Show(false,math.huge)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    let content = lua::ui(&ui.lua).tree.settle();
    lua::tick(&ui.lua, 20.);
    assert_eq!(factor(&ui), 1.);
    assert!(lua::ui(&ui.lua).tree.settle() > content);
    assert!((alpha(&ui) - 0.2).abs() < 0.00001);
}
