//! Component fixture. NPC resolution, population and buffs are explicit synthetic
//! inputs; it does not establish retail entry, origin mapping or live settlement.
use super::*;
use aa_net::{DominionData, DominionEvent, DominionTerritory, ExpeditionEvent, ExpeditionFaction};
use aa_ui::UiRuntime;

pub(super) fn runtime(
    width: f32,
    height: f32,
    open: bool,
    leader: bool,
) -> Result<(UiRuntime, Shared, std::path::PathBuf), Box<dyn std::error::Error>> {
    let (rt, shared, path) = super::super::breath::fixture::runtime(width, height)?;
    let mut spawn = super::super::expansion::fixture::spawn();
    spawn.state.character.expedition_id = 900;
    spawn.unit.expedition_id = 900;
    let character = spawn.character().id;
    let apply = |event: &Event| super::super::expedition::dispatch(None, &shared, event);
    apply(&Event::CharacterSelected(Box::new(spawn.state.clone())));
    apply(&Event::PlayerSpawn(Box::new(spawn)));
    apply(&Event::Expedition(ExpeditionEvent::Factions(vec![
        ExpeditionFaction {
            id: 900,
            mother_id: 2,
            name: b"Fixture expedition".to_vec(),
            owner_id: if leader { character } else { 999 },
            owner_name: b"Fixture lord".to_vec(),
            ..default()
        },
    ])));
    apply(&Event::Dominion(DominionEvent::Data {
        data: DominionData {
            zone_group_id: 33,
            expedition_id: 900,
            obj_id: 900,
            tax_rate: 17,
            peace_tax_money: 12_345_678,
            peace_tax_aa_point: 2345,
            reign_start_time: 1_700_000_000,
            territory: DominionTerritory {
                id: 10,
                id2: 4771,
                max_gates: 4,
                max_walls: 100,
                radius_dominion: 100,
                ..default()
            },
            ..default()
        },
        newly_declared: 0,
        final_data_by_request: 1,
    }));
    {
        let mut data = shared.borrow_mut();
        data.character_id = character;
        data.dialog.npc = Some((201, 42));
        // Deliberately supplied fixture endpoints, not a real resolved NPC or
        // inferred loaded population. Production cannot create this context yet.
        data.territory.context = Some(Context {
            npc: (201, 42),
            zone: 33,
        });
        data.territory.guard = Some(guard::Inputs {
            tower: 900,
            buffs: vec![4772],
            gates: 2,
            walls: 31,
        });
    }
    if open {
        repair_window_events(&rt)?;
        // Exact original callback payload: two arguments, with a numeric group.
        rt.fire_event("NPC_INTERACTION_START", ("dominion_npc", 33));
        rt.lua.load("assert(territoryFrame:IsVisible());assert(territoryFrame.taxRate==17);assert(X2House:GetGuardTowerStepInfo()~=nil)").exec()?;
    }
    Ok((rt, shared, path))
}

pub(crate) fn run() {
    let args: Vec<_> = std::env::args().collect();
    let has = |arg: &str| args.iter().any(|a| a == arg);
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, shared, path) = runtime(
        width as f32,
        height as f32,
        !has("--closed"),
        !has("--nonowner"),
    )
    .unwrap();
    if has("--guard") {
        rt.lua
            .load("territoryFrame.tab:SelectTab(2)")
            .exec()
            .unwrap();
    }
    if let Some(rate) = crate::ingame::enter::flag(&args, "--tax-rate")
        .and_then(|s| s.parse::<u8>().ok())
        .filter(|n| *n <= 50)
    {
        rt.lua
            .load(format!(
                "territoryFrame.tab.window[1].taxRateSlider:SetValue({rate},true)"
            ))
            .exec()
            .unwrap();
    }
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original territory UI — synthetic inputs"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(shared))
        .add_systems(Startup, (physical_pixels, report_scope))
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

fn report_scope() {
    info!(
        "territory fixture: original UI, synthetic received ownership/NPC/tower/population inputs; no live service, origin or settlement proof"
    );
}

fn physical_pixels(mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
