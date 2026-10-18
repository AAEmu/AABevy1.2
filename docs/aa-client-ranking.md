# Original ranking HUD

Production check and all seven focused checks pass, including the real HUD
schedule/typed writer and original physical controls. Full client: 276 passed /
38 existing optional probes ignored in 56.13s. Main and the aa-dev ranking example
build successfully. Four 1920x1080 captures under `shots/hud/ranking/` are inspected:
Arena Ranks (460.0–518.0 FPS), Sport Fisherman (441.4–497.6), the original
reward popup (456.4–500.3), and live lane world select (311.3–442.8). All four
processes self-closed without warnings, errors or panics. The ranking example
uses explicit received fixtures without a network session; world select uses
the real login/world-list receipt. There is no ranking reference capture in the
checked `reference/` set; the unchanged original UI and executed native contracts
provide the ranking layout/data evidence.

`target/debug/aa-client-20261018-ranking.exe` is the inspected main copy;
`target/debug/aa-client-play.exe` is refreshed from it after verifying the alias
was not running. It includes this adapter, physical terrain-hole support, native
32-slot material selection, both cooldown mask paths and the opt-in selector
toolkit (not enabled in production without the remaining native producer map).
The independent raw-caster network commit is included; subsequent network cache
changes require the next build. No in-world ranking settlement is claimed.

X2Rank now uses the real static catalog and received Rankings. The four native
descriptors are Arena Ranks, Sport Fisherman, Heavyweight Fish and Auto Racing;
the unchanged original Lua deliberately builds GetRankCount()-1 visible tabs.
Static catalog localization uses the real English localized_texts columns.
Native SQL queries have no ORDER BY, and association appends in loaded order.
The compact copy differs from the original loaded child arrays. This catalog
reconstructs the captured original four parent vectors by ascending link ID,
explicitly an input-copy compatibility choice. Native runtime labels/order
and owning SQL/association source are archived under data/extract/aa-net/.

The native tab builder runs on an admitted personal receipt. It retains the
static IDs/present flags, chooses the first present one-based default and
falls back from a missing/invalid selected tab. RequestPlayerRecords accepts
zero-based numeric indices and stores the value unchanged, while the later
receipt interprets it as one-based. That original asymmetry is preserved.
The checked catalog has four descriptors; unsafe native tab storage beyond16
is not reproduced. GetRankCount is independent of the builder. Number adapters accept actual Lua
numbers only and use f32/CVTT conversion. Invalid GetRankInfo gives no results;
numeric out-of-range IsOnGoingRank gives one false.

The original getters expose exact personal/snapshot date and value fields,
sparse signed ranking keys with last duplicate winning, optional actual name
and received login-world strings, repeated received scope rows and authored
scope/reward link order. Missing static scope children leave index holes;
missing required reward/scope descriptors abort the whole reward getter.
Optional item/appellation tables require real descriptors; buffType is the
appellation's actual buff_id. Viewing a reward does not grant it.

Personal-board construction has proven empty vectors and zero raw update,
reset and next times before a snapshot. Cached next changes only on genuine
admitted snapshots with updated>0 and a real descriptor: max resolved signed
scope_to below1001 gives1800s, otherwise3600s, with wrapping addition. A zero
update preserves prior next. Reset-period dates use received reset plus3600,
reset+signed(reset_week)*604800-3600, and the latter without the subtraction.

Date providers use the local CRT calendar. The schedule uses the received
acknowledgement time, inclusive unsigned calendar bounds, Monday05:00-07:00
maintenance, authored weekday8/actual weekday and a half-open hour interval
without an invented overnight branch. Static calendar construction preserves
zero-date short circuit, native clamps and forced standard time (isdst=false).
Undefined native CRT outputs are refused rather than replaced by invented
calendar fields; arbitrary invalid timestamp input remains outside the checked
window domain. Native policy/provider oracles are not timezone-conversion
goldens: their CRT-facing providers are explicitly mocked.

Character names remain an independent stream producer. Resolution checks zero
and pending IDs before actual uniquely correlated persistent-ID actors, then
the received raw CString cache; a miss queues a captured stream query once.
An empty received CString is available. No name is guessed from the account,
world, selected header or rank ordinal. Ambiguous actor bindings are refused.
Actual actor names/removals, raw name replies and login-world receipts retain
their source order in callback snapshots. Loading callbacks retain their own
board/tab/provider snapshots; pre-initializer ranking receipts wait for the
real static initializer. Broader mixed-subsystem global callback ordering is
not claimed.

The local lock event precedes timer sampling. Unlock is strictly fresh f32
seconds>stored+3, with the old timestamp still visible inside the callback;
unlock precedes name readiness. Snapshot readiness scans entries in order,
stops at the first missing name and sets ready before RANK_SNAPSHOTS. Requesting
a snapshot marks waiting; a packet alone does not invent readiness. Requests
capture the actual selected owner and use typed CS12f/130 or stream0xa commands.
Transport still requires its real selected header/in-world/stream state.
Lifecycle drops stale captures without optimistic board or score changes.

Source receipts for rank alarms project the actual static kind/text. The native
reward-mail formatter/SYSTEM_MESSAGE integration remains a separate boundary;
no mail delivery or notification is synthesized. Live ranking settlement is
still unverified during the shared lane game-connection stall.

Focused checks cover the captured catalog, return arity/numeric shapes, sparse
rows and name queries, initial/retained native times, pre-ack error callbacks,
tab fallbacks, strict unlock/name readiness, calendar source cases and original
physical tab/refresh/reward buttons and the actual HUD scheduler/typed writer.
Physical popup tests advance the original
scale/alpha animation before clicking its final geometry. The original scripts
and production hit-testing require no workaround.
