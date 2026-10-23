//! Bounded worker rendering into the existing audio engine, with generation cancellation.
use aa_audio::{Category, Engine, PcmClip, PcmOptions, PcmPlaybackObserver, Voice};
use bevy::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock, mpsc};

fn render(midi: &[u8]) -> Result<PcmClip, String> {
    static BANK: OnceLock<Result<aa_midi::dls::Dls, String>> = OnceLock::new();
    let bank = BANK
        .get_or_init(|| {
            let bytes = aa_pak::Pak::global()
                .read("sounds/default.dls")
                .map_err(|e| e.to_string())?;
            aa_midi::dls::Dls::parse(&bytes).map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let song = aa_midi::smf::parse(midi).map_err(|e| e.to_string())?;
    let pcm = aa_midi::synth::render(bank, &song, 0.5).map_err(|e| e.to_string())?;
    PcmClip::mono(24000, &pcm).map_err(|e| e.to_string())
}

struct Job {
    actor: u32, // zero is the local preview/performance slot
    generation: u64,
    result: Mutex<mpsc::Receiver<Result<PcmClip, String>>>,
}

struct Foreign {
    voice: Voice,
    observer: PcmPlaybackObserver,
}

#[derive(Resource, Default)]
pub(super) struct Playback {
    generation: u64,
    jobs: Vec<Job>,
    preview: Option<(Voice, PcmPlaybackObserver)>,
    foreign: BTreeMap<u32, Foreign>,
    pending: BTreeMap<u32, (u64, Vec3)>,
    serial: u64,
    mute_others: bool,
}

impl Playback {
    fn clear_own(&mut self, engine: Option<&Engine>) {
        if let Some((voice, observer)) = self.preview.take() {
            // Explicit stop/replacement erases the native tracking entry before stopping.
            drop(observer);
            if let Some(engine) = engine {
                engine.stop(voice, false);
            }
        }
        // Workers finish into their own one-result channels; stale clips are discarded.
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn clear(&mut self, engine: Option<&Engine>) {
        self.clear_own(engine);
        self.pending.clear();
        for (_, foreign) in std::mem::take(&mut self.foreign) {
            drop(foreign.observer);
            if let Some(engine) = engine {
                engine.stop(foreign.voice, false);
            }
        }
    }

    pub(super) fn stop_foreign(&mut self, engine: Option<&Engine>, actor: u32) {
        self.pending.remove(&actor);
        if let Some(foreign) = self.foreign.remove(&actor) {
            drop(foreign.observer);
            if let Some(engine) = engine {
                engine.stop(foreign.voice, false);
            }
        }
    }

    pub(super) fn play_foreign(
        &mut self,
        engine: Option<&Engine>,
        actor: u32,
        midi: Vec<u8>,
        position: Option<Vec3>,
    ) {
        // Native replacement removes the old actor entry before creation/actor lookup.
        self.stop_foreign(engine, actor);
        let Some(position) = position else {
            return;
        };
        if actor == 0 || self.jobs.len() >= 2 {
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        let serial = self.serial;
        if let Some(job) = spawn_job(actor, serial, midi) {
            self.pending.insert(actor, (serial, position));
            self.jobs.push(job);
        }
    }

    pub(super) fn foreign_actors(&self) -> impl Iterator<Item = u32> + '_ {
        self.foreign.keys().chain(self.pending.keys()).copied()
    }

    pub(super) fn move_foreign(
        &mut self,
        engine: Option<&Engine>,
        positions: &BTreeMap<u32, Vec3>,
        mute: bool,
    ) {
        for (actor, (_, position)) in &mut self.pending {
            if let Some(current) = positions.get(actor) {
                *position = *current;
            }
        }
        if let Some(engine) = engine {
            for (actor, foreign) in &self.foreign {
                if let Some(position) = positions.get(actor) {
                    engine.set_position(foreign.voice, *position);
                }
                if self.mute_others != mute {
                    engine.set_voice_volume(foreign.voice, if mute { 0. } else { 1. });
                }
            }
        }
        self.mute_others = mute;
    }

    pub(super) fn update(
        &mut self,
        engine: Option<&Engine>,
        generation: u64,
        midi: Option<Vec<u8>>,
    ) -> bool {
        if self.generation != generation {
            self.clear_own(engine);
            self.generation = generation;
        }
        self.jobs
            .retain(|job| match job.result.lock().unwrap().try_recv() {
                Ok(result) => {
                    let target = if job.actor == 0 {
                        (job.generation == self.generation).then_some(None)
                    } else {
                        self.pending
                            .get(&job.actor)
                            .filter(|(serial, _)| *serial == job.generation)
                            .map(|(_, position)| Some(*position))
                    };
                    if let Some(position) = target {
                        self.pending.remove(&job.actor);
                        match result {
                            Ok(clip) => {
                                if let Some(engine) = engine {
                                    let mut options = PcmOptions::new(Category::UserMusic);
                                    if let Some(position) = position {
                                        options.spatial = Some(aa_audio::PcmSpatial {
                                            position,
                                            min_distance: 30.,
                                            max_distance: 50.,
                                        });
                                        options.volume = if self.mute_others { 0. } else { 1. };
                                    }
                                    match engine.play_pcm_tracked(&clip, options) {
                                        Ok(tracked) => {
                                            if job.actor == 0 {
                                                self.preview = Some(tracked);
                                            } else {
                                                self.foreign.insert(
                                                    job.actor,
                                                    Foreign {
                                                        voice: tracked.0,
                                                        observer: tracked.1,
                                                    },
                                                );
                                            }
                                        }
                                        Err(error) => warn!("user music: PCM submission: {error}"),
                                    }
                                }
                            }
                            Err(error) => warn!("user music: render: {error}"),
                        }
                    }
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if self
                        .pending
                        .get(&job.actor)
                        .is_some_and(|(serial, _)| *serial == job.generation)
                    {
                        self.pending.remove(&job.actor);
                    }
                    false
                }
                Err(mpsc::TryRecvError::Empty) => true,
            });
        // At most two render buffers/retained clip results, including canceled work.
        if let Some(midi) = midi {
            if self.jobs.len() >= 2 {
                warn!("user music: render workers busy");
                return false;
            }
            if let Some(job) = spawn_job(0, self.generation, midi) {
                self.jobs.push(job);
            }
        }
        self.foreign
            .retain(|_, foreign| !foreign.observer.is_finished());
        if self
            .preview
            .as_ref()
            .is_some_and(|(_, observer)| observer.is_finished())
        {
            let (_, observer) = self.preview.take().unwrap();
            // Finished Acquire precedes the admission query. Failed/muted requests never count.
            return observer.was_started();
        }
        false
    }
}

fn spawn_job(actor: u32, generation: u64, midi: Vec<u8>) -> Option<Job> {
    let (sender, result) = mpsc::sync_channel(1);
    match std::thread::Builder::new()
        .name("score-render".into())
        .spawn(move || {
            let _ = sender.send(render(&midi));
        }) {
        Ok(_) => Some(Job {
            actor,
            generation,
            result: Mutex::new(result),
        }),
        Err(error) => {
            warn!("user music: worker: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(engine: &Engine) -> Vec<Voice> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let voices = engine.take_finished();
            if !voices.is_empty() || std::time::Instant::now() >= deadline {
                return voices;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn ready(generation: u64) -> Job {
        let (sender, result) = mpsc::sync_channel(1);
        sender
            .send(Ok(PcmClip::mono(24000, &[0.; 240]).unwrap()))
            .ok()
            .unwrap();
        Job {
            actor: 0,
            generation,
            result: Mutex::new(result),
        }
    }

    #[test]
    fn canceled_generation_never_submits_pcm_and_muted_completion_does_not_send_wire() {
        let engine = Engine::start(aa_audio::Settings {
            audible: false,
            ..default()
        });
        let mut playback = Playback {
            generation: 10,
            jobs: vec![ready(9)],
            ..default()
        };
        assert!(!playback.update(Some(&engine), 10, None));
        assert!(playback.jobs.is_empty());
        assert!(
            engine.take_finished().is_empty(),
            "stale PCM was never submitted"
        );
        playback.jobs.push(ready(10));
        assert!(!playback.update(Some(&engine), 10, None));
        assert_eq!(
            finished(&engine).len(),
            1,
            "current PCM uses existing muted completion"
        );
        assert!(!playback.update(Some(&engine), 10, None));
        assert!(playback.preview.is_none());
        playback.jobs.push(ready(10));
        assert!(!playback.update(Some(&engine), 11, None));
        assert!(engine.take_finished().is_empty());
    }

    #[test]
    #[ignore = "loads the original 1.2 DLS bank into the bounded client renderer"]
    fn original_bank_score_renders_pcm_without_a_disk_audio_file_or_event_decode() {
        let midi = aa_midi::mml::compile("O5 C8", aa_midi::mml::Voice::melodic(73), 1., 0).unwrap();
        let clip = render(&midi).unwrap();
        assert_eq!(clip.rate(), 24000);
        assert_eq!(clip.samples(), 18000);
        assert_eq!(clip.bytes(), 18000 * 8);
        let engine = Engine::start(aa_audio::Settings {
            audible: false,
            ..default()
        });
        engine
            .play_pcm(&clip, PcmOptions::new(Category::UserMusic))
            .unwrap();
        assert_eq!(finished(&engine).len(), 1);
        assert_eq!(engine.stats().cache_bytes, 0);
    }

    #[test]
    #[ignore = "opens an audio device to verify actual admission and own voice disappearance"]
    fn admitted_own_voice_ends_once_independent_of_finished_drain_and_explicit_stop() {
        let engine = Engine::start(aa_audio::Settings {
            audible: true,
            ..default()
        });
        engine.set_volumes(aa_audio::Volumes {
            master: 0.0,
            ..default()
        });
        let mut playback = Playback {
            generation: 10,
            jobs: vec![ready(10)],
            ..default()
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut ends = 0;
        while ends == 0 && std::time::Instant::now() < deadline {
            // The production audio plugin can consume these before the client polls its observer.
            let _ = engine.take_finished();
            ends += usize::from(playback.update(Some(&engine), 10, None));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(engine.stats().device_error, None);
        assert_eq!(
            ends, 1,
            "only a real accepted handle may request native End"
        );
        assert!(!playback.update(Some(&engine), 10, None));
        playback.jobs.push(ready(10));
        playback.update(Some(&engine), 10, None);
        playback.clear(Some(&engine));
        assert!(
            !playback.update(Some(&engine), 11, None),
            "explicit stop removed tracking"
        );
    }

    #[test]
    fn foreign_pause_replacement_and_missing_actor_retire_pending_without_own_end() {
        let engine = Engine::start(aa_audio::Settings {
            audible: false,
            ..default()
        });
        let mut playback = Playback {
            generation: 10,
            ..default()
        };
        let mut job = ready(1);
        job.actor = 200;
        playback.jobs.push(job);
        playback.pending.insert(200, (1, Vec3::ZERO));
        playback.stop_foreign(Some(&engine), 200);
        assert!(!playback.update(Some(&engine), 10, None));
        assert!(
            engine.take_finished().is_empty(),
            "paused pending result never submits"
        );
        let mut job = ready(2);
        job.actor = 200;
        playback.jobs.push(job);
        playback.pending.insert(200, (2, Vec3::ZERO));
        playback.play_foreign(Some(&engine), 200, vec![1, 2], None);
        assert!(!playback.update(Some(&engine), 10, None));
        assert!(playback.pending.is_empty() && playback.foreign.is_empty());
        assert!(
            engine.take_finished().is_empty(),
            "missing actor replacement erased old pending"
        );
    }

    #[test]
    #[ignore = "real device: foreign zero gain clock and own tracking coexist"]
    fn muted_foreign_clock_finishes_without_end_while_own_ends_once() {
        let engine = Engine::start(aa_audio::Settings {
            audible: true,
            ..default()
        });
        engine.set_volumes(aa_audio::Volumes {
            master: 0.,
            ..default()
        });
        let mut playback = Playback {
            generation: 10,
            ..default()
        };
        let mut foreign = ready(1);
        foreign.actor = 200;
        playback.jobs.extend([foreign, ready(10)]);
        playback.pending.insert(200, (1, Vec3::ZERO));
        playback.move_foreign(Some(&engine), &BTreeMap::from([(200, Vec3::X)]), true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut ends = 0;
        while (engine.stats().started < 2
            || !playback.foreign.is_empty()
            || playback.preview.is_some()
            || !playback.jobs.is_empty())
            && std::time::Instant::now() < deadline
        {
            ends += usize::from(playback.update(Some(&engine), 10, None));
            let _ = engine.take_finished();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(engine.stats().device_error, None);
        assert_eq!(
            engine.stats().started,
            2,
            "zero-gain foreign was actually admitted"
        );
        assert_eq!(ends, 1, "only the own slot sends End");
        assert!(playback.foreign.is_empty() && playback.pending.is_empty());
    }
}
