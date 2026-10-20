# ArcheAge 1.2 feature parity

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

| Area | Current evidence | Remaining work |
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
| NPC interactions and commerce | Quest NPC dialogs; real catalog, buy/sell/buyback and bank in original UI, live transactions; original stablehand quote/confirmation over received Bag+Bank summon details; original player-trade controls bound | Quest/service choice on multi-service NPCs, repair, ability services, priest/teleport, auction, specialty |
| Quests and map | Accept/talk/hand-in/drop, dialogs/rewards/tracker/log; NPC/service/party map icons; completed ship/transport/fish radar snapshots drive original map symbols/tooltips, removal and channel clear with original-script probes/captures | Native radar effective expedition/custom relations, area circles/fog/instance masks and live QA; objective areas/notifier icons, more requirement kinds, cross-zone icons, live housing/public-farm icons, climate/conflict overlays |
| Chat | Live channels/whispers, original chat input/link insertion, clickable item/quest links, native transport escaping and message decoding, system/combat messages; eight native local tab IDs, filters/full palette/font, numeric selection/input method, hover/idle opacity and owner-window locks persist per world/character; original settings Save/Cancel/Reset, context Lock/Unlock, blocked drag warning, first-tab window movement and full Reset Chat with independent extras reload; native pending trial method at actual chat open and source-ordered trial receipt producers | Native send-size overflow policy, window position persistence, other-tab splitting/reparenting, provider-specific extra name filter, full command set, moderation/social integrations and less common channel events |
| Party and raid | Original invite/decline/accept/leave/kick/leader/convert/role/drag-move/loot controls and native group-loot prompts; sparse slots, offline/rejoin, unit-frame marks and authoritative map pings; live two-client coverage | Natural paired live loot settlement,3D overhead marks, party buff/combat indicators; broader raid UI coverage and instance-specific ping surfaces |
| Social/community | Original friend/block tabs, acknowledged commands, status/list refresh and blocked chat; original family roster/count/offline rows, native callbacks, feature/owner-guarded actions and captured invitation/ignore controls; original guild roster/location/offline rows, permission/title editing, invite/reply, leave/disband, kick/rank and received-siege-guarded leadership transfer; guild-service preconfirmation/sponsors/Create and actual received Task35 Rename | Paired live family/guild settlement, overhead family/guild titles and broader interruption coverage; live unblock/offline/restart checks; lane political-system/rename completion gaps, guild web/siege controls, nation and remaining management |
| Mail, trade and auctions | Original mailbox read/claim/write/send and player-trade carts/money/locks/confirm/cancel bound; original-stage fixtures; mail/trade protocols live-tested in aa-net before the lane stall | Paired live original mail/trade and restart checks; native trade eligibility/window bounds; auction search/bid/list/cancel |
| Crafting, gathering and trade packs | Original Folio browses7,010 real recipes, groups/categories/search/materials/products/higher-tier links; received physical materials/raw grades and native labor rounding; static workbench maps and batch controls; original proficiency grades, caps, upgrade/downgrade/certificate expansion controls/events with pointer probes and captures | Natural workbench input/range/permissions, remaining execution/live validation, premium proficiency payment configuration and housing bonuses, gathering interactions, processing/workbenches, specialty/trade pack flows |
| Housing and territory | Static placed world data/map icons; received house snapshots/progress/permission/demolition flags, parent/TL decoration joins and aligned housing unit tails/posture | Housing/placement/farms/permissions, ownership/tax, territory/siege/dominion, construction and interaction flows |
| Pets, slaves and transport | Remote units/support motion rendered; owned mate/slave metadata, original current-pet frame/bar and information panel with native received-level/equipment/buff attributes, pool maxima and modifier colors; original pack-specific equipment icons and own-item hover tooltips with tag29/received pet level cap; filtered skill presentation/capabilities, received-XP getter, original rename and behavior controls; stablehand revival requests and raw equipment transactions decoded | Native speed-component getter, equipment cursor/confirmation/transaction controls, general skill usability/planner/auto-use, cap-aware XP delta folding, mount/dismiss eligibility, summon/unsummon/equipment/skills, slave UI and ships/cannons/transport controls |
| Battlefields | Original recruiter, queue/cancel, three-choice invitation, received match phases/timers/score/results and leave controls; native slot244 skill identity/tag/enable gates; controlled original-script probes and captures | Natural lane matchmaking/live match QA; lane Pong/start timer-origin mismatch; protected native ordinary skill eligibility and actor planner/casting; queue errors and remaining scheduled-event presentation |
| Duels | Challenge from the target popup, challenge prompt (default dialog with the original `locale.duel` texts), accept/refuse, `STARTED_DUEL`/`ENDED_DUEL`, start text and result notice, temporary team factions on plates/target | Retail placement of the countdown text (system message here); the AAEmu server now sends `SCDuelStartCountdownPacket` at acceptance (NickMesser/AAEmu#15), so the countdown shows live; `SCAreaChatBubble` 0x153 (type 543, ignored) |
| Achievements, ranking and trials | Original achievement UI over all1,743 descriptors, received native progress/rewards, six tracing slots and ordered completion/recent callbacks; guarded native type5 tracing hydration/save-before-leave; original physical controls passed; received crime/jury headers, queue request, original labels/hover/waiting tooltip and dishonor glyph; native19-key trial data, record pages, callback snapshots/ticks and actual Task6/7/8, Confirm/Cancel/Verdict controls; original ranking tabs/records/rewards over static catalog, received boards, independent streamed names, native calendar/locks and typed controls; bounty getters/setup/empty bulletin | English summary/description data, unsupported UI-data grammar and live achievement save/re-entry validation; ranking reward-mail formatter and live settlement, original nonempty bounty asset defect/payment lane gaps, trial spatial strings and mixed-subsystem ordering, crime reporting/prison and racing flows; paired live crime/jury settlement |
| Music and other special systems | Audio playback and original music UI assets | Bard/song instruments, composition/playback UI, books/UCC/crest, portals, premium/shop services and other original content groups |
| Options and UI toolkit | Applied account-local options/sound/key bindings and bag persistence; original Apply/Cancel/Default Keys plus restart/action-dispatch tests; native clipped chat captions and fixed/auto tab layout, flat/tree listboxes, state/separator art, row tooltips, dropdown mouse selection/layers, widget animation, Unicode clipboard, IME and keyboard capture | Native binding blob/character synchronization, remaining mode/action consumers, graphics application, scaled clipping/font calibration/layout persistence, listbox template/line-border variants and other native methods |

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
