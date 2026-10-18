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

Latest inspected main: target/debug/aa-client-20261018-native-fades.exe. The idle
target/debug/aa-client-play.exe alias is refreshed from it. Both SHA256:
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
cut precedes the latest frame-order/header-cap/cache integration; the playable
alias remains the previously accepted native-fades build until combined QA.
