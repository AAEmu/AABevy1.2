//! Decoded sounds and the cache that holds them.

use std::collections::HashMap;
use std::sync::Arc;

use kira::Frame;

use crate::decode::Pcm;

/// A decoded sound in the form the mixer plays: stereo float frames at the sound's own rate (the
/// mixer resamples). Mono sounds are duplicated into both channels, which costs 8 bytes per frame
/// whatever the source; short effects dominate, so the cache holds minutes of them.
pub struct Sample {
    pub rate: u32,
    pub frames: Arc<[Frame]>,
    /// Loop start and end (inclusive) in frames, from the bank.
    pub loop_points: Option<(u32, u32)>,
}

impl Sample {
    pub fn from_pcm(pcm: &Pcm) -> Sample {
        let ch = pcm.channels.max(1) as usize;
        let scale = 1.0 / 32768.0;
        let frames: Arc<[Frame]> = pcm
            .samples
            .chunks_exact(ch)
            .map(|f| {
                let l = f[0] as f32 * scale;
                let r = if ch > 1 { f[1] as f32 * scale } else { l };
                Frame { left: l, right: r }
            })
            .collect();
        Sample { rate: pcm.rate, frames, loop_points: pcm.loop_points }
    }

    pub fn bytes(&self) -> usize {
        self.frames.len() * std::mem::size_of::<Frame>()
    }

    pub fn seconds(&self) -> f32 {
        self.frames.len() as f32 / self.rate.max(1) as f32
    }
}

/// Bank path and subsound index.
pub type Key = (String, u32);

/// Least recently used cache of decoded sounds, bounded in bytes.
pub struct SampleCache {
    limit: usize,
    used: usize,
    tick: u64,
    map: HashMap<Key, (Arc<Sample>, u64)>,
    pub hits: u64,
    pub misses: u64,
}

impl SampleCache {
    pub fn new(limit: usize) -> SampleCache {
        SampleCache { limit, used: 0, tick: 0, map: HashMap::new(), hits: 0, misses: 0 }
    }

    pub fn get(&mut self, key: &Key) -> Option<Arc<Sample>> {
        self.tick += 1;
        match self.map.get_mut(key) {
            Some((s, stamp)) => {
                *stamp = self.tick;
                self.hits += 1;
                Some(s.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Adds a sound and drops the least recently used ones while the cache is over its limit (the new
    /// sound stays even if it alone is over).
    pub fn insert(&mut self, key: Key, sample: Arc<Sample>) {
        self.tick += 1;
        self.used += sample.bytes();
        if let Some((old, _)) = self.map.insert(key, (sample, self.tick)) {
            self.used -= old.bytes();
        }
        while self.used > self.limit && self.map.len() > 1 {
            let Some(oldest) = self.map.iter().min_by_key(|(_, (_, stamp))| *stamp).map(|(k, _)| k.clone()) else { break };
            if let Some((s, _)) = self.map.remove(&oldest) {
                self.used -= s.bytes();
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(frames: usize) -> Arc<Sample> {
        Arc::new(Sample { rate: 22050, frames: vec![Frame::ZERO; frames].into(), loop_points: None })
    }

    #[test]
    fn converts_mono_and_stereo() {
        let mono = Pcm { rate: 22050, channels: 1, samples: vec![16384, -32768], loop_points: None };
        let s = Sample::from_pcm(&mono);
        assert_eq!(s.frames.len(), 2);
        assert_eq!((s.frames[0].left, s.frames[0].right), (0.5, 0.5));
        assert_eq!(s.frames[1].left, -1.0);
        let stereo = Pcm { rate: 44100, channels: 2, samples: vec![32767, 0, -16384, 16384], loop_points: None };
        let s = Sample::from_pcm(&stereo);
        assert_eq!(s.frames.len(), 2);
        assert_eq!((s.frames[1].left, s.frames[1].right), (-0.5, 0.5));
        assert_eq!(s.bytes(), 16);
    }

    #[test]
    fn evicts_the_least_recently_used() {
        let mut c = SampleCache::new(8 * 100);
        c.insert(("a".into(), 0), sample(40));
        c.insert(("b".into(), 0), sample(40));
        assert!(c.get(&("a".into(), 0)).is_some()); // a is now newer than b
        c.insert(("c".into(), 0), sample(40)); // 120 frames > 100: drop b
        assert!(c.get(&("b".into(), 0)).is_none());
        assert!(c.get(&("a".into(), 0)).is_some());
        assert!(c.get(&("c".into(), 0)).is_some());
        assert_eq!(c.bytes(), 8 * 80);
        // a sound over the limit by itself is kept
        c.insert(("big".into(), 0), sample(500));
        assert_eq!(c.len(), 1);
        assert_eq!((c.hits, c.misses), (3, 1));
    }
}
