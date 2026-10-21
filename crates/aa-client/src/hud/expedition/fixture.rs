//! The original guild permission screen on explicit received fixture data.
use super::*;
use aa_net::{ExpeditionFaction, ExpeditionMember};
use bevy::window::PrimaryWindow;

pub(super) fn seed(bridge: &Shared) -> aa_net::state::Player {
    let mut spawn = super::super::expansion::fixture::spawn();
    spawn.state.character.expedition_id = 900;
    spawn.unit.expedition_id = 900;
    let state = spawn.state.clone();
    dispatch(None, bridge, &Event::CharacterSelected(Box::new(state)));
    dispatch(None, bridge, &Event::PlayerSpawn(Box::new(spawn.clone())));
    dispatch(
        None,
        bridge,
        &Event::Expedition(ExpeditionEvent::Factions(vec![ExpeditionFaction {
            id: 900,
            mother_id: 2,
            name: b"The Silver Vale".to_vec(),
            owner_id: 47,
            owner_name: b"Breath fixture".to_vec(),
            ..default()
        }])),
    );
    dispatch(
        None,
        bridge,
        &Event::Expedition(ExpeditionEvent::Policies(
            [
                (0, "Member", [0, 0, 0, 0, 0, 1, 0, 0, 0]),
                (1, "Veteran", [0, 1, 0, 0, 0, 1, 0, 0, 0]),
                (2, "Officer", [1, 1, 1, 1, 0, 1, 1, 1, 1]),
                (3, "Steward", [1, 1, 1, 1, 0, 1, 1, 1, 1]),
                (255, "Leader", [1; 9]),
            ]
            .into_iter()
            .map(|(role, name, permissions)| ExpeditionRolePolicy {
                expedition_id: 900,
                role,
                name: name.as_bytes().to_vec(),
                permissions,
            })
            .collect(),
        )),
    );
    dispatch(
        None,
        bridge,
        &Event::Expedition(ExpeditionEvent::Members {
            total: 4,
            expedition_id: 900,
            members: [
                (47, "Breath fixture", 30, 255),
                (48, "Elowen", 40, 2),
                (49, "Rydel", 25, 0),
                (50, "Mara", 40, 1),
            ]
            .into_iter()
            .map(|(character_id, name, level, role)| ExpeditionMember {
                expedition_id: 900,
                character_id,
                name: name.as_bytes().to_vec(),
                level,
                role,
                online: u8::from(character_id != 49),
                in_party: u8::from(character_id == 48),
                zone_id: 142,
                id3: if character_id == 47 { 324 } else { 0 },
                last_world_leave_time: members::now() as i64 - 2 * 86_400,
                abilities: [1, 2, 3],
                ..default()
            })
            .collect(),
        }),
    );
    bridge.borrow_mut().character_id = 47;
    aa_net::state::Player { spawn: Some(spawn) }
}

pub(super) fn runtime(
    width: f32,
    height: f32,
) -> Result<(UiRuntime, Shared, std::path::PathBuf, aa_net::state::Player), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    let player = seed(&bridge);
    rt.lua
        .load("assert(X2Faction:IsExpedInfoLoaded());expedMgmt:ChangeTab(4);expedMgmt:Show(true);assert(expedMgmt:IsVisible())")
        .exec()?;
    Ok((rt, bridge, path, player))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path, mut player) = runtime(width as f32, height as f32).unwrap();
    if aa_dev::args().into_iter().any(|arg| arg == "--create") {
        let args: Vec<_> = aa_dev::args();
        let alliance = crate::ingame::enter::flag(&args, "--alliance")
            .and_then(|value| value.parse().ok())
            .unwrap_or(148);
        show_creation(&rt, &bridge, &mut player, alliance).unwrap();
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--rename") {
        rt.lua.load("expedMgmt:Show(false)").exec().unwrap();
        dispatch(Some(&rt), &bridge, &Event::Expedition(ExpeditionEvent::ShowRenameUi));
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--members") {
        show_members(&rt).unwrap();
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--invitation") {
        rt.lua.load("expedMgmt:Show(false)").exec().unwrap();
        let spawn = player.spawn.as_mut().unwrap();
        spawn.state.character.expedition_id = 0;
        spawn.unit.expedition_id = 0;
        dispatch(Some(&rt), &bridge, &Event::CharacterSelected(Box::new(spawn.state.clone())));
        dispatch(Some(&rt), &bridge, &Event::PlayerSpawn(Box::new(spawn.clone())));
        dispatch(
            Some(&rt),
            &bridge,
            &Event::Expedition(ExpeditionEvent::Invitation {
                inviter_id: 88,
                inviter_name: b"Elowen".to_vec(),
                expedition_id: 900,
                expedition_name: b"Unused packet label".to_vec(),
            }),
        );
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--dismiss") {
        rt.lua.load("X2Faction:DismissExpedition()").exec().unwrap();
        tasks::show(&rt.lua, &bridge);
    }
    if aa_dev::args().into_iter().any(|arg| arg == "--leave") {
        dispatch(
            Some(&rt),
            &bridge,
            &Event::Expedition(ExpeditionEvent::RoleChanged {
                character_id: 47,
                role: 0,
                name: b"Member".to_vec(),
            }),
        );
        rt.lua.load("expedMgmt:ChangeTab(4);X2Faction:LeaveExpedition()").exec().unwrap();
        tasks::show(&rt.lua, &bridge);
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original guild UI"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(player)
        .init_resource::<aa_net::state::Units>()
        .add_message::<aa_net::NetCommand>()
        .add_message::<aa_net::NetEvent>()
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, super::update)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

pub(super) fn show_members(rt: &UiRuntime) -> mlua::Result<()> {
    rt.lua.load("expedMgmt:ChangeTab(3);local tab=expedMgmt.tab.window[3];tab.viewOffMember:SetChecked(true,false);tab:RequestMemberInfo(1,true);assert(tab.frame:GetDataCount()==4)").exec()
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}

pub(super) fn show_creation(rt: &UiRuntime, bridge: &Shared, player: &mut aa_net::state::Player, alliance: u32) -> mlua::Result<()> {
    rt.lua.load("expedMgmt:Show(false);ShowCreateExpedition(false)").exec()?;
    let spawn = player.spawn.as_mut().unwrap();
    let faction = match alliance {
        148 => 101,
        149 => 109,
        114 => 161,
        other => other,
    };
    spawn.state.character.faction_id = faction;
    spawn.state.character.expedition_id = 0;
    spawn.unit.faction_id = faction;
    spawn.unit.expedition_id = 0;
    dispatch(Some(rt), bridge, &Event::CharacterSelected(Box::new(spawn.state.clone())));
    dispatch(Some(rt), bridge, &Event::PlayerSpawn(Box::new(spawn.clone())));
    let mut factions = vec![(101, 148), (109, 149), (161, 114), (148, 0), (149, 0), (114, 0)];
    if alliance >= 1000 {
        factions.push((alliance, 0));
    }
    dispatch(
        Some(rt),
        bridge,
        &Event::Expedition(ExpeditionEvent::Factions(
            factions
                .into_iter()
                .map(|(id, mother_id)| ExpeditionFaction {
                    id,
                    mother_id,
                    name: b"Creation fixture faction".to_vec(),
                    political_system: 1,
                    ..default()
                })
                .collect(),
        )),
    );
    rt.lua
        .load(format!(
            "assert(X2Faction:GetMyTopLevelFaction()=={alliance});ShowCreateExpedition(true,{alliance});assert(expedition:IsVisible())"
        ))
        .exec()
}
