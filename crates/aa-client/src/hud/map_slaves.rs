//! The player's own summoned ship or vehicle on the maps (`my_slave`, the original's `UPDATE_MY_SLAVE_POS_INFO`).
//!
//! The server reports the owned slave's position and health (`SCMySlave`, every 5 s and when it is summoned) whether or
//! not the slave is inside the unit view range, so the marker shows where the ship is far out at sea. aa-net keeps the
//! latest report in `Slaves::position` while the owned timeline lives (summon -> `SCSlaveRemoved`).
//! The carried-backpack markers (`UPDATE_CARRYING_BACKPACK_SLAVE_INFO`) have no data source: no packet gives the
//! client the position of other players' trade packs.

use aa_net::Slaves;

/// `(x, y, kind, tip)` of the own slave's marker; `None` while no slave is summoned or no report has come.
pub(super) fn own_slave_icon(slaves: &Slaves) -> Option<(f32, f32, &'static str, String)> {
    let own = slaves.own.as_ref()?;
    let info = slaves.position.as_ref().filter(|i| i.tl_id == own.tl_id)?;
    let pos = info.position();
    Some((pos.x, pos.y, "my_slave", info.name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::{SlaveCreated, SlavePositionInfo};

    #[test]
    fn marker_follows_the_owned_report() {
        let mut s = Slaves::default();
        assert!(own_slave_icon(&s).is_none());
        let info = SlavePositionInfo { obj_id: 5, tl_id: 7, name: "Ship".into(), template_id: 1, hp: 1, max_hp: 2, x: 0, y: 0, z: 0.0 };
        s.position = Some(info.clone());
        assert!(own_slave_icon(&s).is_none(), "a report without an owned timeline shows nothing");
        s.own = Some(SlaveCreated { owner_obj_id: 1, tl_id: 8, obj_id: 5, hide_spawn_effect: false, raw_id: 0, creator_name: String::new() });
        assert!(own_slave_icon(&s).is_none(), "another timeline");
        s.own.as_mut().unwrap().tl_id = 7;
        let (x, y, kind, tip) = own_slave_icon(&s).unwrap();
        assert_eq!((kind, tip.as_str()), ("my_slave", "Ship"));
        assert_eq!((x, y), (info.position().x, info.position().y));
    }
}
