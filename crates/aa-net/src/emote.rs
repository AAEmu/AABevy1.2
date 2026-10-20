//! Emotes and speech bubbles: `SCEmotionExpressed`, `SCChatBubble`, `SCAreaChatBubble` and the request
//! `CSExpressEmotion`.
//!
//! The wire layouts are the server's (`systems/services/express_text.rs` and
//! `systems/skills/packets_special.rs`; the C# originals `CSExpressEmotionPacket.cs`,
//! `SCEmotionExpressedPacket.cs`, `SCChatBubblePacket.cs`, `SCAreaChatBubblePacket.cs`):
//!
//! | packet | body |
//! |---|---|
//! | `CSExpressEmotion` 0x0ad | `bc character_obj, bc target_obj, u32 emotion_id` |
//! | `SCEmotionExpressed` 0x12b | `bc obj, bc target_obj, u32 emotion_id` |
//! | `SCChatBubble` 0x154 | `bc obj, u8 kind, u8 how, then u32 bubble_id (how != 1) or string text (how == 1)` |
//! | `SCAreaChatBubble` 0x153 | `bool enter, bc obj, u32 bubble_type` |
//!
//! `emotion_id` is an `express_texts` row (its `anim_id` is the clip, its `me` / `other` ... the chat lines).
//! The server broadcasts the emote to everything in range, the sender included, so the sender's own
//! animation also starts from `SCEmotionExpressed`. `kind` of a bubble: 1 normal, 2 think, 3 and above as
//! the client's `CBK_*` (4: no bubble, blue text). The server sends bubbles only for the skill effect
//! `BubbleEffect` (`how` 2: a `bubbles` id) and, in the C# `DuelManager`, the area bubble 543 at a duel.
//! Say-chat bubbles are not a packet: the client draws them from `SCChatMessage`.
use crate::wire::{Reader, Result, WireError};

/// What a speech bubble shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BubbleBody {
    /// `how` 1: this text.
    Text(String),
    /// Any other `how`: the lines of the `bubbles` row with this id (`bubble_chats`).
    Id(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmoteEvent {
    /// `SCEmotionExpressed`: unit `obj_id` expresses emotion `emotion_id`, at `target_obj_id` (0: nobody).
    Expressed { obj_id: u32, target_obj_id: u32, emotion_id: u32 },
    /// `SCChatBubble`: a speech bubble over `obj_id`.
    ChatBubble { obj_id: u32, kind: u8, how: u8, body: BubbleBody },
    /// `SCAreaChatBubble`: the area bubble `bubble_type` starts (`enter`) or ends over `obj_id`.
    AreaBubble { enter: bool, obj_id: u32, bubble_type: u32 },
}

impl EmoteEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x12b | 0x153 | 0x154)
    }
    pub fn name(&self) -> &'static str {
        match self {
            Self::Expressed { .. } => "SCEmotionExpressed",
            Self::ChatBubble { .. } => "SCChatBubble",
            Self::AreaBubble { .. } => "SCAreaChatBubble",
        }
    }
    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            0x12b => Self::Expressed { obj_id: r.bc()?, target_obj_id: r.bc()?, emotion_id: r.u32()? },
            0x154 => {
                let obj_id = r.bc()?;
                let kind = r.u8()?;
                let how = r.u8()?;
                let body = if how == 1 { BubbleBody::Text(r.string()?) } else { BubbleBody::Id(r.u32()?) };
                Self::ChatBubble { obj_id, kind, how, body }
            }
            0x153 => Self::AreaBubble { enter: r.bool()?, obj_id: r.bc()?, bubble_type: r.u32()? },
            _ => return Err(WireError::Invalid("unknown emote opcode".into())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{game::{ClientMsg, ServerMsg}, wire::Writer};

    fn frame(opcode: u16, body: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(crate::game::MAGIC).u8(crate::game::LEVEL_GAME).u8(0).u8(0).u16(opcode).raw(body);
        w.finish()
    }

    #[test]
    fn server_goldens_decode_and_every_truncation_fails() {
        // The body of the server's `load_and_lookup` test (SCEmotionExpressed 0x10 0x20 5), the layouts of
        // `SCChatBubblePacket::write` for both `how` values and of the C# `SCAreaChatBubblePacket`.
        let cases: [(u16, Vec<u8>, EmoteEvent); 4] = [
            (0x12b, vec![0x10, 0, 0, 0x20, 0, 0, 5, 0, 0, 0], EmoteEvent::Expressed { obj_id: 0x10, target_obj_id: 0x20, emotion_id: 5 }),
            (
                0x154,
                vec![0x11, 0x22, 0x33, 2, 2, 0x9b, 0x01, 0, 0],
                EmoteEvent::ChatBubble { obj_id: 0x332211, kind: 2, how: 2, body: BubbleBody::Id(411) },
            ),
            (
                0x154,
                vec![0x11, 0, 0, 1, 1, 2, 0, b'h', b'i'],
                EmoteEvent::ChatBubble { obj_id: 0x11, kind: 1, how: 1, body: BubbleBody::Text("hi".into()) },
            ),
            (0x153, vec![1, 0x11, 0, 0, 0x1f, 0x02, 0, 0], EmoteEvent::AreaBubble { enter: true, obj_id: 0x11, bubble_type: 543 }),
        ];
        for (opcode, body, expected) in cases {
            let mut r = Reader::new(&body);
            assert_eq!(EmoteEvent::read(opcode, &mut r).unwrap(), expected);
            assert!(r.is_empty());
            assert_eq!(ServerMsg::decode(&frame(opcode, &body)).unwrap(), ServerMsg::Emote(expected));
            for n in 0..body.len() {
                assert!(EmoteEvent::read(opcode, &mut Reader::new(&body[..n])).is_err(), "{opcode:x}/{n}");
                assert!(ServerMsg::decode(&frame(opcode, &body[..n])).is_err(), "frame {opcode:x}/{n}");
            }
        }
        assert!(EmoteEvent::read(0x155, &mut Reader::new(&[])).is_err());
    }

    #[test]
    fn the_request_encodes_as_the_server_reads_it() {
        let m = ClientMsg::ExpressEmotion { obj_id: 0x10, target_obj_id: 0x20, emotion_id: 5 };
        assert_eq!(m.name(), "CSExpressEmotionPacket");
        assert_eq!(m.encode_body(), [0x10, 0, 0, 0x20, 0, 0, 5, 0, 0, 0]);
        assert_eq!(m.encode(3), [0xdd, 1, 0, 3, 0xad, 0, 0x10, 0, 0, 0x20, 0, 0, 5, 0, 0, 0]);
        let none = ClientMsg::ExpressEmotion { obj_id: 0x332211, target_obj_id: 0, emotion_id: 0x0102 };
        assert_eq!(none.encode_body(), [0x11, 0x22, 0x33, 0, 0, 0, 2, 1, 0, 0]);
    }
}
