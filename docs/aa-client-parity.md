# ArcheAge 1.2 feature parity

Updated 2026-10-18. This is the working completion ledger for the original-client parity goal.
The original `scriptsbin/x2ui/` content groups, game database, native client, reference captures,
and lane-server protocol are the sources. A window opening with fallback data is not a complete
feature. Each row requires real data, input/actions, lifecycle/error behavior, and appropriate
live or visual validation. No overall completion percentage is inferred from script counts.

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
| Skill progression | Real skill tree/tooltips, learn/passives/reset/swap; server requests/state; received skill/all-descriptor-tag and admitted stop-duration cooldowns/preflight, superseding stale DB estimates; original equipment constant-button sweeps preserve item icons/actions | Full native TL lifetimes, Fired cooldown/default/custom GCD producers and modifiers; broader progression/live coverage; lane cooldown emitter/re-entry verification, tooltip corner cases and damage breakdown option |
| Inventory and equipment | Real bag/equipment/actions, drag/drop/split/merge/destroy, item tooltips, bank transfers/money, independent saved bag/bank tabs; original expansion confirmation/150-slot pool, received auto-AA setting, native view sorting and repair controls/quotes/durability notifier with physical-slot actions | Natural repair service routing/live repair and expansion, native premium repair payment, dye/look/enchant/temper/gem workflows and missing tooltip fields |
| NPC interactions and commerce | Quest NPC dialogs; real catalog, buy/sell/buyback and bank in original UI, live transactions; original stablehand quote/confirmation over received Bag+Bank summon details; original player-trade controls bound | Quest/service choice on multi-service NPCs, repair, ability services, priest/teleport, auction, specialty |
| Quests and map | Accept/talk/hand-in/drop, dialogs/rewards/tracker/log; NPC/service/party map icons; completed ship/transport/fish radar snapshots drive original map symbols/tooltips, removal and channel clear with original-script probes/captures | Native radar effective expedition/custom relations, area circles/fog/instance masks and live QA; objective areas/notifier icons, more requirement kinds, cross-zone icons, live housing/public-farm icons, climate/conflict overlays |
| Chat | Live channels/whispers, original chat input/link insertion, clickable item/quest links, native transport escaping and message decoding, system/combat messages | Native send-size overflow policy, chat layout persistence, full command set, moderation/social integrations and less common channel events |
| Party and raid | Original invite/decline/accept/leave/kick/leader/convert/role/drag-move/loot controls and native group-loot prompts; sparse slots, offline/rejoin, unit-frame marks and authoritative map pings; live two-client coverage | Natural paired live loot settlement,3D overhead marks, party buff/combat indicators; broader raid UI coverage and instance-specific ping surfaces |
| Social/community | Original friend/block tabs, acknowledged commands, status/list refresh and blocked chat; original family roster/count/offline rows, native callbacks, feature/owner-guarded actions and captured invitation/ignore controls; original popup and task pointer checks | Paired live family settlement, overhead family titles and broader interruption coverage; live unblock/offline/restart checks; guild/expedition, nation and management |
| Mail, trade and auctions | Original mailbox read/claim/write/send and player-trade carts/money/locks/confirm/cancel bound; original-stage fixtures; mail/trade protocols live-tested in aa-net before the lane stall | Paired live original mail/trade and restart checks; native trade eligibility/window bounds; auction search/bid/list/cancel |
| Crafting, gathering and trade packs | Original Folio browses7,010 real recipes, groups/categories/search/materials/products/higher-tier links; received physical materials/raw grades and native labor rounding; static workbench maps and batch controls; original proficiency grades, caps, upgrade/downgrade/certificate expansion controls/events with pointer probes and captures | Natural workbench input/range/permissions, remaining execution/live validation, premium proficiency payment configuration and housing bonuses, gathering interactions, processing/workbenches, specialty/trade pack flows |
| Housing and territory | Static placed world data/map icons; received house snapshots/progress/permission/demolition flags, parent/TL decoration joins and aligned housing unit tails/posture | Housing/placement/farms/permissions, ownership/tax, territory/siege/dominion, construction and interaction flows |
| Pets, slaves and transport | Remote units/support motion rendered; owned mate/slave metadata, original current-pet frame/bar and information panel with native received-level/equipment/buff attributes, pool maxima and modifier colors; original pack-specific equipment icons and own-item hover tooltips with tag29/received pet level cap; filtered skill presentation/capabilities, received-XP getter, original rename and behavior controls; stablehand revival requests and raw equipment transactions decoded | Native speed-component getter, equipment cursor/confirmation/transaction controls, general skill usability/planner/auto-use, cap-aware XP delta folding, mount/dismiss eligibility, summon/unsummon/equipment/skills, slave UI and ships/cannons/transport controls |
| Battlefields | Original recruiter, queue/cancel, three-choice invitation, received match phases/timers/score/results and leave controls; native slot244 skill identity/tag/enable gates; controlled original-script probes and captures | Natural lane matchmaking/live match QA; lane Pong/start timer-origin mismatch; protected native ordinary skill eligibility and actor planner/casting; queue errors and remaining scheduled-event presentation |
| Achievements, ranking and trials | Original achievement/ranking/usertrial/racing UI groups load | Actual protocol/state/rewards, trial/jury/crime/prison and racing flows |
| Music and other special systems | Audio playback and original music UI assets | Bard/song instruments, composition/playback UI, books/UCC/crest, portals, premium/shop services and other original content groups |
| Options and UI toolkit | Applied account-local options/sound/key bindings and bag persistence; original Apply/Cancel/Default Keys plus restart/action-dispatch tests; native clipped chat captions and fixed/auto tab layout, flat/tree listboxes, state/separator art, row tooltips, dropdown mouse selection/layers, widget animation, Unicode clipboard, IME and keyboard capture | Native binding blob/character synchronization, remaining mode/action consumers, graphics application, scaled clipping/font calibration/layout persistence, listbox template/line-border variants and other native methods |

## Active ownership

* Root: `aa-client`, original Folio browsing and explicit received NPC body slots/rigid attachments passed original-stage, physical-input and inspected visual checks; crafting/proficiency actions and pets/transport next. Repair/notifier, received auto-AA, facial integration, inventory sorting/150-slot pool, expansion confirmation, received cooldown and original breath bar milestones passed. Key settings, mailbox/group-loot/trade, friends and native combat-display milestones landed.
* Network milestone: `aa-net`, underwater receive/source audit complete; conflict-zone reports, received cooldowns, environmental/combat flags, explicit force-attack, UIData, repair/expansion and owned mate/slave APIs landed, with server limitations documented.
* UI agent: `aa-ui`, native listbox hierarchy/art/tooltips for original crafting/achievement, including visible/animated templates and white/grey state fallbacks; native48px icon rejection audited. Flat dropdowns/layers, font alias audit, IME, chat insertion/transport, cursor categories, clipboard, links, edit and combat timelines landed. Read-only native repair API audit supports root's next integration.
* Material milestone: `aa-cry`, UCC defaults, Eye color and alpha cutouts landed; no root integration hook needed.
* World agent: native Group14 polygon ToD geometry/blending/lifetime landed with native numeric oracles; gameplay subject qualification remains unhooked pending proof of bit0x20's producer. Native skybox/cloud spatial opacity and asset fades, curve tangents/defaults, spatial raw-curve zone blending, cloud motion/geometry/lifecycle, deferred/wrapped diffuse decals, native moons/coronas/orbit, sky order, static entity props, local lights, stars and rigid windmills landed.
* Animation milestone: authored FSQ/FXL facial curves and exact expression overlays for all eight playable heads, including original blink sequences; local-player, preview and supported bound NPC CDF integration passed. Selected visible/awake remote Characters now opt in/out in place with bounded private mesh ownership; full-world target QA remains open. Original facial bone displacement/order/base-pose contracts and native helmet hair/beard selectors landed; root equip/appearance wiring and automatic voice lip sync remain open.

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

The auction live fixture exposed a lane-server stall after refund claim/delete. Login/world-list
still work, but subsequent game connections time out. This is a validation blocker, not a proved
client transaction failure. No service restart or server edits were performed. Further live
claims are gated off; root continues original-script, protocol and rendering checks. Auction
settlement tail/reconnect and final social unblock/offline/restart live checks remain unverified.


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
