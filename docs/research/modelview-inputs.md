# Original modelview inputs and toolkit handoff

The local shop's widget handoff and camera inputs are established. A private app
host now renders original character assets into widget images and consumes local
trial/reset commands. Caller, lifecycle and supplied-fixture visual checks pass.
Shop data remains separate; this does not establish a storefront or
native scene parity.

The toolkit bridge is committed at `93c0793a`; `aa-anim`'s reference-heel lookup
is committed at `b8654f6f`. Both passed their owned checks and independent static
review. Root's prepared camera and caller code passed **11 focused checks**,
including the original salon initialization and an explicit read of the four
real pak layer assets. Whole-client validation is recorded separately; the
offscreen host's newer checks are recorded below rather than folded into those
historical counts.

## Camera authority

The original 32-bit object-service chain is concrete:
engine `+26c -> 30149350`, actor `+0 -> 3014bf90`, camera `+4 -> 3014bff0`.
Its typed XML readers preserve Pos XYZ and reorder Rotate WXYZ to XYZW. Import
does not normalize, rebase, compose a parent or change axes. Four original
`game/create_<race>.lyr` inputs establish four actor and 32 camera records.

The shop chooses camera entries 3/4, takes their position midpoint and shortest-sign
normalized quaternion midpoint, then adds the original Lua camera adjustments.
Widget FOV is native f32 `0x3f060a92` (30 degrees); layer FOV is not substituted.
Actor yaw composes about actor-local Z. The base frustum uses the actual admitted
texture aspect with near/far 0.1/100. Host basis conversion and translation rebasing
are applied afterward, consistently to the actor and camera.

The root consumer compares all 36 imported records exactly, eight base camera
outputs, 14 actor-yaw matrices and 13 conditional height results against retained
original-instruction output. Portable quaternion normalization is checked by its
resulting axes, with a small float tolerance; it is not claimed to reproduce the
original reciprocal-square-root instruction bit-for-bit. Bevy rendering and depth
encoding are separate from these camera input checks.

## Applied mode and heel correction

Camera correction reads **applied beauty-shop state**, not the latest response,
widget visibility, a salon-routing marker or deferred Bevy resource presence.
Native receipt replaces a pending transition: enter waits 1.0 seconds, exit 0.5.
The applied gate is written before synchronous `TOGGLE_BEAUTYSHOP(580)` delivery.
Constructor/reset write false; reset alone does not cancel an existing pending
callback. The separate special-enter and system-close branches retain their
documented boundaries. Root's applied-mode resource now tracks receipts even
while HUD listeners are unavailable, applies the gate after the corresponding
delay, and reconciles resumed UI delivery to the latest queued applied scene,
with the gate already updated. This
UI recovery policy is distinct from proof of retail notification timing during
HUD suspension. Visual fade composition and special/system-close admission are
not supplied by the scalar timing proof. A false reply no longer synthesizes the
separate system-close event.

With an admitted model and applied mode true, native arithmetic is:

```text
delta = f32(current_heel - reference_heel)
camera_z = f32(delta + base_camera_z)
```

Current heel comes from the preview's existing `CharacterAssets::heel`. Its model
root already includes the heel lift; the camera rule does not lift the model again.
Reference heel comes from `GameDb::modelview_reference_heel(race, gender)`:

1. Scan the original `characters` query order for the first race/gender match,
   including non-creatable rows.
2. Resolve that row's model and preview cloth pack, then costume and shoes.
3. Honor the armor group's default-asset precedence and original model-link order.
4. A present costume asset wins even at zero; otherwise try shoes, then native zero.

Proved missing rows/packs/assets yield zero. Database/query/row errors propagate.
The supplied catalog's Nuian female reference is `0.07999999821186066`; the other
seven supported contexts resolve zero. These are queried values, not hardcoded
defaults or a claim about every encrypted retail database version. The conditional
producer audit executes 47 cases, including original row loaders and callbacks.

## Texture extent and widget lifetime

Root uses actual renderer screen dimensions for the original sequential cap.
When width exceeds its limit, native code computes f32 height/width first, then
multiplies by admitted width and truncates. The corresponding height cap computes
width/height first. Reassociating these operations changes real integer results.
Ten additional original executions include three rounding discriminators and a
signed-overflow case. Root declines unusable allocation dimensions; no native
allocator or device-success claim follows from the scalar controls.

The toolkit queues ordered commands and snapshots every live surface, including
hidden ones. Its opaque identity survives Clear and retires permanently on widget
removal. Root supplies the actual image handle and allocation extent for pixel-crop
normalization, removes absent IDs, and guards asynchronous work within one widget
lifetime. Root owns actor/camera/assets/animation, target allocation and teardown.
`UiModelViews` runs after pointer/Lua callbacks and before UI painting.

Toolkit checks cover command order, retirement/reload, hidden handle replacement,
clipping, fade, scale, child order and pixel-only changes without geometry uploads:
**620 passed across 88 targets, one existing optional font check ignored**. The
retained diagnostic captures show initial (0.8s) and later (8s) states of the same
final binary with supplied grid images. They are runtime lifecycle controls, not
a before/after implementation or native character screenshot comparison. Later
FPS samples are 420.9/476.0/413.6/412.9 on the shared machine.

## Retained evidence and remaining integration

Ignored evidence is under
`data/extract/aa-ui/ingame-shop-audit-20261024/` in the modelview contract, camera
prerequisite, object-service and height-producer directories. Root's ten additional
texture controls are in `data/extract/aa-client/modelview-camera-20261024/`.
The numeric consumer fixtures pin their evidence hashes. Earlier accepted files
and goldens remain unchanged. Original modules execute only in bounded isolated
instruction fixtures; no host DLL loading, retail process or commerce occurs.

## App host and local trials

The private host consumes commands inside `UiModelViews`. Init captures complete
received equipment through `NativeItemRecords`, plus the player's customization
and supported model identity. Unknown history and transformed/unsupported models
do not become default characters. Same-object Init preserves the captured snapshot
while bound; a new Init after Clear copies the current received source. Reset uses
the copied snapshot rather than later live gear, including after Clear. Trial items are local ID-zero
records and never mutate inventory, equipment authority or network state.

`GameDb::modelview_trial_item` (`0a1e451a`) supplies descriptor/subtype and original
grade-zero constructor metadata. The reducer preserves the native weapon conflicts,
armor/costume rules, backpack flags, raw default dye and charged detail prefix.
Retained fixtures cover 35 original slot results and ten actual constructor records.
The six appearance bytes and selector are preserved; existing Bevy carried-model
placement is not proof of the original assembly endpoint or glider deployment.
An additional static check of the pinned original Init closes its selector and
six-byte initialization: `392bbfb7..392bbfd6` writes selector zero and copies
`00 00 00 00 14 64` from `399c8bd0`, before testing `create_model`. These values
are therefore original Init writes, not guessed received fields or borrowed
Reset defaults. `trial/native-init.json` pins the bytes/offsets and scope. Full
native actor resolution/assembly and the other opaque adjunct bytes remain
outside that check. Reset admission uses the copied model header even while the
first actor is loading or after Init without assembly. Native Clear (`392baf80`)
retains both appearance snapshots but zeros the active source gender/race. Root
keeps this context separate: Reset can rebuild the copied appearance without
restoring that binding, its reference heel is then native missing-row zero, and
gender-specific trial admission uses the cleared context. A later Init binds and
copies the actual source again.

Two background jobs at most prepare complete assets. Widget incarnation, owner
epoch and request revision fence completions. A failed candidate retains the last
complete actor. A failed preparation reconciles the latest requested playback on
that retained actor, including Stop/Play queued in the failure's delivery frame;
an unavailable replacement clip preserves its last successful clip and clock.
Clear releases content and its object binding while retaining both
copied appearance payloads and target allocation. Destroy, runtime retirement and
owner changes discard the copied data and retire resources. All live widgets retain
lightweight metadata; only admitted model requests consume one of eight scene
permits on layers 128 through 135. These are application budgets. Per-camera ambient and directional lighting use
the existing renderer, not recovered native shop lighting.

Teleport's temporarily suspended HUD retains its view snapshots, actors and
monotonic revisions, with absent cameras disabled. Restoring the same runtime
resumes those views without replaying Init. Genuine retirement is collected after
suspension; owner invalidation and explicit Destroy remain effective during it.
The loading runtime can replace the shared GPU glyph atlas. Resume marks the
retained runtime's atlas dirty as well as invalidating geometry, so its existing
labels reappear without requiring new glyphs. The regression checks actual atlas
bytes through overwrite, suspend/resume and upload.

Widget cameras are excluded from app world picking, projection, culling and camera
parking. `aa-world` commit `0f3f3cac` restricts fallback selection to layer zero and
adds `WorldFocus` priority to dome/ocean following. It preserves image-target world
cameras and explicit Post opt-ins. Shared aa-cry character lighting and Bevy's
extra-view/cascade/range work remain; no independent lighting or zero-cost claim
follows from render layers.

The first two exploratory captures rendered but logged startup mesh-bind-group
errors. Direct drawing did not resolve them and was removed. A preview-only guard
now checks the actual metadata/morph/skin mesh-group key before calling the original
draw. It defers unavailable groups, without catching draw errors or changing
unmarked views. A third exploratory run deferred three draws, rendered the actor,
and had no errors. This is a per-draw binding prerequisite, not a complete GPU
readiness or native pass certificate. The original Init return remains conservative
false. Earlier failed logs/captures remain retained.

`cargo run -p aa-client --example modelview -- --case initial --shot <png>
--shot-after 12` exercises original meshes, layer camera, clip and shop crop with
an explicitly supplied received snapshot. `weapon`, `staff`, `armor`, `reset`, `clear-reset`,
`resize`, `hidden`, `clear`, `lifecycle` and `suspend` exercise the same host.
`initial-applied` and `clear-reset-applied` use the actual delayed salon-state
producer to distinguish bound reference heel from Clear's zero context. The fixture is
not a commerce frame and sends no socket traffic. Captures use the
rebuilt executable with its fixed skeletal pose and reject shader/render errors.
Independent facial timing is not clamped by that fixture.

## Checks after the lifecycle fixes

Host source is frozen at `38157e9b`, following the received-feature gate commit
`c06515a0`. The coordinated animation metadata and world-camera isolation are
`0a1e451a` and `0f3f3cac` respectively.

`cargo test --locked -p aa-client --bin aa-client` passed **732 tests**, with
71 optional checks ignored. The separate `modelview` filter with
`--include-ignored --test-threads=1` passed all **26 checks**, including the
original pak cameras and catalog constructors, actual rig/clip failure recovery,
same-frame Lua playback controls, suspended-runtime revision fencing, original
salon scene transitions and exact restored glyph-atlas bytes.

The main binary and modelview example both build with `--locked`. The source
manifest pins 32 app/example/tool inputs. One Clippy invocation completed with
baseline warnings; four new diagnostics were fixed. The later quota, lifecycle
and atlas corrections were tested and rebuilt without a second lint invocation.
The final independent static review found no introduced P1/P2 findings. That
review is not runtime or native-parity evidence.

Fourteen final 12-second captures passed their actor/clip/target/context assertions
and strict shader/render-log rejection. Normal main-app startup also passed and
was inspected at server selection, without logging in. Reset, Clear then Reset,
and reinitialization each match the initial PNG byte-for-byte; the Clear baseline
also matches its no-image control exactly. The bound-versus-cleared applied-mode
reference changes 19,305 pixels and passes the actual camera-transform checks.

The suspension fixture preserves actor identity/revision and restores both label
regions exactly. Its full image differs at 75 face-region pixels
(`214,260..231,284`); the cause is not established, and full-image suspension
identity is not claimed. An earlier capture with 16,906 changed pixels exposed
missing text from the shared glyph atlas; that failed evidence remains retained.
Thirteen unaffected controls are byte-identical before/after the atlas correction.

Observed initial-preview FPS was **346.5–404.0**; the empty-image control was
**373.5–565.0**, and normal startup was **401.6–432.0**. All are shared-machine
observations, not isolated cost or native-performance comparisons. The full matrix
ranged **296.6–570.6 FPS**. Source/binary/PNG/log hashes, per-case FPS, pixel counts
and the interactive comparison are retained under
`data/extract/aa-client/modelview-host-20261024/verified-evidence.json` and
`comparison.html`. Executable copies are removed after recording their hashes.

Catalog/wallet/error archives and legacy web-market behavior remain separate
prerequisites. No missing value becomes a successful actor, camera, wallet,
purchase or ready state. Production Illum pass routing and vegetation R2 remain
held independently of this host.
