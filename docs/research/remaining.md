# What is left: inventory of AABevy1.2

Date: 2026-10-18 (evening). Read-only stocktaking, no code changed. Sources: every `docs/*.md` gap section, `docs/research/*`,
`docs/aa-client-parity.md` (the completion ledger), `git log` (693 commits, 2026-10-16 to 2026-10-18), `git status`,
`data/extract/game/scriptsbin/x2ui`, `crates/aa-client/src/hud`, `crates/aa-ui/examples/fake`, the AAEmu lane source
(`D:\aaemu-wt\bevy`, its `rust/tools/opcode_coverage/sc_sent.json`), `reference/`.

How sure each claim is: "doc" = a doc says so and I did not re-check the code. "code" = I grepped the tree. "run" = I ran it
today. Two read-only sub-agent sweeps (server packets vs `aa-net`, x2ui windows vs HUD) produced the system lists; I spot-checked
the headline claims (auction, housing, duel, slave, tower defense, portal, cash shop have no `aa-client` consumer; `SCBuffUpdated`
is already decoded) but not every row. Sizes: S = under a day for one agent, M = a few days, L = a week or more.

## 1. Status summary

From a typed login (new `Screen::Login`, committed 18:27 today) through World Select (live lane list), Select Character (3D stage,
authored facial overlays, all eight playable heads), Create Character (all four 1.2 races, both genders, the three steps, server
create, name policy), Loading (world and HUD are built behind it) to the world, the client works end to end in the original Lua
UI, with the game on real data: streamed terrain, brushes, vegetation, roads, decals, water, sky, stars, moons, time of day, fog,
collision, walking / swimming / diving, remote players, NPCs, mounts as remote units, animated and equipped characters, combat
visuals, ambient audio and music, and a very large set of original HUD windows (bag, bank, equipment, merchants, mail, trade, party /
raid, friends, guilds / families, quests, map, chat, skills, buffs, crafting folio, achievements, ranking, trials, battlefield, pets,
enchant, repair, specialty quotes). Typical in-game speed is 70 to 95 fps at 1920x1080 on the shared machine, 38 fps in a big town
before the latest aa-fx fix. **The 2026-10-18 live block is gone:** on 2026-10-20 the client logged in, entered the world and
passed the headless smoke tests (titles, tooltip, Character Info, emotes, guild window) against the AAEmu server. Auction
settlement tail/reconnect and the final social unblock/offline/restart live checks remain unverified. By the parity ledger's
own words, "overall 1.2 parity is unfinished".

## 2. Remaining work items

Visibility = how much a player notices. IDs are for section 3 and 4.

### Blockers and network

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| N1 | network | Live-QA tail after the 2026-10-18 lane-server stall, which no longer reproduces: auction settlement tail/reconnect and the final social unblock/offline/restart checks are still unverified | 2026-10-20 headless smoke run (login, enter world, titles, tooltip, Character Info, emotes, guild window); `aa-client-parity.md` "Verification baseline" | med | S | none |
| N2 | network | Reconnect after a dropped game connection: session returns to idle | `aa-net.md` Known gaps; parity "reconnect policy" | med | S | none |
| N3 | network | About 90 server packets still `ServerMsg::Unhandled`; one that fails inside a `DD04` batch ends the whole batch (risk of losing later packets). Per-system list below (G*, H*) | `aa-net/src/game.rs` `decode_game`; sc_sent.json diff | med | M (spread) | per system |
| N4 | network | Outbound vehicle / ship / hanging / bound-doodad movement is read but not written | `aa-net.md` Known gaps | med | M | G2 |
| N5 | network | `SCLoginCharInfoHouse` (houses in char list), premium grade in lobby, Korean / Mail.Ru / Tencent logins | `aa-net.md`, parity row 1 | low | S | none |
| N6 | tooling | `aa-net.md` "Known gaps" is stale (says `SCBuffUpdated`, slave and mate packets are not decoded; they are) | `aa-net/src/game.rs` grep, commits 52bc34b, f14693e | low | S | none |

### Login, world select, character select, create, loading

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| L1 | login | Fields / buttons are drawn (no original `field.png` / `button*.png`); no text selection, IME, Ctrl+C; generic kick text off World Select | `aa-client-login.md` Not done | low | S | none |
| L2 | tooling / login | Client runs only on this box: game DB read from `D:/aaemu-live/rs-game/Data/compact.sqlite3` (the client's own copy is encrypted), pak at `D:\AA1.2`; log shows "6325 UI texts from D:/aaemu-live/..." | `aa-anim/src/db.rs:29`, `aa-ui/examples/fake/mod.rs:192`, log-sweep #14, run | med (for anyone else) | M | decrypt the client DB, or ship an export |
| W1 | world select | Nothing open besides N5 / N2 (matches captures, 53 of 53 widgets) | `aa-ui.md` table | low | S | none |
| C1 | char select | Skin: one calibrated colour per model instead of the HumanSkin SSS shader; fur drawn flat (ferre); hair spec is a baked strand map; flatter lighting than captures ("Char BackLight" lights missing) | `aa-anim.md` Gaps, `aa-cry.md` Known gaps, `aa-world.md` §6 | med | M | uncommitted calibration (section 4) |
| C2 | char select / create | Hat / beard variants: beard meshes absent in the pak for ferre and orc (only dwarf has one), no automatic voice lip sync, appearance-change rebuilding of remote / NPC units | `aa-anim.md` Gaps, `aa-client-units.md` | low | S to M | none |
| C3 | char select | Order of the hair / hair-colour grids and preset order not verified against the screen | `aa-anim.md` Gaps | low | S | a reference capture |
| CR1 | create | Step 3 field: no wind, no dummy animation; `elf_fog_01` is in no particle library of the pak (WARN logged each time) | `aa-client-create.md`, `aa-fx.md:362`, log-sweep #4 | low | S | substitute effect |
| CR2 | create / perf | Step 3 runs 34 to 76 fps; GPU bound, the stage camera re-filters the environment cube map every frame (`GeneratedEnvironmentMapLight`) | `research/perf.md` "Character create, step 3" | med | S | aa-cry `stage::environment_light` |
| CR3 | create | In-game re-customize / beauty salon (`EnterBeautyShop` etc. are stubs; server has `beauty_salon.rs`) | HUD sweep; opcodes 0x12c to 0x12e, 0x1f1, 0x1f3 | low | M | G* wiring |
| LD1 | loading | No persistent pipeline cache: frame 2 of a cold entry takes 0.1 to 2.8 s; `--enter-world` (test path) has no loading screen or prewarm; interruption / race coverage thin | parity row "Loading", `research/perf.md`, log-sweep #2 | med | S to M | none |

### World rendering

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| WR1 | world | No HLOD / impostors: brushes vanish beyond 1000 m, small vegetation 220 m, trees 600 m; units beyond 140 m not drawn | `aa-world.md` §6 last bullet, `aa-client-units.md` | high | L | perf (P1) |
| WR2 | world / cry | Illum `VERTCOLORS` (RGB squared after interpolation) used by Palace / castle brushes is dropped: `subset_mesh_with` omits `ATTRIBUTE_COLOR`; baked occlusion missing, dark canyon walls too bright | `aa-world.md` §7 "aa-cry, source-audited follow-up" | med | S to M | aa-cry then aa-world |
| WR3 | world | Post: sun shafts, glow buffer, eye adaptation, photofilter / blueshift, SSDO bounce (SSAO opt-in), cooler grade vs real | `aa-world.md` §6, `research/time-of-day.md` | med | M | none |
| WR4 | world | Water: sea foam / whitecaps / beach gradient, caustics, waterfalls, refraction from below, pond bed colour, only one pond calibrated, rivers dark | `aa-world.md` §6 | med | M | reference captures of lakes / sea |
| WR5 | world | Terrain: no `_sp` specular (ground fully rough), 4 missing diffuse maps, decal MRT normal / specular, voxel specular / atlases, portal and area lighting, procedural grass blades | `aa-world.md` §6, parity "World geometry" | low to med | M to L | none |
| WR6 | world | Vegetation bending / wind not applied; animated entities / FlowGraph / unsupported CGA clips; far clouds animation and calibration | `aa-cry.md`, parity | low | M | none |
| WR7 | world | Time of day: area / relic overrides, subject-qualification producer, 12:00 to 19:05 "noon plateau" never checked against the real client | `research/time-of-day.md`, `aa-world.md` §6 | med | S | real-client captures at several hours (`time set hour=H`) |
| WR8 | world | Sub-zone queries pass native tests but are not auto-registered live | parity "World geometry / Environment" | low | S | none |

### Characters, animation, units

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| CH1 | characters | Costume specular / silver armour duller than captures; clothing clipping (the `Cut` hulls leave straddling triangles); dye only for the bake path (weapons, bows, non-default armour dye, dynamic UCC) | `aa-anim.md` Gaps, `aa-cry.md`, parity | med | M | uncommitted calibration |
| CH2 | units | Mount / rider placement unverified, no dismount animation, no mount skills; humanoid NPCs lack combat idle; `SCUnitMovements` types read from the packet not the object | `aa-client-units.md` Gaps | med | M | G2 |
| CH3 | units | Remote units: full-world facial / attachment QA, CDF entity bindings, skinned render slots | parity "Character and animation rendering" | low | M | none |

### Combat and skills

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| CB1 | combat | Effect decals (31 skills), `GeometryInPieces` (2), per-effect lights, `EmissiveHDRDynamic`, `ForceGeneration`, `FxMaterial` flashes (24) | `aa-client-combat.md` "What is left" | med | M | aa-fx |
| CB2 | combat | Mounted / swimming / flying casters, doodad and ship cannon projectiles; cast cancel on mount-jump; poses for 3 channeling skills; two-hand grip, instruments' placement and sounds; remote camera shakes | `aa-client-combat.md` Not done | med | M | G2 |
| CB3 | combat | Cooldown / GCD: native Fired producers, custom GCD, full timeline lifetimes, lane cooldown emitter re-entry check | parity row "Skill progression" | med | M | N1 for live check |
| CB4 | combat | Force-mode (protected / team) eligibility; skill-by-skill reference verification (only 6 combat captures exist) | parity "Combat and HUD", `reference/ingame/combat_*` | med | L | more captures |

### HUD windows and systems on fake or no data

x2ui has about 70 groups. About 22 are wired, 9 partly, and 14 or more have no real data behind them (sub-agent count, loose). The HUD does not log
`FALLBACK` at run time (it appears only in comments): fakes load silently from `aa-ui/examples/fake/*.lua` (8 files) under
`hud/ingame.lua`. Bare stubs (callable, return nothing): `X2Auction`, `X2House`, `X2Coffer`, `X2Nation`, `X2InGameShop`, `X2Ucc`, `X2Warp`,
`X2Priest`, `X2Racing`, `X2Book`, `X2UserMusic`, `X2Security`, `X2SiegeWeapon`, `X2ItemLookConverter`, `X2MiaMail`, `X2GoodsMail`.

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| H1 | HUD | Auction house: protocol, escrow state and a live test exist in aa-net (`auction.rs`, 1273 lines); no HUD module, `X2Auction` is a stub; server is real | grep `aa-client`; `aa-net.md` | high | M | N1 to validate |
| H2 | HUD | Housing: whole `X2House` stub (build, place, decorate, permissions, tax, sale); aa-net decodes 4 packets of ~14; server `systems/housing` ~14k lines | HUD sweep; opcodes 0x0c1 to 0x0ca, 0x056 to 0x060 | high | L | aa-net + aa-client + aa-world placement |
| H3 | HUD | Territory / siege / dominion / nation: `X2Dominion` has 2 constant getters, `X2Nation` stub; guild web and siege controls | HUD sweep; `dominion.rs` unwired | med | L | none |
| H4 | HUD | In-game shop, premium service, cash points: `X2InGameShop` stub, `X2PremiumService` fixed getters; server has `cash_shop.rs` (AA-point wallet is a server gap) | HUD sweep; opcodes 0x1cf to 0x1d8 | med | M | server wallet |
| H5 | HUD | Portals / teleport / navi / return points: nothing wired, `SCCharacterPortals` etc. undecoded | opcodes 0x05b to 0x05d, 0x0b3, 0x115 | med | M | none |
| H6 | HUD | Appellations (titles): `GetAppellationCount` fake 0 in the character window; `SCAppellations` etc. undecoded and `ChangeAppellation` unsent | `fake_ingame.lua`; opcodes 0x1a4 to 0x1a6 | med | S to M | none |
| H7 | HUD | Crafting follow-ups: workbench input / range / permissions, gathering interactions, processing, trade-pack (specialty) sale flow and coin payout, `X2Store` zone-group getters and `BackCarryingOrders` still fake | parity rows "Crafting" and "NPC interactions", `aa-client-specialty.md` | high | M to L | G4 |
| H8 | HUD | NPC services: multi-service NPCs (quest and shop choice), live repair routing, ability services, priest buffs, coin / doodad shops, teleport | parity row "NPC interactions" | med | M | none |
| H9 | HUD | Map: quest objective areas, notifier icons, other-zone icons, climate / conflict overlays, live housing / public farm icons, ship markers | `aa-client-hud.md` Not done | med | M | H2, G2 |
| H10 | HUD | Tooltips: damage breakdown option, `ea_*` / `return_point`, buff caster stats on others, gems / tempers / enchants, repair cost | `aa-client-hud.md` "Tooltip numbers" | med | S to M | none |
| H11 | HUD | Item workflows: enchant protected planning / shared pending / continuation, dye, look converter (`X2ItemLookConverter`), temper, item secure / unsecure sends | `aa-client-enchant.md`, parity row "Inventory" | med | M | none |
| H12 | HUD | Options: graphics settings not applied, key-binding blob and character sync, remaining action consumers, layout persistence | parity row "Options" | med | M | aa-ui |
| H13 | HUD | Chat: send-size overflow, window position persistence, other-tab splitting, full command set, moderation. Emotes and bubbles are done (`CSExpressEmotion`, `SCEmotionExpressed`, `SCChatBubble`, `SCAreaChatBubble` decoded, say-chat bubbles; see `docs/aa-client-hud.md`, "Emotes and speech bubbles"); there is no `SCUserChatMessage` in the server's opcode tables or the C# project | parity row "Chat"; opcodes 0x12b, 0x153, 0x154 | med | S to M | none |
| H14 | HUD | Overhead 3D glyphs: raid markers, family / guild titles; party buff / combat indicators | `aa-client-hud.md`, `aa-ui.md` "Native 3D overhead marker audit" | med | S to M | aa-ui |
| H15 | HUD | Music: `X2UserMusic` stub (composition, percussion windows), bard instruments; `SCSendUserMusic`, `SCUserNoteLoaded` undecoded | HUD sweep; opcodes 0x1df to 0x1e1 | low | L | G* |
| H16 | HUD | UCC / crest / books (`X2Ucc`, `X2Book`), common / public farm, racing, second password, web browser, tutorial (one canned page) | HUD sweep | low | M | server (racing, second password have no server types) |
| H17 | HUD | Social live settlement (paired guild, family, mail, trade, raid loot, friends unblock / restart) never re-run after the lane stalled; 3D titles; nation | parity rows "Mail", "Social", "Party" | med | M | N1 |
| H18 | HUD | Ability change / reset costs fake (`X2Ability`), `X2Player` pay / cash / schedule getters fake | `fake_ingame.lua:790-806` | low | S | none |
| H19 | tooling | No run-time report of which stubs / fakes the scripts hit (`ui.stub_calls()` only printed by the fake example) | HUD sweep: `aa-ui/src/lib.rs:192` | low | S | none |

### Gameplay systems (server supports, client does not)

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| G1 | gameplay | Duels: server real (`duel.rs`); client has no packets or UI (`SCDuel*` 0x08b to 0x08f, `ChallengeDuel` 0x050) | server grep; aa-net grep | med | S to M | none |
| G2 | gameplay | Transport: local owned mounts and mounting UI, summon / dismiss, ships (steering, boarding, cannons), vehicles, gliders, hanging, flying state (`SCSkillControllerState`, `SCUnitFlyingStateChanged`, `SCShipyardState`, `SCHung`, `SCBondDoodad` undecoded); slave equipment handlers are log-only stubs on the server | `aa-client-units.md`, parity "Pets, slaves", opcodes 0x06d, 0x07a, 0x137 to 0x13a, 0x1aa | high | L | N4 |
| G3 | gameplay | Doodad interaction: gathering, farming, coffers, switches (`ChangeDoodadPhase`, `SelectInteractionEx`, `SCCofferContentsUpdate`, `SCVegetationCutdowning` unhandled) | opcodes 0x096, 0x081 | high | M | none |
| G4 | gameplay | Click-to-move, camera preset actions, hanging / moving supports, less common locomotion | parity row "Ground movement" | med | M | none |
| G5 | gameplay | Tower defense (0x16b to 0x16e), siege alerts (0x0ec to 0x0ef), crime arrest (0x170), appearance gender change (0x1f3) | opcodes | low | M | H3 |
| G6 | gameplay | Cinema / quest camera: hidden cinematic actors are "aa-client's screen-flow responsibility"; `X2Quest.UseQuestCamera` false | `aa-world.md` §6 | low | M | none |
| G7 | gameplay | Misc small: dice (0x134), impulse / knockback visuals (0x079), idle status, visual options, PvP point change (0x204), `SCPlaySound` (0x1de), item look convert, lobby refresh (0x1cd) | opcodes | low | S each | none |

### Audio, performance, tooling

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| A1 | audio | Voice-over gaps: `x2voice_prologue` has no FEV (client-driven indun voices), 59 absent events and 41 absent FEVs play nothing; one CELT subsound | `research/audio.md` §5.4, `aa-audio.md` | low | S (CELT L) | none |
| A2 | audio | FEV features read but not played (pitch / low-pass envelopes, doppler, cones, spawn templates, oscillator), no ducking, no `is_playing` / music-ended query, ambience loops decoded whole; moods beyond 5 and live zone / combat crossfade not walked | `aa-audio.md`, `aa-client-sound.md` | low to med | M | none |
| P1 | perf | Town fps: Marianople 38 fps (40k entities, 19k mesh entities, 836 mesh assets rewritten per frame by aa-fx, `allocate_and_free_meshes` 8 ms). One fix landed (77d3f01, invisible emitters skip rewrites); no re-measurement. Frame is render-thread bound, GPU 3 to 4 ms idle | `research/perf.md` (draft), commit 77d3f01 | high | M | aa-fx, aa-world |
| P2 | perf | Entity / draw-call count: merge static part entities, cull earlier; `CryAssets::load_model` is main-thread only (prepare / finish split requested) | `research/perf.md`, `aa-world.md` §7 | med | L | aa-cry |
| P3 | perf | 3 to 10 fps stalls when other processes load the CPU (cause found: scheduler hand-offs; above-normal priority class added in `priority.rs`). Not re-verified on a quiet machine; no memory / VRAM budget measured anywhere | `research/perf.md` | med | S | none |
| T1 | tooling | Reference gaps: no `ingame/worldmap_*` check in docs, `combat_kick`, `combat_death`, tooltips, Palace each mentioned once; no captures at all for skill book, quest log, mail, party, crafting, friends, mounts, ships, housing, weather, night. Real client (`archeage.exe` pid 48924) is up | `reference/`, grep of docs | med | S per set | N1 (lane) |
| T2 | tooling | Docs are 2.6 MB (`aa-net.md` 714 KB, `aa-ui.md` 464 KB, `aa-world.md` 420 KB) and gap lists are stale in places; no single status page besides the parity ledger | `wc -c docs` | low | S | none |

## 3. Suggested order and parallel groups

Round 0, before anything: **N1** (diagnose / restart the lane server; the server owner or the orchestrator, not a code agent) and
decide the uncommitted work in section 4 (item U1 first: HEAD may not build). Everything in "live validation" waits on it.

Round 1, all groups independent. "Owns" = the files only that agent edits. Shared choke points (listed per group) are the only
conflict risk, so register them through one integrator or in separate small commits.

| Group | Items | Owns | Shared files to watch |
|---|---|---|---|
| A World look | WR2 (second half), WR3, WR4, WR7, WR5, then WR1 | `crates/aa-world/**`, `docs/aa-world.md` | none |
| B Characters and materials | WR2 (aa-cry half: vertex colours), C1, CH1, C2, CR2, decide U2 calibration | `crates/aa-cry/**`, `crates/aa-anim/**` | aa-cry change must land before A's WR2 |
| C Effects and combat | CB1, CB2, CB4, CR1, P1 (aa-fx side) | `crates/aa-fx/**`, `crates/aa-client/src/ingame/combat*` | `ingame/combat.rs` has live uncommitted edits (U5) |
| D Auction and small systems | H1, G1, H6, H13 emotes and bubbles, H5 | new `aa-net/src/<feature>.rs` and `aa-client/src/hud/<feature>/` per feature | `aa-net/src/game.rs` (`decode_game`), `opcodes.rs`, `session.rs` Command / Event, `aa-client/src/hud/mod.rs`, `hud/api.rs`, `hud/ingame.lua` |
| E Housing | H2 (then H9 icons) | `aa-net/src/housing.rs`, new `aa-client/src/hud/housing/`, a placement module in aa-world only by agreement | same shared files as D |
| F Transport | G2, N4, CH2, CB2 mounted part, H15 later | `aa-client/src/ingame/mount.rs`, `hud/pets/**`, `aa-net` slave / mate / movement | `aa-net/src/movement` (writer), D's shared files |
| G Crafting, gathering, NPC services | G3, H7, H8, H11, H10 | `aa-client/src/hud/{crafting,specialty,store,repair,enchant}*` | D's shared files |
| H Client perf and loading | P1, P2, P3, LD1, WR1 spike (design only) | `crates/aa-client/src/{ingame/{prewarm,flow,enter}.rs,login/loading.rs,priority.rs}`, `docs/research/perf.md` (finish it) | A and C for the render side |
| I UI toolkit and options | H12, H14, H19, L1 | `crates/aa-ui/**`, `crates/aa-client/src/{settings*,login/login_screen*}` | none |
| J Audio | A1, A2 | `crates/aa-audio/**`, `crates/aa-client/src/sound/**` | none |
| K Portability and docs | L2, N6, T2, T1 (capture sets once N1 is fixed) | `tools/`, `docs/`, `reference/` | none |

Round 2 (after N1 is fixed): live validation of everything marked "offline / fixture" (H17, CB3, H1, mail, trade, guilds), then
H3 / G5 (territory, siege, tower defense, nation), H4 (needs the server's AA-point wallet), H16, G6.

## 4. Uncommitted or abandoned work

`git status` at 19:16 shows 20 modified files and 2 untracked. At least two sessions look active right now (files touched within two hours: U1, U5, U6).

| ID | Files | Age | State | Decision needed |
|---|---|---|---|---|
| U1 | `aa-client/src/hud/data.rs` (`specialties: Tables`, `Tables::load`), `hud/enchant.rs` (`pub(super) fn equipment`), `hud/enchant/selection.rs` (`equipment()`) | 17:45 today | **Committed HEAD depends on them**: `hud/specialty/quote.rs` (commit d612b06, 18:03) uses `game.specialties` and `enchant::equipment`, which exist only in the working tree (I did not run a build to confirm). Looks like the missed tail of the specialty-quote commit | commit now (specialty owner) |
| U2 | `aa-anim/src/character.rs` (skin_tune retune for 8 models, elf-female tint), `aa-anim/examples/compare.rs` (`--ssao`, `--key-from`, `--shadows`), `aa-cry/src/stage.rs` (`StageShadows`, stage-frame doc fix: front is -Z), `aa-cry/src/shade.rs` (costume gloss by `ReflectAmount`) | 10-17 13:46 to 10-18 01:12 | One character-lighting calibration session, stale 18 to 29 hours, half merged: no commit, no doc entry. Visual effect on char select / create unverified | owner of aa-anim / aa-cry: finish (re-shoot against `reference/character_create`) and commit, or `git checkout` |
| U3 | `aa-world/src/post.wesl` (NaN / negative guard for refraction), `sky.rs` (`ambient_rel`, `water_lod` tuning), `water.wesl`, `water_material.rs` (cube map mip level), `examples/fly.rs` (`--pos x,y`) | post / fly 10-17 15:41; sky 10-17 23:10; water 10-18 15:37 to 15:56 | Parity doc calls them "inherited calibration / performance edits ... not counted until validated". The NaN guard is a pure bug fix; `sky.rs` references `TerrainQuery` and is stale | aa-world owner: commit the NaN guard and `fly.rs` now; validate or drop the water / sky tuning |
| U4 | `aa-audio/src/reverb.rs` | 10-17 16:10 | Adds one `#[ignore]` cost test only; harmless | commit |
| U5 | `aa-dev/src/{lib.rs,record.rs (untracked)}`, `aa-dev/Cargo.toml`, `Cargo.lock`, `aa-fx/examples/effects.rs` (`--list` contact sheet), `aa-client/src/ingame/combat.rs` (`AA_COMBAT_FAKE_TARGET`, `--record-gate`) | 19:12 to 19:15 today | Active: a frame-recorder (`--record`) and combat video tooling being written right now. `aa-dev` is shared by every example, so a broken edit blocks everyone | leave; ask the owner to commit `aa-dev` first and in one go with `Cargo.lock` |
| U6 | `aa-net/src/state/stats.rs` (labor wraps like the native signed add, new test), `docs/aa-net.md` (3 sections on labor delta, specialty affordability) | 18:06 to 18:08 today | Active, small and self-contained | aa-net owner: commit |
| U7 | `docs/research/perf.md` (untracked) | 10-17 15:51 | Abandoned draft: still contains the literal placeholders `RESULTS_TABLE`, `AA_FX_SECTION`, `OTHER_CRATES` and no "Fixed in aa-fx" content. The numbers above the placeholders are good and I used them | finish (group H) or commit as a draft with the placeholders removed |
| U8 | `target/agents/*` (perf, sol61, reel, ...), `target/debug/aa-client-2026*.exe` (140 MB each) | n/a | Not in git; many immutable builds of the client pile up on a disk the AGENTS file calls limited | prune by the owners |

Not abandoned but worth a look: the `.bashrc` of this machine begins with a UTF-16 BOM and prints an error on every Bash call
(`$'\377\376export': command not found`); harmless but noisy for every agent.
