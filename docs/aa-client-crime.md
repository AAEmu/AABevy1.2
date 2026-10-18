# Received crime and jury HUD

The original X2Player.GetCrimeInfo reads the selected character header's signed
crime-point i16, infamy i32 and crime-state i16. The HUD now returns its exact
three named fields crimePoint/crimeRecord/crimeScore with native f32 numeric
conversion, independent of whether the spawned actor currently exists.
GetJuryPoint reads the received signed header field with the same conversion.
The original character labels retain their own50-point colour threshold and
the original jury label's point-minus-one display.

Before a selected header exists, native GetCrimeInfo dereferences an invalid
pointer rather than defining a default. The app safely returns an empty table
in that startup domain so original constructors can skip unknown fields; it
does not manufacture zero reputation. GetJuryPoint returns zero without an
active client, matching its explicit native null-client branch; a selected
identity with no known header field returns no values. These startup accommodations
are not additional native header behavior.

Each received SC16f update assigns all three fields before CRIME_REPORTED,
then forwards the three native-width wrapping deltas. Duplicate updates still
dispatch zero deltas. SC18a assigns the jury point before PLAYER_JURY_POINT();
SC182 forwards JURY_WAITING_NUMBER with signed f32 conversion. Queue numbers
remain event data rather than a fabricated persistent native getter. Receipt
tracking continues before the HUD exists; callbacks run only for an active HUD.

The original jury icon's OnEnter calls X2Trial.RequestJuryWaitingNumber. The
binding returns no values and queues the current nonzero selected character ID.
The real NetCommand writer rechecks that ID and connection/header context before
sending. Authentication, world/character changes, leave and disconnect discard
old queued requests, including reentry with the same ID. aa-net then requires
the actual selected identity, InWorld phase and a successful transport write.
The captured ID is a local guard: native CS079 has an empty payload. No request
changes received points, invents queue eligibility or fabricates a response.

Source anchors are native GetCrimeInfo39496420, GetJuryPoint394969c0,
SC16f handler391dd1cb..391dd25d and request39078410→3907c770→39186810.
aa-net's docs and data/extract/aa-net/native-crime-* retain packet, callback and
executed wrapping-oracle evidence. Trial choices, jury service, court progression,
crime reporting and paired live settlement remain separate open integrations.

All four focused checks pass: exact keys/rounding, assignment-before-callbacks
and duplicates, unknown/reset state, captured request identity and the actual
scheduled NetCommand writer. Full client249 passed/38 optional probes ignored
in73.99s. The separate original character-panel physical jury-hover probe passes
in2.36s, asserting the actual localized tooltip line and hide-on-leave behavior.

Original1920x1080 character and live world-list captures were inspected at
305.3-404.6 and337.6-378.6 FPS respectively, self-closing with no WARN/ERROR/panic.
The initial waiting capture lost its hover during app input; the visual-only
fixture now uses the toolkit's built-in virtual pointer in its normal mouse,
script and mesh-generation step. Each original icon-enter request receives an
explicit fixture response on the next frame, without moving the system cursor
or using the network transport.
The refreshed waiting capture is inspected at249.0-287.3 FPS with the original
"Jury Standby Order:17" tooltip visible. It also self-closes without warnings,
errors or panics. Captures are shots/hud/crime/{character,waiting,world-select}.png.

The checked main target/debug/aa-client-20261018-crime.exe has SHA256
4fa976b3bab7f4a8e1a61850671a68c567d91f3d0351b000359311bd578b3119.
After checking no client was running, these same bytes replace the playable alias
target/debug/aa-client-play.exe. The corrected preview is separately retained as
target/debug/aa-crime-20261018-v4.exe; its changes affect only the visual fixture.

Screenshot review also exposes the toolkit's existing unimplemented inline
`|d` dishonor token: the original infamy label shows that literal prefix.
Its native rich-text audit is queued with aa-ui; scripts and labels are not
rewritten to conceal it. Other character-stat fallbacks remain outside this slice.
No live in-world crime/jury settlement is claimed; the lane connection stall
remains open. Reproduce the visual example with:

```powershell
cargo run -p aa-client --example crime -- --size 1920x1080 --shot shots/hud/crime/character.png --shot-after 8
# --waiting physically hovers the original jury icon and injects an explicit waiting-number receipt.
```
