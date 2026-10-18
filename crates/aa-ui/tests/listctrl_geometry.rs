//! Actual native392a4410 oracle cases and the original achievement list's nested controls.
#[path = "../examples/native_achievement_list.rs"]
#[allow(dead_code)]
mod fixture;

use aa_ui::{UiRuntime, input, lua, tree::Id};

fn exec(ui: &UiRuntime, text: &str) {
    ui.exec("@listctrl_geometry", text).unwrap();
}
fn id(ui: &UiRuntime, text: &str) -> Id {
    let value = ui.lua.load(text).eval().unwrap();
    lua::value_id(&ui.lua, &value).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    exec(
        &ui,
        "list=UIParent:CreateWidget('listctrl','nativeList','UIParent');list:SetExtent(394,440);list:AddAnchor('TOPLEFT',UIParent,20,30)",
    );
    ui
}

#[test]
fn native_defaults_zero_guards_and_full_automatic_row_geometry() {
    let ui = setup();
    let list = id(&ui, "list");
    let data = lua::ui(&ui.lua)
        .tree
        .widget(list)
        .unwrap()
        .list
        .clone()
        .unwrap();
    assert_eq!((data.column_height, data.item_line_gap), (30., 0.));
    exec(&ui, "list:InsertRows(6,false)");
    assert_eq!(
        lua::ui(&ui.lua).tree.get(id(&ui, "list.items[1]")).height,
        0.
    );
    // No columns is a native early return, even with allocated rows.
    let ui = setup();
    exec(
        &ui,
        "list:InsertColumn(190,2);list:InsertColumn(200,2);list:SetColumnHeight(0);list:InsertRows(6,false)",
    );
    for i in 1..=6 {
        let row = id(&ui, &format!("list.items[{i}]"));
        let a = id(&ui, &format!("list.items[{i}].subItems[1]"));
        let b = id(&ui, &format!("list.items[{i}].subItems[2]"));
        let u = lua::ui(&ui.lua);
        let r = u.tree.rect(row);
        assert_eq!(
            (r.x, r.y, r.w, r.h),
            (20., 30. + (i - 1) as f32 * 73., 394., 73.)
        );
        assert_eq!(
            (u.tree.rect(a).x, u.tree.rect(a).w, u.tree.rect(a).h),
            (20., 190., 73.)
        );
        assert_eq!(
            (u.tree.rect(b).x, u.tree.rect(b).w, u.tree.rect(b).h),
            (210., 200., 73.)
        );
    }
}

#[test]
fn native_f32_cvtt_then_signed_division_and_gap_header_setters() {
    for (height, header, gap, count, expected) in [
        (440.9, 30.1, 1.3, 6, 67.),
        (20., 30., 0., 3, -3.),
        (f32::INFINITY, 0., 0., 3, -715827904.),
        (f32::NAN, 0., 0., 3, -715827904.),
        (440., 0., 2.5, 6, 71.),
        (440., 30., 0., 6, 68.),
    ] {
        let ui = setup();
        exec(
            &ui,
            &format!("list:InsertColumn(190,2);list:InsertRows({count},false)"),
        );
        let list = id(&ui, "list");
        // Lua literals do not express NaN/Infinity; use the same raw f32 widget extent.
        lua::ui(&ui.lua).tree.get_mut(list).height = height;
        ui.lua.globals().set("header", header).unwrap();
        ui.lua.globals().set("gap", gap).unwrap();
        exec(&ui, "list:SetColumnHeight(header);list:SetItemLineGap(gap)");
        let u = lua::ui(&ui.lua);
        let data = u.tree.widget(list).unwrap().list.as_ref().unwrap();
        assert_eq!(data.row_height, expected);
        for row in &data.rows {
            assert_eq!(u.tree.get(*row).height, expected);
        }
        for subs in &data.sub_items {
            for sub in subs {
                assert_eq!(u.tree.get(*sub).height, expected);
            }
        }
    }
}

#[test]
fn anchored_resize_settles_once_preserves_objects_data_and_legacy_explicit_height() {
    let ui = setup();
    exec(
        &ui,
        r#"
        parent=UIParent:CreateWidget('emptywidget','listParent','UIParent');parent:SetExtent(400,440)
        list:RemoveAllAnchors();list:AddAnchor('TOPLEFT',parent,0,0);list:AddAnchor('BOTTOMRIGHT',parent,0,0)
        list:InsertColumn(190,2);list:SetColumnHeight(0);list:InsertRows(6,false)
        list:InsertData(2,1,'retained');list:Select(1,false)
    "#,
    );
    let row = id(&ui, "list.items[2]");
    let sub = id(&ui, "list.items[2].subItems[1]");
    exec(&ui, "parent:SetExtent(460,500)");
    lua::tick(&ui.lua, 0.);
    let r = lua::ui(&ui.lua).tree.rect(row);
    assert_eq!((r.w, r.h), (460., 83.));
    assert_eq!(id(&ui, "list.items[2].subItems[1]"), sub);
    exec(
        &ui,
        "assert(list:GetSelectedIdx()==2);list.items[2]:SetHeight(19)",
    );
    lua::tick(&ui.lua, 0.);
    assert_eq!(
        lua::ui(&ui.lua).tree.get(row).height,
        19.,
        "unchanged tick must not rebuild rows"
    );
    exec(&ui, "parent:SetHeight(440)");
    lua::tick(&ui.lua, 0.);
    assert_eq!(lua::ui(&ui.lua).tree.get(row).height, 73.);
    // The compatibility override is intentionally absent from the native Lua class list.
    let list = id(&ui, "list");
    lua::ui(&ui.lua)
        .tree
        .widget_mut(list)
        .unwrap()
        .list
        .as_mut()
        .unwrap()
        .explicit_row_height = Some(25.);
    exec(&ui, "parent:SetHeight(600)");
    lua::tick(&ui.lua, 0.);
    assert_eq!(lua::ui(&ui.lua).tree.get(row).height, 25.);
    exec(&ui, "list:SetItemLineGap(2);list:SetColumnHeight(10)");
    assert_eq!(lua::ui(&ui.lua).tree.rect(row).y, 37.);
}

#[test]
fn original_achievement_rows_use_native_height_and_real_checkbox_callbacks() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    fixture::install(&mut ui);
    let list = id(&ui, "nativeAchievementBody.frame.listCtrl");
    let check = id(
        &ui,
        "nativeAchievementBody.frame.listCtrl.items[2].subItems[1].achievementWnd.tracingCheck",
    );
    let (rows, subs) = {
        let u = lua::ui(&ui.lua);
        let d = u.tree.widget(list).unwrap().list.as_ref().unwrap();
        (d.rows.clone(), d.sub_items.clone())
    };
    for row in rows {
        assert_eq!(lua::ui(&ui.lua).tree.rect(row).h, 73.);
    }
    for row in subs {
        assert_eq!(lua::ui(&ui.lua).tree.rect(row[0]).h, 73.);
    }
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, check)
    };
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(check));
    let mut mouse = Default::default();
    for expected in [true, false] {
        input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
        input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
        assert_eq!(
            ui.lua
                .load("X2Achievement:IsTracingAchievement(2)")
                .eval::<bool>()
                .unwrap(),
            expected
        );
    }
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
