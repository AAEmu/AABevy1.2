# ArcheAge 1.2 feature parity

## Mount-walk checkpoint, 2026-10-31

`X2Mate:MountMate()` now matches the 1.2 mount controller as read in the 1.2
decompile: the walk ends at `FUN_390aa730`'s arrival window (horizontal squared
distance `0x399A3240 = 0.64`, i.e. 0.8 m, and `-1.5 < dz < 2.5`), the `+0x1d`
flag stops the steering there, and `FUN_3909b070` sends `CSMountMate` on the same
frame; its 3 m gate (`0x39AC3878 = 9.0` squared) is nested inside the arrival
test and always holds inside the window. The reason is `FUN_3909ae90` from the
arrival position. The ride cooldown is stamped at the accepted request
(`FUN_393642f0` line 701802, before the controller at line 701812) and survives a
cancelled walk; the send stamps nothing. Movement cancels only through
`FUN_390aaf30`'s actions 1..5 (`moveback`, `moveleft`, `moveright`, `turnleft`,
`turnright`); `moveforward` and `jump` do not. A mate moving past 25 m mid-walk
does not cancel: the 25 m rule is `FUN_3935aa10`'s refusal at the request
(`0x399CFF68 = 625.0` squared, detail 0x205 "Target is too far."), so 30 m is
refused and a refused request starts no cooldown. The walk has a 10 s life
(`FUN_390a5f00` + `0x3999CCE8`), not the port's former 3 s no-progress rule. The
new-target and teleport cancels remain capture-pending (A1b): the walk functions
have no test for either and neither was measured. A doodad walk and the mount
walk cancel each other, and a cancelled walk never sends.
`hud/pets/owned.rs` owns the state machine, `hud/pets.rs` the movement and
steering, and `hud/pets/owned/tests.rs` the window/cooldown/input/reach/cancel/
despawn/restart/seat/teleport/doodad/arrival and wire checks. The `mount_walk`
example asserts the 30 m refusal, the 0.8 m send, the cooldown-at-accept and the
2.5 m walk through the real binding and systems. Pending live checks: the exact
arrival distance, the native movement-input mapping, a new target and a
teleport mid-walk. Overall original 1.2 parity remains unfinished.

## Farm planting checkpoint, 2026-10-30

The 2026-10-30 original-client capture settled the seed builder's main constants: valid near 30 m; the decompile allows exactly 30 m
(`d^2 <= 900`) and F1 cannot pin the boundary, random yaw for groups 2/3/12, wire order `template, x, y, z, z_rot, scale(1.0), item`, the area text
"Can't be installed here." (0x114 -> `ui_texts` 277), and a 30 s planting reuse lock (`_DAT_39ac7748` = 30,000 ms)
showing "Can't place this for N second(s)." (`ui_texts` 4598), not the earlier 2 s assumption. It starts at the end of
a world load (`FUN_39426520`/`FUN_393a6de0`) and at a teleport arrival (F3's `SCTeleportUnit`), not at a send. `farm_plant_preview` checks the 30 m boundary, sphere/box
overlap, literal texts, ghost and ring offscreen and exits after PASS/FAIL in a plain run. Still open: F2 2D/3D, group
5/24/40 yaw, F5 water site, F6 exact radii, F7 spread, F8-F12 house/farm/zone/labor, F15-F18 harvest/growth, and the
non-random yaw and the other lock trigger events; see `aa-client-doodads.md` "Planting".

## Doodad cutdown capture checkpoint, 2026-10-30

The original 1.2 client's bundled doodad capture confirmed the tree-cutdown interaction: world old birch
template 407 phase 944 offered 13977/15874/11004 on the dynamic action bar; skill 15874 cast for 4000 ms;
`SCDoodadPhaseChanged` to 6251 (`time_left` 180000) preceded the 6-byte `SCVegetationCutdowning` for the same
doodad in one batch; `SCDoodadRemoved` followed 2367 ms after the cut. The client decodes the 6-byte body, runs
`cutdown::start` before the phase swap so the fall keeps the standing model, and finishes its 0.1 s + 2.2 s fall
67 ms before the captured removal (tests pin both). The capture exposed no vegetation transform, so fall
direction, duration/curve, the phase model swap (stump vs lying tree), the beech/oak cut phases and the
axe/loot presentation remain unverified; see `aa-client-doodads.md` "Pending live checks".

## Transport checkpoint (mounts, carts, ships), 2026-10-29

The older checkpoint sections below still say "mounts remain excluded"; those lines are historical and stay as
written. This checkpoint lifts that exclusion for the mount, cart and ship areas below. Most slices pair a capture on the original 1.2 client (r208022) through AAEmu's
client driver with a Bevy replay; the evidence is text under `tools/evidence/` (the two rendering slices below say what
they used instead).

Merged:

- **#31 mount cooldown, dismiss and reach.** The 2 s ride cooldown, dismiss while mounted as `CSUnMountMate` reason 10
  followed by `CSRemoveMate` after the 1000 ms unmount controller, and the 25 m 3D reach with "Target is too far."
  The hidden 1.2 client with the packet trace and the Bevy replay agree on reasons and remove timing (1125 vs 1110 ms);
  the 25.0/25.2 m boundary is a unit test because the Bevy mate cannot follow the capture's GM teleport
  (`tools/evidence/mount_unmount/`). Open: walking to 3 m before `CSMountMate`, the reason 0xb/buff refusals, the
  selected-target/passenger path, and the cooldown stamped when that walk starts.
- **#32 mount skills.** Pet-bar slots send `CSStartSkill` 0x052 in the server's `npc_mount_skills` order with caster
  kind 3 and the mate id, target kind 0/object 0 when there is no target, and refuse on foot or below the mate's level
  with "Can't be used right now." All eight replay cases match the capture: six casts byte for byte and two silent
  refusals (`tools/evidence/mate_skills/`). Open: passenger-seat riding, a self skill with a target selected, cooldown
  display, `GetAutoStartMountSkill`, and `SCErrorMsg` ordering.
- **#33 cart boarding.** A right-click sends `CSChangeTarget` 0x02c when needed and `CSStartInteraction` 0x068; the
  server's first 0x07b skill 12076 binds through the 3.0 m (3.5 m mounted) surface gap and 4.0 m bind check with
  `CSBindSlave` 0x031, and leaving sends `CSDiscardSlave` 0x032. The capture and replay match the chain at
  2.0/2.9/3.1 m and the leave (`tools/evidence/cart_boarding/`); the 3.5/4.5 m native picks never landed. Open:
  auto-walk, passenger 13863 and destroy 19165, per-collider shapes, the dynamic action bar/F, and player flags.
- **#34 mount animations.** Mount reason 1-4 selects the left/right/behind/front get-on, riding uses the idle/walk/run
  bands with the idle-to-run one-shots, and dismount plays `fist_ac_horse_l_getoff_start`/`_loop`/`_end`;
  `SCUnitAttached`'s reason byte is kept for remote riders. The capture supplies the reasons and timing, the clips come
  from the 1.2 graph, and the replay matches the five wire bodies and clip sequences (`tools/evidence/mount_animations/`).
  Open: mounted jump clips, other mount kinds and poses, remote get-on, right-side get-off, and the approximate
  PseudoSpeed scale.
- **#35 cart driving and wheels.** The driver sends `CSMoveUnit` type 2 on the 100 ms pump (resending at standstill),
  velocity on the 30/32767 scale, `angVel.z = 0.4 * steering`, and can leave while moving; wheels spin and
  `Bone_spindle` steers. The replay matches 35 vs 36 still sends, the 4.0 m/s top speed and `0.4` yaw rate, and the
  `0x032` at 3.84 vs 3.58 m/s (`tools/evidence/cart_driving/`). Open: CryPhysics wheeled physics and terrain following,
  per-vehicle speed values, wheel dust/FX, the standstill yaw limit, camera distance, and remote smoothing.
- **#36 vehicle rendering.** Attached doodads draw in the parent frame, the wagon gets its four crates, the `$driver`
  bench seat and the `fist_ac_steer_sit_idle` pose, vehicle parts use full Illum shading, and the rowboat lantern
  places. Measured by 1.2 screenshots at a fixed spot/time against the same views in Bevy (screenshots are local); the
  #36 merge also carries the follow-up `f7670f3d` for seats and mailboxes on doodads attached to a unit, with the
  authored-seat assets and validation in `tools/evidence/doodad_seats/`. Open: the copper boiler, passenger poses,
  other boats with an undecoded item detail, and the crate attach-point offset.
- **#37 material colours.** Non-sRGB `Diffuse` and `Emissive` are linearised with `pow(c, 2.2)` like the 1.2 `.mtl`
  loader; sRGB materials and `Specular`/`Shininess`/`Opacity` are unchanged. Measured by disassembling that loader and a
  Bevy A/B: whole-scene luminance unchanged, town props -1.3%, non-sRGB metal parts about 17% darker (no capture folder;
  loader details in [materials](aa-cry.md)). Open: the copper boiler is still grey, and `r_UseSRGB` is assumed on.
- **#38 dismount placement.** A reason 9 dismount places the rider beside the mount with the captured forward/left
  offset and hop, using the side probe and the 500/1000 ms unmount windows. The replay stays within 0.063-0.065 m at
  0.5 s and at the end, picks the left side, and hops 0.249 vs the captured 0.375 m
  (`tools/evidence/mount_dismount/`). Open: reason 10 travel, right-side dismount, other players' landing, and the
  settle phase/mid-air height.

Ships (merged after the checkpoint was written):

- **#39 ship steering at the helm.** The helm sends `CSMoveUnit` type 4 `{throttle, steering}` on the 100 ms
  pump, resending 0/0 at idle; throttle is ±127 and steering ramps/decays at the ship model's `steer_vel`; the server's
  type 3 decodes with the 30/32767 velocity scale. The 1.2 capture and replay cover idle, forward, reverse, left/right
  and leaving (`tools/evidence/ship_steering/`). Open: larger ships and helm/rudder visuals, the game-option steering
  factor, the sailboat/clipper capture, and remote ship smoothing; deck standing is #40.
- **#40 ship deck standing (builds on #39).** A player on a deck sends `CSMoveUnit` type 1 with the ship support block
  (actor flag 0x20), ship-local position, the relative facing as the primary direction and the world facing as auxiliary
  z; a jump is 0x44/0x34. Leaving the helm keeps the player on deck, and stepping off resumes world movement. The
  capture and replay in `tools/evidence/ship_deck/` (the full capture is #39's `ship_steering/`) match structurally:
  header/actor sets, ship id, cadence and support rows. Open: hull pitch/roll, a raised helm on larger ships, two
  header-only step-off rows, and deck walking/jump values the capture cannot verify.

Still open across these slices: remote vehicle movement smoothing (S8), seats/passengers (S11), cannons/turrets (S12),
hanging (S13), gliders (S14), vehicle state/FX (S15), auto-walk, mount jump clips, larger ships, and the copper boiler
colour; the walk to the mount before mounting is in the mount-walk checkpoint above. The detailed pending checks are in [units](aa-client-units.md) and the PR
descriptions. Cart terrain following (#42), the ship summon space check (#43) and the vehicle camera distance and driver
action bar (#44) followed with their own captures.

## Slider range-notification checkpoint, 2026-10-26

Source `6661a1eb` corrects finite numeric `Slider.SetMinMaxValues`: normalize
the bounds, commit the clamped value, then deliver one changed-value callback
before Lua resumes. Omitted notification defaults true; explicit false and an
unchanged value emit none. Existing hidden/disabled programmatic behavior is
preserved. The generic SetValue and pointer-routing contracts remain separate.

The unchanged original skill-upgrade notification window supplied the actual
regression: after a real wheel displacement of 40, adding another upgrade
previously rebuilt maximum 351 instead of 391, clipping its last entry. The
corrected Runtime path stores 391 and reaches the last entry. The single copied
0.85-scale capture was inspected: its final icon and both lines fit above
Confirm. It exited OS0 with 397.9-456.3 FPS and zero logged engine/panic failures;
startup logged zero script errors. Current host SKILL_UPGRADED delivery is
source-proved; this fixture is not a live packet/protocol equivalence claim.

Native instruction controls passed 20 cases with declared FH representations
and record-only callback/geometry endpoints. Finite reversed-bound controls
and the strict page-then-range callback regression also pass. Fresh independent
R1 is qualified clear. Final UI validation records **728 passed, zero failed,
one existing ignore**, plus two doctests. Its sole scoped Clippy exited OS0
with **142 warning emissions /115 unique diagnostics**, without a clean-baseline
claim. Earlier fixture failures and the unchanged-source mislaunch remain in
the evidence; they are not represented as successful validation.

Current client integration passed **846 tests, zero failures, 91 existing
ignored**; the ignored names match the prior checkpoint. All 1,767 pinned
files stayed unchanged. The single-job main build exited OS0. Its inspected,
muted login-only copied smoke exited OS0 with **207.9-241.7 FPS**,
zero WARN/ERROR/panic lines and zero logged script errors. It did not authenticate
or exercise the live in-game supplier.

Current playable: `target/debug/aa-client-20261026-slider-range-r1.exe`, SHA256
`fa8aa88ca1b5bce7e2a649e26b6b57a163570e442f676c2a54a8fc07942fa9d8`; `target/debug/aa-client-play.exe` matches.
The prior slider/fog executable remains preserved. Exact source pins, commands,
actual exits, ignored inventory, logs, copied-binary and image hashes are under
`data/extract/aa-client/skill-notify-range-20261026/integrated-final-r1/`.
Older checkpoint sections below are historical.

At the retained AG19 native checkpoint, all 2,851 descriptor serializers had
returned. The exact bundled memset ran with an explicitly controlled nonzero
dispatch fixture, followed by one reader continuation. It reached the combined
8M-instruction bound in 14.423 seconds of native time: 32 primary-input returns,
1,086 state returns versus 2,999 allocated state slots, and zero transitions.
Terminal CPU/memory/host state was saved and restore-verified without verification
instructions; the process and mappings were released. The loader has not returned.
The controlled dispatch is not observed OS/CRT initialization. No graph execution,
held-request retry, CAL/enqueue equivalence or action49 lifetime is established.
Evidence: `data/extract/aa-anim/action-consumer-20261025-r1/ag19-memset-f1-preparation/ACTUAL_RUN_REPORT.md`.
No initialized OS result or graph/held-request/action49 behavior is inferred. The Group14 gameplay subject writer, lifecycle authority,
sphere physicalPoseZ and unsupported fog/view mappings remain held. Native FH
classification/ownership, callback-visible thumb geometry and font/pixel parity
are also unproved. Mounts remain excluded; overall original 1.2 parity is unfinished.

## Integrated fog-distance and Slider checkpoint, 2026-10-26

Source `e1111f66` includes the positive-PageStep Slider correction `43aed618`
and authored fog distance culling. Supported perspective draws use their own
camera position and FOV, the transformed world AABB and the authored distance
ratio. Live distance changes retain the node and asset handles; unload removes
the extracted entry. Unsupported projections preserve the previous draw path.
This adds no inferred Group14 subject or camera-owned area marker.

The original Fog4 baseline produced an actual far-view pixel failure. Fresh
World R2 review is qualified clear; final validation records 309 library tests
and four capture-controller tests passed, with 10 existing ignored. All 12
copied-executable GPU runs exited 0: window/offscreen distance, FOV and legacy
controls, two five-stage distance/unload cycles, and box/ellipsoid volume and
water regressions. All 20 saved images were inspected. Restored world pixels
match exactly; changing window labels prevent a full-frame identity claim.
The separate UI checkpoint records 716 passed, one existing ignored and two
doctests passed. Scoped lint runs exited 0 with retained warnings: 116 unique
UI diagnostics and 219 unique World diagnostics, without a warning-clean claim.

The integrated client binary tests passed **846/0 with 91 existing ignored**;
all 496 pinned files remained unchanged. The single-job main build exited 0.
Its inspected muted login-only smoke self-closed OS 0, with zero engine/script
errors and **264.8-281.7 FPS** on the shared machine. No authentication
was attempted; this smoke does not establish live in-game parity.

Current playable: `target/debug/aa-client-20261026-slider-fog-r7.exe`, SHA256
`d8c8fdc79f6b795451687b94bf46b20e2a02eb338cdea8565770b321836b575d`;
`target/debug/aa-client-play.exe` matches. Integrated commands, source pins,
process results and capture hashes are retained under
`data/extract/aa-client/slider-fog-checkpoint-20261025/integrated-final-20261026/`.
Older checkpoint sections below are historical.

Native animation research returned all 2,851 compiled descriptor serializers,
but the reader remains stopped before input/state/transition construction at an
unresolved same-client indirect binding. Graph execution, held-request retry
and action49 lifetime remain unproved. The Group14 gameplay subject flag writer,
sphere physicalPoseZ, later fog activation/replacement and unsupported view
mappings remain held. Mounts are excluded; overall original 1.2 parity is unfinished.

## Positive-PageStep Slider checkpoint, 2026-10-25

`43aed618` restores the unchanged original Sound Options quality slider's
positive-page track press. From quality 2, a right-track press advances once to
3, original Save returns 3, and held pointer movement does not drag the thumb.
The direct Slider receiver uses the authored PageStep and inclusive effective
thumb edges. Zero-page and actual ThumbButton routes retain their existing
behavior; generic setter ABI, callback geometry/order and broader dispatch
remain qualified rather than being claimed complete.

Fresh R1 and R2 reviews are qualified clear. Final UI validation records 716
passed, zero failed and one existing ignored across 100 targets, plus two
passed doctests. The single scoped Clippy exited 0 with 143 emitted warnings,
116 unique warnings and zero errors. No baseline-clean claim is made.

The first visual example launch exposed an asynchronous-runtime readiness
panic (OS 101, no PNG). Its example-only optional-runtime guard was separately
reviewed and rebuilt. The corrected original Sound capture self-closed OS 0,
asserted quality/Save 3 and one callback, and had zero engine/script errors at
387.2-448.8 FPS. Both failure and corrected provenance remain retained under
`data/extract/aa-ui/active-next-20261025-r1/`.

This is a tested UI source checkpoint. The playable binary recorded below was
built at the earlier framehold checkpoint; a new integrated main build is still
required before attaching this Slider behavior to a new playable binary.

## Receipt-ordered framehold and TextBox checkpoint, 2026-10-25

The tested checkpoint is `5a889671`, including `b1456d15` animation start
admission, `65511be5` TextBox physical layout and `05dab691` exact audio handle
observer coverage. The tree was clean after its push. Older sections below are
historical; their remaining-work rows are not current completion claims.

Gameplay framehold rejects the three Animator start operations before mutation.
Manual pause, speed-zero previews and stops remain distinct. The client folds
admitted buff poses and hold changes in receipt order, retains consumed starts
across FX culling, resolves parked bone attachments and maintains passive FX
distance transitions. Gliding gates ordinary starts after ragdoll/knockdown;
end-animation admission is independent of FX visibility. Fresh R6 review is
clear and all 30 focused checks pass, including three reproduced pre-fix
failures. Native physical ragdoll, graph/FIFO arbitration, the protected receive
prelude and unrendered-controller timing remain open.

Original speech bubbles now measure and paint ordinary TextBoxes using the
native physical font/row rules. Auto-height updates survive effective-scale
changes, anchor callbacks and nested direct/transitive dependencies without
new events. Fresh UI R5 review is clear. The original tooltip sample permits
its scale .85 title/status overlap; no padding or ratio fix was invented.
Native rasterization and absolute font origins remain qualified.

Received family titles survive same-source actor refreshes. Accepted bubble
voices use the existing cache and one exact request handle, with cancellation
before fresh-point sampling; cache misses preserve the prior voice. Native
factory failure, FMOD timing/fades and physical sphere `0x153` inputs stay open.

Final checks: client **846 passed, zero failed, 91 existing ignored**; UI
**706 passed, one existing ignored, two doctests passed**; animation **126
library plus one integration passed, one old scan ignored**; audio **66 passed,
37 existing ignored**. One all-target client Clippy invocation exited 0 with
560 unique warnings and no errors; this is not a baseline-clean claim. The
single-job main build passed. Its inspected muted login smoke self-closed OS 0,
with zero engine/script errors at **236.0-275.5 FPS** on the shared machine.
No authentication was attempted.

Playable: `target/debug/aa-client-20261025-framehold-v7.exe`, SHA256
`f9a503db2d586f5a69cb81711f4d3e0a4a764a734b4bdf91ec6a2678f0e052cd`;
`target/debug/aa-client-play.exe` matches. Source pins, exits and capture hashes
are in `data/extract/aa-client/framehold-admission-20261025/`.
Mounts remain excluded; overall original 1.2 parity is unfinished.


## Native terrain, framehold and received affiliations, 2026-10-25

`94968808` and client integration `f5c95920` replace the shared terrain detail
distance with the original option 11 group's independent cliff XY and ground Z
distances: 256/64, 512/128, 1024/128 and 1024/256 m. The native default therefore
changes ground detail to 256 m. Paired live refresh and late admission retain
material handles; invalid distances have independent 1024/256 fallbacks.
Validation includes 303 world checks (ten existing ignored), two fixture checks,
42 original x86 controls and 159 GPU control samples/rechecks. All 96 patch
identities survive quality changes and retire on unload; the restored full frame matches exactly.
Native vertex-stage interpolation, pass ordering and lighting remain open.

`842fa2db` fixes base animation transitions during framehold and zero delta.
Clip speed 0 remains distinct from pause: its transition and upper-body clocks
can still advance. Seven real CAL/CAF/AnimPlugin regressions and 58 native clock
controls plus 600 held invariant updates pass; the crate passes 121 library tests
and one integration test, with one existing ignored. Native animation-start
rejection, live outgoing FIFO blending and LOD accounting remain separate gaps.

`b38330ee` connects the original affiliation radio to the existing account
option 52 and received plate data, including Cancel/Save/restart. Family titles
require the current received object lifetime; empty owner clears, locale and
role formatting use original keys, and replacement/removal/world exit retire
old lines. Received eligible guild rows preserve a known empty name as `<>`.
The corrected client passes 788 tests, zero failures, 91 existing optional ignored.
Its all-targets check and single final Clippy pass; 182 existing client warnings
have no primary diagnostic on changed root lines. Independent R2 review is clear.
The lane has no runtime family-title sender; nation/mode/full option admission
and native plate geometry remain open. Evidence: owning crate notes and
`data/extract/aa-client/affiliation-20261025/`.

`3487cae8` corrects original edit caret endpoint parameters and fractional
multiline glyph alignment. R2 closes a reproduced one-pixel baseline error;
five focused checks, 78 original native cases, 671 UI checks across 95 targets
(one existing ignored), and both doctests pass. Its single Clippy pass retains
27 library warnings and one test-only iterator advisory. The corrected
self-closing diagnostic has zero script/runtime errors at 327.7-421.5 FPS;
native font-origin, blink and width-policy qualifications remain documented.

The final client rerun against the R2 caret code passes 788 tests with 91 existing
optional ignored (104.60 s); its repeated all-targets check passes as well.
The final main build passes. Its inspected idle-login smoke self-closes with
zero runtime WARN/ERROR at 231.7-245.1 FPS; no authentication was attempted.
Playable copy: `target/debug/aa-client-20261025-parity-3487cae8.exe`, SHA256
`fe43543ef0528dec490563648c7aa90dff38c26bce8b2f5682cc4e88b77fd60f`.
`target/debug/aa-client-play.exe` matches. Final source pins, commands/results
and the capture are retained in `data/extract/aa-client/affiliation-20261025/`
and `shots/hud/nonmount-20261025-parity/`.
Overall original 1.2 parity remains unfinished; mounts remain excluded.

## Complete tested source checkpoint, 2026-10-25

The pending client integration is committed in `dcfce412`: guarded NPC services,
auction/battlefield/expedition/pet/service state and the qualified territory
component. Both independent pending-source reviews are clear. Territory live
entry, authoritative origin/population and tax settlement remain open; the NPC
and territory visual fixtures use explicitly synthetic inputs.

`13d70922` separates Window subtree picking from Clickable and Window background
picking, and restores physical-Shift roadmap tooltips/transitions. Its original
HUD regression now verifies that Window picking alone is insufficient before
supplying physical Shift. R2 is clear. `f630f994` adds bounded integer RandomSound
waits and independent X/Y draws, retaining the documented native RNG/FPU and
actor-versus-camera limitations. The existing reference sequences are retained
in `6bb753fe`; all 145 PNGs and 15 pending JSON files passed format validation.

Final frozen-source checks pass:

- Client: **780 passed, zero failures, 91 existing optional checks ignored**
  (103.10 s), plus `cargo check --locked -p aa-client --all-targets`.
- UI: **666 passed across 94 targets**, one existing font-metrics ignore,
  plus **2 doctests**; the corrected HUD target passes all six checks.
- World: **303 passed**, ten existing optional checks ignored. Earlier accepted
  network and animation validation remains recorded in their crate documents.
- Final main, `npc_interaction` and `territory` example builds pass. Their three
  self-closing captures are inspected, exit zero and have no runtime WARN/ERROR.
  Observed FPS: main **259.7-294.7**, NPC fixture **309.5-351.2**, territory
  fixture **350.8-365.4**. No authentication or live service was exercised.

Copied playable snapshot: `target/debug/aa-client-20261025-push.exe`, SHA256
`ed6913998d66e22ff0103526830938926cbad18cad32cd65943642408a0218df`.
`target/debug/aa-client-play.exe` matches. Logs, frozen source inventories,
commit records and visual checks are retained in ignored
`data/local/push-20261025/`. Earlier checkpoints' inherited uncommitted client
work is now committed. Overall 1.2 parity remains unfinished.

## Window input, ambient and legacy-animation checkpoint, 2026-10-25

`84cff99d` allows a resolved received mark to admit an NPC plate whose template
hides ordinary names. Actual plate/mark-system tests verify sprite parenting,
atlas cell, clear/re-mark and far/near/far admission. Existing distance limits,
geometry and other name-tag options remain unchanged.

This caller also includes independently reviewed `2b19f066` Window background
picking (658 UI checks, two doctests, 25 bounded native cases) and `9e46431c`
unbound ambient silence (297 world checks, ten existing ignored). The latter
removes the invented point trigger while retaining diagnostics and bound-area
behavior; its original-cell actual-system regression fails before and passes
after the correction. Mission/zone autoload and mixer parity remain open.

`1019e769` adds legacy CAF 0x827/0x828 decoding and native duplicate-frame lookup;
0x829 behavior remains unchanged. All 98 original ship CAFs decode with zero
skipped controllers. Validation includes 181 bounded native controls, 115 crate
checks with one ignored, and 12 final CAF checks with one ignored after the
equivalent iterator-style correction. Original weighted helm vertices move,
and the inspected ship diagnostic visibly turns the wheel at 215.5-275.8 FPS.
The retained PNG predates only that iterator correction; the final example was
rebuilt and separately hashed. The lower cable is cropped, and full animation,
CDF rendering, sailing controls and physics are not established by this slice.

The client passes **780 tests, zero failures, 91 optional checks ignored**
(105.22 s). The single scoped client Clippy pass exits 0 with 182 existing
warnings. The final main build follows the CAF iterator correction; its focused
CAF tests pass separately. Inspected idle startup exits cleanly with no runtime
WARN/ERROR and **266.6-288.0 FPS**; no authentication was attempted.

Playable snapshot: `target/debug/aa-client-20261025-native-ui.exe`, SHA256
`4cbb48ca3c6f5ae8b2d86bf6b4c82f0de2d651b96b6049d04b6e557d08f36667`;
`target/debug/aa-client-play.exe` matches. Logs, source/dependency boundaries and
artifact hashes are in ignored `data/local/nonmount-20261025/native-ui-checkpoint.json`.
Inherited uncommitted HUD work remains included. The newer Window EnablePick /
Shift-map and RandomSound sampling work is excluded from this snapshot. Three
Sol agents continue those gaps and the next animation slice. Mounts remain
excluded; overall 1.2 parity is unfinished.

## Terrain, lobby and solo-marker checkpoint, 2026-10-25

The original terrain-detail slider now applies option 11's group XY distances
256 / 512 / 1024 / 1024 m to the existing shared terrain fade. The native Z
projection distances and terrain LOD remain unsupported. World implementation
`f1b90374` preserves live patch/material handles and normalizes late admissions;
295 library checks, two fixture-exit checks and the bounded GPU controls pass.
Default numeric rendering is pixel-identical to the prior executable. The
controlled restored terrain region matches exactly, with nine one-byte river
pixels outside that region. See [terrain evidence](aa-world.md#h12-live-terrain-detail-distance-supported-shared-fade-2026-10-25).

Network commit `848b6325` fixes payment-header tail alignment and preserves
received slot/payment/premium metadata (800 checks pass, nine live checks
ignored). The caller copies the optional headers and uses received extra slots
for `GetExpandedCharacterCount` and base-plus-extras current character limits.
It preserves unknown versus zero internally and does not infer entitlements or
replace per-character points. Slot receipt adds no synthetic refresh event.
See [login API details](aa-client-login.md#character-list-metadata).

Received solo overhead marks now reach the existing renderer when the team ID
is zero. Tests cover the actual Lua request without optimistic display, received
team/identity resolution, reassignment, zero/all clears and world exit. Existing
billboard geometry is approximate; NPC hidden-name overrides and full name-tag
options remain open. No new marker capture or live-server receipt is claimed.

The frozen client passes **778 tests, zero failures, 91 optional checks ignored**
(81.93 s). Main build and the single scoped Clippy pass succeed; Clippy reports
182 existing client warnings. Inspected idle startup exits cleanly with no
runtime WARN/ERROR and **236.7-251.6 FPS**. No authentication was attempted.

Playable snapshot: `target/debug/aa-client-20261025-quality-lobby.exe`, SHA256
`e6172f752b404caaef880e6fccfa44761a88cc9671a2ab3f036812c70f236eb2`;
`target/debug/aa-client-play.exe` matches. The ignored
`data/local/nonmount-20261025/quality-lobby-checkpoint.json` records hashes,
receipts and limitations. The build includes inherited uncommitted HUD work,
and excludes the newly active Window-picking implementation. Three Sol agents
continue Window input, legacy non-mount CAF controllers and ambient-area
ownership research. Mounts remain excluded; overall 1.2 parity is unfinished.

## Later non-mount checkpoint, 2026-10-25

`7006e510` adds mouse caret placement, drag/Shift selection, masked source mapping,
release/focus handling and keyboard retirement to the custom login form. The
final caller passes **773 tests, zero failures, 91 optional checks ignored**
(142.05 s). The 35-phase real-form input matrix passes at two resolutions and
UiScale1.1. Its three PNGs and normal idle startup are inspected and clean;
observed FPS is 283.4–387.8 for the fixtures and 237.6–288.3 for startup. Scoped
client Clippy exits 0 with 182 existing warnings. No authenticated session or
retail login pixel comparison is claimed. See [login](aa-client-login.md).

This rebuilt caller also contains `5d436cc7` destination-sector/environment load
retention (293 world checks), `54d48ce3` ordinary ListCtrl paint/input clipping
and `93a19ac6` clear-all selection ordering (651 toolkit checks plus two
doctests). Original dungeon refresh now clears the old selected channel before
replacing its rows. Stable-key and standalone-selection contracts remain.
`1cb6c0b4`/`3639ab9e` supply checked type4 ship-control construction, with all
795 network checks passing and nine live checks ignored; driver/controller and
Session activation remain separate work.

`2ed2f33a` applies authored opacity address/filter state only to the qualified
moving Blend-Illum path, with isolated image variants and unchanged default
consumers. Its 101 unit/seven loader checks, 35 bounded original-instruction
controls, 12 captures and 2,672 pixel samples pass. Native global filtering,
Z/depth-equal/MRT scheduling, registry reclamation and CPU fallback precision
remain open. The owning Clippy attempt fails on existing baseline lints; two
review rounds find no remaining actionable issue. See [materials](aa-cry.md).

Playable snapshot: `target/debug/aa-client-20261025-pointer.exe`, SHA256
`7fcd1fe7f63bc70b910e174136fd677ffe1d6f8863d32f111388c435e6f271e4`.
The build includes inherited uncommitted aa-client HUD work; its source commit
alone does not reproduce the complete tree. The ignored pointer checkpoint
records the input hashes and dependency commits. Three Sol agents continue
lobby headers, verified terrain quality inputs and overhead presentation.
Mounts remain excluded and overall1.2 parity is unfinished.

## Non-mount checkpoint, 2026-10-25

The user's current scope explicitly excludes mounts. Three Sol 6.1 agents remain
assigned to separate UI, world and network work; overall 1.2 parity is unfinished.

`7159fd7b` adds custom-login keyboard selection, replacement, masked selection,
Windows clipboard access and event-ordered physical Ctrl/Shift handling. The
integrated application passes **771 tests, zero failures, 91 optional checks
ignored** (144.21 s), including eight new editing checks. Four inspected,
self-closing login captures pass with no runtime WARN/ERROR. Fixture FPS is
340.6–422.2; normal idle startup is 237.7–281.2 under shared load. Two review
rounds close the discovered modifier-order defect. Mouse selection/IME remain
open; real clipboard success/failure was not exercised by the headless checks.
See [login evidence](aa-client-login.md).

The same rebuilt caller includes:

- `7728317f`: native ListCtrl mouse double-click behavior; 634 toolkit checks
  pass, one existing optional check remains ignored, and both doctests pass.
- `c356ec80`: qualified vegetation LOD1/2 bending retains original LOD0 bounds;
  290 world checks pass with 10 ignored, 24 numeric GPU controls pass, four
  captures are inspected and all 4,217 receiver IDs retire on unload.
- `28791ac2`: checked fixed Hang/Unhang packet construction; 789 network checks
  pass with nine live checks ignored. High-level admission, expected-target
  fencing and controller/physical integration remain open.

The copied playable build is `target/debug/aa-client-nonmount-20261025.exe`,
SHA256 `9f0b333baa77b2b1b8c0457d7b0b8ed96257e58b20881eee029f87102568914a`.
It is built from the current working tree, including inherited, uncommitted HUD
integration; the source commit alone does not reproduce that entire tree.
`data/local/nonmount-20261025/` retains logs, hashes and the pending-source
snapshot. The existing territory component passes all 14 selected checks,
including original controls and guard hover, but normal service entry still
requires qualified spatial/population inputs. No live territory tax settlement,
live hanging or overall completion is inferred from these checks.

## Latest integration check, 2026-10-24

Published checkpoint `f66b0fd1` includes the original-asset modelview host, received
shop feature gate, animation trial metadata and isolated widget-camera selection.
The final client run passes 732 tests with 71 optional checks ignored; the separate
modelview filter passes all 26 checks with its original-asset cases enabled.
Fourteen source-matched captures and normal server-selection startup pass their
assertions and shader/render-log checks. Reset, Clear then Reset and reinitialization
are exact initial-PNG controls. Suspension restores actor identity, revisions and
glyphs; 75 face-region pixels differ without an established cause. Source, capture,
performance and cleanup details are in [modelview inputs](research/modelview-inputs.md).

This closes the bounded preview host, not the local shop's catalog/controller,
wallet/error/archive or legacy web-market dependencies. The current Dominion cache
supports guild eligibility but omits tax/member/alert deltas; territory management
UI remains unimplemented. Neither static catalog metadata nor constructor defaults
create received ownership, money or timing authority.

Later native pass-routing evidence also limits the older masked-material checks:
native successful-Z tracking, depth-equal forward coverage, independent motion and
authored opacity samplers still need production integration. Earlier GPU equalities
remain valid records of their tested implementation, not proof of that complete
native route. Vegetation wind reception remains held on cache boundary R2. Mount
work remains deferred. README's current missing-work list and the dated owning
notes retain the broader unfinished goal.

## Previous integration check, 2026-10-23

The caller now includes frozen Illum alpha source133712d/evidencea1bb831, qualified sea foam
b64f71e and authored high-pass8d97fe4. Public material interfaces remain unchanged. The
production LodIllumMaterial wrapper preserves the full native material token and inherits
the custom prepass. Eighteen real GPU captures pass, with twelve exact RGBA image equalities
across direct/nested materials and ordinary/bindless bindings. Original hammer AlphaTest.059
on a vertex-alpha gradient gives the expected depth/normal bounds(38,26,230,230) at256x256;
constant alpha.02 writes no color, depth or normal pixels. Every capture log is checked for
warnings, lowercase shader errors and unresolved imports. Reproduce with
`cargo build --locked -p aa-client --example material_alpha_integration`, then
`python tools/verify_material_alpha.py target/debug/examples/material_alpha_integration.exe`.
An additional active mesh-pair check uses `--lod-half --output shots/resume/alpha-active-lod`.
The production MeshLodDither tag has level8 and is active; the direct control uses the same
camera distance within Bevy's2..4m fade. Nine captures and six exact image equalities pass.
The masked depth/normal coverage is19584 pixels versus39168 without fading, with the same
alpha-cutoff bounds. The private native token survives nesting in both cases. Combined:
27 captures and18 exact comparisons. These controls verify the actual production wrapper
and inherited prepass, without changing aa-world or aa-cry code.
The active-fade runs observed342.3-474.7FPS under shared load; no isolated speed claim.
The original-asset bow comparison in README is a final-binary token-disabled control against
authored flags, not a matched native-client scene.169 submaterials/15 masks are admitted;
277 permutations and native deferred/shadow parity remain outside this implementation.

The main client and offline world/stored-score callers rebuild successfully. Inspected court,
sea, idle login and original bag captures exit0 without Lua/render warnings or errors. Court
retains16 terrain patches/14visible/4858objectparts/9authored ratios; sea retains64/18/0/0.
Shared-machine FPS ranges are50.4-101.5 court,117.0-253.7 sea,230.5-318.7 idle login and
445.8-474.6 stored score. Loading samples are included; these are not isolated benchmarks.
`--login-screen --mute` leaves the Connect form idle. The other examples create no Session.
Hashes, logs and raw FPS are in `data/local/resume/caller-alpha-validation.json` and
`shots/resume/alpha/validation.json`. White sea/reflection patterns remain visibly approximate.

Native piano/drum overrides preserve equipment/proficiency admission and receipt-local
ordering. Twelve executed original cases and25 focused application checks pass. Actual buff
startup receipts replay after asynchronous catalog readiness; overflow/decode gaps stay
unknown until a fresh own snapshot. The broader client suite passes613 tests/53optional
ignores before the final decode-forwarding guard; the final focused run explicitly covers
that guard, original UI, DLS and device cases. One scoped Clippy run exits0 with175 existing
test-target warnings; none are introduced by the new voice or nested-material code.

Audio's five supported high-pass roots pass93 owning-crate tests and native/captured-mixer
controls. Qualified foam scalar lighting/camera fade has its owning native/GPU controls.
These checks do not establish full FMOD output or native world-scene parity. An isolated
server lane is being prepared for actual composition/playback settlement; that live result
is not yet claimed. Placed-instrument admission is under read-only audit. Mounts stay deferred.

## Earlier recovery checkpoints

Recovery checkpoint, 2026-10-23: the rebuilt client suite passes **604 tests, zero failures, 42 optional
ignores** (85.92 s). Ten focused music/chat checks also pass with original-asset checks included.
These cover original composition controls, bounded PCM preview, confirmed captured cast/upload ordering,
physical chat splitting/history/filter retention, fresh UI reload and allocation-write failure.
The resource-harness failures and hit-particle/material-flash selection are fixed.
UI-data is isolated by character, and receiving a cinema no longer reports unseen playback as completed.
The original salon controls pass an additional all-eight-model appearance roundtrip; fresh live
enter/edit/cancel captures show populated grids and the changed preview. On a temporary QA lane with
server PR42, both paid apply/re-entry and physical payment pass 18-command scripts. The existing bevy
server still requires that fix and remains running unchanged. See `aa-client-salon.md`.

Housing preview/quote/cancel passes a 20-command live script; construction settlement and decoration remain
open. Network, Illum, distant-object transitions, audio and UI callback recoveries have separate owned
commits and checks in their crate notes. Distant sprite lighting remains approximate; unsupported leaf-only
crowns retain their meshes to remove the flat olive patches. Brush mesh-pair dissolve passes 272 crate tests
and 348,160 GPU coverage checks. Native chat caret/slide previews and release-time reorder pass
271 toolkit checks and 32 original-code groups; the client already handles its same-owner attachment hook.
The terrain forward composition now passes 275 world
checks, 48 GPU controls and five original-code probes; its layer arrays add about 66 MB.
Native deferred terrain, punctual terrain highlights and native shadows remain open.
Forward Illum punctual highlights and authored per-light ratios pass 94 crate checks and 6,240 GPU
samples, with bindless/fallback agreement; default opaque deferred/tiled lighting remains separate.
Loaded-world distance refresh now passes 278 world checks; its actual loaded-scene regression
goes from 3,373 stale ranges to zero and verifies restoration. Authored waterfall type-2 motion
passes 98 crate checks, 36 native matrices and 728 GPU samples; dual passes/deformation remain open.
Below-surface WaterSurface/refraction and plain volume fog pass 136 pixel controls and five pond
crossing states. The dedicated player-music route passes 76 audio checks, 63 native probes and
11 focused client checks; downstream total native master gain remains unresolved. Saved MIDI
option startup/reset and nine existing settings checks pass separately.
Current workers cover native tab-alpha behavior, forced particle generation and the ocean window.
Native sun-shaft controls pass 20 original-code probes and six GPU controls. Outbound music packets pass
698 network tests; adaptive ground decals pass 59 tests; PCM submission passes 64 device-inclusive audio tests;
the MML/MIDI backend passes 31 tests including 54 native compiler cases. Music editor/preview integration
is committed; normal/placed performance, score caching and live composition settlement remain open.
Native quietest policies pass 73 audio checks; native bound timing passes 225 toolkit checks and 48 original-code groups.
Server PR44 fixes uncached note requests with 9 focused checks, 3,596 full lane checks and 7 live response/buff cases;
its lane is removed and the existing bevy server is unchanged. Mount work remains deferred.

The integrated main executable builds successfully. Scoped client Clippy exits successfully with
157 existing warnings and none in the new music or chat-transfer modules. The checks above are a
recovery checkpoint; the remaining rows below still require implementation and live validation.

The final light-texture/terrain/chat checkpoint rebuilds the main executable and the offline
`world_rendering` example successfully. Its inspected 1280×720 capture has 16 terrain patches
(14 visible), 4,894 object parts and nine authored-light ratio components; it exits 0 without
render warnings/errors. Observed FPS is 87.5–117.9 with other builds active. The five client
chat-transfer checks also pass after the toolkit's release-reorder update.

The subsequent MIDI/waterfall/pond checkpoint rebuilds the main client in 4m59s and
the offline world caller in 4.39s. Its final 30-second GPU run exits 0 with the same
16/14 terrain counts, 4,894 object parts and nine light ratios, no render warnings/errors,
and an inspected capture at `shots/resume/world-rendering-post-water.png`. Shared-load
FPS ranges from 82.5 to 119.9. This verifies the combined caller graph; the waterfall
and underwater pictures/controls above retain their separate exact scopes.

The doodad use-target highlight is restored after commit 99d86241 removed it: `hud/doodad.rs` fills
`DoodadHighlights` from `doodad_almighties.use_target_highlight`, and `doodad_visual::highlight` copies the
mesh's `StandardMaterial` or `IllumMaterial` completely (diffuse, normal and every other map kept), tints only
the emissive term (modulated by the diffuse map when the material has no emissive map), leaves another
instance untouched, and restores the original handle on exit even after aa-cry's StandardMaterial -> Illum
swap. The old pale crate was not a lost texture: a flat emissive over the dark wood washed it out, and the
restore could leave the glowing copy in place. The warm tint is still an approximation of native
highlighting (the 2026-10-30 original-client capture took screenshots only, no colour reading).
`use_target_decal` is drawn as a ground ring under the doodad the bar is about (`ingame/doodad_affordance.rs`;
size and tint are ours and capture-pending: the capture stood outside portal 4608's reach), and no 1.2 doodad
sets `use_target_silhouette`. Verified on the original client (2026-10-30): the bar offers a phase's
`func_skill_id`, and its reach is the skill's `max_range` with no slack (the Gather bar, `max_range` 4, was up at
3.0 m and gone at 4.5 m; the first version added 1 m, fixed). The bar still lacks the Mailbox's 11400 (the
fake-use skills come from the natural-gathering fallback, row "Crafting, gathering and trade packs"),
`SCDoodadPhaseMsg` (0x157) is decoded but no captured doodad sent it, and the doodad right-click walk-to and the
plate text are unverified (`aa-client-doodads.md`, `tools/evidence/doodad_affordance/capture-results.txt`).

Updated 2026-10-18. This is the working completion ledger for the original-client parity goal.
The original `scriptsbin/x2ui/` content groups, game database, native client, reference captures,
and lane-server protocol are the sources. A window opening with fallback data is not a complete
feature. Each row requires real data, input/actions, lifecycle/error behavior, and appropriate
live or visual validation. No overall completion percentage is inferred from script counts.

Execution policy (user instruction,2026-10-18): keep multiple Sol6.1 agents working
throughout this goal. Use all available concurrency slots (currently root plus
three agents), assign independent owned-crate slices, and immediately give each
finished agent another concrete parity gap. Root continues integration and QA
alongside them. Preserve this policy across handoffs until the full1.2 client
parity goal is achieved or the user changes the instruction.

Current source integration queues typed enchant requests from the original
Execute API in all four modes, then uses one native receipt-ordered item provider
for result links, per-row consumption and controller restoration. Matching actual
writes arm completion; raw skill errors/stops do not invent native Timeline
ownership. SC093 and SC0A0 use their source-qualified ordered lock callbacks.
The shared native-record implementation now serves both HUD and Session request
checks, including retries after native rows that leave the legacy reducer stale.
The startup adapter supplies the same complete readonly recharge catalog to
both consumers before asynchronous GameData readiness, preserving lifecycle
resets and unknown-versus-known-zero behavior.
The combined build and visual verification pass; the latest immutable main is
target/debug/aa-client-20261018-specialty-layers.exe, SHA256
2495C40316A0E83F735E64D1E2FDC8FA4FECD8F697ACDA6B84742CB450E87994.
The idle target/debug/aa-client-play.exe alias was refreshed from it at17:31:23
local and hash-verified equal.
Its frozen suite passes327 /38 optional ignores in72.75s; nine provider tests
moved to aa-net, whose full611 tests and actual native retry loopbacks pass.
Main+specialty build in2m, final diagnostic example refresh44.78s. The original
localized specialty ratio error and live world-list captures are inspected
without runtime errors; preceding grade idle/working captures and all four
dimmed working states remain documented. This cut includes the UI healthy/
low-health warning restart, admitted-paint Effect clocks (5194c1a), ordinary
drawable-layer flags (3332e92), authored ordinary point HDR gain (3dc3010),
shared startup recharge data (networkf674b41) and the received specialty ratio
getter/callback adapter (networkf51d264). The latter preserves error-before-reset
and cache-before-event ordering; actual request/price/confirmation/sale remain
separate (aa-client-specialty.md). Owner overlay traversal and brush LOD work
are outside this cut. Owned health/spinner/dock captures remain in their docs.
Protected native planning,
global selection pending/effect registration and retained-Timeline stop handling
remain explicit gaps (aa-client-enchant.md).

Previous accepted records-build validation:315 passed /38 existing optional probes ignored
in91.99s with three test threads. Twenty focused enchant checks pass in6.60s.
Selection and BAG totals now use explicitly known native pre-row records,
preserving zero/negative stacks, ID lookup/removal ordering and native empty
records; unsafe/unprovided rows retire certainty. At this frozen cut, typed
request/controller integration was still open. Main and three fixtures build in3m37s; eight
shots/hud/enchant-records/ captures are inspected without runtime errors.
Seven are pixel equal to the prior frames cut; the buff differs only at its
advancing countdown digit. The idle playable alias is refreshed from
target/debug/aa-client-20261018-enchant-records.exe, SHA256
8C3D1CDC2FDDF96F44F050814A93D32822F755D18FF61C1BB1F175D1CC8A3726.
This cut includes native item projections, button-state colors/checkbutton
visibility and native foam precision/static-water lighting. Later ordinary
button-art state, recharge and typed-request/SC093 work are outside this build
(aa-client-enchant.md). No overall feature-completion claim is inferred.

Previous full-client suite:307 passed /38 optional probes ignored in86.92s
with three test threads. Twelve focused enchant checks pass in5.28s: received
physical BAG selection, actual four-mode compatibility, native costs/ratios,
secure/pending/header/loss guards, cursor and synchronous event contracts,
socket-info/modifier tables and full original updater calls. Eight repair
captures are inspected without runtime errors; original source portraits and
effect lines are present. Protected planning, shared operation pending state,
typed request integration and result/consumption continuation remain open.
The previous playable alias was refreshed from target/debug/aa-client-20261018-enchant-frames.exe,
SHA256347DDF0D98E758E49CCBF6E082048BA2894635637CBF7430D14EBD2F83A00327.
Main and three fixtures build; eight shots/hud/enchant-frames/ captures are
inspected, pixel equal to the repaired selection captures, with no runtime
errors. This cut includes native widget frame admission/callback order and
static2D water environment maps/cache admission. Button-state color methods,
foam literal precision and raw Update9 retention are subsequent worker slices
(aa-client-enchant.md).

Previous accepted playable baseline:298 passed /38 optional probes ignored in55.55s
with three test threads. The copied suite and main include received-headgear
hair selection and the toolkit's source-verified native fade manager. Seven
1920x1080 captures under shots/hud/native-fades/ are inspected: buff tooltip
385.0-447.9 FPS, charge badges477.5-544.4, grade result503.9-518.3,
socket result417.1-478.8, grade controls365.4-484.7, unsocket controls354.2-508.5
and real lane world select344.4-422.8. All processes self-close without
WARN/ERROR/panic. Main and the three fixture examples build. The idle playable
alias was refreshed from target/debug/aa-client-20261018-native-fades.exe,
SHA256 C236C8B021D8674370142878D8DA9B0E957A1E54B0768B89E2AC4E90E9E53510.
This build also includes native qualified WaterSurface refraction and authored
cube direction; aa-world records223 passing tests /9 optional probes ignored,
84 exact refraction controls and eight composite/sea controls. Inheritance and
additional material-feature gates remain ongoing worker slices.
Three mode checks cover the original bag enchant button,
physical grade/socket/unsocket/gem controls, native synchronous mode events,
strict return arities, preserved picked items and received session retirement.
Six mode/world-list captures are inspected and the playable alias is refreshed
with NumberOnly labels, authored water normals and the qualified cube correction.
Item eligibility, info/cost providers and request/planner/controller continuation
remain open (aa-client-enchant.md).
Three further focused appearance checks pass for rendered helmet hair selection:
native alternate/hidden hair, actual image overrides/fallback, cosplay priority,
material/other-part retention and bald appearance. Character select, prewarming,
local spawn and remote humanoid preparation now share this adapter. The full
suite and playable baseline above include this integration (aa-client-headgear.md).
Six item-effect checks cover receipt-ordered current versus inline items,
equipment/BAG lookup priority including ID0, native callback/argument shapes,
raw grade name/color lookups, deferred lifecycle and all five original grade
outcomes plus socket success/failure and magical banners. Six result/world-list
captures are inspected without runtime errors; the playable alias is refreshed
with this adapter, raw cooldown setter and triangle/LOD terrain weights.
Actual request/planner/controller continuation and live item settlement remain
open (aa-client-item-effects.md).
All11 focused buff checks pass, including native optional flags, strict numeric
ordinals/zero-result failures, missing-descriptor physical positions, static
category, caster identity and unsigned/f32 duration fields. The original buff
tooltip passes physical hover/time-only refresh/leave. Three tooltip/charge/live
world-list captures are inspected without runtime errors; the playable alias is
refreshed with this getter, native completion artwork, BC1 normal XYZ and socket
result readers. Native actor-frame time, full description providers and live
entry remain explicit gaps (aa-client-buffs.md).
Native buff-count checks add twelve executed branch goldens and original
charge/stack badges responding to signed SCBuffUpdated values without icon
removal. All eight focused buff/catalog/control checks pass. Charge and live
world-list captures are inspected, and the playable alias is refreshed with
this count projection, ordinary-slot edge artwork, factual buff decode-loss
invalidation and signed heightmap projection/normal frames.
Five self-buff checks cover native folded Good-list icon order, real physical
left/right input, valid received instance0, original system-buff warning,
captured identity/lifetime, selection/disconnect guards and receipt-only removal.
Five buff/world-list captures are inspected without runtime errors, and the
playable alias includes this adapter and independent terrain normal UVs.
Fresh QA entry receives an empty character list for the HUD-only account;
the previously documented create account still times out before character list.
No live buff removal or server restart is claimed. See aa-client-buffs.md.
Seven ranking checks cover the real catalog and native calendar/return shapes,
ordered receipt and name readiness, original physical tabs/refresh/rewards,
and the actual HUD scheduler with captured typed requests. Main and ranking
example build; three ranking fixture captures and real lane world select pass
with no runtime errors. The playable alias now includes ranking, physical terrain
support, native 32-slot materials and both cooldown mask paths. Physical terrain support now
omits native holes, preserves received spawn height and accepts actual lower
brush floors; the original hole/proxy regression passes. Live traversal remains
unverified. See aa-client-ranking.md and aa-client-terrain.md.
Five bounty checks add native fifteen-column getter values, ordered complete-map
callbacks, actual actor name/local flags, typed captured controls and the original
empty bulletin/standalone setup button. The nonempty original bulletin still
fails on undefined original Lua globals. The completed all-961-bytecode and
native child-binding audit identifies an original asset defect; no provider
global is invented. The absent SET_BOUNTY-to-setup listener remains an explicit
gap. No bounty payment is inferred.
Three bounty/world-list captures are inspected without warnings/errors; the
playable alias now includes the bounty adapter (see aa-client-bounty.md).
All six trial HUD checks pass, including original physical Task6/7/8, records,
verdict and waiting controls, deferred visual admission, captured identities,
native callback snapshots and original time labels. Seven trial/world-list
captures are inspected; the checked playable alias includes this integration.
The focused chat suite passes all13 checks, including pending native
trial-method consumption at actual chat open and pending clear during reset.
Chat-configuration checks include original Save/Cancel/Reset,
physical context Lock/Unlock, blocked dragging, warning timing, full Reset Chat,
independent selection/method loading without saved tabs, and fresh-stage persistence.
Native tab metadata, selection, method, opacity and eight owner-window locks
persist per world/character. The full reset retires windows/history, deletes only
four owner keys and calls original zero-argument recreation. Three latest reset
and live-world-list captures are inspected; the tested playable alias is refreshed.
The shared period helper matches18 executed original cases, including360-day years.
The native dishonor token now displays its red skull; the original
jury waiting tooltip and the earlier separate physical probe pass. Received
cooldowns advance once per frame before HUD reads; queries remain side-effect free.
Earlier achievement tracing/detail/persistence and list-geometry checks remain passed.
Native slot captions and the actual account option82 integration pass; generic
cooldown buttons retain masks without invented slot captions. Explicit native
GCD selection/state has a tested opt-in toolkit adapter; the actual application
producer/accessor mapping remains open. Generic and ordinary slot mask rendering
now use the source dual-texture path; ordinary slot captures and 42 selected
UI regressions pass. The toolkit selector remains opt-in pending real producers.
Network555 tests pass, including typed052 item-effect transport with captured
received-slot rechecks and original payload goldens, received grade/socket/UCC/schedule item results,
shared static buff catalogs and native buff metadata/capacity/order/descriptor
guards, SCBuffUpdated and factual decoder-loss uncertainty, bounded ranking receipts, bounty receipts/typed commands and initial trial tasks/commands, history/progress,
automatic native CS072, Confirm/Cancel/Verdict and callback-stage snapshots.
World 222 tests / 9
existing probes ignored,120 cloud/transmission GPU comparisons,36 native water-half
oracles and eight original scene/unload runs pass. Sky and native pre-water clouds
now reach the transmission copy in their source order. Fog/deferred decal water
ordering adds32 GPU comparisons and nine original scene/unload runs. Explicit
subzone queries pass22 native controls and315 original polygon probes; automatic
live registration remains unwired. Ordinary road order adds36 executed native
cases, eight exact GPU controls and original scene/unload checks. Owning docs
retain exact evidence and limits. Native terrain holes/height packing/diagonal
and 32-slot surface materials pass numeric regressions, all 64 GPU material
controls, two original on/legacy pairs and unload checks. Independent diffuse/bump
tiling and cell phase pass eight original ATI2 texel controls, original scene pairs
and unload. Ordinary completion effects pass92 selected UI checks/four captures;
raw SetCoolDown arguments and fan gates pass64 selected checks/two captures.
Triangle/LOD SID weights add36 executed original interpolation calls,45 exact GPU
basis controls,15 retained fade controls, original mixed-SID border and unload.
BC1 normal XYZ preservation adds38 GPU controls and six original
scene/unload runs; ATI2/detail controls remain pixel-identical. Signed heightmap projection/frame adds 27 executed original controls,
22 GPU controls and seven inspected original-scene/control/unload runs. Trial spatial strings/live validation, bounty asset
compatibility and ranking live integration, remaining cooldown producers and further rendering/gameplay gaps stay active; overall1.2
parity is unfinished.

## Earlier feature inventory

This table retains the evidence and open work recorded by the dated recovery
checkpoints below. Later integrations above and in the owning feature notes do
not turn it into a current completion claim.

| Area | Evidence at those checkpoints | Remaining work at those checkpoints |
|---|---|---|
| World selection and login | Live Trion login, world list/queue, connect; original Lua, zero-error captures | Other regional authentication, premium/house lobby info, reconnect policy |
| Character selection and creation | All four 1.2 races and both genders; appearance controls and server creation/re-entry; real captures; all eight preview heads use authored facial idle overlays | Remaining appearance/material differences, hat/beard variants, training-field animation/wind |
| Loading and screen lifecycle | Full flow, prebuilt HUD, return-to-list/logout/disconnect, spawn focus tests | Broader interruption/race coverage and persistent driver pipeline cache |
| Ground movement and camera | Walk/run/back/strafe/turn/jump/fall, collision/steps, clipped camera, server movement; original primary/secondary bindings drive movement and zoom | Camera preset actions, click-to-move/native interactions and less common locomotion modes need audit |
| Water and mounted movement | Wading/swimming/diving; original breath bar on underwater reports/corrections and calculated capacity; remote mount/rider placement and movement packets | Natural live breath/drowning QA, local owned mounts/pets, mounting UI, ships/vehicles, gliders, hanging/moving supports |
| World geometry | Terrain with native heightmap-distance alpha order, exterior and visarea interior brushes, vegetation with native vertex AO/authored group brightness, roads, forward and deferred/wrapped diffuse decals including interior placements, authored static entity props/local lights/projectors, verified rigid CGA windmills, streaming, native baked-colour cave base geometry and seven compiled cave collision proxies with verified surfaces/transforms and unload; reference captures | Remaining voxel cover/detail/atlas variants; portal/area receiver lighting; decal normal/specular accumulation and receiver variants; other animated entities/controllers/FlowGraph, far impostors/HLOD, remaining placement types and procedural grass |
| Environment | Native Bezier curves and 147-slot defaults/ranges, spatial zone/raw-curve lighting blend, distant-cloud motion/geometry/lifecycle, original 8,912-star catalogue and native viewport sizing, moon discs/coronas/orbit and sky draw order, fog, water, music/ambient/reverb/moods | Area/relic overrides and legacy XML color migration, discrete skybox/cloud fades, shafts/glow/adaptation, underwater fidelity and remaining mood triggers |
| Character and animation rendering | Real assembled meshes/skeletons, locomotion/combat layers, equipment, customization; authored body-clock facial overlays/blinks for all eight playable local-player/preview heads and supported NPC heads, source-verified facial bone controllers, explicit received NPC body slots and rigid CDF bone attachments; creature and resolved-part CDF effects with visibility/unload lifetime; authored static UCC composition/native default, iris color and conservative alpha cutouts | Voice lip sync; appearance-change rebuilding, full-world selected remote facial and attachment QA, CDF entity bindings; further render reference checks, fur/hair/skin shading differences, clothing clipping, dynamic UCC/gameplay texture overrides and remaining clips |
| Combat and HUD | Skill/cast and native combat-widget timelines, projected moving-unit text, native fall/drown reports and acknowledged Bloodlust icons/toggle, hits/projectiles/buffs, dark/restricted slots, target/range/mana gates, death/resurrection | Force-mode protected/team eligibility; broad skill-by-skill reference verification; FX decals/material flashes/lights/forces; special mounted/flying/ship casting |
| Skill progression | Real skill tree/tooltips, learn/passives/reset/swap; server requests/state; frame-advanced received skill/all-descriptor-tag and admitted stop-duration cooldowns/preflight, superseding stale DB estimates; original equipment constant-button sweeps preserve item icons/actions; native localized slot captions and actual account option82 visibility | Full native TL lifetimes, Fired cooldown/default/custom GCD producers, explicit slot GCD state and modifiers; generic raw-snapshot lifetime; broader progression/live coverage; lane cooldown emitter/re-entry verification, tooltip corner cases and damage breakdown option |
| Inventory and equipment | Real bag/equipment/actions, drag/drop/split/merge/destroy, item tooltips, bank transfers/money, independent saved bag/bank tabs; original expansion confirmation/150-slot pool, received auto-AA setting, native view sorting and repair controls/quotes/durability notifier with physical-slot actions; original enchant mode controls, received physical selection/native compatibility, costs/ratios and socket/gem effect rows; receipt-ordered grade/socket/gem result dialogs | Natural repair service routing/live repair and expansion, native premium repair payment, item-enchant protected planning/shared pending/request/continuation, dye/look/temper workflows and missing tooltip fields |
| NPC interactions and commerce | Quest NPC dialogs; real catalog, buy/sell/buyback and bank in original UI, live transactions; original stablehand quote/confirmation over received Bag+Bank summon details; original player-trade controls bound; the auctioneer NPC opens the original auction house ([aa-client-auction](aa-client-auction.md)) | Quest/service choice on multi-service NPCs, repair, ability services, priest/teleport, specialty |
| Quests and map | Accept/talk/hand-in/drop, dialogs/rewards/tracker/log; NPC/service/party map icons; completed ship/transport/fish radar snapshots drive original map symbols/tooltips, removal and channel clear with original-script probes/captures | Native radar effective expedition/custom relations, area circles/fog/instance masks and live QA; objective areas/notifier icons, more requirement kinds, cross-zone icons, live housing/public-farm icons, climate/conflict overlays |
| Chat | Live channels/whispers, original chat input/link insertion, clickable item/quest links, native transport escaping and message decoding, system/combat messages; eight native local tab IDs, filters/full palette/font, numeric selection/input method, hover/idle opacity and owner-window locks persist per world/character; original settings Save/Cancel/Reset, context Lock/Unlock, blocked drag warning, first-tab window movement and full Reset Chat with independent extras reload; native pending trial method at actual chat open and source-ordered trial receipt producers | Native send-size overflow policy, window position persistence, other-tab splitting/reparenting, provider-specific extra name filter, full command set, moderation/social integrations and less common channel events |
| Party and raid | Original invite/decline/accept/leave/kick/leader/convert/role/drag-move/loot controls and native group-loot prompts; sparse slots, offline/rejoin, unit-frame marks and authoritative map pings; live two-client coverage | Natural paired live loot settlement,3D overhead marks, party buff/combat indicators; broader raid UI coverage and instance-specific ping surfaces |
| Social/community | Original friend/block tabs, acknowledged commands, status/list refresh and blocked chat; original family roster/count/offline rows, native callbacks, feature/owner-guarded actions and captured invitation/ignore controls; original guild roster/location/offline rows, permission/title editing, invite/reply, leave/disband, kick/rank and received-siege-guarded leadership transfer; guild-service preconfirmation/sponsors/Create and actual received Task35 Rename | Paired live family/guild settlement, overhead family/guild titles and broader interruption coverage; live unblock/offline/restart checks; lane political-system/rename completion gaps, guild web/siege controls, nation and remaining management |
| Mail, trade and auctions | Original mailbox read/claim/write/send and player-trade carts/money/locks/confirm/cancel bound; original-stage fixtures; mail/trade protocols live-tested in aa-net before the lane stall; original auction house search, bid, buy-out, listing, cancel and bid history, driven live between two accounts | Paired live original mail/trade and restart checks; native trade eligibility/window bounds; auction settlement mails claimed through the window, bid/buy-out chat notices of other players |
| Crafting, gathering and trade packs | Original Folio browses7,010 real recipes, groups/categories/search/materials/products/higher-tier links; received physical materials/raw grades and native labor rounding; static workbench maps and batch controls; original proficiency grades, caps, upgrade/downgrade/certificate expansion controls/events with pointer probes and captures; natural gathering func-to-skill mapping with the ordinary cast/tool/anim/labor/loot/phase path (1.2 capture 2026-10-30: herb 13990 on F with the "Gather" cast bar and phase 4513->4514, fake-use lamp 13166, local no-labor refusal, bar on at 3 m and off at 4.5 m, cast not gated by a full bag; legacy ore/rock/shear capture-pending) | Natural workbench input/range/permissions, remaining execution/live validation, premium proficiency payment configuration and housing bonuses, legacy ore/rock/shear gathering capture, gathered-item delivery into free bag space (every captured herb gather had a full bag), processing/workbenches, specialty/trade pack flows |
| Housing and territory | Static placed world data/map icons; received house snapshots/progress/permission/demolition flags, parent/TL decoration joins and aligned housing unit tails/posture; seed/sapling placement rules from the 1.2 builder (30 m 3D inclusive per the decompile/F1, group yaw/water, sphere overlap, refusal texts, 30 s reuse lock at world load/teleport per F3/FUN_39426520) with an offscreen self-check | Housing/placement/farms/permissions, ownership/tax, territory/siege/dominion, construction and interaction flows; farm planting capture F1-F18 (F1/F3/F13/F14 settled, F4 partly; box shapes, house/zone rules, crop bar/harvest open) |
| Pets, slaves and transport | Remote units/support motion rendered; owned mate/slave metadata, original current-pet frame/bar and information panel with native received-level/equipment/buff attributes, pool maxima and modifier colors; original pack-specific equipment icons and own-item hover tooltips with tag29/received pet level cap; filtered skill presentation/capabilities, received-XP getter, original rename and behavior controls; stablehand revival requests and raw equipment transactions decoded | Native speed-component getter, equipment cursor/confirmation/transaction controls, general skill usability/planner/auto-use, cap-aware XP delta folding, mount/dismiss eligibility, summon/unsummon/equipment/skills, slave UI and ships/cannons/transport controls |
| Battlefields | Original recruiter, queue/cancel, three-choice invitation, received match phases/timers/score/results and leave controls; native slot244 skill identity/tag/enable gates; controlled original-script probes and captures | Natural lane matchmaking/live match QA; lane Pong/start timer-origin mismatch; protected native ordinary skill eligibility and actor planner/casting; queue errors and remaining scheduled-event presentation |
| Duels | Challenge from the target popup, challenge prompt (default dialog with the original `locale.duel` texts), accept/refuse, `STARTED_DUEL`/`ENDED_DUEL`, start text and result notice, temporary team factions on plates/target | Retail placement of the countdown text (system message here); the AAEmu server now sends `SCDuelStartCountdownPacket` at acceptance (NickMesser/AAEmu#15), so the countdown shows live; `SCAreaChatBubble` 0x153 (type 543, ignored) |
| Achievements, ranking and trials | Original achievement UI over all1,743 descriptors, received native progress/rewards, six tracing slots and ordered completion/recent callbacks; guarded native type5 tracing hydration/save-before-leave; original physical controls passed; received crime/jury headers, queue request, original labels/hover/waiting tooltip and dishonor glyph; native19-key trial data, record pages, callback snapshots/ticks and actual Task6/7/8, Confirm/Cancel/Verdict controls; original ranking tabs/records/rewards over static catalog, received boards, independent streamed names, native calendar/locks and typed controls; bounty getters/setup/empty bulletin | English summary/description data, unsupported UI-data grammar and live achievement save/re-entry validation; ranking reward-mail formatter and live settlement, original nonempty bounty asset defect/payment lane gaps, trial spatial strings and mixed-subsystem ordering, crime reporting/prison and racing flows; paired live crime/jury settlement |
| Music and other special systems | Audio playback and original music UI assets | Bard/song instruments, composition/playback UI, books/UCC/crest, portals, premium/shop services and other original content groups |
| Options and UI toolkit | Applied account-local options/sound/key bindings (middle and side mouse buttons bind too) and bag persistence; the original screen and quality pages apply resolution, screen mode, vsync, gamma, draw distance, object/vegetation distance, shadows, clouds, effect quality, HDR, anti-aliasing and camera mode (`graphics.rs`); original Apply/Cancel/Default Keys plus restart/action-dispatch tests; native clipped chat captions and fixed/auto tab layout, flat/tree listboxes, state/separator art, row tooltips, dropdown mouse selection/layers, widget animation, Unicode clipboard, IME and keyboard capture | Native binding blob/character synchronization, remaining mode/action consumers, the quality page's texture / terrain detail / character / water / DoF items (no renderer knob) and native global far clipping, scaled clipping/font calibration/layout persistence, listbox template/line-border variants and other native methods |

## Active ownership

* Root: `aa-client`, original trial/bounty/ranking received adapters, physical terrain support, native Good-list self-buff cancellation/getters and receipt-ordered item-effect callbacks have documented checks. Item-effect request/planner/controller controls, remaining native buff time/description providers and genuine cooldown producers continue next over executed source contracts. Original Folio browsing, proficiency controls, pet information/rename/behavior, explicit received NPC body slots/rigid attachments, repair/notifier, auto-AA, facial integration, inventory sorting/150-slot pool, expansion confirmation, cooldowns, breath, keys, mail/group-loot/trade, friends and combat-display milestones passed their documented checks. Natural service routing, remaining pet/transport actions and live settlement stay open.
* Network milestone: `aa-net`, underwater receive/source audit complete; conflict-zone reports, received cooldowns, environmental/combat flags, explicit force-attack, UIData, repair/expansion and owned mate/slave APIs landed, with server limitations documented.
* UI agent: `aa-ui`, native listbox hierarchy/art/tooltips for original crafting/achievement, including visible/animated templates and white/grey state fallbacks; native48px icon rejection audited. Flat dropdowns/layers, font alias audit, IME, chat insertion/transport, cursor categories, clipboard, links, edit and combat timelines landed. Read-only native repair API audit supports root's next integration.
* Material milestone: `aa-cry`, UCC defaults, Eye color and alpha cutouts landed; no root integration hook needed.
* World agent: native Group14 polygon ToD geometry/blending/lifetime landed with native numeric oracles; gameplay subject qualification remains unhooked pending proof of bit0x20's producer. Native skybox/cloud spatial opacity and asset fades, curve tangents/defaults, spatial raw-curve zone blending, cloud motion/geometry/lifecycle, deferred/wrapped diffuse decals, native moons/coronas/orbit, sky order, static entity props, local lights, stars and rigid windmills landed.
* Animation milestone: authored FSQ/FXL facial curves and exact expression overlays for all eight playable heads, including original blink sequences; local-player, preview and supported bound NPC CDF integration passed. Selected visible/awake remote Characters now opt in/out in place with bounded private mesh ownership; full-world target QA remains open. Original facial bone displacement/order/base-pose contracts and native helmet hair/beard selectors landed. Rendered head/cosplay/image hair selection is now applied during local/list/remote assembly; later appearance-change rebuilding, beard geometry, visibility policy and automatic voice lip sync remain open.

The shared checkout also contains inherited calibration/performance edits in other crates. Those
are preserved; they are not counted as newly verified features until their owner or an assigned
review validates and commits them. Server-only gaps are recorded as such; the client does not
fake successful transactions to hide missing server support.

## Verification baseline

Before this wave: client 97 passed / 13 intentional probes ignored; UI 78 passed / 1 ignored;
network 120 library tests plus scripted/live session checks; five focused road tests. Two fresh
clients exercised party transactions, disconnect/rejoin and targeting. Settings survived a real
process restart, including volume 0.4, ShowBuffDuration=0 and a custom selected Supplies tab.
Captures in `shots/hud/sol61-wave2/` have zero Lua errors. See crate docs for exact checks and
limitations. Each new milestone updates this ledger and its owning crate's evidence.

Current wave: client174 passed /29 ignored, plus isolated original social/mail/loot/trade/keybinding/combat/breath/expansion/sorting/repair/recipe-book/workbench/stablehand/current-pet/radar/proficiency stage fixtures and inspected CDF effects/hide/unload captures passed. Native mate pools include18 executed original gear arithmetic oracles and independent native buff history. UI167 runtime plus2 doctests passed /1 ignored at the last full baseline, with32 targeted listbox/animation/cursor checks,52 lifecycle checks and11 global-event/popup/hotkey checks passed; animation75 passed /1 ignored plus original part-CDF integration; world164 passed /9 ignored; network331 library tests passed. The earlier full-crate run also passed
2 scripted tests. Fresh live paired raids/friends covered mutation, offline/reconnect and saved
lists. Original commerce UI confirmed 33-copper sale and buyback, 25-copper purchase, bag/bank item
round-trip with identical UID, and 5-copper deposit/withdraw through the original money dialog.
Final captures/logs live under `shots/hud/sol61-commerce/`; these passed with zero Lua errors.
Native lock/queued-sale regression refuses a replaced slot's identity instead of selling the
replacement item. Four decal, five road and three static-entity focused checks passed; owning
docs record exact assets, known variants, source addresses and performance.

Fresh team9 also exercised original raid convert/role/drag/loot controls, marker identity,
peer/root ping echoes, original ping removal, marker reset and dismissal. The inspected
`shots/hud/sol61-raid/` captures/logs have zero Lua errors and 68-95 FPS at 1920x1080.
The world agent inspected original stars at night/off/day and bounded authored local lights,
including cell unloading; their own docs retain renderer approximations and remaining environment limitations. Native moon captures, horizon light selection, alpha-surface ordering and viewport-sized star pixel checks are recorded in aa-world docs.

The auction live fixture exposed a lane-server stall after refund claim/delete on 2026-10-18. It
no longer reproduced on 2026-10-20: the client logged in, entered the world and passed the headless
smoke tests (titles, tooltip, Character Info, emotes, guild window) against the AAEmu server.
Auction settlement tail/reconnect and final social unblock/offline/restart live checks remain
unverified.


### Battlefield integration and fresh playable build (2026-10-18)

The original recruiter/queue/cancel and full-qualifier invitation flow is connected to
received native match phases, mapped clock, progress/score getters, results/roster,
physical Leave and actual-load reset. Slot244 displays real tag204 skill metadata,
first-matching-ID enable bytes and current ordinary HUD restrictions, including its
dark overlay; native protected ordinary can-use and casting planner remain open.
Scheduled-event start receipts supply the source result ratio with all raw fields
retained; the lane has no sender. Its separate Pong/start clock origins remain an
explicit live compatibility defect, not hidden with SC191.now substitution.

Full client suite186 passed/32 ignored; all three original battlefield probes pass.
Inspected invitation/play/results captures contain no runtime errors. Fresh main
binary at `target/debug/aa-client-play.exe` includes previous CDF/pet/radar/proficiency,
native mate-pool and this battlefield work. Live world-list capture confirms the
lane list at369-417 FPS and zero script errors. Full-game parity and natural live
matchmaking/casting remain unfinished. Details: `docs/aa-client-hud.md`.

Parallel landed milestones include native task decoration before layout, statusbar
fill drawable anchors/casting highlights, water endpoint/shoreline alpha and native
per-instance vegetation color variation; those retain their own crate QA evidence.


### Native owned-pet information panel (2026-10-18)

Original info-toggle pointer probe and all11 attribute labels/17 modifier keys are
connected to the pet's own native actor formulas, equipment and known buff history.
Received armor/regeneration changes update values and original green colors. Separate
compiled-expression child rounding and gear/presentation paths match72 executed x86
gear cases, two expression cases and ten modifier cases (static lookup/AST and external
CRT mocks are explicit). Existing native pool checks remain valid. Unsupported inputs
close the info view rather than showing player fallback stats. Speed's actual engine
component, equipment UI/cursor/confirmation workflows and pet skill planning remain open.
Full suite195 passed/33 ignored; original panel probe1 pass/2.54s and original battlefield
regression1 pass/3s. Inspected base info capture260-441 FPS with zero runtime errors.
The visible world-select client remains the previous tested940a8c6 copy; this panel
milestone is validated through its separate aa-dev example until the next main rebuild.
The modified capture also confirms original green defense122 and regeneration18
labels, both received buff icons and204-306 FPS without runtime errors.

### Original equipment cooldown controls (2026-10-18)

Original equipment buttons now observe received timers through a cooldown-only
raw-skill hook, preserving their item icons and physical actions. All generic
`tagged_skills` memberships participate in received cooldown selection/preflight,
with global/tag/raw-ID native strict-greater tie order and unchanged zero totals.
Full client199 passed/34 ignored, final HUD regression133 passed/21 ignored, and
actual original equipment-screen probe1 passed/2.38s. Inspected active/ready aa-dev
captures run188-268/405-417 FPS without runtime errors. Existing DB/cast-start
estimates are explicitly retained as estimates; source-closed Fired/Stopped/GCD
producer gates and skill-specific duration modifiers remain the next work.

### Original guild management and received siege eligibility (2026-10-18)

The original guild roster/permissions window now reads received native caches,
including native local50-row paging, online locations and Unix offline ages.
Invite/reply, kick/rank, permission/title editing, leave/disband and leadership
transfer send captured-character requests and wait for actual server receipts to
change state. Original Task3/26/27 controls preserve ignore/duplicate handling and
close on stale character/guild context. Leadership transfer uses the actual
received Dominion phase/participants; decode gaps refuse unknown eligibility.
Guild creation/rename, web/siege controls, overhead titles and paired live guild
settlement remain open. Detailed native/app boundaries are in aa-client-hud.md.

All11 focused guild checks pass; full client219 passed/36 ignored in14.37s.
Five inspected original-script aa-dev views self-close with zero runtime errors:
permissions286.8-313.2 FPS, members286.8-424.2, invitation426.5-471.0,
leave294.4-324.2 and disband388.8-426.6. These fixtures establish original control
behavior, not paired live settlement or a native guild screenshot comparison.
Parallel network428 tests pass, native chat internal clipping has56 selected
checks, and world184 tests/9 ignored plus wind GPU/forest controls passed; each
owning crate's docs retain its exact scope and remaining source boundaries.
The integrated main executable `target/debug/aa-client-20261018-guild.exe`
reaches the live world list at260.3-361.5 FPS with zero script errors; a fresh
original guild-member example capture runs316.6-450.0 FPS. Both self-close.
The game-connection timeout remains a live validation blocker.

### Guild creation, rename prompt and active sponsor controls (2026-10-18)

Original guild-service preconfirmation and all four creation layouts now send
captured requests using actual current faction, cached mother chain and native
name validation. Received SC00f opens original Task35 with cached-owner and
political-system admission. The original edit handler validates names; the direct
Rename method retains its native wire-only/boolean-true behavior. Received state
alone establishes membership or a new name. Faction-name policy metadata now uses
the native en_us record instead of the generic2..26 fallback.

All16 focused guild checks pass, including physical Create/sponsor/preconfirmation
and rename Confirm/Cancel, duplicate and stale callback cases. Full client224
passed/36 ignored in14.87s; network433/433 passes; native active-tab70 selected
checks and11 executed native oracle groups pass. Authored box/ellipsoid fog adds
13 GPU controls and world189 passed/9 ignored. Owning docs retain precise scope.

Live creation/rename is still unverified. The lane sends new factions with
political_system1 while native Created/rename-task effects require0: actual later
SC01c supplies membership, and such lane-created guilds cannot open native Task35.
The lane Rename handler only logs. These server/native gaps are explicit;
the client does not fabricate successful requests or relax native checks.

Fresh main `target/debug/aa-client-20261018-guild-create.exe` and its playable
alias reach the live world list at266.3-346.9 FPS with zero script errors. All five
inspected creation/rename aa-dev captures self-close without warnings/errors;
ranges and the executable SHA256 are in aa-client-hud.md. No live in-game guild
settlement is claimed.


### User-requested stopping integration checkpoint (2026-10-26)

Feature and source research stopped at the user's request. The explicit Slider
SetValue checkpoint (`9f86b3cb`), browser/baked-pak branches (`6fd6d75c`), browser
lock checkpoint (`d51dbdcb`) and editor (`cde14ae1`) are merged into main.
`11dd6788` reconciles their native/browser boundaries: native-only pak writing
and streaming handles, WASI headers for the CELT decoder, browser clocks and
32-bit-host Lua number conversions. Native setter/range semantics remain scoped
as documented in aa-ui; this does not establish overall client parity.

The merged client/editor/bake/web native check passed at `cde14ae1`. The final
portability source passed `cargo check -p aa-client --jobs 1 --target
wasm32-unknown-unknown`. At `11dd6788`, `cargo test -p aa-client --jobs 1 --bin
aa-client` exited 0: **846 passed, 0 failed, 91 ignored**. Its fresh dependency
build took 31m49s and the tests 108.65s; warnings and complete output are retained.
No further compiler batch, Clippy, GPU capture, native oracle or server run was
started for this stopping checkpoint.

The earlier SetValue capture remains a 60s timeout with no PNG/FPS/provider
receipt. Its temporary host diagnostic also timed out after the visible-page
chunk returned; later stages were unreached. Neither result proves a cause or
nontermination. The diagnostic source/results were archived before removal.
No new rendered/browser/audio/native-graph success is claimed, and the playable
aliases were not refreshed for this integrated source. Native animation loader
completion, world subject authority and mounted-ship identity remain held.

All eight linked worktrees were removed, and their committed heads are reachable
from main. Before removal, 2,875 local/ignored files (735,378,012 bytes), including
editor screenshots/local data and browser artifacts, were preserved and rehashed
under the ignored `data/extract/aa-client/stopping-consolidation-20261026-r1/`.
Private account-cache data was kept out of Git. Only the main checkout remains;
merged local topic branches were deleted after push. Work is paused here.


### Resumed native startup validation (2026-10-26)

The parity goal resumed after the stopping checkpoint. At main `17d6258a`,
`cargo build --locked -p aa-client --jobs 1 --bin aa-client` exited 0 in
523.219s. The exclusive copy
`target/debug/aa-client-20261026-resumed-r1.exe` is 160,306,688 bytes,
SHA256 `4b43a73a4412f72fda784145091ddaf71054a8a4b96b67c47101d7595fb619fd`.
One muted custom-login capture at 1280x720 self-closed with OS0 in 13.219s.
The inspected PNG contains the complete login panel and controls; logged FPS
samples were 240.4, 250.0, 277.2 and 283.8. Complete logs contain no warning,
error, panic or script-failure line. The startup UI counter reported one object
and zero script errors. No login was submitted and no server was launched.

Screenshot: `shots/resumed-integration-20261026-r1/native-login.png`, SHA256
`52642beb010c02e0f4621a2ebe59a096390727cdfd8243afa270cdacf1e3c2a5`.
The playable `target/debug/aa-client-play.exe` alias now matches this tested
copy; the earlier immutable Slider-range executable remains unchanged.
Build, process, image inspection and alias proofs are under
`data/extract/aa-client/resumed-integration-20261026-r1/`.

This exercises the custom Bevy login, not the original World/Sound Options
consumer. It does not resolve the earlier SetValue capture/diagnostic timeouts,
establish original-client pixels or audio, or close native animation, world
subject or mounted-information parity. All 1,387 integrated production/build
source pins still match the merged checkpoint; this follow-up changes only
Root documentation and ignored evidence. The worktree cleanup remains intact.

### Mounted-information metadata slice (2026-10-26)

The read-only mounted-header supplier is now implemented in `hud/mounted.rs`.
Six focused cases passed, including the unchanged original hidden slave-window
method over explicit detach/load/bind receipts and real descriptor rows.
Received ownership and mounted selection stay distinct; an unknown initial
player attachment still yields no result. The first pre-edit fixture admission
was corrected before the passing run, and both versions/results are preserved.
See `aa-client-hud.md` for the domain and conservative lifetime limits.

This is not complete ship-control integration. The normal show-action producer,
motion/vehicle/equipment APIs, positive initial snapshot route and remaining
native lifetime/engine joins are still open. The aa-net motion-field proposal
remains unapplied. Earlier full-suite and playable-alias results describe their
recorded source revisions; no new full-suite, image, audio or live-gameplay
success is claimed for this slice.

The new slice also passed a separate wasm32 browser compile check and one
final scoped native Clippy. Clippy retained 597 warning emissions /418 unique
diagnostic-site records and zero errors; it is not a clean-baseline result.
All six source pins remained unchanged through those checks. No new main
executable or visual capture was built, so the playable alias remains the
earlier recorded startup build.

### Mounted-equipment read-side stopping checkpoint (2026-10-26)

The read-side equipment queries now supply the unchanged original hidden slave
equipment controls from receipt-local items and real customization tables.
Matching gear receipts update the cache; failed/repeated transaction notices
refresh the original controls without treating report payloads as settlement.
The corrected aa-client bin/tests check passed, followed by 11 focused mounted
tests with zero failures or ignores. The initial two FromSql compile errors and
their narrow correction remain recorded separately. See aa-client-hud.md for
the exact domain, feature/configuration behavior and retained warnings.

The user requested another stopping point. No further build/research wave was
started after the active test terminated. New browser/Clippy/image/live-server
validation was not run for this slice. Motion, outgoing equipment requests,
the kind-2 full-item wire-decoder proposal and normal visible ship-control
integration remain unfinished. Historical suite, image and playable-alias
results continue to describe their own recorded revisions. Ignored research,
failure evidence and unapplied patches are preserved rather than discarded.
