//! Original hidden information-window consumer over explicit loaded/bound receipts.
//! This is a controlled public-method case, not a claim about the mode-action producer.

use aa_net::{Event, model::UnitKind, slave::SlaveEvent};

struct BindingsFile(std::path::PathBuf);

impl Drop for BindingsFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
#[ignore = "requires the original World UI, game pak and read-only game database"]
fn original_bound_slave_header_uses_received_binding_and_hex_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = super::breath::fixture::runtime(1280., 720.)?;
    let _bindings = BindingsFile(path);
    let own = super::expansion::fixture::spawn();
    let mut bound = own.unit.clone();
    bound.obj_id = 0x9abc;
    bound.name = "Received bound catapult".into();
    bound.kind = UnitKind::Slave {
        id: 700,
        tl_id: 8,
        template_id: 1, // Original slaves row: Catapult, siege_weapon.
        summoner_id: 100,
    };
    let mut owned = bound.clone();
    owned.obj_id = 0x1234;
    owned.name = "Owned announcement is a different object".into();
    owned.kind = UnitKind::Slave {
        id: 701,
        tl_id: 9,
        template_id: 2,
        summoner_id: 100,
    };
    for event in [
        Event::CharacterSelected(Box::new(own.state.clone())),
        Event::PlayerSpawn(Box::new(own)),
        Event::UnitDetached {
            obj_id: 100,
            reason: 0,
        },
        Event::UnitSpawned(Box::new(owned)),
        Event::Slave(SlaveEvent::Created(aa_net::slave::SlaveCreated {
            owner_obj_id: 100,
            tl_id: 9,
            obj_id: 0x1234,
            hide_spawn_effect: true,
            raw_id: 0,
            creator_name: String::new(),
        })),
        Event::UnitSpawned(Box::new(bound)),
        Event::Slave(SlaveEvent::Bound {
            master_id: 47,
            obj_id: 0x9abc,
        }),
    ] {
        super::enchant::dispatch(Some(&rt), &state, &event, None);
    }
    rt.lua
        .load(
            r#"
        assert(slaveInfoFrame and not slaveInfoFrame:IsVisible(), 'original hidden window')
        local original = slaveInfoFrame.UpdateSlaveDefaultInfo
        local source = debug.getinfo(original, 'S')
        assert(source.what == 'Lua' and source.linedefined == 7, 'original Lua consumer')
        original(slaveInfoFrame)
        assert(slaveInfoFrame.UpdateSlaveDefaultInfo == original)
        assert(slaveInfoFrame.unitId == '9abc', 'received bound reference, not owned announcement')
        assert(slaveInfoFrame.name:GetText() == 'Received bound catapult')
        assert(slaveInfoFrame.class:GetText() == 'Catapult')
        assert(slaveInfoFrame.kind:GetText() == locale.map.slaveKind('siege_weapon'))
        assert(not slaveInfoFrame:IsVisible(), 'method case does not invent SHOW_SLAVE_INFO')
    "#,
        )
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    Ok(())
}
