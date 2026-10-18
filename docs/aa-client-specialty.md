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
specialty message-box handle. Its price formatter still requires the actual
NPC/backpack providers below; no local sale or placeholder transaction is
created. A valid native empty equipment slot remains an itemType0 table, not
nil, so an unrequested nonzero receipt with no goods is not clean ordinary
request-domain QA. Original scripts extracted through aa-pak and decompiled for this audit
are under ignored data/extract/aa-client/specialty/. Native codec/handler oracles
and source are retained under data/extract/aa-net/native-specialty-ratio-*;
the owning network notes contain exact source and mocked-endpoint boundaries.

Automatic requests, priced confirmation and sale remain separate. The genuine
CS043 payload is the equipped backpack ITEM TEMPLATE ID, not an NPC ID. Its
original specialty_trader callback requires the actual backpack, class kind3,
static item price/origin specialty-zone/region and optional skill18458 admission. The lane's
NPC interaction-option chain has no specialty service branch, so an arbitrary
NPC interaction must not synthesize one. Refund's registered adapter requires
strict numeric input even though its body computes from actual NPC/backpack
context rather than the supplied refund. These providers need their source
contracts before the corresponding original confirmation and actions are bound.

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
