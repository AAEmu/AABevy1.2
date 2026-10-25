//! Explicit synthetic component endpoints. These do not establish live offers.
use super::*;

pub(in crate::hud) fn begin(data: &mut HudData, player: &Player, unit: &aa_net::model::Unit) {
    let spawn = player.spawn.as_ref().expect("fixture player");
    let UnitKind::Npc { template_id, .. } = unit.kind else {
        panic!("fixture NPC")
    };
    data.character_id = spawn.character().id;
    data.snapshot.unit_id = spawn.obj_id.to_string();
    let flow = &mut data.dialog.npc_flow;
    flow.observe(&Event::PlayerSpawn(Box::new(spawn.clone())), true);
    flow.observe(&Event::EnteredWorld, true);
    flow.observe(&Event::UnitSpawned(Box::new(unit.clone())), true);
    flow.begin_frame();
    flow.close_owed = false;
    assert!(
        flow.query(
            (spawn.character().id, spawn.obj_id),
            (unit.obj_id, template_id)
        )
        .is_some()
    );
}

pub(in crate::hud) fn receipt(data: &mut HudData, event: &Event) -> Option<SelectedNpc> {
    let flow = &mut data.dialog.npc_flow;
    flow.observe(event, true);
    flow.derive(false);
    let selected = flow.take_default()?;
    assert!(flow.stage(selected));
    data.dialog.npc = Some(selected.npc());
    assert!(data.dialog.npc_flow.finish(selected, true));
    Some(selected)
}

pub(in crate::hud) fn selected(
    data: &mut HudData,
    player: &Player,
    unit: &aa_net::model::Unit,
    skill: u32,
) -> SelectedNpc {
    begin(data, player, unit);
    receipt(
        data,
        &Event::Interaction(InteractEvent::SkillList {
            npc_obj_id: unit.obj_id,
            obj_id: 0,
            extra_info: 0,
            pick_id: 0,
            mouse_button: 2,
            modifier_keys: 0,
            skills: vec![skill],
            interactable: true,
        }),
    )
    .expect("fixture selected action")
}
