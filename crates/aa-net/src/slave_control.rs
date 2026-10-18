//! Explicit native slave controls. Requests never mutate authoritative owned/attachment state.

use crate::opcodes::cs;
use crate::wire::Writer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlaveItemLocation {
    pub container: u8,
    pub slot: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlaveSpawnRequest {
    pub template_id: u32,
    /// Native long fixed-point coordinates, retained without a client-side position planner.
    pub x: i64,
    pub y: i64,
    pub z: f32,
    pub yaw: f32,
    pub item_id: u64,
    /// Present exactly when item_id != 0. Native writes two bytes, not the lane's four.
    pub item_location: Option<SlaveItemLocation>,
    pub hide_spawn_effect: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SlaveCommand {
    Bind {
        tl_id: u16,
    },
    Unbind {
        tl_id: u16,
    },
    Dismiss {
        tl_id: u16,
    },
    Despawn {
        obj_id: u32,
    },
    Rename {
        tl_id: u16,
        name: String,
    },
    /// Exact native packet only: lane handler is a stub and its reader mismatches item locations.
    Spawn(SlaveSpawnRequest),
}

impl SlaveCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Bind { tl_id }
            | Self::Unbind { tl_id }
            | Self::Dismiss { tl_id }
            | Self::Rename { tl_id, .. }
                if *tl_id == 0 =>
            {
                return Err("slave timeline ID must be nonzero");
            }
            Self::Despawn { obj_id } if *obj_id == 0 || *obj_id > 0xffffff => {
                return Err("slave object ID must be nonzero and fit in 24 bits");
            }
            _ => {}
        }
        match self {
            Self::Rename { name, .. }
                if name.trim().is_empty() || name.len() > 128 || name.contains('\0') =>
            {
                Err(
                    "slave name must contain text, fit the native 128-byte buffer and contain no NUL",
                )
            }
            Self::Spawn(p) if p.template_id == 0 => Err("slave template ID must be nonzero"),
            Self::Spawn(p) if !p.z.is_finite() || !p.yaw.is_finite() => {
                Err("slave spawn height and yaw must be finite")
            }
            Self::Spawn(p) if (p.item_id != 0) != p.item_location.is_some() => {
                Err("slave item location must be present exactly when the item ID is nonzero")
            }
            _ => Ok(()),
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::Bind { .. } => cs::CSBindSlavePacket,
            Self::Unbind { .. } => cs::CSDiscardSlavePacket,
            Self::Dismiss { .. } => cs::CSDestroySlavePacket,
            Self::Despawn { .. } => cs::CSDespawnSlavePacket,
            Self::Rename { .. } => cs::CSChangeSlaveNamePacket,
            Self::Spawn(_) => cs::CSSpawnSlavePacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::Bind { tl_id } | Self::Unbind { tl_id } | Self::Dismiss { tl_id } => {
                w.u16(*tl_id);
            }
            Self::Despawn { obj_id } => {
                w.bc(*obj_id);
            }
            Self::Rename { tl_id, name } => {
                w.u16(*tl_id).string(name);
            }
            Self::Spawn(p) => {
                w.u32(p.template_id)
                    .i64(p.x)
                    .i64(p.y)
                    .f32(p.z)
                    .f32(p.yaw)
                    .u64(p.item_id);
                if p.item_id != 0 {
                    if let Some(location) = p.item_location {
                        w.u8(location.container).u8(location.slot);
                    }
                }
                w.bool(p.hide_spawn_effect);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ClientMsg;

    pub(crate) fn spawn() -> SlaveSpawnRequest {
        SlaveSpawnRequest {
            template_id: 68,
            x: 0x0102030405060708,
            y: 0x1122334455667788,
            z: 3.5,
            yaw: 0.5,
            item_id: 0,
            item_location: None,
            hide_spawn_effect: true,
        }
    }

    #[test]
    fn native_controls_have_exact_timeline_object_and_string_widths() {
        let cases = [
            (SlaveCommand::Bind { tl_id: 0x102 }, 0x31, vec![2, 1]),
            (SlaveCommand::Unbind { tl_id: 0xffff }, 0x32, vec![255, 255]),
            (SlaveCommand::Dismiss { tl_id: 0x102 }, 0x30, vec![2, 1]),
            (
                SlaveCommand::Despawn { obj_id: 0xaabbcc },
                0x2f,
                vec![0xcc, 0xbb, 0xaa],
            ),
            (
                SlaveCommand::Rename {
                    tl_id: 0x102,
                    name: "Boat".into(),
                },
                0x34,
                vec![2, 1, 4, 0, b'B', b'o', b'a', b't'],
            ),
        ];
        for (command, opcode, body) in cases {
            assert!(command.validate().is_ok());
            let msg = ClientMsg::Slave(command);
            assert_eq!(msg.opcode(), opcode);
            assert_eq!(msg.encode_body(), body);
        }
    }

    #[test]
    fn native_spawn_conditionally_writes_only_two_item_location_bytes() {
        let p = spawn();
        let base = vec![
            68, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0,
            0, 0x60, 0x40, 0, 0, 0, 0x3f,
        ];
        let mut expected = base.clone();
        expected.extend([0; 8]);
        expected.push(1);
        let command = SlaveCommand::Spawn(p.clone());
        assert!(command.validate().is_ok());
        let msg = ClientMsg::Slave(command);
        assert_eq!(msg.opcode(), 0x2e);
        assert_eq!(msg.encode_body(), expected);
        assert_eq!(expected.len(), 37);
        let p = SlaveSpawnRequest {
            item_id: 0x0102030405060708,
            item_location: Some(SlaveItemLocation {
                container: 2,
                slot: 9,
            }),
            hide_spawn_effect: false,
            ..p
        };
        let mut expected = base;
        expected.extend([8, 7, 6, 5, 4, 3, 2, 1, 2, 9, 0]);
        assert_eq!(
            ClientMsg::Slave(SlaveCommand::Spawn(p)).encode_body(),
            expected
        );
        assert_eq!(expected.len(), 39);
        // The lane reader requires four slot bytes then hide, so it cannot read this native body.
        let mut r = crate::wire::Reader::new(&expected);
        r.skip(36).unwrap();
        assert!(r.bytes(5).is_err());
    }

    #[test]
    fn malformed_ids_names_geometry_and_conditional_item_locations_are_rejected() {
        let mut invalid = vec![
            SlaveCommand::Bind { tl_id: 0 },
            SlaveCommand::Unbind { tl_id: 0 },
            SlaveCommand::Dismiss { tl_id: 0 },
            SlaveCommand::Despawn { obj_id: 0 },
            SlaveCommand::Despawn { obj_id: 0x1000000 },
        ];
        for name in ["".into(), " ".into(), "x\0y".into(), "x".repeat(129)] {
            invalid.push(SlaveCommand::Rename { tl_id: 1, name });
        }
        let p = spawn();
        for p in [
            SlaveSpawnRequest {
                template_id: 0,
                ..p.clone()
            },
            SlaveSpawnRequest {
                z: f32::NAN,
                ..p.clone()
            },
            SlaveSpawnRequest {
                yaw: f32::INFINITY,
                ..p.clone()
            },
            SlaveSpawnRequest {
                item_id: 9,
                ..p.clone()
            },
            SlaveSpawnRequest {
                item_location: Some(SlaveItemLocation {
                    container: 2,
                    slot: 0,
                }),
                ..p
            },
        ] {
            invalid.push(SlaveCommand::Spawn(p));
        }
        for command in invalid {
            assert!(command.validate().is_err(), "{command:?}");
        }
        assert!(
            SlaveCommand::Rename {
                tl_id: 1,
                name: "x".repeat(128)
            }
            .validate()
            .is_ok()
        );
    }
}
