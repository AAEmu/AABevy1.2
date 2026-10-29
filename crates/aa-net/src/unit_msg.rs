//! Packets about what other units do: batched movements, aggro, the first hit of a fight, riding a mount and posture changes.
//!
//! Layouts follow the server's writers (`models/movement.rs`, `systems/skills/packets.rs`, `packets/world_objects.rs`,
//! `systems/vehicles/packets.rs`, `systems/npcs/npc/packets.rs`) and their byte tests.

use crate::model::Movement;
use crate::wire::{Reader, Result, WireError};

/// The server says a batch holds at most this many movements.
const MAX_MOVEMENTS: u16 = 400;

/// `SCUnitMovements`: `u16` count, then per entry the unit's `bc` id and `MoveType.Write`. There is **no type byte** per entry
/// (unlike `SCOneUnitMovement`): the real client takes the movement type from the object. Every entry is read as the movement
/// of a unit (players, NPCs, mounts), which is what the server batches; a vehicle or ship entry would desync the rest.
pub fn read_unit_movements(r: &mut Reader) -> Result<Vec<(u32, Movement)>> {
    let count = r.u16()?;
    if count > MAX_MOVEMENTS {
        return Err(WireError::Invalid(format!("{count} batched unit movements")));
    }
    let mut out = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let obj_id = r.bc()?;
        out.push((obj_id, Movement::read_typed(1, r)?));
    }
    Ok(out)
}

/// One hostile unit of an `SCAiAggro` list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AggroEntry {
    pub hostile: u32,
    /// The damage the hostile unit has done to the NPC so far (the server's "summarize damage").
    pub damage: i32,
    pub val2: i32,
    pub val3: i32,
    /// The server always sends 135.
    pub flags: u8,
}

/// `SCAiAggro`: the NPC's `bc` id and the number of hostile units, then per hostile unit its `bc`, three `i32` and a flag byte.
/// A count of 0 clears the NPC's list (it left combat or died).
pub fn read_ai_aggro(r: &mut Reader) -> Result<(u32, Vec<AggroEntry>)> {
    let npc = r.bc()?;
    let count = r.i32()?;
    // Every entry is 16 bytes. Consume the announced list in full: silently clamping it
    // leaves entry bytes where the following opcode of a DD04 batch is expected.
    let count = usize::try_from(count).map_err(|_| WireError::Invalid("negative aggro count".into()))?;
    if count > r.remaining() / 16 {
        return Err(WireError::Invalid(format!("{count} aggro entries exceed packet body")));
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(AggroEntry { hostile: r.bc()?, damage: r.i32()?, val2: r.i32()?, val3: r.i32()?, flags: r.u8()? });
    }
    Ok((npc, entries))
}

/// Raw native house posture bytes; bool consumers compare each value to1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HousePosture {
    pub has_loot_left: bool,
    pub doors: [u8; 2],
    pub windows: [u8; 6],
}

impl HousePosture {
    pub fn read(r: &mut Reader, has_loot_left: bool) -> Result<Self> {
        Ok(Self { has_loot_left, doors: [r.u8()?, r.u8()?], windows: [r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?] })
    }
}

/// What `SCUnitModelPostureChanged` reports: the unit's posture (`ModelPostureType`: 0 none, 1 house, 4 actor model state,
/// 7 farm field, 8 turret) and, for an actor model state, the animation action (`anim_actions.id`) it stands in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Posture {
    pub kind: u8,
    /// The unit still has loot to take.
    pub has_loot_left: bool,
    /// `anim_actions.id` of the idle the unit plays (a merchant's pose, a guard's stance); 0 for none.
    pub anim_action: u32,
    /// Play it now (false: the unit leaves the pose).
    pub activate: bool,
    pub house: Option<HousePosture>,
}

impl Posture {
    /// `Unit.ModelPosture`: the posture byte, the loot flag, and the fields of the posture.
    pub fn read(r: &mut Reader) -> Result<Self> {
        let kind = r.u8()?;
        let has_loot_left = r.bool()?;
        let mut posture = Self { kind, has_loot_left, anim_action: 0, activate: false, house: None };
        match kind {
            0 => {}
            1 => posture.house = Some(HousePosture::read(r, has_loot_left)?), // two doors, six windows
            4 => {
                posture.anim_action = r.u32()?;
                posture.activate = r.bool()?;
            }
            7 => r.skip(14)?, // farm field: u32, f32, i32, two flags
            8 => r.skip(8)?,  // turret: pitch, yaw
            other => return Err(WireError::Invalid(format!("model posture {other}"))),
        }
        Ok(posture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{actor_flags, MoveSupport};
    use crate::wire::{Pos, Writer};

    #[test]
    fn aggro_lists_are_consumed_in_full_and_invalid_counts_are_rejected() {
        let mut w = Writer::new();
        w.bc(5).i32(65);
        for id in 0..65 {
            w.bc(id).i32(7).i32(0).i32(0).u8(135);
        }
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let (npc, entries) = read_ai_aggro(&mut r).unwrap();
        assert_eq!((npc, entries.len(), entries[64].hostile), (5, 65, 64));
        assert!(r.is_empty());
        for count in [-1, 1, i32::MAX] {
            let mut w = Writer::new();
            w.bc(5).i32(count);
            assert!(read_ai_aggro(&mut Reader::new(&w.finish())).is_err(), "count {count}");
        }
    }

    /// The unit movement bytes of the server's tests (`unit_bytes`): `flags`, `actor_flags`, the optional blocks.
    fn unit_bytes(flags: u8, actor: u8) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(0xAABB_CCDD).u8(flags);
        if flags & 0x10 != 0 {
            w.u32(0x0403_0201).u8(5); // ScType, Phase
        }
        w.position(Pos::new(17_123.25, 9_876.5, 112.375));
        w.i16(1).i16(-2).i16(3);
        w.i8(0x10).i8(-16).i8(127); // rotation
        w.i8(-1).i8(1).i8(-128); // delta movement
        w.u8(1).u8(2).u8(actor); // stance, alertness, actor flags
        if actor & 0x80 != 0 {
            w.u16(0x1234);
        }
        if actor & 0x20 != 0 {
            w.u8(0x5A).u16(0x0102);
            w.position(Pos::new(-3.5, 4.25, -12.0));
            w.i8(-2).i8(2).i8(0x40);
        }
        if actor & 0x60 != 0 {
            w.u32(0x0BAD_F00D);
        }
        if actor & 0x40 != 0 {
            w.u32(0x1357_9BDF);
        }
        w.finish()
    }

    #[test]
    fn unit_movement_with_every_actor_flag_combination() {
        for flags in [0x00u8, 0x10, 0x16] {
            for af in 0..=255u8 {
                let bytes = unit_bytes(flags, af);
                let mut r = Reader::new(&bytes);
                let m = Movement::read_typed(1, &mut r).unwrap();
                assert!(r.is_empty(), "flags {flags:#x} actor flags {af:#x}: {} bytes left", r.remaining());
                assert_eq!((m.velocity, m.tilt, m.direction, m.delta_movement), ([1, -2, 3], [0x10, -16], 127, [-1, 1, -128]));
                assert_eq!((m.stance, m.alertness, m.actor_flags), (1, 2, af));
                assert_eq!(m.fall_vel, if af & actor_flags::FALL_VELOCITY != 0 { 0x1234 } else { 0 });
                let on_object = af & (actor_flags::STANDING_ON_OBJECT | actor_flags::HANGING_FROM_OBJECT) != 0;
                assert_eq!(m.support.is_some(), on_object);
                if let Some(s) = &m.support {
                    assert_eq!(s.obj_id, 0x0BAD_F00D);
                    assert_eq!(s.climb, (af & actor_flags::HANGING_FROM_OBJECT != 0).then_some(0x1357_9BDF));
                    assert_eq!(s.world.is_some(), af & actor_flags::STANDING_ON_OBJECT != 0);
                }
            }
        }
    }

    #[test]
    fn standing_on_an_object_has_its_world_position() {
        let bytes = unit_bytes(0, actor_flags::STANDING_ON_OBJECT | actor_flags::STANDING_ON_SOLID);
        let m = Movement::read_typed(1, &mut Reader::new(&bytes)).unwrap();
        let MoveSupport { obj_id, kind, part, world, climb } = m.support.unwrap();
        assert_eq!((obj_id, kind, part, climb), (0x0BAD_F00D, 0x5A, 0x0102, None));
        let (pos, rot) = world.unwrap();
        assert_eq!((pos.x, pos.y), (-3.5, 4.25));
        assert_eq!(rot, [-2, 2, 0x40]);
        // the position of the movement itself is the relative one
        assert_eq!((m.pos.x, m.pos.y), (17_123.25, 9_876.5));
    }

    #[test]
    fn truncated_unit_movements_are_errors() {
        let bytes = unit_bytes(0, 0xE0);
        for n in 0..bytes.len() {
            assert!(Movement::read_typed(1, &mut Reader::new(&bytes[..n])).is_err(), "len {n}");
        }
    }

    #[test]
    fn vehicle_ship_and_transfer_movements() {
        let pos = Pos::new(-3.5, 4.25, -12.0);
        for wheels in [0u8, 1, 4] {
            let mut w = Writer::new();
            w.u32(55).u8(0x02).position(pos).i16(1).i16(2).i16(3);
            w.i16(0).i16(0).i16(23170); // 90 degrees about z
            w.f32(0.5).f32(-1.25).f32(1.0).f32(-1.0).u8(wheels);
            for i in 0..wheels {
                w.f32(f32::from(i) * 1.5);
            }
            let bytes = w.finish();
            let mut r = Reader::new(&bytes);
            let m = Movement::read_typed(2, &mut r).unwrap();
            assert!(r.is_empty());
            assert_eq!((m.pos.x, m.velocity), (-3.5, [1, 2, 3]));
            assert!((m.yaw() - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "{}", m.yaw());
        }
        // ship: flags with the sc type and phase block
        let mut w = Writer::new();
        w.u32(1234).u8(0x10).u32(7).u8(3).position(pos);
        w.i16(-1).i16(-32768).i16(1);
        w.i16(0).i16(0).i16(0);
        w.f32(1.0).f32(2.0).f32(3.0).i8(-127).i8(127).u16(210).u8(1);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let m = Movement::read_typed(3, &mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(m.velocity, [-1, -32768, 1]);
        assert_eq!(m.yaw(), 0.0);
        // transfer: velocity shorts (m/s / 50), rotation shorts, angular velocity, steering, path point, speed, reverse
        let mut w = Writer::new();
        w.u32(86_399_999).u8(0x02).position(pos);
        w.i16(1000).i16(-327).i16(0);
        w.i16(0).i16(-12345).i16(23170);
        w.f32(0.25).f32(-0.5).f32(8.0).i32(-1).i32(17).f32(5.5).u8(1);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let m = Movement::read_typed(5, &mut r).unwrap();
        assert!(r.is_empty());
        // 1000 / 32767 * 50 m/s
        assert!((f32::from(m.velocity[0]) / 1024.0 - 1000.0 / 32767.0 * 50.0).abs() < 0.01);
        // a ship request has no position at all: throttle, steering
        let mut w = Writer::new();
        w.u32(2).u8(0).i8(-1).i8(1);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let m = Movement::read_typed(4, &mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(m.pos, Pos::default());
    }

    /// The server's `sc_unit_movements_body` test uses ship requests (type 4); the units version of the same layout.
    #[test]
    fn unit_movements_batch_has_no_type_byte() {
        let mut w = Writer::new();
        w.u16(2);
        w.bc(1);
        w.raw(&unit_bytes(0x00, 0x04));
        w.bc(0x00FF_FFFF);
        w.raw(&unit_bytes(0x10, 0x84));
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let list = read_unit_movements(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].0, list[1].0), (1, 0x00FF_FFFF));
        assert_eq!(list[0].1.time, 0xAABB_CCDD);
        assert_eq!(list[1].1.fall_vel, 0x1234);
        // an empty batch is two zero bytes
        assert!(read_unit_movements(&mut Reader::new(&[0, 0])).unwrap().is_empty());
        // a count the server never sends
        assert!(read_unit_movements(&mut Reader::new(&[0x91, 0x01])).is_err());
    }

    #[test]
    fn ai_aggro_golden_bytes() {
        // `small_packets` of the server: `SCAiAggroPacket::new(1, 0)` is the bc and a zero count.
        let (npc, list) = read_ai_aggro(&mut Reader::new(&[1, 0, 0, 0, 0, 0, 0])).unwrap();
        assert_eq!((npc, list.len()), (1, 0));
        // the interaction answer of the server (`npc` test): the bc of object 0x200 and a zero count
        let (npc, list) = read_ai_aggro(&mut Reader::new(&[0x00, 0x02, 0x00, 0, 0, 0, 0])).unwrap();
        assert_eq!((npc, list.len()), (0x200, 0));
        // count 1: hostile bc, damage, val2, val3 and the 135 flag
        let mut w = Writer::new();
        w.bc(0x1234).i32(1).bc(2).i32(3).i32(0).i32(0).u8(135);
        let bytes = w.finish();
        assert_eq!(bytes.len(), 3 + 4 + 3 + 12 + 1);
        let mut r = Reader::new(&bytes);
        let (npc, list) = read_ai_aggro(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(npc, 0x1234);
        assert_eq!(list, [AggroEntry { hostile: 2, damage: 3, val2: 0, val3: 0, flags: 135 }]);
    }

    #[test]
    fn postures() {
        // the server's `SCUnitModelPostureChanged`: a unit with no posture (the 5 byte packet that arrives all the time)
        let p = Posture::read(&mut Reader::new(&[0, 0])).unwrap();
        assert_eq!(p, Posture::default());
        // an actor model state: the animation action and the activate flag
        let mut w = Writer::new();
        w.u8(4).bool(true).u32(100).bool(true);
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        let p = Posture::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(p, Posture { kind: 4, has_loot_left: true, anim_action: 100, activate: true, house: None });
        for (kind, len) in [(1u8, 8usize), (7, 14), (8, 8)] {
            let mut bytes = vec![kind, 0];
            bytes.extend(std::iter::repeat_n(1u8, len));
            let mut r = Reader::new(&bytes);
            Posture::read(&mut r).unwrap();
            assert!(r.is_empty(), "posture {kind}");
        }
        assert!(Posture::read(&mut Reader::new(&[9, 0])).is_err());
    }

    fn frame(opcode: u16, body: &[u8]) -> crate::game::ServerMsg {
        let mut f = vec![0xDD, 0x01, 0, 0];
        f.extend(opcode.to_le_bytes());
        f.extend(body);
        crate::game::ServerMsg::decode(&f).unwrap()
    }

    /// The 2026-10-28 live capture (`s3/native.csv`) logged every `SCUnitAttached` S->C only as opcode + byte
    /// count: 14 bytes, a 6-byte header and an 8-byte body. The 1.2 reader `FUN_397c60c0` (with `FUN_397c03b0`)
    /// proves the body order `bc child`, `u8 point`, `bc parent`, `u8 reason`; this test proves the reader layout
    /// only, not the reply's reason value.
    #[test]
    fn the_captured_unit_attached_is_fourteen_bytes_with_the_mount_reason() {
        use crate::game::ServerMsg as M;
        use crate::opcodes::sc;
        for (point, reason) in [(1u8, 1u8), (1, 2), (1, 3), (1, 4)] {
            let mut w = Writer::new();
            w.bc(0x004D_352C).u8(point).bc(0x004D_3500).u8(reason);
            let body = w.finish();
            assert_eq!(body.len(), 8, "bc + u8 + bc + u8");
            let mut f = vec![0xDD, 0x01, 0x0E, 0x00];
            f.extend(sc::SCUnitAttachedPacket.to_le_bytes());
            f.extend(&body);
            assert_eq!(f.len(), 14, "the captured frame's byte count");
            assert_eq!(
                crate::game::ServerMsg::decode(&f).unwrap(),
                M::UnitAttached {
                    obj_id: 0x004D_352C,
                    mount: 0x004D_3500,
                    point,
                    reason
                }
            );
        }
    }

    #[test]
    fn the_packets_decode_from_frames() {
        use crate::game::ServerMsg as M;
        use crate::opcodes::sc;
        // `SCCombatFirstHitPacket { victim: 1, hitter: 2, hit_type: 0 }` of the server's `small_packets` test
        assert_eq!(
            frame(sc::SCCombatFirstHitPacket, &[1, 0, 0, 2, 0, 0, 0, 0, 0, 0]),
            M::CombatFirstHit { victim: 1, hitter: 2, hit_type: 0 }
        );
        // `SCAggroTargetChangedPacket { npc_id: 1, target_id: 0 }`: two `bc` ids, seen after Sunder Earth / Play Dead on the lane
        assert_eq!(frame(sc::SCAggroTargetChangedPacket, &[1, 0, 0, 0, 0, 0]), M::AggroTargetChanged { npc: 1, target: 0 });
        // `mount_and_dismount_flow`: the owner mounts (child bc, driver point 1, mount bc, reason 1), then dismounts (bc, reason 4)
        let mut w = Writer::new();
        w.bc(0x3DF6).u8(1).bc(0x4E21).u8(1);
        assert_eq!(
            frame(sc::SCUnitAttachedPacket, &w.finish()),
            M::UnitAttached { obj_id: 0x3DF6, mount: 0x4E21, point: 1, reason: 1 }
        );
        let mut w = Writer::new();
        w.bc(0x3DF6).u8(4);
        assert_eq!(frame(sc::SCUnitDetachedPacket, &w.finish()), M::UnitDetached { obj_id: 0x3DF6, reason: 4 });
        // the 5 byte posture packet the server sends all the time, and a pose
        assert_eq!(
            frame(sc::SCUnitModelPostureChangedPacket, &[0x56, 0x34, 0x12, 0, 0]),
            M::UnitPosture { obj_id: 0x12_3456, posture: Posture::default() }
        );
        let mut w = Writer::new();
        w.bc(7).u8(4).bool(false).u32(104).bool(true);
        assert_eq!(
            frame(sc::SCUnitModelPostureChangedPacket, &w.finish()),
            M::UnitPosture { obj_id: 7, posture: Posture { kind: 4, has_loot_left: false, anim_action: 104, activate: true, house: None } }
        );
        let mut w = Writer::new();
        w.bc(0x99).i32(1).bc(5).i32(77).i32(0).i32(0).u8(135);
        assert_eq!(
            frame(sc::SCAiAggroPacket, &w.finish()),
            M::AiAggro { npc: 0x99, hostiles: vec![AggroEntry { hostile: 5, damage: 77, val2: 0, val3: 0, flags: 135 }] }
        );
        let mut w = Writer::new();
        w.u16(1).bc(0x42).raw(&unit_bytes(0, 0x04));
        let M::UnitMovements(list) = frame(sc::SCUnitMovementsPacket, &w.finish()) else { panic!("not a batch") };
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, 0x42);
    }
}

/// Raw SCUnitGmModeChanged (0x203), including unknown signed mode indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitGmModeChanged {
    pub obj_id: u32,
    pub mode: i32,
    pub value: u8,
}
impl UnitGmModeChanged {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { obj_id: r.bc()?, mode: r.i32()?, value: r.u8()? })
    }
}

/// SCUnitPvPPointsChanged (0x204): `bc unit, u8 kind, i32 point`. Kind 0 is the honor gained in
/// combat, kind 1 the hostile faction kills (the server's port of the C# `CharacterCombat`; not
/// checked against the retail client).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitPvpPointsChanged {
    pub obj_id: u32,
    pub kind: u8,
    pub point: i32,
}
impl UnitPvpPointsChanged {
    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self { obj_id: r.bc()?, kind: r.u8()?, point: r.i32()? })
    }
}

#[cfg(test)]
mod pvp_points_tests {
    use super::*;
    use crate::{game::ServerMsg, session::Event, state::{tests::unit, WorldState}, wire::Writer};
    #[test]
    fn decodes_the_servers_byte_test_packet() {
        let body = [5u8, 0, 0, 2, 10, 0, 0, 0];
        let mut r = Reader::new(&body);
        assert_eq!(
            UnitPvpPointsChanged::read(&mut r).unwrap(),
            UnitPvpPointsChanged { obj_id: 5, kind: 2, point: 10 }
        );
        assert!(r.is_empty());
        let mut w = Writer::new();
        w.bc(0xabcdef).u8(1).i32(-3);
        let body = w.finish();
        for end in 0..body.len() {
            assert!(UnitPvpPointsChanged::read(&mut Reader::new(&body[..end])).is_err());
        }
        let _ = ServerMsg::UnitPvpPointsChanged(UnitPvpPointsChanged { obj_id: 1, kind: 0, point: 0 });
    }
    #[test]
    fn updates_the_unit_values() {
        let mut world = WorldState::default();
        let ev = |obj_id, kind, point| Event::UnitPvpPointsChanged(UnitPvpPointsChanged { obj_id, kind, point });
        world.apply(&Event::UnitSpawned(Box::new(unit(8))));
        assert!(world.units.apply(&ev(8, 0, 120)));
        assert!(world.units.apply(&ev(8, 1, 3)));
        assert!(!world.units.apply(&ev(8, 1, 3)));
        assert!(!world.units.apply(&ev(8, 9, 3)));
        assert_eq!((world.units.by_id[&8].pvp_honor, world.units.by_id[&8].pvp_kills), (120, 3));
    }
}

#[cfg(test)]
mod gm_mode_tests {
    use super::*;
    use crate::{game::{ServerMsg,MAGIC},opcodes::sc,wire::Writer};
    #[test]
    fn native_lane_gm_mode_retains_signed_mode_and_raw_byte_at_exact_packet_boundary() {
        for mode in [i32::MIN,-1,0,1,2,3,4,i32::MAX] {
            let mut w=Writer::new(); w.bc(0xabcdef).i32(mode).u8(255);
            let body=w.finish();
            for end in 0..body.len() { assert!(UnitGmModeChanged::read(&mut Reader::new(&body[..end])).is_err()); }
            let mut r=Reader::new(&body);
            assert_eq!(UnitGmModeChanged::read(&mut r).unwrap(),UnitGmModeChanged{obj_id:0xabcdef,mode,value:255});
            assert!(r.is_empty());
            let mut raw=vec![0,0];
            raw.extend(sc::SCUnitGmModeChangedPacket.to_le_bytes()); raw.extend(&body);
            raw.extend([0,0]); raw.extend(sc::SCSkillEndedPacket.to_le_bytes()); raw.extend(77u16.to_le_bytes());
            let mut batch=vec![MAGIC,4,2,0,1]; // final stored DEFLATE block
            let n=raw.len() as u16; batch.extend(n.to_le_bytes()); batch.extend((!n).to_le_bytes()); batch.extend(raw);
            let ServerMsg::Batch(events)=ServerMsg::decode(&batch).unwrap() else {panic!("not batch")};
            assert_eq!(events.len(),2);
            assert!(matches!(events[0],ServerMsg::UnitGmModeChanged(UnitGmModeChanged{obj_id:0xabcdef,mode:m,value:255}) if m==mode));
            assert_eq!(events[1],ServerMsg::SkillEnded{tl:77});
        }
    }
    #[test]
    fn mode_updates_existing_player_and_units_only_and_snapshot_replaces_old_flags() {
        use crate::{session::{Event,PlayerSpawn},state::{WorldState,tests::unit}};
        let mut world=WorldState::default();
        let event=|obj_id,mode,value| Event::UnitGmModeChanged(UnitGmModeChanged{obj_id,mode,value});
        world.apply(&event(7,1,255)); assert!(world.units.by_id.is_empty()); assert!(world.player.spawn.is_none());
        world.apply(&Event::PlayerSpawn(Box::new(PlayerSpawn{obj_id:7,state:Default::default(),unit:unit(7)})));
        world.apply(&Event::UnitSpawned(Box::new(unit(8))));
        for index in 0..4 { world.apply(&event(7,index,(index+1) as u8)); world.apply(&event(8,index,(index+5) as u8)); }
        assert_eq!(world.player.spawn.as_ref().unwrap().unit.gm_modes,[1,2,3,4]);
        assert_eq!(world.units.by_id[&8].gm_modes,[5,6,7,8]);
        for mode in [-1,4,i32::MIN,i32::MAX] { world.apply(&event(8,mode,255)); }
        assert_eq!(world.units.by_id[&8].gm_modes,[5,6,7,8]);
        assert!(!world.units.apply(&event(8,1,6)),"identical byte reports remain available as events but do not mutate state");
        world.apply(&Event::UnitsRemoved(vec![8])); world.apply(&event(8,1,99)); assert!(!world.units.by_id.contains_key(&8));
        world.apply(&Event::UnitSpawned(Box::new(unit(8)))); assert_eq!(world.units.by_id[&8].gm_modes,[0;4]);
        world.apply(&Event::LeftWorld{target:0}); assert!(world.units.by_id.is_empty() && world.player.spawn.is_none());
    }

}
