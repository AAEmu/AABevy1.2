# Original specialty ratio receipts

The received SC099 signed ratio now feeds the original X2Store:GetSpecialtyRatio
getter and NPC_INTERACTION_START("specialty") callback. The network forwards
the native/lane four-byte value without a second cache (f51d264). The HUD keeps
its cache independently of ordinary merchant-window close/reset state.

Original394fb610 stores every nonzero signed ratio before the synchronous
specialty callback. Zero reports localized error225,
STORE_NPC_NOT_HANDLE_THIS_ITEM, while callbacks still read the previous ratio,
then resets to100 without a specialty callback. Negative values are not rejected.
The receipt has no active-Game/actor/interaction gate. When no UI runtime exists,
the cache still changes and the dropped callback is not replayed on a later HUD.
World/character/session receipts do not invent extra native cache resets.

GetSpecialtyRatio394aeb80 ignores its Lua arguments and always returns exactly
one numeric value, converting the cached signed i32 through f32 first. The
constructor39991f50 copies global39ac9b74; that global and the original cache
were both read as100 in the verified original StandMode client. This100 is an
explicit compatibility input, not a claim to reproduce its protected initializer.
The other native reset occurs after a genuine specialty-sale sender; no sale
or fabricated reset is introduced by this receipt slice.

The unchanged original npc_interaction.alb routes "specialty" to the original
specialty message-box handle. Its price formatter now uses the received-target,
physical-backpack and known-bundle providers described below. A valid native
empty equipment slot remains an itemType0 table, not
nil, so an unrequested nonzero receipt with no goods is not clean ordinary
request-domain QA. Original scripts extracted through aa-pak and decompiled for this audit
are under ignored data/extract/aa-client/specialty/. Native codec/handler oracles
and source are retained under data/extract/aa-net/native-specialty-ratio-*;
the owning network notes contain exact source and mocked-endpoint boundaries.

Automatic requests and sale remain separate from passive priced confirmation. The genuine
CS043 payload is the equipped backpack ITEM TEMPLATE ID, not an NPC ID. Its
original specialty_trader callback requires the actual backpack, class kind3,
static item price/origin specialty-zone/region and optional skill18458 admission. The lane's
NPC interaction-option chain has no specialty service branch, so an arbitrary
NPC interaction must not synthesize one. Refund's registered adapter requires
strict numeric input even though its body computes from actual NPC/backpack
context rather than the supplied refund. The known-bundle confirmation subset
is now bound; regional fallback and genuine request/sale admission remain open.

Four focused client checks are implemented for the numeric/arity contract,
error/reset/event ordering, actual Bevy receipt scheduling without a runtime,
and the unchanged original dispatch into its specialty handle with only the
unprovided OnShow price-formatter endpoint mocked. All four pass in2.25s.
The full frozen client suite passes327 tests, with38 existing optional probes
ignored, in72.75s (data/extract/aa-client/trial/specialty-layer-full-tests.log).
It includes UI3332e92's ordinary drawable-layer flag changes; the preceding
pre-layer327 run is separate evidence and is not substituted for this final run.

Main+specialty build passes in2m; the final diagnostic example refresh takes
44.78s. The aa-dev specialty example injects a separately explicit PRE cache137
without a UI callback, then a synthetic received zero ratio after2s through the
actual Bevy NetEvent consumer. It uses the unchanged original HUD and localized
error225, with no connection, request, NPC quote or sale. A3s capture shows the
one-second-old error while retaining the normal2s FPS log sample.

Both1920x1080 shots/hud/specialty-layers/ captures are inspected and self-close,
with no script/runtime warnings, errors or panics: ratio-error462.4-494.7 FPS
and live world-select383.2-421.6. The former visibly prints the original
"This merchant doesn't deal in this type of item."; the latter shows Lane-bevy
Up. These are explicit fixture and live world-list checks, not natural specialty
request/confirmation/sale validation or isolated performance measurements.

The frozen main target/debug/aa-client-20261018-specialty-layers.exe has SHA256
2495C40316A0E83F735E64D1E2FDC8FA4FECD8F697ACDA6B84742CB450E87994;
target/debug/aa-specialty-layers-20261018.exe has SHA256
7742066C7768C7A47614ACAB8E50588C789E5753480A2194DBCFA047F0D76715.
The idle target/debug/aa-client-play.exe alias was refreshed at17:31:23 local
and hash-verified equal to the main. This cut includes networkf51d264 and
UI3332e92; subsequent overlay traversal and brush LOD work are outside it.

Current source now dispatches specialty callbacks inside the same native item
receipt pass used by enchant selection and result links. A callback therefore
reads all preceding native item rows before later packets in the same frame.
The existing item callbacks retain their HUD admission; SC099 alone retains
the actual stage runtime without a Game/HUD-up gate. Its cache still changes
when no runtime exists, without replay into a later stage.

All five focused checks pass in5.25s, including the added ordered-row regression:
Count7->4, ratio callback observes4, then a later Count produces9. This follows
the physical native item despite mismatched packet ID/template and runs with
HUD-up false to verify independent SC099 admission. The aa-dev error fixture
now injects in PreUpdate and uses this shared receipt consumer. This follow-up
is outside the inspected2495C403...0E87994 executable until the next refresh.

## Received-target known-bundle confirmation

X2Store:GetSpecialtyRatioRefund now replaces the fallback with the native strict
numeric adapter. Missing, nil, boolean, string and table arguments return zero
results. Numeric arguments, including NaN and infinity, enter the body; their
converted value is unused. A known successful money quote returns exactly
true,amount; a specialty-coin quote returns true,amount,coinType. Signed amounts
and coin IDs cross the native f32 boundary before becoming Lua numbers. Zero
and negative computed amounts retain the successful return shape.

Refund reads the current target, not the separately retained interaction NPC.
The native SC084 callback391d5270 forwards BC object/target IDs to394270a0,
which resolves the actor and writes its encrypted+1aa0 target field. For the
actual local actor this is the same Game+1aa0 provider used by394ae8c0. The
client therefore keeps a private receipt-ordered aa_net::state::Target alongside
received NPC actor/template rows. Other actors' target receipts do not change
it, and later packets in a frame cannot leak into an earlier SC099 callback.
Native local target setters also exist; requiring a received local target is an
explicit supported app subset. Unknown actors/templates and certainty/lifecycle
boundaries decline a quote. These fences do not reset the independent ratio.

Equipment comes from the active physical native record at EQUIPMENT26, without
a BAG-completeness, count, UID or supplied-refund gate. GameData loads the static
item_backpacks class, actual NPC descriptor coin, specialty_npcs bundle,
specialty_bundle_items profit/ratio, raw item-grade refund multiplier and
content_configs row ID22 through its existing readonly SQLite connection.
Native first matching bundle item wins; missing mapped-bundle goods report
localized error225 before returning one false. Missing descriptors, grades or
required configuration refuse a quote. A missing NPC bundle still refuses the
unprovided regional fallback rather than guessing the current region.

The arithmetic uses aa_net::specialty::quote_known_bundle (6032de1): wrapping
integer products, native float stores/rounding and the source-initialized x87
PC64 sequence, including coin division. The captured original1.2 NPC factor
map is empty and its getter returns100; this is a qualified compatibility input,
not a configurable future assumption. Configuration lookup is by ID22=10000,
not kind_id22, whose different row would incorrectly divide by2.

The aa-dev specialty example accepts --specialty-quote money|coin|full. It
supplies explicitly synthetic native equipment/NPC/target receipts and the
ordinary Inventory-to-HUD projection, then receives SC099 through the actual
shared Bevy consumer. The original tooltip, OnShow formatter, dialog builder,
ratio text and button policy run unchanged. A delegating observer retains only
the created dialog identity. No connection, automatic request or sale is sent.
The cases select money308464 at130%, coin33 at130%, and coin0 at-1% with the
original OK button disabled. Full-client executable/visual validation follows
separately; the previously accepted2495C403 cut does not contain this source.

Eleven focused checks pass in6.88s, including all twelve executed original-price
vectors across three NPCs/raw grades/signed ratios, strict adapter arity, actual
receipt-order equipment/target observations, independent interaction-manager
state, static first-match/configuration/error boundaries, and the unchanged full
original confirmation in all three example cases. Each original dialog is closed
through a physical hit/press/release on its Cancel button, without a queued
server command. The test observer delegates to the real dialog builder and
does not replace tooltip, pricing, OnShow, rendering or button handlers.

## Trade-pack sale, 2026-10-28

The original quote dialog's OkProc calls X2Store:SellBackPackGoods(); the
native function sends CSSellBackpackGoods (0x042) with the bc-encoded interaction
manager's NPC object id. The owner-driven 1.2 session
(`tools/evidence/specialty_sale/native-manual-session.txt`, en_us r208022) at
Hedge, a Gold Trader (npc template 4473, object 39124) recorded two CSInteractNPC
packets: the early 0x065 `len=4 body=d4 98 00 01` at talk start, then the
quote's CS043 `len=4 body=7a 4e 00 00` (backpack template 20090) answered by
SC099 130 %, then the tail 0x065 `len=4 body=d4 98 00 00`, then
CSSellBackpackGoods `len=3 body=d4 98 00`. The server sent SCItemTaskSuccess
twice and SCCharacterLaborPowerChanged, with no SCErrorMsgPacket 0x10A; the
pack was consumed, labor went 5000 -> 4940 and the lane wrote mail 10002
type 19 "Speciality Payment". The same session's ordinary NPC
("Collector Remnos", npc template 9050, object e6 41 00; not a merchant, no
service window opened) sends one 0x065 per talk, `..01` on the
first talk and `..00` on repeats. The `..00` holds while the interaction stays
active; after a window close the existing flag logic sends `..01` again
(upstream behaviour, not changed here).

X2Store:SellBackPackGoods queues the same request through hud::api::Request
and aa_net::Command/ClientMsg, encoded with the existing three-byte bc writer.
The NPC is the interaction manager's NPC (`dialog.npc`, the NPC the quote is
opened with), not Refund's current target, matching the native
394fb8a0/manager39aff9b4+4 source (docs/aa-net.md:7615); the unit test keeps a different current
target set (0x1111) and still sends the dialog NPC (0x684b).
Every NPC talk now sends CSInteractNPC (0x065) when the talk is staged, before
the offer's service runs, with `target_changed` true only on the first talk to
that NPC. A talk that opens a local store/service window has no further 0x065:
the merchant "Anara" (npc template 11643), whose store window opened on both
talks, sent one 0x065 per talk, `..01` then `..00`, each right after the
server's SCNpcInteractionSkillList reply
(`tools/evidence/specialty_sale/native-merchant-session.txt`). The specialty trader's received
offer is the data-only interaction skill 25205: it opens no local window, so
after `finish()` the tail 0x065 `..00` is sent (1.2's second 0x065 after the
interaction skill), and `dialog.npc` keeps the trader for
X2Store:SellBackPackGoods unless a callback cancelled it. The captures end at
the sale's 0x042 and do not show the trader's end-of-talk (NpcEnd), so that
packet is not covered by a 1.2 capture yet.
The replay script `tools/evidence/specialty_sale/replay.txt`, run by
`tools/specialty_sale_replay.py` with `aa-client --headless --mute` on the same
account and character, targets and interacts with the trader, then sends
`0x065 len=4 body=d4 98 00 01`, `0x065 len=4 body=d4 98 00 00` and
`0x042 len=3 body=d4 98 00`; the server completes the sale (SCItemTaskSuccess
twice, no SCErrorMsgPacket 0x10A, new mail 10004). The replay invokes the
dialog OK action directly and sends neither CS043 nor CSStartSkill 25205, and
the server still walks the same CS042 sale path.

## Known gaps

* Bevy sends the tail 0x065 immediately at dispatch, while 1.2 sends it after
  the 0x052 CSStartSkill and the 0x043 CSSpecialtyRatio (about 2 s later in the
  capture).
* Bevy's 0x068 CSStartInteraction `extra_info`/`mouse_button` fields are 0/2,
  while 1.2 uses 1/3 for the same talk
  (`tools/evidence/specialty_sale/report.txt:96-97`).
* Bevy sends neither 0x052 CSStartSkill nor 0x043 CSSpecialtyRatio when the
  replay invokes the sale dialog directly.

