# aa-midi: instrument score backend

Owns MML compilation, bounded SMF reading, the original DLS1 bank reader, and
offline mono PCM synthesis. No Bevy, audio-device, Lua, network, or client dependency.
Client composition/playback windows, proficiency, inventory, packets, remote-player
positioning, mute/volume, cancellation, and audio submission remain aa-client work.
Mount work is outside this task.
The orchestrator can add `aa-midi = { path = "../aa-midi" }` to aa-client's
dependencies; no workspace dependency or lockfile changes are part of this commit.

## Public API for integration

```rust
use aa_midi::{dls::Dls, mml, smf, synth};

// Read/cache once through aa-pak in the owning client, off the frame thread.
let bank = Dls::parse(&pak.read("sounds/default.dls")?)?;
let midi = mml::compile(text, mml::Voice::melodic(program), quality, seed)?;
let song = smf::parse(&midi)?;
let pcm: Vec<f32> = synth::render(&bank, &song, 0.5)?;
// pcm is mono at synth::RATE (24_000 Hz), finite and within -1..1.
```

- `mml::Voice { program: u8, channel: u8, octave: i32 }`,
  `Voice::melodic(program)`, and `Voice::PERCUSSION` (35, 9, 3).
- `mml::compile(&str, Voice, quality: f32, seed: u32) -> Result<Vec<u8>, mml::Error>`.
  Errors are `Code(String)`, `Empty`, `TooLong`, and `InvalidParameter(&'static str)`.
  Use quality 1 for preview/perfect performance; the caller supplies proficiency
  quality and time/random seed for actual playing. Seed restarts per track.
- `mml::MAX_CHARACTERS = 1000`, `mml::MAX_BYTES = 32768`.
- `smf::parse(&[u8]) -> Result<Song, smf::Error>` for remote SMF payloads; do not
  recompile MIDI bytes as score text. `Song { tracks, events, duration_ms }`,
  `Song::length_ms()`, `Event { ms, channel, kind }`, and `Kind` are plain data.
  Equal-time events retain track/file order. Duration includes end-of-track silence.
- `Dls::parse(&[u8]) -> Result<Dls, dls::Error>`; waves share PCM using `Arc<[i16]>`.
  `Dls::instrument(program, drum)` selects GM bank zero and drum bank zero.
- `synth::render(&Dls, &Song, tail_seconds: f32) -> Result<Vec<f32>, synth::Error>`.
  `TooLong` is explicit; there is no silently truncated success. Other invalid
  inputs return `Invalid(&'static str)`. This final signature replaces the stopped
  worker's unchecked `Vec<f32>` return.

Do parsing/rendering on a worker, with client-side request deduplication and a
bounded playback/cache policy. The crate bounds each call; it does not own a queue
or concurrent-render budget. Reuse the parsed bank. Submit PCM through the existing
audio API, and let the client handle stopping and spatial gain.

For the original windows the client still needs to implement `X2UserMusic`'s
`PreSaveNotes`, `SaveNotes`, `TestPlayNotes`, and `StopPlayNotes`, plus the existing
composition-limit queries/events. This crate supplies their backend, not Lua methods.
The original scripts are `x2ui/music/{music,user_music_view,percussion_view}.alb`.
`OPEN_MUSIC_SHEET` opens the composition view; `CLOSE_MUSIC_SHEET` closes it.

## Native compiler evidence

Original `D:/aa-re/client12/x2game_unpacked.dll`, image base `0x38ff0000`, SHA-256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
The stopped worker's decompilations remain under `data/extract/ghidra/out/music*.c`.

| Native address | Observed responsibility |
| --- | --- |
| `39083960` | UTF-8 wrapper, equipment/program selection, proficiency quality, accepts result sizes 1..0x8000 |
| `390838a0` | Top-level compiler; up to three comma-separated tracks |
| `39081d90` | Split into three fixed 1000-codepoint buffers; leading empty track terminates track count |
| `390828b0` | Initialize velocity 100, quarter 500, octave 5, instrument/channel; percussion override octave 3 |
| `39083690` | SMF headers, channel-0 program changes, track sizes and end markers |
| `39083480` | Tokenize a complete track, code-error snippet, `srand(time(NULL))` per track |
| `39081860` / `390817d0` | Tokenizer and numeric ranges |
| `39081cf0` / `39081d50` | Note length/dots and pitch |
| `39082e60` / `39082a30` | Notes, controls, ties and final tie flush |
| `390816a0` | Delta VLQ and note-on/note-off byte writer |

`tests/native_oracle.py` executes the **original x86 compiler instructions** in
Unicorn and emits `tests/native.tsv`. It does not load the game into a process or
contact a running client. Runtime allocation/vector copying, CRT calls, character
helpers and error-message UI are stubbed; tokenization, state changes, splitting,
timing arithmetic and MIDI emission execute native instructions. The equipment
octave override is injected after initialization. Error fixtures stop at the
native code-error UI call, before displaying a dialog. Existing Python packages
`pefile` and `unicorn` are RE-only prerequisites, not Rust dependencies.

The 54 retained cases cover literal native bytes, multilingual notes, all three
tracks, leading/interior/trailing empty tracks, state carryover, tempo rounding,
repeated dots, rests, ties/ignored N, missing octave number, invalid ranges,
integer accumulator wrapping, NUL termination, drum channel/program bytes and
seeded imperfect playing. Empty/no-note and malformed-header differences are
asserted explicitly. Regenerate with:

```
python crates/aa-midi/tests/native_oracle.py --fixtures
cargo test --locked -p aa-midi --test native
```

Character-helper RE: `xlcommon.dll` exports `XlIsSpace(uint)` at `3301a990` (only
ASCII space/tab/CR/LF/VT/FF), `XlIsDigit(uint)` at `3301a890` (ASCII digits), and
`XlToUpper(uint)` at `3301a9f0` (ASCII table, CRT `towupper` through U+07FF,
unchanged above U+07FF). The oracle stubs the ASCII cases and preserves other
codepoints. Rust uses Unicode uppercase below U+0800; unusual non-ASCII CRT locale
case mappings in that range have not been exhaustively compared.

Confirmed details that differ from generic MML implementations:

- Case-insensitive letter notes; Korean, Japanese, and simplified Chinese
  `哆来咪发梭拉西`. `來` and `發` are not native note names.
- Number ranges: note/rest/default length 1..64, octave 0..10, velocity/ignored N
  0..127, tempo 32..255. Unknown characters are skipped. Numeric accumulation
  wraps as x86 i32 before range validation. Error context begins at the token
  input position and keeps up to 14 characters; native localized byte conversion
  may shorten non-ASCII snippets further.
- 500 PPQN, fixed default MIDI tempo 500000 us/quarter. Tempo commands scale
  note/rest deltas, using truncated native f32 arithmetic. `T137 L7 C` is not
  calculated with a conventional tempo meta event. Default tempo is 120 BPM;
  the localized help's T100 wording is not the native default.
- Each dot multiplies the current integer length by 1.5 and truncates. Rest dots
  are ignored. Missing `O` number assigns -1, so the next low note clamps to zero.
- Same-pitch ties combine, different pitches remain sequential. N does not flush
  ties. Other controls flush before changing state. Trailing rests are discarded.
- State carries across tracks; time restarts at zero. A truly empty track stops
  the track list; whitespace-only tracks instead fail for no emitted notes.
  Text after the third comma is ignored.
- Program changes are always on channel 0, even for percussion; percussion notes
  use channel 9. Thus the native emitted program byte 35 does not change the drum
  channel's default kit. The renderer preserves this behavior.
- Native seeded wrong notes use MSVC `rand`, two draws per emitted note when
  quality is below 1, a threshold from `rand()%10000`, and offset `rand()%15-7`.
  Notes clamp to 0..127 after the offset.

## Original score limits and intentional defensive differences

Original Lua `user_music_view.alb`: title editor limit 12 characters, score editor
limit 1000. Saving also checks current proficiency and nonempty title/text; preview
only checks nonempty text. The read-only server SQLite `music_note_limits` gives:

| Proficiency step | 0 | 1 | 2 | 3 | 4..10 |
| --- | --- | --- | --- | --- | --- |
| Maximum score characters | 200 | 400 | 600 | 800 | 1000 |

`PreSaveNotes`/`SaveNotes` (`390799e0` / `39079640`) compare score length with the
step limit before compilation. The client must enforce those save limits and
select equipment/proficiency; aa-midi has no character or database state.
Native packet serialization caps notes at 12000 bytes (`397bc740`) and MIDI at
32768 bytes (`397b1400`). These are distinct from the editor/proficiency limits.

Defensive differences are deliberate and tested:

- Reject text exceeding 1000 codepoints before allocating/tokenizing, including
  unused later tracks. Native lower-level buffers allow unsafe overflow and its
  UTF-8 wrapper has a separate 3001-wide-element conversion buffer.
- Native empty input / leading empty track can return a 14-byte zero-track header;
  this backend returns `Empty`. Nonempty tracks emitting no notes fail in both.
- Reject note/rest/tie durations outside the four-byte MIDI delta range and reject
  runaway dotted integer lengths instead of reproducing native overflow/invalid
  VLQ output. Voice parameters and quality must be valid/finite.
- Reject renderer durations over 240 seconds including tail, rather than cutting
  a valid long composition short. Supporting longer performances needs a future
  incremental renderer; no claim is made that all 1000-character songs fit.

## Bank, synthesis, and resource bounds

`sounds/default.dls`, read through aa-pak, is exactly 3,440,660 bytes, SHA-256
`3229b09b9d7d9f3f4793b0d9b34fe6abc75cfa4a2503c0c90f43ff651ba7f2c0`:
235 instruments (128 melodic bank-zero programs, variations, nine drum kits),
495 mono signed PCM16 waves, 1498 regions, 439 looping waves. 492 waves are 22050 Hz
and three are 24000 Hz. All observed loops are forward type 0.

RIFF/chunk bounds, padding, header sizes/counts, cue offsets, wave references,
sample-loop ranges, region ranges, mono PCM8/16 format, and region `wsmp` overrides
are checked. Wave `data` may precede `fmt `. Pool-table extended `cbSize` is honored.
The DLS layout/constants/forward release-loop interpretation were checked against
Microsoft's [public DLS1 header](https://raw.githubusercontent.com/microsoft/win32metadata/main/generation/WinSDK/RecompiledIdlHeaders/shared/dls1.h).

SMF limits: 1 MiB input, 64 tracks, 65536 total events including ignored/meta
events, u32 milliseconds. Format 0/1 PPQN only. Invalid statuses, running status,
event data, VLQs, tempo/end markers, declared track lengths and missing EOT fail.
Tempo integration is one stable sorted pass with integer rational time, avoiding
the stopped worker's per-event tempo-map scan and partial-chunk acceptance.

DLS limits: 16 MiB input, 4096 waves/cues, 1024 instruments, 16384 total regions,
16384 chunks per list. Unsupported compression, stereo, DLS2 regions/articulation,
multiple loops and non-forward loops return explicit errors.

Synthesis: 24 kHz mono, at most 5,760,000 f32 samples (~23 MB) per render, 48 voices
with oldest-voice stealing, bounded event count, validated event order and channel
data. Loops wrap by modulo even when pitch advances multiple loop periods per
sample. Each voice caches its own modulation/gain. Sustain 64, volume 7,
expression 11, fixed +/-2-semitone bend, all-notes-off 123, all-sound-off 120,
and controller reset 121 are handled. All-sound-off kills immediately; note-off
and all-notes-off honor sustain. Output duration preserves track-end silence.

This is **not bit-identical FMOD synthesis**. The envelope curves, velocity
response, modulation normalization, gain limiter and control rate are approximate.
Pan is discarded for a positional mono source. Unimplemented connection routing /
transforms, bank-select controllers, RPN bend range, chorus/reverb, region option
flags and phase groups are not FMOD-parity features. Region articulation currently
replaces the instrument articulation; full graph inheritance is not implemented.
Missing drum programs use the nearest lower bank-zero kit (a local fallback policy,
not native-FMOD verified). Gains/modulation are bounded for finite output. Pitch,
non-silence and safe-loop tests verify basic sound, not timbral equivalence.

## Validation (2026-10-23)

`cargo check --locked -p aa-midi` passed. Full focused tests:

```
cargo test --locked -p aa-midi --all-targets -- --include-ignored --nocapture
```

31 tests pass, including all three original-bank tests and 54 native fixtures
inside the compiler regression test. MIDI tests cover cross-track tempo changes,
simultaneous ordering, rational rounding, running-status cancellation, trailing
silence, truncated prefixes, malformed events and resource/duration limits.
Bank tests cover cue reordering, extended headers, region overrides, PCM8/16,
truncated prefixes, invalid counts/references/loops and unsupported formats.
Synth tests isolate channel-control caching, short loops with large pitch steps,
sustain versus immediate all-sound-off, invalid inputs and bounded rendering.
The single permitted `cargo clippy --locked -p aa-midi --all-targets -- -D warnings`
run found five style lints (three collapsible conditions, array chunk iteration,
and indexed output iteration). All five were corrected, then the full test command
above passed. Clippy was not rerun, respecting the shared-box once-per-task rule.

Original-bank scale peaks (program -> peak): 73 -> 0.156208, 24 -> 0.119632,
106 -> 0.181046, 0 -> 0.112048. Each render has 48000 finite samples. Percussion
is also non-silent. Flute autocorrelation finds 436.364 Hz for A4 (440 expected)
and 888.889 Hz for A5 (880 expected), within the test's 4% integer-lag tolerance.

```
cargo run --locked -p aa-midi --example render
```

Produces ignored local `crates/aa-midi/evidence/flute.wav`: mono PCM16 24000 Hz,
36000 samples / 1.5 seconds, peak 0.156208, RMS 0.044430, 65-byte MIDI / 1000 ms.
WAV SHA-256 `94f3b66dab07290498200531c1eee5dc977d3be9c324f76ff7fa4d9e24403cb0`.
The example is device-free and creates no window. No FPS or screenshot claim.
This numerical artifact check is not a listening comparison with original FMOD.

No alternate target, workspace tests, game writes, process kills, client changes,
lockfile edits, or new runtime dependencies were needed. No worker/project copy
was created by this recovery. The pre-existing stopped worker's `proj-misc` is
outside owned paths and was left untouched.
