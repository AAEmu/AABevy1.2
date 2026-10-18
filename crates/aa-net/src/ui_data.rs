//! Per-character opaque UI data. This module does not interpret serialized UI tables.

use crate::opcodes::cs;
use crate::session::Event;
use crate::wire::{Reader, Result, WireError, Writer};

/// Native fixed-buffer reader limit (the buffer has an additional NUL byte).
pub const MAX_UI_DATA_BYTES: usize = 8191;
/// Application cache budget, independent of the native type table or server ownership rules.
pub const MAX_CACHED_UI_DATA: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiDataCommand {
    /// The lane responds only for a character owned by this connection. An unknown ID is silent.
    Request { character_id: u32, data_type: u16 },
    /// No save acknowledgement exists. The lane ignores this ID and saves to its active character;
    /// the session therefore requires this ID to match the active selected character.
    Save {
        character_id: u32,
        data_type: u16,
        payload: String,
    },
}

impl UiDataCommand {
    pub fn character_id(&self) -> u32 {
        match self {
            Self::Request { character_id, .. } | Self::Save { character_id, .. } => *character_id,
        }
    }

    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.character_id() == 0 {
            return Err("UI data requires a nonzero character ID");
        }
        if let Self::Save { payload, .. } = self {
            if payload.len() > MAX_UI_DATA_BYTES {
                return Err("UI data exceeds the native 8191-byte limit");
            }
            if payload.contains('\0') {
                return Err("UI data save contains an embedded NUL");
            }
        }
        Ok(())
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Request { .. } => cs::CSRequestUIDataPacket,
            Self::Save { .. } => cs::CSSaveUIDataPacket,
        }
    }

    /// Use `validate` before encoding a command outside the session API.
    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Request {
                character_id,
                data_type,
            } => {
                w.u16(*data_type).u32(*character_id);
            }
            Self::Save {
                character_id,
                data_type,
                payload,
            } => {
                w.u16(*data_type).u32(*character_id).string(payload);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiDataResponse {
    pub character_id: u32,
    /// Raw native/server type ID, including IDs outside the native seven-entry table.
    pub data_type: u16,
    /// Opaque UTF-8 text, preserved verbatim, including any received NULs.
    pub payload: String,
    /// Native signed trailing field, preserved without assigning it a universal meaning.
    /// The lane writes UTF-16 code units + 1; the native lobby consumer copies this many bytes.
    pub raw_length: i32,
}

impl UiDataResponse {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let character_id = r.u32()?;
        let data_type = r.u16()?;
        let length = r.i16()?;
        if length < 0 || length as usize > MAX_UI_DATA_BYTES {
            return Err(WireError::Invalid(format!("UI data byte length {length}")));
        }
        let payload = String::from_utf8(r.bytes(length as usize)?.to_vec())
            .map_err(|_| WireError::Invalid("UI data is not UTF-8".into()))?;
        Ok(Self {
            character_id,
            data_type,
            payload,
            raw_length: r.i32()?,
        })
    }

    /// Checks the lane writer's exact rule, counting supplementary characters as two units.
    /// A mismatch is metadata, not an allocation length or evidence of a failed save.
    pub fn matches_lane_utf16_length(&self) -> bool {
        self.raw_length == self.payload.encode_utf16().count() as i32 + 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiDataEvent {
    Response(UiDataResponse),
    /// The request reached the local socket. This is neither a server acknowledgement nor proof
    /// of ownership, acceptance, persistence or a response to a preceding save.
    Requested(UiDataCommand),
    /// Local validation only. The lane has no corresponding rejection packet.
    RequestRejected {
        command: UiDataCommand,
        message: String,
    },
}

/// Last received value for each (character ID, raw type) in wire-arrival order, oldest first.
/// Responses alone populate this bounded cache; requests and saves never predict server values.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct UiData {
    entries: Vec<UiDataResponse>,
}

impl UiData {
    pub fn entries(&self) -> &[UiDataResponse] {
        &self.entries
    }

    pub fn get(&self, character_id: u32, data_type: u16) -> Option<&UiDataResponse> {
        self.entries
            .iter()
            .find(|v| v.character_id == character_id && v.data_type == data_type)
    }

    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::UiData(UiDataEvent::Response(value)) => {
                if value.payload.len() > MAX_UI_DATA_BYTES {
                    return false;
                }
                if let Some(index) = self.entries.iter().position(|v| {
                    v.character_id == value.character_id && v.data_type == value.data_type
                }) {
                    if self.entries[index] == *value && index + 1 == self.entries.len() {
                        return false;
                    }
                    self.entries.remove(index);
                }
                if self.entries.len() >= MAX_CACHED_UI_DATA {
                    self.entries.remove(0);
                }
                self.entries.push(value.clone());
                true
            }
            // Lobby responses can precede selection; retain only that character's values then.
            Event::CharacterSelected(state) => {
                let before = self.entries.len();
                self.entries
                    .retain(|v| v.character_id == state.character.id);
                before != self.entries.len()
            }
            Event::LoginSucceeded(_)
            | Event::ReadyToConnect { .. }
            | Event::ConnectFailed { .. }
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. } => {
                let changed = !self.entries.is_empty();
                self.entries.clear();
                changed
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};
    use crate::opcodes::sc;

    fn response_body(value: &UiDataResponse) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(value.character_id)
            .u16(value.data_type)
            .string(&value.payload)
            .i32(value.raw_length);
        w.finish()
    }

    fn dd01(body: &[u8]) -> Result<ServerMsg> {
        let mut frame = vec![0xdd, 1, 0, 0, 0xca, 1];
        frame.extend_from_slice(body);
        ServerMsg::decode(&frame)
    }

    #[test]
    fn ui_data_request_save_goldens_and_opaque_round_trip() {
        let request = UiDataCommand::Request {
            character_id: 0xf1020304,
            data_type: 0xff05,
        };
        assert_eq!(
            ClientMsg::UiData(request).encode(7),
            [0xdd, 1, 0, 7, 0x17, 1, 5, 0xff, 4, 3, 2, 0xf1]
        );
        let payload = "x\u{00e9}\u{1f600}";
        let save = UiDataCommand::Save {
            character_id: 0xf1020304,
            data_type: 0xff05,
            payload: payload.into(),
        };
        assert!(save.validate().is_ok());
        let bytes = ClientMsg::UiData(save).encode_body();
        assert_eq!(
            bytes,
            [
                5, 0xff, 4, 3, 2, 0xf1, 7, 0, b'x', 0xc3, 0xa9, 0xf0, 0x9f, 0x98, 0x80
            ]
        );
        // Independently consume exactly the lane request/save reader's widths, without a tail.
        let mut r = Reader::new(&bytes);
        assert_eq!((r.u16().unwrap(), r.u32().unwrap()), (0xff05, 0xf1020304));
        assert_eq!(r.bytes(r.remaining()).unwrap(), &bytes[6..]);
        let value = UiDataResponse {
            character_id: 0xf1020304,
            data_type: 0xff05,
            payload: payload.into(),
            raw_length: 5,
        };
        assert!(value.matches_lane_utf16_length());
        assert_eq!(
            dd01(&response_body(&value)).unwrap(),
            ServerMsg::UiData(value)
        );
    }

    #[test]
    fn ui_data_response_exact_bounds_truncations_and_raw_length_are_preserved() {
        let value = UiDataResponse {
            character_id: 1,
            data_type: 2,
            payload: "ab".into(),
            raw_length: 3,
        };
        let body = response_body(&value);
        assert_eq!(body, [1, 0, 0, 0, 2, 0, 2, 0, b'a', b'b', 3, 0, 0, 0]);
        let mut r = Reader::new(&body);
        assert_eq!(UiDataResponse::read(&mut r).unwrap(), value);
        assert!(r.is_empty());
        for cut in 0..body.len() {
            assert!(dd01(&body[..cut]).is_err(), "cut {cut}");
        }
        for raw_length in [i32::MIN, -1, 0, 2, i32::MAX] {
            let value = UiDataResponse {
                raw_length,
                ..value.clone()
            };
            assert!(!value.matches_lane_utf16_length());
            assert_eq!(
                dd01(&response_body(&value)).unwrap(),
                ServerMsg::UiData(value)
            );
        }
        let nul = UiDataResponse {
            payload: "\0ab\0".into(),
            raw_length: 5,
            ..value.clone()
        };
        assert_eq!(dd01(&response_body(&nul)).unwrap(), ServerMsg::UiData(nul));
        for size in [0, MAX_UI_DATA_BYTES] {
            let value = UiDataResponse {
                payload: "a".repeat(size),
                raw_length: size as i32 + 1,
                ..value.clone()
            };
            assert_eq!(
                dd01(&response_body(&value)).unwrap(),
                ServerMsg::UiData(value)
            );
        }
        for length in [-1_i16, i16::MIN, 8192, i16::MAX] {
            let mut invalid = vec![1, 0, 0, 0, 2, 0];
            invalid.extend(length.to_le_bytes());
            assert!(dd01(&invalid).is_err());
        }
        for invalid in [vec![0xc0, 0x80], vec![0xed, 0xa0, 0x80], vec![0xf0, 0x9f]] {
            let mut w = Writer::new();
            w.u32(1)
                .u16(5)
                .i16(invalid.len() as i16)
                .raw(&invalid)
                .i32(1);
            assert!(dd01(&w.finish()).is_err());
        }
    }

    #[test]
    fn ui_data_dd04_preserves_response_order_and_following_state() {
        let values = [
            UiDataResponse {
                character_id: 1,
                data_type: 5,
                payload: String::new(),
                raw_length: 1,
            },
            UiDataResponse {
                character_id: 2,
                data_type: 0xffff,
                payload: "\u{1f600}".into(),
                raw_length: 3,
            },
            UiDataResponse {
                character_id: 1,
                data_type: 5,
                payload: "replacement".into(),
                raw_length: 12,
            },
        ];
        let mut inner = Writer::new();
        for value in &values {
            inner
                .u16(0)
                .u16(sc::SCResponseUIDataPacket)
                .raw(&response_body(value));
        }
        inner.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let inner = inner.finish();
        let length = inner.len() as u16;
        let mut frame = Writer::new();
        frame
            .raw(&[0xdd, 4])
            .u16(4)
            .u8(1)
            .u16(length)
            .u16(!length)
            .raw(&inner);
        let expected: Vec<_> = values
            .into_iter()
            .map(ServerMsg::UiData)
            .chain([ServerMsg::Snow(true)])
            .collect();
        assert_eq!(
            ServerMsg::decode(&frame.finish()).unwrap(),
            ServerMsg::Batch(expected)
        );
    }

    #[test]
    fn ui_data_cache_is_received_only_bounded_and_scoped_to_connection_and_character() {
        let mut cache = UiData::default();
        let save = UiDataCommand::Save {
            character_id: 1,
            data_type: 5,
            payload: "new".into(),
        };
        assert!(!cache.apply(&Event::UiData(UiDataEvent::Requested(save.clone()))));
        assert!(cache.entries().is_empty());
        for data_type in 0..=MAX_CACHED_UI_DATA as u16 {
            assert!(
                cache.apply(&Event::UiData(UiDataEvent::Response(UiDataResponse {
                    character_id: 1,
                    data_type,
                    payload: data_type.to_string(),
                    raw_length: -1,
                })))
            );
        }
        assert_eq!(cache.entries().len(), MAX_CACHED_UI_DATA);
        assert!(cache.get(1, 0).is_none());
        assert_eq!(cache.get(1, 1).unwrap().payload, "1");
        let value = UiDataResponse {
            character_id: 1,
            data_type: 1,
            payload: "replacement".into(),
            raw_length: 12,
        };
        assert!(cache.apply(&Event::UiData(UiDataEvent::Response(value.clone()))));
        assert!(!cache.apply(&Event::UiData(UiDataEvent::Response(value.clone()))));
        assert_eq!(cache.entries().last(), Some(&value));
        assert!(!cache.apply(&Event::UiData(UiDataEvent::Requested(save))));
        assert_eq!(cache.get(1, 1), Some(&value));
        let oversize = UiDataResponse {
            payload: "a".repeat(MAX_UI_DATA_BYTES + 1),
            ..value.clone()
        };
        assert!(!cache.apply(&Event::UiData(UiDataEvent::Response(oversize))));
        let mut selected = crate::model::CharacterState::default();
        selected.character.id = 1;
        assert!(!cache.apply(&Event::CharacterSelected(Box::new(selected.clone()))));
        selected.character.id = 2;
        assert!(cache.apply(&Event::CharacterSelected(Box::new(selected))));
        assert!(cache.entries().is_empty());
        for lifecycle in [
            Event::LeftWorld { target: 1 },
            Event::Disconnected {
                message: "end".into(),
            },
            Event::ConnectFailed {
                message: "fail".into(),
            },
        ] {
            cache.apply(&Event::UiData(UiDataEvent::Response(value.clone())));
            assert!(cache.apply(&lifecycle));
            assert!(cache.entries().is_empty());
        }
    }

    #[test]
    fn ui_data_save_preflight_uses_bytes_and_rejects_native_c_string_truncation() {
        for payload in [
            String::new(),
            "a".repeat(MAX_UI_DATA_BYTES),
            "\u{00e9}".repeat(4095),
        ] {
            assert!(
                UiDataCommand::Save {
                    character_id: 1,
                    data_type: u16::MAX,
                    payload
                }
                .validate()
                .is_ok()
            );
        }
        for payload in [
            "a".repeat(MAX_UI_DATA_BYTES + 1),
            "\u{00e9}".repeat(4096),
            "a\0b".into(),
        ] {
            assert!(
                UiDataCommand::Save {
                    character_id: 1,
                    data_type: 5,
                    payload
                }
                .validate()
                .is_err()
            );
        }
        assert!(
            UiDataCommand::Request {
                character_id: 0,
                data_type: 5
            }
            .validate()
            .is_err()
        );
    }
}
