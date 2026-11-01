# What is left: inventory of AABevy1.2

Refreshed 2026-11-01 against `upstream/main` 37fa30d7. Since the 2026-10-31 audit at 50b0bc59 these
merged: #52 remote vehicle movement, #54 passenger seats, #55 cannons and turrets, #56 seed placement
locks. The audit (`handoff/2026-10-31-remaining-refresh.md`) checked all 73 old rows; this refresh
re-checked every row it changed with `git grep` / `git show` and added the 2026-11-01 findings.
Nothing was built or run for this document.

How to read a row: the status is the start of the Item cell. **DONE** needs a merged PR or commit and
how it was verified: *capture* = the original 1.2 client, *live* = Bevy against a server, *test* =
automated checks, *code-only* = no real-client check exists. `unaudited since <date>` = the last
dated audit of that row; the 2026-10-31 audit did not re-check these and this refresh did not either. Sizes: S = under a
day for one agent, M = a few days, L = a week or more.

Open PRs are in review, not done: client #57 Options freeze, #58 ship cannons drawn and manned,
#59 linked-item skills (Use Glider), #60 vehicle picking (stacked on #58), #61 cinema held keys;
server #30 doodad item gate and unpaid rider skills (the fuel bypass). In progress (branches, not
merged): vehicle idle animation `vehicle-idle-anim` (no commits yet), Farm Wagon retail pass
`fix/wagon-retail`.

## 1. Status summary

At 37fa30d7 the client runs the whole ladder: typed login, World Select, Select Character, Create
Character, Loading into the world on real data (streamed terrain, brushes, vegetation, roads, decals,
water, sky, time of day, collision, walking/swimming, remote players, NPCs, mounts, animated and
equipped characters, combat visuals, ambient audio and the original Lua HUD windows). The 2026-10-18
live block is gone; the auction settlement tail and the final social live checks (N1) are still
unverified. Remote vehicle movement, passenger seats, cannons/turrets and the seed placement locks
landed after the last audit, and the housing builder and construction windows were already on main
(the audit's "not merged" was wrong). The 2026-11-01 owner tests found player-visible vehicle gaps:
the mode-bar function slots (no UI dismount), the F "Grab Machine" prompt, the engine idle and its
sound. Options still freezes the client (PR #57). Overall 1.2 parity is unfinished.

| Status | Rows | Notes |
|---|---|---|
| DONE (features/code) | 8 | each row names its PR/commit and check |
| PARTLY | 23 | |
| OPEN | 46 | 23 of them unaudited |
| CLOSED (no work of its own / meta) | 2 | W1, P4 |
| Total | 79 | 65 old inventory rows + 14 new rows, old section 4 removed |

The 2026-10-31 audit of the old 73-row file found 14 DONE, 21 PARTLY, 36 OPEN (26 unaudited) and 2
WRONG. Its 14 DONE included the seven U1-U7 working-tree rows; those are committed and their rows are
gone with old section 4 (U8 was also removed with that section). Its 2 WRONG were N2 (DONE by design)
and N3 (re-scoped). H2, N5, G5, G7, H9 and H18 were corrected where main had moved on: the housing
builder is merged, the character-list houses and premium grade are wired, tower-defence/siege/arrest
notices are decoded and shown, impulse
and PvP/lobby packets are decoded, own houses are on the map, and the ability change cost is the
service cost. New rows carry today's gaps.

## 2. Prerequisites and cross-cutting (network, login, loading)

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| N1 | network | OPEN (verification): auction settlement mails claimed through the window, other players' bid/buy-out chat notices, and the final social unblock / offline / restart live checks; the 2026-10-18 lane stall no longer reproduces | `aa-client-parity.md:994-995,1046`; 2026-10-20 headless smoke | med | S | none |
| N2 | network | DONE by design (61759806, code-only): a dropped game connection is not reconnected, as in the 1.2 client - the session reports `Disconnected` and returns to idle with the original disconnect dialog; return to world select works through `SCReconnectAuth` + `CARequestReconnect` | `aa-net.md:2088-2090`, `login.rs:168`, `game.rs:1069` | low | - | none |
| N3 | network | OPEN (re-scoped): only 1 of the undecoded `sc` opcodes is sent by the Rust server - `SCCharDetail` 0x053 (`SCShipyardState` 0x1aa decoded 2026-11-01, G12); batch loss no longer applies (an undecodable packet in a `DD04` batch is skipped and the batch resynchronised, `game.rs:946-968`). Add a guard test; the other 123 wait for the server to send them | handoff `unhandled-packets` §1/§3; `git grep` finds neither name in `aa-net/src` | med | S | G12 |
| N4 | network | PARTLY: cart type 2, ship helm request type 4 and deck type 1 with the supporting-object block are written; hanging type 1 with the `0x40` climb data is written (S13, `aa-net/src/climb.rs`); no bound-doodad movement is written (only the ship deck sets `MoveSupport`); `SCUnitFlyingStateChanged` and `SCSkillControllerState` are decoded but nothing in `aa-client` consumes them; `aa-net.md:2085` still says vehicles are not written (stale) | `aa-net.md:259,2085`, `model.rs:1256-1275,1443-1487`, `ship/deck.rs:618`, handoff `s13-s15-decompile` §1 | med | M | G10 |
| N5 | network | PARTLY: `SCLoginCharInfoHouse` is decoded and the character list builds each character's house table (`character_select.rs:190`, `gameapi.rs:407`); the per-character premium grade is computed from `premium_grades` (`gameapi.rs:180`); open: Korean / Mail.Ru / Tencent logins; the parity ledger's "premium/house lobby info" line is stale; no original-client check of either display (code-only) | `housing.rs:128`, parity `:979` | low | S | none |
| N6 | tooling | DONE (14ed6b23, code-only): `aa-net.md` "Known gaps" refreshed 2026-10-23; two stale sentences remain - movement :2085 (N4) and quest cinemas :2080, which #51 now renders | `aa-net.md:2061-2064`, commit 14ed6b23 | low | S | none |
| L1 | login | PARTLY: selection and Ctrl+A/C/X/V done; open: `field.png` / `button*.png` art, IME composition, word-level pointer selection, generic kick text off World Select | `aa-client-login.md:51-62,157-162` | low | S | none |
| L2 | tooling / login | OPEN: game DB read from `compact.sqlite3` next to the server (the client's own copy is encrypted), pak at `<client>` | `aa-anim/src/db.rs:31`, `fake/mod.rs:192` | med (for anyone else) | M | decrypt the client DB, or ship an export |
| W1 | world select | CLOSED (no work of its own): the screen matches the captures (53 of 53 widgets); the outstanding world-select items are N5 (regional logins) and N2 (by design) | `aa-ui.md` table | low | - | none |
| LD1 | loading | PARTLY: `ingame/prewarm.rs` warms the world behind the character list; no persistent driver pipeline cache, `--enter-world` has no loading screen, interruption/race coverage thin | `prewarm.rs`, parity `:981` | med | S to M | none |

## 3. Goal 1: the game loop

Owner priority 2026-11-01: combat and skills, then quests end to end, gathering/crafting/farming,
trade runs and vehicles, housing, HUD windows on fake data.

### 3a. Combat and skills

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| CB1 | combat | OPEN (unaudited since 2026-10-23): sun lighting / live HDR / pool cleanup and bounded adaptive ground drapes are committed and checked; client FxMaterial hit flashes integrated. Bounded direct constant-clock Target ancestors implemented (d34e9e4; 65 tests). Native GeometryInPieces implemented for the four authored flat compound root-node models (0cb85b4; 72 tests), with ordered safety-budget truncation. Native receiver/depth projection, nested/skinned/per-piece-material compounds, Gravity areas, unsupported Wind producer shapes/clocks, indirect Target clocks and integrated per-skill comparisons remain open. Bounded constant spherical Wind producers and other-root FX reception implemented (71b8ff6; 82 unit tests); the client opts in at cap 64. Vegetation reception held on native recurring cache invalidation. Stretched beams do not establish ForceGeneration parity | 2026-10-23 d0b98da/cc9fc34; `aa-fx.md` rounds 17/18, 59 tests plus all-target check; curved geometry has measured cost | med | M | native particle gaps and integrated comparisons |
| CB2 | combat | PARTLY: glider casting merged (#49); ship cannons and siege turrets merged (#55); open: mounted / swimming / flying casters, cast cancel on mount-jump, poses for three channeling skills, two-hand grip, instruments' placement and sounds, remote camera shakes; PR #59 (linked-item skills, Use Glider) is in review | `aa-client-combat.md:651`, #55, #59 | med | M | #59 |
| CB3 | combat | OPEN: native Fired producers, custom GCD, full timeline lifetimes, lane cooldown emitter re-entry check | parity `:988` | med | M | N1 for live check |
| CB4 | combat | OPEN: force-mode (protected / team) eligibility; skill-by-skill reference verification (only 6 combat captures exist) | parity `:987`, `reference/ingame/combat_*` | med | L | more captures |
| G1 | gameplay | DONE (e1e4a6ce, live): duels; challenge prompt, accept/refuse, start/result notices and temporary team factions | `aa-net/src/duel.rs`, `hud/duel.rs/.lua`; live 2026-10-20, not recorded as the original client | med | - | none |
| G5 | gameplay | PARTLY: tower-defence start/end/wave, siege alerts and arrest messages are decoded and shown by `hud/notices.rs`; open: the tower-defence list 0x16b and siege member 0x0ec have no HUD, siege/dominion controls (H3), and the gender/model change 0x1f3 display (`salon.rs` decodes it) | `notice.rs`, `notices.rs`, `salon.rs`; code-only | low | M | H3 |
| G7 | gameplay | PARTLY: dice, `SCPlaySound`, impulse/knockback 0x079, PvP point change 0x204 and lobby refresh 0x1cd are decoded in aa-net; open: the aa-client consumers - no `UnitFx` consumer (knockback, idle status, weapon set, gear visibility), no PvP-point display, look convert | `unit_fx.rs`, `game.rs:1444`, `account.rs:13`; `git grep UnitFx crates/aa-client/src` empty | low | S each | none |
| G9 | gameplay | OPEN (PR #59 in review): "Use Glider" action-bar/skillbook skill 14901 sends `CSStartSkill` with no client gate; the server refuses with 326 `MUST_EQUIP_PROPER_ITEM` (an alert, nothing visible happens). This is inferred from a **Bevy log against our server** (`owner-test-bugs` Bug 1), not from the 1.2 client; the 326 refusal is our server's reply. PR #59 casts the equipped glider's use skill through the 1.2 linked-item rules | handoff `owner-test-bugs` Bug 1 (Bevy), #59 | med | S | #59 |

### 3b. Quests end to end

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| G6 | gameplay | DONE (#51 ae7ca47f, 1.2 capture + PR tests): quest cinemas - `hud/cinema.rs` (993 lines), `hud/cinema_flow.rs`; `X2Quest.UseQuestCamera` follows the option. The 2026-10-31 capture confirmed the 1.2 client's natural playback, HUD hidden and `CSCompletedCinema` at the sequence end (it checked 1.2, not Bevy; the held-key difference is G16); prologue `id_300_*` sequences live only under `instance_prologue`, so in `main_world` the client sends start+complete in the same second | ae7ca47f, handoff `2026-10-31-cinemacap.md`; PR tests | med | - | none |
| G16 | gameplay | OPEN (in review in #61): a key still held when input resumes moves the character at about +1.8 s by the client clock; the 2.419 s figure is unproven (the capture timed packets by server-log read time; the cause may be our server's visible-object resend on `CSCompletedCinema` or a fade/loading step; capture pending). A key released during the movie does not stick; 1.2 has no cinema skip (`LeaveQuestDirectingMode`, `DeclineDirectingQuest`, Esc all do nothing). Main replays held keys immediately (`hud/cinema.rs`, test `a_held_key_comes_back_after_the_movie`); #61 restores the 1.2 input window. Do not add a skip | handoff `2026-10-31-cinemacap.md`, `2026-11-01-evidence-audit-captures.md`; PR #61 | low | S | none |
| Q1 | quests | OPEN: `SCQuestContextFailed`, `SCQuestUnitReqFailed` and the quest error notices are not decoded (the lane server never sent them); the objective texts are client-side. Need a real 1.2 server reply or a lane that sends them | `aa-net.md:2078-2079`; `git grep` empty | med | S | a server/capture |

### 3c. Gathering, crafting, farming

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| G3 | gameplay | PARTLY: gathering, cutdown, ring/plate/phase messages, planting rules (#45-#48) and crop growth + harvest bar (#53, 36168bb4); #56 (37fa30d7) restored the 2 s post-send cooldown, moved the 30 s lock to world load, random yaw only for groups {2,3,12,40} and the 6 m terrain spread rule. Open: `ChangeDoodadPhase` / `SelectInteractionEx` (absent from `aa-client/src`), coffers, legacy ore/rock/shear and gathered-item delivery into free bag space (capture-pending), the Mailbox 11400 "Open Mailbox", tree fall direction / model swap. Trade-pack storage: the 1.2 client sends 15307 with no pre-check (confirmed on the original client), so Bevy is right; the refusal belongs on the server (AAEmu #30 in review) | #56, `aa-client-doodads.md`, handoff `2026-10-31-cinemacap.md` case 2, handoff `2026-10-31-tradepack-gate.md` | high | M | server #30 |
| H7 | HUD | PARTLY: gathering verified (#45-#47), growth/harvest #53, specialty quote exists; open: coin payout, `BackCarryingOrders`, `X2Store` zone-group getters, workbench input/range, legacy ore/rock/shear and item delivery (capture), trade-pack sale flow | parity `:996`, `aa-client-specialty.md` | high | M to L | G3 |
| H8 | HUD | PARTLY: repair, store, specialty and portal windows exist; multi-service NPC choice, ability services, priest buffs, coin/doodad shops, live repair routing open | parity `:990` | med | M | none |

### 3d. Trade runs and vehicles

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| G2 | gameplay | PARTLY. Merged: mounts/carts #31-#38, ship steering #39, deck standing #40, terrain #42, summon check #43, camera/bar #44, dismount #38, walk-to-mount #50, gliders #49, remote vehicle movement #52, passenger seats #54, cannons/turrets #55. Open: climbing exit keys and remote hanger animation (G10), `SCShipyardState` 0x1aa and shipyard building (G12), vehicle exhaust/splash FX (G11), larger-ship specifics, auto-walk, mount jump clips, the copper boiler colour, the mode-bar function slots (G13), the F board prompt (G14), the engine idle (G15) and its sound (A3); PR #58 (ship cannons drawn and manned at the parent's attach point) is in review. Server: 1.2 gives the Farm Wagon (slave 60, model 1008) no point 2 (`model_bindings` = 1, 35, 9-12), so the passenger seat our server offered and accepted is a server bug (fix in progress, no client change); skill 20291 grants buff 4841 without fuel on main, fixed in server #30 (in review; bottest 3c); a GM teleport during a glide does not clear buff 1029 (server, bottest 7c) | PRs #31-#56, handoffs `s13-s15-decompile`, `2026-11-01-disembark-and-wagon-seat.md`, `2026-10-31-bottest.md` | high | L | G10-G15, G12 |
| G8 | gameplay | PARTLY: camera and driver bar merged (#44); #55 adds the gunner bar (shows the point-1 attached skill, casts the real fire skill, keeps every row). Open: the two native function slots (G13), ship helm/siege bars, a bound-but-unseated bar, player death while driving (does 1.2 clear the slave bar?), whether the siege/mount bar really goes to 0 beyond `input_modes.set(1, ...)`, and the pressed slot's exact target field | #55, `aa-client-hud.md`, handoff `2026-11-01-disembark-and-wagon-seat.md` | low | S | G13, live capture |
| G4 | gameplay | OPEN: click-to-move (a setting only; `git grep` finds no implementation), camera preset actions, less common locomotion; hanging/ladders landed in S13 (G10) | parity "Ground movement", handoff `s13-s15-decompile` | med | M | none |
| G10 | gameplay | PARTLY: ladders and hanging landed (S13): a climbable doodad is a `DoodadFuncClimb` phase with an exact `$ladder` helper (the climb segment is the box of the chunk it names), F grabs within the 1.5 m use reach and sends `CSHang` 0x0cb, the climb movement writes the packed 14/10/8-bit `climb_data`, a hang on a moving hull rides it, and remote hangers are placed from `SCHung`. Open: the exit keys and the remote hanger animation are capture-pending; ropes, nets, masts and ledges need live checks | `aa-client-parity.md` S13, `examples/ladder_climb`, handoff `s13-s15-decompile` | med | M | capture |
| G11 | gameplay | PARTLY: vehicle damage states (S15) are in: `FUN_3989f8b0`'s truncating HP percentage (1 normal, 2 `damaged25`, 3 `damaged50`, 4 `damaged75`, 6 `dead`), max HP from unit attribute 6 (the `unit_formulas` base plus `unit_modifiers`; `SCMySlave.max_hp` is not used), variant walk-down, body/wheel/spindle/helper swap on `SCUnitPoints`/`SCUnitState`/`SCUnitDeath` read off-thread and applied only when ready, death entering dying (5) with the `vehicle_models.dying` prefab for `models.dying_time`, the active damage fire/smoke of the variant prefab and its `_spawn_effect*` on a worsening state, and the dead wreck hulls. Open: speed-driven exhaust/splash (`FUN_390c78d0`), `_despawn_effect`, the dying prefab's `dead_01` animation override, debris, and a live 1.2 visual comparison | x2game.dll.c `FUN_3989f8b0`, `FUN_390b4a70`, `FUN_390eb610`, `FUN_390e7910`, `FUN_390eaac0`, `FUN_39109120`; `examples/vehicle_damage`; `aa-client-units.md` "Damage states (S15)" | med | S | a live 1.2 capture |
| G12 | gameplay | PARTLY: Shipyard construction state: `SCShipyardState` 0x1aa decoded byte-exactly, `Shipyards` folded by shipyard id, the unit's prefab drawn at the `step` model and swapped on each packet (step 1000 = the finished ship), `GetShipyardBaseProgress` / `GetShipyardRemainProgress` from `shipyard_steps.num_actions`; `examples/shipyard_stages` checks template 5 through three stages and the launch model. Open: the shipyard material window (it needs `GetShipyardConstructionStepInfo`, the step's material list) and a live capture of a real build | `aa-net/src/shipyard.rs`; `ingame/shipyard.rs`; `hud/housing.rs`; `examples/shipyard_stages` | med | S | none |
| G13 | gameplay | OPEN: the 1.2 mode bar is native (`FUN_39204500` / `FUN_392045f0`), not Lua. Captured 2026-11-01 on the original client (driver's `client_mode_actions`): the Farm Wagon driver's slots 1-5 are 20704/20291/15622/15450/15546, slot 6's tooltip is "Disembark from current device." and slot 7's is "Check details on a summon."; clicking slot 6 sent `CSDiscardSlave` 0x032 and the Grab Machine F prompt sent `CSBindSlave`; the dismount and the seat are our server's replies (`unbind_slave` reason 5, `SCSlaveBound`). Decompile-only, not captured: a passenger gets the same "unbind" function slot (any seat), that slot sends `CSDiscardSlave` 0x032 and is refused above 5 m/s with 0x67, the driver's slot 7 raises `SHOW_SLAVE_INFO` 0x60 with no packet, and a mate's first slot is `command_unmount` (the stub pushes pet command 9; not traced further). Bevy fills skill ids only (`mode_actions.rs` `View.actions: Vec<u32>`) and only for the point-1 driver, so the Farm Wagon driver has no UI way off, a passenger has no bar, and nothing fires `SHOW_SLAVE_INFO` on main (the unmerged `fix/wagon-retail` branch adds an Info key trigger). Fix: an entry enum {Skill, Function}, the function slots in order, a passenger bar, `use_slot` handling; `mode_actions/tests.rs` expects 5 slots and must become 7 for the wagon. This corrects the earlier "1.2 has no dismount button" | handoff `2026-11-01-disembark-and-wagon-seat.md` §1-§2; 2026-11-01 original-client session | high | M | none |
| G14 | gameplay | OPEN: the "Grab Machine" prompt - skill 12076 (점유, "Grab the machine to control it.") on the native dynamic action bar (`dynamic_action_bar.alb`, low-centre, `DYNAMIC_ACTION_BAR_SHOW("npc", id)`), F = `do_interaction_1` -> `CSBindSlave`. Bevy publishes the bar from doodads only (`dynamic_actions.rs` `Source::Npc` is dead, no slave producer), so right-click boards and F shows nothing. Needs a slave candidate (facing + reach, not mounted, not bound), a Source variant, and F/`ExecuteDynamicAction` -> `slave::bind`; 13863 and 19165 wait for a generic interaction cast | handoff `2026-11-01-engine-anim-and-grab-prompt.md` §b | high | S to M | none |
| G15 | gameplay | OPEN (in progress): the wagon body's `fist_ba_relaxed_idle` is the engine loop (`UpdateIdleAnim`, `FUN_390d6690`) and runs for as long as the vehicle exists. Bevy spawns the rigged body through `creature::spawn`, which already starts the same clip, but the owner sees no engine motion; the cause (Animator not ticked / overridden by Locomotion / visibility LOD / clip bindings) is unverified - a live `Bone_engine01` transform over time decides it. Add a headless test that the bone rotation changes between two ticks. Branch `vehicle-idle-anim` has no commits yet | handoff `2026-11-01-engine-anim-and-grab-prompt.md` §a; `ingame/vehicle.rs:296` (`prepare`) and `:499-503` (`creature::spawn`) | med | S to M | none |
| G17 | gameplay | OPEN (in review in #60, stacked on #58; 10 commits): vehicle targeting - left-click/Tab picks with a 1.1 m sphere at `pos + 0.9 * scale`, so most of a ~4 m wagon is missed; right-click works and does not exclude the own vehicle (1.2 targets it too). Tab prefers hostile units within 40 m and uses the stale HUD position while seated. Fix: model bounds / oriented box for slaves, `unit_centre` and the replicated attached position in Tab, and a log line for a pick miss | handoff `2026-10-31-wagon-passenger.md` §3; PR #60 | med | S to M | #58 |
| CH2 | units | PARTLY: mount/rider placement, dismount, walk-to-mount and mount skills done (#31-#38, #50); glider casting #49; passenger seats #54; remote vehicle smoothing #52 merged. Open: humanoid NPCs lack combat idle; `SCUnitMovements` types are read from the packet, not the object | `aa-client-units.md`, #52 | med | M | none |

### 3e. Housing

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| H2 | HUD | PARTLY: aa-net decodes the housing S->C events and writes all 10 C->S housing commands (`housing.rs`); houses render at their stage model; `X2House` (build, construction, house, tax, sale windows) is installed; the builder (ghost, rotation, housing-land/category/garden checks, tax quote, `CSConstructHouseTax` / `CSCreateHouse`) and the construction/build windows are on main (3de58d05, fd986eba); own houses on the map; slice 6a (owner menu/interaction parity: `HOUSE_INTERACTION_START` `viewType`, permission gates, `GetHouseDecoCount` with built-ins, the four actability getters, raw tax weeks) is done with an original-script self-check (`examples/house_owner.rs`); the 2026-10-23 Bevy live check got the 45-certificate Start Construction quote (no create sent). Open: furniture/decoration (H21, the decoration builder `CSDecorateHouse`), sale/buy/cancel/demolish client gates and their `*_FAIL` events, live create/construction/tax/sale settlement, the original-client comparison, door/stair models, house HP/repair, siege/castle land rules | `aa-client-housing.md`; `hud/housing.rs:193`, `ingame/housing/placement.rs`; commits above | high | M to L | H21, capture |
| H21 | HUD | OPEN: house decoration/furniture mode - the decoration tables are loaded but the window's actability calls answer empty (`CSDecorateHouse` not sent; whether the doodad position is house-local is unverified) - and the live housing tail above (create, construction, tax/sale settlement, original-client comparison). The placement checks themselves are implemented and follow the server, so the placement risk is the live create, not missing rules | `aa-client-housing.md` "Not done"; handoff `housing-6-7` §4-§5 | high | L | H2 |

### 3f. HUD windows on fake data

The 2026-10-18 sweep counted about 70 x2ui groups, roughly 22 wired, 9 partly and 14+ without real
data. Since then `X2Auction`, `X2House`, `X2Warp`, `X2UserMusic` and `X2ItemLookConverter` were
installed and the coffer window exists (`coffer.lua`), so that count is stale. Bare stubs still include
`X2Ucc`, `X2Book`, `X2Racing`, `X2Security`, `X2Nation`, `X2Priest`, `X2MiaMail` and `X2GoodsMail`;
`X2PremiumService` has fixed getters and `X2InGameShop` only the enable flag.

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| H1 | HUD | DONE (7f891719, live): auction house - `hud/auction.rs` installs `X2Auction`; search, bid, buy-out, listing, cancel and bid history were driven live between two accounts (Bevy headless, 2026-10-22; not the original client). Open follow-ups are N1: settlement mails through the window, other players' bid/buy-out notices | `aa-client-auction.md`, parity `:995` | high | S | N1 |
| H3 | HUD | PARTLY: territory service and `X2Dominion` over received state (`hud/territory.rs`, 10bca82c); `X2Nation` is a stub; guild web and siege controls open | parity `:994,997` | med | L | none |
| H4 | HUD | PARTLY: `aa-net/src/cash_shop.rs` decodes; `hud/api.rs:1390-1460` supplies the shop-enabled flag and native menu list; no purchase flow; server AA-point wallet missing | `aa-client-hud.md:1395` | med | M | server wallet |
| H5 | HUD | DONE (346b9a03, live): Teleport Book - `aa-net/src/portal.rs`, `hud/portal.rs` installs `X2Warp`, `ingame/teleport.rs`; return points, save/rename/delete and gate use; two live passes 2026-10-19 (not recorded as the original client). Fidelity audit only; 0x115 is anti-tamper, not a portal | `aa-client-portals.md`, handoff `duels-portals-titles` | med | S | none |
| H6 | HUD | DONE (e0414355, code-only): appellations - `aa-net/src/appellation.rs`, `hud/appellation.rs/.lua` | commit e0414355 | med | - | none |
| H9 | HUD | PARTLY: objective areas, notifier icons, service icons, stations/gravesites/public-farm managers, own slave and own houses on the map (`map_quests.rs`, `map_services.rs`, `map_slaves.rs`, `markers.rs`); open: other players' ship markers | `aa-client-hud.md`; code-only | med | M | none |
| H10 | HUD | OPEN (unaudited since 2026-10-23): tooltips - damage breakdown option, `ea_*` / `return_point`, buff caster stats on others, gems / tempers / enchants, repair cost | `aa-client-hud.md` "Tooltip numbers" | med | S to M | none |
| H11 | HUD | PARTLY: look converter done (`hud/look_conversion.rs`, 9754fe97); enchant protected planning/continuation, dye, temper and item secure/unsecure not re-audited | parity "Inventory"; code-only | med | M | none |
| H12 | HUD | PARTLY: quality pages apply (`graphics.rs`); the texture / terrain / character / water / DoF knobs have no renderer knob, key-binding blob and character sync, remaining mode/action consumers, layout persistence open. **The Options window freeze is H20 (PR #57)** | parity `:1003`, handoff `options-hang` | med | M | aa-ui, H20 |
| H13 | HUD | PARTLY: native move/anchor/bound callbacks, resize borders, tab transfer and character metadata persistence recovered; caret/slide preview, drag preparation and release reorder pass 271 toolkit checks; open: broader class/geometry/hover/held-item/link paths and the remaining send-size / commands / moderation audit | f51b3fb/389b6d1/9b3ae0c/f6f0929/f93a96c/6fed35a/9a618ee; `aa-ui.md`, `aa-client-hud.md` | med | S to M | aa-ui follow-up |
| H14 | HUD | OPEN (unaudited since 2026-10-23): overhead 3D glyphs - raid markers, family/guild titles; party buff/combat indicators | `aa-client-hud.md`, `aa-ui.md` | med | S to M | aa-ui |
| H15 | HUD | PARTLY: composition window (`hud/user_music.rs`, 831 lines) with the listed open items: live seat/ACK coverage, Blink local rotation, parented impulse physics, exact graph/camera behaviour, wider mode producers, exact RNG/FMOD/output, existing-Bevy rollout | `aa-client-music.md` | low | M | live render integration |
| H16 | HUD | OPEN: `X2Ucc`, `X2Book`, `X2Racing`, `X2Security` are still stubs; common/public farm, second password, web browser, tutorial (one canned page) open | `git grep` (only login stubs); parity `:1002` | low | M | server |
| H17 | HUD | OPEN: paired live settlement of guild, family, mail, trade, raid loot and friends (unblock/restart); 3D titles; nation | parity `:994-995,1046` | med | M | N1 |
| H18 | HUD | PARTLY (code-only): the ability change cost is now the ability service's (`store/services.lua:17` -> `HUD.service_info().ability_cost`, level x cost); the `X2Player` pay/cash/schedule getters were not located in `aa-client`; not fully re-audited | `hud/store/services.lua`, `hud/store/services.rs:107,216` | low | S | none |
| H19 | HUD | DONE (36641d2e, code-only): `AA_STUB_REPORT=1` reports the stubs and fakes the scripts hit | `hud/stubs.rs` | low | - | none |
| H20 | HUD | OPEN (PR #57 in review): opening Options blocks the main thread 54-77 s (not a loop): `textbox::refit_scales` (6114a22e, 2026-10-25) re-lays out every auto TextBox on every anchor propagation and handler call. Reproduces on main (53.9 s) without buffs/mount/vehicle; the owner's client was killed after ~51 s. PR #57 refits once per change; not merged | handoff `options-hang` (cause `anchors.rs:37,77`, `lua/mod.rs:341,377`, `textbox.rs:36,71`, `animation/widgets.rs:209,495,584`); handoff `owner-test-bugs` Bug 2 | high | M | aa-ui |

## 4. Goal 2: graphics

After the game loop, coordinate with the maintainer's lighting work. These rows carry long dated open
lists; the 2026-10-31 audit did not re-check them and neither did this refresh. Keep them open.

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| WR1 | world | OPEN (unaudited since 2026-10-23): far-plane dissolve, mesh/sprite handoff and shared quads recovered. Olive patches fixed by retaining unsupported normal-mapped leaf-only meshes; 23 models retain sprites. Native brush mesh-pair distance dissolve/reversal/FOV guards recovered; the 0.3-second cvar has registration-only references. Sprite-lighting parity, broader profiling and unit distance policy remain open | b2f2499/2bf76a0/54309c7; 272 tests, 348160 GPU pixels with zero coverage/mask mismatches; `aa-world.md` | med | L | native sprite atlas and P1 |
| WR2 | world / cry | OPEN (unaudited since 2026-10-23): Illum preserves source vertex RGB and squares it after interpolation/composition. Forward punctual highlights and exact authored light specular ratios pass bindless/fallback/churn controls. Terrain forward per-layer gloss/normal/Phong is recovered, adding about 66 MB of arrays. Ordinary VERTALPHA/ALPHAGLOW main and masked prepasses are recovered for 169 submaterials (15 masks); 277 permutations are declined. Open: excluded alpha permutations, native opaque deferred/tiled shading, native shadows, punctual terrain specular and integrated profiling | 56f179a/ab918ae/b690462/3498eeb/328d6a4; terrain 275 tests and 48 GPU controls; Illum punctual 94 checks and 6240 GPU samples; alpha 108 checks, 144 native cases, 3360 GPU samples; `aa-cry.md`, `aa-world.md` | med | M | native pass follow-up |
| WR3 | world | OPEN (unaudited since 2026-10-23): native HDR sun-shaft controls, depth mask, three blur passes and post-film blend recovered; levels/photofilter/colour matrix and eye adaptation follow the engine, blueshift is unused. Camera-far bridge, matched native forest capture, glow buffer/SSDO bounce and broader performance remain open | 2026-10-23 6855713; `aa-world.md`, 20 native probes, six GPU controls, 595 integrated client tests; shared-load FPS not isolated | med | M | camera-far / reference capture |
| WR4 | world | OPEN (unaudited since 2026-10-23): waterfall type-2 UV motion and anchored opacity recovered (53910e9; 98 checks, 36 native matrices, 728 GPU samples). Authored WaterSurface underside and plain volume Outof recovered (227d2f3; 136 pixel controls, 5 crossing states). Source-qualified ocean underside refraction/Fresnel and cone-free post recovered (896a287; 179 GPU controls, 11 image identities). Open: waterfall DualMaterial/Custom/VertexDeform, exact sea colour/scatter/reflection and accepted-receiver foam depth, native near-clip/fog ordering, pond calibration/river darkness, FFT sea displacement. Native water tessellation/crest support and lake/river caustics still need audit | `aa-world.md` §6 | med | M | reference captures of lakes/sea |
| WR5 | world | OPEN (unaudited since 2026-10-23): terrain forward per-layer specular recovered; native deferred/punctual terrain lighting, 4 missing diffuse maps, decal MRT normal/specular, voxel specular/atlases, portal and area lighting, procedural grass blades | `aa-world.md` §6, parity "World geometry" | low to med | M to L | none |
| WR6 | world | OPEN (unaudited since 2026-10-23): qualified steady mesh LOD1/2 bending recovered (2026-10-25, c356ec80; 290 world checks and 24 GPU controls). Remaining: WindArea/dynamic reception and native cache/history/timer, compound/complex/sprite wind, distant suppression/sliding-frame precision; animated entities / FlowGraph / unsupported CGA clips; far clouds animation and calibration | `aa-world.md`, `aa-cry.md`, parity | low | M | native wind cache/subject contracts |
| WR7 | world | OPEN (unaudited since 2026-10-23): time of day - area/relic overrides, subject-qualification producer, 12:00 to 19:05 "noon plateau" never checked against the real client | `research/time-of-day.md`, `aa-world.md` §6 | med | S | real-client captures at several hours |
| WR8 | world | OPEN (unaudited since 2026-10-23): sub-zone queries pass native tests but are not auto-registered live | parity "World geometry / Environment" | low | S | none |
| C1 | char select | OPEN (unaudited since 2026-10-23): skin - one calibrated colour per model instead of the HumanSkin SSS shader; fur drawn flat (ferre); hair spec is a baked strand map; flatter lighting than captures ("Char BackLight" lights missing) | `aa-anim.md` Gaps, `aa-cry.md`, `aa-world.md` §6 | med | M | calibration |
| C2 | char select / create | OPEN (unaudited since 2026-10-23): hat/beard variants - beard meshes absent in the pak for ferre and orc (only dwarf has one), no automatic voice lip sync, appearance-change rebuilding of remote/NPC units | `aa-anim.md` Gaps, `aa-client-units.md` | low | S to M | none |
| C3 | char select | OPEN (unaudited since 2026-10-23): order of the hair/hair-colour grids and preset order not verified against the screen | `aa-anim.md` Gaps | low | S | a reference capture |
| CR1 | create | OPEN (unaudited since 2026-10-23): step 3 field - no wind, no dummy animation; `elf_fog_01` is in no particle library of the pak (WARN each time) | `aa-client-create.md`, `aa-fx.md:362` | low | S | substitute effect |
| CR2 | create / perf | OPEN (checked code-only 2026-10-31): step 3 runs 34-76 fps; GPU bound, the stage camera re-filters the environment cube map every frame (`GeneratedEnvironmentMapLight` at `aa-cry/src/stage.rs:28,53`) | `research/perf.md` "Character create, step 3" | med | S | `stage::environment_light` |
| CR3 | create | OPEN (unaudited since 2026-10-23): salon original UI, preview, paid apply/re-entry and physical payment pass on an isolated server with PR42; data round-trips all eight models; remote appearance, eight-model live coverage and camera/lighting parity remain open | 2026-10-23 `aa-client-salon.md`, `shots/resume/salon-qa` | low | M | bevy rollout |
| CH1 | characters | OPEN (unaudited since 2026-10-23): costume specular / silver armour duller than captures; clothing clipping (the `Cut` hulls leave straddling triangles); dye only for the bake path (weapons, bows, non-default armour dye, dynamic UCC) | `aa-anim.md` Gaps, `aa-cry.md`, parity | med | M | calibration |
| CH3 | units | OPEN (unaudited since 2026-10-23): remote units - full-world facial/attachment QA, CDF entity bindings, skinned render slots | parity "Character and animation rendering" | low | M | none |

## 5. Goal 3: audio, performance, tooling

| ID | Area | Item | Evidence | Vis | Size | Depends on |
|---|---|---|---|---|---|---|
| A1 | audio | OPEN (unaudited since 2026-10-23): `x2voice_prologue` has no FEV (client-driven indun voices); 59 absent events and 41 absent FEVs play nothing, as in the original client (same pak; nothing to substitute). Legacy CELT decoding recovered: all 11517 subsounds decode, native sword-bank error max 1 LSB. Its combat critical/twohand/miss/pierce caller gates and native impact timing still need integration | bc19e27/e33a783; 101 audio checks, `aa-audio.md`; native producer 365 cases | low | M | combat parameter/timing |
| A2 | audio | OPEN (unaudited since 2026-10-23): modulation, doppler, spawning/playlists, queries and long-loop streaming recovered; native envelope flags/failed-voice cleanup fixed. Open: unsupported high-pass topologies, reverb/time curves, auto-pitch, seek/sustain, mood pitch/EQ/occlusion, remaining client triggers and live zone crossfades | fe13c6c/88afe6c/9ec84a2/fa43393; 73 device-inclusive checks, 13 native selection cases; `aa-audio.md`, `aa-client-sound.md` | low to med | M | aa-audio/client follow-up |
| A3 | audio | OPEN: no vehicle engine sound. 1.2 plays `sound_packs` 176 "run_tractor" (`VehicleUnitModel` comment), item `run -> sound 950` (`sounds/x2vehicles:trailer:run`), `yaw -> 950`, through `VehicleUnitModel::UpdateSound`; the speed-to-loop mapping is not read. grep for `run_tractor` and `sound_pack` in `aa-client/src` hits only skills and modelview | handoff `2026-11-01-engine-anim-and-grab-prompt.md` §a | low to med | S | G15, aa-audio |
| P1 | perf | OPEN (unaudited since 2026-10-23): Marianople 38 fps (40k entities, 19k mesh entities, 836 mesh assets rewritten per frame by aa-fx, `allocate_and_free_meshes` 8 ms). One fix landed (77d3f01, invisible emitters skip rewrites); no re-measurement. Frame is render-thread bound, GPU 3-4 ms idle | `research/perf.md`, commit 77d3f01 | high | M | aa-fx, aa-world |
| P2 | perf | OPEN (unaudited since 2026-10-23): entity/draw-call count - merge static part entities, cull earlier; `CryAssets::load_model` is main-thread only | `research/perf.md`, `aa-world.md` §7 | med | L | aa-cry |
| P3 | perf | OPEN (unaudited since 2026-10-23): 3-10 fps stalls when other processes load the CPU (scheduler hand-offs; above-normal priority added in `priority.rs`); not re-verified on a quiet machine; no memory/VRAM budget measured anywhere | `research/perf.md` | med | S | none |
| P4 | perf / docs | CLOSED (meta, doc-only, not counted as DONE): the old status summary, suggested order and uncommitted-work section were replaced; this file is the current inventory | this file, 2026-11-01 | low | - | none |
| T1 | tooling | PARTLY: original-client captures exist for mounts, carts, ships (2026-10-29), the 2026-10-31 recaptures and cinema; the 2026-10-31 bot runs (seats, fuel, storage, plant, house, glider) prove server behaviour, not 1.2 client behaviour. Still none for skill book, quest log, mail, party, crafting, friends, weather, night; housing build only on the bot | handoffs `2026-10-31-recapture*`, `2026-10-31-cinemacap.md`, `2026-10-31-bottest.md` | med | S per set | N1 |
| T2 | tooling | OPEN: docs are 2.6 MB (`aa-net.md` 714 KB, `aa-ui.md` 464 KB, `aa-world.md` 420 KB) and per-crate gap lists are stale in places; this file is now the status page, so the remaining work is keeping the crate gap sections aligned with it | `wc -c docs` | low | S | none |

## 6. Server-side notes (not client rows)

These block client parity but are not Bevy work; they are not counted above.

* **AAEmu #30** (in review, "refuse unmet doodad item requirements and unpaid rider skills at cast
  start"): it covers both the trade-pack storage gate (15307 with an empty back slot gets a failed
  start; the 1.2 client sends it either way, confirmed on the original client) and the vehicle fuel
  bypass (the rider skill's reagent is checked before the slave skill applies buff 4841; on main
  20291 fires and applies the buff without fuel, `2026-10-31-bottest.md` case 3c).
* **Farm Wagon seat**: the server offers and accepts a point-2 passenger seat on slave 60, which 1.2
  data does not have (`model_bindings` model 1008 = points 1, 35, 9-12). Fix in progress (model
  bindings gate in `bind_slave` / `slave_interaction_skill`).
* **Glider teleport**: a GM teleport during a glide does not clear buff 1029; the next 12373 is
  refused (`2026-10-31-bottest.md` case 7c).
* **SG1/SG2 (2026-10-30)**: the server never refuses player moves and negative/bogus fall damage
  heals; not re-checked.
