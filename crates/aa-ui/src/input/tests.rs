//! Release completion uses real GUI dispatch and CPU-only frame delivery, without a renderer.
use super::*;
use crate::UiRuntime;
use std::{
    cell::Cell,
    sync::{Arc, Mutex},
};

fn runtime() -> UiRuntime {
    let rt = UiRuntime::new("en_us", 800., 600.).unwrap();
    rt.exec(
        "@release_setup",
        "log={};function note(s) table.insert(log,s) end",
    )
    .unwrap();
    rt
}

fn record(lua: &Lua, value: impl Into<String>) -> mlua::Result<()> {
    let log: mlua::Table = lua.globals().get("log")?;
    log.raw_set(log.raw_len() + 1, value.into())
}

fn log(rt: &UiRuntime) -> String {
    rt.lua.load("table.concat(log, '|')").eval().unwrap()
}

fn id(rt: &UiRuntime, name: &str) -> Id {
    lua::value_id(&rt.lua, &rt.lua.globals().get(name).unwrap()).unwrap()
}

fn button(rt: &UiRuntime) -> Id {
    rt.exec(
        "@button",
        r#"
        b=UIParent:CreateWidget('button','releaseButton','UIParent')
        b:SetExtent(40,40);b:AddAnchor('TOPLEFT',UIParent,100,100);b:Show(true)
        b:SetHandler('OnClick',function(self,button) note('click:'..button) end)
    "#,
    )
    .unwrap();
    id(rt, "b")
}

fn install_recorder(lua: &Lua) {
    set_release_callback(lua, |lua, button| {
        record(lua, format!("release:{button:?}"))
    })
    .unwrap();
}

fn deliver(
    rt: &UiRuntime,
    mouse: &mut MouseState,
    cursor: Option<(f32, f32)>,
    buttons: MouseButtons,
) {
    update_buttons(&rt.lua, mouse, cursor, buttons, 0.);
}

#[test]
fn unregistered_release_does_not_add_a_pending_flush() {
    let rt = runtime();
    let b = button(&rt);
    rt.exec(
        "@probe",
        "b:SetHandler('Probe',function() note('queued') end)",
    )
    .unwrap();
    ui(&rt.lua).pending.push((b, "Probe".into(), vec![]));
    let mut mouse = MouseState::default();
    deliver(
        &rt,
        &mut mouse,
        None,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "");
    assert_eq!(ui(&rt.lua).pending.len(), 1);
    set_release_callback(&rt.lua, |_, _| Ok(())).unwrap();
    deliver(
        &rt,
        &mut mouse,
        None,
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "queued");
    assert!(ui(&rt.lua).pending.is_empty());
}

#[test]
fn empty_and_cursor_none_releases_notify_once_in_left_right_order() {
    let rt = runtime();
    install_recorder(&rt.lua);
    let mut mouse = MouseState::default();
    deliver(&rt, &mut mouse, None, MouseButtons::default());
    deliver(
        &rt,
        &mut mouse,
        None,
        MouseButtons {
            press: true,
            right_press: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "");
    deliver(
        &rt,
        &mut mouse,
        None,
        MouseButtons {
            release: true,
            right_release: true,
            ..Default::default()
        },
    );
    // The compatibility entry point forwards through the same notification sites.
    update(
        &rt.lua,
        &mut mouse,
        Some((700., 500.)),
        false,
        true,
        true,
        0.,
    );
    assert_eq!(
        log(&rt),
        "release:Left|release:Right|release:Left|release:Right"
    );
    let other = runtime();
    deliver(
        &other,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&other), "");
}

#[test]
fn slot_use_and_queued_continuations_finish_before_next_release_wheel_and_tick() {
    let rt = runtime();
    rt.exec(
        "@slot",
        r#"
        AAUI.SlotContent=function() return {type='skill',id=7} end
        b=UIParent:CreateWidget('slot','releaseSlot','UIParent')
        b:SetExtent(40,40);b:AddAnchor('TOPLEFT',UIParent,100,100);b:Show(true)
        b:EstablishSlot(ISLOT_ACTION,1)
        b:SetHandler('OnMouseUp',function(self,button) note('up:'..button) end)
        b:SetHandler('OnClick',function(self,button) note('click:'..button) end)
        b:SetHandler('Queued',function(self,button) note('queued:'..button) end)
        b:SetHandler('OnWheelUp',function() note('wheel') end)
        b:SetHandler('OnUpdate',function() note('tick') end)
        UIParent:SetEventHandler('MOUSE_UP',function() note('MOUSE_UP') end)
        AAUI.UseSlot=function(ty,idx,button)
            assert(ty==ISLOT_ACTION and idx==1);note('use:'..button);Queue(button)
        end
    "#,
    )
    .unwrap();
    let b = id(&rt, "b");
    rt.lua
        .globals()
        .set(
            "Queue",
            rt.lua
                .create_function(move |lua, button: String| {
                    ui(lua).pending.push((
                        b,
                        "Queued".into(),
                        vec![Value::String(lua.create_string(button)?)],
                    ));
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    let mut mouse = MouseState::default();
    deliver(
        &rt,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    install_recorder(&rt.lua);
    update_buttons(
        &rt.lua,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            release: true,
            right_release: true,
            ..Default::default()
        },
        1.,
    );
    lua::tick(&rt.lua, 16.);
    assert_eq!(
        log(&rt),
        "up:LeftButton|MOUSE_UP|click:LeftButton|use:LeftButton|queued:LeftButton|release:Left|up:RightButton|MOUSE_UP|click:RightButton|use:RightButton|queued:RightButton|release:Right|wheel|tick"
    );
}

#[test]
fn right_held_clear_precedes_completion_and_skips_click() {
    let rt = runtime();
    button(&rt);
    rt.exec(
        "@held",
        r#"
        X2Cursor:SetHeld({kind='thing'})
        b:SetHandler('OnMouseUp',function() assert(X2Cursor:GetHeld()~=nil);note('up-held') end)
    "#,
    )
    .unwrap();
    set_release_callback(&rt.lua, |lua, button| {
        assert_eq!(button, MouseButton::Right);
        assert!(!lua::drag::holding(lua));
        record(lua, "cleared")
    })
    .unwrap();
    deliver(
        &rt,
        &mut MouseState::default(),
        Some((120., 120.)),
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "up-held|cleared");
}

#[test]
fn drag_receive_stop_and_drop_on_nothing_all_precede_completion() {
    let rt = runtime();
    button(&rt);
    rt.exec(
        "@drag",
        r#"
        b:RegisterForDrag('LeftButton')
        b:SetHandler('OnDragStart',function() X2Cursor:SetHeld({kind='thing'}) end)
        b:SetHandler('OnDragStop',function() note('stop') end)
        t=UIParent:CreateWidget('button','dropTarget','UIParent')
        t:SetExtent(40,40);t:AddAnchor('TOPLEFT',UIParent,200,100);t:Show(true)
        t:SetHandler('OnDragReceive',function() note('receive') end)
        AAUI.DropOnNothing=function(overUi) assert(overUi);note('drop');X2Cursor:ClearCursor() end
    "#,
    )
    .unwrap();
    let mut mouse = MouseState::default();
    deliver(
        &rt,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    deliver(&rt, &mut mouse, Some((220., 120.)), MouseButtons::default());
    install_recorder(&rt.lua);
    deliver(
        &rt,
        &mut mouse,
        Some((220., 120.)),
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "receive|stop|drop|release:Left");
    assert!(!rt.holding());
}

#[test]
fn sizing_and_disabled_fading_and_empty_slots_still_complete() {
    for mode in ["sizing", "disabled", "fading", "empty"] {
        let rt = runtime();
        rt.lua
            .globals()
            .set(
                "controlType",
                if mode == "empty" { "slot" } else { "button" },
            )
            .unwrap();
        rt.exec(
            "@control",
            r#"
            b=UIParent:CreateWidget(controlType,'releaseControl','UIParent')
            b:SetExtent(40,40);b:AddAnchor('TOPLEFT',UIParent,100,100);b:Show(true)
            b:SetHandler('OnClick',function() note('click') end)
            AAUI.UseSlot=function() error('empty slot used') end
        "#,
        )
        .unwrap();
        match mode {
            "sizing" => rt.exec("@sizing", "b:StartSizing('RIGHT')").unwrap(),
            "disabled" => rt.exec("@disabled", "b:Enable(false)").unwrap(),
            "fading" => {
                rt.exec("@hide", "b:IgnoreInputOnFading(true);b:Show(false)")
                    .unwrap();
                lua::tick(&rt.lua, 0.);
                rt.exec("@show", "b:Show(true,1000)").unwrap();
            }
            _ => (),
        }
        let mut mouse = MouseState::default();
        deliver(
            &rt,
            &mut mouse,
            Some((120., 120.)),
            MouseButtons {
                press: true,
                ..Default::default()
            },
        );
        install_recorder(&rt.lua);
        deliver(
            &rt,
            &mut mouse,
            Some((120., 120.)),
            MouseButtons {
                release: true,
                ..Default::default()
            },
        );
        assert_eq!(
            log(&rt),
            if mode == "empty" {
                "click|release:Left"
            } else {
                "release:Left"
            },
            "{mode}"
        );
    }
}

#[test]
fn native_tab_early_return_completes_after_selection() {
    let rt = runtime();
    rt.exec(
        "@tabs",
        r#"
        tab=UIParent:CreateWidget('tab','releaseTabs','UIParent')
        tab:SetExtent(200,100);tab:AddAnchor('TOPLEFT',UIParent,100,100);tab:Show(true)
        for i=1,2 do tab:AddSimpleTab('tab'..i)
            tab.selectedButton[i]:SetExtent(40,30);tab.unselectedButton[i]:SetExtent(40,30)
        end
        tab:SetHandler('OnTabChanged',function(self,i) note('selected:'..i) end)
    "#,
    )
    .unwrap();
    lua::tick(&rt.lua, 0.);
    let target = lua::value_id(
        &rt.lua,
        &rt.lua.load("tab.unselectedButton[2]").eval().unwrap(),
    )
    .unwrap();
    let at = {
        let u = ui(&rt.lua);
        let r = u.tree.rect(target);
        (r.x + 10., r.y + 10.)
    };
    let mut mouse = MouseState::default();
    deliver(
        &rt,
        &mut mouse,
        Some(at),
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    install_recorder(&rt.lua);
    deliver(
        &rt,
        &mut mouse,
        Some(at),
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "selected:2|release:Left");
}

#[test]
fn flushes_surround_callback_without_retaining_state_or_registry_borrows() {
    let rt = runtime();
    let b = button(&rt);
    rt.exec(
        "@queued",
        "b:SetHandler('Queued',function(self,label) note(label) end)",
    )
    .unwrap();
    ui(&rt.lua).pending.push((
        b,
        "Queued".into(),
        vec![Value::String(rt.lua.create_string("before").unwrap())],
    ));
    set_release_callback(&rt.lua, move |lua, _| {
        assert_eq!(
            lua.load("table.concat(log,'|')").eval::<String>()?,
            "before"
        );
        record(lua, "callback")?;
        // These require independent app-data, tree and Lua registry access.
        assert!(ui(lua).tree.get(b).alive);
        lua.load("child=b:CreateChildWidget('button','duringRelease',0,true);child:Show(true)")
            .exec()?;
        assert!(lua::object_table(lua, b)?.is_some());
        ui(lua).pending.push((
            b,
            "Queued".into(),
            vec![Value::String(lua.create_string("after")?)],
        ));
        Ok(())
    })
    .unwrap();
    deliver(
        &rt,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "before|callback|after");
}

#[test]
fn callback_replacement_is_safe_and_applies_to_the_next_release() {
    let rt = runtime();
    let dropped = Rc::new(Cell::new(false));
    struct DropFlag(Rc<Cell<bool>>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let flag = DropFlag(dropped.clone());
    let during = dropped.clone();
    set_release_callback(&rt.lua, move |lua, button| {
        let _keep_alive = &flag;
        record(lua, format!("old:{button:?}"))?;
        set_release_callback(lua, |lua, button| record(lua, format!("new:{button:?}")))?;
        assert!(!during.get());
        record(lua, "old-finished")
    })
    .unwrap();
    deliver(
        &rt,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "old:Left|old-finished|new:Right");
    assert!(dropped.get());
}

#[test]
fn borrowed_appdata_rejects_replacement_without_panicking_or_removing_old_callback() {
    let rt = runtime();
    install_recorder(&rt.lua);
    let borrowed = ui(&rt.lua);
    assert!(set_release_callback(&rt.lua, |_, _| panic!("rejected replacement ran")).is_err());
    drop(borrowed);
    deliver(
        &rt,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "release:Left");
}

#[test]
fn dropping_the_lua_state_drops_its_callback() {
    let lua = aa_lua::new_lua().unwrap();
    let captured = Rc::new(());
    let weak = Rc::downgrade(&captured);
    set_release_callback(&lua, move |_, _| {
        let _keep_alive = &captured;
        Ok(())
    })
    .unwrap();
    assert!(weak.upgrade().is_some());
    drop(lua);
    assert!(weak.upgrade().is_none());
}

#[test]
fn surviving_hover_keeps_its_identity_when_callback_creates_a_new_top_widget() {
    let rt = runtime();
    let b = button(&rt);
    rt.exec("@hover", "b:SetHandler('OnEnter',function() note('enter') end);b:SetHandler('OnLeave',function() note('leave') end)").unwrap();
    let mut mouse = MouseState::default();
    deliver(
        &rt,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    rt.exec("@clear", "log={}").unwrap();
    set_release_callback(&rt.lua, |lua, button| {
        record(lua, format!("release:{button:?}"))?;
        if button == MouseButton::Left {
            lua.load(
                r#"
                cover=UIParent:CreateWidget('button','releaseCover','UIParent')
                cover:SetExtent(40,40);cover:AddAnchor('TOPLEFT',UIParent,100,100);cover:Show(true)
                cover:SetHandler('OnClick',function() note('cover-click') end)
                cover:SetHandler('OnEnter',function() note('cover-enter') end)
            "#,
            )
            .exec()?;
        }
        Ok(())
    })
    .unwrap();
    deliver(
        &rt,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            release: true,
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(
        log(&rt),
        "click:LeftButton|release:Left|click:RightButton|release:Right"
    );
    assert_eq!(mouse.last_click.map(|(id, _)| id), Some(b));
    assert_eq!(ui(&rt.lua).hover, Some(b));
    assert_eq!(pick(&rt.lua, 120., 120.), Some(id(&rt, "cover")));
}

#[test]
#[should_panic(expected = "release-callback-panic")]
fn rust_callback_panics_are_not_converted_to_recoverable_lua_errors() {
    let rt = runtime();
    set_release_callback(&rt.lua, |_, _| panic!("release-callback-panic")).unwrap();
    deliver(
        &rt,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
}

#[test]
fn pending_flush_replacement_does_not_change_the_snapshotted_invocation() {
    let rt = runtime();
    let b = button(&rt);
    install_recorder(&rt.lua);
    rt.lua
        .globals()
        .set(
            "ReplaceRelease",
            rt.lua
                .create_function(|lua, ()| {
                    set_release_callback(lua, |lua, button| {
                        record(lua, format!("replacement:{button:?}"))
                    })
                })
                .unwrap(),
        )
        .unwrap();
    rt.exec(
        "@replace_handler",
        "b:SetHandler('Replace',function() ReplaceRelease() end)",
    )
    .unwrap();
    ui(&rt.lua).pending.push((b, "Replace".into(), vec![]));
    deliver(
        &rt,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "release:Left|replacement:Right");
}

#[test]
fn nested_delivery_with_a_separate_mouse_state_is_synchronous() {
    let rt = runtime();
    set_release_callback(&rt.lua, |lua, button| {
        record(lua, format!("begin:{button:?}"))?;
        if button == MouseButton::Left {
            update_buttons(
                lua,
                &mut MouseState::default(),
                None,
                MouseButtons {
                    right_release: true,
                    ..Default::default()
                },
                0.,
            );
        }
        record(lua, format!("end:{button:?}"))
    })
    .unwrap();
    deliver(
        &rt,
        &mut MouseState::default(),
        None,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(log(&rt), "begin:Left|begin:Right|end:Right|end:Left");
}

#[test]
fn left_callback_retirement_filters_next_right_and_wheel_without_repicking() {
    let rt = runtime();
    rt.exec(
        "@under",
        r#"
        under=UIParent:CreateWidget('button','releaseUnder','UIParent')
        under:SetExtent(40,40);under:AddAnchor('TOPLEFT',UIParent,100,100);under:Show(true)
        under:SetHandler('OnClick',function() error('repicked under') end)
        under:SetHandler('OnWheelUp',function() error('repicked under wheel') end)
    "#,
    )
    .unwrap();
    let b = button(&rt);
    rt.exec(
        "@retire",
        r#"
        b:EnableHidingIsRemove(true)
        b:SetHandler('OnMouseUp',function(self,button) note('up:'..button) end)
        b:SetHandler('OnWheelUp',function() note('retired-wheel') end)
        UIParent:SetEventHandler('MOUSE_UP',function(target) note('target:'..target) end)
    "#,
    )
    .unwrap();
    set_release_callback(&rt.lua, move |lua, button| {
        record(lua, format!("release:{button:?}"))?;
        if button == MouseButton::Left {
            lua.load("b:Show(false)").exec()?;
            // Hide schedules removal; the existing fade/frame lifecycle retires it.
            lua::tick(lua, 0.);
        }
        Ok(())
    })
    .unwrap();
    let mut mouse = MouseState::default();
    deliver(
        &rt,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    update_buttons(
        &rt.lua,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            release: true,
            right_release: true,
            ..Default::default()
        },
        1.,
    );
    assert_eq!(
        log(&rt),
        format!(
            "up:LeftButton|target:{b}|click:LeftButton|release:Left|target:{ROOT}|release:Right"
        )
    );
    assert!(!ui(&rt.lua).tree.get(b).alive);
    assert!(mouse.last_click.is_none());
}

#[test]
fn right_callback_retirement_prunes_all_captures_before_wheel() {
    let rt = runtime();
    let b = button(&rt);
    rt.exec("@retire","b:EnableHidingIsRemove(true);b:SetHandler('OnWheelUp',function() note('retired-wheel') end)").unwrap();
    set_release_callback(&rt.lua, |lua, _| {
        lua.load("b:Show(false)").exec()?;
        lua::tick(lua, 0.);
        Ok(())
    })
    .unwrap();
    let mut mouse = MouseState {
        pressed: Some(b),
        last_click: Some((b, 0.)),
        slider: Some(b),
        drag: Some(DragPress {
            src: b,
            at: (120., 120.),
            started: false,
        }),
        ..Default::default()
    };
    update_buttons(
        &rt.lua,
        &mut mouse,
        Some((120., 120.)),
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
        1.,
    );
    assert!(!ui(&rt.lua).tree.get(b).alive);
    assert!(
        mouse.pressed.is_none()
            && mouse.last_click.is_none()
            && mouse.slider.is_none()
            && mouse.drag.is_none()
    );
    assert_eq!(log(&rt), "click:RightButton");
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn callback_error_logs_once_flushes_pending_and_continues_right_and_tick() {
    let rt = runtime();
    let b = button(&rt);
    rt.exec(
        "@after_error",
        r#"
        b:SetHandler('AfterError',function() note('after-error') end)
        b:SetHandler('OnUpdate',function() note('tick') end)
    "#,
    )
    .unwrap();
    set_release_callback(&rt.lua, move |lua, button| {
        record(lua, format!("release:{button:?}"))?;
        if button == MouseButton::Left {
            ui(lua).pending.push((b, "AfterError".into(), vec![]));
            return Err(mlua::Error::RuntimeError("release-test-failure".into()));
        }
        Ok(())
    })
    .unwrap();
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogWriter(bytes.clone());
    let subscriber = bevy::log::tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    bevy::log::tracing::subscriber::with_default(subscriber, || {
        deliver(
            &rt,
            &mut MouseState::default(),
            None,
            MouseButtons {
                release: true,
                right_release: true,
                ..Default::default()
            },
        );
        lua::tick(&rt.lua, 16.);
    });
    assert_eq!(log(&rt), "release:Left|after-error|release:Right|tick");
    let logged = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
    assert_eq!(
        logged.matches("release-test-failure").count(),
        1,
        "{logged}"
    );
    assert!(logged.contains("Left release callback"));
}

#[test]
fn actual_ui_frame_ors_physical_virtual_edges_once_and_consumes_both_sources() {
    use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*, window::PrimaryWindow};
    let rt = runtime();
    install_recorder(&rt.lua);
    let mut app = App::new();
    app.insert_non_send(rt)
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseScroll>()
        .init_resource::<Time>()
        .init_resource::<crate::render::UiRenderState>()
        .init_resource::<crate::UiStats>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<ColorMaterial>>()
        .init_resource::<Assets<crate::render::MaskMaterial>>()
        .init_resource::<Assets<Image>>()
        .insert_resource(crate::UiVirtualPointer {
            press: true,
            release: true,
            right_press: true,
            right_release: true,
            ..Default::default()
        })
        .add_systems(Update, crate::ui_callbacks.in_set(crate::UiCallbacks))
        .add_systems(Update, crate::ui_paint.in_set(crate::UiPaint));
    crate::modelview_frame_sets(&mut app);
    app.world_mut().spawn((
        Window {
            resolution: (800, 600).into(),
            ..Default::default()
        },
        PrimaryWindow,
    ));
    {
        let mut physical = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        physical.press(MouseButton::Left);
        physical.release(MouseButton::Left);
        physical.press(MouseButton::Right);
        physical.release(MouseButton::Right);
    }
    // No WinitPlugin/RenderPlugin, RenderApp, GPU device or native window is created.
    app.update();
    let rt = app.world().non_send::<UiRuntime>();
    assert_eq!(log(rt), "release:Left|release:Right");
    let virtual_ = app.world().resource::<crate::UiVirtualPointer>();
    assert!(
        !virtual_.press && !virtual_.release && !virtual_.right_press && !virtual_.right_release
    );
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.update();
    assert_eq!(
        log(app.world().non_send::<UiRuntime>()),
        "release:Left|release:Right"
    );
}
