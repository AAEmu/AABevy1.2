//! Original appearance-conversion window over received physical BAG records.

use bevy::{prelude::*, window::PrimaryWindow};

mod bindings;
mod catalog;
mod controller;
mod pointer;
mod target;
pub(crate) use pointer::{PointerFinished, blocks as blocks_look_pointer};
pub(super) use bindings::install;
pub(super) use catalog::Tables;
pub(super) use controller::{View, observe, replay, row};
pub(crate) use target::ItemLookSource;
pub(crate) use target::source_use;
pub(super) use target::{Admission, admit_request, cancel_target, escape, plugin};
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) fn test_runtime() -> (super::api::Shared, aa_ui::UiRuntime) {
    let state = tests::state();
    {
        let mut data = state.borrow_mut();
        data.snapshot.hp = 100;
        data.snapshot.mp = 1000;
        data.snapshot.level = 30;
    }
    let rt = tests::runtime(&state);
    (state, rt)
}

pub(super) fn fixture() {
    let case = std::env::args()
        .collect::<Vec<_>>()
        .windows(2)
        .find(|args| args[0] == "--case")
        .map(|args| args[1].clone())
        .unwrap_or_else(|| "empty".into());
    let skin = case.starts_with("skinize");
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (runtime, shared, _settings, mut inventory) =
        super::expansion::fixture::runtime(width as f32, height as f32).unwrap();
    if case != "empty" {
        use aa_net::{
            Event,
            items::InvenChunk,
            model::{EquipDetail, Item, ItemDetail},
        };
        let spawn = super::expansion::fixture::spawn();
        let mut items = vec![None; 50];
        items[6] = Some(Item {
            template_id: 26060,
            id: 9_007_199_254_740_993,
            grade: 4,
            count: 1,
            detail: ItemDetail::Equip(EquipDetail::default()),
            ..default()
        });
        items[11] = Some(Item {
            template_id: 26118,
            id: 9_007_199_254_740_995,
            grade: 1,
            flags: if skin { 0 } else { 8 },
            count: 1,
            detail: ItemDetail::Equip(EquipDetail::default()),
            ..default()
        });
        items[12] = Some(Item {
            template_id: 29880,
            id: 9_007_199_254_740_997,
            count: if case == "materials" { 8 } else { 10 },
            ..default()
        });
        for event in [
            Event::CharacterSelected(Box::new(spawn.state.clone())),
            Event::PlayerSpawn(Box::new(spawn)),
            Event::InventoryInit {
                bag_slots: 50,
                bank_slots: 0,
            },
            Event::InventoryChunk(InvenChunk {
                container: 2,
                chunks: 5,
                start_chunk: 0,
                items,
                truncated: false,
            }),
        ] {
            inventory.apply(&event);
            super::enchant::dispatch(None, &shared, &event, None);
        }
        let mut world = World::new();
        world.insert_resource(inventory.clone());
        let mut data = shared.borrow_mut();
        data.snapshot.hp = 100;
        data.snapshot.mp = 100;
        data.panels = super::panels::refresh(
            &world,
            data.game.get(),
            0,
            100,
            data.start,
            &data.snapshot,
            Vec3::ZERO,
        )
        .0;
        data.panels.actions_known = true;
        drop(data);
        super::api::push_changes(&runtime, &shared);
        runtime.fire_event("BAG_UPDATE", (-1i64, -1i64));
    }
    if skin {
        runtime
            .exec(
                "@look-task-capture",
                r#"
            local request=X2DialogManager.RequestTaskDialog
            X2DialogManager.RequestTaskDialog=function(self,task,info,owner)
              local wnd=request(self,task,info,owner)
              if task==19 then LOOK_TASK=wnd;LOOK_TASK_INFO=info end
              return wnd
            end
        "#,
            )
            .unwrap();
        target::fixture_source(&runtime, &shared, &inventory, 9_007_199_254_740_997);
        runtime.exec("@look-target-admitted", "assert(X2Item:IsInItemSkinizeMode() and X2Cursor:GetCursorInfo()==nil and not X2Bag:IsLocked(13))").unwrap();
        if case != "skinize" {
            aa_ui::lua::tick(&runtime.lua, 16.0);
            aa_ui::lua::flush(&runtime.lua);
            super::expansion::fixture::click(&runtime, "newbag.slots.btns[12]").unwrap();
            runtime.exec("@look-target-task", "assert(LOOK_TASK and LOOK_TASK:IsVisible() and LOOK_TASK_INFO.itemInfo.itemType==26118 and not X2Item:IsInItemSkinizeMode() and X2Bag:IsLocked(12))").unwrap();
            if case == "skinize-cancel" || case == "skinize-ok" {
                let button = if case == "skinize-ok" {
                    "LOOK_TASK.btnOk"
                } else {
                    "LOOK_TASK.btnCancel"
                };
                aa_ui::lua::tick(&runtime.lua, 16.0);
                aa_ui::lua::flush(&runtime.lua);
                super::expansion::fixture::click(&runtime, button).unwrap();
                runtime
                    .exec("@look-task-finished", "assert(not X2Bag:IsLocked(12))")
                    .unwrap();
                assert_eq!(
                    shared.borrow().requests.len(),
                    usize::from(case == "skinize-ok")
                );
            }
        }
        eprintln!(
            "look-conversion fixture: case={case}; actual app admission; original task/physical input; provided records only, no socket or native planner success"
        );
    } else {
        runtime
            .exec(
                "@look-conversion-fixture",
                "ToggleChangeItemLookWindow(true, 16)",
            )
            .unwrap();
    }
    if case != "empty" && !skin {
        runtime.exec("@look-conversion-provided-records",r#"
            assert(AttachItemToItemConverter(7));assert(AttachItemToItemConverter(12))
            local base=X2ItemLookConverter:GetConvertBaseItemInfo();local image=X2ItemLookConverter:GetConvertLookItemInfo()
            local result=X2ItemLookConverter:GetConvertResultItemInfo();local plan=X2ItemLookConverter:GetItemLookConvertInfo()
            assert(base.itemType==26060 and base.itemGrade==4 and image.itemType==26118 and image.itemGrade==1)
            assert(result.lookType==26118 and result.itemGrade==4 and plan.items[1].need==9)
            assert(X2Bag:IsLocked(7) and X2Bag:IsLocked(12))
        "#).unwrap();
        if case == "confirm" {
            runtime
                .exec(
                    "@look-conversion-confirmation",
                    r#"
                local request=X2DialogManager.RequestDefaultDialog
                X2DialogManager.RequestDefaultDialog=function(self,handler,owner)
                    return request(self,function(wnd) handler(wnd);LOOK_CONFIRM=wnd end,owner)
                end
            "#,
                )
                .unwrap();
            aa_ui::lua::tick(&runtime.lua, 16.0);
            aa_ui::lua::flush(&runtime.lua);
            super::expansion::fixture::click(&runtime, "changeItemLookWindow.okButton").unwrap();
            runtime
                .exec(
                    "@look-conversion-unsubmitted-confirmation",
                    "assert(LOOK_CONFIRM~=nil)",
                )
                .unwrap();
            assert!(
                shared.borrow().requests.is_empty(),
                "confirmation itself must not submit a conversion"
            );
        }
        eprintln!(
            "look-conversion fixture: case={case}; provided received records; no socket/server mutation"
        );
    }
    aa_ui::lua::tick(&runtime.lua, 16.0);
    aa_ui::lua::flush(&runtime.lua);
    if !skin {
        // Inspect the original widget identity after its explicit show request.
        let ui = aa_ui::lua::ui(&runtime.lua);
        let window = ui
            .tree
            .objects
            .iter()
            .find(|object| object.alive && object.name == "changeItemLook");
        assert!(
            window.is_some_and(|object| object.shown),
            "original conversion window missing after toggle"
        );
    }
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Original appearance conversion"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(runtime)
        .insert_non_send(super::api::HudBridge(shared))
        .add_systems(Startup, physical_pixels)
        .run();
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
