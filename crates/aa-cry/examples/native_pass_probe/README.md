# Supplied coverage renderer mechanism probe

This private example compiles and executes Bevy **0.20.0** renderer hooks. It is
**not native Illum material completion**, a native GPU oracle, a camera policy for
the client, or an implementation of the native instance-input producer. Production
sources, public APIs, shaders, manifests, dependencies and Cargo.lock are unchanged.

```powershell
cargo check -p aa-cry --locked
cargo build -p aa-cry --example native_pass_probe --locked
cargo run -p aa-cry --example native_pass_probe -- --shot shots/aa-cry/native-pass-probe.png --size 640x480 --shot-after 6
python crates/aa-cry/examples/native_pass_probe/run.py --archive
python crates/aa-cry/examples/native_pass_probe/validate.py
```

The example uses `aa_dev::default_plugins` and `DevPlugin`. `run.py` copies the
built executable, runs each control sequentially with screenshot/exit, records
commands/source/executable/attachment hashes and FPS, and removes its own copy.
Python validation requires Pillow. No Rust dependency or example declaration was
added. Timestamp diagnostics request GPU timestamp-query features; measured runs
use Vulkan on the shared RX 9070 XT. CPU timing and FPS remain ordinary wall time.

## Explicit supplied inputs

Both cameras, every coverage scalar and every motion admission are supplied test
inputs. Camera 0 requests participating Z; camera 1 deliberately requests initial
forward rendering. `--no-z` also suppresses participating Z for camera 0. Stock
materials keep their own prepass producers in either camera. Their depth does not
grant a participating draw a Z ticket. This distinction preserves requested stock
normal attachments and does **not** resolve native no-Z/deferred/SSAO camera policy.

`--attachments none|depth|normal|motion|all` requests the indicated actual Bevy
attachments. Normal-only and motion-only can submit Z without a DepthPrepass
marker. `--separate-targets` puts camera 1 on an Image target. `--occlusion` enables
stock early/late GPU culling. No batching/indirect-drawing disable is used in the
acceptance runs. An optional `--direct-diagnostic` exists solely for debugging.
`none` explicitly requests initial rendering with no prepass resources. Its
bounded acceptance scene uses stock StandardMaterial/zero-token Illum controls.

The six green rows, top to bottom, repeat three instances of each group:

| Group | Supplied diffuse halves | Cutoff | Composed alpha | Purpose |
|---|---|---|---|---|
| 0 | 0 / 1 | .25 | 1 | Diffuse holes after Z versus composed initial coverage |
| 1 | 0 / 0 | .25 | 1 | All-hole submitted Z must stay on Equal, never fall back |
| 2 | .3 / .8 | .059 | 0 | Output alpha 0 survives post-Z; motion discards .3 at .5 |
| 3 | 1 / 1 | .25 | 1 | Actual PipelineCache lookup of an unavailable paired ID |
| 4 | 1 / 1 | .25 | 1 | Explicit resource-availability Skip before DrawMesh |
| 5 | .3 / .8 | .059 | 1 | Participating Z withheld on alternate frame epochs |

Green is the output color; raw alpha is a procedural supplied pattern, not DDS
sampling. Per-vertex `control.rgb/a` carries the supplied raw halves/cutoff/composed
alpha. It is **not** native vertex color, PB/PI binding, material opacity, dissolve,
or an inferred runtime default. Groups 100/101 are explicit eligible Blend/Add
motion controls; neither their AlphaMode nor a view marker grants eligibility.
Their supplied velocity weight is .5 and runtime alpha testing is not selected.

Blue and red bottom controls use unmodified StandardMaterial and actual
IllumMaterial with its zero private token. The lit receiver mode also uses both
actual material types. The existing Illum positive-token composed coverage cannot
be used honestly for the new native coverage paths while its shader seam is frozen.

## Executed mechanism

* `PrepassCommands` appends `RecordZ` **after actual DrawMesh**. Command tuples stop
  on Skip/Failure. The recorder uses a read-only resource containing a Mutex.
  A separate actual DrawMesh missing-mesh witness returns `Draw::draw()==Ok(())`
  while the trailing recorder remains zero. A successful draw result is not a
  submission certificate.
* Tickets contain current view entity, render-frame epoch and paired Z pipeline
  identity. The ledger resets before each frame. Group 1 records all three
  instances even when every fragment discards. Group 5's next-frame suppression
  cannot reuse the preceding ticket. No marker or sampled depth value selects
  the forward route.
* Initial, after-Z, Z and motion variants are cloned only from processed, ready
  base descriptors. The after-Z pipeline uses **Equal, depth writes false** and
  removes composed clipping while retaining supplied output alpha. Z and forward
  are both withheld if the pair is unavailable. Resource/binding/DrawMesh skips
  cannot append a ticket. Intentional no-Z uses the initial pipeline independently.
* Both phase caches receive distinct pipeline identities for the paired view,
  source group, actual mesh/material assets and original main/prepass pipelines.
  Existing Bevy material/slab/bin keys remain intact. Identity changes mark
  `DirtySpecializations.changed_renderables`, because persistent bins do not
  automatically notice cache-ID edits. Three instances remain one automatically
  batched indirect range; every repeated column is pixel-checked. There is no
  blanket NoAutomaticBatching/NoIndirectDrawing substitution.
* One shared invariant vertex entry computes the current rigid position in
  forward, Z and motion. Actual Bevy mesh/view/material bind groups and DrawMesh
  are reused, including ordinary and bindless material binding.
* Stock early/late prepass graph nodes are replaced **before initialization**
  using public `ScheduleGraph.systems` / `SystemWithAccess`. Existing graph edges
  and stock system-type anchors stay attached to those nodes. Matching original
  function system types avoids dependence on disabled debug names. Unmarked
  views invoke the original stock functions with the original query/arguments.
  Shadow draw-function registries are not replaced.
* Adding an arbitrary replacement with `.in_set(early_prepass.into_system_set())`
  **panics in Bevy 0.20**. Removing/readding that way in the accepted proposal was
  not a usable hook. The early deferred function is private. Node replacement
  avoids guessing its ordering or changing Bevy registry sources.
* Participating Z and stock Z/normal producers write no motion. Motion is replayed
  after MainPass (therefore after main indirect-parameter rebuilding and initial
  forward depth production), before the `PostProcess` set. This does **not**
  establish ordering against `EarlyPostProcess` or every possible motion consumer.
  Early/late guards,
  depth copies and viewport are retained. All writers to the owned motion
  attachment are split on marked views. Nonparticipating material equations stay
  stock. Explicit Blend/Add items are queued independently with the public
  `PrepassPipelineSpecializer` into the already registered/preprocessed phase;
  stock AlphaMode prepass exclusion cannot drop them.
* The **actual ViewPrepassTextures.motion_vectors** attachment is initialized once
  with supplied sentinel `(0.125,-0.25)`. The later pass uses Load/Store and no
  depth writes. The depth attachment is read-only during motion, so wgpu also
  rejects accidental depth-writing pipeline replay. Motion uses supplied ordinary
  reversed-Z GreaterEqual, independently
  of forward Equal, so no-Z composed holes do not become a second motion clip.
  Raw alpha .3 is discarded, .8 produces nonzero previous/current-transform motion.
  Eligible Blend/Add output uses raw alpha times explicit supplied weight.
* A moving stock backing under group 2 provides real prior velocity. In the Z
  view it is occluded and cannot leak velocity through the foreground's .5
  discard. In the initial view it is visible; its prior velocity survives the
  native-control discard. The high half receives foreground motion. No zero-vector
  substitution or fullscreen repair is used.
* Depth, normal, color and motion readbacks come from the actual renderer
  attachments. Readable copies of the requested color attachments become the
  actual ViewPrepassTextures attachments for the probe; depth COPY_SRC is requested
  explicitly. `before-motion`/`motion` bit equality verifies that motion changes
  no depth, normal or forward color. CSVs retain submission/readiness/range state.
  Background motion is explicitly disabled on probe cameras so the sentinel has
  clear ownership.

## Production contract required before editing

This is a concrete proposal for coordination, not a released API or authorization.

1. **Shader seam in aa-cry**: `src/shade/illum.wesl`, `illum_prepass.wesl` and
   `illum_bindings.wesl` need private pass definitions for initial, after-Z, Z,
   shadow and motion. Retain `fragment` entry names. Separate output-alpha
   evaluation from coverage. Initial clips composed alpha at authoritative PI.w;
   after-Z does not call Bevy Mask `alpha_discard` or force output alpha to 1;
   Z/normal/shadow use their source Diffuse/PB.w predicates; motion ALPHATEST uses
   fixed .5 and ALPHABLEND uses explicit source weight. Preserve native UV/sampler,
   deformation, runtime alpha-test enable/disable and all accepted alpha/glow laws.
   Bindings 55-58, light lookup, clock and zero-token behavior stay intact.
2. **Private renderer registration**: new `src/shade/illum/passes.rs` with
   `pub(super) fn install(app: &mut App)` called by `IllumPlugin::build` after root
   has installed all Core3d/material producers, before RenderStartup/PrepareAssets.
   Register aliases through `DrawFunctionsInternal::add_with` for the type-erased
   `DrawMaterial`, `DrawPrepass`, `DrawDepthOnlyPrepass` labels only in camera phases;
   retain stock fallback commands and leave Shadow alone. Root owns global plugin
   order: these labels are **global**, not generic Illum-only labels. Preserve
   existing system nodes/edges and require exactly one original early/late node.
   Pair planning runs before both public queue systems; requeue when identities
   change. Motion runs after the final depth-producing stage and before consumers.
3. **Caller authority (new API must be agreed)**: the minimum proposed view
   component is `IllumViewPassPolicy { z_requested: bool }`, alongside existing
   attachment requests, with no implicit installation/default. A matching ticket
   remains private. Instance inputs require a separately coordinated component or
   adapter with the exact value contract
   `IllumInstancePassInput { z_eligible: bool, motion_eligible: bool,
   pb_alpha_test: Vec4, pi_alpha_test: Vec4, motion_weight: f32 }`.
   No Default is proposed: root must supply every value from proved native inputs.
   These eligibility booleans are the result of the full native qualifiers, never
   AlphaMode or marker guesses. Private loader metadata retains source/permutation
   identity independently of converted AlphaMode and follows the same entity
   through LOD wrapping. Public IllumParams/IllumExt fields need no addition.
4. **Per-instance GPU input indexing remains a real seam**: production needs
   `fn native_instance_input(instance_index: u32) -> NativeInstanceInput` available
   in every participating pass. Its payload must distinguish PB and PI vectors and
   the explicit motion weight. The current material-only registry cannot represent
   different object byte99 biases on shared material instances. Do not steal
   MeshTag, binding55 or assume Bevy's per-phase packed instance index is a stable
   entity index. Root must agree either a real packed instance channel/binding or
   distinct, deduplicated material input variants with correct wrapper retention
   and invalidation. This probe does not choose a binding number or fake that seam.
5. **Exact position contract across owners**: private shared invariant current
   position/IO modules in aa-cry, with corresponding aa-world
   `objects/lod_vertex.wesl` / `lod_prepass.wesl` integration. Preserve outer
   Default-fragment forwarding, skin/morph current and previous poses, compressed
   vertices, jitter, viewport/resolution, depth bias and identical visibility
   dither. Ticket/group identity must include the actual source, binding generation
   and deformation/LOD snapshot; the current fixed rigid fixture is not sufficient
   evidence for those changing production inputs.
6. **Root camera/effect policy**: resolve no-Z versus requested deferred/SSAO normals,
   motion attachment ownership and downstream consumers explicitly. Current login
   and in-game camera constructors do not request prepass; aa-world Post may add
   depth/normal, and there is no current MotionVectorPrepass caller. Never enable
   native object motion from that absence/presence. A future shared motion-ready
   system set is needed only when another owner consumes this result.

Root's 477 input cases pin the PB/PI vectors, byte99 bias, list7/17 dynamic override,
motion qualifier and configured/effective/suppressed motion flow. Their golden hash
is `0adce48e1d67a6fdead72aa8e3d8f6d168ac77bf59fcc113fc70786260f1bbbb`.
Motion requires the signed technique alias, object high bit, effective global,
ordinary recursion and absence of the proved decal/object exclusion flags.
Requested CVar default 0, authored config 0 and scene suppression are inputs, not
a captured selected profile. PB.z's SSDO time channel has not gained parity here.

## Proof limits

No native view/instance defaults, scene capture, shipping DXBC execution, original
DLL load, server or live-client operation occurred. There are no real-DDS/source
composition or native sampler tests in this example. The earlier 372 routing,
opacity/scheduling evidence and Texture.attributes commit `0bd4e470` are inputs,
preserved byte-identically. Raw Texture attributes remain uninterpreted.

Rigid, single-sample rectangles establish the mechanism only. Skin/morph/outer
LOD overrides, compressed input parity, native sample-mask/MSAA coverage, jitter
changes, resource/registry churn, nested wrappers, hardware motion encoding versus
native encoding and native deferred payloads remain production acceptance work.
`--occlusion` exercises stock early/late scheduling and final indirect replay; it
is not a controlled native disocclusion/animation scene. Transparent controls
establish independent admission and supplied weights, not native transparent
ordering/state for all permutations. Replacing readable attachments is probe
instrumentation, not a proposed production allocation policy. GPU timestamp and
CPU/FPS measurements describe this small shared-machine example, not world-scale
cost or a measured production regression.

## Historical verification at 507b6546, 2026-10-24

`data/extract/aa-cry/native-pass-probe-20261024/verified/` retains **15 clean runs**
and **5,496 passing assertions**. `captures.json` pins the exact executable,
Rust/shader source, commands, every attachment/PNG hash, shader logs, FPS and
timestamp diagnostics. `validation.json` states the proof scope. Raw readbacks
are packed losslessly in each case's `attachments.zip`; every entry was compared
byte-for-byte before packing, and validation passes directly against the archives.
Both executable copies were removed after runs. No foreign process was stopped.
These captures remain historical and unchanged. They do not cover the startup
plan/draw readiness race found in independent review-01. Current capture defaults
use a separate directory and `run.py` refuses to write under the historical root.
Validate historical data without changing its result:

```powershell
python crates/aa-cry/examples/native_pass_probe/validate.py data/extract/aa-cry/native-pass-probe-20261024/verified --read-only
```

The executable SHA256 is
`9d48813466402e90e8e50ff99836a9ee3225780afb6e3102e0afbf397be9ae3d`.
Bindless/ordinary actual color, depth, normal and motion bytes match exactly at
both retained frame epochs. All three instances of each participating group are
checked, including the all-hole submission and frame-reset discriminator.
Directional and point shadow captures match the stock-renderer controls exactly;
switching shadows off changes **5,788** and **11,006** receiver/control pixels,
respectively. Unmarked-camera fallback also matches the stock screenshot exactly.

The inspected supplied captures show holes, complete all-hole rejection after Z,
initial composed coverage and real backing velocities. Actual retained motion is
approximately **±.00249863** for the foreground, **±.00499725** for the backing,
and **±.000374794 / ±.000999451** for explicit Blend/Add alpha-weight controls.
Discard preserves either the supplied sentinel or the actual prior backing
velocity. After-Z keeps group 2's output alpha **0** while retaining its color.

Measured at 640x480 on Vulkan/RX 9070 XT and Ryzen 5 5600X, with other sessions
sharing the machine: default bindless **235.1–239.0 FPS**, ordinary **251.3–282.5**,
depth-only **246.0–294.2**, normal-only **243.2–309.4**, motion-only **254.5–289.1**,
explicit no-Z+motion **258.2–294.1**, separate targets **252.0–268.5**, and
occlusion-enabled **183.5–218.4**. No speed comparison is inferred from these runs.
Default pair planning averaged **19.44 µs/frame** (two supplied views). Render
diagnostic span means across sampled frames were **22.14–22.29 µs CPU /
21.21–21.27 µs GPU** for supplied Z, and **19.38–20.18 µs CPU /
19.84–19.85 µs GPU** for split motion. These are the recorded spans, not total
engine time, independent-queue cost or an isolated production overhead estimate.

`cargo check -p aa-cry --locked` and the example build pass. The accepted-input
hash manifest covers **1,578 files** including aa-cry production sources, all
manifests/lockfile and the accepted research directories; all compare unchanged.

The single Clippy run completed with 50 existing library warnings and 15 probe
suggestions; probe suggestions were addressed, required multi-resource render
systems have documented lint expectations, and Clippy was not rerun. No baseline
cleanup was made. The PowerShell wrapper classified stderr warnings as a nonzero
shell result; the retained Cargo log ends in `Finished` with no compiler errors.

## Review-01 readiness fix

The P2 concerned a stock base unavailable in `plan()`, then ready before draw.
The old marked-view fallback selected that unsplit original in both Z and motion.
It could write motion too early and replay a depth-writing pipeline. Marked stock
draws now require an entry in the split plan and the selected variant must be
ready; otherwise they Skip. The unmarked-view original fallback is unchanged.
Planning retains the base ID for later processing and dirty/requeue notification
adopts the split Z identity when it becomes available. No blanket batching flag
or production source change is involved.

`readiness.rs` supplies a deterministic control, enabled by
`--readiness-transition`. The `readiness` and `readiness-ordinary` configurations
use the default marked cameras with all attachments. The control queues a
**fresh real stock pipeline ID** from a warm
descriptor at the capture epoch, asserts PipelineCache reports it unavailable
during planning, then performs real compilation **after plan/queue and before
draw**. The first availability frame must have no split plan, no selected original
pipeline and no actual DrawMesh submission in either Z or motion. The next frame
blocks only this test's split variants to ready, asserts the retained queue now
contains the split Z ID, and records actual Z and motion submissions. This is the
first availability of an introduced pipeline at epoch 360, not a claim about all
first-launch process timings. Blocking compilation is fixture instrumentation,
not the proposed production readiness behavior.

CSV records pin actual before/after readiness, queued and selected IDs, selected
depth-write state and the trailing DrawMesh record. GPU snapshots require the
entire before-motion attachment to retain `(0.125,-0.25)`. The cold stock control
stays at that sentinel after motion, then receives its actual stock velocity on
the following planned frame. A read-only depth attachment plus unchanged raw
depth bytes rejects depth-writing original replay. Its ordinary stock **main**
draw may still supply its own depth while Z is deferred; validation allows that
change only inside the stock control's exact footprint, preserving the native
post-Z no-depth-write assertion everywhere else.

`none-stock` / `none-split` also compare actual no-attachment cameras. The marked
capture asserts ViewPrepassTextures and both prepass phases are absent, retains
stock main color/depth, and must match the stock-renderer lit/shadow screenshot.
This does not close native caller policy or deformation/MSAA/positive-token work.

New evidence is separated under
`data/extract/aa-cry/native-pass-probe-review01-fix-20261024/`; the original 15-run
proof and all earlier accepted hashes are retained. No additional Clippy run is
permitted or performed for this fix. Ordering against EarlyPostProcess, controlled
current late disocclusion, deformation/binding-generation/MSAA and actual
positive-token Illum shadow coverage remain production prerequisites.

Final-source verification for this fix passes **8,105 assertions across 19 clean
GPU runs**, including **388 logical attachment hashes**, both deterministic
readiness cases and the no-attachment comparison. The other 15 configurations
were rerun because the change affects the shared camera prepass dispatcher and
motion attachment state. All final captures pin one Rust/shader/support source
manifest and executable SHA256
`005a93a43a42acea1bad3f57748f3f88bf3cba80d1d70eaec727c4441b7138cd`.
The original 538 evidence files and all 1,578 earlier frozen production/input files
compare unchanged. Updated validation supports read-only historical checks and
checks every retained data/PNG/log/CSV/archive hash before pixel assertions.

At 640x480 on the shared machine, the final default capture observed **254.0-291.9
FPS**, **20.36 us/frame** pair planning, **13.31-15.18 us GPU** Z spans and
**12.22-14.01 us GPU** motion spans. Readiness controls observed **165.0-220.6** FPS
bindless and **188.0-206.9** ordinary; no-attachment split **206.6-238.6** FPS. These
are new-run observations, not a before/after performance claim. Focused example
check/build pass; no Clippy rerun, production mutation or foreign-process action
occurred. Owned executable copies were removed by the runner.
