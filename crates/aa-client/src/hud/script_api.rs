//! The HUD's side of the script runner (`crate::script`): what the runner may read of the HUD's shared state and the
//! requests it may queue, the same ones the windows queue (a quest's Accept button is `Req::Accept`).

use aa_net::Unit;
use bevy::prelude::*;

pub(crate) use super::api::HudBridge;
pub(crate) use super::chat::ScriptedKeys;
use super::dialog::Req;
pub(crate) use super::session::ScriptedSession;

/// What the dialog and the loot icons show right now.
#[derive(Debug, Default, Clone)]
pub struct HudFacts {
    /// The player's position (game coordinates).
    pub pos: Vec3,
    /// The NPC talked to: object id and template.
    pub talking_to: Option<(u32, u32)>,
    /// The quest dialog is up (the rest of the HUD hidden).
    pub directing: bool,
    /// Quests the NPC offers, quests it takes in.
    pub offers: Vec<u32>,
    pub completes: Vec<u32>,
    /// The nearest corpse in reach with loot: `(owner type, object id)`.
    pub loot_nearest: Option<(u16, u32)>,
    /// The loot window has rows.
    pub loot_rows: usize,
    /// A duel is running, a challenge waits for an answer (the challenger's character id), and the keys of the
    /// results of the duels so far (`result_draw`, `result_winner`, ...).
    pub duel_active: bool,
    pub duel_challenge: Option<u32>,
    pub duel_results: Vec<&'static str>,
}

pub fn facts(bridge: &HudBridge) -> HudFacts {
    let d = bridge.0.borrow();
    HudFacts {
        pos: d.pos,
        talking_to: d.dialog.npc,
        directing: d.dialog.directing,
        offers: d.dialog.start.clone(),
        completes: d.dialog.complete.clone(),
        loot_nearest: d.loot.nearest,
        loot_rows: d.loot.rows.len(),
        duel_active: d.duel.received.active.is_some(),
        duel_challenge: d.duel.received.challenge,
        duel_results: d.duel.results.iter().map(|r| r.key()).collect(),
    }
}

/// `friendly`, `neutral` or `hostile`: how the nameplates colour the unit for a player of `own_faction`.
pub fn relation(bridge: &HudBridge, own_faction: u32, unit: &Unit) -> &'static str {
    let d = bridge.0.borrow();
    super::plates::relation_of(d.game.get(), own_faction, unit)
}

/// The dialog's Accept (`quest`) / Complete (`quest`, reward) buttons.
pub fn accept(bridge: &HudBridge, quest: u32) {
    bridge.0.borrow_mut().dialog.requests.push(Req::Accept(quest));
}

pub fn complete(bridge: &HudBridge, quest: u32, reward: i32) {
    bridge.0.borrow_mut().dialog.requests.push(Req::Complete(quest, reward));
}

/// The talk is over (`X2Interaction:CancelNPCInteraction()`).
pub fn end_talk(bridge: &HudBridge) {
    bridge.0.borrow_mut().dialog.requests.push(Req::End);
}
