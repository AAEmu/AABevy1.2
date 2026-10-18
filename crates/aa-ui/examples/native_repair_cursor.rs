//! Original repair window and original normal/large OS cursor resource previews.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: native repair cursor"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            let item: mlua::Table = ui.lua.globals().get("X2Item").unwrap();
            for (name, enabled) in [("EnterRepairMode", true), ("LeaveRepairMode", false)] {
                item.raw_set(
                    name,
                    ui.lua
                        .create_function(move |lua, _: mlua::MultiValue| {
                            aa_ui::cursor::set_repair_mode(lua, enabled);
                            lua.globals().set("cursorRepairActive", enabled)
                        })
                        .unwrap(),
                )
                .unwrap();
            }
            ui.exec(
                "@repair_cursor_api",
                r#"
                cursorRepairActive=false
                X2Item.IsInRepairMode=function() return cursorRepairActive end
                X2Item.RepairAllCost=function() return 141 end
                X2Equipment.DameagedItems=function() return {},{},{} end
                X2Util.GetMyMoneyString=function() return '10000' end
            "#,
            )
            .unwrap();
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            ui.exec(
                "@repair_cursor_fixture",
                r#"
                ToggleRepairWindow(true,BLACK_SMITH)
                assert(repairWindow~=nil and repairWindow:IsVisible())
                if tutorial and tutorial.window then
                    tutorial.window.hideFromOption=true;tutorial.window:Show(false)
                end
                function cursorLabel(id,text,x,y)
                    local label=UIParent:CreateWidget('label',id,'UIParent')
                    label:SetExtent(280,24);label:AddAnchor('TOPLEFT',UIParent,x,y)
                    label:SetText(text);label:Show(true)
                end
                cursorLabel('cursorNormal','Native normal: 32 x 32',650,760)
                cursorLabel('cursorLarge','Native large: 64 x 64',960,760)
                cursorLabel('cursorNote','Original OS cursors; hotspot (0, 0)',800,900)
            "#,
            )
            .unwrap();
            let point = ui
                .lua
                .load("local x,y=repairWindow.partRepairButton:GetOffset();return x+20,y+20")
                .eval()
                .unwrap();
            let mut mouse = Default::default();
            aa_ui::input::update(&ui.lua, &mut mouse, Some(point), true, false, false, 0.);
            aa_ui::input::update(&ui.lua, &mut mouse, Some(point), false, true, false, 0.);
            ui.exec(
                "@repair_cursor_clicked",
                "assert(X2Item:IsInRepairMode()); assert(X2Cursor:GetHeld()==nil)",
            )
            .unwrap();
            fake::report(ui, "native_repair_cursor");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(
            Update,
            (fake::scripted_input, previews.after(aa_ui::UiFrame)),
        )
        .run();
}

fn previews(
    rt: Option<NonSend<aa_ui::UiRuntime>>,
    source: Res<aa_ui::cursor::CursorSource>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    mut ready: Local<bool>,
) {
    let Some(rt) = rt else { return };
    if *ready {
        return;
    }
    *ready = true;
    let scale = rt.ui_scale();
    for (large, x, size) in [(false, 790., 32.), (true, 1100., 64.)] {
        let (image, hotspot) = aa_ui::cursor::load_repair_image(&source.0, large).unwrap();
        assert_eq!(hotspot, (0, 0));
        assert_eq!(image.width(), size as u32);
        commands.spawn((
            Sprite::from_image(images.add(image)),
            Transform::from_xyz(x * scale + size / 2., -810. * scale - size / 2., 110.),
        ));
    }
}
