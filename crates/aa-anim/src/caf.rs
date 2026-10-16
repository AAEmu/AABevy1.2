//! `.caf` animation clips: per-bone rotation / position tracks.
//!
//! A `.caf` is a CryEngine chunk file (`0xFFFF0001`, container 0x744 or 0x745) holding
//!
//! * one **Timing** chunk (`0xCCCC000E`, v0x918): `f32 secs_per_tick, i32 ticks_per_frame,
//!   char name[32], i32 start_frame, i32 end_frame` (always 1/4800 s per tick, 160 ticks per
//!   frame = 30 fps);
//! * one **Controller** chunk (`0xCCCC000D`) per animated bone; the controller id is the CRC-32 of
//!   the case-sensitive bone name ([`aa_cry::skeleton::controller_id`]);
//! * `0xAAFC0002` / `0xAAFC0003` (motion parameters, foot plants): ignored.
//!
//! Controller chunk **v0x829** (25,045 of the 25,202 `.caf` in the 1.2 pak, every player,
//! NPC and animal clip), after the 16-byte header copy:
//!
//! ```text
//! +0  u32 controller_id
//! +4  u16 num_rot_keys        +6  u16 num_pos_keys
//! +8  u8  rot_format          +9  u8  rot_time_format
//! +10 u8  pos_format          +11 u8  pos_keys_info (1 = own pos times, 0 = share rot times)
//! +12 u8  pos_time_format     +13 u8  tracks_aligned     (+14..16 padding)
//! +16 rot keys, rot times, [pos keys, pos times]
//! ```
//!
//! Rotation formats: 1 = `f32 x,y,z,w`; 5 = 48-bit "small tree" quaternion; 8 = 64-bit extended
//! small tree quaternion. A small-tree quaternion stores the three smallest components
//! quantised to `[-1/sqrt2, 1/sqrt2]` plus the 2-bit index of the largest one in the top bits
//! (48 bit: 3x15 bits, index at bits 46-47; 64 bit: 21+21+20 bits, index at bits 62-63, lowest
//! bits first, in component order x,y,z,w skipping the largest); the largest is rebuilt from
//! unit length. Position format 2 = `f32 x,y,z` (metres, relative to the parent bone). Time
//! formats: 0 = `f32`, 1 = `u16`, 2 = `u8`, all in **frames**.
//!
//! Values are the bone's *absolute local transform* (relative to its parent, replacing the
//! bind pose), not a delta. A bone without a position track keeps its bind position. The root
//! `Bip01` is the identity at the origin in idle clips (root motion in locomotion clips).
//!
//! Older chunk versions (v0x827, v0x828; 399 clips: ships, glider, sealion ...) are skipped
//! and counted in [`Clip::skipped_controllers`].

use aa_cry::chunk::{ChunkFile, FILE_ANIMATION, Reader, ty};
use glam::{Quat, Vec3};

use crate::{Error, Result};

/// Frames per second of every clip in the pak (160 ticks of 1/4800 s).
pub const DEFAULT_FPS: f32 = 30.0;

/// Animation of one bone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    /// CRC-32 of the bone name.
    pub controller: u32,
    /// Seconds from the clip start, ascending.
    pub rot_times: Vec<f32>,
    pub rot: Vec<Quat>,
    pub pos_times: Vec<f32>,
    pub pos: Vec<Vec3>,
}

impl Track {
    /// Local rotation at time `t` (seconds, clamped to the key range); `None` if no rotation keys.
    pub fn sample_rot(&self, t: f32) -> Option<Quat> {
        let (i, j, f) = locate(&self.rot_times, t)?;
        let a = self.rot[i];
        let b = self.rot[j];
        if i == j || f <= 0.0 {
            return Some(a);
        }
        // shortest arc; keys are close together so a normalised lerp is indistinguishable from slerp
        let b = if a.dot(b) < 0.0 { -b } else { b };
        Some(a.lerp(b, f).normalize())
    }

    /// Local position at time `t`; `None` if no position keys.
    pub fn sample_pos(&self, t: f32) -> Option<Vec3> {
        let (i, j, f) = locate(&self.pos_times, t)?;
        Some(self.pos[i].lerp(self.pos[j], f))
    }
}

/// Key pair around `t`: `(i, j, fraction)`. `None` when there are no keys.
fn locate(times: &[f32], t: f32) -> Option<(usize, usize, f32)> {
    let n = times.len();
    if n == 0 {
        return None;
    }
    if n == 1 || t <= times[0] {
        return Some((0, 0, 0.0));
    }
    if t >= times[n - 1] {
        return Some((n - 1, n - 1, 0.0));
    }
    // first index with time > t
    let j = times.partition_point(|&k| k <= t).clamp(1, n - 1);
    let i = j - 1;
    let span = times[j] - times[i];
    let f = if span > 1e-9 { ((t - times[i]) / span).clamp(0.0, 1.0) } else { 0.0 };
    Some((i, j, f))
}

/// One decoded animation.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    pub name: String,
    pub fps: f32,
    /// Seconds.
    pub duration: f32,
    /// Sorted by controller id.
    pub tracks: Vec<Track>,
    /// Controller chunks of unsupported versions that were skipped.
    pub skipped_controllers: usize,
}

impl Clip {
    /// Decodes a `.caf` file.
    pub fn parse(bytes: &[u8]) -> Result<Clip> {
        let file = ChunkFile::parse(bytes)?;
        if file.file_type != FILE_ANIMATION {
            return Err(Error::Format(format!("file type {:#x} is not an animation", file.file_type)));
        }
        let mut clip = Clip { fps: DEFAULT_FPS, ..Default::default() };
        let mut start_frame = 0i32;
        let mut end_frame = 0i32;
        if let Some(t) = file.first(ty::TIMING) {
            let mut r = t.reader();
            let secs_per_tick = r.f32()?;
            let ticks_per_frame = r.i32()?;
            clip.name = r.fixed_str(32)?;
            start_frame = r.i32()?;
            end_frame = r.i32()?;
            let spf = secs_per_tick * ticks_per_frame as f32;
            if spf > 1e-6 && spf.is_finite() {
                clip.fps = 1.0 / spf;
            }
        }
        clip.duration = (end_frame - start_frame).max(0) as f32 / clip.fps;
        let frame_offset = start_frame as f32;
        for c in file.of_type(ty::CONTROLLER) {
            match c.version {
                0x829 => {
                    let track = decode_829(c.data, clip.fps, frame_offset)?;
                    clip.tracks.push(track);
                }
                _ => clip.skipped_controllers += 1,
            }
        }
        clip.tracks.sort_by_key(|t| t.controller);
        // Clips without a timing chunk: take the length from the keys.
        if clip.duration <= 0.0 {
            clip.duration = clip
                .tracks
                .iter()
                .flat_map(|t| [t.rot_times.last(), t.pos_times.last()])
                .flatten()
                .fold(0.0f32, |a, &b| a.max(b));
        }
        Ok(clip)
    }

    /// Loads and decodes a `.caf` from the global pak.
    pub fn load(path: &str) -> Result<Clip> {
        Clip::parse(&aa_cry::read(path)?)
    }

    pub fn track(&self, controller: u32) -> Option<&Track> {
        self.tracks.binary_search_by_key(&controller, |t| t.controller).ok().map(|i| &self.tracks[i])
    }
}

fn decode_829(data: &[u8], fps: f32, frame_offset: f32) -> Result<Track> {
    let mut r = Reader::new(data);
    let controller = r.u32()?;
    let num_rot = r.u16()? as usize;
    let num_pos = r.u16()? as usize;
    let rot_format = r.u8()?;
    let rot_time_format = r.u8()?;
    let pos_format = r.u8()?;
    let pos_keys_info = r.u8()?;
    let pos_time_format = r.u8()?;
    let aligned = r.u8()? != 0;
    r.seek(16);

    let align = |r: &mut Reader<'_>| {
        if aligned {
            let p = (r.pos() + 3) & !3;
            r.seek(p);
        }
    };

    let mut track = Track { controller, ..Default::default() };

    // rotations
    if num_rot > 0 {
        let stride = match rot_format {
            1 => 16,
            5 => 6,
            8 => 8,
            f => return Err(Error::Format(format!("controller {controller:#x}: rotation format {f}"))),
        };
        let raw = r.bytes(num_rot * stride)?;
        track.rot = raw
            .chunks_exact(stride)
            .map(|k| match rot_format {
                1 => Quat::from_xyzw(f(k, 0), f(k, 4), f(k, 8), f(k, 12)),
                5 => small_tree_48(k),
                _ => small_tree_64_ext(k),
            })
            .map(|q| if q.is_finite() && q.length_squared() > 0.5 { q.normalize() } else { Quat::IDENTITY })
            .collect();
        align(&mut r);
        track.rot_times = read_times(&mut r, num_rot, rot_time_format, fps, frame_offset)?;
        align(&mut r);
    }

    // positions
    if num_pos > 0 {
        if pos_format != 2 {
            return Err(Error::Format(format!("controller {controller:#x}: position format {pos_format}")));
        }
        let raw = r.bytes(num_pos * 12)?;
        track.pos = raw.chunks_exact(12).map(|k| Vec3::new(f(k, 0), f(k, 4), f(k, 8))).collect();
        align(&mut r);
        if pos_keys_info == 1 {
            track.pos_times = read_times(&mut r, num_pos, pos_time_format, fps, frame_offset)?;
        } else if num_pos == num_rot {
            track.pos_times = track.rot_times.clone();
        } else if num_pos == 1 {
            track.pos_times = vec![0.0];
        } else {
            return Err(Error::Format(format!(
                "controller {controller:#x}: {num_pos} position keys share {num_rot} rotation times"
            )));
        }
    }
    Ok(track)
}

fn f(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn read_times(r: &mut Reader<'_>, n: usize, format: u8, fps: f32, frame_offset: f32) -> Result<Vec<f32>> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let frame = match format {
            0 => r.f32()?,
            1 => r.u16()? as f32,
            2 => r.u8()? as f32,
            fmt => return Err(Error::Format(format!("key time format {fmt}"))),
        };
        v.push((frame - frame_offset) / fps);
    }
    Ok(v)
}

const RANGE: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Rebuilds a unit quaternion from three quantised components (in x,y,z,w order skipping `idx`).
fn from_small(idx: usize, small: [f32; 3]) -> Quat {
    let mut q = [0.0f32; 4];
    let mut it = small.iter();
    let mut sum = 0.0;
    for (i, slot) in q.iter_mut().enumerate() {
        if i != idx {
            let v = *it.next().unwrap();
            *slot = v;
            sum += v * v;
        }
    }
    q[idx] = (1.0 - sum).max(0.0).sqrt();
    Quat::from_xyzw(q[0], q[1], q[2], q[3])
}

/// 48-bit small-tree quaternion: 3x15 bits + 2-bit index (bits 46..48).
fn small_tree_48(k: &[u8]) -> Quat {
    let v = u16::from_le_bytes([k[0], k[1]]) as u64
        | (u16::from_le_bytes([k[2], k[3]]) as u64) << 16
        | (u16::from_le_bytes([k[4], k[5]]) as u64) << 32;
    let idx = (v >> 46) as usize & 3;
    let scale = 32767.0 / (2.0 * RANGE); // 23169.7
    let comp = |n: u32| ((v >> (15 * n)) & 0x7FFF) as f32 / scale - RANGE;
    from_small(idx, [comp(0), comp(1), comp(2)])
}

/// 64-bit extended small-tree quaternion: 21+21+20 bits + 2-bit index (bits 62..64).
fn small_tree_64_ext(k: &[u8]) -> Quat {
    let v = u64::from_le_bytes(k[..8].try_into().unwrap());
    let idx = (v >> 62) as usize & 3;
    let c = |shift: u32, bits: u32| {
        let max = ((1u64 << bits) - 1) as f32;
        ((v >> shift) & ((1u64 << bits) - 1)) as f32 / (max / (2.0 * RANGE)) - RANGE
    };
    from_small(idx, [c(0, 21), c(21, 21), c(42, 20)])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encoder matching `small_tree_48` (for round-trip tests).
    fn encode_48(q: Quat) -> [u8; 6] {
        let a = q.to_array();
        let idx = (0..4).max_by(|&i, &j| a[i].abs().total_cmp(&a[j].abs())).unwrap();
        let sign = if a[idx] < 0.0 { -1.0 } else { 1.0 };
        let scale = 32767.0 / (2.0 * RANGE);
        let mut v = (idx as u64) << 46;
        let mut shift = 0;
        for (i, &c) in a.iter().enumerate() {
            if i != idx {
                v |= (((c * sign + RANGE) * scale).round() as u64) << shift;
                shift += 15;
            }
        }
        let b = v.to_le_bytes();
        [b[0], b[1], b[2], b[3], b[4], b[5]]
    }

    #[test]
    fn small_tree_48_round_trips() {
        for q in [
            Quat::from_euler(glam::EulerRot::XYZ, 0.3, -1.2, 2.5),
            Quat::from_rotation_z(1.0),
            Quat::IDENTITY,
            Quat::from_xyzw(0.5, -0.5, 0.5, 0.5),
        ] {
            let d = small_tree_48(&encode_48(q));
            assert!(d.dot(q).abs() > 0.9999, "{q:?} -> {d:?}");
        }
    }

    #[test]
    fn small_tree_64_round_trips() {
        let q = Quat::from_euler(glam::EulerRot::XYZ, 0.7, 0.1, -2.0).normalize();
        let a = q.to_array();
        let idx = (0..4).max_by(|&i, &j| a[i].abs().total_cmp(&a[j].abs())).unwrap();
        let sign = if a[idx] < 0.0 { -1.0 } else { 1.0 };
        let mut v = (idx as u64) << 62;
        let mut shift = 0;
        for (i, &c) in a.iter().enumerate() {
            if i != idx {
                let bits = if shift == 42 { 20 } else { 21 };
                let max = ((1u64 << bits) - 1) as f32;
                v |= (((c * sign + RANGE) * (max / (2.0 * RANGE))).round() as u64) << shift;
                shift += bits;
            }
        }
        let d = small_tree_64_ext(&v.to_le_bytes());
        assert!(d.dot(q).abs() > 0.99999, "{q:?} -> {d:?}");
    }

    #[test]
    fn locate_clamps_and_interpolates() {
        let t = [0.0, 1.0, 3.0];
        assert_eq!(locate(&t, -1.0), Some((0, 0, 0.0)));
        assert_eq!(locate(&t, 9.0), Some((2, 2, 0.0)));
        let (i, j, f) = locate(&t, 2.0).unwrap();
        assert_eq!((i, j), (1, 2));
        assert!((f - 0.5).abs() < 1e-6);
        assert_eq!(locate(&[], 1.0), None);
    }

    #[test]
    fn track_sampling() {
        let tr = Track {
            controller: 1,
            rot_times: vec![0.0, 1.0],
            rot: vec![Quat::IDENTITY, Quat::from_rotation_z(1.0)],
            pos_times: vec![0.0, 2.0],
            pos: vec![Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)],
        };
        let q = tr.sample_rot(0.5).unwrap();
        assert!((q.dot(Quat::from_rotation_z(0.5)).abs() - 1.0).abs() < 1e-3);
        assert_eq!(tr.sample_pos(1.0), Some(Vec3::new(1.0, 0.0, 0.0)));
        assert!(Track::default().sample_rot(0.0).is_none());
    }

    /// Hand-built v0x829 controller payload (16-byte descriptor, then the keys).
    fn controller_829(id: u32, rot_fmt: u8, rot: &[u8], times: &[u8], pos: &[f32]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&id.to_le_bytes());
        d.extend_from_slice(&(times.len() as u16).to_le_bytes());
        d.extend_from_slice(&((pos.len() / 3) as u16).to_le_bytes());
        d.extend_from_slice(&[rot_fmt, 2, if pos.is_empty() { 0 } else { 2 }, 0, 0, 0, 0, 0]);
        d.extend_from_slice(rot);
        d.extend_from_slice(times);
        for p in pos {
            d.extend_from_slice(&p.to_le_bytes());
        }
        d
    }

    /// A 0x745 `.caf` container: `(type, version, id, payload)` chunks, each with its header copy.
    fn build_caf(chunks: &[(u32, u32, i32, Vec<u8>)]) -> Vec<u8> {
        let table = 20usize;
        let mut out = Vec::new();
        out.extend_from_slice(b"CryTek  ");
        out.extend_from_slice(&FILE_ANIMATION.to_le_bytes());
        out.extend_from_slice(&0x745u32.to_le_bytes());
        out.extend_from_slice(&(table as u32).to_le_bytes());
        out.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
        let mut off = table + 4 + chunks.len() * 20;
        let mut offsets = Vec::new();
        for (t, v, id, body) in chunks {
            let size = 16 + body.len();
            for w in [*t, *v, off as u32, *id as u32, size as u32] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            offsets.push(off);
            off += size;
        }
        for ((t, v, id, body), o) in chunks.iter().zip(offsets) {
            for w in [*t, *v, o as u32, *id as u32] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn parses_small_clip() {
        let mut rot = encode_48(Quat::IDENTITY).to_vec();
        rot.extend_from_slice(&encode_48(Quat::from_rotation_x(0.5)));
        let ctrl = controller_829(0xDEAD, 5, &rot, &[0, 30], &[0.0, 0.0, 1.0, 0.0, 0.0, 2.0]);
        let mut timing = Vec::new();
        timing.extend_from_slice(&(1.0f32 / 4800.0).to_le_bytes());
        timing.extend_from_slice(&160i32.to_le_bytes());
        timing.extend_from_slice(&[0u8; 32]);
        timing.extend_from_slice(&0i32.to_le_bytes());
        timing.extend_from_slice(&30i32.to_le_bytes());
        let file = build_caf(&[(ty::TIMING, 0x918, 1, timing), (ty::CONTROLLER, 0x829, 2, ctrl)]);
        let clip = Clip::parse(&file).unwrap();
        assert!((clip.duration - 1.0).abs() < 1e-4, "{}", clip.duration);
        assert_eq!(clip.tracks.len(), 1);
        let t = clip.track(0xDEAD).unwrap();
        assert_eq!(t.rot.len(), 2);
        assert!((t.rot_times[1] - 1.0).abs() < 1e-4);
        assert_eq!(t.sample_pos(0.5), Some(Vec3::new(0.0, 0.0, 1.5)));
    }

    #[test]
    fn truncated_clip_is_an_error_not_a_panic() {
        let ctrl = controller_829(1, 5, &[0u8; 6], &[0, 1], &[]); // claims 2 keys, has 1 key worth
        let file = build_caf(&[(ty::CONTROLLER, 0x829, 2, ctrl)]);
        assert!(Clip::parse(&file).is_err());
    }

    /// Decodes every `.caf` of the pak (about 15 s); run with `--ignored`.
    #[test]
    #[ignore]
    fn decodes_the_whole_pak() {
        let pak = aa_pak::Pak::global();
        let names: Vec<String> =
            pak.names().filter(|n| n.to_ascii_lowercase().ends_with(".caf")).map(String::from).collect();
        assert!(names.len() > 25_000);
        let (mut ok, mut skipped, mut failed) = (0, 0, Vec::new());
        for n in &names {
            match Clip::load(n) {
                Ok(c) => {
                    ok += 1;
                    skipped += c.skipped_controllers;
                }
                Err(e) => failed.push(format!("{n}: {e}")),
            }
        }
        eprintln!("{ok} ok, {} failed, {skipped} controllers skipped (old versions)", failed.len());
        for f in failed.iter().take(20) {
            eprintln!("  {f}");
        }
        assert!(failed.is_empty());
    }
}
