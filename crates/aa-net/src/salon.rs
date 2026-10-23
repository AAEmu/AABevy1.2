//! The beauty salon: entering and leaving it, paying for a new look, and the answers.
//!
//! The layouts are the server's (`systems/services/beauty_salon.rs`, `systems/skills/packets_special.rs`):
//!
//! | packet | body |
//! |---|---|
//! | `CSEnterBeautySalon` 0x12e | empty |
//! | `CSExitBeautySalon` 0x12c | empty |
//! | `CSBeautyshopData` 0x12d | `u32 hair item template`, [`ModelParams`] |
//! | `SCToggleBeautyshopResponse` 0x1f1 | `u8 state` (1 in the salon, 0 out) |
//! | `SCCharacterGenderAndModelModified` 0x1f3 | `u32 character id`, `u32 hair item template`, [`ModelParams`], `u32 0` |
//!
//! The salon is entered by casting the "Enter the beauty salon" skill (22780) on the salon helper
//! NPC (or by the cash shop); the client then asks `CSEnterBeautySalon`, the server answers the
//! toggle (and puts the buff `InBeautySalon` on the player). Paying needs a Salon Certificate in the
//! bag; the server sets the new look, takes the item and tells everyone around with the modified
//! packet. The client leaves with `CSExitBeautySalon`, answered by the toggle (0).
use crate::model::ModelParams;
use crate::opcodes::{cs, sc};
use crate::wire::{Reader, Result, Writer};

/// The Salon Certificate (`items.id`) a new look costs one of.
pub const SALON_CERTIFICATE: u32 = 30811;

/// A request of the salon (`Command::Salon`).
#[derive(Clone, Debug, PartialEq)]
pub enum SalonCommand {
    /// `CSEnterBeautySalon`
    Enter,
    /// `CSExitBeautySalon`
    Exit,
    /// `CSBeautyshopData`: apply this hair style (an `item_body_parts.item_id`) and appearance.
    Apply { hair: u32, params: ModelParams },
}

impl SalonCommand {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::Enter => cs::CSEnterBeautySalonPacket,
            Self::Exit => cs::CSExitBeautySalonPacket,
            Self::Apply { .. } => cs::CSBeautyshopDataPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        if let Self::Apply { hair, params } = self {
            w.u32(*hair);
            params.write(w);
        }
    }
}

/// What the server tells about the salon (`Event::Salon`).
#[derive(Clone, Debug, PartialEq)]
pub enum SalonEvent {
    /// `SCToggleBeautyshopResponse`: the player is in (`true`) or out of the salon.
    Toggled(bool),
    /// `SCCharacterGenderAndModelModified`: character `character_id` has a new look (its hair item
    /// template, 0 for none, and appearance).
    Modified { character_id: u32, hair: u32, params: ModelParams },
}

impl SalonEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, sc::SCToggleBeautyshopResponsePacket | sc::SCCharacterGenderAndModelModifiedPacket)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Toggled(_) => "SCToggleBeautyshopResponse",
            Self::Modified { .. } => "SCCharacterGenderAndModelModified",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCToggleBeautyshopResponsePacket => Self::Toggled(r.u8()? != 0),
            _ => {
                let character_id = r.u32()?;
                let hair = r.u32()?;
                let params = ModelParams::read(r)?;
                r.u32()?;
                Self::Modified { character_id, hair, params }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toggle_and_the_modified_look_decode() {
        let mut r = Reader::new(&[1]);
        assert_eq!(SalonEvent::read(sc::SCToggleBeautyshopResponsePacket, &mut r).unwrap(), SalonEvent::Toggled(true));
        assert_eq!(SalonEvent::read(sc::SCToggleBeautyshopResponsePacket, &mut Reader::new(&[0])).unwrap(), SalonEvent::Toggled(false));
        assert!(SalonEvent::read(sc::SCToggleBeautyshopResponsePacket, &mut Reader::new(&[])).is_err());

        let params = ModelParams { kind: ModelParams::SKIN, hair_color: 7, skin_color: 9, model_id: 11, face: None };
        let mut w = Writer::new();
        w.u32(42).u32(1234);
        params.write(&mut w);
        w.u32(0);
        let body = w.finish();
        let event = SalonEvent::read(sc::SCCharacterGenderAndModelModifiedPacket, &mut Reader::new(&body)).unwrap();
        assert_eq!(event, SalonEvent::Modified { character_id: 42, hair: 1234, params });
        for n in 0..body.len() {
            assert!(SalonEvent::read(sc::SCCharacterGenderAndModelModifiedPacket, &mut Reader::new(&body[..n])).is_err(), "{n}");
        }
    }

    #[test]
    fn the_requests_encode_as_the_server_reads_them() {
        assert_eq!(SalonCommand::Enter.opcode(), 0x12e);
        assert_eq!(SalonCommand::Exit.opcode(), 0x12c);
        let apply = SalonCommand::Apply { hair: 5, params: ModelParams { kind: ModelParams::HAIR, hair_color: 3, ..ModelParams::default() } };
        assert_eq!(apply.opcode(), 0x12d);
        let mut w = Writer::new();
        apply.write(&mut w);
        assert_eq!(w.finish(), [5, 0, 0, 0, 1, 3, 0, 0, 0]);
        let mut w = Writer::new();
        SalonCommand::Enter.write(&mut w);
        assert!(w.finish().is_empty());
    }
}
