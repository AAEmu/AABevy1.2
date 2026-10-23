//! FEV instance-limit admission. The native metric is theoretical authored gain,
//! not sample RMS, priority, elapsed fades or the sum of currently decoded parts.
//! See tools/native_playback_oracle.py and docs/aa-audio.md for evidence/limits.

use std::time::Instant;

use crate::engine::Voice;
use crate::fev::{Body, Category, Event, Instance};
use crate::library::{active_instances, normalized};

pub(crate) struct Candidate {
    pub id: Voice,
    pub slot: usize,
    pub requested: Instant,
    pub gain: f32,
}

/// The native idle-slot scan begins after its cursor (initially zero). A steal
/// reuses the victim's slot without moving that cursor.
pub(crate) fn free_slot(limit: usize, cursor: usize, candidates: &[Candidate]) -> usize {
    (1..=candidates.len() + 1).map(|step| (cursor + step) % limit)
        .find(|slot| candidates.iter().all(|c| c.slot != *slot)).expect("pool has a free slot")
}

/// `None` refuses the newcomer. Candidates are occupied, non-stopping pool slots.
pub(crate) fn victim(policy: u32, candidates: &[Candidate], incoming: f32) -> Option<Voice> {
    let oldest = || candidates.iter().min_by_key(|c| (c.requested, c.id.0));
    let selected = match policy {
        4 => None,
        2 => candidates.iter().max_by_key(|c| (c.requested, c.id.0)),
        3 | 5 => {
            // The DLL replaces its candidate only for a strictly lower metric.
            let quietest = candidates.iter().min_by(|a, b| a.gain.partial_cmp(&b.gain)
                .unwrap_or(std::cmp::Ordering::Equal).then(a.slot.cmp(&b.slot)));
            if policy == 3 && candidates.first().is_some_and(|first| candidates.iter().all(|c| c.gain == first.gain)) {
                oldest()
            } else {
                quietest.filter(|c| policy == 3 || incoming > c.gain)
            }
        }
        _ => oldest(),
    };
    selected.map(|c| c.id)
}

/// EventSound's authored volume is the +28 field (raw[5]), independent of the
/// sound-definition template's volume/random pick and of sample amplitude.
fn instance_gain(instance: &Instance) -> f32 {
    f32::from_bits(instance.raw[5])
}

pub(crate) fn audibility(ev: &Event, values: &[f32], distance: Option<f32>, user: f32, category: f32) -> f32 {
    let layers = match &ev.body {
        Body::Simple(instance) => instance_gain(instance),
        Body::Complex { layers, .. } => {
            let active = active_instances(ev, values);
            layers.iter().enumerate().map(|(li, layer)| {
                // sub_1001E360 takes the first active instance of each layer.
                let Some((_, ii)) = active.iter().find(|(l, _)| *l == li) else { return 0.0 };
                let mut gain = instance_gain(&layer.instances[*ii]);
                for env in &layer.envelopes {
                    if env.flags & 8 != 0 {
                        gain *= env.at(normalized(ev, values, env.parameter as usize));
                    } else if env.flags & 0x2000 != 0 && env.dsp_param == 0 {
                        gain *= 1.0 - env.at(normalized(ev, values, env.parameter as usize));
                    }
                }
                gain
            }).sum()
        }
    };
    // sub_10016B10 clamps distance at max; custom rolloff adds no attenuation
    // here (its volume curves were already evaluated). Keep this separate from
    // the player's existing near-max silence ramp.
    let spatial = distance.map_or(1.0, |d| {
        if d <= ev.min_distance {
            return 1.0;
        }
        let d = d.min(ev.max_distance);
        match ev.mode & 0x0470_0000 {
            0x0010_0000 => if d > 0.0 { ev.min_distance / d } else { 1.0 },
            mode @ (0x0020_0000 | 0x0040_0000) => {
                let gain = if ev.max_distance > ev.min_distance {
                    (ev.max_distance - d) / (ev.max_distance - ev.min_distance)
                } else { 1.0 };
                if mode == 0x0040_0000 { gain * gain } else { gain }
            }
            _ => 1.0,
        }
    });
    // The engine has no FMOD geometry/direct event-occlusion input or source
    // orientation. Original FEV cones are neutral. Do not estimate these by RMS.
    layers * spatial * category * ev.volume * user
}

pub(crate) fn authored_category_gain(root: &Category, path: &str) -> f32 {
    let mut node = root;
    let mut gain = node.volume;
    for name in path.split('/').skip(1) {
        let Some(child) = node.children.iter().find(|c| c.name.eq_ignore_ascii_case(name)) else { break };
        gain *= child.volume;
        node = child;
    }
    gain
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use crate::fev::{EnvPoint, Envelope, Layer, Parameter};

    #[test]
    fn selection_matches_both_original_dll_paths() {
        let now = Instant::now();
        for row in include_str!("../tests/native_playback_cases.tsv").lines().filter(|r| !r.starts_with('#')) {
            let cols: Vec<_> = row.split('\t').collect();
            let gains: Vec<f32> = cols[2].split(',').map(|v| v.parse().unwrap()).collect();
            let ages: Vec<u64> = cols[3].split(',').map(|v| v.parse().unwrap()).collect();
            let candidates: Vec<_> = gains.into_iter().zip(ages).enumerate().map(|(slot, (gain, age))| Candidate {
                id: Voice(slot as u64), slot, requested: now + Duration::from_millis(age), gain,
            }).collect();
            let expected: i64 = cols[5].parse().unwrap();
            let want = (expected >= 0).then_some(Voice(expected as u64));
            assert_eq!(victim(cols[1].parse().unwrap(), &candidates, cols[4].parse().unwrap()), want, "{}", cols[0]);
            // HashMap/iteration order must not replace the original pool order.
            let mut reversed = candidates;
            reversed.reverse();
            assert_eq!(victim(cols[1].parse().unwrap(), &reversed, cols[4].parse().unwrap()), want, "{} reversed", cols[0]);
        }
    }

    #[test]
    fn idle_slot_rotation_matches_native_acquisition() {
        let now = Instant::now();
        for (occupied, cursor, want) in [(vec![], 0, 1), (vec![1], 1, 2), (vec![1, 2], 2, 0),
                                         (vec![0, 2], 0, 1), (vec![0, 1], 1, 2)] {
            let candidates: Vec<_> = occupied.into_iter().map(|slot| Candidate {
                id: Voice(slot as u64), slot, requested: now, gain: 1.0,
            }).collect();
            assert_eq!(free_slot(3, cursor, &candidates), want);
        }
    }

    fn instance(gain: f32) -> Instance {
        let mut raw = [0; 10];
        raw[5] = gain.to_bits();
        Instance { sound_def: 0, start: 0.0, length: 1.0, autopitch: false, loop_word: 0, raw }
    }

    fn event() -> Event {
        Event {
            name: String::new(), group: String::new(), flags: 0, guid: [0; 16], volume: 0.75,
            pitch: 0.0, pitch_random: 0.0, volume_random: 1.0, priority: 128, max_playbacks: 2,
            playback_behaviour: 3, mode: 8, min_distance: 1.0, max_distance: 10.0, flags2: 0,
            speaker_levels: [1.0; 8], cone_inside: 360.0, cone_outside: 360.0, cone_outside_volume: 1.0,
            doppler: 0.0, reverb_dry: 0.0, reverb_wet: 0.0, speaker_spread: 0.0, pan_level: 1.0,
            fade_in_ms: 6000, fade_out_ms: 4000, spawn_intensity: 0.0, spawn_intensity_random: 0.0,
            raw: [0; 4], category: "master/test".into(), body: Body::Simple(instance(0.5)),
        }
    }

    #[test]
    fn native_authored_metric_ignores_priority_randomization_and_fades() {
        let mut ev = event();
        assert_eq!(audibility(&ev, &[], None, 0.25, 0.5), 0.046875); // DLL oracle
        ev.priority = 256;
        ev.volume_random = 0.0;
        ev.fade_in_ms = 0;
        ev.fade_out_ms = 0;
        assert_eq!(audibility(&ev, &[], None, 0.25, 0.5), 0.046875);
        assert_eq!(audibility(&ev, &[], None, 0.25, 0.0), 0.0);
    }

    #[test]
    fn metric_sums_first_active_instances_and_volume_direct_occlusion_curves() {
        let mut ev = event();
        ev.volume = 1.0;
        let env = Envelope { parent: -1, dsp: String::new(), dsp_param: 0, flags: 8, flags2: 0, parameter: 0,
            points: vec![EnvPoint { x: 0.0, y: 0.0, curve: 2 }, EnvPoint { x: 1.0, y: 1.0, curve: 2 }] };
        let mut layer = Layer { flags: 0, priority: 0, control_parameter: 0,
            instances: vec![instance(0.25), instance(0.9)], envelopes: Vec::new() };
        let mut second = layer.clone();
        second.instances = vec![instance(0.5)];
        let parameter = Parameter { name: "control".into(), min: 0.0, max: 1.0, velocity: 0.0,
            flags: 0, seek_speed: 0.0, raw_44: 0, sustain_points: Vec::new() };
        ev.body = Body::Complex { layers: vec![layer.clone(), second], parameters: vec![parameter.clone()], props: Vec::new() };
        assert_eq!(audibility(&ev, &[0.5], None, 1.0, 1.0), 0.75); // DLL oracle
        layer.instances[0].start = 0.5;
        layer.instances[0].length = 0.5;
        layer.envelopes.push(env.clone());
        let mut occ = env;
        occ.flags = 0x2000;
        layer.envelopes.push(occ.clone());
        occ.dsp_param = 1; // reverb-path occlusion does not enter direct audibility
        layer.envelopes.push(occ);
        ev.body = Body::Complex { layers: vec![layer], parameters: vec![parameter], props: Vec::new() };
        assert_eq!(audibility(&ev, &[0.5], None, 1.0, 1.0), 0.0625);
        assert!((audibility(&ev, &[0.25], None, 1.0, 1.0) - 0.9 * 0.25 * 0.75).abs() < 1e-6);
    }

    #[test]
    fn policy_rolloff_and_category_hierarchy_follow_the_native_metric() {
        let mut ev = event();
        ev.volume = 1.0;
        ev.body = Body::Simple(instance(1.0));
        ev.mode = 0x100010;
        assert_eq!(audibility(&ev, &[], Some(20.0), 1.0, 1.0), 0.1); // log holds at max
        ev.mode = 0x200010;
        assert_eq!(audibility(&ev, &[], Some(5.5), 1.0, 1.0), 0.5);
        ev.mode = 0x400010;
        assert_eq!(audibility(&ev, &[], Some(5.5), 1.0, 1.0), 0.25);
        ev.mode = 0x4000010;
        assert_eq!(audibility(&ev, &[], Some(20.0), 1.0, 1.0), 1.0); // custom belongs to curves
        let root = Category { name: "master".into(), volume: 0.25, pitch: 0.0, max_playbacks: 0,
            children: vec![Category { name: "test".into(), volume: 0.5, pitch: 0.0, max_playbacks: 0, children: Vec::new() }] };
        assert_eq!(authored_category_gain(&root, "master/test"), 0.125); // native core oracle
    }
}
