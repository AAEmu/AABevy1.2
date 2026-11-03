use super::*;
use crate::hud::data::housing::{CommonFarm, FarmGroup, FarmGroupDoodad, Tables};

use std::cell::RefCell;
use std::rc::Rc;

use aa_ui::UiRuntime;

use crate::hud::api::Shared;

fn game() -> GameData {
    let mut game = GameData::default();
    let mut housing = Tables::default();
    for (id, name, count, description) in [
        (1, "Farm", 10, "crops"),
        (2, "Nursery", 5, "trees"),
    ] {
        housing.farm_groups.insert(
            id,
            FarmGroup {
                id,
                name: name.into(),
                count,
                description: description.into(),
            },
        );
        housing.farm_group_ids.push(id);
    }
    housing.farm_group_doodads = vec![
        FarmGroupDoodad {
            id: 13,
            farm_group_id: 1,
            doodad_id: 380,
            item_id: 13725,
        },
        FarmGroupDoodad {
            id: 14,
            farm_group_id: 1,
            doodad_id: 381,
            item_id: 13726,
        },
        FarmGroupDoodad {
            id: 15,
            farm_group_id: 2,
            doodad_id: 413,
            item_id: 4865,
        },
    ];
    housing.common_farms = vec![
        CommonFarm {
            id: 10,
            name: "Farm".into(),
            guard_time: 86_400_000,
            farm_group_id: 1,
            comments: "zone (33)".into(),
        },
        CommonFarm {
            id: 12,
            name: "Nursery".into(),
            guard_time: 0,
            farm_group_id: 2,
            comments: String::new(),
        },
    ];
    game.housing = housing;
    game
}

fn setup(game: GameData) -> (UiRuntime, Shared) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    assert!(bridge.borrow().game.0.set(game).is_ok());
    super::super::api::install(&mut rt, &bridge).unwrap();
    (rt, bridge)
}

#[test]
fn static_getters_match_the_native_shapes() {
    let (rt, _) = setup(game());
    rt.lua
        .load(
            r#"
            local ids = X2:GetFarmGroups()
            assert(#ids == 2 and ids[1].type == 1 and ids[2].type == 2, 'farm groups')
            assert(ids[1].name == 'Farm' and ids[2].name == 'Nursery', 'farm group names')
            local name, count, description = X2:GetFarmGorupInfo(1)
            assert(name == 'Farm' and count == 10 and description == 'crops', 'group info')
            assert(X2:GetFarmGorupInfo(99) == nil, 'unknown group')
            local doodads = X2:GetFarmGroupDoodadList(1)
            assert(#doodads == 2, 'doodad list')
            assert(X2:GetFarmGroupDoodadList(99) == nil, 'unknown group list')
            local farm, guard, group = X2:GetCommonFarmInfo(10)
            assert(farm == 'Farm' and guard == 86400000 and group == 1, 'common farm')
            assert(X2:GetCommonFarmInfo(99) == nil, 'unknown common farm')
        "#,
        )
        .exec()
        .unwrap();
}
