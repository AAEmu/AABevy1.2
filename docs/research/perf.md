# In-game performance pass

Date: 2026-10-17. Release builds (`cargo build --release -p aa-client`, copies run from `target/agents/perf/<label>/`), 1920 x 1080
windowed, Ryzen 5 5600X (6 cores, 12 threads), RX 9070 XT, account `bevyperf` (character `Perfrunner`), `--hour 12` (the
same light in every run). The machine was shared all day: other sessions' builds (rustc, three clippy-drivers), a real
ArcheAge client and two AAEmu servers kept the CPU at 95-100 % (`typeperf "\Processor(_Total)\% Processor Time"`) during
nearly every measurement below. Single runs swing by a factor of 3; every comparison is alternating runs of the builds.

Spots (`aa-client --enter-world --account bevyperf --hour 12 --pos X,Y --yaw D --bench 15 --bench-after 15`):

| spot | flags | what is there |
|---|---|---|
| Nuian spawn | `--pos 15578.042,15382.122 --yaw 160` | 4-6 k part entities, 12 units, 229 effects (7 active) |
| Marianople | `--pos 11186,12002 --yaw 0` | 39.9 k entities, 19.2 k mesh entities, 89 units and 197 doodads drawn, 1410 effects (571 active, 1188 emitters) |
| Haranya jungle | `--pos 21000,9000 --yaw 180` | 13.7 k entities, 9.3 k mesh entities, 13 units, 734 effects (49 active, 13.5 k particles) |

## Tools

* `AA_PROFILE=1` (`aa_world::profile`): every 3 s the main thread slices, the render thread stages, the process' CPU cycles
  per frame (all threads; stable under load where milliseconds are not), Bevy's render diagnostics (CPU / GPU time per
  pass) and the entity / visible mesh counts. `AA_PROFILE_THREADS=1` adds cycles per thread.
* Per-system time: `cargo build --release -p aa-client --features trace` (new feature of aa-client: `bevy/trace_chrome`;
  rebuilds Bevy once into its own artifacts, the binary replaces `target/release/aa-client.exe`, so copy it aside and
  rebuild the normal one). Run with `TRACE_CHROME=<file.json>`: every system, schedule, render pass and par_iter task is a
  span. 20 s in game are 1-2.3 GB of JSON; `tools/trace-sum.py <trace> <from_s> <to_s>` sums the spans per thread kind
  (main, render, compute pool) per frame, `tools/trace-long.py <trace> <from_s> <to_s> <min_frame_ms>` lists the long
  frames and what ran in them. Tracing costs about 10 % of the frame rate.
* `--bench` (`ingame/bench.rs`) now also prints the asset changes per frame (meshes, images, standard and colour
  materials: every changed asset is extracted and prepared again by the render world) and aa-fx's counters.
* `aa_cry::prof` sampling (`AA_SAMPLE=<thread names>`) suspends the sampled threads every 0.7 ms and slows the frame a lot
  (36 fps against 70); only useful for the self time of leaf functions.

## Where the frame goes

`AA_PROFILE=1`, release, quiet-ish rounds (before the fixes):

| | spawn | Marianople | jungle |
|---|---|---|---|
| frame | 14.6 ms (68 fps) | 25.6 ms (39 fps) | 22.3 ms (45 fps) |
| main thread: update / anim+propagate / check-visibility / post | 2.4 / 0.7 / 1.1 / 0.5 ms | 2.8 / 1.0 / 2.8 / 1.0 ms | 3.7 / 1.5 / 3.3 / 0.7 ms |
| main thread waiting for the render thread (`gap`) | 9.7 ms | 17.7 ms | 12.7 ms |
| render thread: assets / bind-groups / write-buffers / render+present / acquire | 1.5 / 1.4 / 1.1 / 5.1 / 3.3 ms | 9.5 / 1.8 / 1.4 / 8.5 / 0.5 ms | 2.7 / 6.0 / 1.1 / 5.5 / 0.6 ms |
| GPU, all passes | 3.8 ms (opaque 1.7, prepass 0.4) | 2.8 ms | 3.0 ms |
| CPU cycles of the process per frame | 120 M (compute pool 85, render 19, main 15) | 231 M | 136 M |
| entities / mesh entities / visible / distinct mesh+material | 8.6 k / 4.7 k / 931 / 134 | 39.9 k / 19.2 k / 3197 / 724 | 13.7 k / 9.3 k / 983 / 206 |
| mesh assets changed per frame (distinct) | 17 (126) | 836 (1257) | 146 (272), 50 k vertices |

The frame is bound by the render thread everywhere; the GPU is idle most of the frame (3-4 ms of passes) and the main
thread waits 10-18 ms per frame for the render thread. Shadows: one directional light, 3 cascades to 300 m (aa-world
`sky.rs`), units cast shadows within 45 m. Draw calls are not counted by Bevy; the distinct visible mesh + material pairs
(the batches of the GPU preprocessing path) are the proxy: 134, 724 and 206.

Per system (trace build, Chrome trace, ms per frame; the compute pool runs both worlds' parallel systems):

* Nuian spawn (77 fps under tracing): main thread `Main` schedule 6.8 (PostUpdate 3.1, Update 2.3 of which `aa_ui::ui_frame`
  1.2 and `hud::sync_hud` 0.11, ExtractSchedule 0.8), `RenderExtractApp` 5.8 (mostly waiting for the render thread).
  Render thread `Render` schedule 11.8: `render_system` 5.0 (`camera_driver` 1.7, `submit_pending_command_buffers` 2.7 of
  which `queue_submit` of 26 command buffers 1.7, `present` 0.5). Render systems on the compute pool:
  `allocate_and_free_meshes` 1.4, `prepare_preprocess_bind_groups` 1.0, `prepare_windows` 0.6, `write_binned_instance_buffers`
  (shadow 0.45, opaque 0.29, prepass 0.18), shadow / prepass / opaque command encoding 0.6 / 0.24 / 0.23. Main world on the
  pool: CPU culling and the cascade culling 1.1 (par_iter), transform propagation 0.5, `aa_world::objects::stream_objects`
  0.23, `aa_anim::animate` 0.13, `aa_fx::simulate` 0.08. **All aa-client systems together: 0.4 ms**; aa-ui 1.26, aa-world 0.45,
  aa-fx 0.16, aa-anim 0.13.
* Marianople (38 fps): `allocate_and_free_meshes` **8.1 ms per frame** (max 11 ms), `extract_render_asset<RenderMesh>` 0.9,
  `prepare_assets<RenderMesh>` 0.8, the meshes' `Aabb` recomputed (`calculate_bounds`) 0.35 and `mark_3d_meshes_as_changed`
  0.26: the 836 mesh assets aa-fx rewrites every frame (one per emitter of the 571 active effects, about 2 quads each). Then
  the shadow pass encoding 2.1, the early prepass 1.4, the opaque pass 1.1, CPU culling and cascades 4 (par_iter).
* Jungle (74 fps under tracing): `allocate_and_free_meshes` 2.1, aa-fx simulation 2.6 (par_iter over 49 effects with 13.5 k
  particles), mesh extract + prepare 0.7, `prepare_preprocess_bind_groups` 0.8, culling 1.8.

## The 3-10 fps stalls

The log sweep (`docs/research/log-sweep.md`) saw the in-game fps drop to 3-10 for seconds at a time, every few seconds. The
same happens in release builds (e.g. 50 s at the spawn: 70-85 fps for 30 s, then 11 fps for 2 s with 88-160 ms frames;
the normal login flow: 6-8 fps for 10 s right after entering). It is not the game's own work:

* the process' CPU cycles per frame stay the same in the slow windows (124 M against 117 M per frame in the fast ones), all
  main thread slices grow alike (check-visibility 6.5 ms instead of 1.3), the render thread's `bind-groups` stage (where it
  waits for the most parallel systems) grows from 1.5 to 7-15 ms;
* the machine's CPU was at 98-100 % in every slow window (other sessions' rustc, clippy-driver, a real client, Defender);
* raising the process' priority class removes them: alternating `Normal` / `AboveNormal` runs at the spawn with the CPU at
  100 % (same binary, 15 s each): **41 / 28 / 57 fps (1 % low 4 / 4 / 6, 226 / 212 / 112 frames over 20 ms) at normal
  priority, 68 / 76 / 82 fps (1 % low 17 / 45 / 59, 20 / 2 / 3 frames over 20 ms) above it**.

A frame hands work between the main thread, the render thread and the six compute pool threads dozens of times; with every
core taken, each hand-off waits for a scheduler quantum, and a frame of 13 ms becomes 100-400 ms. The client now raises its
priority class to above normal at start-up (`aa-client/src/priority.rs`; `AA_PRIORITY=normal` keeps the default). That
takes CPU from builds while the game runs (the game uses 2-3 cores), not more.

## Entering the world

* `--enter-world` has no loading screen and no prewarm: its first frames build the whole HUD stage (0.6 s in release,
  `login: UI stage ready`), compile the pipelines and upload everything at once (frame 2: 922 ms in release, 768-2847 ms in
  the sweep's debug build). It is a test path; the player's path is the login flow.
* The login flow (`--account bevyperf --goto loading`): loading 2.8-5.2 s, first in-game frame 0.1 s later, frame 2 in game
  57-104 ms, nothing else over 33 ms in the first 5 s when the machine is not saturated. Traced: frame 1 is 78 ms
  (`hud::build_hud` 31 ms: the entering events into the prebuilt Lua stage; `aa_ui::ui_frame` 25 ms: the first layout and
  meshes of the whole HUD), frame 2 47 ms (`bevy_sprite::update_text2d_layout` 34 ms: the first layout of the name plate and
  floating text fonts, `prepare_assets<GpuImage>` 4.8 ms).
* `hud: game data in 615-1077 ms` is the HUD's database read on its own thread (`hud::data::SharedGameData`), once per
  process, started at start-up; nothing waits for it. `login: UI stage ready in 621-1615 ms` on every entry of the sweep is
  the `--enter-world` path building the world stage in one frame; the login flow builds it during loading in 8 ms slices
  (58 frames, 665 ms of work, worst frame 61 ms) and swaps it in (27-31 ms). Terrain layers (0.3-0.9 s) load on a task.

## Character create, step 3

GPU bound: `main_opaque_pass_3d` 5.1 ms of GPU (40 visible standard meshes plus the grass), `lightprobe_irradiance_map` +
`lightprobe_radiance_map` 1.0 ms (the stage camera's `GeneratedEnvironmentMapLight`, aa-cry `stage::environment_light`,
filters the cube map again every frame), 6.8 ms of GPU in all; the render thread waits 6.5 ms per frame for the swap
chain. Main thread 2 ms. 70-80 fps here with the GPU shared with a real ArcheAge client.

## Fixed in aa-client

* `priority.rs`: above normal priority class (see the stalls).
* `ingame::units::move_units` wrote every unit's `Transform` every frame, moving or not; a changed root makes Bevy propagate
  through all of its bones and parts and re-extract their meshes. It now writes only a real move (0.1 mm, 1e-6 rotation).
* `hud::plates::update_plates` wrote every plate root's translation every frame; now only when it moved.
* `ingame::bench`: asset change counters and aa-fx counters in the `BENCH` lines.

The aa-client systems are 0.4 ms of the frame, so the last two are small (within the noise of this machine).

The alternating-round results table, the aa-fx section and the ranked list for other crates were never written: the
session that measured them ended first. The later aa-fx fix (77d3f01, invisible emitters skip their mesh rewrites) has
not been re-measured either.
