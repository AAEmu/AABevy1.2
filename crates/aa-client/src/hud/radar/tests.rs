use super::*;
use aa_net::radar::{FishRadarContact, RadarToggle, ShipRadarContact, TransferRadarContact};
use aa_net::wire::Pos;

fn ship(id: u32, hp: i32) -> ShipRadarContact {
    ShipRadarContact {
        obj_id: id,
        summoner_id: 47,
        raw_unit_type: 2,
        template_id: 71,
        pos: Pos::new(30., 40., 0.),
        faction_id: 3,
        expedition_id: 0,
        name: "Windfinder".into(),
        summoner_name: "Test captain".into(),
        hp,
        max_hp: 1000,
        raw_tail: 3554,
    }
}

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1280., 720.)?;
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
    super::super::api::install(&mut rt, &bridge)?;
    Ok((rt, bridge))
}

#[test]
fn complete_snapshots_replay_getter_values_and_numeric_removals_in_packet_order() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    rt.lua
        .load(
            r#"
        assert(select('#', X2Map:GetTelescopeUnitHealth(201))==0)
        notices={}
        UIParent:SetEventHandler('UPDATE_TELESCOPE_INFO',function(...)
            assert(select('#',...)==0)
            local hp,max=X2Map:GetTelescopeUnitHealth(201)
            table.insert(notices, hp or -1)
        end)
        UIParent:SetEventHandler('REMOVE_TELESCOPE_INFO',function(id)
            assert(type(id)=='number' and id==201)
            assert(select('#', X2Map:GetTelescopeUnitHealth(id))==0)
            table.insert(notices,'removed')
        end)
    "#,
        )
        .exec()?;
    dispatch(&rt, &bridge, &Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: true, range: 1200. })));
    rt.lua.load("notices={}").exec()?;
    dispatch(
        &rt,
        &bridge,
        &Event::Radar(RadarEvent::ShipContacts {
            last: false,
            contacts: vec![ship(201, 720)],
        }),
    );
    rt.lua.load("assert(#notices==0 and select('#',X2Map:GetTelescopeUnitHealth(201))==0)").exec()?;
    dispatch(&rt, &bridge, &Event::Radar(RadarEvent::ShipContacts { last: true, contacts: vec![] }));
    dispatch(
        &rt,
        &bridge,
        &Event::Radar(RadarEvent::ShipContacts {
            last: true,
            contacts: vec![ship(201, 310)],
        }),
    );
    dispatch(&rt, &bridge, &Event::Radar(RadarEvent::ShipContacts { last: true, contacts: vec![] }));
    rt.lua
        .load("assert(#notices==4 and notices[1]==720 and notices[2]==310 and notices[3]=='removed' and notices[4]==-1)")
        .exec()?;
    // The raw getter rounds each i32 through f32 and never clamps current HP to max.
    dispatch(
        &rt,
        &bridge,
        &Event::Radar(RadarEvent::ShipContacts {
            last: true,
            contacts: vec![ship(201, 16_777_217)],
        }),
    );
    rt.lua
        .load("local hp,max=X2Map:GetTelescopeUnitHealth(201); assert(hp==16777216 and max==1000); assert(select('#',X2Map:GetTelescopeUnitHealth('201'))==0)")
        .exec()?;
    Ok(())
}

#[test]
fn classified_rows_use_native_template_keys_and_separate_channel_names() {
    let mut data = HudData::default();
    let mut game = GameData::default();
    game.world_rects.insert(2, [0., 0., 100., 100.]);
    game.world_rects.insert(3, [0., 0., 100., 100.]);
    game.world_rects.insert(9, [0., 0., 100., 100.]);
    game.sub_zone_rects.insert(1, [0., 0., 0., 100.]);
    game.factions.insert(3, "Hostile".into());
    game.factions.insert(4, "West".into());
    game.radar.kinds.insert(71, "speedboat");
    game.radar.transfers.insert(81, Symbol::Airship);
    game.radar.fish_names.insert(91, "School of Marlins".into());
    data.game.0.set(game).ok().unwrap();
    data.radar.faction = 4;
    data.radar.ships.contacts = Some(vec![ship(201, 1200)]);
    data.radar.transfers.contacts = Some(vec![
        TransferRadarContact {
            obj_id: 301,
            template_id: 81,
            pos: Pos::default(),
            name: "Received airship".into(),
        },
        TransferRadarContact {
            obj_id: 302,
            template_id: 999,
            pos: Pos::default(),
            name: "Unknown template".into(),
        },
    ]);
    data.radar.fish.contacts = Some(vec![FishRadarContact {
        obj_id: 401,
        template_id: 91,
        pos: Pos::default(),
    }]);
    let s = snapshot(&data);
    assert_eq!(s.maps.len(), 2);
    assert_eq!(s.ships[0].symbol, Symbol::EnemyShip);
    let Some(Tooltip::Ship(info)) = &s.ships[0].tooltip else { panic!() };
    assert_eq!(info.kind.as_deref(), Some("speedboat"));
    assert_eq!(info.buff, Some(true));
    assert_eq!(info.hp, Some(1000.));
    assert_eq!(info.max_hp, Some(1000.));
    assert_eq!(info.expedition, None);
    assert_eq!(s.transfers.len(), 1);
    assert_eq!(s.transfers[0].tooltip, Some(Tooltip::Text("Received airship".into())));
    assert_eq!(s.fish[0].tooltip, Some(Tooltip::Text("School of Marlins".into())));
    let mut row = ship(202, 1);
    row.raw_unit_type = 99;
    data.radar.ships.contacts = Some(vec![row]);
    assert_eq!(snapshot(&data).ships[0].symbol, Symbol::EnemyShipyard);
}

#[test]
fn channel_clear_does_not_clear_other_radars_and_off_chunks_stay_unpublished() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge) = setup()?;
    rt.lua
        .load("clears=0; UIParent:SetEventHandler('CLEAR_TELESCOPE_INFO',function(...) assert(select('#',...)==0);clears=clears+1 end)")
        .exec()?;
    for event in [
        RadarEvent::ShipToggled(RadarToggle { on: true, range: 100. }),
        RadarEvent::FishToggled(RadarToggle { on: true, range: 200. }),
        RadarEvent::ShipContacts {
            last: true,
            contacts: vec![ship(201, 10)],
        },
        RadarEvent::FishContacts {
            last: true,
            contacts: vec![FishRadarContact {
                obj_id: 301,
                template_id: 1,
                pos: Pos::default(),
            }],
        },
        RadarEvent::ShipToggled(RadarToggle { on: false, range: 0. }),
        RadarEvent::ShipContacts {
            last: true,
            contacts: vec![ship(201, 50)],
        },
    ] {
        dispatch(&rt, &bridge, &Event::Radar(event));
    }
    assert!(bridge.borrow().radar.ships.contacts.is_none());
    assert_eq!(bridge.borrow().radar.fish.contacts.as_ref().unwrap().len(), 1);
    rt.lua
        .load("assert(clears==1);assert(select('#',X2Map:GetTelescopeUnitHealth(201))==0)")
        .exec()?;
    dispatch(&rt, &bridge, &Event::Disconnected { message: "test".into() });
    assert!(bridge.borrow().radar.fish.contacts.is_none());
    Ok(())
}

#[test]
#[ignore = "loads the original world-stage bytecode and read-only game catalog"]
fn original_maps_consume_received_radar_and_keep_other_channels_when_ships_clear() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = fixture::runtime(1920., 1080.)?;
    let icons = |name: &str| -> usize {
        let value: Value = rt.lua.globals().get(name).unwrap();
        let id = aa_ui::lua::value_id(&rt.lua, &value).unwrap();
        let ui = aa_ui::lua::ui(&rt.lua);
        ui.tree
            .widget(id)
            .unwrap()
            .drawables
            .iter()
            .filter(|&&id| {
                ui.tree.visible(id)
                    && ui
                        .tree
                        .get(id)
                        .drawable()
                        .is_some_and(|d| d.texture.as_deref() == Some("ui/map/icon/npc_icon.dds"))
            })
            .count()
    };
    assert_eq!(icons("worldmap"), 8);
    assert_eq!(icons("roadmap"), 8);
    rt.lua
        .load("local hp,max=X2Map:GetTelescopeUnitHealth(201);assert(hp==720 and max==1000)")
        .exec()?;
    dispatch(&rt, &bridge, &Event::Radar(RadarEvent::ShipToggled(RadarToggle { on: false, range: 0. })));
    aa_ui::lua::tick(&rt.lua, 16.);
    assert_eq!(icons("worldmap"), 4);
    assert_eq!(icons("roadmap"), 4);
    rt.lua.load("assert(select('#',X2Map:GetTelescopeUnitHealth(201))==0)").exec()?;
    drop(rt);
    drop(bridge);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
