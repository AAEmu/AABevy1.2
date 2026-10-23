//! Received housing metadata and outgoing housing commands.
//!
//! The layouts follow the AAEmu server writers (`systems/housing/packets.rs`,
//! `models/house.rs::House::write` and the `CS*House*` readers): the model is one fixed
//! `u32` (not a PISC pair), strings are `i16`-length prefixed and the sale price is a
//! `u32`. The native client's 128-byte name cap is a local buffer limit, not a wire
//! constraint, so it is not imposed here.

use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{coord_to_long, long_to_coord, Pos, Reader, Result, WireError, Writer};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct HouseSnapshot {
    pub tl_id: u16,
    pub db_id: u32,
    pub obj_id: u32,
    pub template_id: u32,
    pub model_id: u32,
    pub co_owner_id: u32,
    pub owner_id: u32,
    pub owner_name: String,
    pub account_id: u32,
    /// Raw permission enum, without assuming unknown values are public/private.
    pub permission: u8,
    pub all_actions: i32,
    pub current_actions: i32,
    pub tax: i32,
    pub x: i64,
    pub y: i64,
    pub z: f32,
    pub name: String,
    pub allow_recover: bool,
    /// Server `u32` sale price, kept as signed bits (the native field is signed).
    pub sell_price: i32,
    pub sell_to_player_id: u32,
    pub sell_to_name: String,
}

impl HouseSnapshot {
    /// `models/house.rs::House::write` (the only layout the AAEmu server sends).
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            tl_id: r.u16()?,
            db_id: r.u32()?,
            obj_id: r.bc()?,
            template_id: r.u32()?,
            model_id: r.u32()?,
            co_owner_id: r.u32()?,
            owner_id: r.u32()?,
            owner_name: r.string()?,
            account_id: r.u32()?,
            permission: r.u8()?,
            all_actions: r.i32()?,
            current_actions: r.i32()?,
            tax: r.i32()?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
            name: r.string()?,
            allow_recover: r.bool()?,
            sell_price: r.i32()?,
            sell_to_player_id: r.u32()?,
            sell_to_name: r.string()?,
        })
    }

    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

/// `SCHouseTaxInfoPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct HouseTaxInfo {
    pub tl_id: u16,
    pub dominion_tax_rate: i32,
    pub money_amount: i32,
    pub money_amount2: i32,
    /// Unix seconds (`PacketStream.Write(DateTime)`).
    pub due: i64,
    pub is_already_paid: bool,
    pub weeks_without_pay: i32,
    pub is_heavy_tax_house: bool,
}

/// `SCConstructHouseTaxPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstructHouseTax {
    pub design_id: u32,
    pub heavy_tax_house_count: i32,
    pub normal_tax_house_count: i32,
    pub is_heavy_tax_house: bool,
    pub base_tax_money_amount: i32,
    pub deposit_tax_money_amount: i32,
    pub total_tax_money_amount: i32,
}

/// `SCHouseSetForSalePacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct HouseSale {
    pub tl_id: u16,
    pub money_amount: u32,
    pub sell_to_player_id: u32,
    pub sell_to_name: String,
    pub house_name: String,
}

/// `SCHouseFarmPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct HouseFarm {
    pub name: String,
    pub total: i32,
    pub harvestable: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HousingEvent {
    /// `SCCharBrief`: a character identity lookup (for example a prospective house buyer).
    /// It carries no object ID or house timeline and is never treated as a unit spawn.
    CharacterBrief { character_id: u32, name: String, race: u8 },
    /// House doors/windows from SCUnitModelPostureChanged, not a fabricated snapshot.
    UnitPosture { obj_id: u32, posture: crate::unit_msg::HousePosture },
    State(HouseSnapshot),
    /// `SCMyHousePacket`: the full body of one of the account's houses.
    MyHouse(HouseSnapshot),
    /// `SCLoginCharInfoHousePacket`: the character-list house, prefixed by its owner id.
    LoginCharInfoHouse { owner_id: u32, house: HouseSnapshot },
    BuildProgress {
        tl_id: u16,
        model_id: u32,
        all_actions: i32,
        current_actions: i32,
    },
    /// `SCHouseBuildPayChangedPacket`.
    BuildPay { tl_id: u16, money_amount: i32 },
    PermissionChanged {
        tl_id: u16,
        permission: u8,
    },
    Demolished {
        tl_id: u16,
    },
    MyHouseRemoved {
        tl_id: u16,
    },
    Farm(HouseFarm),
    TaxInfo(HouseTaxInfo),
    ConstructHouseTax(ConstructHouseTax),
    RecoverToggle {
        tl_id: u16,
        allow_recover: bool,
    },
    SetForSale(HouseSale),
    ResetForSale {
        tl_id: u16,
        house_name: String,
    },
    Sold {
        tl_id: u16,
        previous_owner_id: u32,
        new_owner_id: u32,
        new_owner_account_id: u32,
        owner_name: String,
        house_name: String,
    },
    OwnerNameChanged {
        tl_id: u16,
        name: String,
    },
}

impl HousingEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCCharBriefPacket
                | sc::SCHouseStatePacket
                | sc::SCHouseBuildProgressPacket
                | sc::SCHouseBuildPayChangedPacket
                | sc::SCHousePermissionChangedPacket
                | sc::SCHouseDemolishedPacket
                | sc::SCMyHousePacket
                | sc::SCMyHouseRemovedPacket
                | sc::SCHouseFarmPacket
                | sc::SCHouseTaxInfoPacket
                | sc::SCConstructHouseTaxPacket
                | sc::SCHousingRecoverTogglePacket
                | sc::SCHouseSetForSalePacket
                | sc::SCHouseResetForSalePacket
                | sc::SCHouseSoldPacket
                | sc::SCHouseOwnerNameChangedPacket
                | sc::SCLoginCharInfoHousePacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::CharacterBrief { .. } => "SCCharBrief",
            Self::UnitPosture { .. } => "SCUnitModelPostureChanged(House)",
            Self::State(_) => "SCHouseState",
            Self::MyHouse(_) => "SCMyHouse",
            Self::LoginCharInfoHouse { .. } => "SCLoginCharInfoHouse",
            Self::BuildProgress { .. } => "SCHouseBuildProgress",
            Self::BuildPay { .. } => "SCHouseBuildPayChanged",
            Self::PermissionChanged { .. } => "SCHousePermissionChanged",
            Self::Demolished { .. } => "SCHouseDemolished",
            Self::MyHouseRemoved { .. } => "SCMyHouseRemoved",
            Self::Farm(_) => "SCHouseFarm",
            Self::TaxInfo(_) => "SCHouseTaxInfo",
            Self::ConstructHouseTax(_) => "SCConstructHouseTax",
            Self::RecoverToggle { .. } => "SCHousingRecoverToggle",
            Self::SetForSale(_) => "SCHouseSetForSale",
            Self::ResetForSale { .. } => "SCHouseResetForSale",
            Self::Sold { .. } => "SCHouseSold",
            Self::OwnerNameChanged { .. } => "SCHouseOwnerNameChanged",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCCharBriefPacket => Self::CharacterBrief { character_id: r.u32()?, name: r.string()?, race: r.u8()? },
            sc::SCHouseStatePacket => Self::State(HouseSnapshot::read(r)?),
            sc::SCMyHousePacket => Self::MyHouse(HouseSnapshot::read(r)?),
            sc::SCLoginCharInfoHousePacket => Self::LoginCharInfoHouse {
                owner_id: r.u32()?,
                house: HouseSnapshot::read(r)?,
            },
            sc::SCHouseBuildProgressPacket => Self::BuildProgress {
                tl_id: r.u16()?,
                model_id: r.u32()?,
                all_actions: r.i32()?,
                current_actions: r.i32()?,
            },
            sc::SCHouseBuildPayChangedPacket => Self::BuildPay {
                tl_id: r.u16()?,
                money_amount: r.i32()?,
            },
            sc::SCHousePermissionChangedPacket => Self::PermissionChanged {
                tl_id: r.u16()?,
                permission: r.u8()?,
            },
            sc::SCHouseDemolishedPacket => Self::Demolished { tl_id: r.u16()? },
            sc::SCMyHouseRemovedPacket => Self::MyHouseRemoved { tl_id: r.u16()? },
            sc::SCHouseFarmPacket => Self::Farm(HouseFarm {
                name: r.string()?,
                total: r.i32()?,
                harvestable: r.i32()?,
            }),
            sc::SCHouseTaxInfoPacket => Self::TaxInfo(HouseTaxInfo {
                tl_id: r.u16()?,
                dominion_tax_rate: r.i32()?,
                money_amount: r.i32()?,
                money_amount2: r.i32()?,
                due: r.i64()?,
                is_already_paid: r.bool()?,
                weeks_without_pay: r.i32()?,
                is_heavy_tax_house: r.bool()?,
            }),
            sc::SCConstructHouseTaxPacket => Self::ConstructHouseTax(ConstructHouseTax {
                design_id: r.u32()?,
                heavy_tax_house_count: r.i32()?,
                normal_tax_house_count: r.i32()?,
                is_heavy_tax_house: r.bool()?,
                base_tax_money_amount: r.i32()?,
                deposit_tax_money_amount: r.i32()?,
                total_tax_money_amount: r.i32()?,
            }),
            sc::SCHousingRecoverTogglePacket => Self::RecoverToggle {
                tl_id: r.u16()?,
                allow_recover: r.bool()?,
            },
            sc::SCHouseSetForSalePacket => Self::SetForSale(HouseSale {
                tl_id: r.u16()?,
                money_amount: r.u32()?,
                sell_to_player_id: r.u32()?,
                sell_to_name: r.string()?,
                house_name: r.string()?,
            }),
            sc::SCHouseResetForSalePacket => Self::ResetForSale {
                tl_id: r.u16()?,
                house_name: r.string()?,
            },
            sc::SCHouseSoldPacket => Self::Sold {
                tl_id: r.u16()?,
                previous_owner_id: r.u32()?,
                new_owner_id: r.u32()?,
                new_owner_account_id: r.u32()?,
                owner_name: r.string()?,
                house_name: r.string()?,
            },
            sc::SCHouseOwnerNameChangedPacket => Self::OwnerNameChanged {
                tl_id: r.u16()?,
                name: r.string()?,
            },
            _ => return Err(WireError::Invalid(format!("housing opcode {opcode:#x}"))),
        })
    }
}

/// `CS*House*` requests. Each variant is the exact body the server's reader expects.
#[derive(Clone, Debug, PartialEq)]
pub enum HouseCommand {
    ConstructHouseTax { design_id: u32, x: f32, y: f32, z: f32 },
    CreateHouse {
        design_id: u32,
        x: f32,
        y: f32,
        z: f32,
        z_rot: f32,
        item_id: u64,
        money_amount: i32,
        ht: i32,
        auto_use_aa_point: bool,
    },
    DecorateHouse {
        house_tl: u16,
        design_id: u32,
        x: f32,
        y: f32,
        z: f32,
        quat: [f32; 4],
        parent_obj_id: u32,
        item_id: u64,
    },
    ChangeName { tl: u16, name: String },
    ChangePermission { tl: u16, permission: u8 },
    RequestTax { tl: u16 },
    AllowRecover { tl: u16 },
    Sell { tl: u16, money_amount: u32, sell_to: String },
    CancelSell { tl: u16 },
    Buy { tl: u16, money_amount: u32 },
}

impl HouseCommand {
    pub fn opcode(&self) -> u16 {
        match self {
            Self::ConstructHouseTax { .. } => cs::CSConstructHouseTaxPacket,
            Self::CreateHouse { .. } => cs::CSCreateHousePacket,
            Self::DecorateHouse { .. } => cs::CSDecorateHousePacket,
            Self::ChangeName { .. } => cs::CSChangeHouseNamePacket,
            Self::ChangePermission { .. } => cs::CSChangeHousePermissionPacket,
            Self::RequestTax { .. } => cs::CSRequestHouseTaxPacket,
            Self::AllowRecover { .. } => cs::CSAllowHousingRecoverPacket,
            Self::Sell { .. } => cs::CSSellHousePacket,
            Self::CancelSell { .. } => cs::CSSellHouseCancelPacket,
            Self::Buy { .. } => cs::CSBuyHousePacket,
        }
    }

    pub fn name(&self) -> &'static str {
        cs::name(self.opcode())
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::ConstructHouseTax { design_id, x, y, z } => {
                w.u32(*design_id)
                    .i64(coord_to_long(*x))
                    .i64(coord_to_long(*y))
                    .f32(*z);
            }
            Self::CreateHouse {
                design_id,
                x,
                y,
                z,
                z_rot,
                item_id,
                money_amount,
                ht,
                auto_use_aa_point,
            } => {
                w.u32(*design_id)
                    .i64(coord_to_long(*x))
                    .i64(coord_to_long(*y))
                    .f32(*z)
                    .f32(*z_rot)
                    .u64(*item_id)
                    .i32(*money_amount)
                    .i32(*ht)
                    .bool(*auto_use_aa_point);
            }
            Self::DecorateHouse {
                house_tl,
                design_id,
                x,
                y,
                z,
                quat,
                parent_obj_id,
                item_id,
            } => {
                w.u16(*house_tl).u32(*design_id).f32(*x).f32(*y).f32(*z);
                for v in quat {
                    w.f32(*v);
                }
                w.bc(*parent_obj_id).u64(*item_id);
            }
            Self::ChangeName { tl, name } => {
                w.u16(*tl).string(name);
            }
            Self::ChangePermission { tl, permission } => {
                w.u16(*tl).u8(*permission);
            }
            Self::RequestTax { tl } | Self::AllowRecover { tl } | Self::CancelSell { tl } => {
                w.u16(*tl);
            }
            Self::Sell { tl, money_amount, sell_to } => {
                w.u16(*tl).u32(*money_amount).string(sell_to);
            }
            Self::Buy { tl, money_amount } => {
                w.u16(*tl).u32(*money_amount);
            }
        }
    }
}

/// Received attachment identity, before static decoration lookup or housing access policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HousingAttachment {
    pub house_obj_id: u32,
    pub house_tl_id: u16,
    pub doodad_obj_id: u32,
    pub item_template_id: u32,
}

/// Join only an explicit doodad parent with an existing received housing unit.
/// No ownership/proximity inference, static decoration classification, or cached ordering.
pub fn housing_attachments<'a>(doodads: &'a crate::state::Doodads, units: &'a crate::state::Units) -> impl Iterator<Item = HousingAttachment> + 'a {
    doodads.by_id.values().filter_map(move |doodad| {
        let parent = units.by_id.get(&doodad.parent_obj_id)?;
        let crate::model::UnitKind::Housing { tl_id, .. } = parent.kind else {
            return None;
        };
        Some(HousingAttachment {
            house_obj_id: parent.obj_id,
            house_tl_id: tl_id,
            doodad_obj_id: doodad.obj_id,
            item_template_id: doodad.item_template_id,
        })
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct House {
    pub snapshot: HouseSnapshot,
    /// A received demolition notice, not a locally guessed disappearance.
    pub demolished: bool,
    pub build_pay: Option<i32>,
    pub tax: Option<HouseTaxInfo>,
    pub sale: Option<HouseSale>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Houses {
    pub by_tl_id: HashMap<u16, House>,
    /// Character ID to received (name, race), for house sale identity lookups.
    pub character_briefs: HashMap<u32, (String, u8)>,
    /// Last `SCConstructHouseTax` quote; it does not authorize a build.
    pub construct_quote: Option<ConstructHouseTax>,
    /// Last `SCHouseFarm` receipt. The packet carries no timeline, so it is not
    /// attributed to a specific house.
    pub last_farm: Option<HouseFarm>,
}

fn replace<T: PartialEq>(slot: &mut Option<T>, value: T) -> bool {
    if slot.as_ref() == Some(&value) {
        return false;
    }
    *slot = Some(value);
    true
}

impl Houses {
    fn upsert(&mut self, snapshot: &HouseSnapshot) -> bool {
        if self
            .by_tl_id
            .get(&snapshot.tl_id)
            .is_some_and(|h| h.snapshot == *snapshot && !h.demolished)
        {
            return false;
        }
        // A reused runtime object ID must not keep a stale timeline entry.
        self.by_tl_id
            .retain(|tl, h| *tl == snapshot.tl_id || h.snapshot.obj_id != snapshot.obj_id);
        if let Some(house) = self.by_tl_id.get_mut(&snapshot.tl_id) {
            house.snapshot = snapshot.clone();
            house.demolished = false;
            // a full snapshot that is no longer for sale ends the sale it replaced
            if house.snapshot.sell_price == 0 {
                house.sale = None;
            }
        } else {
            self.by_tl_id.insert(
                snapshot.tl_id,
                House {
                    snapshot: snapshot.clone(),
                    demolished: false,
                    build_pay: None,
                    tax: None,
                    sale: None,
                },
            );
        }
        true
    }

    fn mark_demolished(&mut self, tl_id: u16) -> bool {
        let Some(house) = self.by_tl_id.get_mut(&tl_id) else {
            return false;
        };
        if house.demolished {
            return false;
        }
        house.demolished = true;
        true
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Housing(HousingEvent::CharacterBrief { character_id, name, race }) => {
                let brief = (name.clone(), *race);
                if self.character_briefs.get(character_id) == Some(&brief) {
                    return false;
                }
                self.character_briefs.insert(*character_id, brief);
                true
            }
            Event::Housing(HousingEvent::State(snapshot) | HousingEvent::MyHouse(snapshot)) => {
                self.upsert(snapshot)
            }
            Event::Housing(HousingEvent::LoginCharInfoHouse { house, .. }) => self.upsert(house),
            Event::Housing(HousingEvent::BuildProgress {
                tl_id,
                model_id,
                all_actions,
                current_actions,
            }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                let state = &mut house.snapshot;
                if (state.model_id, state.all_actions, state.current_actions) == (*model_id, *all_actions, *current_actions) {
                    return false;
                }
                state.model_id = *model_id;
                state.all_actions = *all_actions;
                state.current_actions = *current_actions;
                true
            }
            Event::Housing(HousingEvent::BuildPay { tl_id, money_amount }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                replace(&mut house.build_pay, *money_amount)
            }
            Event::Housing(HousingEvent::PermissionChanged { tl_id, permission }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                if house.snapshot.permission == *permission {
                    return false;
                }
                house.snapshot.permission = *permission;
                true
            }
            Event::Housing(HousingEvent::Demolished { tl_id } | HousingEvent::MyHouseRemoved { tl_id }) => {
                self.mark_demolished(*tl_id)
            }
            Event::Housing(HousingEvent::Farm(farm)) => replace(&mut self.last_farm, farm.clone()),
            Event::Housing(HousingEvent::TaxInfo(info)) => {
                let Some(house) = self.by_tl_id.get_mut(&info.tl_id) else {
                    return false;
                };
                replace(&mut house.tax, info.clone())
            }
            Event::Housing(HousingEvent::ConstructHouseTax(quote)) => {
                replace(&mut self.construct_quote, quote.clone())
            }
            Event::Housing(HousingEvent::RecoverToggle { tl_id, allow_recover }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                if house.snapshot.allow_recover == *allow_recover {
                    return false;
                }
                house.snapshot.allow_recover = *allow_recover;
                true
            }
            Event::Housing(HousingEvent::SetForSale(sale)) => {
                let Some(house) = self.by_tl_id.get_mut(&sale.tl_id) else {
                    return false;
                };
                let price = sale.money_amount as i32;
                if house.sale.as_ref() == Some(sale)
                    && house.snapshot.sell_price == price
                    && house.snapshot.sell_to_player_id == sale.sell_to_player_id
                    && house.snapshot.sell_to_name == sale.sell_to_name
                {
                    return false;
                }
                house.snapshot.sell_price = price;
                house.snapshot.sell_to_player_id = sale.sell_to_player_id;
                house.snapshot.sell_to_name = sale.sell_to_name.clone();
                house.sale = Some(sale.clone());
                true
            }
            Event::Housing(HousingEvent::ResetForSale { tl_id, .. }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                if house.sale.is_none()
                    && house.snapshot.sell_price == 0
                    && house.snapshot.sell_to_player_id == 0
                    && house.snapshot.sell_to_name.is_empty()
                {
                    return false;
                }
                house.sale = None;
                house.snapshot.sell_price = 0;
                house.snapshot.sell_to_player_id = 0;
                house.snapshot.sell_to_name.clear();
                true
            }
            Event::Housing(HousingEvent::Sold {
                tl_id,
                new_owner_id,
                new_owner_account_id,
                owner_name,
                house_name,
                ..
            }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                let changed = house.snapshot.owner_id != *new_owner_id
                    || house.snapshot.account_id != *new_owner_account_id
                    || house.snapshot.owner_name != *owner_name
                    || house.snapshot.name != *house_name
                    || house.sale.is_some()
                    || house.snapshot.sell_price != 0
                    || house.snapshot.sell_to_player_id != 0
                    || !house.snapshot.sell_to_name.is_empty();
                if !changed {
                    return false;
                }
                house.snapshot.owner_id = *new_owner_id;
                house.snapshot.account_id = *new_owner_account_id;
                house.snapshot.owner_name = owner_name.clone();
                house.snapshot.name = house_name.clone();
                house.sale = None;
                house.snapshot.sell_price = 0;
                house.snapshot.sell_to_player_id = 0;
                house.snapshot.sell_to_name.clear();
                true
            }
            Event::Housing(HousingEvent::OwnerNameChanged { tl_id, name }) => {
                let Some(house) = self.by_tl_id.get_mut(tl_id) else {
                    return false;
                };
                if house.snapshot.owner_name == *name {
                    return false;
                }
                house.snapshot.owner_name = name.clone();
                true
            }
            Event::UnitsRemoved(ids) => {
                let before = self.by_tl_id.len();
                self.by_tl_id.retain(|_, h| !ids.contains(&h.snapshot.obj_id));
                self.by_tl_id.len() != before
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let changed = !self.by_tl_id.is_empty()
                    || !self.character_briefs.is_empty()
                    || self.construct_quote.is_some()
                    || self.last_farm.is_some();
                self.by_tl_id.clear();
                self.character_briefs.clear();
                self.construct_quote = None;
                self.last_farm = None;
                changed
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::game::ServerMsg;

    fn hex(raw: &str) -> Vec<u8> {
        raw.split_whitespace()
            .flat_map(|word| (0..word.len()).step_by(2).map(move |i| u8::from_str_radix(&word[i..i + 2], 16).unwrap()))
            .collect()
    }

    // `models/house.rs::House::write` fixture (one fixed u32 model, not PISC).
    fn native_body() -> Vec<u8> {
        hex(
            "0700 0d0c0b0a 030201 2c010000 10000000 06000000 05000000 0300 4f776e 09000000 02 28000000 07000000 fa000000 0000000000100000 0000000000200000 00006040 0300 487574 01 e8030000 08000000 0000",
        )
    }

    pub(crate) fn snapshot() -> HouseSnapshot {
        HouseSnapshot::read(&mut Reader::new(&native_body())).unwrap()
    }

    fn with_tail(opcode: u16, body: &[u8], expected: HousingEvent) {
        let mut plain = Writer::new();
        plain.raw(&[0xdd, 1, 0, 1]).u16(opcode).raw(body);
        assert_eq!(ServerMsg::decode(&plain.finish()).unwrap(), ServerMsg::Housing(expected.clone()));
        let mut inner = Writer::new();
        inner.u16(0).u16(opcode).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut compressed = Writer::new();
        compressed.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&inner);
        assert_eq!(
            ServerMsg::decode(&compressed.finish()).unwrap(),
            ServerMsg::Batch(vec![ServerMsg::Housing(expected), ServerMsg::Snow(true)])
        );
    }

    fn command_body(command: &HouseCommand) -> Vec<u8> {
        let mut w = Writer::new();
        command.write(&mut w);
        w.finish()
    }

    #[test]
    fn house_snapshot_matches_the_server_native_writer() {
        let body = native_body();
        let house = snapshot();
        assert_eq!(
            (house.tl_id, house.db_id, house.obj_id, house.template_id, house.model_id),
            (7, 0x0a0b0c0d, 0x010203, 300, 16)
        );
        assert_eq!((house.co_owner_id, house.owner_id, house.account_id, house.permission), (6, 5, 9, 2));
        assert_eq!((&*house.owner_name, &*house.name, &*house.sell_to_name), ("Own", "Hut", ""));
        assert_eq!((house.all_actions, house.current_actions, house.tax), (40, 7, 250));
        assert_eq!(house.position(), Pos::new(1.0, 2.0, 3.5));
        assert!(house.allow_recover);
        assert_eq!((house.sell_price, house.sell_to_player_id), (1000, 8));
        with_tail(sc::SCHouseStatePacket, &body, HousingEvent::State(house.clone()));
        with_tail(sc::SCMyHousePacket, &body, HousingEvent::MyHouse(house.clone()));
        let mut reader = Reader::new(&body);
        assert_eq!(HouseSnapshot::read(&mut reader).unwrap(), house);
        assert!(reader.is_empty());
        for cut in 0..body.len() {
            assert!(HouseSnapshot::read(&mut Reader::new(&body[..cut])).is_err(), "cut {cut}");
        }
        // Strings are ordinary i16-length blobs; no client-side 128-byte cap.
        let long = "x".repeat(200);
        let mut writer = Writer::new();
        writer.u16(7).u32(1).bc(2).u32(3).u32(4).u32(5).u32(6).string(&long).u32(8).u8(0)
            .i32(0).i32(0).i32(0).i64(0).i64(0).f32(0.0).string(&long).bool(false).i32(0).u32(0).string("");
        let bytes = writer.finish();
        let mut reader = Reader::new(&bytes);
        assert_eq!(HouseSnapshot::read(&mut reader).unwrap().owner_name, long);
        assert!(reader.is_empty());
    }

    #[test]
    fn house_control_packets_decode_byte_exactly() {
        let cases = [
            (
                sc::SCHouseBuildProgressPacket,
                hex("0700 ffffffff 00000080 ffffff7f"),
                HousingEvent::BuildProgress { tl_id: 7, model_id: u32::MAX, all_actions: i32::MIN, current_actions: i32::MAX },
            ),
            (sc::SCHouseBuildPayChangedPacket, hex("0700 fbffffff"), HousingEvent::BuildPay { tl_id: 7, money_amount: -5 }),
            (sc::SCHousePermissionChangedPacket, hex("0700 fe"), HousingEvent::PermissionChanged { tl_id: 7, permission: 254 }),
            (sc::SCHouseDemolishedPacket, hex("0700"), HousingEvent::Demolished { tl_id: 7 }),
            (sc::SCMyHouseRemovedPacket, hex("0700"), HousingEvent::MyHouseRemoved { tl_id: 7 }),
            (
                sc::SCHouseFarmPacket,
                hex("0200 6162 03000000 01000000"),
                HousingEvent::Farm(HouseFarm { name: "ab".into(), total: 3, harvestable: 1 }),
            ),
            (
                sc::SCHouseTaxInfoPacket,
                hex("0500 00000000 400d0300 a0860100 00105e5f00000000 01 ffffffff 00"),
                HousingEvent::TaxInfo(HouseTaxInfo {
                    tl_id: 5,
                    dominion_tax_rate: 0,
                    money_amount: 200000,
                    money_amount2: 100000,
                    due: 0x5f5e1000,
                    is_already_paid: true,
                    weeks_without_pay: -1,
                    is_heavy_tax_house: false,
                }),
            ),
            (
                sc::SCConstructHouseTaxPacket,
                hex("44332211 02000000 01000000 01 a0860100 400d0300 e0930400"),
                HousingEvent::ConstructHouseTax(ConstructHouseTax {
                    design_id: 0x11223344,
                    heavy_tax_house_count: 2,
                    normal_tax_house_count: 1,
                    is_heavy_tax_house: true,
                    base_tax_money_amount: 100000,
                    deposit_tax_money_amount: 200000,
                    total_tax_money_amount: 300000,
                }),
            ),
            (sc::SCHousingRecoverTogglePacket, hex("0500 01"), HousingEvent::RecoverToggle { tl_id: 5, allow_recover: true }),
            (
                sc::SCHouseSetForSalePacket,
                hex("0500 e8030000 09000000 0200 426f 0100 48"),
                HousingEvent::SetForSale(HouseSale {
                    tl_id: 5,
                    money_amount: 1000,
                    sell_to_player_id: 9,
                    sell_to_name: "Bo".into(),
                    house_name: "H".into(),
                }),
            ),
            (
                sc::SCHouseResetForSalePacket,
                hex("0500 0100 48"),
                HousingEvent::ResetForSale { tl_id: 5, house_name: "H".into() },
            ),
            (
                sc::SCHouseSoldPacket,
                hex("0500 01000000 02000000 03000000 0100 42 0100 48"),
                HousingEvent::Sold {
                    tl_id: 5,
                    previous_owner_id: 1,
                    new_owner_id: 2,
                    new_owner_account_id: 3,
                    owner_name: "B".into(),
                    house_name: "H".into(),
                },
            ),
            (
                sc::SCHouseOwnerNameChangedPacket,
                hex("0201 0100 4e"),
                HousingEvent::OwnerNameChanged { tl_id: 0x0102, name: "N".into() },
            ),
        ];
        for (opcode, raw, event) in cases {
            with_tail(opcode, &raw, event.clone());
            assert!(HousingEvent::handles(opcode));
            let mut reader = Reader::new(&raw);
            assert_eq!(HousingEvent::read(opcode, &mut reader).unwrap(), event);
            assert!(reader.is_empty());
            for cut in 0..raw.len() {
                assert!(HousingEvent::read(opcode, &mut Reader::new(&raw[..cut])).is_err(), "{opcode:#x} cut {cut}");
            }
        }
        let mut login = 35u32.to_le_bytes().to_vec();
        login.extend(native_body());
        let expected = HousingEvent::LoginCharInfoHouse { owner_id: 35, house: snapshot() };
        with_tail(sc::SCLoginCharInfoHousePacket, &login, expected);
        for cut in 0..login.len() {
            assert!(HousingEvent::read(sc::SCLoginCharInfoHousePacket, &mut Reader::new(&login[..cut])).is_err());
        }
        assert!(HousingEvent::read(0xffff, &mut Reader::new(&[])).is_err());
    }

    #[test]
    fn house_commands_match_the_server_readers() {
        let construct = HouseCommand::ConstructHouseTax {
            design_id: 300,
            x: 1000.5,
            y: 2000.25,
            z: 55.5,
        };
        assert_eq!(
            command_body(&construct),
            hex("2c010000 0000000000883e00 0000000000047d00 00005e42")
        );
        assert_eq!(construct.opcode(), 0x056);
        assert_eq!(construct.name(), "CSConstructHouseTaxPacket");

        let create = HouseCommand::CreateHouse {
            design_id: 300,
            x: 10.0,
            y: 20.0,
            z: 30.0,
            z_rot: 1.5,
            item_id: 0xAABBCCDD,
            money_amount: 500,
            ht: 7,
            auto_use_aa_point: true,
        };
        assert_eq!(
            command_body(&create),
            hex("2c010000 0000000000a00000 0000000000400100 0000f041 0000c03f ddccbbaa00000000 f4010000 07000000 01")
        );
        assert_eq!(create.opcode(), 0x057);

        let decorate = HouseCommand::DecorateHouse {
            house_tl: 7,
            design_id: 50,
            x: 1.0,
            y: 2.0,
            z: 3.0,
            quat: [0.0, 0.0, 0.5, 0.75],
            parent_obj_id: 0x123456,
            item_id: 99,
        };
        assert_eq!(
            command_body(&decorate),
            hex("0700 32000000 0000803f 00000040 00004040 00000000 00000000 0000003f 0000403f 563412 6300000000000000")
        );
        assert_eq!(decorate.opcode(), 0x058);

        assert_eq!(
            command_body(&HouseCommand::ChangeName { tl: 0x0102, name: "villa".into() }),
            hex("0201 0500 76696c6c61")
        );
        assert_eq!(command_body(&HouseCommand::ChangePermission { tl: 3, permission: 1 }), hex("0300 01"));
        assert_eq!(command_body(&HouseCommand::RequestTax { tl: 9 }), hex("0900"));
        assert_eq!(command_body(&HouseCommand::AllowRecover { tl: 9 }), hex("0900"));
        assert_eq!(
            command_body(&HouseCommand::Sell { tl: 3, money_amount: 1000, sell_to: "Bob".into() }),
            hex("0300 e8030000 0300 426f62")
        );
        assert_eq!(command_body(&HouseCommand::CancelSell { tl: 9 }), hex("0900"));
        assert_eq!(command_body(&HouseCommand::Buy { tl: 3, money_amount: 77 }), hex("0300 4d000000"));
        for (command, opcode) in [
            (HouseCommand::ChangeName { tl: 3, name: "villa".into() }, 0x059),
            (HouseCommand::ChangePermission { tl: 3, permission: 1 }, 0x05a),
            (HouseCommand::RequestTax { tl: 3 }, 0x05c),
            (HouseCommand::AllowRecover { tl: 3 }, 0x05d),
            (HouseCommand::Sell { tl: 3, money_amount: 1, sell_to: String::new() }, 0x05e),
            (HouseCommand::CancelSell { tl: 3 }, 0x05f),
            (HouseCommand::Buy { tl: 3, money_amount: 1 }, 0x060),
        ] {
            assert_eq!(command.opcode(), opcode);
            assert_eq!(crate::game::ClientMsg::House(command).opcode(), opcode);
        }
    }

    #[test]
    fn housing_received_state_deltas_removal_replacement_and_lifecycle() {
        let mut world = crate::state::WorldState::default();
        let house = snapshot();
        let state = Event::Housing(HousingEvent::State(house.clone()));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::Demolished { tl_id: 7 })));
        world.apply(&state);
        assert!(world.units.by_id.is_empty());
        assert!(!world.houses.apply(&state));

        let delta = Event::Housing(HousingEvent::BuildProgress {
            tl_id: 7,
            model_id: 55,
            all_actions: 80,
            current_actions: 60,
        });
        assert!(world.houses.apply(&delta));
        assert!(!world.houses.apply(&delta));
        assert_eq!(world.houses.by_tl_id[&7].snapshot.current_actions, 60);

        assert!(world.houses.apply(&Event::Housing(HousingEvent::BuildPay { tl_id: 7, money_amount: -5 })));
        assert_eq!(world.houses.by_tl_id[&7].build_pay, Some(-5));

        assert!(world.houses.apply(&Event::Housing(HousingEvent::PermissionChanged { tl_id: 7, permission: 255 })));
        assert_eq!(world.houses.by_tl_id[&7].snapshot.permission, 255);

        let farm = HouseFarm { name: "Farm".into(), total: 4, harvestable: 2 };
        assert!(world.houses.apply(&Event::Housing(HousingEvent::Farm(farm.clone()))));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::Farm(farm.clone()))));
        assert_eq!(world.houses.last_farm.as_ref(), Some(&farm));

        let tax = HouseTaxInfo {
            tl_id: 7,
            dominion_tax_rate: 0,
            money_amount: 200000,
            money_amount2: 100000,
            due: 0x5f5e1000,
            is_already_paid: true,
            weeks_without_pay: -1,
            is_heavy_tax_house: false,
        };
        assert!(world.houses.apply(&Event::Housing(HousingEvent::TaxInfo(tax.clone()))));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::TaxInfo(tax.clone()))));
        assert_eq!(world.houses.by_tl_id[&7].tax.as_ref(), Some(&tax));

        let quote = ConstructHouseTax {
            design_id: 300,
            heavy_tax_house_count: 2,
            normal_tax_house_count: 1,
            is_heavy_tax_house: true,
            base_tax_money_amount: 100000,
            deposit_tax_money_amount: 200000,
            total_tax_money_amount: 300000,
        };
        assert!(world.houses.apply(&Event::Housing(HousingEvent::ConstructHouseTax(quote.clone()))));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::ConstructHouseTax(quote.clone()))));
        assert_eq!(world.houses.construct_quote.as_ref(), Some(&quote));

        let sale = HouseSale {
            tl_id: 7,
            money_amount: 1000,
            sell_to_player_id: 8,
            sell_to_name: "Bo".into(),
            house_name: "Hut".into(),
        };
        assert!(world.houses.apply(&Event::Housing(HousingEvent::SetForSale(sale.clone()))));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::SetForSale(sale.clone()))));
        assert_eq!(world.houses.by_tl_id[&7].snapshot.sell_price, 1000);
        assert_eq!(world.houses.by_tl_id[&7].snapshot.sell_to_player_id, 8);
        assert!(world.houses.apply(&Event::Housing(HousingEvent::ResetForSale { tl_id: 7, house_name: "Hut".into() })));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::ResetForSale { tl_id: 7, house_name: "Hut".into() })));
        assert!(world.houses.by_tl_id[&7].sale.is_none());
        assert_eq!(world.houses.by_tl_id[&7].snapshot.sell_price, 0);
        world.houses.apply(&Event::Housing(HousingEvent::SetForSale(sale)));
        let sold = HousingEvent::Sold {
            tl_id: 7,
            previous_owner_id: 5,
            new_owner_id: 9,
            new_owner_account_id: 10,
            owner_name: "New".into(),
            house_name: "Villa".into(),
        };
        assert!(world.houses.apply(&Event::Housing(sold.clone())));
        assert!(!world.houses.apply(&Event::Housing(sold)));
        let renamed = &world.houses.by_tl_id[&7];
        assert_eq!((renamed.snapshot.owner_id, renamed.snapshot.account_id), (9, 10));
        assert_eq!((&*renamed.snapshot.owner_name, &*renamed.snapshot.name), ("New", "Villa"));
        assert!(renamed.sale.is_none());
        assert_eq!(renamed.snapshot.sell_price, 0);

        assert!(world.houses.apply(&Event::Housing(HousingEvent::OwnerNameChanged { tl_id: 7, name: "Other".into() })));
        assert!(!world.houses.apply(&Event::Housing(HousingEvent::OwnerNameChanged { tl_id: 7, name: "Other".into() })));
        assert_eq!(world.houses.by_tl_id[&7].snapshot.owner_name, "Other");
        assert!(world.houses.apply(&Event::Housing(HousingEvent::RecoverToggle { tl_id: 7, allow_recover: false })));
        assert!(!world.houses.by_tl_id[&7].snapshot.allow_recover);

        let mut other = snapshot();
        other.tl_id = 8;
        other.obj_id = 0x010204;
        assert!(world.houses.apply(&Event::Housing(HousingEvent::MyHouse(other.clone()))));
        assert_eq!(world.houses.by_tl_id.len(), 2);
        let mut login = snapshot();
        login.tl_id = 11;
        login.obj_id = 0x010205;
        assert!(world.houses.apply(&Event::Housing(HousingEvent::LoginCharInfoHouse { owner_id: 5, house: login })));
        assert_eq!(world.houses.by_tl_id.len(), 3);

        let demolished = Event::Housing(HousingEvent::MyHouseRemoved { tl_id: 8 });
        assert!(world.houses.apply(&demolished));
        assert!(!world.houses.apply(&demolished));
        assert!(world.houses.by_tl_id[&8].demolished, "removal notice does not erase the snapshot");

        assert!(!world.houses.apply(&Event::UnitsRemoved(vec![999])));
        assert!(world.houses.apply(&Event::UnitsRemoved(vec![other.obj_id])));
        assert!(!world.houses.by_tl_id.contains_key(&8));

        let mut moved = snapshot();
        moved.tl_id = 12;
        moved.obj_id = house.obj_id;
        assert!(world.houses.apply(&Event::Housing(HousingEvent::State(moved.clone()))));
        assert!(!world.houses.by_tl_id.contains_key(&7), "reused object ID drops the old timeline");
        assert_eq!(world.houses.by_tl_id[&12].snapshot.obj_id, house.obj_id);

        for reset in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
        ] {
            world.apply(&state);
            world.houses.apply(&Event::Housing(HousingEvent::ConstructHouseTax(quote.clone())));
            assert!(world.houses.apply(&reset));
            assert!(!world.houses.apply(&reset));
            assert!(world.houses.construct_quote.is_none());
        }
    }

    #[test]
    fn housing_attachment_identity_is_received_parent_kind_and_timeline_not_owner() {
        use crate::model::{Doodad, UnitKind};
        // models/doodad.rs::write field sequence: owner obj != parent obj and
        // persistent owner DB id != housing timeline. Neither is a membership key.
        let raw = hex(
            "030201 2a000000 999999 563412 ff 000000000000000000 0000 0000 0000 0000803f 00 00000000 44332211 0000000000000000 88776655 00000000 00000000 0000000000000000 00000000 00000000 ffffffff 03 efcdab89 00000000",
        );
        let mut reader = Reader::new(&raw);
        let doodad = Doodad::read(&mut reader).unwrap();
        assert!(reader.is_empty());
        assert_eq!(
            (
                doodad.obj_id,
                doodad.owner_obj_id,
                doodad.parent_obj_id,
                doodad.item_template_id,
                doodad.owner_db_id
            ),
            (0x010203, 0x999999, 0x123456, 0x55667788, 0x89abcdef)
        );
        for cut in 0..raw.len() {
            assert!(Doodad::read(&mut Reader::new(&raw[..cut])).is_err());
        }
        let mut world = crate::state::WorldState::default();
        world.apply(&Event::DoodadsSpawned(vec![doodad.clone()]));
        assert_eq!(housing_attachments(&world.doodads, &world.units).count(), 0);
        let mut parent = crate::state::tests::unit(doodad.parent_obj_id);
        world.apply(&Event::UnitSpawned(Box::new(parent.clone())));
        assert_eq!(
            housing_attachments(&world.doodads, &world.units).count(),
            0,
            "ordinary character parent is not housing"
        );
        parent.kind = UnitKind::Housing {
            tl_id: 321,
            template_id: 99,
            build_step: -1,
        };
        world.apply(&Event::UnitSpawned(Box::new(parent.clone())));
        let expected = HousingAttachment {
            house_obj_id: 0x123456,
            house_tl_id: 321,
            doodad_obj_id: 0x010203,
            item_template_id: 0x55667788,
        };
        assert_eq!(housing_attachments(&world.doodads, &world.units).collect::<Vec<_>>(), [expected]);
        // Same owner without the explicit attachment never qualifies.
        let mut unattached = doodad.clone();
        unattached.obj_id = 4;
        unattached.parent_obj_id = 0;
        world.apply(&Event::DoodadsSpawned(vec![unattached]));
        assert_eq!(housing_attachments(&world.doodads, &world.units).count(), 1);
        world.apply(&Event::DoodadsRemoved(vec![doodad.obj_id]));
        assert_eq!(housing_attachments(&world.doodads, &world.units).count(), 0);
        world.apply(&Event::DoodadsSpawned(vec![doodad.clone()]));
        world.apply(&Event::UnitsRemoved(vec![parent.obj_id]));
        assert_eq!(housing_attachments(&world.doodads, &world.units).count(), 0);
        // Reverse packet order and unknown/zero item IDs remain raw identities.
        world.apply(&Event::UnitSpawned(Box::new(parent)));
        let mut zero = doodad;
        zero.item_template_id = 0;
        world.apply(&Event::DoodadsSpawned(vec![zero]));
        assert_eq!(housing_attachments(&world.doodads, &world.units).next().unwrap().item_template_id, 0);
        for reset in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected { message: String::new() },
        ] {
            let mut parent = crate::state::tests::unit(0x123456);
            parent.kind = UnitKind::Housing {
                tl_id: 321,
                template_id: 99,
                build_step: -1,
            };
            world.apply(&Event::UnitSpawned(Box::new(parent)));
            let mut reader = Reader::new(&raw);
            world.apply(&Event::DoodadsSpawned(vec![Doodad::read(&mut reader).unwrap()]));
            assert_eq!(housing_attachments(&world.doodads, &world.units).count(), 1);
            world.apply(&reset);
            assert!(world.units.by_id.is_empty());
            assert!(world.doodads.by_id.is_empty());
            assert_eq!(housing_attachments(&world.doodads, &world.units).count(), 0);
        }
    }

    fn house_unit_body() -> Vec<u8> {
        // Lane world/unit_state_tests.rs::house_unit_state_bytes source fixture.
        let mut w = Writer::new();
        w.raw(&hex("300000 0400 486f6d65 03 0400 2c010000 fdff 0000"));
        w.position(Pos::new(10.0, 20.0, 30.0)).raw(&hex("0000c03f 05 4d000000"));
        w.raw(&[0; 112]).u8(0); // 28 empty full-item equipment slots; model params none.
        w.raw(&hex("000000 10270000 88130000 ff ff 01 00 0101010101010101 00 00 00000000 000040 00 0000000000 000d000000 000000 0020 000000"));
        w.finish()
    }

    #[test]
    fn complete_housing_unit_tail_keeps_house_snapshot_and_following_dd04_aligned() {
        use crate::model::{Unit, UnitKind};
        let body = house_unit_body();
        let mut reader = Reader::new(&body);
        let unit = Unit::read(&mut reader).unwrap();
        assert!(reader.is_empty(), "house unit tail must consume complete posture/rotation/faction/buff suffix");
        assert_eq!(unit.kind, UnitKind::Housing { tl_id: 4, template_id: 300, build_step: -3 });
        assert_eq!((unit.hp, unit.mp, unit.faction_id, unit.flags), (100, 50, 13, 0x2000));
        assert_eq!(unit.rotation, [0.0, 0.0, 64.0 * (std::f32::consts::TAU / 127.0)]);
        assert_eq!(
            unit.house_posture,
            Some(crate::unit_msg::HousePosture { has_loot_left: false, doors: [1; 2], windows: [1; 6] })
        );
        assert!(!unit.dead);
        let snapshot_body = native_body();
        let mut inner = Writer::new();
        inner
            .u16(0)
            .u16(sc::SCUnitStatePacket)
            .raw(&body)
            .u16(0)
            .u16(sc::SCHouseStatePacket)
            .raw(&snapshot_body)
            .u16(0)
            .u16(sc::SCOnOffSnowPacket)
            .bool(true);
        let raw = inner.finish();
        let len = raw.len() as u16;
        let mut frame = Writer::new();
        frame.raw(&[0xdd, 4]).u16(3).u8(1).u16(len).u16(!len).raw(&raw);
        assert_eq!(
            ServerMsg::decode(&frame.finish()).unwrap(),
            ServerMsg::Batch(vec![ServerMsg::UnitState(Box::new(unit)), ServerMsg::Housing(HousingEvent::State(snapshot())), ServerMsg::Snow(true)])
        );
        for cut in 0..body.len() {
            assert!(Unit::read(&mut Reader::new(&body[..cut])).is_err(), "house unit cut {cut}");
        }
        // Strict housing parse must never accept an unknown tail length as a complete unit.
        let tail_start = body.len() - 51;
        let mut bad = body.clone();
        bad[tail_start + 11] = 1;
        assert!(Unit::read(&mut Reader::new(&bad)).is_err());
    }

    #[test]
    fn house_posture_preserves_raw_bytes_without_fabricating_unknown_units() {
        use crate::unit_msg::{HousePosture, Posture};
        let raw = hex("01 01 00 01 02 ff 01 00 80 01");
        let mut reader = Reader::new(&raw);
        let posture = Posture::read(&mut reader).unwrap();
        assert!(reader.is_empty());
        let house = HousePosture { has_loot_left: true, doors: [0, 1], windows: [2, 255, 1, 0, 128, 1] };
        assert_eq!(posture.house, Some(house));
        for cut in 0..raw.len() {
            assert!(Posture::read(&mut Reader::new(&raw[..cut])).is_err());
        }
        let event = Event::Housing(HousingEvent::UnitPosture { obj_id: 33, posture: house });
        let mut state = crate::state::WorldState::default();
        assert!(!state.units.apply(&event));
        state.apply(&Event::UnitSpawned(Box::new(crate::state::tests::unit(33))));
        assert!(state.units.apply(&event));
        assert!(!state.units.apply(&event));
        assert_eq!(state.units.by_id[&33].house_posture, Some(house));
        state.apply(&Event::UnitPosture { obj_id: 33, kind: 0, anim_action: 0, activate: false });
        assert_eq!(state.units.by_id[&33].house_posture, None);
    }
}
