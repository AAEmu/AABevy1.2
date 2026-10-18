//! The fields read by native 3961f500 into its 192-byte item (constructor 3901cb10).
//! Its 128-byte tagged detail union is transported verbatim by 3961edf0.

use super::NATIVE_ITEM_BYTES;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemRecord {
    pub id: u64,
    pub item_type: u32,
    pub grade: u8,
    pub flags: u8,
    pub count: i32,
    pub made_unit_id: u32,
    pub detail: ItemDetail,
    pub create_time: i64,
    pub lifespan_mins: i32,
    pub world_id: u8,
    pub unsecure_time: i64,
    pub unpack_time: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ItemDetail {
    #[default]
    None,
    Equip(EquipDetail),
    Mate {
        exp: i32,
        dead: u8,
        level: u8,
    },
    /// An uninterpreted native detail union. All 127 bytes after its tag survive.
    /// An application whose network model discarded these bytes cannot construct this variant.
    Opaque {
        kind: u8,
        payload: [u8; 127],
    },
}

impl ItemDetail {
    /// Initialize the native union from actual wire bytes, validating its fixed payload size.
    /// Unused bytes are ctor-zeroed; uninterpreted variants retain all supplied payload bytes.
    /// `None` means an unknown tag or a truncated/extra payload, never guessed missing data.
    pub fn from_wire(kind: u8, payload: &[u8]) -> Option<Self> {
        // Native 3961edf0 jump table: total sizes include the tag itself.
        let size = match kind {
            0 => 0,
            1 => 55,
            2 => 29,
            3 => 6,
            4 => 9,
            5 => 24,
            6 | 7 => 16,
            8 => 8,
            9 => 4,
            _ => return None,
        };
        if payload.len() != size {
            return None;
        }
        let mut bytes = [0u8; NATIVE_ITEM_BYTES];
        bytes[24] = kind;
        bytes[25..25 + size].copy_from_slice(payload);
        Some(unpack(&bytes).detail)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquipDetail {
    pub image_item_template_id: u32,
    pub durability: u8,
    /// Native raw bytes between durability and rune ID. The bevy server writes i16(0).
    /// Kept explicit so decoding and re-encoding other supplied records does not discard them.
    pub reserved: [u8; 2],
    pub rune_id: u32,
    pub charge_time: i64,
    pub dye_item_id: u32,
    pub gem_ids: [u32; 7],
    pub temper_physical: u16,
    pub temper_magical: u16,
}

/// Pack source-backed native fields; untouched union bytes and C++ alignment padding are zero.
/// This does not apply the chat formatter's ID/count normalization; call `format` afterward.
pub fn pack(item: &ItemRecord) -> [u8; NATIVE_ITEM_BYTES] {
    let mut out = [0u8; NATIVE_ITEM_BYTES];
    out[0..8].copy_from_slice(&item.id.to_le_bytes());
    out[8..12].copy_from_slice(&item.item_type.to_le_bytes());
    out[12] = item.grade;
    out[13] = item.flags;
    out[16..20].copy_from_slice(&item.count.to_le_bytes());
    out[20..24].copy_from_slice(&item.made_unit_id.to_le_bytes());
    match &item.detail {
        ItemDetail::None => {}
        ItemDetail::Equip(d) => {
            out[24] = 1;
            out[25..29].copy_from_slice(&d.image_item_template_id.to_le_bytes());
            out[29] = d.durability;
            out[30..32].copy_from_slice(&d.reserved);
            out[32..36].copy_from_slice(&d.rune_id.to_le_bytes());
            out[36..44].copy_from_slice(&d.charge_time.to_le_bytes());
            out[44..48].copy_from_slice(&d.dye_item_id.to_le_bytes());
            for (i, gem) in d.gem_ids.iter().enumerate() {
                out[48 + i * 4..52 + i * 4].copy_from_slice(&gem.to_le_bytes());
            }
            out[76..78].copy_from_slice(&d.temper_physical.to_le_bytes());
            out[78..80].copy_from_slice(&d.temper_magical.to_le_bytes());
        }
        ItemDetail::Mate { exp, dead, level } => {
            out[24] = 3;
            out[25..29].copy_from_slice(&exp.to_le_bytes());
            out[29] = *dead;
            out[30] = *level;
        }
        ItemDetail::Opaque { kind, payload } => {
            out[24] = *kind;
            out[25..152].copy_from_slice(payload);
        }
    }
    out[152..160].copy_from_slice(&item.create_time.to_le_bytes());
    out[160..164].copy_from_slice(&item.lifespan_mins.to_le_bytes());
    out[168] = item.world_id;
    out[176..184].copy_from_slice(&item.unsecure_time.to_le_bytes());
    out[184..192].copy_from_slice(&item.unpack_time.to_le_bytes());
    out
}

/// Read proven native fields, retaining every byte of an uninterpreted detail variant.
/// `decode_payload` remains available when the exact input padding/unused union bytes are needed.
pub fn unpack(record: &[u8; NATIVE_ITEM_BYTES]) -> ItemRecord {
    fn bytes<const N: usize>(r: &[u8], i: usize) -> [u8; N] {
        r[i..i + N].try_into().unwrap()
    }
    let detail = match record[24] {
        0 => ItemDetail::None,
        1 => ItemDetail::Equip(EquipDetail {
            image_item_template_id: u32::from_le_bytes(bytes(record, 25)),
            durability: record[29],
            reserved: bytes(record, 30),
            rune_id: u32::from_le_bytes(bytes(record, 32)),
            charge_time: i64::from_le_bytes(bytes(record, 36)),
            dye_item_id: u32::from_le_bytes(bytes(record, 44)),
            gem_ids: std::array::from_fn(|i| u32::from_le_bytes(bytes(record, 48 + i * 4))),
            temper_physical: u16::from_le_bytes(bytes(record, 76)),
            temper_magical: u16::from_le_bytes(bytes(record, 78)),
        }),
        3 => ItemDetail::Mate {
            exp: i32::from_le_bytes(bytes(record, 25)),
            dead: record[29],
            level: record[30],
        },
        kind => ItemDetail::Opaque {
            kind,
            payload: bytes(record, 25),
        },
    };
    ItemRecord {
        id: u64::from_le_bytes(bytes(record, 0)),
        item_type: u32::from_le_bytes(bytes(record, 8)),
        grade: record[12],
        flags: record[13],
        count: i32::from_le_bytes(bytes(record, 16)),
        made_unit_id: u32::from_le_bytes(bytes(record, 20)),
        detail,
        create_time: i64::from_le_bytes(bytes(record, 152)),
        lifespan_mins: i32::from_le_bytes(bytes(record, 160)),
        world_id: record[168],
        unsecure_time: i64::from_le_bytes(bytes(record, 176)),
        unpack_time: i64::from_le_bytes(bytes(record, 184)),
    }
}
