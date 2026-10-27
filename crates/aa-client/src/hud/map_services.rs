//! Where the map's transportation, gravesite and public farm icons come from.
//!
//! * `npc_station`: the carriage and airship conductors, the NPCs whose `npcs.npc_nickname_id` is 22 / 23
//!   ("Carriage Conductor") or 25 ("Airship Conductor"). They sit in `npc_map/<zone>.dat` like the bankers do.
//! * `common_farm`: the public farm, nursery and ranch managers (`npc_nickname_id` 49, "Public Farm Master").
//! * `npc_graveyard`: the resurrection points. The client's database names them (`return_points`, the
//!   `grave_*` ones) but has no position; the server's `Data/Portal/respawns.json` has (the dead are put at the
//!   nearest one), copied to `graveyards.tsv` so the client needs no server files.

/// The icon of an NPC's nickname (`npcs.npc_nickname_id`), for the NPCs the service flags leave without one.
pub fn nickname_icon(nickname: i64) -> Option<&'static str> {
    match nickname {
        22 | 23 | 25 => Some("npc_station"),
        49 => Some("common_farm"),
        _ => None,
    }
}

/// A resurrection point: game position and English name.
pub struct Graveyard {
    pub x: f32,
    pub y: f32,
    pub name: &'static str,
}

/// Every resurrection point of the world.
pub fn graveyards() -> impl Iterator<Item = Graveyard> {
    include_str!("graveyards.tsv").lines().filter(|l| !l.starts_with('#')).filter_map(|l| {
        let mut f = l.split('\t');
        let _id = f.next()?;
        Some(Graveyard { x: f.next()?.parse().ok()?, y: f.next()?.parse().ok()?, name: f.next()? })
    })
}

/// The resurrection points inside a map rectangle (west, south, width, height).
pub fn graveyards_in(rect: [f32; 4]) -> impl Iterator<Item = Graveyard> {
    graveyards().filter(move |g| g.x >= rect[0] && g.x < rect[0] + rect[2] && g.y >= rect[1] && g.y < rect[1] + rect[3])
}
