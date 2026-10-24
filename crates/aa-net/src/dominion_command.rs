//! Original Dominion requests. Sending never changes received territory state.
use crate::wire::Writer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DominionCommand {
    /// CS012: whole percent, inclusive 0..=50.
    ChangeTaxRate { zone_group_id: u16, tax_rate: i32 },
    /// CS013: native integer scale, 1000 = 100 percent. Not registered by the lane.
    ChangeNationalTaxRate { zone_group_id: u16, tax_rate: i32 },
    /// CS02d: persistent character identity, independently of actor object IDs.
    RequestCharacterBrief { requested_id: u32 },
}

impl DominionCommand {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        match *self {
            Self::ChangeTaxRate {
                zone_group_id,
                tax_rate,
            } => {
                if zone_group_id == 0 || !(0..=50).contains(&tax_rate) {
                    return Err("Dominion tax requires a nonzero zone group and rate 0..=50");
                }
            }
            Self::ChangeNationalTaxRate {
                zone_group_id,
                tax_rate,
            } => {
                if zone_group_id == 0 || !(0..=1000).contains(&tax_rate) {
                    return Err("national tax requires a nonzero zone group and rate 0..=1000");
                }
            }
            Self::RequestCharacterBrief { requested_id: 0 } => {
                return Err("character brief requires a nonzero persistent identity");
            }
            Self::RequestCharacterBrief { .. } => {}
        }
        Ok(())
    }

    pub(crate) fn opcode(&self) -> u16 {
        match self {
            Self::ChangeTaxRate { .. } => 0x012,
            Self::ChangeNationalTaxRate { .. } => 0x013,
            Self::RequestCharacterBrief { .. } => 0x02d,
        }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        match *self {
            Self::ChangeTaxRate {
                zone_group_id,
                tax_rate,
            }
            | Self::ChangeNationalTaxRate {
                zone_group_id,
                tax_rate,
            } => {
                w.u16(zone_group_id).i32(tax_rate);
            }
            Self::RequestCharacterBrief { requested_id } => {
                w.u32(requested_id);
            }
        }
    }
}
