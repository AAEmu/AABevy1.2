//! `MoveUnit` (0x130 bytes) and `FUN_397c0470`, the move serializer/deserializer
//! for types 0–5. Also `FUN_39024610`, the field-wise copy used by the
//! movement queue.

use super::codec::{
    DEFAULT_CELL_SIZE, FALL_VEL_MAX, FALL_VEL_SCALE, FixedPos, VEL_SCALE_TYPE0, VEL_SCALE_TYPE1,
    VEL_SCALE_VEHICLE, read_delta_i8_vec3, read_i16_vec3, read_position, read_quat_i8,
    read_quat_i16, write_delta_i8_vec3, write_i16_vec3, write_position, write_quat_i8,
    write_quat_i16,
};
use crate::wire::Writer;

pub const MOVE_UNIT_SIZE: usize = 0x130;
pub const MAX_WHEELS: usize = 18;

/// The 1.2 move struct. It is kept as raw bytes so the queue slot size, the
/// union tails and `FUN_39024610`'s field copy all stay exact.
#[derive(Clone, PartialEq)]
pub struct MoveUnit {
    pub bytes: [u8; MOVE_UNIT_SIZE],
}

impl Default for MoveUnit {
    fn default() -> Self {
        Self {
            bytes: [0u8; MOVE_UNIT_SIZE],
        }
    }
}

impl core::fmt::Debug for MoveUnit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MoveUnit")
            .field("type", &self.move_type())
            .field("time", &self.time())
            .field("flags", &self.flags())
            .finish_non_exhaustive()
    }
}

impl MoveUnit {
    pub fn move_type(&self) -> u8 {
        self.bytes[0x00]
    }
    pub fn set_move_type(&mut self, v: u8) {
        self.bytes[0x00] = v;
    }
    pub fn time(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x04..0x08].try_into().unwrap())
    }
    pub fn set_time(&mut self, v: u32) {
        self.bytes[0x04..0x08].copy_from_slice(&v.to_le_bytes());
    }
    pub fn fixed_pos(&self) -> FixedPos {
        FixedPos {
            x: i64::from_le_bytes(self.bytes[0x08..0x10].try_into().unwrap()),
            y: i64::from_le_bytes(self.bytes[0x10..0x18].try_into().unwrap()),
            z: f32::from_le_bytes(self.bytes[0x18..0x1c].try_into().unwrap()),
        }
    }
    pub fn set_fixed_pos(&mut self, p: FixedPos) {
        self.bytes[0x08..0x10].copy_from_slice(&p.x.to_le_bytes());
        self.bytes[0x10..0x18].copy_from_slice(&p.y.to_le_bytes());
        self.bytes[0x18..0x1c].copy_from_slice(&p.z.to_le_bytes());
    }
    pub fn second_fixed_pos(&self) -> FixedPos {
        FixedPos {
            x: i64::from_le_bytes(self.bytes[0x68..0x70].try_into().unwrap()),
            y: i64::from_le_bytes(self.bytes[0x70..0x78].try_into().unwrap()),
            z: f32::from_le_bytes(self.bytes[0x78..0x7c].try_into().unwrap()),
        }
    }
    pub fn set_second_fixed_pos(&mut self, p: FixedPos) {
        self.bytes[0x68..0x70].copy_from_slice(&p.x.to_le_bytes());
        self.bytes[0x70..0x78].copy_from_slice(&p.y.to_le_bytes());
        self.bytes[0x78..0x7c].copy_from_slice(&p.z.to_le_bytes());
    }
    pub fn rotation(&self) -> [f32; 4] {
        [
            f32::from_le_bytes(self.bytes[0x20..0x24].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x24..0x28].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x28..0x2c].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x2c..0x30].try_into().unwrap()),
        ]
    }
    pub fn set_rotation(&mut self, q: [f32; 4]) {
        for (i, v) in q.iter().enumerate() {
            let o = 0x20 + i * 4;
            self.bytes[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    pub fn second_rotation(&self) -> [f32; 4] {
        [
            f32::from_le_bytes(self.bytes[0x80..0x84].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x84..0x88].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x88..0x8c].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x8c..0x90].try_into().unwrap()),
        ]
    }
    pub fn set_second_rotation(&mut self, q: [f32; 4]) {
        for (i, v) in q.iter().enumerate() {
            let o = 0x80 + i * 4;
            self.bytes[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
    pub fn velocity(&self) -> [f32; 3] {
        [
            f32::from_le_bytes(self.bytes[0x30..0x34].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x34..0x38].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x38..0x3c].try_into().unwrap()),
        ]
    }
    pub fn set_velocity(&mut self, v: [f32; 3]) {
        for (i, x) in v.iter().enumerate() {
            let o = 0x30 + i * 4;
            self.bytes[o..o + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    pub fn flags(&self) -> u8 {
        self.bytes[0x3c]
    }
    pub fn set_flags(&mut self, v: u8) {
        self.bytes[0x3c] = v;
    }
    pub fn sc_type(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x40..0x44].try_into().unwrap())
    }
    pub fn set_sc_type(&mut self, v: u32) {
        self.bytes[0x40..0x44].copy_from_slice(&v.to_le_bytes());
    }
    pub fn phase(&self) -> u8 {
        self.bytes[0x44]
    }
    pub fn set_phase(&mut self, v: u8) {
        self.bytes[0x44] = v;
    }
    pub fn delta_movement(&self) -> [f32; 3] {
        [
            f32::from_le_bytes(self.bytes[0x48..0x4c].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x4c..0x50].try_into().unwrap()),
            f32::from_le_bytes(self.bytes[0x50..0x54].try_into().unwrap()),
        ]
    }
    pub fn set_delta_movement(&mut self, v: [f32; 3]) {
        for (i, x) in v.iter().enumerate() {
            let o = 0x48 + i * 4;
            self.bytes[o..o + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    pub fn fall_vel(&self) -> f32 {
        f32::from_le_bytes(self.bytes[0x54..0x58].try_into().unwrap())
    }
    pub fn set_fall_vel(&mut self, v: f32) {
        self.bytes[0x54..0x58].copy_from_slice(&v.to_le_bytes());
    }
    pub fn stance(&self) -> u8 {
        self.bytes[0x58]
    }
    pub fn set_stance(&mut self, v: u8) {
        self.bytes[0x58] = v;
    }
    pub fn alertness(&self) -> u8 {
        self.bytes[0x59]
    }
    pub fn set_alertness(&mut self, v: u8) {
        self.bytes[0x59] = v;
    }
    pub fn actor_flags(&self) -> u8 {
        self.bytes[0x5a]
    }
    pub fn set_actor_flags(&mut self, v: u8) {
        self.bytes[0x5a] = v;
    }
    pub fn gc_flags(&self) -> u8 {
        self.bytes[0x5b]
    }
    pub fn set_gc_flags(&mut self, v: u8) {
        self.bytes[0x5b] = v;
    }
    pub fn gc_part_id(&self) -> u16 {
        u16::from_le_bytes(self.bytes[0x5c..0x5e].try_into().unwrap())
    }
    pub fn set_gc_part_id(&mut self, v: u16) {
        self.bytes[0x5c..0x5e].copy_from_slice(&v.to_le_bytes());
    }
    pub fn gc_id(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x60..0x64].try_into().unwrap())
    }
    pub fn set_gc_id(&mut self, v: u32) {
        self.bytes[0x60..0x64].copy_from_slice(&v.to_le_bytes());
    }
    pub fn climb_data(&self) -> u32 {
        u32::from_le_bytes(self.bytes[0x90..0x94].try_into().unwrap())
    }
    pub fn set_climb_data(&mut self, v: u32) {
        self.bytes[0x90..0x94].copy_from_slice(&v.to_le_bytes());
    }

    pub fn vehicle_ang_vel(&self) -> [f32; 3] {
        read_vec3(&self.bytes, 0x98)
    }
    pub fn set_vehicle_ang_vel(&mut self, v: [f32; 3]) {
        write_vec3(&mut self.bytes, 0x98, v);
    }
    pub fn vehicle_steering(&self) -> f32 {
        read_f32(&self.bytes, 0xa4)
    }
    pub fn set_vehicle_steering(&mut self, v: f32) {
        write_f32(&mut self.bytes, 0xa4, v);
    }
    pub fn wheel_count(&self) -> u8 {
        self.bytes[0xa8]
    }
    pub fn set_wheel_count(&mut self, v: u8) {
        self.bytes[0xa8] = v;
    }
    pub fn wheel_ang_vel(&self, i: usize) -> f32 {
        read_f32(&self.bytes, 0xac + i * 4)
    }
    pub fn set_wheel_ang_vel(&mut self, i: usize, v: f32) {
        write_f32(&mut self.bytes, 0xac + i * 4, v);
    }

    pub fn ship_ang_vel(&self) -> [f32; 3] {
        read_vec3(&self.bytes, 0xf4)
    }
    pub fn set_ship_ang_vel(&mut self, v: [f32; 3]) {
        write_vec3(&mut self.bytes, 0xf4, v);
    }
    pub fn ship_steering(&self) -> i8 {
        self.bytes[0x100] as i8
    }
    pub fn set_ship_steering(&mut self, v: i8) {
        self.bytes[0x100] = v as u8;
    }
    pub fn ship_throttle(&self) -> i8 {
        self.bytes[0x101] as i8
    }
    pub fn set_ship_throttle(&mut self, v: i8) {
        self.bytes[0x101] = v as u8;
    }
    pub fn ship_zone_id(&self) -> u16 {
        u16::from_le_bytes(self.bytes[0x102..0x104].try_into().unwrap())
    }
    pub fn set_ship_zone_id(&mut self, v: u16) {
        self.bytes[0x102..0x104].copy_from_slice(&v.to_le_bytes());
    }
    pub fn ship_stuck(&self) -> u8 {
        self.bytes[0x104]
    }
    pub fn set_ship_stuck(&mut self, v: u8) {
        self.bytes[0x104] = v;
    }

    pub fn request_throttle(&self) -> i8 {
        self.bytes[0x108] as i8
    }
    pub fn set_request_throttle(&mut self, v: i8) {
        self.bytes[0x108] = v as u8;
    }
    pub fn request_steering(&self) -> i8 {
        self.bytes[0x109] as i8
    }
    pub fn set_request_steering(&mut self, v: i8) {
        self.bytes[0x109] = v as u8;
    }

    pub fn transfer_ang_vel(&self) -> [f32; 3] {
        read_vec3(&self.bytes, 0x10c)
    }
    pub fn set_transfer_ang_vel(&mut self, v: [f32; 3]) {
        write_vec3(&mut self.bytes, 0x10c, v);
    }
    pub fn transfer_steering(&self) -> i32 {
        i32::from_le_bytes(self.bytes[0x118..0x11c].try_into().unwrap())
    }
    pub fn set_transfer_steering(&mut self, v: i32) {
        self.bytes[0x118..0x11c].copy_from_slice(&v.to_le_bytes());
    }
    pub fn transfer_path_point(&self) -> i32 {
        i32::from_le_bytes(self.bytes[0x11c..0x120].try_into().unwrap())
    }
    pub fn set_transfer_path_point(&mut self, v: i32) {
        self.bytes[0x11c..0x120].copy_from_slice(&v.to_le_bytes());
    }
    pub fn transfer_speed(&self) -> f32 {
        read_f32(&self.bytes, 0x120)
    }
    pub fn set_transfer_speed(&mut self, v: f32) {
        write_f32(&mut self.bytes, 0x120, v);
    }
    pub fn transfer_reverse(&self) -> u8 {
        self.bytes[0x124]
    }
    pub fn set_transfer_reverse(&mut self, v: u8) {
        self.bytes[0x124] = v;
    }
}

#[inline]
fn read_f32(b: &[u8], off: usize) -> f32 {
    f32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
#[inline]
fn write_f32(b: &mut [u8], off: usize, v: f32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
#[inline]
fn read_vec3(b: &[u8], off: usize) -> [f32; 3] {
    [read_f32(b, off), read_f32(b, off + 4), read_f32(b, off + 8)]
}
#[inline]
fn write_vec3(b: &mut [u8], off: usize, v: [f32; 3]) {
    write_f32(b, off, v[0]);
    write_f32(b, off + 4, v[1]);
    write_f32(b, off + 8, v[2]);
}

/// The `Serialization::IArchive` surface used by `FUN_397c0470` / `FUN_3961e2c0`.
pub trait MoveArchive {
    fn is_reading(&self) -> bool;
    fn u8(&mut self, key: &'static str, v: &mut u8);
    fn u16(&mut self, key: &'static str, v: &mut u16);
    fn u32(&mut self, key: &'static str, v: &mut u32);
    fn i8(&mut self, key: &'static str, v: &mut i8);
    fn i16(&mut self, key: &'static str, v: &mut i16);
    fn i32(&mut self, key: &'static str, v: &mut i32);
    fn f32(&mut self, key: &'static str, v: &mut f32);
    fn f32x3(&mut self, key: &'static str, v: &mut [f32; 3]);
    fn bool8(&mut self, key: &'static str, v: &mut u8) {
        self.u8(key, v);
    }
}

/// `FUN_3961e2c0`: the 9-byte position codec, called by the move serializer.
pub fn position_codec(pos: &mut FixedPos, ar: &mut dyn MoveArchive, cell_size: f32) {
    if ar.is_reading() {
        let mut b = [0u8; 9];
        const KEYS: [&str; 9] = [
            "ix0", "ix1", "ix2", "iy0", "iy1", "iy2", "iz0", "iz1", "iz2",
        ];
        for (k, v) in KEYS.iter().zip(b.iter_mut()) {
            ar.u8(k, v);
        }
        *pos = read_position(&b, cell_size);
    } else {
        let b = write_position(*pos, cell_size);
        const KEYS: [&str; 9] = [
            "ix0", "ix1", "ix2", "iy0", "iy1", "iy2", "iz0", "iz1", "iz2",
        ];
        for (k, v) in KEYS.iter().zip(b.iter()) {
            let mut byte = *v;
            ar.u8(k, &mut byte);
        }
    }
}

fn write_vec3_i16(ar: &mut dyn MoveArchive, mv: &mut MoveUnit, write_scale: f32, read_scale: f32) {
    let reading = ar.is_reading();
    let mut raw = [0i16; 3];
    if !reading {
        raw = write_i16_vec3(mv.velocity(), write_scale);
    }
    ar.i16("vel.x", &mut raw[0]);
    ar.i16("vel.y", &mut raw[1]);
    ar.i16("vel.z", &mut raw[2]);
    if reading {
        mv.set_velocity(read_i16_vec3(raw, read_scale));
    }
}

fn write_quat_i16_field(ar: &mut dyn MoveArchive, mv: &mut MoveUnit) {
    let reading = ar.is_reading();
    let mut raw = [0i16; 3];
    if !reading {
        raw = write_quat_i16(mv.rotation());
    }
    ar.i16("rot.x", &mut raw[0]);
    ar.i16("rot.y", &mut raw[1]);
    ar.i16("rot.z", &mut raw[2]);
    if reading {
        mv.set_rotation(read_quat_i16(raw));
    }
}

fn write_quat_i8_field(ar: &mut dyn MoveArchive, mv: &mut MoveUnit) {
    let reading = ar.is_reading();
    let mut raw = [0i8; 3];
    if !reading {
        raw = write_quat_i8(mv.rotation());
    }
    ar.i8("rot.x", &mut raw[0]);
    ar.i8("rot.y", &mut raw[1]);
    ar.i8("rot.z", &mut raw[2]);
    if reading {
        mv.set_rotation(read_quat_i8(raw));
    }
}

fn write_delta_field(ar: &mut dyn MoveArchive, mv: &mut MoveUnit) {
    let reading = ar.is_reading();
    let mut raw = [0i8; 3];
    if !reading {
        raw = write_delta_i8_vec3(mv.delta_movement());
    }
    ar.i8("actor.deltaMovement.x", &mut raw[0]);
    ar.i8("actor.deltaMovement.y", &mut raw[1]);
    ar.i8("actor.deltaMovement.z", &mut raw[2]);
    if reading {
        mv.set_delta_movement(read_delta_i8_vec3(raw));
    }
}

/// `FUN_397c0470`: serialize or deserialize a `MoveUnit`.
pub fn serialize_move(mv: &mut MoveUnit, ar: &mut dyn MoveArchive) {
    let mut move_type = mv.move_type();
    ar.u8("type", &mut move_type);
    mv.set_move_type(move_type);

    let mut time = mv.time();
    ar.u32("time", &mut time);
    mv.set_time(time);

    let mut flags = mv.flags();
    ar.u8("flags", &mut flags);
    mv.set_flags(flags);

    if flags & 0x10 != 0 {
        let mut sc_type = mv.sc_type();
        ar.u32("scType", &mut sc_type);
        mv.set_sc_type(sc_type);
        let mut phase = mv.phase();
        ar.u8("phase", &mut phase);
        mv.set_phase(phase);
    }

    match move_type {
        1 => {
            let mut pos = mv.fixed_pos();
            position_codec(&mut pos, ar, DEFAULT_CELL_SIZE);
            mv.set_fixed_pos(pos);

            write_vec3_i16(ar, mv, VEL_SCALE_TYPE1, super::codec::VEL_TYPE1);
            write_quat_i8_field(ar, mv);
            write_delta_field(ar, mv);

            let mut stance = mv.stance();
            ar.u8("actor.stance", &mut stance);
            mv.set_stance(stance);
            let mut alertness = mv.alertness();
            ar.u8("actor.alertness", &mut alertness);
            mv.set_alertness(alertness);
            let mut actor_flags = mv.actor_flags();
            ar.u8("actor.flags", &mut actor_flags);
            mv.set_actor_flags(actor_flags);

            if actor_flags & 0x80 != 0 {
                let reading = ar.is_reading();
                let mut raw = 0u16;
                if !reading {
                    let f = mv.fall_vel() * FALL_VEL_SCALE;
                    let clamped = if f < 0.0 {
                        0.0
                    } else if f < 1.0 {
                        f
                    } else {
                        1.0
                    };
                    raw = super::codec::round_half_floor_f64(clamped * 65535.0 + 0.5) as u16;
                }
                ar.u16("actor.fallVel", &mut raw);
                mv.set_fall_vel(raw as f32 / 65535.0 * FALL_VEL_MAX);
            }
            if actor_flags & 0x20 != 0 {
                let mut gc_flags = mv.gc_flags();
                ar.u8("actor.gcFlags", &mut gc_flags);
                mv.set_gc_flags(gc_flags);
                let mut part = mv.gc_part_id();
                ar.u16("actor.gcPartId", &mut part);
                mv.set_gc_part_id(part);

                let mut pos = mv.second_fixed_pos();
                position_codec(&mut pos, ar, DEFAULT_CELL_SIZE);
                mv.set_second_fixed_pos(pos);

                let reading = ar.is_reading();
                let mut raw = [0i8; 3];
                if !reading {
                    // `FUN_397adca0` reads the second quaternion at +0x80
                    // (0x397c08b4).
                    raw = write_quat_i8(mv.second_rotation());
                }
                ar.i8("rot.x", &mut raw[0]);
                ar.i8("rot.y", &mut raw[1]);
                ar.i8("rot.z", &mut raw[2]);
                if reading {
                    mv.set_second_rotation(read_quat_i8(raw));
                }
            }
            if actor_flags & 0x60 != 0 {
                let mut gc_id = mv.gc_id();
                ar.u32("actor.gcId", &mut gc_id);
                mv.set_gc_id(gc_id);
            }
            if actor_flags & 0x40 != 0 {
                let mut climb = mv.climb_data();
                ar.u32("actor.climbData", &mut climb);
                mv.set_climb_data(climb);
            }
        }
        2 => {
            let mut pos = mv.fixed_pos();
            position_codec(&mut pos, ar, DEFAULT_CELL_SIZE);
            mv.set_fixed_pos(pos);

            write_vec3_i16(ar, mv, VEL_SCALE_VEHICLE, super::codec::VEL_VEHICLE);
            write_quat_i16_field(ar, mv);

            let mut ang = mv.vehicle_ang_vel();
            ar.f32x3("vehicle.angVel", &mut ang);
            mv.set_vehicle_ang_vel(ang);

            let mut steering = mv.vehicle_steering();
            ar.f32("vehicle.steering", &mut steering);
            mv.set_vehicle_steering(steering);

            let mut count = mv.wheel_count();
            ar.u8("vehicle.wheelVelCount", &mut count);
            mv.set_wheel_count(count);
            let n = (count as usize).min(MAX_WHEELS);
            for i in 0..n {
                let mut w = mv.wheel_ang_vel(i);
                ar.f32("vehicle.wheelAngVel", &mut w);
                mv.set_wheel_ang_vel(i, w);
            }
        }
        3 => {
            let mut pos = mv.fixed_pos();
            position_codec(&mut pos, ar, DEFAULT_CELL_SIZE);
            mv.set_fixed_pos(pos);

            write_vec3_i16(ar, mv, VEL_SCALE_VEHICLE, super::codec::VEL_VEHICLE);
            write_quat_i16_field(ar, mv);

            let mut ang = mv.ship_ang_vel();
            ar.f32x3("ship.angVel", &mut ang);
            mv.set_ship_ang_vel(ang);

            let mut steering = mv.ship_steering();
            ar.i8("ship.steering", &mut steering);
            mv.set_ship_steering(steering);
            let mut throttle = mv.ship_throttle();
            ar.i8("ship.throttle", &mut throttle);
            mv.set_ship_throttle(throttle);
            let mut zone = mv.ship_zone_id();
            ar.u16("ship.zoneId", &mut zone);
            mv.set_ship_zone_id(zone);
            let mut stuck = mv.ship_stuck();
            ar.bool8("ship.stucked", &mut stuck);
            mv.set_ship_stuck(stuck);
        }
        4 => {
            let mut throttle = mv.request_throttle();
            ar.i8("shipRequest.throttle", &mut throttle);
            mv.set_request_throttle(throttle);
            let mut steering = mv.request_steering();
            ar.i8("shipRequest.steering", &mut steering);
            mv.set_request_steering(steering);
        }
        5 => {
            let mut pos = mv.fixed_pos();
            position_codec(&mut pos, ar, DEFAULT_CELL_SIZE);
            mv.set_fixed_pos(pos);

            write_vec3_i16(ar, mv, VEL_SCALE_VEHICLE, super::codec::VEL_VEHICLE);
            write_quat_i16_field(ar, mv);

            let mut ang = mv.transfer_ang_vel();
            ar.f32x3("transfer.angVel", &mut ang);
            mv.set_transfer_ang_vel(ang);

            let mut steering = mv.transfer_steering();
            ar.i32("transfer.steering", &mut steering);
            mv.set_transfer_steering(steering);
            let mut path = mv.transfer_path_point();
            ar.i32("transfer.pathPointIndex", &mut path);
            mv.set_transfer_path_point(path);
            let mut speed = mv.transfer_speed();
            ar.f32("transfer.speed", &mut speed);
            mv.set_transfer_speed(speed);
            let mut reverse = mv.transfer_reverse();
            ar.bool8("transfer.reverse", &mut reverse);
            mv.set_transfer_reverse(reverse);
        }
        _ => {
            let mut pos = mv.fixed_pos();
            position_codec(&mut pos, ar, DEFAULT_CELL_SIZE);
            mv.set_fixed_pos(pos);

            write_vec3_i16(ar, mv, VEL_SCALE_TYPE0, super::codec::VEL_TYPE0);
            write_quat_i16_field(ar, mv);
        }
    }
}

/// Writes `FUN_397c0470`'s fields to a game packet body. The archive keys are
/// the decompile's; only the read path uses them.
struct WriterArchive<'a> {
    writer: &'a mut Writer,
}

impl MoveArchive for WriterArchive<'_> {
    fn is_reading(&self) -> bool {
        false
    }
    fn u8(&mut self, _key: &'static str, v: &mut u8) {
        self.writer.u8(*v);
    }
    fn u16(&mut self, _key: &'static str, v: &mut u16) {
        self.writer.u16(*v);
    }
    fn u32(&mut self, _key: &'static str, v: &mut u32) {
        self.writer.u32(*v);
    }
    fn i8(&mut self, _key: &'static str, v: &mut i8) {
        self.writer.i8(*v);
    }
    fn i16(&mut self, _key: &'static str, v: &mut i16) {
        self.writer.i16(*v);
    }
    fn i32(&mut self, _key: &'static str, v: &mut i32) {
        self.writer.i32(*v);
    }
    fn f32(&mut self, _key: &'static str, v: &mut f32) {
        self.writer.f32(*v);
    }
    fn f32x3(&mut self, _key: &'static str, v: &mut [f32; 3]) {
        for x in v {
            self.writer.f32(*x);
        }
    }
}

impl MoveUnit {
    /// Serialize this body with `FUN_397c0470`, without the `bc` object id.
    pub fn write_to(&self, w: &mut Writer) {
        let mut mv = self.clone();
        serialize_move(&mut mv, &mut WriterArchive { writer: w });
    }
}

/// `FUN_39024610`: the field-wise move copy. The original leaves a few
/// padding gaps untouched: +0x3d..+0x40, +0x5e..+0x60, +0x64..+0x68,
/// +0x94..+0x98, +0x10a..+0x10c and +0x128..+0x130.
pub fn copy_move(dst: &mut MoveUnit, src: &MoveUnit) {
    let ranges = [
        (0x00, 0x3d),
        (0x40, 0x44),
        (0x44, 0x48),
        (0x48, 0x5e),
        (0x60, 0x64),
        (0x68, 0x94),
        (0x98, 0xf4),
        (0xf4, 0x10a),
        (0x10c, 0x128),
    ];
    for (start, end) in ranges {
        dst.bytes[start..end].copy_from_slice(&src.bytes[start..end]);
    }
}
