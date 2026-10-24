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

## Remaining integration

The native kind-2 attachment enables action 19, `fist_ac_sit_down`: start, held loop, then
`fist_ac_sit_up` on exit. It disables rotation; the loop holds position. Performance buffs
7011/8216 choose separate piano/drum sequences. Sitting buffs 5068/8079 have no pose of their own.

The app still needs received `DoodadBonds` presentation, phase/root readiness gating,
local and remote seated pose sequencing, movement/yaw-triggered unbond, and pending local
detach state after an accepted `UnbondRequested`. Received history must not immediately
reseat the actor after that local detach. Unknown own history must fail closed; mounts
retain their separate gate. No new Space/Escape trigger, camera offset or seat-height
constant is justified by the retained evidence. Live rendering/ack behavior and precise
root motion remain unverified.


## Verified checkpoint

All five instrument catalog cases and three original prefab/chair helper matrices pass.
A synthetic rotated/nonuniformly scaled parent chain preserves the signed helper name and
full affine. Ordered source tracking now invalidates remove/recreate and replacement snapshots;
stale worker completions cannot clear the current preparing flag. Actual phase puzzle/item,
data and owner receipts fold into the stored source exactly as the network reducer does,
so ordinary updates preserve the old geometry until a successful replacement is ready.
General non-doodad Unit worker identity remains outside this change.

The consistent-source client suite passes 633 checks with 63 optional ignores. Six affected
doodad checks pass after the final whitespace-only Clippy corrections; the five selected
instrument checks also pass. One scoped Clippy run completed with existing warnings and six
new assignment-spacing suggestions, all corrected without a second Clippy invocation.
Independent review round 03 found no remaining P1/P2 in this bounded prerequisite.

The main client, prefab example and offline world caller rebuild successfully. Matched
1280x720 controls show the former brush-only omission and all three complete layouts. Final
logs contain zero warnings/errors, including shader-import errors; the fixture reports one
visible floor mesh before and nine visible mesh entities after, with zero pipelines waiting.
FPS samples range 273.1?512.0 before and 391.1?501.4 after under shared load; no causal
performance conclusion is drawn. These are geometry controls, not native scene lighting
or seated-character captures.

Reproduce with `cargo run -p aa-client --example doodad_prefabs -- --shot <png> --shot-after 12`;
add `--before` for the same-binary old lookup control. Source/binary/PNG hashes, pixel delta,
commands, counters and exact log paths are in `tools/evidence/doodad_seats/prefab-validation.json`.


Cleanup of the two task-owned QA executable copies was rejected by automatic approval
review with ?blocked by policy,? before the deletion process started. The copies remain
under `data/local/resume/`; no retry or alternate cleanup method was attempted. Required
screenshots and logs are retained intentionally.
