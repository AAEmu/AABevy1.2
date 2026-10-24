# Placed instrument geometry and seating

Native reference: `tools/evidence/doodad_seats/` retains the original helper lookup,
attachment-action instructions and asset matrices. These establish isolated native behavior
and authored transforms, not a live native scene or a successful seated remake session.

## Geometry prerequisite

The old doodad model lookup selected the first `object_Model` property from a prefab.
The piano and drum prefabs use Brush parts instead: the instrument body and a separate chair.
The raw phase reference must therefore survive database resolution and worker preparation.
`ingame/doodad_model.rs` prepares every mesh part with its material and placement, then
spawns the layout under one root. Ordinary single-file models retain their existing root.
Missing parts prevent publication of a partially assembled layout. Phase changes keep the
old geometry until replacement succeeds and track the phase whose model is actually present.

Bond helpers are retained in prefab-local Cry metres by multiplying the part transform by
the CGF node's complete parent chain. The piano body has its own 90-degree rotation; that
rotation does not affect the chair's `$bond0`. Actual authored origins are:

| Prefab | Helper origin in Cry metres |
|---|---|
| `furniture.piano_a` | (-0.080078125, -1.2835332, 0) |
| `furniture.piano_b` | (-0.080078125, -1.3024902, 0) |
| `furniture.percussion_a` | (-0.14785767, -0.41488084, 0) |

These numbers are test expectations from the authored matrices, never runtime fallback offsets.
Native helper selection formats `bond{spot}` using the signed spot; it does not substitute
attach point 2 or a default spot. Attachment uses the helper's world translation/rotation
with scale one, and falls back to the doodad world transform when a helper is absent.
Full native CGF-to-layout import execution remains an evidence boundary.

## Native-derived seating consumer

The native kind-2 attachment enables action 19, `fist_ac_sit_down`: start, held loop, then
`fist_ac_sit_up` on exit. It disables rotation; the loop holds position. Performance buffs
7011/8216 choose separate piano/drum sequences. Sitting buffs 5068/8079 have no pose of their own.

The app folds `DoodadBonds` in receipt order throughout selection/loading/gameplay, rather
than reading a future frame-final bond. A seat waits for the current owner's complete
rendered phase and helper data. Actors remain independently owned, so owner destruction
cannot recursively despawn them. Initial attachment strips world scale once and stores
`inverse(parent) * actorWorld`; subsequent parent motion follows that affine without
resampling the helper or restarting the pose. Source loss preserves the actor's last world
pose. Rebinding across a pending phase is a coherent application policy; full native
hierarchy migration remains unproved.

Movement and character-yaw intent are read before the seated motion lock. Eligible existing
cast stops precede loot-close and the captured unbond command. Enqueue alone does not detach:
only a matching successful `UnbondRequested` suppresses that captured presentation receipt.
Received history stays intact, and a fresh accepted bond can seat again. Input exits run
only during actual in-game input, including the existing teleport input gate. Unknown own
history fails closed; mounts retain their separate gate. No universal Space/Escape trigger,
camera offset, chair-height adjustment, buff removal, or music End is invented.

An obsolete completion cannot retire a newer visual receipt. Its actual completed write
still folds through the same protocol reducer as the session: the app cannot retract a
real write by privately ignoring its receive-gate change. An accepted equal bond over an
already occupied presentation preserves the visual serial and any newer exit capture.
An accepted changed bond is fresh authority and updates the next full capture. A same-owner
old SC13A remains indistinguishable on wire; local counters do not create a wire epoch.

Remote movement received after an accepted release resets the old interpolation track at
that exact message boundary and survives the world-pose handoff. Pre-release reports do
not drag a detached actor back to an old pre-seat path. Seat retirement precedes authoritative
server teleport placement. Worker results also carry a unit source generation: changed
actor/model identity and remove/recreate reject old preparations before touching the current
worker flag. Same-identity health/position refreshes preserve the existing visual.

The base action uses the model's own CAL-resolved start/loop/up clips through the existing
locked binder. Six race/gender CALs have stationary roots; their pelvis and child bones
produce sitting. Native animation-driven root-reset instructions were executed separately.
This supports the bounded base presentation, not native graph/FIFO/transition, render-location
smoothing, IK or camera parity. Actual performance actions keep ownership of the animator;
when they end, an attached actor returns to the retained seat loop. No piano/drum performance
is inferred from the seating buffs alone. Hariharan drum-start lookup and some performance
loop seams remain open in the retained root-motion report.

Forced movement distinguishes physical parenting from logical bond history. The original
Blink position setter refuses a physically parented actor; a logical bond alone does not
refuse translation. The app declines that parented translation and movement report without
sending unbond. Blink's separate native local-rotation write remains unsupported.

Only successfully admitted kind-1/2/4 controllers supply the native activity condition for
seat exit. Their activity survives the player-input lock, preserving eligible primary and
secondary cast stops, loot-close, then captured unbond ordering. Pending or rejected release
holds this app's solver clock and movement reports. A matching completion retires the seat
before the solver resumes. This asynchronous transport hold is an application approximation;
the original global controller/input frame order is not proved. Matched Float buff removal
ends controller activity even while its approximate falling phase remains.

Knock/impulse does not establish that controller activity. Its downstream parented physics
is unproved, so the app declines the bound speculative flight while preserving damage,
buffs, bond history and existing action poses. This is a declared limitation, not a proved
native impulse-position rule. Remote development simulation gains no local unbond authority.
No live seating/ack or matched native seated-scene claim follows from the offline fixture.


## Geometry prerequisite checkpoint (`eab634c`)

All five instrument catalog cases and three original prefab/chair helper matrices pass.
A synthetic rotated/nonuniformly scaled parent chain preserves the signed helper name and
full affine. Ordered source tracking now invalidates remove/recreate and replacement snapshots;
stale worker completions cannot clear the current preparing flag. Actual phase puzzle/item,
data and owner receipts fold into the stored source exactly as the network reducer does,
so ordinary updates preserve the old geometry until a successful replacement is ready.
General non-doodad Unit worker identity was outside that prerequisite; the seating consumer above adds the actor/model generation guard.

The consistent-source client suite passes 633 checks with 63 optional ignores. Six affected
doodad checks pass after the final whitespace-only Clippy corrections; the five selected
instrument checks also pass. One scoped Clippy run completed with existing warnings and six
new assignment-spacing suggestions, all corrected without a second Clippy invocation.
Independent review round 03 found no remaining P1/P2 in this bounded prerequisite.

The main client, prefab example and offline world caller rebuild successfully. Matched
1280x720 controls show the former brush-only omission and all three complete layouts. Final
logs contain zero warnings/errors, including shader-import errors; the fixture reports one
visible floor mesh before and nine visible mesh entities after, with zero pipelines waiting.
FPS samples range 273.1 to 512.0 before and 391.1 to 501.4 after under shared load; no causal
performance conclusion is drawn. These are geometry controls, not native scene lighting
or seated-character captures.

Reproduce with `cargo run -p aa-client --example doodad_prefabs -- --shot <png> --shot-after 12`;
add `--before` for the same-binary old lookup control. Source/binary/PNG hashes, pixel delta,
commands, counters and exact log paths are in `tools/evidence/doodad_seats/prefab-validation.json`.


Cleanup of the two task-owned QA executable copies was rejected by automatic approval
review with "blocked by policy," before the deletion process started. The copies remain
under `data/local/resume/`; no retry or alternate cleanup method was attempted. Required
screenshots and logs are retained intentionally.


## Reproduction and evidence

The integrated client suite passes 664 tests with 67 optional checks ignored. The affected
in-game run passes 213 checks, including whole-plugin initialization and five cross-system
movement cases. After the single final Clippy run's new collapsible-match suggestion was
applied, all 21 affected seat checks passed again. Clippy exited successfully with existing
warnings; it was not rerun. Review round 04 found no concrete introduced P1/P2 issues by
static inspection. Review is not an independent execution result.

Both final callers build. All six source-matched 1280x720 captures were inspected: piano
and drum controls, helper seating, the side view, and local completion returning to a
standing pose at the retained world position. Each log has zero application/shader warnings
or errors and zero waiting scene pipelines before the screenshot request. Piano FPS was
403.5-482.6 before and 365.9-479.4 seated; drum FPS was 391.7-474.3 before and 380.2-476.0
seated. These shared-machine samples establish no causal performance change. Diagnostic
lighting and cape behavior do not establish native scene parity.

The rebuilt main client also rendered the login screen with isolated settings, zero script
errors and a successful screenshot acknowledgement. Its startup constructs the explicit
64-field FX Wind opt-in; this is not a live Wind-response or vegetation-reception test.
The latter remains deferred on the native recurring cache invalidation boundary R2.

`tools/evidence/doodad_seats/client-seating-validation.json` records the exact source,
binary, PNG and log hashes, commands, FPS samples, review outcomes and remaining limits.
Automatic approval review rejected the one exact-file cleanup attempt for
`data/local/resume/seating-qa-20261023.exe` with "blocked by policy" before launch. The
114,563,072-byte copy remains; no retry, overwrite or alternate removal was attempted
after that rejection. Previously blocked scratch was untouched.

`cargo run -p aa-client --example doodad_seating -- --shot <png> --shot-after 20`
uses the original piano prefab and Nuian model with synthetic selection/spawn/bond receipts.
`--before` omits the bond in the same binary; `--drum` selects the original drum layout;
`--side` changes the diagnostic camera. `--unbond-after 12` explicitly simulates the send
completion for the production captured request. There is no socket or fabricated live-server
success in this fixture. The capture driver rejects warnings, shader errors and unsettled
scene metrics before the screenshot request, then requires a successful save acknowledgement.
Screenshot capture can request its own new pipeline after that boundary.

`python -B tools/doodad_seat_lifecycle_oracle.py` replays 50 pinned original-instruction
lifecycle cases and compares the entire retained result. `python -B tools/doodad_seat_root_motion_oracle.py`
replays 16 root-policy cases, including six actual CAL loop samples, against retained results.
Both require the pinned local original modules and the existing Python RE dependencies;
they write only ignored replay output and never load original DLLs into a live process.
`python -B tools/doodad_seat_forced_movement_oracle.py` replays all 46 original
position/controller/input-tail cases, verifies the 17 prior evidence files, and compares
the complete result with its retained golden. Their mock boundaries, asset samples and
source hashes are under `tools/evidence/doodad_seats/{lifecycle,root_motion,forced_movement}/`.
Those hash-pinned raw export folders preserve their original bytes through local Git
attributes; staged blobs were checked byte-for-byte. Their disassembly whitespace is
intentional and is not normalized along with application source.
