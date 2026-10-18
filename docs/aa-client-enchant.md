# Original item-enchant controls

The real bag's enchant button now calls the original window through the native
mode event contract. Grade, socketing, unsocket and gem modes use the original
tabs, submenu, slot layout and close handlers. The mode is updated before its
synchronous callback, so the original tab/submenu callbacks can safely reenter
the same switch. A same-mode request emits nothing. Entry emits only
ENTER_ENCHANT_ITEM_MODE(mode), another active mode emits only
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
branch on Leave and external393be180(1) registration side effect remain separate
providers. Source and controlled-oracle limits are recorded in docs/aa-net.md
and data/extract/aa-net/native-enchant-mode-oracle-20261018.json.

The app also retires this mode on received character/session replacement,
return, disconnect or kick, hiding the old original window while its HUD is
still active. This is an explicit application lifecycle boundary.

Item-selection validation, exact instance-info fields, ratios/costs, protected
ordinary skill planning, captured pending keys and result/skill rendezvous are
still being integrated. The empty window keeps Execute disabled through its
original scripts. No always-admissible action, local settlement, item removal
or invented payment is added. The typed aa-net CS052 request transport is
already validated separately; this adapter does not send it prematurely.

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
