//! Talking to NPCs and the quest requests.
//!
//! What the client sends when the player talks to an NPC (`CSInteractNPC` when the talk starts,
//! `CSInteractNPCEnd` when it ends, `CSStartInteraction` for the NPC's function), accepts a quest
//! (`CSStartQuestContext`), hands one in (`CSCompleteQuestContext`), drops one
//! (`CSDropQuestContext`) or says what a talk objective asks for (`CSQuestTalkMade`), and the server's
//! answers that are not quest state (`SCNpcInteractionSkillList`, `SCDoodadQuestAccept`,
//! `SCQuestRewardedByMail`, `SCNpcChatMessage`). The quest state itself (started, updated, completed)
//! is [`crate::quest`].
//!
//! Layouts follow the server's `systems/npcs/npc/packets.rs` and `systems/quests/packets.rs` (the
//! C# `Read` / `Write` of the 1.2 packets).

use crate::opcodes::{cs, sc};
use crate::wire::{Reader, Result, Writer};

/// A request to the server about an NPC or a quest (`Command::Interact`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Interact {
    /// `CSInteractNPC`: the player starts talking to the unit `obj_id` (the server remembers it as the
    /// interaction object and, with `target_changed`, as the target) and answers with an empty
    /// `SCAiAggro`.
    Npc { obj_id: u32, target_changed: bool },
    /// `CSInteractNPCEnd`: the talk is over.
    NpcEnd { obj_id: u32 },
    /// `CSStartInteraction`: ask the NPC `npc_obj_id` for its function (store, bank, ...); `obj_id` is
    /// the player's unit. Answered by [`InteractEvent::SkillList`] with the same values echoed.
    Start { npc_obj_id: u32, obj_id: u32, extra_info: i32, pick_id: i32, mouse_button: u8, modifier_keys: i32 },
    /// `CSStartQuestContext`: accept quest `quest_id` from the NPC `npc_obj_id` (or the doodad
    /// `doodad_obj_id`, or the sphere `sphere_id`; all 0: the quest itself). Answered by
    /// `QuestStarted` (+ `QuestUpdated`), or nothing when the server refuses.
    QuestStart { quest_id: u32, npc_obj_id: u32, doodad_obj_id: u32, sphere_id: u32 },
    /// `CSCompleteQuestContext`: hand quest `quest_id` in at the NPC (or doodad); `selected` is the
    /// chosen reward of a quest with selective rewards (1 based, 0 for none). Answered by
    /// `QuestCompleted`, the rewards (`ExpChanged`, `ItemsChanged`) and maybe the next quest.
    QuestComplete { quest_id: u32, npc_obj_id: u32, doodad_obj_id: u32, selected: i32 },
    /// `CSDropQuestContext`: give quest `quest_id` up. Answered by `QuestUpdated` with status dropped.
    QuestDrop { quest_id: u32 },
    /// `CSQuestTalkMade`: the talk objective `act_id` (`quest_acts.id`) of component `component_id` of
    /// quest `quest_id` was done with the NPC `npc_obj_id`. Answered by `QuestUpdated`.
    QuestTalk { npc_obj_id: u32, quest_id: u32, component_id: u32, act_id: u32 },
}

impl Interact {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::Npc { .. } => cs::CSInteractNPCPacket,
            Self::NpcEnd { .. } => cs::CSInteractNPCEndPacket,
            Self::Start { .. } => cs::CSStartInteractionPacket,
            Self::QuestStart { .. } => cs::CSStartQuestContextPacket,
            Self::QuestComplete { .. } => cs::CSCompleteQuestContextPacket,
            Self::QuestDrop { .. } => cs::CSDropQuestContextPacket,
            Self::QuestTalk { .. } => cs::CSQuestTalkMadePacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match *self {
            Self::Npc { obj_id, target_changed } => {
                w.bc(obj_id).bool(target_changed);
            }
            Self::NpcEnd { obj_id } => {
                w.bc(obj_id);
            }
            Self::Start { npc_obj_id, obj_id, extra_info, pick_id, mouse_button, modifier_keys } => {
                w.bc(npc_obj_id).bc(obj_id).i32(extra_info).i32(pick_id).u8(mouse_button).i32(modifier_keys);
            }
            Self::QuestStart { quest_id, npc_obj_id, doodad_obj_id, sphere_id } => {
                w.u32(quest_id).bc(npc_obj_id).bc(doodad_obj_id).u32(sphere_id);
            }
            Self::QuestComplete { quest_id, npc_obj_id, doodad_obj_id, selected } => {
                w.u32(quest_id).bc(npc_obj_id).bc(doodad_obj_id).i32(selected);
            }
            Self::QuestDrop { quest_id } => {
                w.u32(quest_id);
            }
            Self::QuestTalk { npc_obj_id, quest_id, component_id, act_id } => {
                w.bc(npc_obj_id).u32(quest_id).u32(component_id).u32(act_id);
            }
        }
    }
}

/// What the server says about NPCs and quests besides the quest state (`Event::Interaction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractEvent {
    /// `SCNpcInteractionSkillList`, the answer to [`Interact::Start`]: the skills that use the NPC's
    /// function (`skills.id`: 12087 use store, 13238 warehouse, 12086 repair ...; 0 = none).
    SkillList { npc_obj_id: u32, obj_id: u32, extra_info: i32, pick_id: i32, mouse_button: u8, skills: Vec<u32>, interactable: bool, modifier_keys: i32 },
    /// `SCDoodadQuestAccept`: the doodad `doodad_obj_id` (a notice board ...) offers quest `quest_id`.
    DoodadQuestAccept { doodad_obj_id: u32, quest_id: u32 },
    /// `SCQuestRewardedByMail`: the rewards of these quests were sent by mail (the bag was full).
    RewardedByMail { quests: Vec<u32> },
    /// `SCNpcChatMessage`: an NPC says something (`kind` 1: the text `text_id` of the client's table,
    /// else `message`).
    NpcChat { chat_type: i16, faction_id: u32, npc_obj_id: u32, npc_name: String, character_obj_id: u32, kind: u8, text_id: u32, message: String },
}

impl InteractEvent {
    /// Whether `opcode` is one of the packets this module reads.
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, sc::SCNpcInteractionSkillListPacket | sc::SCDoodadQuestAcceptPacket | sc::SCQuestRewardedByMailPacket | sc::SCNpcChatMessagePacket)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::SkillList { .. } => "SCNpcInteractionSkillList",
            Self::DoodadQuestAccept { .. } => "SCDoodadQuestAccept",
            Self::RewardedByMail { .. } => "SCQuestRewardedByMail",
            Self::NpcChat { .. } => "SCNpcChatMessage",
        }
    }

    /// Reads the body of `opcode` (one of [`Self::handles`]).
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCNpcInteractionSkillListPacket => {
                let (npc_obj_id, obj_id, extra_info, pick_id, mouse_button) = (r.bc()?, r.bc()?, r.i32()?, r.i32()?, r.u8()?);
                let n = r.i32()?.max(0) as usize;
                let mut skills = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    skills.push(r.u32()?);
                }
                Self::SkillList { npc_obj_id, obj_id, extra_info, pick_id, mouse_button, skills, interactable: r.bool()?, modifier_keys: r.i32()? }
            }
            sc::SCDoodadQuestAcceptPacket => Self::DoodadQuestAccept { doodad_obj_id: r.bc()?, quest_id: r.u32()? },
            sc::SCQuestRewardedByMailPacket => {
                let n = r.u8()?;
                let mut quests = Vec::with_capacity(usize::from(n));
                for _ in 0..n {
                    quests.push(r.u32()?);
                }
                Self::RewardedByMail { quests }
            }
            _ => {
                let chat_type = r.i16()?;
                r.i16()?; // sub type (the faction, truncated)
                let (faction_id, npc_obj_id, npc_name, character_obj_id, kind) = (r.u32()?, r.bc()?, r.string()?, r.bc()?, r.u8()?);
                let (text_id, message) = if kind == 1 { (r.u32()?, String::new()) } else { (0, r.string()?) };
                Self::NpcChat { chat_type, faction_id, npc_obj_id, npc_name, character_obj_id, kind, text_id, message }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(m: &Interact) -> Vec<u8> {
        let mut w = Writer::new();
        m.write(&mut w);
        w.finish()
    }

    #[test]
    fn requests_match_the_servers_read() {
        // CSInteractNPC: bc obj, bool target changed (the server's `decodes_client_packets` bytes)
        assert_eq!(body(&Interact::Npc { obj_id: 0x200, target_changed: true }), [0x00, 0x02, 0x00, 1]);
        assert_eq!(body(&Interact::NpcEnd { obj_id: 0x010203 }), [0x03, 0x02, 0x01]);
        // CSStartInteraction: bc npc, bc obj, i32 extra, i32 pick, u8 mouse, i32 modifiers
        let b = body(&Interact::Start { npc_obj_id: 0x200, obj_id: 0x100, extra_info: -2, pick_id: 9, mouse_button: 1, modifier_keys: 4 });
        assert_eq!(b, [0x00, 0x02, 0x00, 0x00, 0x01, 0x00, 0xFE, 0xFF, 0xFF, 0xFF, 9, 0, 0, 0, 1, 4, 0, 0, 0]);
        // CSStartQuestContext: u32 quest, bc npc, bc doodad, u32 sphere
        let b = body(&Interact::QuestStart { quest_id: 330, npc_obj_id: 0x0A0B0C, doodad_obj_id: 0, sphere_id: 0 });
        assert_eq!(b, [0x4A, 0x01, 0, 0, 0x0C, 0x0B, 0x0A, 0, 0, 0, 0, 0, 0, 0]);
        // CSCompleteQuestContext: u32 quest, bc npc, bc doodad, i32 selected
        let b = body(&Interact::QuestComplete { quest_id: 330, npc_obj_id: 5, doodad_obj_id: 0, selected: 0 });
        assert_eq!(b, [0x4A, 0x01, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(body(&Interact::QuestDrop { quest_id: 330 }), [0x4A, 0x01, 0, 0]);
        // CSQuestTalkMade: bc npc, u32 quest, u32 component, u32 act
        let b = body(&Interact::QuestTalk { npc_obj_id: 7, quest_id: 1, component_id: 2, act_id: 3 });
        assert_eq!(b, [7, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
        assert_eq!(Interact::QuestStart { quest_id: 0, npc_obj_id: 0, doodad_obj_id: 0, sphere_id: 0 }.opcode(), 0x0d5);
        assert_eq!(Interact::Npc { obj_id: 0, target_changed: false }.opcode(), 0x065);
    }

    #[test]
    fn server_answers_decode() {
        // the server's `interaction_skill_list_bytes`
        #[rustfmt::skip]
        let bytes = [
            0x0C, 0x0B, 0x0A, 0x02, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 7, 0, 0, 0, 2,
            2, 0, 0, 0, 0x37, 0x2F, 0, 0, 5, 0, 0, 0, 1, 0x44, 0x33, 0x22, 0x11,
        ];
        let e = InteractEvent::read(sc::SCNpcInteractionSkillListPacket, &mut Reader::new(&bytes)).unwrap();
        assert_eq!(
            e,
            InteractEvent::SkillList { npc_obj_id: 0x0A0B0C, obj_id: 0x102, extra_info: -1, pick_id: 7, mouse_button: 2, skills: vec![12087, 5], interactable: true, modifier_keys: 0x11223344 }
        );
        // the server's `npc_chat_bytes`, both kinds
        let bytes = [0xFD, 0xFF, 0x02, 0x00, 0x02, 0x00, 0x01, 0x00, 5, 0, 0, 1, 0, b'N', 6, 0, 0, 1, 77, 0, 0, 0];
        match InteractEvent::read(sc::SCNpcChatMessagePacket, &mut Reader::new(&bytes)).unwrap() {
            InteractEvent::NpcChat { chat_type: -3, faction_id: 0x10002, npc_obj_id: 5, npc_name, character_obj_id: 6, kind: 1, text_id: 77, .. } => assert_eq!(npc_name, "N"),
            e => panic!("{e:?}"),
        }
        let bytes = [0xFD, 0xFF, 0x02, 0x00, 0x02, 0x00, 0x01, 0x00, 5, 0, 0, 1, 0, b'N', 6, 0, 0, 0, 1, 0, b'x'];
        match InteractEvent::read(sc::SCNpcChatMessagePacket, &mut Reader::new(&bytes)).unwrap() {
            InteractEvent::NpcChat { kind: 0, message, .. } => assert_eq!(message, "x"),
            e => panic!("{e:?}"),
        }
        let e = InteractEvent::read(sc::SCDoodadQuestAcceptPacket, &mut Reader::new(&[1, 2, 0, 250, 0, 0, 0])).unwrap();
        assert_eq!(e, InteractEvent::DoodadQuestAccept { doodad_obj_id: 0x201, quest_id: 250 });
        let e = InteractEvent::read(sc::SCQuestRewardedByMailPacket, &mut Reader::new(&[2, 1, 0, 0, 0, 2, 0, 0, 0])).unwrap();
        assert_eq!(e, InteractEvent::RewardedByMail { quests: vec![1, 2] });
        assert!(InteractEvent::handles(0x07b) && !InteractEvent::handles(0x147));
    }
}
