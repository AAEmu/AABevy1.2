//! Original family tab and optional invitation over explicit received fixtures.
use super::*;
use aa_net::family::{FamilyEvent, FamilyInvitation, FamilyMember, FamilyRoster};
use bevy::window::PrimaryWindow;

pub(super) fn runtime(width: f32, height: f32) -> Result<(UiRuntime, Shared, std::path::PathBuf, aa_net::state::Player), Box<dyn std::error::Error>> {
    let (rt, bridge, path) = super::super::breath::fixture::runtime(width, height)?;
    rt.lua.load("X2Option:SetItemFloatValue(OPTION_ITEM_IGNORE_FAMILY_INVITATION,0)").exec()?;
    let player = aa_net::state::Player {
        spawn: Some(super::super::expansion::fixture::spawn()),
    };
    bridge.borrow_mut().character_id = 47;
    dispatch(Some(&rt), &bridge, &Event::PlayerSpawn(Box::new(player.spawn.as_ref().unwrap().clone())));
    bridge.borrow_mut().family.changes = Some(true);
    let members = [
        (47, "Breath fixture", "", 1, true),
        (48, "Elowen", "Sister", 0, true),
        (49, "Rydel", "Brother", 0, false),
        (50, "Mara", "Cousin", 0, true),
    ]
    .into_iter()
    .map(|(character_id, name, title, role, online)| FamilyMember {
        character_id,
        name: name.as_bytes().to_vec(),
        title: title.as_bytes().to_vec(),
        role,
        online,
    })
    .collect();
    dispatch(
        Some(&rt),
        &bridge,
        &Event::Family(FamilyEvent::Described(FamilyRoster { family_id: 12, members })),
    );
    rt.lua.load("ToggleRelationshipUI(true);relationshipFrame.tab:SelectTab(relationshipFrame.tab.familyTabIdx);local tab=relationshipFrame.tab.window[3];tab.showOffMember:SetChecked(true,false);tab:FillMemberData();assert(tab.frame.dataCount==4)").exec()?;
    Ok((rt, bridge, path, player))
}

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge, path, player) = runtime(width as f32, height as f32).unwrap();
    if std::env::args().any(|arg| arg == "--invitation") {
        dispatch(
            Some(&rt),
            &bridge,
            &Event::Family(FamilyEvent::Invitation(FamilyInvitation {
                inviter_id: 88,
                inviter_name: b"Elowen".to_vec(),
                family_id: 24,
                role: b"Sibling".to_vec(),
            })),
        );
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original family UI"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(player)
        .add_message::<NetCommand>()
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, update)
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
