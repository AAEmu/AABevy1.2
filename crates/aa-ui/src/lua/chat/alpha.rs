//! Native chat options and owner-layer state; these never rewrite drawable tint or text alpha.
use super::super::{Ui, ui, value_id};
use crate::tree::Id;
use mlua::{Lua, Table, Value};

#[derive(Clone, Copy)]
struct Options {
    over: f32,
    leave: f32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            over: 0.7,
            leave: 0.5,
        }
    }
}

/// Application integration for native global chat options. Save configuration before calling.
/// Raw f32 values are retained; refreshing an unchanged pair still resets live windows to idle.
pub fn set_alpha_options(lua: &Lua, over_alpha: f32, leave_alpha: f32) {
    lua.set_app_data(Options {
        over: over_alpha,
        leave: leave_alpha,
    });
    let windows = ui(lua).tree.of_class("chatwindow");
    for window in windows {
        configure(lua, window);
        state(&mut ui(lua), window, false);
    }
}

fn pairs(u: &mut Ui, id: Id, values: [f32; 2]) {
    let Some(w) = u.tree.widget_mut(id) else {
        return;
    };
    w.dynamic_drawable.pairs = [values; 4];
    let children = w.children.clone();
    for child in children {
        pairs(u, child, values);
    }
}

pub(super) fn scroll_pairs(lua: &Lua, message: Id) {
    let Ok(parts) = super::parts(lua, message) else {
        return;
    };
    for name in ["slider", "upButton", "downButton", "downToBottomButton"] {
        if let Ok(Some(table)) = parts.raw_get::<Option<Table>>(name)
            && let Some(id) = value_id(lua, &Value::Table(table))
        {
            pairs(&mut ui(lua), id, [0., 1.]);
        }
    }
}

/// Native39512a40: recursive global pair, input exceptions, then enable three layers.
pub(super) fn configure(lua: &Lua, window: Id) {
    let Some(c) = super::chat(lua, window) else {
        return;
    };
    let options = lua
        .app_data_ref::<Options>()
        .map(|o| *o)
        .unwrap_or_default();
    pairs(&mut ui(lua), window, [options.leave, options.over]);
    for tab in &c.tabs {
        scroll_pairs(lua, tab.message);
    }
    pairs(&mut ui(lua), c.edit, [1., 1.]);
    pairs(&mut ui(lua), c.selector, [1., 1.]);
    if let Ok(parts) = super::parts(lua, window)
        && let Ok(Some(table)) = parts.raw_get::<Option<Table>>("imeToggle")
        && let Some(id) = value_id(lua, &Value::Table(table))
    {
        pairs(&mut ui(lua), id, [1., 1.]);
    }
    for layer in 0..3 {
        enable(lua, window, layer, true);
    }
}

fn propagate(u: &mut Ui, id: Id, layer: usize, over: bool) {
    if !u.tree.get(id).alive {
        return;
    }
    let Some(w) = u.tree.widget(id) else { return };
    let value = w.dynamic_drawable.pairs[layer][usize::from(over)];
    let children = w.children.clone();
    if w.dynamic_drawable.current[layer].to_bits() != value.to_bits() {
        u.tree.widget_mut(id).unwrap().dynamic_drawable.current[layer] = value;
    }
    for child in children {
        propagate(u, child, layer, over);
    }
}

pub(crate) fn state(u: &mut Ui, id: Id, over: bool) {
    let Some(w) = u.tree.widget(id) else { return };
    let enabled = w.dynamic_drawable.enabled;
    for (layer, on) in enabled.into_iter().enumerate() {
        if on {
            propagate(u, id, layer, over);
        }
    }
}

pub(super) fn force(lua: &Lua, id: Id, layer: usize, over: bool) {
    propagate(&mut ui(lua), id, layer, over);
}

pub(crate) fn hover(lua: &Lua, id: Id, over: bool) {
    state(&mut ui(lua), id, over);
}

/// Native3927d4b0: disabling holds current state; enabling snaps and propagates now.
pub(crate) fn enable(lua: &Lua, id: Id, layer: usize, on: bool) {
    let mut u = ui(lua);
    if !u.tree.get(id).alive {
        return;
    }
    let Some(w) = u.tree.widget_mut(id) else {
        return;
    };
    w.dynamic_drawable.enabled[layer] = on;
    if on {
        let over = u.entered.contains(&id);
        propagate(&mut u, id, layer, over);
    }
}

#[cfg(test)]
mod tab_alpha_tests {
    use super::*;
    use crate::UiRuntime;

    fn pair(ui: &UiRuntime, name: &str) -> [f32; 2] {
        let id = value_id(&ui.lua, &ui.lua.load(name).eval().unwrap()).unwrap();
        super::super::chat(&ui.lua, id).unwrap().tab_button_alpha
    }

    #[test]
    fn strict_pair_keeps_native_raw_floats_defaults_and_zero_results() {
        let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        ui.exec(
            "@tab_alpha",
            "window=UIParent:CreateWidget('chatwindow','alpha','UIParent')",
        )
        .unwrap();
        assert_eq!(pair(&ui, "window"), [1., 0.3]);
        for args in [
            "",
            "nil",
            "0.1",
            "0.1,nil",
            "'0.1',0.2",
            "0.1,'0.2'",
            "true,false",
            "{},0.2",
        ] {
            ui.exec(
                "@invalid_pair",
                &format!("assert(select('#',window:SetTabButtonAlpha({args}))==0)"),
            )
            .unwrap();
            assert_eq!(pair(&ui, "window"), [1., 0.3], "{args}");
        }
        ui.exec(
            "@raw_pair",
            "assert(select('#',window:SetTabButtonAlpha(1.5,-0.25,42))==0)",
        )
        .unwrap();
        assert_eq!(pair(&ui, "window"), [1.5, -0.25]);
        ui.exec("@wrong_class", "window.SetTabButtonAlpha(UIParent,0,0)")
            .unwrap();
        assert_eq!(pair(&ui, "window"), [1.5, -0.25]);
        ui.exec("@raw_zero", "window:SetTabButtonAlpha(-0,math.huge)")
            .unwrap();
        assert_eq!(pair(&ui, "window")[0].to_bits(), (-0_f32).to_bits());
        assert_eq!(pair(&ui, "window")[1], f32::INFINITY);
        ui.exec("@raw_nan", "window:SetTabButtonAlpha(0/0,-math.huge)")
            .unwrap();
        assert!(pair(&ui, "window")[0].is_nan());
        assert_eq!(pair(&ui, "window")[1], f32::NEG_INFINITY);
    }

    #[test]
    fn split_copies_pair_before_creation_then_observes_reentrant_overrides() {
        let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        ui.lua
            .globals()
            .set(
                "readTabAlpha",
                ui.lua
                    .create_function(|lua, table: Table| {
                        let id = value_id(lua, &Value::Table(table)).unwrap();
                        let pair = super::super::chat(lua, id).unwrap().tab_button_alpha;
                        Ok((pair[0], pair[1]))
                    })
                    .unwrap(),
            )
            .unwrap();
        ui.exec(
            "@split_pair",
            r#"
            AAUI={ChatTabs=function() return {{id=0,name='All'},{id=2,name='Harbor'}} end}
            window=UIParent:CreateWidget('chatwindow','alpha','UIParent')
            window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100)
            window:SetChatWindowId(0);window:SetTabButtonAlpha(1.5,-0.25)
            function OnCreateChatWindow(w)
                duplicate=w
                atCreate={readTabAlpha(w)}
                w:SetTabButtonAlpha(.6,.2)
                window:SetTabButtonAlpha(.4,.8)
            end
        "#,
        )
        .unwrap();
        let window = value_id(&ui.lua, &ui.lua.load("window").eval().unwrap()).unwrap();
        let row = super::super::chat(&ui.lua, window).unwrap().tabs[1].clone();
        super::super::transfer::split(&ui.lua, window, row.button);
        assert_eq!(
            ui.lua.load("atCreate").eval::<[f32; 2]>().unwrap(),
            [1.5, -0.25]
        );
        assert_eq!(pair(&ui, "duplicate"), [0.6, 0.2]);
        assert_eq!(pair(&ui, "window"), [0.4, 0.8]);
        let duplicate = value_id(&ui.lua, &ui.lua.load("duplicate").eval().unwrap()).unwrap();
        assert_eq!(
            super::super::chat(&ui.lua, duplicate).unwrap().tabs,
            vec![row]
        );
        ui.exec("@retire_pair", "duplicate:Show(true);duplicate:EnableHidingIsRemove(true);duplicate:Show(false);duplicate:SetTabButtonAlpha(9,9)").unwrap();
        super::super::super::tick(&ui.lua, 0.);
        assert!(!crate::lua::ui(&ui.lua).tree.get(duplicate).alive);
        ui.exec("@dead_pair", "duplicate:SetTabButtonAlpha(8,8)")
            .unwrap();
        assert_eq!(pair(&ui, "duplicate"), [9., 9.]);
    }
}
