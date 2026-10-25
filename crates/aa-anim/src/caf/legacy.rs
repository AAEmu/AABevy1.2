//! Original 1.2 CControllerPQLog (0x827 / 0x828).
//!
//! CryAnimation 3162F6E0 imports both into the same keys; 3162AC20 samples
//! conjugate exp(log). Its 3162A840 helper exp/nlerps/logs between keys, then
//! DecodeKey exponentiates again. Keeping the decoded endpoints therefore uses
//! Track's existing shortest-arc nlerp, within the native float roundoff.

use super::{Track, f};
use crate::{Error, Result};
use aa_cry::chunk::Reader;
use glam::{Quat, Vec3};

pub(super) fn decode(data: &[u8], fps: f32, frame_offset: f32) -> Result<Track> {
    let mut r = Reader::new(data);
    let count = r.u32()? as usize;
    let controller = r.u32()?;
    // Host validation, not native rejection behavior: bound allocations to the
    // payload and decline ambiguous trailing data, empty or decreasing tracks.
    let size = count
        .checked_mul(28)
        .ok_or_else(|| Error::Format("PQLog key count overflow".into()))?;
    let keys = r.bytes(size)?;
    if count == 0 || data.len() != 8 + size {
        return Err(Error::Format(format!(
            "controller {controller:#x}: PQLog key count/payload size"
        )));
    }
    let mut track = Track {
        controller,
        pq_log: true,
        ..Default::default()
    };
    let mut previous = None;
    for key in keys.as_chunks::<28>().0 {
        // The native signed divide truncates toward zero BEFORE converting to
        // the sampler's frame units, independently of Timing.ticks_per_frame.
        let frame = i32::from_le_bytes(key[..4].try_into().unwrap()) / 160;
        if previous.is_some_and(|p| frame < p) {
            return Err(Error::Format(format!(
                "controller {controller:#x}: decreasing PQLog frames"
            )));
        }
        previous = Some(frame);
        let pos = Vec3::new(f(key, 4), f(key, 8), f(key, 12)) * 0.01;
        let log = Vec3::new(f(key, 16), f(key, 20), f(key, 24));
        if !pos.is_finite() || !log.is_finite() || !log.length_squared().is_finite() {
            return Err(Error::Format(format!(
                "controller {controller:#x}: nonfinite PQLog key"
            )));
        }
        let angle = log.length();
        let rotation = if angle > 0.0 {
            let (sin, cos) = angle.sin_cos();
            let xyz = log * (-sin / angle);
            Quat::from_xyzw(xyz.x, xyz.y, xyz.z, cos)
        } else {
            Quat::IDENTITY
        };
        track.rot_times.push((frame as f32 - frame_offset) / fps);
        track.rot.push(rotation);
        track.pos.push(pos);
    }
    track.pos_times = track.rot_times.clone();
    Ok(track)
}

/// Native DecodeKey's lookup, including its choice among equal integer frames.
/// Quantizing the original anchor's 100-tick keys to /160 frames creates these
/// duplicates. Keep the 0x829 partition-point lookup entirely separate.
pub(super) fn locate(times: &[f32], t: f32) -> Option<(usize, usize, f32)> {
    let n = times.len();
    if n == 0 {
        return None;
    }
    if n == 1 || t < times[0] {
        return Some((0, 0, 0.0));
    }
    if t >= times[n - 1] {
        return Some((n - 1, n - 1, 0.0));
    }
    let mut j = n / 2;
    let mut step = n / 4;
    while step != 0 {
        if times[j] > t {
            j -= step;
        } else if times[j] < t {
            j += step;
        } else {
            break;
        }
        step /= 2;
    }
    while t > times[j] {
        j += 1;
    }
    while j > 1 && times[j - 1] > t {
        j -= 1;
    }
    let i = j - 1;
    if times[j] == times[i] {
        return Some((j, j, 0.0));
    }
    Some((i, j, (t - times[i]) / (times[j] - times[i])))
}
