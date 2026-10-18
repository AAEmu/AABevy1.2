# Original item-enchant controls

The real bag's enchant button now calls the original window through the native
mode event contract. Grade, socketing, unsocket and gem modes use the original
tabs, submenu, slot layout and close handlers. The mode is updated before its
synchronous callback, so the original tab/submenu callbacks can safely reenter
the same switch. A same-mode request emits nothing. Entry first emits
NPC_INTERACTION_END(), then ENTER_ENCHANT_ITEM_MODE(mode); another active mode emits only
SWITCH_ENCHANT_ITEM_MODE(mode), and leaving emits LEAVE_ENCHANT_ITEM_MODE()
without arguments. Switching does not invent UPDATE_ENCHANT_ITEM_MODE.

Original39064b20/39064ba0/39064c20/39064ca0/39064d20/39064da0 ignore Lua arguments,
select fixed modes1/1/2/3/4/0 and return zero Lua values. X2Item:IsInEnchantMode
(394808c0) returns exactly one boolean from mode!=0. The Lua global
IsActivatedItemEnchant is a different, original visibility predicate; it is not
replaced with a fabricated native method.

The wrappers call39426af0, but that helper cancels only the separate native
action-cursor kind4. It does not clear a picked BAG item (3942e790). The app
preserves the picked item rather than applying a general drag clear. This
action-cursor provider and its mapped action cancellation remain open.

Actual39046740/39046600 execution goldens cover six transitions, including
same-mode retention and callback state/order. Native changed modes retire the
three selected packed keys before callbacks. Mode changes do not directly
reset operation active/input/result latches; this mode-only adapter does not
create such latches or synthesize completion. The ordinary matching cast-stop
branch on Leave remains a separate provider. The first-entry393be180(1) helper's
unconditional NPC_INTERACTION_END is source-closed and delivered after the new
mode and empty selected keys are visible. Native stored-interaction/effect cleanup
and its conditional CS066 require their actual interaction context; no cancel
packet is invented here. Source and controlled-oracle limits are recorded in docs/aa-net.md
and data/extract/aa-net/native-enchant-mode-oracle-20261018.json.

The app also retires this mode on received character/session replacement,
return, disconnect or kick, hiding the old original window while its HUD is
still active. This is an explicit application lifecycle boundary.

The selected-item and quote adapter described below now supplies the original
window. Protected ordinary skill planning, operation-wide pending keys and
result/skill rendezvous remain separate. No local settlement, item removal or
payment is added. The typed aa-net CS052 request transport is already validated
separately; this adapter does not send it prematurely.

Three focused checks pass in4.44s: exact native mode argument/return/event
contracts with reentrant callbacks and preserved picked BAG items; the original
bag button, tabs, unsocket submenu and close button through actual physical
hits; and received disconnect/return retirement in the real HUD schedule. The
fixture advances the normal next layout frame after lazy window creation before
clicking its new tab buttons. Main and the aa-dev enchant example build.
The full copied client suite passes295 tests /38 optional probes ignored in
132.31s with one test thread, recorded in
data/extract/aa-client/trial/enchant-full-tests.log.

Six captures under shots/hud/enchant/ are inspected without script
errors/WARN/ERROR/panics: grade314.8-375.8 FPS, socket387.7-436.0,
unsocket436.0-509.4, gem358.2-438.2,1280x720 grade423.8-473.9 and live
lane world select313.6-460.9. The other captures are1920x1080. All six
processes self-close. These are original-script/asset mode and layout checks;
no matching native enchant capture is present in reference/, so paired pixel
equivalence and natural live enchanting are not claimed. Fixture state and
its black background are explicit; there is no game session or item operation.

Earlier native-fades main: target/debug/aa-client-20261018-native-fades.exe. At that
milestone the idle target/debug/aa-client-play.exe alias was refreshed from it. SHA256:
C236C8B021D8674370142878D8DA9B0E957A1E54B0768B89E2AC4E90E9E53510.
The combined suite passes298 tests /38 optional probes ignored in55.55s with
three test threads (data/extract/aa-client/trial/native-fades-full-tests.log).
Seven inspected captures under shots/hud/native-fades/ include the original
grade365.4-484.7 FPS and unsocket354.2-508.5 controls, grade/socket results,
buff tooltip/charges and real lane world select344.4-422.8. All self-close
without WARN/ERROR/panic. This build includes received-headgear hair selection,
native widget fades, NumberOnly labels, native qualified WaterSurface refraction,
authored cube direction, typed052 transport and preceding item-result/buff
adapters. Inheritance remains in progress; original planner, selection and
controller limits above remain unchanged by this combined validation.

## Selected items and quotes

The adapter tracks received selected character, actual local spawn, BAG chunks,
item changes, level and InitialConfig in receipt order. A complete known BAG is
required before selection/count quotes. Genuine Init/Expanded receipts supply
the native low-byte, ten-slot-floor BAG extent (3961bcc0); content chunks prove
row knowledge without changing that header. Selection and totals respect the
native extent, including nonmultiple-ten and byte-wrap cases. Decoding loss, truncated tasks/chunks
and session retirement clear selected keys. An ordinary unsupported framed
packet preserves them. This is an explicit application certainty boundary.
The original static BAG/BANK stores each contain130 records and SC4c holds
at most five blocks. This adapter refuses whole-BAG quotes above130 enabled
slots and retires certainty for an oversized/out-of-storage contents receipt.
Raw headers/packets remain intact in aa-net; these are application admission
limits, not fabricated native rejection behavior for its unsafe accesses.

SetItemSlotFromBag and X2Item:IsEnchantable use strict numeric-f32/CVTT inputs,
including wrapping decrement and the source's different signed-query versus
low-byte-setter guards. Wrong types return zero results.
The signed query emits the original invalid-slot UI error before its false
result; CVTT overflow followed by wrapping decrement retains its positive-key
branch. The setter tries target, source, then support; failure returns one false
without an event/error. FromPick
consumes a supported item cursor before validation, including invalid equipment,
bank and item-copy sources; absent/skill cursors return no values and survive.
Successful setters emit synchronous UPDATE(presence,false) before returning one
true. Clear methods always emit their native update, even when already empty.
Changed modes clear all selected keys before their mode callback; same mode
retains them. No RefCell borrow is held across these callbacks.

Selection uses actual grade_enchantable, raw item level/grade, Equip detail,
prefix gem count, native slot/socket maps and content_configs.id62. Source
use-skill routing covers the genuine impl0/15/21 families and the direct static
branch outside the native exceptions. Grade and socket compatibility use real
SpecialEffect92/106 rows, not WorldInteractionEffect. The complete zero-free
tag catalog proves the current all-zero effect conditions; nonzero unprovided
conditions are refused. Gem and support validators retain actual group/level/
raw-grade rules, event restrictions and the native grade-order list. The current
catalog has one effect per ordinary source skill, so association order is
unambiguous; no generalized ID sort is claimed.

The secure-item branch uses actual received FeatureSet byte5 bit0x20 and item
flag4. Unknown feature data never becomes false. Native pending keys from the
shared inventory are read before input; selected keys reject repeated selection
and join the existing shop's X2Bag:IsLocked provider without replacing its locks.
Operation-wide shared pending/effect registration remains part of the controller
work. The observed-cast guard conservatively refuses every current local cast;
it is not the native Game cast-pair/Timeline provider, which permits some
nonmatching casts. Nonzero actability effective attributes and the exceptional
model/equipment/special-skill use routes remain unprovided and refuse selection.

Selected getters return zero values for absent/unavailable records and one
ordinary existing item-tip table otherwise. They retain raw itemGrade and add
BAG-only exact-template/raw-grade wrapping-i32 total, including secure/pending
stacks. The existing item formatter's broader native instance/description
coverage remains a separate limit. Grade targets add signed-f32 cost/currency:
formula22 uses signed-i8 item.level, the raw grade's grade_enchant_cost coefficient
and actual slot cost, with native per-child narrowing and final CVTT. Currency
uses received FeatureSet byte5 bit0x10; missing data prevents an invented quote.
Ratios expose three base availability BOOLs; only a selected support adds three
signed-f32 relative variation fields. They are not absolute success odds.

Selected equipment tables also supply socketInfo from the actual slot/raw-grade/
signed-level lookup. maxSocket is distinct from the installed prefix count;
socketItem contains every raw slot through that maximum, including zeros.
Original Socket:SlotAllUpdate is invoked directly in tests as well as through
events, so a missing nested field cannot silently stop before the source portrait.

Gems and socket sources expose native modifier tables from the literal Item-owned
unit_modifiers vectors. All357 current impl15/21 sources have no weapon/armor/
accessory generation record;1785 executed original helper cases prove generated
primary values zero before the formula branch. Static primary attributes are
emitted in native0..4 order, then secondary descriptor order with the shared
54/55/74/119 first-row rule. Zero primary values are omitted; a genuinely empty
modifier vector still returns an empty table. Replacement catalogs with missing
generation/name providers refuse this source presentation rather than fabricating
an empty effect. The native GetItemInfoByType socket-modifier flag distinguishes
missing (-1), explicit nil/wrong type (0), and numeric f32/CVTT values. Impl15
includes its modifier table regardless of that flag. General omitted-grade and
other item-info flag/field contracts remain part of the broader formatter audit.

Execute returns zero results. Missing primary keys retain the original
UI_ERROR_MESSAGE('invalid item slot index'); selected items currently produce
an explicit application unavailable message. It emits no working update,
request, acknowledgement or settlement. This closes the previous silent stub
while request/planner/controller continuation is integrated separately.

Twelve focused checks pass in5.28s:33 executed native cost goldens,39 native ratio
getter goldens, actual four-mode compatibility in either selection order,
source descriptors/support order/prefix gems, native header/packet extents,
known-snapshot/feature/pending/loss
guards, numeric arities, cursor clearing/error order, reentrant updates, preserved
shop locks, exact clear/getter contracts, explicit Execute refusal and original
physical BAG right-clicks for grade/support, socket, unsocket and gem, native
modifier ordering/empty vectors, strict socket-modifier flags and direct full
original updater calls. Fixture
receipts contain real catalog items and full64-bit instance IDs; no connection
or item transaction is performed.
The copied full client suite passes307 tests /38 optional probes ignored
in86.92s with three test threads
(data/extract/aa-client/trial/enchant-frames-full-tests.log).
The cooldown integration fixture now supplies an actual effective GM-mode
receipt before expecting a native stop-duration timer; a spawn's PISH word
does not establish that permission. Its focused check also passes.

Eight1920x1080 repair captures under shots/hud/enchant-selection-repair/ are
inspected: grade200.5-218.5 FPS, support232.0-243.3, socket290.4-300.3,
unsocket251.0-255.9, gem227.4-245.5, buff tooltip156.7-256.1,
grade result291.9-293.7 and real lane world select228.2-287.0. Compilation was
active on the shared machine; these figures are observations rather than an
isolated performance comparison. All eight processes self-close without
WARN/ERROR/panic. Socket/removal source portraits, installed gem rows and the
actual Max Mana200 / Agility2 / Stamina3 effect lines are now visible. The
rejected earlier socket captures are not accepted as validation. No matching
native enchant reference exists, so these are original-script/asset checks,
not a paired pixel-equivalence or live-operation claim. This repair binary
cut precedes the frame-order/header-cap/cache integration validated below.

## Combined playable build

The inspected main is target/debug/aa-client-20261018-enchant-frames.exe;
the idle target/debug/aa-client-play.exe alias is refreshed from this exact
copy. Both SHA256:
347DDF0D98E758E49CCBF6E082048BA2894635637CBF7430D14EBD2F83A00327.
Main and all three enchant/buffs/item_effects fixtures build in1m58s. This cut
contains4591bc3's selected providers,8f5db9e's widget frame admission/update
order and channel retirement,52dae4e's static2D water environment maps/cache
admission fix and87c52d1's pure native task lookup. The full suite passes307
tests /38 optional probes ignored in86.92s, and twelve focused checks pass
in5.28s. New button-state colors, foam-literal precision and raw Update9
retention are subsequent worker slices, outside this binary.

All eight1920x1080 shots/hud/enchant-frames/ captures are inspected and pixel
equal to the repaired selection captures: grade178.9-209.5 FPS,
support263.1-276.1, socket186.2-229.1, unsocket198.8-256.5,
gem223.4-232.0, buff tooltip152.8-189.3, grade result211.2-327.1 and
real lane world select273.0-305.5. All eight self-close without
WARN/ERROR/panic; shared-machine compilation remains active. These are
original-script/asset regression checks, not natural live item transactions
or paired original-client pixel comparisons. Request/planner/controller and
full native inventory/tooltip limitations above remain explicit.

## Native item records for selection and continuation

Enchant selection and its BAG totals now read receipt-ordered native modeled
records instead of the legacy Inventory reducer. Equipment has28 physical
records; BAG and bank each retain130. Unknown is distinct from a known native
empty record. SC040 and local Character-kind0 SC069 body slots19..25 retain
the native template-setter flags1/count1/world255, including template0.
Ordinary empty equipment and Remove resets have world255; SC04c's whole-block
zero initialization has world0. Local SC094 replaces full received records,
including body slots, and duplicate entries apply in received order.

Each supported row resolves its actual PRE-row context through aa-net's pure
projection before the next row applies. Count ignores packet IDs, uses wrapping
i32 arithmetic and preserves zero/negative stacks. Remove resolves equipment,
BAG then bank by ID and may clear a record despite failed declared-location
admission. Grade returns its old grade before mutation; supported raw128 Update
and Add/Take/Owner paths return the resulting record. Move resolves both old
physical records before applying its ordered swap writes, with no consumed-item
output. Flags, non-unpack Security, Seize and native-false RemoveCrafting use
their source-qualified projections. Item/model fields remain the public parsed
representation; this does not expose every native union byte or runtime field.

Unprovided mutations retire affected record certainty rather than adopting a
legacy relocated/post-batch value. A later full contents block can restore
specifically unknown rows. Nonzero unpack Security remains unprovided because
its static-item branch may also change equipment detail. Unsafe/truncated/lost
receipts and local-actor retirement require a fresh selected-character boundary;
same-owner spawn cannot repair them. Init's fresh-row requirement and retiring
records named in the lane's force-remove tail are explicit stronger application
safety policies. The original SC090 handler does not dispatch that tail or
generate an item-consumption observer from it.

Twenty focused enchant checks pass in6.60s, including seven record-provider
regressions and a selection/quote regression across zero-stack then ID-removal
receipts. Existing physical original controls/getters also pass over this new
provider. The copied full client suite passes315 tests /38 existing optional
probes ignored in91.99s with three test threads; its log is
data/extract/aa-client/trial/enchant-records-full-tests.log. The synthetic
result fixture's Add constructor explicitly supplies
raw_detail_block=None; a synthetic item is not promoted to a native raw128
receipt. Execute still refuses explicitly: typed request integration, shared
operation pending/effects and controller callbacks are the next integration
boundary. No request, ACK or item settlement is inferred from this provider.

## Accepted records build

The inspected main target/debug/aa-client-20261018-enchant-records.exe and the
idle-refreshed target/debug/aa-client-play.exe share SHA256
8C3D1CDC2FDDF96F44F050814A93D32822F755D18FF61C1BB1F175D1CC8A3726.
Main and the enchant/buffs/item_effects examples build in3m37s. This frozen cut
contains81bef84's native records,25c549a's button colors,2883227's checkbutton
art visibility,09ea3bf/5520f1b's item projections and87a3c0a/487cfd7's water
precision/static-light changes. The worker's later recharge helper, ordinary
button-art state work, typed request preparation and SC093 follow-up are outside
this executable. Its frozen suite is315 passed /38 optional probes ignored
in91.99s, with twenty focused enchant checks passing in6.60s.

Eight1920x1080 shots/hud/enchant-records/ captures are inspected: grade
283.0-304.3 FPS, support306.4-325.3, socket303.0-310.6, unsocket315.7-324.5,
gem284.2-321.1, buff tooltip166.2-200.0, grade result206.4-224.5 and live lane
world select218.8-265.0. All eight self-close without WARN/ERROR/panic.
Compilation is active on the shared machine; these are observations, not an
isolated performance comparison. Seven captures are pixel equal to the prior
frames cut; the buff differs only in45 pixels at its advancing countdown digit.
These remain original-script/asset regression checks, not paired original-client
pixel parity or live enchant transactions. The controller/planner/transport
integration limitations above remain open.
