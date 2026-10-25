# Territory management integration

The original territory window is NPC service **16912**, not a separate management
dashboard. Its loaded Lua fills both the territory and guard tabs on entry. It
does not request a territory list, details refresh, or tax collection. The received
Dominion rows supply those values.

## Authority and formatting

The HUD reads the same received Dominion cache used by expedition ownership
guards. A decode gap withholds territory values; a later individual row does not
prove that the missing incremental history has been recovered. The faction name,
lord name and leader character ID come from received expedition records. Native
tax admission compares that leader ID to the local persistent character ID,
independently of guild role and policy.

Peace money and AA points are signed decimal strings without clamping or scaling.
`GetTaxations` computes the total base tax plus surcharge, with the original f32
operations before truncating to a signed integer. These are preview values, not
received payment or wallet state.

The native taxation loader uses `SELECT id, desc, show, tax FROM taxations` without
sorting, then resolves each description through localization. The guard-step
loader explicitly sorts by setting and step. The integration preserves these
different orders. The earlier four native arithmetic goldens supplied sorted
taxation fixtures; the independent full-loader proof determines real row order.

Reign timestamps remain received Unix seconds. Calendar display uses the host
client's local timezone through Lua's `os.date`, and projects only year, month,
day, hour, minute and second. Unsupported dates remain unavailable. This policy
does not establish that the host timezone matches a particular retail reference.
Pong's mapped clock is not used for these dates.

## Original research

The ignored evidence directories are:

- `data/extract/aa-ui/territory-management-20261024/`: original loaded ALBs,
  native getter, service, guard, money, date and request contracts. The initial
  audit passed 152 bounded cases and 79 assertions.
- `data/extract/aa-net/territory-management-20261024/`: fixed-field archives,
  receive mutations, request bodies and transport contract. Concrete CryNetwork
  bool/blob behavior is explicitly outside that evidence.

The guard projection implements the initial row, step rows and signed-byte
limits. Its complete metadata is description, category, tipType, name, buff_id
and path. The six real English descriptors have no formula placeholders. Locale
callbacks execute after releasing the HUD bridge borrow. Runtime-only buff
fields such as duration and stack are not added to these guard tooltips.

The native guard getter requires the active NPC's resolved territory, the actual
tower entity and received buffs, and the loaded-house manager's spatial counts.
Catalog records and account-owned house lists cannot establish those values.
`Units` supplies loaded house membership; `Houses` also contains account and
unloaded records. `Buffs.native.of(tower)` distinguishes a known empty list from
missing history. Complete housing-catalog readiness must also be established:
the existing general HUD loader can fall back to an empty housing catalog.

Full service entry is still being implemented. In particular, the native
Seamless query uses local f32 coordinates and signed origin cells. Eight retained
negative-local cases differ from querying a rebased global point. The existing
global sector query cannot silently replace that domain. Normal service-16912
offer/query availability and loaded-population recovery after a lost stream or
same-folder instance change are separate open input contracts.

The follow-up native position codec audit corrected the earlier precision
premise: packed nine-byte XY is limited to ±32767.998046875 metres, and every
encodable XY value is exact in `f32`. The earlier 32832-metre counterexample is
outside that packed domain. No aa-net field addition is required for territory
XY under a proven packed-receipt source restriction. Long-coordinate packets can
lose XY precision; character-selection positions, unsupported relative movement
and rendered/interpolated points cannot silently enter that restricted supplier.
The native local-coordinate/origin history requirement still applies.

The original visible-house-refresh closure captures a nil upvalue. The component
fixture installs a narrow correction that calls the original tab's fill methods
and closes if its guard inputs are unavailable. It is a deliberate correction
to the original defect. The unattended `territory` example uses explicitly
synthetic ownership, NPC, tower and population inputs; it proves no live entry,
retail coordinate mapping or tax settlement.

Protocol T1 is locally committed at `3f893111`, with 775 aa-net tests passing and
nine live tests ignored. It adds the fixed-field receipts and guarded requests;
it creates no optimistic territory state. Root's current caller integration is
uncommitted. Fourteen focused caller tests passed after the ListCtrl correction,
including actual catalog and six-buff native metadata comparisons, original
Apply controls, lifecycle retirement, effective guard-cell paint traversal, and
real mouse picking plus original buff-tooltip enter/leave for all six icons.

## Component rendering correction

The first guard capture had six empty rows: the toolkit hid manually filled
ListCtrl cells until a data refresh. Original native insertion explicitly shows
those cells. The aa-ui correction is committed at `65280223`; manual rows retain
`has_data=false`, and data mutations still hide empty data cells.

Five fresh captures were inspected after rebuilding the caller. Every guard row
now displays its name, badge and buff icon; Refuge Rank 1 is highlighted with
Gate 2/0 and Wall + Tower 31/33. The closed, owner, changed-rate preview and
nonowner controls remain pixel-identical to their earlier captures. All exterior
regions remain unchanged. The new guard capture is
`shots/aa-client/territory-component-guard-fixed-20261024.png`; the before capture
and both manifests remain under the ignored evidence directory. Measured UI-only
FPS across these five shared-machine runs was 391.2–509.8. This is component
rendering evidence, not normal entry, retail coordinate or live settlement proof.

## Ordinary NPC ownership migration

The caller now routes the six service-list consumers through one private owner.
Discovery sends the second object ID as zero. Only a matching received list can
provide the ordered actions, including duplicates and zero. The supported quest
prefix precedes those actions; `interactable=false` suppresses that local
augmentation while preserving the received actions. Only the first effective
action dispatches automatically. Catalog service flags remain metadata, not
received offers. Territory 16912 is not inserted to compensate for its missing
lane producer.

Local epochs qualify objects and delayed UI work; they cannot distinguish identical
old SC07b replies because the wire carries no nonce. A fresh user intent may
replace a cancelled or missing query. The client will not impose an indefinite
global interaction stall to disguise that protocol limit. No socket-write
acknowledgment or completed service admission is inferred from a queued command.
Each selected action has a distinct local cleanup token, even when two actions
come from the same copied offer. An old store's hide/cleanup cannot cancel the
auction selected afterward. Commands and confirmation closures retain their
originating action. Learning an empty ability slot remains available outside an
NPC interaction; replacing an existing ability requires its current service.
Anonymous auction tooltip quotes remain independent of service admission.

The native action deadline is separate from discovery: same-active-actor
dispatch requires unsigned `now > deadline`, unless the deadline is zero.
Accepted attempts store `now.wrapping_add(500)` before later admission checks.
Cancellation preserves the deadline. The host samples `Time<Virtual>` integer
milliseconds; this matches the game-clock category without claiming identical
native clamping, smoothing or pacing.

Native callback reentry distinguishes the in-flight selection from retained
activity. A changed-target callback sees no active interaction, so public cancel
is inactive; the surviving actor still receives065(true) and becomes active.
For the same target, callback cancellation emits END/066 before065(false), and
that tail does not reactivate it. Inactive public cancel preserves the copied
offer and pending discovery. Actual actor/session invalidation independently
blocks continuation. The caller invokes original END listeners outside its HUD
borrow and deletes only the proven NPC message-box owners' dialogs.

The runtime checkpoint passed 752 default application tests (87 ignored).
The later loading/object-retirement fixes and shared fixture passed 24 focused
NPC checks, including the original Lua and all six feature systems. Empty-slot
learning also passed its Lua-to-command regression. Loading retains received
identity and monotonic stamps while retiring old UI work. Replacement receipts
withdraw only the affected object's queued closes, picks and pointer intents.
These are CPU component checks, not normal NPC reachability, socket-write
acknowledgement or retail admission proof.

The `npc_interaction` example supplies standard rendered controls for closed,
store, auction, empty-reply and ended states. All five captures were inspected;
closed/ended and store/empty are each pixel-identical. Observed shared-machine
FPS was 323.4–482.1. The auction choice is programmatic; these captures do not
prove physical secondary selection. Logs, hashes and scope are retained in
`data/extract/aa-client/territory-management-20261024/npc-visual-before-manifest.json`.

The subsequent native audit establishes a separate secondary-selection path:
`DYNAMIC_ACTION_BAR_SHOW("npc", actorId)` and one-based
`ExecuteDynamicAction(index)`. Zero and duplicate slots retain their source
indices. `SelectNpcInteraction` is a native no-op; ordinary first dispatch hides
the dynamic bar before opening its service. A generic `INTERACTION_START` event
on receipt would not reproduce this path. The 37 bounded native/ALB cases and
their limits are retained in the secondary-menu audit. The later 136-case
candidate audit establishes first-insertion deduplication, retaining one zero,
before publishing that separate context list. Received SC07b rows still retain
their duplicates. Current target-candidate activation must not equate the two.

Five caller checks reproduce the recovered service order, native unsorted custom
links, real catalog rows and 350 ms refresh rule. Missing providers remain
distinct from known empty results. Three ordered status-history checks preserve
SC080 across repeated snapshots, ignore status for absent actors, and retain
uncertainty after lost history. The raw carrier is frozen in aa-net at
`3499145193f20133397d519aac139479be6b9088`. Only the explicitly accepted Rust/lane
0/1 wire convention is projected; concrete CryNetwork BOOL normalization remains
unproved, and noncanonical bytes cannot establish native actor availability.
The scoped independent review found no introduced P1/P2 issues. These checks do
not supply orientation, metadata registration, player admission or the remaining
quest, greeting and region providers, and do not establish production NPC bar
activation.

Nation, siege management, native absolute countdowns and tax settlement remain
separate contracts; opcode recovery alone does not establish server support.
