//! Gimmick announcements and raw motion reports (elevators, rafts and physics props).
//!
//! This table records received metadata, not loaded physics entities or ordinary units.

use std::collections::{HashMap, hash_map::Entry};

use crate::opcodes::sc;
use crate::session::Event;
use crate::wire::{Pos, Reader, Result, WireError, long_to_coord};

pub const MAX_GIMMICKS_CREATED: u8 = 30;
pub const MAX_GIMMICKS_REMOVED: u16 = 500;
pub const MAX_GIMMICK_JOINTS: u8 = 200;
/// Native serializer397c1330 passes 0x63 to its fixed-buffer string reader.
pub const MAX_GIMMICK_MODEL_PATH_BYTES: usize = 99;

fn model_path(r: &mut Reader) -> Result<String> {
    let length = r.i16()?;
    if length < 0 || length as usize > MAX_GIMMICK_MODEL_PATH_BYTES {
        return Err(WireError::Invalid(format!(
            "gimmick model path length {length}"
        )));
    }
    String::from_utf8(r.bytes(length as usize)?.to_vec())
        .map_err(|_| WireError::Invalid("gimmick model path is not UTF-8".into()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct GimmickSpawn {
    /// Full u32 here, unlike the BC IDs in `SCGimmicksRemoved`.
    pub obj_id: u32,
    pub template_id: u32,
    pub entity_guid: i64,
    pub faction_id: u32,
    pub spawner_obj_id: u32,
    pub grasper_obj_id: u32,
    /// Native signed i32 field preserved as the lane's u32 bit pattern.
    pub zone_id: u32,
    pub model_path: String,
    pub x: i64,
    pub y: i64,
    pub z: f32,
    /// Four wire floats, without quaternion normalization or component conversion.
    pub rotation: [f32; 4],
    pub scale: f32,
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub scale_velocity: f32,
}

impl GimmickSpawn {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            obj_id: r.u32()?,
            template_id: r.u32()?,
            entity_guid: r.i64()?,
            faction_id: r.u32()?,
            spawner_obj_id: r.u32()?,
            grasper_obj_id: r.u32()?,
            zone_id: r.u32()?,
            model_path: model_path(r)?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
            rotation: [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
            scale: r.f32()?,
            velocity: [r.f32()?, r.f32()?, r.f32()?],
            angular_velocity: [r.f32()?, r.f32()?, r.f32()?],
            scale_velocity: r.f32()?,
        })
    }

    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GimmickMovement {
    pub obj_id: u32,
    /// Raw source timestamp; the lane uses milliseconds since UTC midnight.
    pub time: u32,
    pub x: i64,
    pub y: i64,
    pub z: f32,
    pub rotation: [f32; 4],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub scale: f32,
}

impl GimmickMovement {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            obj_id: r.u32()?,
            time: r.u32()?,
            x: r.i64()?,
            y: r.i64()?,
            z: r.f32()?,
            rotation: [r.f32()?, r.f32()?, r.f32()?, r.f32()?],
            velocity: [r.f32()?, r.f32()?, r.f32()?],
            angular_velocity: [r.f32()?, r.f32()?, r.f32()?],
            scale: r.f32()?,
        })
    }

    pub fn position(&self) -> Pos {
        Pos::new(long_to_coord(self.x), long_to_coord(self.y), self.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GimmickJoint {
    pub obj_id: u32,
    pub joint_id: i32,
    /// Original signed wire field, without inferring a position or physics impulse.
    pub epicenter: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GimmickEvent {
    Created {
        raw_count: u8,
        gimmicks: Vec<GimmickSpawn>,
    },
    Removed {
        raw_count: u16,
        obj_ids: Vec<u32>,
    },
    Moved(GimmickMovement),
    /// Receive-only raw joint notices; the lane currently emits empty lists.
    JointsBroken {
        raw_count: u8,
        entries: Vec<GimmickJoint>,
    },
}

impl GimmickEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCGimmicksCreatedPacket
                | sc::SCGimmicksRemovedPacket
                | sc::SCGimmickMovementPacket
                | sc::SCGimmickJointsBrokenPacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Created { .. } => "SCGimmicksCreated",
            Self::Removed { .. } => "SCGimmicksRemoved",
            Self::Moved(_) => "SCGimmickMovement",
            Self::JointsBroken { .. } => "SCGimmickJointsBroken",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCGimmicksCreatedPacket => {
                let raw_count = r.u8()?;
                let gimmicks = (0..raw_count.min(MAX_GIMMICKS_CREATED))
                    .map(|_| GimmickSpawn::read(r))
                    .collect::<Result<Vec<_>>>()?;
                Self::Created {
                    raw_count,
                    gimmicks,
                }
            }
            sc::SCGimmicksRemovedPacket => {
                let raw_count = r.u16()?;
                let obj_ids = (0..raw_count.min(MAX_GIMMICKS_REMOVED))
                    .map(|_| r.bc())
                    .collect::<Result<Vec<_>>>()?;
                Self::Removed { raw_count, obj_ids }
            }
            sc::SCGimmickMovementPacket => Self::Moved(GimmickMovement::read(r)?),
            sc::SCGimmickJointsBrokenPacket => {
                let raw_count = r.u8()?;
                let entries = (0..raw_count.min(MAX_GIMMICK_JOINTS))
                    .map(|_| {
                        Ok(GimmickJoint {
                            obj_id: r.u32()?,
                            joint_id: r.i32()?,
                            epicenter: r.i32()?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Self::JointsBroken { raw_count, entries }
            }
            _ => return Err(WireError::Invalid(format!("gimmick opcode {opcode:#x}"))),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gimmick {
    pub spawn: GimmickSpawn,
    /// Last report in wire arrival order, not an interpolated or settled physics transform.
    pub last_movement: Option<GimmickMovement>,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Gimmicks {
    pub by_id: HashMap<u32, Gimmick>,
}

impl Gimmicks {
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::Gimmick(GimmickEvent::Created { gimmicks, .. }) => {
                let mut changed = false;
                for spawn in gimmicks {
                    // Native manager39179910 leaves an already registered object unchanged.
                    if let Entry::Vacant(entry) = self.by_id.entry(spawn.obj_id) {
                        entry.insert(Gimmick {
                            spawn: spawn.clone(),
                            last_movement: None,
                        });
                        changed = true;
                    }
                }
                changed
            }
            Event::Gimmick(GimmickEvent::Removed { obj_ids, .. }) => {
                let mut changed = false;
                for id in obj_ids {
                    changed |= self.by_id.remove(id).is_some();
                }
                changed
            }
            Event::Gimmick(GimmickEvent::Moved(movement)) => {
                let Some(gimmick) = self.by_id.get_mut(&movement.obj_id) else {
                    return false;
                };
                if gimmick.last_movement == Some(*movement) {
                    return false;
                }
                gimmick.last_movement = Some(*movement);
                true
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.by_id.is_empty() {
                    return false;
                }
                self.by_id.clear();
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::game::ServerMsg;
    use crate::wire::Writer;

    fn hex(raw: &str) -> Vec<u8> {
        raw.split_whitespace()
            .flat_map(|word| {
                (0..word.len())
                    .step_by(2)
                    .map(move |n| u8::from_str_radix(&word[n..n + 2], 16).unwrap())
            })
            .collect()
    }

    pub(crate) fn spawn() -> GimmickSpawn {
        GimmickSpawn {
            obj_id: 0xf1020304,
            template_id: 44,
            entity_guid: -2,
            faction_id: 1,
            spawner_obj_id: 0x203,
            grasper_obj_id: 0x304,
            zone_id: 100,
            model_path: "m".into(),
            x: 0x0000100089abcdef,
            y: -0x00001ffffedcba99,
            z: 3.5,
            rotation: [0.25, -0.5, 0.75, 1.0],
            scale: 1.5,
            velocity: [1.0, 2.0, 3.0],
            angular_velocity: [-4.0, -5.0, -6.0],
            scale_velocity: 0.5,
        }
    }

    pub(crate) fn movement() -> GimmickMovement {
        let spawn = spawn();
        GimmickMovement {
            obj_id: spawn.obj_id,
            time: 0x01020304,
            x: spawn.x,
            y: spawn.y,
            z: spawn.z,
            rotation: spawn.rotation,
            velocity: [0.0, 0.0, 0.25],
            angular_velocity: [0.0; 3],
            scale: 1.0,
        }
    }

    fn spawn_bytes() -> Vec<u8> {
        // Independent LE field golden for models/gimmick.rs::write, including nonzero low XY bits.
        hex(
            "040302f1 2c000000 feffffffffffffff 01000000 03020000 04030000 64000000 01006d
            efcdab8900100000 6745230100e0ffff 00006040
            0000803e 000000bf 0000403f 0000803f 0000c03f
            0000803f 00000040 00004040 000080c0 0000a0c0 0000c0c0 0000003f",
        )
    }

    fn movement_bytes() -> Vec<u8> {
        // Independent LE golden for gimmick/packets.rs (u32 obj/time + long XYZ + raw floats).
        hex(
            "040302f1 04030201 efcdab8900100000 6745230100e0ffff 00006040
            0000803e 000000bf 0000403f 0000803f
            00000000 00000000 0000803e 00000000 00000000 00000000 0000803f",
        )
    }

    fn dd01(opcode: u16, body: &[u8]) -> Result<ServerMsg> {
        let mut w = Writer::new();
        w.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(body);
        ServerMsg::decode(&w.finish())
    }

    fn with_tail(opcode: u16, body: &[u8], expected: GimmickEvent) {
        let mut inner = Writer::new();
        inner
            .u16(0)
            .u16(opcode)
            .raw(body)
            .u16(0)
            .u16(sc::SCOnOffSnowPacket)
            .bool(true);
        let inner = inner.finish();
        let len = inner.len() as u16;
        let mut w = Writer::new();
        w.raw(&[0xdd, 4])
            .u16(2)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&inner);
        let mut r = Reader::new(body);
        assert_eq!(GimmickEvent::read(opcode, &mut r).unwrap(), expected);
        assert!(r.is_empty());
        let expected = ServerMsg::Gimmick(expected);
        assert_eq!(dd01(opcode, body).unwrap(), expected);
        assert_eq!(
            ServerMsg::decode(&w.finish()).unwrap(),
            ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)])
        );
    }

    #[test]
    fn four_packet_goldens_preserve_exact_field_widths_and_every_truncation_fails() {
        let raw_spawn = spawn_bytes();
        assert_eq!(raw_spawn.len(), 103);
        let mut r = Reader::new(&raw_spawn);
        assert_eq!(GimmickSpawn::read(&mut r).unwrap(), spawn());
        assert!(r.is_empty());
        assert_eq!(spawn().position(), Pos::new(1.0, -2.0, 3.5));
        let raw_movement = movement_bytes();
        assert_eq!(raw_movement.len(), 72);
        assert_eq!(movement().position(), spawn().position());
        let mut raw_created = vec![1];
        raw_created.extend(raw_spawn);
        let cases = [
            (
                sc::SCGimmicksCreatedPacket,
                raw_created,
                GimmickEvent::Created {
                    raw_count: 1,
                    gimmicks: vec![spawn()],
                },
            ),
            (
                sc::SCGimmicksRemovedPacket,
                hex("0200 563412 ffffff"),
                GimmickEvent::Removed {
                    raw_count: 2,
                    obj_ids: vec![0x123456, 0xffffff],
                },
            ),
            (
                sc::SCGimmickMovementPacket,
                raw_movement,
                GimmickEvent::Moved(movement()),
            ),
            (
                sc::SCGimmickJointsBrokenPacket,
                hex("01 040302f1 00000080 ffffff7f"),
                GimmickEvent::JointsBroken {
                    raw_count: 1,
                    entries: vec![GimmickJoint {
                        obj_id: 0xf1020304,
                        joint_id: i32::MIN,
                        epicenter: i32::MAX,
                    }],
                },
            ),
        ];
        for (opcode, raw, expected) in cases {
            assert!(GimmickEvent::handles(opcode));
            assert!(!expected.name().is_empty());
            with_tail(opcode, &raw, expected);
            for n in 0..raw.len() {
                assert!(
                    GimmickEvent::read(opcode, &mut Reader::new(&raw[..n])).is_err(),
                    "{opcode:x} cut {n}"
                );
                assert!(dd01(opcode, &raw[..n]).is_err(), "{opcode:x} frame cut {n}");
            }
        }
        assert!(!GimmickEvent::handles(0x140));
        assert!(GimmickEvent::read(0x140, &mut Reader::new(&[])).is_err());
    }

    #[test]
    fn native_inline_caps_consume_only_the_bounded_list_and_keep_original_counts() {
        for raw_count in [0, 1, 30, 31, u8::MAX] {
            let mut raw = Writer::new();
            raw.u8(raw_count);
            let gimmicks = vec![spawn(); usize::from(raw_count.min(30))];
            for _ in &gimmicks {
                raw.raw(&spawn_bytes());
            }
            with_tail(
                sc::SCGimmicksCreatedPacket,
                &raw.finish(),
                GimmickEvent::Created {
                    raw_count,
                    gimmicks,
                },
            );
        }
        for raw_count in [0, 1, 500, 501, u16::MAX] {
            let obj_ids: Vec<u32> = (0..raw_count.min(500))
                .map(|n| if n % 2 == 0 { 0 } else { 0xffffff })
                .collect();
            let mut raw = Writer::new();
            raw.u16(raw_count);
            for id in &obj_ids {
                raw.bc(*id);
            }
            with_tail(
                sc::SCGimmicksRemovedPacket,
                &raw.finish(),
                GimmickEvent::Removed { raw_count, obj_ids },
            );
        }
        for raw_count in [0, 1, 200, 201, u8::MAX] {
            let entry = GimmickJoint {
                obj_id: u32::MAX,
                joint_id: -1,
                epicenter: i32::MIN,
            };
            let entries = vec![entry; usize::from(raw_count.min(200))];
            let mut raw = Writer::new();
            raw.u8(raw_count);
            for entry in &entries {
                raw.u32(entry.obj_id)
                    .i32(entry.joint_id)
                    .i32(entry.epicenter);
            }
            with_tail(
                sc::SCGimmickJointsBrokenPacket,
                &raw.finish(),
                GimmickEvent::JointsBroken { raw_count, entries },
            );
        }
    }

    #[test]
    fn model_paths_obey_the_native_fixed_buffer_byte_limit_without_losing_the_tail() {
        assert!(model_path(&mut Reader::new(&[1, 0, 0xff])).is_err());
        for path in [
            String::new(),
            "m".into(),
            "a".repeat(99),
            "\u{e9}".repeat(49) + "x",
            "m\0x".into(),
        ] {
            let original = spawn_bytes();
            let mut body = Writer::new();
            body.raw(&original[..32]).string(&path).raw(&original[35..]);
            let mut raw = Writer::new();
            raw.u8(1).raw(&body.finish());
            let mut expected = spawn();
            expected.model_path = path;
            with_tail(
                sc::SCGimmicksCreatedPacket,
                &raw.finish(),
                GimmickEvent::Created {
                    raw_count: 1,
                    gimmicks: vec![expected],
                },
            );
        }
        for length in [i16::MIN, -1, 100, i16::MAX] {
            let original = spawn_bytes();
            let mut body = Writer::new();
            body.raw(&original[..32])
                .i16(length)
                .raw(&vec![0; length.max(0) as usize])
                .raw(&original[35..]);
            assert!(GimmickSpawn::read(&mut Reader::new(&body.finish())).is_err());
        }
    }

    #[test]
    fn metadata_lifecycle_uses_announcements_and_keeps_motion_reports_unsettled() {
        let mut state = crate::state::WorldState::default();
        let motion = movement();
        assert!(
            !state
                .gimmicks
                .apply(&Event::Gimmick(GimmickEvent::Moved(motion)))
        );
        assert!(state.gimmicks.by_id.is_empty());
        let created = Event::Gimmick(GimmickEvent::Created {
            raw_count: 1,
            gimmicks: vec![spawn()],
        });
        state.apply(&created);
        assert_eq!(state.gimmicks.by_id[&motion.obj_id].spawn, spawn());
        assert!(state.gimmicks.by_id[&motion.obj_id].last_movement.is_none());
        assert!(!state.gimmicks.apply(&created));
        for time in [0x01020304, u32::MAX, 0, 1] {
            let report = GimmickMovement {
                time,
                x: i64::MIN,
                ..motion
            };
            assert!(
                state
                    .gimmicks
                    .apply(&Event::Gimmick(GimmickEvent::Moved(report)))
            );
            assert_eq!(
                state.gimmicks.by_id[&motion.obj_id].last_movement,
                Some(report)
            );
            assert_eq!(state.gimmicks.by_id[&motion.obj_id].spawn, spawn());
            assert!(
                !state
                    .gimmicks
                    .apply(&Event::Gimmick(GimmickEvent::Moved(report)))
            );
        }
        let mut duplicate = spawn();
        duplicate.model_path = "changed".into();
        duplicate.template_id += 1;
        assert!(
            !state.gimmicks.apply(&Event::Gimmick(GimmickEvent::Created {
                raw_count: 1,
                gimmicks: vec![duplicate.clone()],
            }))
        );
        assert_eq!(state.gimmicks.by_id[&motion.obj_id].spawn, spawn());
        let joints = Event::Gimmick(GimmickEvent::JointsBroken {
            raw_count: 1,
            entries: vec![GimmickJoint {
                obj_id: motion.obj_id,
                joint_id: 1,
                epicenter: 2,
            }],
        });
        assert!(!state.gimmicks.apply(&joints));
        assert!(
            state.units.by_id.is_empty(),
            "metadata must not create ordinary units"
        );
        assert!(state.player.spawn.is_none());
        let removed = Event::Gimmick(GimmickEvent::Removed {
            raw_count: 1,
            obj_ids: vec![motion.obj_id],
        });
        assert!(state.gimmicks.apply(&removed));
        assert!(!state.gimmicks.apply(&removed));
        assert!(
            !state
                .gimmicks
                .apply(&Event::Gimmick(GimmickEvent::Moved(motion)))
        );
        state.apply(&Event::Gimmick(GimmickEvent::Created {
            raw_count: 1,
            gimmicks: vec![duplicate.clone()],
        }));
        assert_eq!(state.gimmicks.by_id[&motion.obj_id].spawn, duplicate);
        assert!(state.gimmicks.by_id[&motion.obj_id].last_movement.is_none());
        for reset in [
            Event::CharacterSelected(Box::default()),
            Event::LeftWorld { target: 0 },
            Event::Disconnected {
                message: "end".into(),
            },
        ] {
            state.apply(&created);
            assert!(state.gimmicks.apply(&reset));
            assert!(!state.gimmicks.apply(&reset));
            assert!(state.gimmicks.by_id.is_empty());
        }
    }
}
