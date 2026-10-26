//! Original received specialty error/known-bundle quotes; explicit fixture data, no Session.
use super::*;
use bevy::window::PrimaryWindow;

pub(in crate::hud) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, state, path) = crate::hud::breath::fixture::runtime(width as f32, height as f32).unwrap();
    let args: Vec<String> = aa_dev::args();
    let quote = args
        .iter()
        .position(|a| a == "--specialty-quote")
        .map(|i| match args.get(i + 1).map(String::as_str) {
            Some("money") => (4473, 6, 130),
            Some("coin") => (4482, 6, 130),
            Some("full") => (4482, 0, -1),
            _ => panic!("--specialty-quote requires money, coin or full"),
        });
    if let Some((npc, grade, _)) = quote {
        prepare_quote(&rt, &state, npc, grade).unwrap();
    }
    // Explicit PRE cache without a UI callback; inject the requested receipt
    // after2s so a3s capture retains a normal FPS sample and original UI output.
    receive(None, &state, &Event::SpecialtyRatio(137));
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("aa-client: original specialty receipts"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .insert_non_send(HudBridge(state))
        .insert_resource(FixtureRatio(quote.map_or(0, |(_, _, ratio)| ratio)))
        .add_message::<NetEvent>()
        .insert_resource(crate::hud::HudBuilt { up: true, fresh: false })
        .add_systems(PreUpdate, show_error)
        .add_systems(Startup, physical_pixels);
    crate::hud::enchant::plugin(&mut app);
    app.run();
    if path.exists() {
        std::fs::remove_file(path).unwrap();
    }
}

#[derive(Resource)]
struct FixtureRatio(i32);

fn show_error(mut sent: Local<bool>, time: Res<Time>, ratio: Res<FixtureRatio>, mut events: MessageWriter<NetEvent>) {
    if !*sent && time.elapsed_secs() >= 2.0 {
        events.write(NetEvent(Event::SpecialtyRatio(ratio.0)));
        *sent = true;
    }
}

pub(super) fn prepare_quote(rt: &aa_ui::UiRuntime, state: &Shared, template: u32, grade: u8) -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{
        items::InvenChunk,
        model::{Item, UnitKind, slot},
        state::Inventory,
    };
    crate::hud::api::mark_pushed(state);
    let mut spawn = crate::hud::expansion::fixture::spawn();
    let item = Item {
        id: 123,
        template_id: 20090,
        grade,
        count: 1,
        ..default()
    };
    spawn.unit.equipment.slots[slot::BACKPACK] = Some(item.clone());
    spawn.state.character.equipment.slots[slot::BACKPACK] = Some(item);
    let mut npc = spawn.unit.clone();
    npc.obj_id = 500;
    npc.kind = UnitKind::Npc {
        template_id: template,
        owner_id: 0,
    };
    let events = [
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
            items: vec![None; 50],
            truncated: false,
        }),
        Event::UnitSpawned(Box::new(npc)),
        Event::TargetChanged {
            obj_id: 100,
            target_obj_id: 500,
        },
    ];
    let mut inventory = Inventory::default();
    for event in &events {
        inventory.apply(event);
        crate::hud::enchant::dispatch(None, state, event, None);
    }
    {
        let mut data = state.borrow_mut();
        let mut world = World::new();
        world.insert_resource(inventory);
        data.panels = crate::hud::panels::refresh(&world, data.game.get(), 0, 100, data.start, &data.snapshot, Vec3::ZERO).0;
        data.panels.actions_known = true;
    }
    crate::hud::api::push_changes(rt, state);
    rt.lua
        .load(
            r#"
        assert(X2Equipment:GetEquippedItemTooltipText('player',ES_BACKPACK).itemType==20090)
        local request=X2DialogManager.RequestDefaultDialog
        X2DialogManager.RequestDefaultDialog=function(self,handler,owner)
            local dialog=request(self,handler,owner)
            SPECIALTY_DIALOG=dialog
            return dialog
        end
    "#,
        )
        .exec()?;
    Ok(())
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    for mut window in &mut windows {
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}
