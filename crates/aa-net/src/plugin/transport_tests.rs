//! Exercise the actual receive bridge without starting NetPlugin or a network thread.
use super::*;
use bevy::ecs::message::{MessageCursor, Messages};
use bevy::ecs::system::RunSystemOnce;

pub(crate) fn forward_through_bridge(client: Client) -> Vec<Event> {
    let mut app = App::new();
    init_state(&mut app);
    app.add_message::<NetEvent>();
    app.insert_resource(NetClient(Mutex::new(client)));
    let unit = crate::state::tests::unit(0x112233);
    app.world_mut()
        .resource_mut::<Units>()
        .by_id
        .insert(unit.obj_id, unit.clone());
    app.world_mut().clear_trackers();
    app.world_mut().run_system_once(receive_events).unwrap();
    let mut cursor = MessageCursor::<NetEvent>::default();
    let events: Vec<_> = cursor
        .read(app.world().resource::<Messages<NetEvent>>())
        .map(|event| event.0.clone())
        .collect();
    assert_eq!(
        app.world().resource::<Units>().get(unit.obj_id),
        Some(&unit)
    );
    assert_eq!(app.world().resource::<Units>().by_id.len(), 1);
    assert!(app.world().resource::<Player>().spawn.is_none());
    assert!(app.world().resource::<Slaves>().created.is_empty());
    assert!(
        app.world()
            .resource::<crate::DoodadBonds>()
            .by_unit
            .is_empty()
    );
    assert!(
        !app.world()
            .get_resource_ref::<Units>()
            .unwrap()
            .is_changed()
    );
    assert!(
        !app.world()
            .get_resource_ref::<Player>()
            .unwrap()
            .is_changed()
    );
    assert!(
        !app.world()
            .get_resource_ref::<Slaves>()
            .unwrap()
            .is_changed()
    );
    assert!(
        !app.world()
            .get_resource_ref::<Mates>()
            .unwrap()
            .is_changed()
    );
    assert!(
        !app.world()
            .get_resource_ref::<crate::DoodadBonds>()
            .unwrap()
            .is_changed()
    );
    app.world_mut().run_system_once(receive_events).unwrap();
    assert!(
        cursor
            .read(app.world().resource::<Messages<NetEvent>>())
            .next()
            .is_none()
    );
    events
}
