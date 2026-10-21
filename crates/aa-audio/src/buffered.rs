//! Music in the browser: kira's streaming sounds need a thread, so there a [`Feeder`] decodes on the main thread a little at a
//! time ([`Engine::pump`](crate::Engine::pump)) into a queue of frames that a kira [`Sound`] plays. The sound never blocks: when
//! the queue runs dry it plays silence until the feeder catches up.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kira::command::{CommandReader, CommandWriter, ValueChangeCommand, command_writer_and_reader};
use kira::info::Info;
use kira::sound::{PlaybackState, Sound, SoundData};
use kira::{Decibels, Frame, Parameter, Tween, Value};
use web_time::Instant;

use crate::fsb5::Error;
use crate::stream::StreamDecoder;

/// How much audio the feeder keeps queued.
pub const QUEUE_SECONDS: f64 = 2.0;
/// The longest one [`Feeder::fill`] call decodes.
pub const BUDGET: Duration = Duration::from_millis(2);

/// Where the decoded frames wait for the mixer.
struct Queue {
    frames: Mutex<VecDeque<Frame>>,
    /// The feeder has queued the last frame.
    ended: AtomicBool,
    state: AtomicU8,
}

impl Queue {
    fn state(&self) -> PlaybackState {
        match self.state.load(Ordering::SeqCst) {
            0 => PlaybackState::Playing,
            5 => PlaybackState::Stopping,
            _ => PlaybackState::Stopped,
        }
    }

    fn set_state(&self, state: PlaybackState) {
        self.state.store(state as u8, Ordering::SeqCst);
    }

    fn frames(&self) -> std::sync::MutexGuard<'_, VecDeque<Frame>> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Decoded chunks of a track, in order.
pub trait ChunkSource {
    fn rate(&self) -> u32;
    /// Frames in the track.
    fn total(&self) -> usize;
    /// The next chunk (never empty), padded with silence up to `total`.
    fn next_chunk(&mut self) -> Result<Vec<Frame>, Error>;
    fn rewind(&mut self);
}

impl ChunkSource for StreamDecoder {
    fn rate(&self) -> u32 {
        StreamDecoder::rate(self)
    }
    fn total(&self) -> usize {
        StreamDecoder::total(self)
    }
    fn next_chunk(&mut self) -> Result<Vec<Frame>, Error> {
        self.decode_chunk()
    }
    fn rewind(&mut self) {
        self.restart();
    }
}

/// Decodes a track into the queue of its sound.
pub struct Feeder<S: ChunkSource = StreamDecoder> {
    source: S,
    queue: Arc<Queue>,
    /// Frames of the track decoded so far (counting the ones skipped).
    pos: usize,
    /// After a repeat, frames before this one are decoded but not queued.
    skip_until: usize,
    /// The part that repeats `(start, end)`, if the track loops.
    repeat: Option<(usize, usize)>,
    done: bool,
}

impl<S: ChunkSource> Feeder<S> {
    /// Decodes until about [`QUEUE_SECONDS`] are queued or `budget` is spent.
    pub fn fill(&mut self, budget: Duration) {
        let start = Instant::now();
        let target = (self.source.rate() as f64 * QUEUE_SECONDS) as usize;
        while !self.done && self.queue.frames().len() < target {
            self.step();
            if start.elapsed() >= budget {
                break;
            }
        }
    }

    /// Decodes one chunk.
    fn step(&mut self) {
        let end = self.repeat.map_or(self.source.total(), |(_, e)| e);
        let chunk = match self.source.next_chunk() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("aa-audio: music decode stopped: {e}");
                self.finish();
                return;
            }
        };
        let (lo, hi) = (self.pos.max(self.skip_until), (self.pos + chunk.len()).min(end));
        if lo < hi {
            self.queue.frames().extend(&chunk[lo - self.pos..hi - self.pos]);
        }
        self.pos += chunk.len();
        if self.pos >= end {
            match self.repeat {
                Some((start, _)) => {
                    self.source.rewind();
                    self.pos = 0;
                    self.skip_until = start;
                }
                None => self.finish(),
            }
        }
    }

    fn finish(&mut self) {
        self.done = true;
        self.queue.ended.store(true, Ordering::SeqCst);
    }

    /// The sound is gone (played out or dropped by the mixer): nothing is left to feed.
    pub fn is_dead(&self) -> bool {
        Arc::strong_count(&self.queue) == 1 || self.queue.state() == PlaybackState::Stopped
    }
}

/// Controls a [`BufferedData`] sound once it plays (the part of kira's streaming handle that music uses).
pub struct BufferedHandle {
    queue: Arc<Queue>,
    volume: CommandWriter<ValueChangeCommand<Decibels>>,
    stop: CommandWriter<Tween>,
}

impl BufferedHandle {
    pub fn state(&self) -> PlaybackState {
        self.queue.state()
    }

    pub fn set_volume(&mut self, volume: Decibels, tween: Tween) {
        self.volume.write(ValueChangeCommand { target: Value::Fixed(volume), tween });
    }

    /// Fades out, then stops.
    pub fn stop(&mut self, tween: Tween) {
        self.stop.write(tween);
    }
}

/// A sound that plays what a [`Feeder`] queues.
pub struct BufferedData {
    queue: Arc<Queue>,
    rate: u32,
    volume: Decibels,
    playback_rate: f64,
    fade_in: Option<Tween>,
}

/// A track to play at `volume` and `playback_rate`, fading in over `fade_in`; `repeat` is the part that loops (loop points of
/// the bank, else all of it) when the track loops. Hand the data to the mixer and keep the feeder, calling [`Feeder::fill`].
pub fn buffered<S: ChunkSource>(source: S, volume: Decibels, playback_rate: f64, fade_in: Option<Tween>, repeat: Option<Option<(u32, u32)>>) -> (BufferedData, Feeder<S>) {
    let total = source.total();
    let repeat = repeat.map(|points| match points {
        Some((a, b)) if b > a => (a as usize, (b as usize + 1).min(total)),
        _ => (0, total),
    });
    let queue = Arc::new(Queue { frames: Mutex::new(VecDeque::new()), ended: AtomicBool::new(false), state: AtomicU8::new(PlaybackState::Playing as u8) });
    let data = BufferedData { queue: queue.clone(), rate: source.rate(), volume, playback_rate, fade_in };
    (data, Feeder { source, queue, pos: 0, skip_until: 0, repeat, done: false })
}

impl SoundData for BufferedData {
    type Error = std::convert::Infallible;
    type Handle = BufferedHandle;

    fn into_sound(self) -> Result<(Box<dyn Sound>, BufferedHandle), Self::Error> {
        let (volume_writer, volume_reader) = command_writer_and_reader();
        let (stop_writer, stop_reader) = command_writer_and_reader();
        let mut fade = Parameter::new(Value::Fixed(Decibels::IDENTITY), Decibels::IDENTITY);
        if let Some(tween) = self.fade_in {
            fade = Parameter::new(Value::Fixed(Decibels::SILENCE), Decibels::SILENCE);
            fade.set(Value::Fixed(Decibels::IDENTITY), tween);
        }
        let handle = BufferedHandle { queue: self.queue.clone(), volume: volume_writer, stop: stop_writer };
        let sound = BufferedSound {
            queue: self.queue,
            rate: self.rate as f64 * self.playback_rate,
            volume: Parameter::new(Value::Fixed(self.volume), Decibels::IDENTITY),
            fade,
            stopping: false,
            volume_reader,
            stop_reader,
            window: [Frame::ZERO; 2],
            have: 0,
            fraction: 0.0,
        };
        Ok((Box::new(sound), handle))
    }
}

struct BufferedSound {
    queue: Arc<Queue>,
    /// Frames of the track per second of output.
    rate: f64,
    volume: Parameter<Decibels>,
    /// Fade in, and the fade out of a stop.
    fade: Parameter<Decibels>,
    stopping: bool,
    volume_reader: CommandReader<ValueChangeCommand<Decibels>>,
    stop_reader: CommandReader<Tween>,
    /// The two frames the output is between, and how many of them are loaded.
    window: [Frame; 2],
    have: usize,
    fraction: f64,
}

impl Sound for BufferedSound {
    fn on_start_processing(&mut self) {
        self.volume.read_command(&mut self.volume_reader);
        if let Some(tween) = self.stop_reader.read()
            && !self.stopping
        {
            self.stopping = true;
            self.fade.set(Value::Fixed(Decibels::SILENCE), tween);
            self.queue.set_state(PlaybackState::Stopping);
        }
    }

    fn process(&mut self, out: &mut [Frame], dt: f64, info: &Info) {
        out.fill(Frame::ZERO);
        if self.finished() {
            return;
        }
        let span = dt * out.len() as f64;
        self.volume.update(span, info);
        if self.fade.update(span, info) && self.stopping {
            self.queue.set_state(PlaybackState::Stopped);
            return;
        }
        // never wait for the feeder: no lock, no frames, silence
        let Ok(mut queue) = self.queue.frames.try_lock() else { return };
        let step = self.rate * dt;
        let n = out.len();
        for (i, frame) in out.iter_mut().enumerate() {
            while self.have < 2 {
                let Some(next) = queue.pop_front() else { break };
                self.window = [self.window[1], next];
                self.have += 1;
            }
            if self.have < 2 {
                if self.queue.ended.load(Ordering::SeqCst) && queue.is_empty() {
                    self.queue.set_state(PlaybackState::Stopped);
                }
                break;
            }
            let t = (i + 1) as f64 / n as f64;
            let gain = self.volume.interpolated_value(t).as_amplitude() * self.fade.interpolated_value(t).as_amplitude();
            let f = self.fraction as f32;
            let (a, b) = (self.window[0], self.window[1]);
            *frame = Frame { left: a.left + (b.left - a.left) * f, right: a.right + (b.right - a.right) * f } * gain;
            self.fraction += step;
            let whole = self.fraction.floor();
            self.fraction -= whole;
            self.have = self.have.saturating_sub(whole as usize);
        }
    }

    fn finished(&self) -> bool {
        self.queue.state() == PlaybackState::Stopped
    }
}

#[cfg(test)]
mod tests {
    use kira::info::MockInfoBuilder;

    use super::*;

    /// `total` frames that count up from 1, in chunks of 100.
    struct Ramp {
        total: usize,
        at: usize,
    }

    impl ChunkSource for Ramp {
        fn rate(&self) -> u32 {
            1000
        }
        fn total(&self) -> usize {
            self.total
        }
        fn next_chunk(&mut self) -> Result<Vec<Frame>, Error> {
            let n = 100.min(self.total - self.at);
            let chunk = (self.at..self.at + n).map(|i| Frame::from_mono((i + 1) as f32)).collect();
            self.at += n;
            Ok(chunk)
        }
        fn rewind(&mut self) {
            self.at = 0;
        }
    }

    fn start(total: usize, repeat: Option<Option<(u32, u32)>>) -> (Feeder<Ramp>, Box<dyn Sound>, BufferedHandle) {
        let (data, feeder) = buffered(Ramp { total, at: 0 }, Decibels::IDENTITY, 1.0, None, repeat);
        let (sound, handle) = data.into_sound().unwrap();
        (feeder, sound, handle)
    }

    /// Plays `n` output frames of 1 ms (one track frame each).
    fn play(sound: &mut dyn Sound, n: usize) -> Vec<f32> {
        let info = MockInfoBuilder::new().build();
        (0..n).map(|_| sound.process_one(0.001, &info).left).collect()
    }

    #[test]
    fn underrun_is_silent_then_plays_and_finishes() {
        let (mut feeder, mut sound, _handle) = start(250, None);
        assert_eq!(play(&mut *sound, 3), [0.0; 3]);
        assert!(!sound.finished());
        feeder.fill(Duration::from_secs(1));
        assert_eq!(feeder.queue.frames().len(), 250);
        let out = play(&mut *sound, 200);
        assert_eq!(out[100], 101.0);
        assert!(!sound.finished());
        play(&mut *sound, 100);
        assert!(sound.finished());
    }

    #[test]
    fn fill_stops_at_the_queue_target() {
        let (mut feeder, _sound, _handle) = start(100_000, None);
        feeder.fill(Duration::from_secs(5));
        let n = feeder.queue.frames().len();
        assert!((2000..2100).contains(&n), "{n}");
        assert!(!feeder.done);
    }

    #[test]
    fn loop_region_repeats() {
        let (mut feeder, _sound, _handle) = start(300, Some(Some((100, 199))));
        feeder.fill(Duration::from_secs(1));
        let q: Vec<f32> = feeder.queue.frames().iter().take(400).map(|f| f.left).collect();
        assert_eq!(q[0], 1.0);
        assert_eq!(q[199], 200.0);
        // after frame 200 (the end of the region) comes frame 101 (its start) again
        assert_eq!(q[200], 101.0);
        assert_eq!(q[300], 101.0);
    }

    #[test]
    fn stop_fades_out_to_stopped() {
        let (mut feeder, mut sound, mut handle) = start(100_000, None);
        feeder.fill(Duration::from_secs(1));
        play(&mut *sound, 10);
        handle.stop(Tween { duration: Duration::from_millis(20), ..Tween::default() });
        sound.on_start_processing();
        assert_eq!(handle.state(), PlaybackState::Stopping);
        play(&mut *sound, 100);
        assert_eq!(handle.state(), PlaybackState::Stopped);
        assert!(feeder.is_dead());
    }
}
