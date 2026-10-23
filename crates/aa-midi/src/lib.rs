//! Instrument music of the 1.2 client.
//!
//! * [`mml`]: the score text of the composition window ("C D E R4 O5 T120 ...") to a standard MIDI file, the same
//!   bytes of the original `x2game.dll` compiler (`FUN_390838a0`), with defensive limits.
//! * [`smf`]: reads such a file back into timed events.
//! * [`dls`] and [`synth`]: the client plays that MIDI file with FMOD and `sounds/default.dls` (Roland GM bank in
//!   the DLS level 1 format); here the file is rendered to PCM from the same bank.
//!   This is an approximate software renderer, not FMOD output. See `docs/aa-midi.md` in the repository.

pub mod dls;
pub mod mml;
pub mod smf;
pub mod synth;
