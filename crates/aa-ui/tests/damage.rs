//! Native damagedisplay timelines and the app's projected target position hook.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{
    UiRuntime, lua,
    render::{Quad, Tex},
};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn setup(frames: &str) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@setup",
        r#"
        d=UIParent:CreateWidget('damagedisplay','damage','UIParent')
        d:SetExtent(142,82); d:Show(false); d:Clickable(false)
        d.style:SetFont('ui/font/yoon_fired.ttf',40); d.style:SetColor(1,0.5,0.2,1)
        d:SetText('128'); d:SetUnitId('source','target'); d:SetInitPos(10.9,-20.9)
        px,py,culled=800,300,false; calls=0
        AAUI.CombatTextPosition=function(source,target)
            calls=calls+1; seen=source..'/'..target
            if culled then return nil end
            return {x=px,y=py}
        end
    "#,
    )
    .unwrap();
    ui.exec(
        "@frames",
        &format!("d:SetAnimFrameInfo({frames}); d:Animation(true)"),
    )
    .unwrap();
    ui
}

fn quads(ui: &UiRuntime) -> Vec<Quad> {
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| q.tex == Tex::Atlas && q.color[0] == 1.0 && q.color[1] == 0.5)
        .collect()
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.001, "{a} != {b}");
}

#[test]
fn frames_hold_interpolate_advance_once_follow_target_and_stop_without_layout_churn() {
    let ui = setup(
        "{{scale=1,alpha=0.2,showTime=100,timeToNextFrame=100},{scale=0.5,alpha=1,showTime=100},{alpha=0,showTime=100}}",
    );
    assert_eq!(eval::<String>(&ui, "seen"), "source/target");
    let id = lua::ui(&ui.lua).tree.find("damage").unwrap();
    let center = || {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, id);
        [r.x + r.w * 0.5, r.y + r.h * 0.5]
    };
    assert_eq!(center(), [810., 280.]); // integer native SetInitPos
    let content = lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    lua::tick(&ui.lua, 50.);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    ui.exec("@target", "px=900; py=400").unwrap();
    lua::tick(&ui.lua, 100.);
    assert_eq!(center(), [910., 380.]);
    assert!(quads(&ui).iter().all(|q| (q.color[3] - 0.6).abs() < 0.001));
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    // A large elapsed interval consumes one phase; overshoot is discarded.
    lua::tick(&ui.lua, 1000.);
    assert!(eval::<bool>(&ui, "d:IsVisible()"));
    assert!(quads(&ui).iter().all(|q| (q.color[3] - 1.).abs() < 0.001));
    lua::tick(&ui.lua, 99.);
    assert!(eval::<bool>(&ui, "d:IsVisible()"));
    lua::tick(&ui.lua, 1.);
    assert!(quads(&ui).iter().all(|q| q.color[3] == 0.));
    lua::tick(&ui.lua, 100.);
    assert!(!eval::<bool>(&ui, "d:IsVisible()"));
    let calls = eval::<u32>(&ui, "calls");
    lua::tick(&ui.lua, 1000.);
    assert_eq!(eval::<u32>(&ui, "calls"), calls);
}

#[test]
fn count_reveal_afterimages_and_culling_use_native_frame_rules() {
    let ui = setup("{{animType=2,timeToNextFrame=100},{showTime=500}}");
    lua::tick(&ui.lua, 50.);
    let counted = quads(&ui);
    ui.exec(
        "@compare",
        "d:SetText('64'); d:SetAnimFrameInfo({{showTime=1000}})",
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    let plain = quads(&ui);
    assert_eq!(plain.len(), counted.len());
    for (a, b) in plain.iter().zip(counted) {
        assert_eq!((a.pos, a.uv, a.color), (b.pos, b.uv, b.color));
    }
    // Missing projection suppresses all attached art while the finite timeline still ends.
    ui.exec("@cull", "culled=true").unwrap();
    lua::tick(&ui.lua, 2000.);
    assert!(quads(&ui).is_empty());
    assert!(!eval::<bool>(&ui, "d:IsVisible()"));
    ui.exec("@uncull", "culled=false").unwrap();
    lua::tick(&ui.lua, 0.);
    assert!(!eval::<bool>(&ui, "d:IsVisible()"));
    ui.exec(
        "@reveal",
        r#"
        d:SetText('A BC'); d.extraStyle:SetFont('ui/font/yoon_firedgothic_b.ttf',40)
        d:SetAnimFrameInfo({{animType=20,showTime=1000,linearDisplayTime=100,
            afterImage={{time=300,x=10,y=-4,alpha=0.15,scale=1.8}}}})
        d:Animation(true)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 100.);
    let ghosts = quads(&ui);
    assert!(ghosts.iter().any(|q| (q.color[3] - 0.15).abs() < 0.001));
    let main: Vec<_> = ghosts.iter().filter(|q| q.color[3] == 1.).collect();
    // At 100 ms two visible letters plus the intervening space: "A B".
    assert_eq!(main.len(), 2);
    lua::tick(&ui.lua, 201.);
    let later = quads(&ui);
    assert!(later.iter().all(|q| q.color[3] == 1.));
    assert_eq!(later.len(), 3);
}

#[test]
fn movement_is_text_only_and_restarting_active_animation_is_a_noop() {
    let ui = setup("{{animType=1,moveX=100,moveY=-40,timeToNextFrame=100},{showTime=100}}");
    ui.exec("@art", r#"
        art=d:CreateColorDrawable(1,1,1,1,'artwork'); art:SetExtent(10,10); art:AddAnchor('CENTER',d,0,0)
    "#).unwrap();
    let before = quads(&ui);
    lua::tick(&ui.lua, 50.);
    let half = quads(&ui);
    near(half[0].pos[0] - before[0].pos[0], 50.);
    near(half[0].pos[1] - before[0].pos[1], -20.);
    ui.exec("@again", "d:Animation(true)").unwrap();
    lua::tick(&ui.lua, 25.);
    let later = quads(&ui);
    near(later[0].pos[0] - before[0].pos[0], 75.);
    let image = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .find(|q| q.color == [1.0; 4])
        .unwrap();
    assert_eq!(image.pos, [805., 275., 815., 285.]);
    ui.exec("@stop", "d:Animation(false)").unwrap();
    assert!(!eval::<bool>(&ui, "d:IsVisible()"));
    let calls = eval::<u32>(&ui, "calls");
    lua::tick(&ui.lua, 1000.);
    assert_eq!(eval::<u32>(&ui, "calls"), calls);
}

#[test]
fn native_shake_uses_remaining_time_half_periods_then_stops() {
    let ui = setup("{{animType=8,showTime=200,shakeTime=100,shakeCount=2,shakeX=6,shakeY=-4}}");
    let before = quads(&ui);
    lua::tick(&ui.lua, 20.);
    let negative = quads(&ui);
    near(negative[0].pos[0] - before[0].pos[0], -12.);
    near(negative[0].pos[1] - before[0].pos[1], 8.);
    lua::tick(&ui.lua, 10.);
    assert_eq!(quads(&ui)[0].pos, before[0].pos);
    lua::tick(&ui.lua, 71.);
    let stopped = quads(&ui);
    near(stopped[0].pos[0] - before[0].pos[0], -6.);
    near(stopped[0].pos[1] - before[0].pos[1], 4.);
}

#[test]
fn original_combat_text_uses_authored_phases_and_releases_pool_slot() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@combat",
        r#"
        AAUI.CombatTextPosition=function(source,target) return {x=800,y=400} end
    "#,
    )
    .unwrap();
    ui.fire_event(
        "COMBAT_TEXT",
        ("source", "enemy", 128, "SWING", "CRITICAL", 1, false, 0.0),
    );
    ui.exec("@active", r#"
        active=0
        for i,w in ipairs(combatTextFrame.combatTexts) do if w:IsVisible() then active=active+1; activeText=w end end
    "#).unwrap();
    assert_eq!(eval::<u32>(&ui, "active"), 1);
    assert!(eval::<bool>(&ui, "activeText.drawable:IsVisible()"));
    assert!(
        aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
            .iter()
            .any(|q| q.color[3] > 0.)
    );
    // Native one-phase-per-render rule: each call must let the authored frame advance.
    for _ in 0..20 {
        lua::tick(&ui.lua, 100.);
    }
    assert!(!eval::<bool>(&ui, "activeText:IsVisible()"));
    let own: String = eval(&ui, "X2Unit:GetUnitId('player')");
    ui.fire_event("UNIT_COMBAT_STATE_CHANGED", (true, own));
    lua::tick(&ui.lua, 100.);
    assert!(eval::<bool>(
        &ui,
        "for _,w in ipairs(combatTextFrame.combatTexts) do if w:IsVisible() then return true end end return false"
    ));
}

#[test]
fn original_offscreen_pool_finishes_and_reuses_slots_with_fresh_units() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@missing_projection",
        r#"
        culled=true
        AAUI.CombatTextPosition=function(source,target)
            calls=(calls or 0)+1
            lastSource,lastTarget=source,target
            if not culled then return {x=800,y=400} end
        end
    "#,
    )
    .unwrap();
    let count: usize = eval(&ui, "#combatTextFrame.combatTexts-1");
    for _ in 0..count + 3 {
        ui.fire_event(
            "COMBAT_TEXT",
            (
                "old-source",
                "old-enemy",
                -22,
                "SWING",
                "HIT",
                0,
                false,
                0.0,
            ),
        );
    }
    let shown = || {
        eval::<usize>(
            &ui,
            "local n=0; for _,w in ipairs(combatTextFrame.combatTexts) do if w:IsVisible() then n=n+1 end end return n",
        )
    };
    assert_eq!(shown(), count);
    assert!(eval::<bool>(
        &ui,
        "for _,w in ipairs(combatTextFrame.combatTexts) do if w:IsVisible() then return true end end return false"
    ));
    for _ in 0..20 {
        lua::tick(&ui.lua, 1000.);
    }
    assert_eq!(shown(), 0);
    assert!(!eval::<bool>(
        &ui,
        "for _,w in ipairs(combatTextFrame.combatTexts) do if w:IsVisible() then return true end end return false"
    ));
    let calls = eval::<u32>(&ui, "calls");
    lua::tick(&ui.lua, 1000.);
    assert_eq!(eval::<u32>(&ui, "calls"), calls);
    ui.fire_event(
        "COMBAT_TEXT",
        (
            "new-source",
            "new-enemy",
            -77,
            "SWING",
            "HIT",
            0,
            false,
            0.0,
        ),
    );
    assert_eq!(shown(), 1);
    assert_eq!(eval::<String>(&ui, "lastSource"), "new-source");
    assert_eq!(eval::<String>(&ui, "lastTarget"), "new-enemy");
    ui.exec("@same_frame_projection", "culled=false").unwrap();
    lua::tick(&ui.lua, 0.);
    assert_eq!(shown(), 1);
    for _ in 0..20 {
        lua::tick(&ui.lua, 1000.);
    }
    assert_eq!(shown(), 0);
}
