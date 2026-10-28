//! Item containers, item change tasks and the action bar, as the server sends them.
//!
//! * `SCCharacterInvenContents`: the bag / bank in chunks of 10 slots (`Item.Write` each).
//! * `SCItemTaskSuccess`: every later change (loot, use, move, money ...) as a list of tasks.
//! * `SCActionSlots`: the 85 hotbar slots.
//!
//! Layouts follow the server's `ItemTask::write`, `SCCharacterInvenContentsPacket` and
//! `write_action_slots`.

use crate::model::{Item, ItemDetail};
use crate::wire::{Reader, Result, WireError};

/// Original SC0A0; the active lane names this six-byte receipt `SCUnknownPacket`.
/// A nonzero error is presented before two ordered pending-key releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemOperationError {
    pub container: u8,
    pub slot: u8,
    pub second_container: u8,
    pub second_slot: u8,
    pub error_code: i16,
}

impl ItemOperationError {
    pub fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { container:r.u8()?,slot:r.u8()?,second_container:r.u8()?,second_slot:r.u8()?,error_code:r.i16()? })
    }

    /// Native397d2200 false-branch admission, in callback order. Equal keys are
    /// deliberately repeated. This releases no contents and is not a success ACK.
    pub fn pending_releases(&self) -> impl Iterator<Item=(u8,u8)> {
        [(self.container,self.slot),(self.second_container,self.second_slot)].into_iter()
            .filter(move |&(kind,slot)|self.error_code!=0 && kind!=4 && (kind!=0 || slot!=0))
    }
}

/// Original SC093: a variable tagged detail prefix, not a padded128 wire field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemDetailUpdate {
    pub item_id: u64,
    pub container: u8,
    pub slot: u8,
    /// Includes tag. Original native reader consumes only tag for unknown kinds.
    pub raw_detail: Vec<u8>,
}

impl ItemDetailUpdate {
    fn payload_size(kind: u8) -> usize {
        match kind { 1=>55, 2=>29, 3=>6, 4=>9, 5=>24, 6|7=>16, 8=>8, 9=>4, _=>0 }
    }

    pub fn read(r: &mut Reader) -> Result<Self> {
        let item_id=r.u64()?;
        let container=r.u8()?;
        let slot=r.u8()?;
        let kind=r.u8()?;
        let mut raw_detail=Vec::with_capacity(1+Self::payload_size(kind));
        raw_detail.push(kind);
        raw_detail.extend_from_slice(r.bytes(Self::payload_size(kind))?);
        Ok(Self{item_id,container,slot,raw_detail})
    }

    /// Constructor-derived native union. Only the prefix was received; the original
    /// SC093 factory explicitly zeros the entire128 before reading it.
    pub fn native_detail_block(&self) -> Option<[u8;128]> {
        let kind=*self.raw_detail.first()?;
        if self.raw_detail.len()!=1+Self::payload_size(kind) {return None;}
        let mut block=[0;128];
        block[..self.raw_detail.len()].copy_from_slice(&self.raw_detail);
        Some(block)
    }

    pub fn detail(&self) -> Option<ItemDetail> {
        self.native_detail_block()?;
        let kind=self.raw_detail[0];
        if !matches!(kind,0|1|3) {return Some(ItemDetail::Other(kind));}
        let mut reader=Reader::new(&self.raw_detail[1..]);
        let detail=ItemDetail::read(&mut reader,kind).ok()?;
        reader.is_empty().then_some(detail)
    }
}

/// `SCSyncItemLifespan` metadata for an exact item identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemLifespan {
    pub added: bool,
    pub item_id: u64,
    pub template_id: u32,
    /// Lane login sends an expiry endpoint; equipment unpack can send the charge start instead.
    /// Keep the original signed seconds without inferring a countdown or removing an item.
    pub raw_time: i64,
}

impl ItemLifespan {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            added: r.bool()?,
            item_id: r.u64()?,
            template_id: r.u32()?,
            raw_time: r.i64()?,
        })
    }
}

/// `SlotType`: which container an item is in.
pub mod container {
    pub const NONE: u8 = 0;
    pub const EQUIPMENT: u8 = 1;
    pub const INVENTORY: u8 = 2;
    pub const BANK: u8 = 3;
    pub const TRADE: u8 = 4;
    pub const MAIL: u8 = 5;
    pub const AUCTION: u8 = 6;
    pub const EQUIPMENT_MATE: u8 = 252;
    pub const SYSTEM: u8 = 255;
}

/// Slots on the hotbar (`Character.MaxActionSlots`).
pub const ACTION_SLOTS: usize = 85;
/// Action bar pages (`FUN_393f6910` refuses page >= 4).
pub const ACTION_PAGES: u8 = 4;
/// Slots the main action bar shows per page (`new_actionbar_view.alb` creates 12).
pub const ACTION_PAGE_SLOTS: usize = 12;

/// What a hotbar slot holds (`ActionSlotType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActionKind {
    #[default]
    None,
    /// Any item of a template (`id` = `items.id`): the slot shows the count in the bag.
    ItemType,
    /// A skill (`id` = `skills.id`).
    Spell,
    Macro,
    /// One specific item (`id` = the item's database id).
    ItemId,
    RidePetSpell,
    Unknown(u8),
}

impl ActionKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::ItemType,
            2 => Self::Spell,
            3 => Self::Macro,
            4 => Self::ItemId,
            5 => Self::RidePetSpell,
            o => Self::Unknown(o),
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ItemType => 1,
            Self::Spell => 2,
            Self::Macro => 3,
            Self::ItemId => 4,
            Self::RidePetSpell => 5,
            Self::Unknown(o) => o,
        }
    }
}

/// One hotbar slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionSlot {
    pub kind: ActionKind,
    pub id: u64,
}

impl ActionSlot {
    pub fn is_empty(&self) -> bool {
        self.kind == ActionKind::None
    }

    /// The skill id of a `Spell` / `RidePetSpell` slot.
    pub fn skill_id(&self) -> Option<u32> {
        matches!(self.kind, ActionKind::Spell | ActionKind::RidePetSpell).then_some(self.id as u32)
    }

    /// The item template id of an `ItemType` slot.
    pub fn item_template_id(&self) -> Option<u32> {
        (self.kind == ActionKind::ItemType).then_some(self.id as u32)
    }

    /// The database id of an `ItemId` slot.
    pub fn item_id(&self) -> Option<u64> {
        (self.kind == ActionKind::ItemId).then_some(self.id)
    }

    /// Body of `SCActionSlots` / `CSUpdateActionSlot` without the slot index: kind byte and id.
    pub fn read(r: &mut Reader) -> Result<Self> {
        let kind = ActionKind::from_u8(r.u8()?);
        let id = match kind {
            ActionKind::ItemType | ActionKind::Spell | ActionKind::RidePetSpell => u64::from(r.u32()?),
            ActionKind::ItemId => r.u64()?,
            // The server writes no id for these (and logs an error).
            ActionKind::None | ActionKind::Macro | ActionKind::Unknown(_) => 0,
        };
        Ok(Self { kind, id })
    }

    pub fn write(&self, w: &mut crate::wire::Writer) {
        w.u8(self.kind.as_u8());
        match self.kind {
            ActionKind::ItemType | ActionKind::Spell | ActionKind::RidePetSpell => {
                w.u32(self.id as u32);
            }
            ActionKind::ItemId => {
                w.u64(self.id);
            }
            ActionKind::None | ActionKind::Macro | ActionKind::Unknown(_) => {}
        }
    }
}

/// Body of `SCActionSlots`: the slots back to back until the packet ends.
pub fn read_action_slots(r: &mut Reader) -> Result<Vec<ActionSlot>> {
    let mut slots = Vec::with_capacity(ACTION_SLOTS);
    while !r.is_empty() {
        slots.push(ActionSlot::read(r)?);
    }
    Ok(slots)
}

/// The block `CSSwapItems` and `CSSplitBagItem` start with: the item in one slot and what is in the other.
///
/// Both slots are named by container and index, with the ids of the items in them (0 for an empty one). Moving to an
/// empty slot, swapping, merging stacks and wearing or taking off gear are all the same packet:
///
/// * bag to bag: `from` the bag item, `to` the bag item there (or 0 with the slot);
/// * wear an item: `from` the **empty** equipment slot (item 0, container [`container::EQUIPMENT`]) `to` the bag item; when
///   something is worn there already, `from` the worn item and `to` the bag item swaps them;
/// * take off: `from` the worn item, `to` the bag slot (item 0 when it is empty).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemMove {
    pub from_item_id: u64,
    pub to_item_id: u64,
    pub from_container: u8,
    pub from_slot: u8,
    pub to_container: u8,
    pub to_slot: u8,
}

impl ItemMove {
    pub fn write(&self, w: &mut crate::wire::Writer) {
        w.u64(self.from_item_id)
            .u64(self.to_item_id)
            .u8(self.from_container)
            .u8(self.from_slot)
            .u8(self.to_container)
            .u8(self.to_slot);
    }
}

/// One chunk of `SCCharacterInvenContents`: up to 10 consecutive slots of a container.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InvenChunk {
    pub container: u8,
    pub chunks: u8,
    /// Index of the first chunk this packet carries: slot = `(start_chunk + n) * 10 + i`.
    pub start_chunk: u8,
    pub items: Vec<Option<Item>>,
    /// An item in the chunk could not be decoded; `items` holds the ones before it.
    pub truncated: bool,
}

impl InvenChunk {
    pub const SLOTS_PER_CHUNK: usize = 10;

    pub fn read(r: &mut Reader) -> Result<Self> {
        let container = r.u8()?;
        let chunks = r.u8()?;
        let start_chunk = r.u8()?;
        let mut items = Vec::new();
        let mut truncated = false;
        while items.len() < usize::from(chunks) * Self::SLOTS_PER_CHUNK {
            match Item::read(r) {
                Ok(item) => items.push(item),
                Err(e) => {
                    tracing::warn!("inventory chunk of container {container}: {e}");
                    truncated = true;
                    break;
                }
            }
        }
        Ok(Self { container, chunks, start_chunk, items, truncated })
    }

    /// Slot index of the first item of this chunk.
    pub fn first_slot(&self) -> usize {
        usize::from(self.start_chunk) * Self::SLOTS_PER_CHUNK
    }
}

/// `ItemAction`: the first byte of a task.
pub mod action {
    pub const CHANGE_MONEY_AMOUNT: u8 = 1;
    pub const CHANGE_BANK_MONEY_AMOUNT: u8 = 2;
    pub const CHANGE_GAME_POINT: u8 = 3;
    pub const ADD_STACK: u8 = 4;
    pub const CREATE: u8 = 5;
    pub const TAKE: u8 = 6;
    pub const REMOVE: u8 = 7;
    pub const SWAP_SLOT: u8 = 8;
    pub const UPDATE_DETAIL: u8 = 9;
    pub const SET_FLAGS_BITS: u8 = 10;
    pub const UPDATE_FLAGS: u8 = 11;
    pub const REMOVE_CRAFTING: u8 = 12;
    pub const SEIZE: u8 = 13;
    pub const CHANGE_GRADE: u8 = 14;
    pub const CHANGE_OWNER: u8 = 15;
    pub const CHANGE_AA_POINT: u8 = 16;
    pub const CHANGE_BANK_AA_POINT: u8 = 17;
    pub const CHANGE_AUTO_USE_AA_POINT: u8 = 18;
}

/// One change of the item containers or the purse (`ItemTask`).
#[derive(Clone, Debug, PartialEq)]
pub enum ItemTask {
    /// Copper added (negative: spent) in the bag.
    Money(i32),
    BankMoney(i32),
    /// Honor (kind 0) or vocation (kind 1) points added.
    GamePoint { kind: u8, amount: i32 },
    /// AA cash points.
    AaPoint(i32),
    BankAaPoint(i32),
    AutoUseAaPoint(u8),
    /// An item entered `container` at `slot` (`action` = create / change owner / take).
    Add { action: u8, container: u8, slot: u8, item: Item, raw_detail_block: Option<Vec<u8>> },
    /// `delta` more (or fewer) in a stack.
    Count { container: u8, slot: u8, item_id: u64, delta: i32, template_id: u32 },
    Grade { container: u8, slot: u8, item_id: u64, grade: u8 },
    /// Two slots swapped (`to_item_id` 0 when the target was empty); the containers may differ.
    Move {
        from_container: u8,
        from_slot: u8,
        from_item_id: u64,
        to_container: u8,
        to_slot: u8,
        to_item_id: u64,
    },
    Remove { container: u8, slot: u8, item_id: u64, template_id: u32 },
    /// `ItemRemoveCrafting`: an item removed by id only.
    RemoveCrafting { item_id: u64 },
    /// `ItemRemoveSlot` (seized).
    RemoveSlot { container: u8, slot: u8, item_id: u64 },
    /// New details (durability, gems ...) of an item.
    Update {
        container: u8, slot: u8, item_id: u64, detail: ItemDetail, detail_payload: Option<Vec<u8>>,
        /// Entire consumed declared padded block, including tag/padding. None for synthetics.
        /// Legacy tolerant decoding is unchanged; presence does not prove native-safe length.
        raw_detail_block: Option<Vec<u8>>,
    },
    /// New flag bits (`ItemFlag`: secure, soulbound ...).
    Flags { container: u8, slot: u8, item_id: u64, bits: u8 },
    /// Action11 replaces flags and both timestamps. All native bool bytes remain raw.
    Security { container:u8,slot:u8,item_id:u64,bits:u8,is_unsecure_excess:u8,is_unsecure_set:u8,is_unpack:u8,unsecure_time:i64,unpack_time:i64 },
}

/// Body of `SCItemTaskSuccess`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemTasks {
    /// `ItemTaskType`: why the items changed (loot, quest reward, buy ...).
    pub reason: u8,
    pub tasks: Vec<ItemTask>,
    /// Item ids the server removed from the client's cache without a slot.
    pub force_remove: Vec<u64>,
    /// A task of an unknown kind ended the list early.
    pub truncated: bool,
}

impl ItemTasks {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let reason = r.u8()?;
        let n = r.u8()?;
        let mut out = Self { reason, ..Self::default() };
        for _ in 0..n {
            match ItemTask::read(r) {
                Ok(t) => out.tasks.push(t),
                Err(e) => {
                    tracing::warn!("item task (reason {reason}): {e}");
                    out.truncated = true;
                    return Ok(out);
                }
            }
        }
        let n = r.u8()?;
        for _ in 0..n {
            out.force_remove.push(r.u64()?);
        }
        Ok(out)
    }
}

impl ItemTask {
    fn read(r: &mut Reader) -> Result<Self> {
        let act = r.u8()?;
        Ok(match act {
            action::CHANGE_MONEY_AMOUNT => Self::Money(r.i32()?),
            action::CHANGE_BANK_MONEY_AMOUNT => Self::BankMoney(r.i32()?),
            action::CHANGE_GAME_POINT => Self::GamePoint { kind: r.u8()?, amount: r.i32()? },
            action::CHANGE_AA_POINT => Self::AaPoint(r.i32()?),
            action::CHANGE_BANK_AA_POINT => Self::BankAaPoint(r.i32()?),
            action::CHANGE_AUTO_USE_AA_POINT => Self::AutoUseAaPoint(r.u8()?),
            action::CREATE | action::CHANGE_OWNER | action::TAKE => {
                let container = r.u8()?;
                let slot = r.u8()?;
                let (item, raw_detail_block) = read_task_item_with_raw(r)?;
                Self::Add { action: act, container, slot, item, raw_detail_block: Some(raw_detail_block) }
            }
            action::ADD_STACK => Self::Count {
                container: r.u8()?,
                slot: r.u8()?,
                item_id: r.u64()?,
                delta: r.i32()?,
                template_id: r.u32()?,
            },
            action::CHANGE_GRADE => Self::Grade {
                container: r.u8()?,
                slot: r.u8()?,
                item_id: r.u64()?,
                grade: r.u8()?,
            },
            action::SWAP_SLOT => {
                let from_container = r.u8()?;
                let from_slot = r.u8()?;
                let to_container = r.u8()?;
                let to_slot = r.u8()?;
                let from_item_id = r.u64()?;
                let to_item_id = r.u64()?;
                r.skip(4)?; // flags
                Self::Move { from_container, from_slot, from_item_id, to_container, to_slot, to_item_id }
            }
            action::REMOVE => Self::Remove {
                container: r.u8()?,
                slot: r.u8()?,
                item_id: r.u64()?,
                template_id: r.u32()?,
            },
            action::REMOVE_CRAFTING => Self::RemoveCrafting { item_id: r.u64()? },
            action::SEIZE => Self::RemoveSlot { container: r.u8()?, slot: r.u8()?, item_id: r.u64()? },
            action::UPDATE_DETAIL => {
                let container = r.u8()?;
                let slot = r.u8()?;
                let item_id = r.u64()?;
                let (detail, detail_payload, raw_detail_block) = read_padded_detail_block(r)?;
                Self::Update { container, slot, item_id, detail, detail_payload, raw_detail_block: Some(raw_detail_block.to_vec()) }
            }
            action::SET_FLAGS_BITS => {
                let t = Self::Flags { container: r.u8()?, slot: r.u8()?, item_id: r.u64()?, bits: r.u8()? };
                r.skip(8)?;
                t
            }
            action::UPDATE_FLAGS => Self::Security {
                container:r.u8()?,slot:r.u8()?,item_id:r.u64()?,bits:r.u8()?,
                is_unsecure_excess:r.u8()?,is_unsecure_set:r.u8()?,is_unpack:r.u8()?,unsecure_time:r.i64()?,unpack_time:r.i64()?,
            },
            other => return Err(WireError::Invalid(format!("item task action {other}"))),
        })
    }
}

/// `i16` length (128) followed by the detail type, the details and zero padding.
#[cfg(test)]
fn read_padded_detail(r: &mut Reader) -> Result<(ItemDetail, Option<Vec<u8>>)> {
    let (detail, payload, _) = read_padded_detail_block(r)?;
    Ok((detail, payload))
}

fn read_padded_detail_block<'a>(r: &mut Reader<'a>) -> Result<(ItemDetail, Option<Vec<u8>>, &'a [u8])> {
    let len = r.i16()?.max(0) as usize;
    let block = r.bytes(len)?;
    if block.is_empty() {
        return Ok((ItemDetail::None, None, block));
    }
    let mut d = Reader::new(block);
    let detail_type = d.u8()?;
    match ItemDetail::read(&mut d, detail_type) {
        Ok(detail) => Ok((detail, Some(block[1..d.pos()].to_vec()), block)),
        // Existing tolerant padded-block behavior: retain the tag, but do not claim a partial
        // or unknown-size block is an exact payload. The outer block still preserves alignment.
        Err(_) => Ok((ItemDetail::Other(detail_type), None, block)),
    }
}

/// An item as `write_task_item_details` writes it: like `Item.Write`, but the detail block is
/// padded to a fixed length.
#[cfg(test)]
fn read_task_item(r: &mut Reader) -> Result<Item> {
    read_task_item_with_raw(r).map(|(item, _)| item)
}

fn read_task_item_with_raw(r: &mut Reader) -> Result<(Item, Vec<u8>)> {
    let template_id = r.u32()?;
    let id = r.u64()?;
    let grade = r.u8()?;
    let flags = r.u8()?;
    let count = r.i32()?;
    let (detail, detail_payload, raw_detail_block) = read_padded_detail_block(r)?;
    let raw_detail_block = raw_detail_block.to_vec();
    Ok((Item {
        template_id,
        id,
        grade,
        flags,
        count,
        detail,
        detail_payload,
        create_time: r.i64()?,
        lifespan_mins: r.i32()?,
        made_unit_id: r.u32()?,
        world_id: r.u8()?,
        unsecure_time: r.i64()?,
        unpack_time: r.i64()?,
    }, raw_detail_block))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;

    #[test]
    fn lifespan_fixed_body_retains_both_flags_exact_ids_and_signed_time() {
        // Independent LE golden for item_packets.rs's bool/u64/u32/i64 writer.
        let raw = [
            1, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x98, 0xba, 0xdc, 0xfe, 0xf9, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ];
        let mut r = Reader::new(&raw);
        assert_eq!(
            ItemLifespan::read(&mut r).unwrap(),
            ItemLifespan {
                added: true,
                item_id: 0x1122334455667788,
                template_id: 0xfedcba98,
                raw_time: -7,
            }
        );
        assert!(r.is_empty());
        for n in 0..raw.len() {
            assert!(ItemLifespan::read(&mut Reader::new(&raw[..n])).is_err());
            let mut frame = Writer::new();
            frame.raw(&[0xdd, 1, 0, 0]).u16(0x098).raw(&raw[..n]);
            assert!(crate::game::ServerMsg::decode(&frame.finish()).is_err());
        }
        for added in [false, true] {
            for raw_time in [i64::MIN, -1, 0, i64::MAX] {
                let mut w = Writer::new();
                w.bool(added).u64(u64::MAX).u32(u32::MAX).i64(raw_time);
                let raw = w.finish();
                let mut r = Reader::new(&raw);
                assert_eq!(
                    ItemLifespan::read(&mut r).unwrap(),
                    ItemLifespan {
                        added,
                        item_id: u64::MAX,
                        template_id: u32::MAX,
                        raw_time,
                    }
                );
                assert!(r.is_empty());
            }
        }
    }

    #[test]
    fn lifespan_notifications_preserve_following_dd04_inventory_and_world_entries() {
        use crate::game::ServerMsg;
        let mut raw = Writer::new();
        for added in [true, false] {
            raw.u16(0).u16(0x098).bool(added).u64(7).u32(8).i64(9);
        }
        raw.u16(0)
            .u16(crate::opcodes::sc::SCInvenExpandedPacket)
            .u8(2)
            .u8(60);
        raw.u16(0)
            .u16(crate::opcodes::sc::SCOnOffSnowPacket)
            .bool(true);
        let raw = raw.finish();
        let len = raw.len() as u16;
        let mut outer = Writer::new();
        outer
            .raw(&[0xdd, 4])
            .u16(4)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&raw);
        let expected = vec![
            ServerMsg::ItemLifespan(ItemLifespan {
                added: true,
                item_id: 7,
                template_id: 8,
                raw_time: 9,
            }),
            ServerMsg::ItemLifespan(ItemLifespan {
                added: false,
                item_id: 7,
                template_id: 8,
                raw_time: 9,
            }),
            ServerMsg::InvenExpanded {
                container: 2,
                slots: 60,
            },
            ServerMsg::Snow(true),
        ];
        assert_eq!(expected[0].name(), "SCSyncItemLifespan");
        assert_eq!(
            ServerMsg::decode(&outer.finish()).unwrap(),
            ServerMsg::Batch(expected)
        );
    }

    fn plain_item(w: &mut Writer, template: u32, id: u64, count: i32) {
        w.u32(template).u64(id).u8(1).u8(0).i32(count).u8(0); // detail type 0
        w.i64(10).i32(0).u32(0).u8(1).i64(0).i64(0);
    }

    #[test]
    fn action_slots_golden_bytes() {
        // Spell 1234, nothing, an item type, one specific item, a macro (no id), then nothing.
        let mut w = Writer::new();
        w.u8(2).u32(1234);
        w.u8(0);
        w.u8(1).u32(8001);
        w.u8(4).u64(0x0102_0304_0506);
        w.u8(3);
        w.u8(0);
        let bytes = w.finish();
        let slots = read_action_slots(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(slots.len(), 6);
        assert_eq!(slots[0].skill_id(), Some(1234));
        assert!(slots[1].is_empty());
        assert_eq!(slots[2].item_template_id(), Some(8001));
        assert_eq!(slots[3].item_id(), Some(0x0102_0304_0506));
        assert_eq!(slots[4].kind, ActionKind::Macro);
        // Writing them again gives the same bytes (what CSUpdateActionSlot sends per slot).
        let mut again = Writer::new();
        for s in &slots {
            s.write(&mut again);
        }
        assert_eq!(again.finish(), bytes);
    }

    #[test]
    fn inventory_chunk_with_items() {
        // `SCCharacterInvenContents`: type 2 (bag), 1 chunk, starts at chunk 3, ten slots with
        // a potion stack in the third and a bare 0 in the others.
        let mut w = Writer::new();
        w.u8(container::INVENTORY).u8(1).u8(3);
        for i in 0..10 {
            if i == 2 {
                plain_item(&mut w, 8001, 555, 20);
            } else {
                w.i32(0);
            }
        }
        let bytes = w.finish();
        let c = InvenChunk::read(&mut Reader::new(&bytes)).unwrap();
        assert!(!c.truncated);
        assert_eq!((c.container, c.chunks, c.start_chunk, c.first_slot()), (2, 1, 3, 30));
        assert_eq!(c.items.len(), 10);
        let it = c.items[2].as_ref().unwrap();
        assert_eq!((it.template_id, it.id, it.count), (8001, 555, 20));
        assert!(c.items[0].is_none());
        // A chunk cut inside an item keeps the slots before it.
        let cut = &bytes[..bytes.len() - 30];
        let c = InvenChunk::read(&mut Reader::new(cut)).unwrap();
        assert!(c.truncated);
    }

    /// A task item as `write_task_item_details` does: details padded to 128 bytes.
    fn task_item(w: &mut Writer, template: u32, id: u64, count: i32) {
        w.u32(template).u64(id).u8(0).u8(0).i32(count);
        w.i16(128).raw(&[0u8; 128]); // detail type 0 + padding
        w.i64(10).i32(0).u32(0).u8(1).i64(0).i64(0);
    }

    #[test]
    fn padded_add_and_update_keep_only_the_exact_detail_payload() {
        let payload: Vec<u8> = (1..=55).collect(); // Nonzero equipment reserved bytes included.
        let mut block = vec![0xee; 128];
        block[0] = 1;
        block[1..56].copy_from_slice(&payload);
        let mut w = Writer::new();
        w.u32(123).u64(456).u8(2).u8(3).i32(4).i16(128).raw(&block);
        w.i64(10).i32(11).u32(12).u8(13).i64(14).i64(15);
        let item = read_task_item(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!(item.detail_payload, Some(payload.clone()));
        let mut w = Writer::new();
        w.u8(action::UPDATE_DETAIL).u8(2).u8(3).u64(456).i16(128).raw(&block).u32(99);
        let raw = w.finish();
        let mut r = Reader::new(&raw);
        let ItemTask::Update { detail_payload, .. } = ItemTask::read(&mut r).unwrap() else { panic!() };
        assert_eq!(detail_payload, Some(payload));
        assert_eq!(r.u32().unwrap(), 99);
        // The established tolerant fixed-block decoder does not claim malformed data is exact.
        assert_eq!(read_padded_detail(&mut Reader::new(&[2, 0, 1, 5])).unwrap(), (ItemDetail::Other(1), None));
    }

    #[test]
    fn item_task_success_golden_bytes() {
        // Reason 9 (loot): +50 copper, a new item in bag slot 4, a stack grows by 3, force remove 7.
        let mut w = Writer::new();
        w.u8(9).u8(3);
        w.u8(action::CHANGE_MONEY_AMOUNT).i32(50);
        w.u8(action::CREATE).u8(container::INVENTORY).u8(4);
        task_item(&mut w, 8001, 600, 5);
        w.u8(action::ADD_STACK).u8(container::INVENTORY).u8(1).u64(555).i32(3).u32(8001);
        w.u8(1).u64(7);
        w.u32(0); // type(id), trailing
        let bytes = w.finish();
        let t = ItemTasks::read(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(t.reason, 9);
        assert!(!t.truncated);
        assert_eq!(t.tasks.len(), 3);
        assert_eq!(t.tasks[0], ItemTask::Money(50));
        let ItemTask::Add { container: c, slot, item, .. } = &t.tasks[1] else { panic!("{:?}", t.tasks[1]) };
        assert_eq!((*c, *slot, item.template_id, item.id, item.count), (2, 4, 8001, 600, 5));
        assert_eq!(
            t.tasks[2],
            ItemTask::Count { container: 2, slot: 1, item_id: 555, delta: 3, template_id: 8001 }
        );
        assert_eq!(t.force_remove, vec![7]);
    }

    #[test]
    fn item_task_move_remove_and_unknown() {
        let mut w = Writer::new();
        w.u8(5).u8(2);
        w.u8(action::SWAP_SLOT).u8(2).u8(1).u8(2).u8(6).u64(555).u64(0).i32(0);
        w.u8(action::REMOVE).u8(2).u8(6).u64(555).u32(8001);
        w.u8(0);
        let t = ItemTasks::read(&mut Reader::new(&w.finish())).unwrap();
        assert_eq!(
            t.tasks[0],
            ItemTask::Move {
                from_container: 2,
                from_slot: 1,
                from_item_id: 555,
                to_container: 2,
                to_slot: 6,
                to_item_id: 0
            }
        );
        assert_eq!(t.tasks[1], ItemTask::Remove { container: 2, slot: 6, item_id: 555, template_id: 8001 });

        // An action this client does not know ends the list but keeps what came before.
        let mut w = Writer::new();
        w.u8(5).u8(2);
        w.u8(action::CHANGE_MONEY_AMOUNT).i32(-1);
        w.u8(99);
        let t = ItemTasks::read(&mut Reader::new(&w.finish())).unwrap();
        assert!(t.truncated);
        assert_eq!(t.tasks, vec![ItemTask::Money(-1)]);
    }

    #[test]
    fn update_detail_literal_retains_whole_native_block_and_following_task() {
        // Native397cfd40 + lane ItemTask::write: 9,container,slot,u64ID,i16length.
        let mut wire = vec![9, 2, 3, 0xc8, 1, 0, 0, 0, 0, 0, 0, 128, 0];
        let mut block = vec![0xa5; 128];
        block[..7].copy_from_slice(&[3, 0xff, 0xff, 0xff, 0xff, 0xf2, 0xfe]);
        wire.extend_from_slice(&block);
        let body_len = wire.len();
        wire.extend_from_slice(&[1, 4, 3, 2, 1]); // following Money task
        let mut reader = Reader::new(&wire);
        let task = ItemTask::read(&mut reader).unwrap();
        assert!(matches!(task, ItemTask::Update {
            container: 2, slot: 3, item_id: 456,
            detail: ItemDetail::Mate { exp: -1, dead: 0xf2, level: 0xfe },
            ref detail_payload, ref raw_detail_block,
        } if detail_payload.as_deref()==Some(&block[1..7]) && raw_detail_block.as_deref()==Some(block.as_slice())));
        assert_eq!(ItemTask::read(&mut reader).unwrap(), ItemTask::Money(0x01020304));
        assert!(reader.is_empty());
        for end in 0..body_len {
            assert!(ItemTask::read(&mut Reader::new(&wire[..end])).is_err(), "prefix {end}");
        }
    }

    #[test]
    fn update_detail_raw_retention_does_not_restrict_legacy_tolerant_lengths() {
        for block in [vec![], vec![1, 7, 8, 9], vec![0xff; 128], vec![0; 129]] {
            let mut wire = vec![9, 2, 3, 0xc8, 1, 0, 0, 0, 0, 0, 0];
            wire.extend_from_slice(&(block.len() as i16).to_le_bytes());
            wire.extend_from_slice(&block);
            wire.extend_from_slice(&[1, 4, 3, 2, 1]);
            let mut reader = Reader::new(&wire);
            let ItemTask::Update { detail, detail_payload, raw_detail_block, .. } = ItemTask::read(&mut reader).unwrap() else { panic!() };
            assert_eq!(raw_detail_block.as_deref(), Some(block.as_slice()));
            if block.is_empty() { assert_eq!((detail, detail_payload), (ItemDetail::None, None)); }
            else if block==[1, 7, 8, 9] { assert_eq!((detail, detail_payload), (ItemDetail::Other(1), None)); }
            assert_eq!(ItemTask::read(&mut reader).unwrap(), ItemTask::Money(0x01020304));
            assert!(reader.is_empty());
        }
    }

    #[test]
    fn add_receipt_retains_declared_padding_and_following_task_alignment() {
        // Independent native397d0a00/lane Add5 literal body, not Item serializer reuse.
        let mut block=vec![0xa5;128];block[0]=3;block[1..5].copy_from_slice(&(-1i32).to_le_bytes());block[5]=0xf2;block[6]=0xfe;
        let mut raw=vec![5,2,3];raw.extend_from_slice(&9001u32.to_le_bytes());raw.extend_from_slice(&999u64.to_le_bytes());
        raw.extend_from_slice(&[8,4]);raw.extend_from_slice(&17i32.to_le_bytes());raw.extend_from_slice(&128i16.to_le_bytes());raw.extend_from_slice(&block);
        raw.extend_from_slice(&(-7i64).to_le_bytes());raw.extend_from_slice(&(-9i32).to_le_bytes());raw.extend_from_slice(&55u32.to_le_bytes());raw.push(7);
        raw.extend_from_slice(&66i64.to_le_bytes());raw.extend_from_slice(&77i64.to_le_bytes());
        for end in 0..raw.len() {assert!(ItemTask::read(&mut Reader::new(&raw[..end])).is_err(),"prefix {end}");}
        raw.extend_from_slice(&[1,50,0,0,0]);
        let mut reader=Reader::new(&raw);
        let ItemTask::Add {action,container,slot,item,raw_detail_block}=ItemTask::read(&mut reader).unwrap() else {panic!()};
        assert_eq!((action,container,slot),(5,2,3));assert_eq!(raw_detail_block,Some(block.clone()));
        assert_eq!(item.detail_payload.as_deref(),Some(&block[1..7]));assert_eq!(item.id,999);assert_eq!(item.template_id,9001);
        assert_eq!((item.create_time,item.lifespan_mins,item.made_unit_id,item.world_id,item.unsecure_time,item.unpack_time),(-7,-9,55,7,66,77));
        assert_eq!(ItemTask::read(&mut reader).unwrap(),ItemTask::Money(50));assert!(reader.is_empty());
    }

    #[test]
    fn detail_update_original_codec_goldens_keep_fixed_native_tag_lengths() {
        // Actual397b3840/3961edf0 read/write oracle, including fixed29 kind2.
        for (kind,total) in [(0,11),(1,66),(2,40),(3,17),(4,20),(5,35),(6,27),(7,27),(8,19),(9,15),(10,11),(127,11),(128,11),(255,11)] {
            let mut raw=vec![0x88,0x77,0x66,0x55,0x44,0x33,0x22,0x11,2,3,kind];
            raw.extend((0..total-11).map(|n|((n*17+3)&255) as u8));
            for n in 0..raw.len() {assert!(ItemDetailUpdate::read(&mut Reader::new(&raw[..n])).is_err(),"kind {kind} prefix {n}");}
            let mut reader=Reader::new(&raw);let update=ItemDetailUpdate::read(&mut reader).unwrap();assert!(reader.is_empty());
            assert_eq!(update.item_id,0x1122334455667788);assert_eq!((update.container,update.slot),(2,3));assert_eq!(update.raw_detail,&raw[10..]);
            let block=update.native_detail_block().unwrap();assert_eq!(&block[..total-10],&raw[10..]);assert!(block[total-10..].iter().all(|x|*x==0));
            assert!(update.detail().is_some());
            let mut invalid=update.clone();invalid.raw_detail.push(0);assert!(invalid.native_detail_block().is_none());assert!(invalid.detail().is_none());
        }
        let empty=ItemDetailUpdate{item_id:0,container:2,slot:0,raw_detail:vec![]};assert!(empty.native_detail_block().is_none());
    }

    #[test]
    fn item_operation_error_native_lane_signed_goldens_and_ordered_release_contract() {
        // Actual397c1ec0 read/write oracle; first row is independent lane g2c_misc literal.
        for (raw,expected) in [
            ([2,3,1,4,255,255],ItemOperationError{container:2,slot:3,second_container:1,second_slot:4,error_code:-1}),
            ([2,0,2,0,0,0],ItemOperationError{container:2,slot:0,second_container:2,second_slot:0,error_code:0}),
            ([4,255,255,128,0,128],ItemOperationError{container:4,slot:255,second_container:255,second_slot:128,error_code:i16::MIN}),
            ([0,0,3,255,255,127],ItemOperationError{container:0,slot:0,second_container:3,second_slot:255,error_code:i16::MAX}),
        ] {
            let mut reader=Reader::new(&raw);assert_eq!(ItemOperationError::read(&mut reader).unwrap(),expected);assert!(reader.is_empty());
            for n in 0..raw.len() {assert!(ItemOperationError::read(&mut Reader::new(&raw[..n])).is_err());}
        }
        let receipt=ItemOperationError{container:2,slot:3,second_container:2,second_slot:3,error_code:-1};
        assert_eq!(receipt.pending_releases().collect::<Vec<_>>(),[(2,3),(2,3)]);
        assert_eq!(ItemOperationError{error_code:0,..receipt}.pending_releases().count(),0);
        assert_eq!(ItemOperationError{container:0,slot:0,second_container:4,second_slot:255,..receipt}.pending_releases().count(),0);
        assert_eq!(ItemOperationError{container:0,slot:1,second_container:255,second_slot:0,..receipt}.pending_releases().collect::<Vec<_>>(),[(0,1),(255,0)]);
    }

    #[test]
    fn item_operation_error_dd04_preserves_repeated_receipts_and_following_packet() {
        use crate::game::ServerMsg;
        let receipt=ItemOperationError{container:2,slot:3,second_container:1,second_slot:4,error_code:-1};
        let mut raw=Writer::new();
        for _ in 0..2 {raw.u16(0).u16(0xa0).raw(&[2,3,1,4,255,255]);}
        raw.u16(0).u16(0x4d).raw(&[2,60]);
        let raw=raw.finish();let len=raw.len() as u16;let mut outer=Writer::new();outer.raw(&[0xdd,4]).u16(3).u8(1).u16(len).u16(!len).raw(&raw);
        assert_eq!(ServerMsg::decode(&outer.finish()).unwrap(),ServerMsg::Batch(vec![ServerMsg::ItemOperationError(receipt),ServerMsg::ItemOperationError(receipt),ServerMsg::InvenExpanded{container:2,slots:60}]));
        assert_eq!(ServerMsg::ItemOperationError(receipt).name(),"SCItemOperationError");
    }

}
