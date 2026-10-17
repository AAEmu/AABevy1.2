# ArcheAge 1.2 feature parity

Updated 2026-10-17. This is the working completion ledger for the original-client parity goal.
The original `scriptsbin/x2ui/` content groups, game database, native client, reference captures,
and lane-server protocol are the sources. A window opening with fallback data is not a complete
feature. Each row requires real data, input/actions, lifecycle/error behavior, and appropriate
live or visual validation. No overall completion percentage is inferred from script counts.

| Area | Current evidence | Remaining work |
|---|---|---|
| World selection and login | Live Trion login, world list/queue, connect; original Lua, zero-error captures | Other regional authentication, premium/house lobby info, reconnect policy |
| Character selection and creation | All four 1.2 races and both genders; appearance controls and server creation/re-entry; real captures | Remaining appearance/material differences, facial idle animation, hat/beard variants, training-field animation/wind |
| Loading and screen lifecycle | Full flow, prebuilt HUD, return-to-list/logout/disconnect, spawn focus tests | Broader interruption/race coverage and persistent driver pipeline cache |
| Ground movement and camera | Walk/run/back/strafe/turn/jump/fall, collision/steps, clipped camera, server movement | Rebindable input; click-to-move/native interactions and less common locomotion modes need audit |
| Water and mounted movement | Wading/swimming/diving; remote mount/rider placement and movement packets | Local owned mounts/pets, mounting UI, ships/vehicles, gliders, hanging/moving supports |
| World geometry | Terrain, brushes, vegetation, roads, forward decals, authored static entity props, streaming, collision; reference captures and unload tests | Deferred/wrapped decals, animated entities/FlowGraph, local lights (active), far impostors/HLOD, remaining placement types and procedural grass |
| Environment | Data-driven time of day, fog, clouds, water, music/ambient/reverb/moods | Night sky, shafts/glow/adaptation, underwater fidelity, moving clouds, zone blending, remaining mood triggers |
| Character and animation rendering | Real assembled meshes/skeletons, locomotion/combat layers, equipment, customization | Facial animation/lip sync/blinks, fur/hair/skin shading differences, clothing clipping and remaining clips |
| Combat and HUD | Skill/cast timelines, hits/projectiles/buffs, dark/restricted slots, target/range/mana gates, death/resurrection | Broad skill-by-skill reference verification; FX decals/material flashes/lights/forces; special mounted/flying/ship casting |
| Skill progression | Real skill tree/tooltips, learn/passives/reset/swap; server requests/state | Broader progression/live coverage; cooldown and tooltip corner cases; damage breakdown option |
| Inventory and equipment | Real bag/equipment/actions, drag/drop/split/merge/destroy, item tooltips, bank transfers/money, independent saved bag/bank tabs | Repair/sorting/expansion, dye/look/enchant/temper/gem workflows and missing tooltip fields |
| NPC interactions and commerce | Quest NPC dialogs; real catalog, buy/sell/buyback and bank in original UI, live transactions | Quest/service choice on multi-service NPCs, repair, ability services, priest/teleport, auction, trade, specialty |
| Quests and map | Accept/talk/hand-in/drop, dialogs/rewards/tracker/log; NPC/service/party map icons | Objective areas/notifier icons, more requirement kinds, cross-zone icons, live housing/ship/public-farm icons, climate/conflict overlays |
| Chat | Live channels/whispers, original chat input and parsing, system/combat messages | Chat layout persistence, full command set, moderation/social integrations and less common channel events |
| Party and raid | Invite/decline/accept/leave/kick/leader; original frames, sparse slots, offline/rejoin, live two-client coverage | Original UI bindings for role/move/convert/loot rules/markers/pings (protocol and two-peer live tests done), group loot, party buff/combat indicators; full raid UI coverage |
| Social/community | Original relationship/expedition/nation frames load | Friends/blocked-users protocol active; their original UI integration, family, guild/expedition, nation and management |
| Mail, trade and auctions | Original content groups load | Real list/detail/attachments/send/claim/delete, trade agreement/locking, auction search/bid/list/cancel |
| Crafting, gathering and trade packs | World entities and item/quest data present | Crafting recipes/actions/proficiencies, gathering interactions, processing/workbenches, specialty/trade pack flows |
| Housing and territory | Static placed world data and some map icons | Housing/placement/farms/permissions, ownership/tax, territory/siege/dominion, construction and interaction flows |
| Pets, slaves and transport | Remote units/support motion decoded and rendered | Owned mate/slave lifecycle, summon/unsummon/equipment/skills, mount UI, ships/cannons/transport controls |
| Achievements, ranking and trials | Original achievement/ranking/usertrial/battlefield/racing UI groups load | Actual protocol/state/rewards, trial/jury/crime/prison, queue/match/score/racing flows |
| Music and other special systems | Audio playback and original music UI assets | Bard/song instruments, composition/playback UI, books/UCC/crest, portals, premium/shop services and other original content groups |
| Options and UI toolkit | Applied account-local options and sound; bag persistence; Reset/Cancel/restart tests; native atlas/phase/movement/widget animation and keyboard capture | Key rebinding and graphics application, layout persistence, remaining native widget methods |

## Active ownership

* Root: `aa-client`, finish commerce verification, then party/friends UI and account-backed input bindings.
* Network agent: `aa-net`, repair/expansion/sort contracts; shop/bank, raid and friends protocol milestones landed.
* UI agent: `aa-ui`, effect clipping/persistent scale and native combat-text timelines; motion/keyboard milestones landed.
* World agent: `aa-world`, authored local lights; forward decals and static entity props landed.

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

Current wave: client 103 passed / 13 ignored, UI 88 passed / 1 ignored, network 136 library plus
2 scripted passed. Fresh live paired raids/friends covered mutation, offline/reconnect and saved
lists. Original commerce UI confirmed 33-copper sale and buyback, 25-copper purchase, bag/bank item
round-trip with identical UID, and 5-copper deposit/withdraw through the original money dialog.
Final captures/logs live under `shots/hud/sol61-commerce/`; these passed with zero Lua errors.
Native lock/queued-sale regression refuses a replaced slot's identity instead of selling the
replacement item. Four decal, five road and three static-entity focused checks passed; owning
docs record exact assets, known variants, source addresses and performance.
