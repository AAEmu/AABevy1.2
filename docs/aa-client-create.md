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
| 3 skillset + name | six skillsets with descriptions, skill icons and the pentagon diagram, name field with the client's own rules, "Finalize this character?" dialog, "Failed to create character." dialogs | combat stance in front of the camera of `step3_*` (no training field, see gaps) |

Create sends `Command::CreateCharacter` (race, gender, face + hair items, the full `ModelParams`
customization: slider bytes, decals, normal map, colours, hair and skin colour ids, skillset) and,
when the server answers `CharacterCreated`, goes to `Screen::CharacterSelect` (the session refreshes
the list by itself). `CharacterCreateFailed` reasons 3, 4, 5 (pending, duplicate, format) show the
client's own message boxes (`error_char_name_*`).

## Flags

`--goto create`, `--race nuian|elf|ferre|hariharan`, `--gender male|female`, `--step 1|2|3`,
`--exit-after-create` and the scripted input below. The account is the login module's
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
`AA_CREATE_NO_SUBJECT=1` leaves the stage empty (background shots for silhouette measuring).

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
* `preview.rs`: the preview cache (below). `saved.rs`: Save / Load Appearance files.

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
* All 24 step captures compared in `shots/create/cmp/` (ours) against `reference/character_create`.

## Gaps and notes

* Step 3 has no training field (grass, dummies, sky): the login level 3D is not in aa-world, only the
  painted UI background is behind the figure.
* Lighting / materials differ from the captures (flatter skin, hair specular: dark hair renders grey,
  no ambient occlusion); that is aa-cry / aa-anim.
* Firran (ferre) heads: the head placement bug is being fixed in aa-anim; the step 2 / 3 placement of
  the two ferre models was measured with it present and needs a re-measure (`login/create/tools/`).
* Name validation is the client's (2-18 letters and digits in this build, server reasons 3, 4, 5 for
  the rest); the protocol has no separate name check.
* Race / gender stay Nuian male on entering the screen (the real client does too), not the last pick.
