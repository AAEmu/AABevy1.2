//! Original composer over explicit received paper, instrument and proficiency fixtures.
use super::*;
use aa_net::{items::InvenChunk, model::Item};
use std::path::PathBuf;

pub(in crate::hud) fn runtime() -> Result<(UiRuntime, Shared, PathBuf), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::super::breath::fixture::runtime(1280., 900.)?;
    let mut spawn = super::super::expansion::fixture::spawn();
    let instrument = Item {
        id: 200,
        template_id: 6177,
        count: 1,
        ..default()
    };
    spawn.state.character.equipment.slots[aa_net::model::slot::MUSICAL] = Some(instrument.clone());
    spawn.unit.equipment.slots[aa_net::model::slot::MUSICAL] = Some(instrument);
    let mut items = vec![None; 50];
    items[3] = Some(Item {
        id: 9_007_199_254_740_993,
        template_id: 27902,
        count: 1,
        grade: 2,
        made_unit_id: 77,
        ..default()
    });
    for event in [
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::InventoryInit {
            bag_slots: 50,
            bank_slots: 50,
        },
        Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 5,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ] {
        super::super::enchant::dispatch(None, &state, &event, None);
    }
    let mut received = aa_net::Actabilities::default();
    received.apply(&Event::Actability(aa_net::ActabilityEvent::Snapshot {
        last: true,
        entries: vec![aa_net::ActabilityValue {
            id: 33,
            point: 0,
            step: 0,
        }],
    }));
    {
        let mut world = World::new();
        world.insert_resource(received.clone());
        let mut data = state.borrow_mut();
        data.character_id = 47;
        super::super::crafting::refresh(&world, &mut data);
        data.user_music.actabilities = received;
    }
    state
        .borrow()
        .user_music
        .cache
        .borrow_mut()
        .identity(Some(17), 47, 100);
    cache::sync_directory(&rt.lua, &state.borrow().user_music.cache);
    Ok((rt, state, path))
}

pub(super) fn open(rt: &UiRuntime) -> mlua::Result<()> {
    rt.fire_event("OPEN_MUSIC_SHEET", (true, "9007199254740993"));
    aa_ui::lua::tick(&rt.lua, 16.);
    aa_ui::lua::flush(&rt.lua);
    let score: Value = rt.lua.load("composition.score").eval()?;
    let score_id = aa_ui::lua::value_id(&rt.lua, &score).expect("original composer exists");
    for (suffix, global) in [
        ("sampleButton", "MUSIC_SAMPLE"),
        ("saveButton", "MUSIC_SAVE"),
    ] {
        let ids: Vec<_> = aa_ui::lua::ui(&rt.lua)
            .tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, object)| object.parent == Some(score_id) && object.name.ends_with(suffix))
            .map(|(id, _)| id as aa_ui::tree::Id)
            .collect();
        assert_eq!(ids.len(), 1);
        rt.lua
            .globals()
            .set(global, aa_ui::lua::object_table(&rt.lua, ids[0])?)?;
    }
    Ok(())
}

pub(in crate::hud) fn run() {
    let (rt, state, path) = runtime().unwrap();
    if !std::env::args().any(|arg| arg == "--before") {
        open(&rt).unwrap();
        rt.lua.load("composition.score.titleEdit:SetText('First Song'); composition.score.scoreEdit:SetText('T120 O5 L8 C D E F G A B > C');").exec().unwrap();
        if std::env::args().any(|arg| arg == "--preview") {
            super::super::expansion::fixture::click(&rt, "MUSIC_SAMPLE").unwrap();
        }
    }
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: original composer"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .add_plugins(aa_audio::AaAudioPlugin::default())
        .init_resource::<playback::Playback>()
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .add_systems(
            Update,
            |bridge: NonSend<HudBridge>,
             engine: Res<aa_audio::Engine>,
             mut playback: ResMut<playback::Playback>| {
                let mut data = bridge.0.borrow_mut();
                playback.update(
                    Some(&engine),
                    data.user_music.generation,
                    data.user_music.preview.take(),
                );
            },
        )
        .run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}
