# Native local chat configuration

The original chat UI uses a local native manager, not game-server chat-tab packets.
`hud/chat/config` now owns its eight global raw tab IDs, ordered window rows,
metadata and independently saved numeric selection/input-method extras. The app
maps native local storage into the existing account settings JSON, further keyed
by selected world byte and signed character DWORD (`world:character`). There is
no native LevelDB import or binary-format compatibility claim. Genuine
ReadyToConnect/CharacterSelected/PlayerSpawn identity is required for writes;
offline fixtures without that identity stay ephemeral.

The original en_us script sets the default font to15. Initial IDs0/1/2 use the
original localized All/Party/Trade names, complete native200-colour palette and
41/10/6 enabled filters. Hidden filter60 remains in General. New user tabs use
base metadata with no enabled filters even when reusing rawID1 or2. The first
free globalID0..7 is appended to its window's current row order; saved tab
records are in ascending rawID order, so a restart can reorder reused IDs while
retaining the numeric selected index. No rawID-based selection repair is added.

GetChatTabInfoTable and GetDefaultChatTabInfoTable return the exact six native
fields; unknown IDs return zero Lua values. Lua exposes only filters/colours2..59.
Updates require id/name/fontSize and bgColor/filters/filterColors tables. Missing
or wrong-kind optional background channels and filter booleans retain the old
values; an existing palette row needs all four numeric components or the whole
update fails. Fields outside2..59 survive the copied-record update. Numeric
inputs use nativef32/CVTT signed conversion. Raw RGBA values are preserved as
f32 bit patterns in app JSON, including signed zero and NaN payloads. Each
palette uses one compact hex string (200RGBA rows, eight hex digits per float),
so six characters with eight tabs each stay within the existing account-file
size limit.

AddNewChatTabByUser applies the shared native chat-name policy2..10 and succeeds
only after the actual live window accepts the new row. Direct Rename uses the
weaker native record rule1..10; the original dialog itself still enforces2..10.
Delete removes the widget before the protected rawID0 model guard, including
its native false-result edge. Normal deletion keeps/clamps the live numeric
selection without manufacturing OnSelected callbacks. Empty model windows
retire on the next update. ClearChatContentByUser clears only the actual target
message frame and returns the native boolean result.

AAUI.ChatSelection saves the numeric index before original selection callbacks,
preserving the separate method field. Tab append/update/remove use aa-ui's
source-verified lifecycle helpers, retaining other widgets, histories, handlers
and message scroll state. A newly allocated reusedID clears its old history.

GetChatOption exposes overAlpha/leaveAlpha; UpdateChatOption copies existing
values, replaces only numeric fields, saves locally, then refreshes native
owner-layer alpha. Raw options remain unclamped. Missing/wrong-kind fields
retain their old values; updating an unchanged pair still performs the native
idle reset. Message content stays fully opaque; metadata background alpha is
stored while the original message background DRAW alpha is1. Its background
uses the global dynamic .5/.7 pair; tab/divider decorations retain metadata alpha.

Native source evidence includes record parser39458870, manager record lookup
39503d20, validator39503df0, add39507e00, save-tabs39504680,
save-extra395044d0, local key39286690, selection395139b4..bb,
option update39508b90 and default/load395072c0. Complete native metadata,
lifecycle/selection and alpha execution artifacts are archived under
`data/extract/aa-ui/native-chat-*`; checked-in native_defaults.json preserves
palette/filter provenance and PE SHA256.

Remaining boundaries: provider-specific extra name filtering at39507e18,
InitChatWindow full reset, native window locks/tab dragging/tearing off,
and input-method saving require their remaining toolkit/source integration.
Original physical right-click context dispatch is implemented in aa-ui, including
the protected rawID0 clear action without changing selection. Legacy fallbacks for the remaining
methods are not counted as implemented. Character-scoped writes preserve other
settings and do not commit pending graphics/options edits.

## Checks

All seven focused configuration checks pass, including physical original
Save/Cancel/Reset, complete palette restart, signed character keys, raw float
bits and six-character file-size admission. The strict native palette parser
also exposed the toolkit's prior sequence-only Drawable.GetColor result;
aa-ui now supplies its native named r/g/b/a table, allowing the original Save
handler to succeed without weakening native validation.

Seven1920x1080 aa-dev captures in shots/hud/chat-config were inspected:
options252.8-259.7 FPS, style270.2-301.3, chat264.3-469.7,
received equipment cooldown206.9-309.8, stopped228.8-283.2,
ready385.9-458.4, live world list236.6-410.7. Each self-closed with no
WARN/ERROR/panic. These original-script fixtures establish local configuration
and received cooldown behavior; they do not establish live in-world settlement.
The live game-connection stall remains open. Full client regression results are
recorded in the parity ledger.

The checked main binary is target/debug/aa-client-20261018-chat.exe,
SHA2567bd38357598faa45dbfe23d3857392ecca6bf9837ce4fd6df6be102c5bd21a30.
The same checked bytes are installed as target/debug/aa-client-play.exe after
verifying no client was running. Full client245 passed/37 optional probes
ignored in57.34s; the separate original equipment-cooldown probe passed in2.05s.
The reproducible example is:

```powershell
cargo run -p aa-client --example chat_config -- --size 1920x1080 --shot shots/hud/chat-config/options.png --shot-after 8
# --style shows the original font/background/opacity page; --chat shows chat alone.
```
