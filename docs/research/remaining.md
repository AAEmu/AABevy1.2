# What is left: inventory of AABevy1.2

Original inventory: 2026-10-18 (evening). Selected rows were corrected on 2026-10-23 after source and runtime checks;
other rows remain historical and need a current-code audit before assigning work. Sources: every `docs/*.md` gap section, `docs/research/*`,
`docs/aa-client-parity.md` (the completion ledger), `git log` (693 commits, 2026-10-16 to 2026-10-18), `git status`,
`data/extract/game/scriptsbin/x2ui`, `crates/aa-client/src/hud`, `crates/aa-ui/examples/fake`, the AAEmu lane source
(`<aaemu-src>`, its `rust/tools/opcode_coverage/sc_sent.json`), `reference/`.

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
| N4 | network | Outbound movement: cart type 2, ship type 4 and deck type 1 with the support block are written; hanging / bound-doodad movement is still read but not written | `aa-net.md` Known gaps; `tools/evidence/cart_*`, `ship_steering`, `ship_deck` | med | M | G2 |
| N5 | network | `SCLoginCharInfoHouse` (houses in char list), premium grade in lobby, Korean / Mail.Ru / Tencent logins | `aa-net.md`, parity row 1 | low | S | none |
| N6 | tooling | `aa-net.md` "Known gaps" is stale (says `SCBuffUpdated`, slave and mate packets are not decoded; they are) | `aa-net/src/game.rs` grep, commits 52bc34b, f14693e | low | S | none |

### Login, world select, character select, create, loading

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| L1 | login | Fields / buttons are drawn (no original `field.png` / `button*.png`); no text selection, IME, Ctrl+C; generic kick text off World Select | `aa-client-login.md` Not done | low | S | none |
| L2 | tooling / login | Client runs only on this box: game DB read from `<server>/Data/compact.sqlite3` (the client's own copy is encrypted), pak at `<client>`; log shows "6325 UI texts from <server>/..." | `aa-anim/src/db.rs:29`, `aa-ui/examples/fake/mod.rs:192`, log-sweep #14, run | med (for anyone else) | M | decrypt the client DB, or ship an export |
| W1 | world select | Nothing open besides N5 / N2 (matches captures, 53 of 53 widgets) | `aa-ui.md` table | low | S | none |
| C1 | char select | Skin: one calibrated colour per model instead of the HumanSkin SSS shader; fur drawn flat (ferre); hair spec is a baked strand map; flatter lighting than captures ("Char BackLight" lights missing) | `aa-anim.md` Gaps, `aa-cry.md` Known gaps, `aa-world.md` §6 | med | M | uncommitted calibration (section 4) |
| C2 | char select / create | Hat / beard variants: beard meshes absent in the pak for ferre and orc (only dwarf has one), no automatic voice lip sync, appearance-change rebuilding of remote / NPC units | `aa-anim.md` Gaps, `aa-client-units.md` | low | S to M | none |
| C3 | char select | Order of the hair / hair-colour grids and preset order not verified against the screen | `aa-anim.md` Gaps | low | S | a reference capture |
| CR1 | create | Step 3 field: no wind, no dummy animation; `elf_fog_01` is in no particle library of the pak (WARN logged each time) | `aa-client-create.md`, `aa-fx.md:362`, log-sweep #4 | low | S | substitute effect |
| CR2 | create / perf | Step 3 runs 34 to 76 fps; GPU bound, the stage camera re-filters the environment cube map every frame (`GeneratedEnvironmentMapLight`) | `research/perf.md` "Character create, step 3" | med | S | aa-cry `stage::environment_light` |
| CR3 | create | Salon original UI, preview, paid apply/re-entry and physical payment pass on an isolated server with PR42; data round-trips all eight models. Existing bevy still needs rollout. Remote appearance, eight-model live coverage and camera/lighting parity remain open | 2026-10-23 `aa-client-salon.md`, inspected `shots/resume/salon-qa`, two 18-command apply checks and seven server regressions | low | M | bevy rollout and remaining visual/live checks |
| LD1 | loading | No persistent pipeline cache: frame 2 of a cold entry takes 0.1 to 2.8 s; `--enter-world` (test path) has no loading screen or prewarm; interruption / race coverage thin | parity row "Loading", `research/perf.md`, log-sweep #2 | med | S to M | none |

### World rendering

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| WR1 | world | Far-plane dissolve, mesh/sprite handoff and shared quads recovered. Olive patches fixed by retaining unsupported normal-mapped leaf-only meshes; 23 models retain sprites. Native brush mesh-pair distance dissolve/reversal/FOV guards recovered; the 0.3-second cvar has registration-only references. Sprite-lighting parity, broader profiling and unit distance policy remain open. | b2f2499/2bf76a0/54309c7; 272 tests, 348160 GPU pixels with zero coverage/mask mismatches; `aa-world.md` | med | L | native sprite atlas and perf (P1) |
| WR2 | world / cry | Illum preserves source vertex RGB and squares it after interpolation/composition. Forward punctual highlights and exact authored light specular ratios pass bindless/fallback/churn controls. Terrain forward per-layer gloss/normal/Phong is recovered, adding about 66 MB of arrays. Ordinary VERTALPHA/ALPHAGLOW main and masked prepasses are recovered for 169 submaterials (15 masks); 277 permutations are explicitly declined. Open: excluded alpha permutations, native opaque deferred/tiled shading, native shadows, punctual terrain specular and integrated profiling. | 56f179a/ab918ae/b690462/3498eeb/328d6a4; terrain 275 tests and 48 GPU controls; Illum punctual 94 checks and 6240 GPU samples max0.575 sRGB step; alpha 108 checks, 144 native cases, 3360 GPU samples and client nested-prepass controls; `aa-cry.md`, `aa-world.md` | med | M | native pass follow-up |
| WR3 | world | Native HDR sun-shaft controls, depth mask, three blur passes and post-film blend are recovered; levels/photofilter/colour matrix and eye adaptation follow the engine, blueshift is unused. Camera-far bridge, matched native forest capture, glow buffer/SSDO bounce and broader performance remain open | 2026-10-23 6855713; `aa-world.md`, 20 native probes, six GPU controls, inspected matched release pictures and 595 integrated client tests; shared-load FPS is not isolated | med | M | camera-far/reference capture |
| WR4 | world | Waterfall type-2 UV motion and anchored opacity recovered (53910e9; 98 checks, 36 native matrices, 728 GPU samples). Authored WaterSurface underside and plain volume Outof recovered (227d2f3; 136 pixel controls, 5 crossing states). Source-qualified ocean underside refraction/Fresnel and cone-free post are also recovered (896a287;179 GPU controls and11 image identities). Open: waterfall DualMaterial/Custom/VertexDeform, exact sea colour/scatter/reflection and accepted-receiver foam depth, native near-clip/fog ordering, pond calibration/river darkness and FFT sea displacement. Qualified sea foam scalar lighting and camera fade pass native/GPU controls on both sides (b64f71e). Native water tessellation/crest support and lake/river caustics still require audit; the renderer DLL name does not establish capability | `aa-world.md` §6 | med | M | reference captures of lakes / sea |
| WR5 | world | Terrain forward per-layer specular is recovered; native deferred/punctual terrain lighting, 4 missing diffuse maps, decal MRT normal / specular, voxel specular / atlases, portal and area lighting, procedural grass blades | `aa-world.md` §6, parity "World geometry" | low to med | M to L | none |
| WR6 | world | Qualified steady mesh LOD1/2 bending recovered (2026-10-25, c356ec80; 290 world checks and 24 GPU controls). Remaining: WindArea/dynamic reception and native cache/history/timer, compound/complex/sprite wind, distant suppression/sliding-frame precision; animated entities / FlowGraph / unsupported CGA clips; far clouds animation and calibration | `aa-world.md`, `aa-cry.md`, parity | low | M | native wind cache/subject contracts |
| WR7 | world | Time of day: area / relic overrides, subject-qualification producer, 12:00 to 19:05 "noon plateau" never checked against the real client | `research/time-of-day.md`, `aa-world.md` §6 | med | S | real-client captures at several hours (`time set hour=H`) |
| WR8 | world | Sub-zone queries pass native tests but are not auto-registered live | parity "World geometry / Environment" | low | S | none |

### Characters, animation, units

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| CH1 | characters | Costume specular / silver armour duller than captures; clothing clipping (the `Cut` hulls leave straddling triangles); dye only for the bake path (weapons, bows, non-default armour dye, dynamic UCC) | `aa-anim.md` Gaps, `aa-cry.md`, parity | med | M | uncommitted calibration |
| CH2 | units | Mount / rider placement, dismount animation and mount skills are done in #31-#38; still open: humanoid NPCs lack combat idle, `SCUnitMovements` types are read from the packet not the object | `aa-client-units.md` Gaps; `tools/evidence/mount_*` | med | M | G2 |
| CH3 | units | Remote units: full-world facial / attachment QA, CDF entity bindings, skinned render slots | parity "Character and animation rendering" | low | M | none |

### Combat and skills

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| CB1 | combat | Sun lighting/live HDR/pool cleanup and bounded adaptive ground drapes are committed and checked; client FxMaterial hit flashes are integrated. Bounded direct constant-clock Target ancestors are implemented (d34e9e4;65 tests). Native GeometryInPieces is implemented for the four authored flat compound root-node models (0cb85b4;72 tests), with ordered safety-budget truncation. Native receiver/depth projection, nested/skinned/per-piece-material compounds, Gravity areas, unsupported Wind producer shapes/clocks, indirect Target clocks and integrated per-skill comparisons remain open. Bounded constant spherical Wind producers and other-root FX reception are implemented (71b8ff6;82 unit tests); the client opts in at cap64. Vegetation reception remains held on native recurring cache invalidation (R2). Stretched beams do not establish ForceGeneration parity | 2026-10-23 d0b98da/cc9fc34; `aa-fx.md` rounds17/18, 59 tests plus all-target check and inspected captures; curved geometry has measured cost | med | M | native particle gaps and integrated comparisons |
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
| H7 | HUD | Crafting follow-ups: workbench input / range / permissions, natural gathering (herb and fake-use verified on 1.2 2026-10-30; legacy ore/rock/shear and item delivery into free bag space capture-pending), processing, trade-pack (specialty) sale flow and coin payout, `X2Store` zone-group getters and `BackCarryingOrders` still fake | parity rows "Crafting" and "NPC interactions", `aa-client-specialty.md`, `tools/evidence/gather/capture-results.txt` | high | M to L | G4 |
| H8 | HUD | NPC services: multi-service NPCs (quest and shop choice), live repair routing, ability services, priest buffs, coin / doodad shops, teleport | parity row "NPC interactions" | med | M | none |
| H9 | HUD | Map: quest objective areas, notifier icons (other-zone service icons and climate / conflict overlays done 2026-10-27), live housing icons, ship markers (stations, graveyards, public farm managers done 2026-10-27) | `aa-client-hud.md` Not done | med | M | H2, G2 |
| H10 | HUD | Tooltips: damage breakdown option, `ea_*` / `return_point`, buff caster stats on others, gems / tempers / enchants, repair cost | `aa-client-hud.md` "Tooltip numbers" | med | S to M | none |
| H11 | HUD | Item workflows: enchant protected planning / shared pending / continuation, dye, look converter (`X2ItemLookConverter`), temper, item secure / unsecure sends | `aa-client-enchant.md`, parity row "Inventory" | med | M | none |
| H12 | HUD | Options: texture / terrain detail / character / water quality and DoF have no renderer knob, loaded object-distance refresh is implemented (d26c359; 278 checks), native global far clipping remains open, key-binding blob and character sync, remaining action consumers, layout persistence | parity row "Options" | med | M | aa-ui |
| H13 | HUD | Native move/anchor/bound callbacks, covered resize borders, secondary-tab split/whole-window transfer and character-scoped metadata persistence recovered. Caret/slide preview, drag draw preparation and release-time reorder now pass 271 toolkit checks; hover does not mutate native rows. Actual drag/history/filter/reload and allocation-write failure checks pass. Open: broader class/geometry/hover/held-item/link paths and remaining send-size/commands/moderation audit. SetTabButtonAlpha raw storage/inactive native draw behavior is recovered (4b68eb9;285 checks). Emotes/bubbles remain implemented. | f51b3fb/389b6d1/9b3ae0c/f6f0929/f93a96c/6fed35a/9a618ee; `aa-ui.md`, `aa-client-hud.md` | med | S to M | native UI follow-up |
| H14 | HUD | Overhead 3D glyphs: raid markers, family / guild titles; party buff / combat indicators | `aa-client-hud.md`, `aa-ui.md` "Native 3D overhead marker audit" | med | S to M | aa-ui |
| H15 | HUD | Native composition/cache/tooltips, receipt-ordered Started93/94/Fired97, tracked PCM and own natural End are integrated. Original buff mode bar/input modes pass9 checks;29 music/device checks pass. Native task detail8 now supports fresh sheets. Actual windowed Bevy fresh-sheet compose/two plays/Stop/Close passes on an isolated lane with one explicit, proven original-script Lua allowance. Normal curtain-entry and delayed captures show textured actors/HUD; direct-entry readiness remains bounded. Native-derived helper seating/base CAL pose and guarded local unbond are integrated with offline checks. Live seat/ACK coverage, Blink local rotation, parented impulse physics, exact graph/camera behavior, wider mode producers, exact RNG/FMOD/output and existing-bevy rollout remain open. | 5e92f2c/6667ef5/613c9a4; server draftPR45; `aa-client-music.md` | low | M | live render integration and seat presentation |
| H16 | HUD | UCC / crest / books (`X2Ucc`, `X2Book`), common / public farm, racing, second password, web browser, tutorial (one canned page) | HUD sweep | low | M | server (racing, second password have no server types) |
| H17 | HUD | Social live settlement (paired guild, family, mail, trade, raid loot, friends unblock / restart) never re-run after the lane stalled; 3D titles; nation | parity rows "Mail", "Social", "Party" | med | M | N1 |
| H18 | HUD | Ability change / reset costs fake (`X2Ability`), `X2Player` pay / cash / schedule getters fake | `fake_ingame.lua:790-806` | low | S | none |
| H19 | tooling | No run-time report of which stubs / fakes the scripts hit (`ui.stub_calls()` only printed by the fake example) | HUD sweep: `aa-ui/src/lib.rs:192` | low | S | none |

### Gameplay systems (server supports, client does not)

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| G1 | gameplay | Duels: server real (`duel.rs`); client has no packets or UI (`SCDuel*` 0x08b to 0x08f, `ChallengeDuel` 0x050) | server grep; aa-net grep | med | S to M | none |
| G2 | gameplay | Transport: mounts / mounting UI (the walk into 3 m before `CSMountMate` done 2026-10-30: confirmed at 7.642 m and a second run at ~15 m walked and mounted; the ride-cooldown timing and the walk's new-target, 25 m drift and teleport cancels are capture-pending), summon / dismiss, cart boarding / driving, rendering and dismount are done in #31-#38; ship steering at the helm and rowboat deck standing in #39/#40; passenger seats (S11: 13863 `CSStartSkill`, row-less vehicle seats stand, remote `$passengerN` followers, leave-then-board switching) landed with `examples/passenger_seats`; still open: larger ships, cannons, gliders, hanging, flying state (`SCSkillControllerState`, `SCUnitFlyingStateChanged`, `SCShipyardState`, `SCHung`, `SCBondDoodad` undecoded), remote vehicle smoothing done in S8 (full type-2/3 quaternion, 1.2 delayed sample queue; the delay and the snap/zone forms are capture-pending S8-C1, S8-C2, S8-C3 and S8-C5), S11-C1..C5 (row-less poses, seat-taken feedback, helper spelling, remote capture, mate seat) and the remaining undecoded opcodes; slave equipment handlers are log-only stubs on the server. Ship/vehicle summon items now run the 1.2 40 x 3 space search, show error 100 instead of sending, and send the chosen spot as the `CSStartSkill` Position target with the ship yaw (57-byte group-A2 capture; `hud/summon.rs`, `ground_target::route`); left: the native cancel range, the mates' 108 path, and verifying the static-only collision / rotated box centre against a capture | `tools/evidence/mount_*`, `cart_*`, `ship_steering`, `ship_deck`; `aa-client-units.md` "Ship and vehicle summon items", parity "Pets, slaves", opcodes 0x06d, 0x07a, 0x137 to 0x13a, 0x1aa | high | L | N4 |
| G3 | gameplay | Doodad interaction: natural gathering offers the func's own `Use`/`FakeUse` skill and the server's legacy OreMine/RockMine/Shear mapping, with the ordinary cast/tool/anim/labor/loot/phase path; use-target highlight/decal/silhouette mapping, walk-to, the name/owner/growth plate and `SCDoodadPhaseMsg` -> `DOODAD_PHASE_MSG` are implemented (`aa-client-doodads.md`). Verified by the 2026-10-30 1.2 capture: herb 2274/13990 (F, "Gather", 4000 ms, 1 LP, 4513->4514), the fake-use fallback (lamp 13166), the local "Insufficient Labor Points." refusal, the reach (`max_range` with no slack: on at 3 m, off at 4.5 m) and a cast not gated by a full bag. Still capture-pending: the legacy 11106/11107/11642 adapter (no 1.2 world spawns; GM spawns are not interactive), the clover/iron vein/bell/fire copies that offered nothing, the `DoodadFuncUse.skill_id` fallback, gathered-item delivery into free bag space (every captured herb gather had a full bag: `BAG_FULL`, no item task), ring size/tint/projection (the capture stood outside reach), highlight colour, plate text (no tooltip text), doodad right-click walk-to (not run) and 0x157 (no captured doodad sent it). Not on the bar yet: the Mailbox's 11400 "Open Mailbox". Tree cutdown 0x081 is decoded and the fall/phase deferral implemented (`aa-client-doodads.md` "Cutting down trees"); the 2026-10-30 capture confirmed the 4000 ms skill 15874 cast, `SCDoodadPhaseChanged` 6251 before the 6-byte cutdown in one batch and the removal 2367 ms after the cut; still not captured: fall direction/duration/curve, the phase model swap (stump vs lying tree), beech/oak cut phases. Farming, coffers, switches (`ChangeDoodadPhase`, `SelectInteractionEx`, `SCCofferContentsUpdate`) remain | opcodes 0x096, 0x081, 0x157; `crates/aa-client/src/hud/doodad/gather.rs`, `tools/evidence/gather/capture-results.txt`, `tools/evidence/doodad_affordance/capture-results.txt` | high | M | none |
| G4 | gameplay | Click-to-move, camera preset actions, hanging / moving supports, less common locomotion | parity row "Ground movement" | med | M | none |
| G5 | gameplay | Tower defense (0x16b to 0x16e), siege alerts (0x0ec to 0x0ef), crime arrest (0x170), appearance gender change (0x1f3) | opcodes | low | M | H3 |
| G6 | gameplay | Cinema / quest camera: hidden cinematic actors are "aa-client's screen-flow responsibility"; `X2Quest.UseQuestCamera` false | `aa-world.md` §6 | low | M | none |
| G7 | gameplay | Misc small: dice (0x134), impulse / knockback visuals (0x079), idle status, visual options, PvP point change (0x204), `SCPlaySound` (0x1de), item look convert, lobby refresh (0x1cd) | opcodes | low | S each | none |
| G8 | gameplay | Vehicle camera distance (A8) and the driven slave's mode bar (C6) are in: the Farm Wagon driver gets the model's wide 16 m camera limit and its five `slave_mount_skills` entries, restored on leaving; the Eznan Cutter helm (model 393, normal 40 / wide 25) reads the same fields at 25 m in camera mode 1, and a bar press casts the slave skill with the slave as the `Mount` caster. Open: the native bar's two function buttons (slots 6/7), ship helm/siege bars (the Cutter helm bar was not read), a bound-but-unseated bar, player death while driving (does 1.2 clear the slave bar?), whether the siege/mount bar really goes to 0 beyond `input_modes.set(1, ...)`, and the pressed slot's exact target field | `captures/2026-10-30/groupA2`, `captures/2026-10-30/doodads`, `aa-client-movement.md`, `aa-client-hud.md`, `tools/evidence/vehicle_camera/` | low | S | a later live capture (ship helm bar, function buttons, death while driving) |

### Audio, performance, tooling

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| A1 | audio | Voice-over gaps: `x2voice_prologue` has no FEV (client-driven indun voices), 59 absent events and 41 absent FEVs play nothing, as in the original client (same pak; nothing to substitute). Legacy CELT decoding is recovered: all11517 subsounds decode, native sword-bank error max1LSB. Its combat critical/twohand/miss/pierce caller gates and native impact timing still need integration; no replacement sound. | bc19e27/e33a783;101 audio checks, `aa-audio.md`; native producer365 cases in ignored celt-audit/producer | low | M | combat parameter/timing integration |
| A2 | audio | Modulation, doppler, spawning/playlists, queries and long-loop streaming recovered; native envelope flags/failed-voice cleanup fixed. Quietest policies 3/5 now use native audibility/admission/pool ordering; 20ms steal ramp and documented gain bridges remain approximations. Authored 3D pan and five proven log-mapped high-pass roots are recovered (7d0281b/8d97fe4; 93 device-inclusive tests and native/captured-mixer controls). Open: unsupported high-pass topologies, reverb/time curves, auto-pitch, seek/sustain, mood pitch/EQ/occlusion, remaining client triggers/live zone crossfades. One authoring-test oscillator exists. | fe13c6c/88afe6c/9ec84a2/fa43393; 73 device-inclusive checks, 13 native selection cases on both paths plus 6 slot cases; `aa-audio.md`, `aa-client-sound.md` | low to med | M | aa-audio/client follow-up |
| P1 | perf | Town fps: Marianople 38 fps (40k entities, 19k mesh entities, 836 mesh assets rewritten per frame by aa-fx, `allocate_and_free_meshes` 8 ms). One fix landed (77d3f01, invisible emitters skip rewrites); no re-measurement. Frame is render-thread bound, GPU 3 to 4 ms idle | `research/perf.md` (draft), commit 77d3f01 | high | M | aa-fx, aa-world |
| P2 | perf | Entity / draw-call count: merge static part entities, cull earlier; `CryAssets::load_model` is main-thread only (prepare / finish split requested) | `research/perf.md`, `aa-world.md` §7 | med | L | aa-cry |
| P3 | perf | 3 to 10 fps stalls when other processes load the CPU (cause found: scheduler hand-offs; above-normal priority class added in `priority.rs`). Not re-verified on a quiet machine; no memory / VRAM budget measured anywhere | `research/perf.md` | med | S | none |
| T1 | tooling | Reference gaps: no `ingame/worldmap_*` check in docs, `combat_kick`, `combat_death`, tooltips, Palace each mentioned once; captures now exist for mounts, carts and ships (`tools/evidence/mount_*`, `cart_*`, `ship_steering`, `ship_deck`); no captures yet for skill book, quest log, mail, party, crafting, friends, housing, weather, night. Real client (`archeage.exe` pid 48924) is up | `reference/`, `tools/evidence/`, grep of docs | med | S per set | N1 (lane) |
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
