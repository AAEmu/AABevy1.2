//! Native stored tab-alpha values never rewrite the generic draw state.
#[path = "../examples/native_chat_split.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, input, lua, render, tree::Id};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_tab_alpha", code).unwrap();
}
fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(code).eval().unwrap()).unwrap()
}
fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        callbacks=0
        function OnCreateChatMessage(m) m:AddMessage('retained history') end
        function OnCreateChatTabButton(b)
            made=made or {};made[#made+1]=b
            b:SetAlpha(0.4)
            b.art=b:CreateColorDrawable(0.23,0.41,0.67,0.8,'background')
            b.art:AddAnchor('TOPLEFT',b,0,0);b.art:AddAnchor('BOTTOMRIGHT',b,0,0)
        end
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=2,name='Harbor'}} end,
            ChatSelection=function() callbacks=callbacks+1 end}
        window=UIParent:CreateWidget('chatwindow','tabAlpha','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:SetChatWindowId(0);window:AllowTabSwitch(true);window:Show(true)
        for _,b in ipairs(made) do b:SetAlpha(0.4) end
        window:GetAddButton():SetAlpha(0.35)
        callbacks=0
    "#,
    );
    let window = id(&ui, "window");
    // ColorDrawable has no public SetAlpha in the native class surface. Seed
    // its common draw-state factor directly; the chat frame must leave it alone.
    for tab in tabs(&ui, window) {
        let draw = lua::ui(&ui.lua).tree.widget(tab.button).unwrap().drawables[0];
        lua::ui(&ui.lua).tree.get_mut(draw).alpha = 0.7;
    }
    (ui, window)
}

fn tabs(ui: &UiRuntime, window: Id) -> Vec<lua::chat::ChatTab> {
    lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs
        .clone()
}
fn alphas(ui: &UiRuntime, window: Id) -> Vec<f32> {
    let u = lua::ui(&ui.lua);
    let c = u.tree.widget(window).unwrap().chat.as_ref().unwrap();
    c.tabs
        .iter()
        .flat_map(|t| {
            let w = u.tree.widget(t.button).unwrap();
            std::iter::once(u.tree.get(t.button).alpha)
                .chain(w.drawables.iter().map(|&d| u.tree.get(d).alpha))
        })
        .chain(std::iter::once(u.tree.get(c.add_button).alpha))
        .collect()
}

#[test]
fn idle_hover_and_time_preserve_explicit_tab_and_drawable_alpha() {
    let (ui, window) = setup();
    let before = alphas(&ui, window);
    let rows = tabs(&ui, window);
    let mut mouse = Default::default();
    for (point, dt) in [
        ((900., 500.), 0.),
        ((200., 180.), 1.),
        ((900., 500.), 1000.),
    ] {
        input::update(&ui.lua, &mut mouse, Some(point), false, false, false, 0.);
        lua::tick(&ui.lua, dt);
        assert_eq!(alphas(&ui, window), before, "point={point:?},dt={dt}");
        assert_eq!(tabs(&ui, window), rows);
    }
    assert_eq!(ui.lua.load("callbacks").eval::<i32>().unwrap(), 0);
}

#[test]
fn pair_is_visually_inactive_while_global_layers_and_draw_restore_still_apply_once() {
    let (ui, window) = setup();
    lua::tick(&ui.lua, 0.);
    let before = render::build_quads(&mut lua::ui(&ui.lua), None);
    let art = before
        .iter()
        .find(|q| q.color[..3] == [0.23, 0.41, 0.67])
        .unwrap();
    assert!((art.color[3] - 0.8 * 0.7 * 0.4 * 0.5).abs() < 1e-6);
    for pair in ["0.05,0.95", "1.5,-0.25", "0,0"] {
        run(
            &ui,
            &format!("assert(select('#',window:SetTabButtonAlpha({pair}))==0)"),
        );
        lua::tick(&ui.lua, 1000.);
        assert_eq!(paint(&ui), fingerprint(&before));
    }
    let button = rows_button(&ui, window);
    assert_eq!(lua::ui(&ui.lua).tree.get(button).alpha, 0.4);
    lua::chat::set_alpha_options(&ui.lua, 0.9, 0.25);
    let quads = render::build_quads(&mut lua::ui(&ui.lua), None);
    let art = quads
        .iter()
        .find(|q| q.color[..3] == [0.23, 0.41, 0.67])
        .unwrap();
    assert!((art.color[3] - 0.8 * 0.7 * 0.4 * 0.25).abs() < 1e-6);
    assert_eq!(alphas(&ui, window), vec![0.4, 0.7, 0.4, 0.7, 0.35]);
}

fn rows_button(ui: &UiRuntime, window: Id) -> Id {
    tabs(ui, window)[1].button
}

#[test]
fn original_bytecode_idle_art_and_names_retain_their_native_factors() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    let window = original::install(&mut ui);
    let rows = tabs(&ui, window);
    lua::ui(&ui.lua).cursor = (1100., 40.);
    lua::tick(&ui.lua, 600.);
    let values = alphas(&ui, window);
    assert!(values.iter().all(|&a| a == 1.), "{values:?}");
    let before = render::build_quads(&mut lua::ui(&ui.lua), None);
    run(&ui, "splitSource:SetTabButtonAlpha(0.15,0.85)");
    lua::tick(&ui.lua, 0.);
    assert_eq!(alphas(&ui, window), values);
    assert_eq!(tabs(&ui, window), rows);
    assert_eq!(paint(&ui), fingerprint(&before));
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

#[test]
fn physical_split_release_and_reentrant_setter_keep_rows_history_and_layer_restore() {
    let (ui, window) = setup();
    lua::tick(&ui.lua, 0.);
    let row = tabs(&ui, window)[1].clone();
    let history = lua::ui(&ui.lua)
        .tree
        .widget(row.message)
        .unwrap()
        .message
        .clone()
        .unwrap()
        .lines;
    ui.lua
        .globals()
        .set(
            "moving",
            lua::object_table(&ui.lua, row.button).unwrap().unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        moving:SetHandler('OnDragStart',function() window:SetTabButtonAlpha(1.5,-.25) end)
        moving:SetHandler('OnDragStop',function(self) self:GetParent():SetTabButtonAlpha(.05,.95) end)
    "#,
    );
    let mut mouse = Default::default();
    let r = lua::ui(&ui.lua).tree.rect(row.button);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(
        &ui.lua,
        &mut mouse,
        Some((p.0 + 5., p.1)),
        false,
        false,
        false,
        0.,
    );
    lua::tick(&ui.lua, 0.);
    let duplicate = lua::ui(&ui.lua).tree.get(row.button).parent.unwrap();
    assert_ne!(duplicate, window);
    assert_eq!(tabs(&ui, duplicate), vec![row.clone()]);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(row.message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines,
        history
    );
    assert_eq!(lua::ui(&ui.lua).tree.get(row.button).alpha, 0.4);
    let draw = lua::ui(&ui.lua).tree.widget(row.button).unwrap().drawables[0];
    assert_eq!(lua::ui(&ui.lua).tree.get(draw).alpha, 0.7);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(duplicate)
            .unwrap()
            .dynamic_drawable
            .enabled[..3],
        [false; 3]
    );
    input::update(
        &ui.lua,
        &mut mouse,
        Some((900., 500.)),
        false,
        true,
        false,
        0.,
    );
    lua::tick(&ui.lua, 0.);
    assert_eq!(tabs(&ui, duplicate), vec![row.clone()]);
    assert_eq!(lua::ui(&ui.lua).tree.get(row.button).alpha, 0.4);
    assert_eq!(lua::ui(&ui.lua).tree.get(draw).alpha, 0.7);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(duplicate)
            .unwrap()
            .dynamic_drawable
            .enabled[..3],
        [true; 3]
    );
    let layers = lua::ui(&ui.lua)
        .tree
        .widget(row.button)
        .unwrap()
        .dynamic_drawable
        .current;
    // The moved window follows the release point, so it is still under the
    // pointer. The normal frame then follows a real leave after restoration.
    assert_eq!(layers[..3], [0.7; 3]);
    input::update(&ui.lua, &mut mouse, Some((0., 0.)), false, false, false, 0.);
    lua::tick(&ui.lua, 0.);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(row.button)
            .unwrap()
            .dynamic_drawable
            .current[..3],
        [0.5; 3]
    );
    run(
        &ui,
        "moving:EnableHidingIsRemove(true);moving:Show(false);window:SetTabButtonAlpha(0,0)",
    );
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(row.button).alive);
    assert!(lua::ui(&ui.lua).tree.get(row.message).alive);
}

fn fingerprint(quads: &[render::Quad]) -> Vec<([f32; 4], [f32; 4], [f32; 4])> {
    quads.iter().map(|q| (q.pos, q.uv, q.color)).collect()
}
fn paint(ui: &UiRuntime) -> Vec<([f32; 4], [f32; 4], [f32; 4])> {
    fingerprint(&render::build_quads(&mut lua::ui(&ui.lua), None))
}
