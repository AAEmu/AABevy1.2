# Original item-effect result callbacks

The received adapter consumes SC09c/09d/09e/09f in their original order.
It retains a private inventory mirror by applying the same received events as
aa-net, so SC09d formats the post-item-task item while SC09e can format the
pre-item-task item. A successful result notification never modifies items,
money, grades, gems or operation locks.

Native397ccfb0 searches equipment slots0..27 before BAG2 only. Empty equipment
records have persistent ID0: the first such hit stops the search and its invalid
template0 fails formatting, rather than falling through to the bag. A valid
first equipment item whose ID happens to be0 remains admitted. The bag lookup
skips invalid-template records and never searches bank3. Full u64 identities are
compared without passing through Lua numbers.

Socket callbacks require the app's genuinely received spawn and known inventory
history. Character selection, return/disconnect and fresh connection retire the
old active context and pending callbacks. Factual decoder loss or a truncated
item-task batch disables current-item lookup until a fresh selected character;
ordinary unknown framed packets do not. These are explicit app lifecycle and
certainty boundaries. Inline grade results and broadcasts have no invented
active-Game/current-inventory gate: they format their received item directly.

The formatter uses the existing native item-link record path, rejects invalid
template0 and unsupported/inconsistent details, and does not require a static
catalog name. Local grade results admit numeric codes0..4; broadcasts retain the
numeric code without that gate and project the first NUL of safely admitted raw
names. Binary Lua strings are preserved. Names outside the native CString
destination's safe domain remain available in aa-net without an app callback.

SC09d emits ITEM_SOCKETING_RESULT(success,itemLink,templateType,install), with
success exactly result==1 and templateType converted through unsigned f32.
SC09e emits ITEM_ENCHANT_MAGICAL_RESULT(success,itemLink,templateType).
Noncanonical boolean bytes remain raw in aa-net; the frontend refuses to invent
their interpretation. SC09c emits GRADE_ENCHANT_RESULT(result,itemLink,before,after)
and SC09f emits GRADE_ENCHANT_BROADCAST(name,result,itemLink,before,after).
Accepted notifications wait in receipt order while the original HUD is built.

The observed post-callback internal notifications68/69/70 belong to an
item-operation controller rendezvous, not a generic dialog-task close. That
controller owns captured pending keys and further operation continuations.
Those effects, request/choice controls and natural live settlement remain open;
this adapter does not synthesize them. Source, wire oracles and corrected
controller notes are retained in docs/aa-net.md and its ignored source archives.

The stronger original-popup check identified missing GradeName/GradeColor
providers: the popup opened but aborted before its item title. Both now follow
native strict numeric arguments, f32/CVTT conversion and LOW BYTE raw-ID lookup,
with zero Lua results for wrong/missing arguments or descriptors. The existing
display-keyed catalog's Basic/Crude0/1 swap is reversed at this boundary; the
public wrappers add no grade conversion or default. GradeName returns the actual
localized item_grades.name, and GradeColor returns its actual color_argb string.

All6 focused checks pass in18.76s: lookup priority/ID0, receipt order and no
optimistic mutation, result/formatter/CString/numeric domains, raw grade helpers,
the actual deferred HUD schedule and unchanged original grade/socket alarm
scripts. The original check uses actual Crushing Grip24488 and actual catalog
Lunagem/Lunafrost descriptors, checks the completed title and alarm width, and
exercises all five grade outcomes and their actual physical Confirm buttons,
socket success/failure and magical-result paths. The Confirm button is private
in the original script; the fixture obtains it through its real parent/child
widget identity and verifies the actual pointer hit. Main and the aa-dev
item_effects example build. The full copied client
suite passes292 tests /38 optional probes ignored in35.04s, logged in
data/extract/aa-client/trial/item-effects-full-tests.log.

Six1920x1080 captures under shots/hud/item-effects/ are inspected without
script errors/WARN/ERROR/panics: grade success361.4-371.1 FPS, major failure
368.0-416.1, socket success428.0-499.6, socket failure395.5-489.0, actual
Lunafrost result454.0-544.3, and live lane world select336.6-456.2. All six
processes self-close. The fixture has no network session and applies explicit
receipts; it performs no enchantment, item removal, payment or claim on a server.
These are original-script/asset presentation checks, not paired native-scene
pixel equivalence or live item-effect settlement.

Latest inspected main is target/debug/aa-client-20261018-item-effects.exe;
the idle target/debug/aa-client-play.exe alias is refreshed from it. Both SHA256:
2AE30F1F22E4E6058F116119D0DAB7F69A550FE10AB6998F4C3EED2B5AC43AF5.
This build includes raw native cooldown setter behavior, triangle/LOD terrain
weights, native buff getters and the verified received protocol additions.
Later NumberOnly label and WaterSurface-normal work needs another build.
