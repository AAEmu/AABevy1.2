//! Published radar fixture shared by the unattended example and original-bytecode tests.

use std::sync::Arc;

use aa_ui::{
    UiRuntime,
    lua::{
        self,
        radar::{self, Contact, MapBounds, ShipTooltip, Snapshot, Symbol, Tooltip},
    },
};

pub fn snapshot() -> Snapshot {
    let db = rusqlite::Connection::open_with_flags(
        std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into()),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let rect = |sql: &str| -> [f32; 4] {
        db.query_row(sql, [], |r| {
            Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])
        })
        .unwrap()
    };
    let zone = rect("SELECT x,y,w,h FROM zone_groups WHERE id=5");
    let contact = |obj_id: u32, u: f32, v: f32, symbol: Symbol, tooltip: Tooltip| Contact {
        obj_id,
        position: [zone[0] + u * zone[2], zone[1] + (1. - v) * zone[3]],
        symbol,
        tooltip: Some(tooltip),
    };
    let ship = |name: &str| {
        Tooltip::Ship(ShipTooltip {
            name: Some(name.into()),
            owner: Some("Radar fixture".into()),
            hp: Some(720.),
            max_hp: Some(1000.),
            ..Default::default()
        })
    };
    Snapshot {
        maps: Arc::new(vec![
            MapBounds {
                level: 0,
                id: 2,
                rect: rect("SELECT x,y,w,h FROM world_groups WHERE id=2"),
            },
            MapBounds {
                level: 1,
                id: 3,
                rect: rect("SELECT x,y,w,h FROM world_groups WHERE id=3"),
            },
            MapBounds {
                level: 2,
                id: 5,
                rect: zone,
            },
        ]),
        ships: Arc::new(vec![
            contact(201, 0.32, 0.44, Symbol::Ship, ship("Windfinder")),
            contact(202, 0.40, 0.51, Symbol::Shipyard, ship("Shipyard")),
            contact(203, 0.48, 0.58, Symbol::EnemyShip, ship("Enemy ship")),
            contact(
                1_234_567_891,
                0.56,
                0.65,
                Symbol::EnemyShipyard,
                ship("Enemy yard"),
            ),
        ]),
        transfers: Arc::new(vec![
            contact(
                301,
                0.30,
                0.68,
                Symbol::Airship,
                Tooltip::Text("Fixture airship".into()),
            ),
            contact(
                302,
                0.42,
                0.75,
                Symbol::Carriage,
                Tooltip::Text("Fixture carriage".into()),
            ),
            contact(
                303,
                0.54,
                0.82,
                Symbol::Landship,
                Tooltip::Text("Fixture landship".into()),
            ),
        ]),
        fish: Arc::new(vec![contact(
            401,
            0.24,
            0.58,
            Symbol::FishSchool,
            Tooltip::Text("Fish school".into()),
        )]),
    }
}

pub fn install(ui: &mut UiRuntime) -> Snapshot {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@radar_fixture",
        r#"
        AAUI.WorldMap=function() return {level=2,zone=5,player={u=0.7594,v=0.2657}} end
        AAUI.RoadMap=function() return {level=2,zone=5,player={u=0.846,v=0.265}} end
        X2Map.GetTelescopeUnitHealth=function(self,id) return 720,1000 end
        ADDON:ShowContent(UIC_WORLDMAP,true)
        roadmap:SetMapSize(100)
        roadmapWindow:SetExtent(926,556)
        roadmapWindow:Show(true)
    "#,
    )
    .unwrap();
    // Finish the original map window's opening animation before pointer assertions.
    lua::tick(&ui.lua, 500.);
    ui.exec("@radar_full_map", "SetExpandLevel(1,true)").unwrap();
    let snapshot = snapshot();
    radar::set_snapshot(&ui.lua, snapshot.clone());
    for event in [
        "UPDATE_TELESCOPE_INFO",
        "UPDATE_TRANSFER_TELESCOPE_INFO",
        "UPDATE_FISH_SCHOOL_INFO",
    ] {
        lua::fire_ui_event(&ui.lua, event, vec![]);
    }
    lua::tick(&ui.lua, 16.);
    snapshot
}
