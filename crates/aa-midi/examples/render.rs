//! Deterministic, device-free PCM regression artifact from the original bank.
use aa_midi::{dls::Dls, mml, smf, synth};
use std::{error::Error, io::Write, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "crates/aa-midi/evidence/flute.wav".into());
    let bytes = aa_pak::Pak::global().read("sounds/default.dls")?;
    let bank = Dls::parse(&bytes)?;
    let midi = mml::compile("O5 T120 L8 C E G > C", mml::Voice::melodic(73), 1.0, 7)?;
    let song = smf::parse(&midi)?;
    let pcm = synth::render(&bank, &song, 0.5)?;
    let peak = pcm.iter().fold(0.0f32, |p, s| p.max(s.abs()));
    let rms = (pcm.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
    assert!(pcm.iter().all(|s| s.is_finite()) && peak > 0.05 && peak <= 1.0);
    let data_bytes = u32::try_from(pcm.len() * 2)?;
    let path = Path::new(&output);
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut wav = std::io::BufWriter::new(std::fs::File::create(path)?);
    wav.write_all(b"RIFF")?;
    wav.write_all(&(36 + data_bytes).to_le_bytes())?;
    wav.write_all(b"WAVEfmt ")?;
    wav.write_all(&16u32.to_le_bytes())?;
    wav.write_all(&1u16.to_le_bytes())?;
    wav.write_all(&1u16.to_le_bytes())?;
    wav.write_all(&synth::RATE.to_le_bytes())?;
    wav.write_all(&(synth::RATE * 2).to_le_bytes())?;
    wav.write_all(&2u16.to_le_bytes())?;
    wav.write_all(&16u16.to_le_bytes())?;
    wav.write_all(b"data")?;
    wav.write_all(&data_bytes.to_le_bytes())?;
    for sample in pcm {
        wav.write_all(&((sample * 32767.0).round() as i16).to_le_bytes())?;
    }
    wav.flush()?;
    println!(
        "{}: rate={} mono PCM16, samples={}, peak={peak:.6}, rms={rms:.6}, MIDI={} bytes, {} ms",
        path.display(),
        synth::RATE,
        data_bytes / 2,
        midi.len(),
        song.length_ms()
    );
    Ok(())
}
