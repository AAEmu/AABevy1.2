# aa-client: character creation (`login/create`)

The create screen runs the **original Lua UI** (`loginstage/character_create/*`, `customizing/*`) on
aa-ui, with a live 3D preview of the character on the login stage and the real create request to the
server. Code: `crates/aa-client/src/login/create/`.

```bash
# unattended: log in as bevycreate, pick the server, jump to the screen, replay step 1..3 for a model
cargo run -p aa-client -- --goto create --race elf --gender female --step 2 --account bevycreate \
    --shot shots/create/step2_elf_female.png --shot-after 20
```

## What works

| Step | UI (original scripts) | 3D |
|---|---|---|
| 1 race / gender | 4 races x 2 genders, race text, advantages (race buffs from the DB), race congestion from the server | full body in the login costume, idle animation, camera of `reference/character_create/step1_*` |
| 2 appearance | hair style + colour, eyes (colour), style (eyebrow, makeup, deco + colours), skin, paint (tattoo, scar with location / size / angle), premade faces, premade eyes / nose / mouth / head, detail sliders per part, Blend Looks, Keep Appearance, Save / Load Appearance | head close-up (camera of `step2_*`), every change rebuilds the preview, mouse wheel zooms |
| 3 skillset + name | six skillsets with descriptions, skill icons and the pentagon diagram, name field with the client's own rules, "Finalize this character?" dialog, "Failed to create character." dialogs | combat stance in the training field (terrain, grass, rocks, straw dummies, sky; see below) seen by the camera of `step3_*` |

Create sends `Command::CreateCharacter` (race, gender, face + hair items, the full `ModelParams`
customization: slider bytes, decals, normal map, colours, hair and skin colour ids, skillset) and,
when the server answers `CharacterCreated`, goes to `Screen::CharacterSelect` (the session refreshes
the list by itself). `CharacterCreateFailed` reasons 3, 4, 5 (pending, duplicate, format) show the
client's own message boxes (`error_char_name_*`).

The shared original en_us name adapter now replaces the old18-character,
alphanumeric-only create fallback. The original field uses its native2-26 policy,
preserves accepted punctuation and applies native initial-uppercase/rest-lowercase
formatting through its own OnTextChanged handler. Metadata/validation/formatting
are shared with login deletion and HUD name dialogs. The server remains responsible
for availability and final acceptance; no local check reports creation success.
Exact executed-source golden cases and malformed UTF-8 limits are documented in
aa-client-hud.md under shared original English name rules.

## Flags

`--goto create`, `--race nuian|elf|ferre|hariharan`, `--gender male|female`, `--step 1|2|3`,
`--exit-after-create` and the scripted input below. `--create-offline` (with `--account`) opens the
screen as soon as the login is through and never enters a world: screenshots and tuning while the
account is busy in the game (the create request cannot be answered then). The login module accepts
`--goto create` silently (this screen drives itself there) and `--lua create:CODE`. The account is the login module's
(`--account NAME`, alias `--user`, `--login HOST:PORT`, `--password-file PATH`, see
`ingame::enter::Credentials`); this screen's runs use **`bevycreate`** (the server lets one
connection per account in).

Scripted input (all repeatable, `T` = seconds after the create UI is up, pointer coordinates are
physical pixels of the 1920x1080 window):

| Flag | Does |
|---|---|
| `--clua CODE T` | run Lua in the UI (`--lua` belongs to the login module) |
| `--hover X Y T`, `--click X Y T` | move / click the virtual pointer (`aa_ui::UiVirtualPointer`) |
| `--drag X1 Y1 X2 Y2 T` | press, move over 8 frames, release (sliders, the scar location pad) |
| `--wheel D T` | one mouse-wheel notch (+1 up) on the screen's window |
| `--type TEXT T` | type into the focused edit box |
| `--cshot FILE T` | screenshot (several per run; the first one freezes the idle pose) |

`AA_LOOKS_DIR` moves the Save Appearance folder (default `Documents/AABevy/looks`),
`AA_CREATE_NO_SUBJECT=1` leaves the stage empty (background shots for silhouette measuring),
`AA_CREATE_NO_FIELD=1` leaves the step 3 field out. Fitting knobs of the field: `AA_FIELD`,
`AA_BG_GRADE`, `AA_DUMMY_YAW` (see `scenery.rs`).

## How it fits together

* `mod.rs`: plugin, systems, flags. `OnEnter(Screen::CharacterCreate)` builds the stage and a
  `CreateScreen` resource; `build_ui` calls the login module's `host::switch_stage` (fresh Lua state)
  with `base::install` + the stage scripts. `--goto create` reuses the login screens: the same
  server-select click as `--goto character-select`, then `OnEnter(CharacterSelect)` redirects once.
* `look.rs`: `Catalog` (the DB lists in the client's order: hair by icon number, colours by comment,
  decals by name per category, skins by comment, premade faces and parts) and `Customizer` (`Look`
  state, `desc()` to `CharacterDesc`, premade presets, slider bytes, blend, `create_request`).
* `api.rs`: the `CC` table of Rust functions over `Arc<Mutex<Shared>>`; scripts mutate the look and
  queue `Request`s (step change, create, cancel, reload cosmetic) that `mod.rs` carries out.
* `game_base.lua`, `game_create.lua`, `base.rs`: `X2LoginCharacter`, `X2Customizer`, the customizing
  unit (`ModifyFaceParamValue`, `SetCustomizingParam`, ...), name policy, tooltips as thin Lua over `CC`.
  The login module has its own `gameapi.lua` for the other screens; each stage has its own Lua state,
  so the two never meet.
* `poses.rs`: the 24 `camera.json` of the captures (embedded) and where the figure stands per model
  and step (step 1 solved from feet / head pixels, step 2 and 3 corrected by registering our render
  onto the captures: `login/create/tools/{align3,bbox}.py`).
* `scenery.rs`: the step 3 training field (below).
* `preview.rs`: the preview cache (below). `saved.rs`: Save / Load Appearance files.

## The step 3 training field

It is not in the `game/create_<race>.lyr` layer files (those only hold lights and cameras): it is a
corner of the **login2** world, cell `000_001`. The capture cameras (`05_skill_<race>` of the cell's
`entities.xml`, `camera.json`) are in that cell's local coordinates and the terrain heights of the
cell match the solved feet z of all eight models. `scenery.rs` builds it from public pieces of
aa-world and aa-cry on the stage render layer, asking aa-world for nothing:

* terrain: `heightmap.dat` patches (`terrain::build_patch_mesh`, LOD by distance), the baked
  `cover.ctc` and the surface type layers through aa-world's `TerrainMaterial` shader, with a
  `LayerLibrary` built for login2 on a task (the in-game world's own library knows only main_world);
* grass, flowers and rocks: `vegetation.dat` + `vegetation.xml` and the cell's brushes
  (`load_cell_objects`) within 170 m of the camera, placed like aa-world places them;
* the `target_<race>_N` AnimObjects of `entities.xml` (quaternion `Rotate="w,x,y,z"`): three straw
  dummies per race. The model is a static bind pose with two arm logs and a painted shield that the
  game poses away: those parts are left out and the model is turned a quarter;
* the painted `step3_<race>` background is the sky: drawn with +0.92 EV and no tone curve (the
  captured sky is that much brighter than the texture, fitted to 3 / 255 on all four races), the
  field covers its lower half. Per race (`mood`): a fog colour for the far ground and linear factors
  on the terrain and on the grass / rocks (the stage lights are tuned for the figure and about eight
  times too bright for the captured ground). The values come from the mean colours of regions of the
  captures (sky, haze, ridge), within about 10 % on most regions;
* the grass and bushes (not the flowers) have a **tone curve** (`grass.rs`, below).

### The grass tone curve

The grass of the field is `common_plants_shine_fa03` (593 of the instances within 30 m of the camera; its material is
`common_plants_shine_fa01`: `Vegetation` shader, `waterweed_b.dds`, `Diffuse` 0.50 / 0.40 / 0.11, `BackDiffuse` x 0.72,
terrain blend 0.5), plus flowers and the other plants of the cell. With the stage's lights and only a factor per race it came out
dull and flat: the means of the near region matched the captures, but its dark quarter was 25 / 255 against 9 and its
sunlit tail 61 / 255 against 131. The quantile map of the near field of the captures against ours is, in every race, a
power of about 2 - 2.6 (Nuian 2.3, Elf 2.6, Hariharan 2.0, Ferre 2.2) of the radiance: the captured grass is lit by an HDR
sun (the `Vegetation` shader: front light `max(N.L, 0.5)` x sun, back light `BackDiffuse x BackDiffuseMultiplier x
LeafShadingBack` x the opacity map, terrain blend, vertex AO, all multiplied up by the film curve) while the stage lights
(key, fill and rim for the figure) give a narrow range. A brightness factor, ambient occlusion (a 1 x 1 occlusion map on
the terrain and the grass), diffuse transmission, normals bent to the sky and an unlit grass were tried first; none of
them widens the range (the darks stay lit by the stage's directional fill).

So the grass is a bindless `ExtendedMaterial` (`grass.rs` + `grass.wesl`, like the character back lights, aa-cry
`shade::backlight`) that applies `rgb' = rgb * (floor + (c / reference) ^ (gamma - 1))` to the lit colour before the fog,
with `c` the channels or, to keep the hue (the elf leaves went neon green), their luminance (`saturation` 0.3 for the Elf).
`mood` holds `[gamma, reference, floor, saturation]` and a colour factor per race, fitted on the luminance quantiles
(tools: a quantile refit from a capture run, inverting the curve to the pre-curve radiance) and the bright / dark tails:

| race | gamma | reference | floor | saturation | grass factor | terrain factor |
|---|---|---|---|---|---|---|
| Nuian | 3.87 | 0.0234 | 0.10 | 1 | 0.113 / 0.133 / 0.12 | 0.25 |
| Elf | 2.75 | 0.0304 | 0 | 0.3 | 0.18 / 0.18 / 0.25 | 0.35 |
| Hariharan | 2.77 | 0.0200 | 0.14 | 1 | 0.102 / 0.125 / 0.13 | 0.35 |
| Ferre | 5.25 | 0.0237 | 0.19 | 1 | 0.095 / 0.111 / 0.10 | 0.25 |

Measured on `step3_*` (male + female pooled, sRGB 0-255; regions `near` = 620..1250 x 830..1010, `mid` = 600..1250 x
710..810, which avoid the figure and the mist patch; real / before / after): mean colour, the brightest 15 % of the pixels
(the lit blades), the darkest 25 %, and the rms over 12 luminance quantiles (10 - 98 %) against the capture.

| race | region | mean R G B | brightest 15 % | darkest 25 % | quantile rms |
|---|---|---|---|---|---|
| Nuian | near | 40 39 1 / 42 42 0 / 43 43 0 | 132 108 0 / 61 72 0 / 137 114 0 | 8 11 1 / 25 23 1 / 5 6 0 | 25.9 -> 8.5 |
| Nuian | mid | 39 42 5 / 50 52 12 / 52 54 14 | 116 98 4 / 78 87 38 / 125 109 27 | 12 16 2 / 32 30 3 / 14 16 2 | 14.7 -> 14.1 |
| Elf | near | 53 55 3 / 49 51 1 / 55 55 1 | 146 139 15 / 80 79 1 / 143 139 7 | 9 12 0 / 28 32 1 / 12 11 0 | 36.3 -> 4.4 |
| Elf | mid | 89 93 34 / 58 60 11 / 69 72 17 | 157 157 41 / 98 101 42 / 150 161 38 | 50 56 21 / 32 33 2 / 15 17 1 | 38.6 -> 22.2 |
| Hariharan | near | 46 29 6 / 44 32 17 / 43 31 9 | 92 65 28 / 66 49 30 / 91 69 26 | 19 8 0 / 25 17 10 / 11 8 4 | 12.0 -> 2.4 |
| Hariharan | mid | 45 30 8 / 45 33 19 / 42 31 12 | 94 76 42 / 88 71 43 / 99 81 39 | 16 7 0 / 24 16 9 / 9 8 4 | 16.7 -> 13.6 |
| Ferre | near | 48 32 0 / 49 38 1 / 46 36 0 | 118 91 0 / 91 61 1 / 129 97 2 | 15 10 0 / 25 23 1 / 7 7 0 | 20.5 -> 8.5 |
| Ferre | mid | 66 52 14 / 60 51 13 / 57 50 15 | 136 123 51 / 122 111 65 / 137 126 55 | 26 15 0 / 27 25 2 / 9 9 0 | 12.2 -> 5.8 |

What is left: the Elf and Nuian mid region (real mist and haze lift its dark quarter to 50, ours stays at 15: `elf_fog_01`
is missing from the pak and the mist is aa-fx's), the near-left corner of Nuian (ours 1.5 x too bright), and the captures'
near blades are still broader and the real grass sparser with larger gaps. The sky, the horizon and the ridge bands are
unchanged to within 1 / 255 (the far ground band, rows 520-600, moves by at most 3 / 255 where the grass reaches it).

The files are read on a task when the screen opens, the models of the race are loaded ahead while
steps 1 and 2 are shown and the field is spawned (terrain at once, props within a 6 ms frame budget)
when step 3 starts; steps 2 <-> 3 cut instead of flying the camera (the real client changes place
there). Not drawn: the ground mist (`flow_fog_01`, `elf_fog_01`, ... particle effects), birds, wind
sway, the dummies' idle animation.

## Responsiveness

`spawn_character` rebuilds a whole character (no per-part API), so every look change respawns the
subject. `CharacterDesc::resolve` costs 50-70 ms (database, `.cdf` files); `PreviewCache` keeps the
structure (skeleton, parts, skin, hair material) and refills only what sliders, decals, colours and
the normal map change (`resolve` 0.2 ms, a 30-40 ms respawn in total). Rebuilds are at most every
80 ms while a slider moves, race / gender / hair changes rebuild at once (130-200 ms). The test
`preview::tests::resolve_matches_cache` (ignored, needs the data) keeps the cached path equal to a
full `resolve`.

## Verified

* `bevycreate` created `Bevylook1` (Nuian male, premade face, non-default hair style and colour, tattoo, scar, Archery) and
  `Bevylook2` (Elf female); the server log has `Registered character name Bevylook1 (5)` and the
  client log `the server created Bevylook1 (id 5, race 1, gender 1)`. `--enter-world --account
  bevycreate --character Bevylook1` showed the same face, hair, tattoo and scar in game; a second create
  with the same name gives the "name already in use" dialog (reason 4).
* All 24 step captures compared in `shots/create/cmp/` (ours) against `reference/character_create`;
  `shots/create/sheet{1,2,3}.png` put reference and ours side by side (`tools/sheet.py`). Step 3 now
  has the field: the sky, the dummies, the rocks and the ridge are where the captures have them.
* The Firran head fix (aa-anim `08ae0d4`) was re-measured: step 1 and 2 of both ferre models register
  onto the captures (scale 1.00, shift 0 with `tools/align3.py`), step 3 top / bottom rows of the
  silhouette are within 12 px of the captures (`tools/bbox.py` against a `AA_CREATE_NO_SUBJECT=1` run).

## Gaps and notes

* Step 3 field: no wind, no dummy animation; the grass is the vegetation of the cell within 170 m. The ground mist is aa-fx
  (`flow_fog_01`, `fog_01`, `leaf_01`; `elf_fog_01` is in no particle library of the pak). The
  Hariharan carcass is dark now and the flowers close to yellow (aa-cry `b538dc7`, `ef0bda6`: additive
  flag only for translucent materials, terrain colour blend of vegetation baked in).
* The ferre step 3 stance is more upright than the captures (their legs are bent more); the
  silhouette height and position match.
* Lighting / materials differ from the captures (flatter skin, hair specular: dark hair renders grey,
  no ambient occlusion); that is aa-cry / aa-anim.
* Name validation/formatting now uses the original English2-26 policy and native case maps,
  including accepted punctuation. Server reasons3/4/5 remain authoritative; the protocol has
  no separate availability check. The source and original-input checks are in aa-client-hud.md.
* Race / gender stay Nuian male on entering the screen (the real client does too), not the last pick.

## Authored facial integration (2026-10-17)

Character select/create `stage3d::spawn_subject` and the in-world local player's
`spawn_with_locomotion` now opt into aa-anim's verified facial overlays. Login and direct-entry
plugin paths each ensure FacialPlugin is present once. Matching CAL facial entries follow
the selected CAF's clock, including idle changes; customization rebuilding still uses the
same preview cache and original assembled character. Unproved head mappings remain neutral
with diagnostics. Exact authored baseline/topology proofs in aa-anim a299df2 now cover all eight
playable heads. Automatic voice lip sync, facial bone controllers and remote-unit opt-in remain
separate gaps; see aa-anim's compatibility evidence and its original XYZ-only morph-stream audit.

`cargo build -p aa-client --example client --example facial_client` passed. The inspected
`shots/hud/sol61-facial/client-paths.png` renders through both actual client spawn helpers:
stage open at0.7s and local-player blink at its authored1.10122s peak. The harness asserts both
resolve the original FSQ, remain on the body clock and deliver blink weight1 to GPU morphs.
It measured222-237FPS before capture,203FPS during capture, and exited normally. This is a
two-character isolated render with no network or streamed world, not a live gameplay claim.

The refreshed playable copy `target/debug/aa-client-play.exe` includes expansion and the
same-batch/deferred breath correction, SHA256
`15D057BD68E48B1FC25D659BFB33D0B1127C09A615F54EB86CA3F2CAB4FF0B3E`.
Its inspected `shots/hud/sol61-facial/world-select.png` shows the live Lane-bevy list;
334-423FPS, zero script errors, automatic exit. Game-session validation remains constrained
by the existing lane stall; no server process was restarted.

The next verified playable copy after inventory sorting/capacity work is SHA256
`1F9789094736416D34A5D866BB7F6132A871E4931FC755EABE084FA4EF5F4AD0`.
`shots/hud/sol61-sorting/create-ferre-final.png` was inspected on the original Ferre-female
step2 appearance UI, using login/world-list plus the existing `--create-offline` harness:
149-185FPS before capture,140FPS during capture, zero script errors. It submitted no create
request. The rebuilt actual stage/local-player facial fixture also retains authored body-clock
alignment and blink weight1 with the new native bone controller implementation. Its image
and exact shared-load FPS are recorded in aa-client-hud.md.
