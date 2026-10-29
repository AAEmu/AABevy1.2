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
//! SC080 follows original serializer397b4b00 with the accepted one-byte bool wire convention;
//! its raw byte does not establish concrete native archive normalization.

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
    /// `CSNaviTeleport`: teleport by the navi doodad `doodad_obj_id` (a func `DoodadFuncNaviTeleport` without a skill).
    /// The answer would be `SCNaviTeleport` (a position); the lane only logs the request.
    NaviTeleport { doodad_obj_id: u32 },
    /// `CSNaviOpenPortal`: open the portal of the navi doodad `doodad_obj_id` (`DoodadFuncNaviOpenPortal`, a Guide
    /// Gate). The lane only logs it.
    NaviOpenPortal { doodad_obj_id: u32 },
    /// `CSStartedCinema` 0x0cf / `CSCompletedCinema` 0x0ce (empty): the quest cinema of the current
    /// step began / ended. The server marks the step's `ObjCinema` act playing on the first and
    /// counts it done on the second; it never starts a cinema itself.
    Cinema { completed: bool },
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
            Self::NaviTeleport { .. } => cs::CSNaviTeleportPacket,
            Self::NaviOpenPortal { .. } => cs::CSNaviOpenPortalPacket,
            Self::Cinema { completed: false } => 0x0cf,
            Self::Cinema { completed: true } => 0x0ce,
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
            Self::NaviTeleport { doodad_obj_id } | Self::NaviOpenPortal { doodad_obj_id } => {
                w.bc(doodad_obj_id);
            }
            Self::Cinema { .. } => {}
        }
    }
}

/// Native SCWorldInteractionSkillList payload. Names for the two outer IDs remain
/// deliberately neutral: OnInteractionSkillList ignores them and forwards the inner context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldInteractionSkillList {
    pub outer_obj_ids: [u32; 2],
    pub context_obj_id: u32,
    pub doodad_obj_id: u32,
    pub interaction_type: u32,
    pub pick_obj_id: u32,
    /// The serializer transfers at most ten IDs, preserving an out-of-range announced count.
    pub raw_count: i32,
    pub extra_info: i32,
    pub skills: Vec<u32>,
    pub mouse_button: u8,
    pub modifier_keys: i32,
}

impl WorldInteractionSkillList {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let outer_obj_ids = [r.bc()?, r.bc()?];
        let context_obj_id = r.bc()?;
        let doodad_obj_id = r.bc()?;
        let interaction_type = r.u32()?;
        let pick_obj_id = r.bc()?;
        let raw_count = r.i32()?;
        let extra_info = r.i32()?;
        let mut skills = Vec::with_capacity(raw_count.clamp(0, 10) as usize);
        for _ in 0..raw_count.clamp(0, 10) { skills.push(r.u32()?); }
        Ok(Self { outer_obj_ids, context_obj_id, doodad_obj_id, interaction_type, pick_obj_id, raw_count, extra_info, skills,
            mouse_button: r.u8()?, modifier_keys: r.i32()? })
    }
}

/// What the server says about NPCs and quests besides the quest state (`Event::Interaction`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InteractEvent {
    WorldSkillList(WorldInteractionSkillList),
    /// Native cancellation uses BC unit ID followed by FULL u32 doodad ID.
    WorldCanceled { unit_obj_id: u32, doodad_obj_id: u32 },
    /// `SCNpcInteractionSkillList`, the answer to [`Interact::Start`]: the skills that use the NPC's
    /// function (`skills.id`: 12087 use store, 13238 warehouse, 12086 repair ...; 0 = none).
    SkillList { npc_obj_id: u32, obj_id: u32, extra_info: i32, pick_id: i32, mouse_button: u8, skills: Vec<u32>, interactable: bool, modifier_keys: i32 },
    /// SC080's observed wire status byte, independent of SC07b's skill-list `interactable`.
    /// The original archive uses a bool slot; noncanonical native normalization remains unknown.
    NpcStatusChanged { npc_obj_id: u32, status_wire_byte: u8 },
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
        matches!(opcode, sc::SCWorldInteractionSkillListPacket | sc::SCWorldInteractionCanceledPacket | sc::SCNpcInteractionSkillListPacket | sc::SCNpcInteractionStatusChangedPacket | sc::SCDoodadQuestAcceptPacket | sc::SCQuestRewardedByMailPacket | sc::SCNpcChatMessagePacket)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::WorldSkillList(_) => "SCWorldInteractionSkillList",
            Self::WorldCanceled { .. } => "SCWorldInteractionCanceled",
            Self::SkillList { .. } => "SCNpcInteractionSkillList",
            Self::NpcStatusChanged { .. } => "SCNpcInteractionStatusChanged",
            Self::DoodadQuestAccept { .. } => "SCDoodadQuestAccept",
            Self::RewardedByMail { .. } => "SCQuestRewardedByMail",
            Self::NpcChat { .. } => "SCNpcChatMessage",
        }
    }

    /// Reads the body of `opcode` (one of [`Self::handles`]).
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCWorldInteractionSkillListPacket => Self::WorldSkillList(WorldInteractionSkillList::read(r)?),
            sc::SCWorldInteractionCanceledPacket => Self::WorldCanceled { unit_obj_id: r.bc()?, doodad_obj_id: r.u32()? },
            sc::SCNpcInteractionStatusChangedPacket => Self::NpcStatusChanged { npc_obj_id: r.bc()?, status_wire_byte: r.u8()? },
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
mod npc_status_tests;

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
        // the 2026-10-28 capture: a right click on cart bc 0x5f3c (slave) sent this exact body
        let b = body(&Interact::Start { npc_obj_id: 0x5f3c, obj_id: 0, extra_info: 1, pick_id: 0, mouse_button: 3, modifier_keys: 0 });
        assert_eq!(b, [0x3C, 0x5F, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x00]);
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
        // CSNaviTeleport / CSNaviOpenPortal: bc doodad (the server's `CSNavi*Packet::read`)
        assert_eq!(body(&Interact::NaviTeleport { doodad_obj_id: 0x0A0B0C }), [0x0C, 0x0B, 0x0A]);
        assert_eq!(Interact::NaviTeleport { doodad_obj_id: 1 }.opcode(), 0x0e7);
        assert_eq!(Interact::NaviOpenPortal { doodad_obj_id: 1 }.opcode(), 0x0e8);
        // CSStartedCinema / CSCompletedCinema: empty
        assert!(body(&Interact::Cinema { completed: false }).is_empty());
        assert_eq!(Interact::Cinema { completed: false }.opcode(), 0x0cf);
        assert_eq!(Interact::Cinema { completed: true }.opcode(), 0x0ce);
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
    fn hex(raw: &str) -> Vec<u8> {
        raw.split_whitespace().flat_map(|word| (0..word.len()).step_by(2).map(move |i| u8::from_str_radix(&word[i..i+2],16).unwrap())).collect()
    }

    fn with_world_tail(opcode: u16, body: &[u8], expected: InteractEvent) {
        use crate::game::ServerMsg;
        let mut reader=Reader::new(body);
        assert_eq!(InteractEvent::read(opcode,&mut reader).unwrap(),expected);
        assert!(reader.is_empty());
        let mut plain=Writer::new();plain.raw(&[0xdd,1,0,1]).u16(opcode).raw(body);
        let expected=ServerMsg::Interaction(expected);
        assert_eq!(ServerMsg::decode(&plain.finish()).unwrap(),expected);
        let mut inner=Writer::new();inner.u16(0).u16(opcode).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let raw=inner.finish();let len=raw.len() as u16;
        let mut frame=Writer::new();frame.raw(&[0xdd,4]).u16(2).u8(1).u16(len).u16(!len).raw(&raw);
        assert_eq!(ServerMsg::decode(&frame.finish()).unwrap(),ServerMsg::Batch(vec![expected,ServerMsg::Snow(true)]));
        for cut in 0..body.len() { assert!(InteractEvent::read(opcode,&mut Reader::new(&body[..cut])).is_err(),"opcode {opcode:x} cut {cut}"); }
    }

    #[test]
    fn native_world_interaction_list_and_cancel_preserve_distinct_fields_and_dd04_alignment() {
        let body=hex("030201 c0b0a0 332211 665544 ffffffff 998877 02000000 feffffff 00000000 674523f1 fe 01000080");
        with_world_tail(sc::SCWorldInteractionSkillListPacket,&body,InteractEvent::WorldSkillList(WorldInteractionSkillList {
            outer_obj_ids:[0x010203,0xa0b0c0], context_obj_id:0x112233,doodad_obj_id:0x445566,
            interaction_type:u32::MAX,pick_obj_id:0x778899,raw_count:2,extra_info:-2,skills:vec![0,0xf1234567],mouse_button:254,modifier_keys:0x80000001u32 as i32,
        }));
        with_world_tail(sc::SCWorldInteractionCanceledPacket,&hex("563412 98badcfe"),InteractEvent::WorldCanceled{unit_obj_id:0x123456,doodad_obj_id:0xfedcba98});
    }

    #[test]
    fn native_world_list_transfers_at_most_ten_skills_but_preserves_raw_count() {
        for raw_count in [i32::MIN,-1,0,1,10,11,i32::MAX] {
            let count=raw_count.clamp(0,10);
            let mut w=Writer::new();
            w.bc(1).bc(2).bc(3).bc(4).u32(5).bc(6).i32(raw_count).i32(7);
            let skills:Vec<_>=(0..count as u32).collect();
            for skill in &skills { w.u32(*skill); }
            w.u8(2).i32(-3);
            let raw=w.finish();
            let list=WorldInteractionSkillList::read(&mut Reader::new(&raw)).unwrap();
            assert_eq!(list.raw_count,raw_count);assert_eq!(list.skills,skills);
            with_world_tail(sc::SCWorldInteractionSkillListPacket,&raw,InteractEvent::WorldSkillList(list));
        }
    }

}
