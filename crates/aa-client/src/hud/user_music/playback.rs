//! Bounded worker rendering into the existing audio engine, with generation cancellation.
use aa_audio::{Category, Engine, PcmClip, PcmOptions, Voice};
use bevy::prelude::*;
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
    generation: u64,
    result: Mutex<mpsc::Receiver<Result<PcmClip, String>>>,
}

#[derive(Resource, Default)]
pub(super) struct Playback {
    generation: u64,
    jobs: Vec<Job>,
    preview: Option<Voice>,
}

impl Playback {
    pub(super) fn clear(&mut self, engine: Option<&Engine>) {
        if let Some(voice) = self.preview.take()
            && let Some(engine) = engine
        {
            engine.stop(voice, false);
        }
        // Workers finish into their own one-result channels; stale clips are discarded.
        self.generation = self.generation.wrapping_add(1);
    }

    pub(super) fn update(
        &mut self,
        engine: Option<&Engine>,
        generation: u64,
        midi: Option<Vec<u8>>,
    ) {
        if self.generation != generation {
            self.clear(engine);
            self.generation = generation;
        }
        self.jobs
            .retain(|job| match job.result.lock().unwrap().try_recv() {
                Ok(result) => {
                    if job.generation == self.generation {
                        match result {
                            Ok(clip) => {
                                if let Some(engine) = engine {
                                    // Player score uses the music slider; native MIDI mixer parity is unclaimed.
                                    match engine.play_pcm(&clip, PcmOptions::new(Category::Music)) {
                                        Ok(voice) => self.preview = Some(voice),
                                        Err(error) => warn!("user music: PCM submission: {error}"),
                                    }
                                }
                            }
                            Err(error) => warn!("user music: render: {error}"),
                        }
                    }
                    false
                }
                Err(mpsc::TryRecvError::Disconnected) => false,
                Err(mpsc::TryRecvError::Empty) => true,
            });
        // At most two render buffers/retained clip results, including canceled work.
        if let Some(midi) = midi {
            if self.jobs.len() >= 2 {
                warn!("user music: render workers busy");
                return;
            }
            let (sender, result) = mpsc::sync_channel(1);
            let job_generation = self.generation;
            match std::thread::Builder::new()
                .name("score-render".into())
                .spawn(move || {
                    let _ = sender.send(render(&midi));
                }) {
                Ok(_) => self.jobs.push(Job {
                    generation: job_generation,
                    result: Mutex::new(result),
                }),
                Err(error) => warn!("user music: worker: {error}"),
            }
        }
        if self
            .preview
            .is_some_and(|voice| engine.is_none_or(|engine| !engine.is_playing(voice)))
        {
            self.preview = None;
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
        playback.update(Some(&engine), 10, None);
        assert!(playback.jobs.is_empty());
        assert!(
            engine.take_finished().is_empty(),
            "stale PCM was never submitted"
        );
        playback.jobs.push(ready(10));
        playback.update(Some(&engine), 10, None);
        assert_eq!(
            finished(&engine).len(),
            1,
            "current PCM uses existing muted completion"
        );
        assert!(playback.preview.is_none());
        playback.jobs.push(ready(10));
        playback.update(Some(&engine), 11, None);
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
            .play_pcm(&clip, PcmOptions::new(Category::Music))
            .unwrap();
        assert_eq!(finished(&engine).len(), 1);
        assert_eq!(engine.stats().cache_bytes, 0);
    }
}
