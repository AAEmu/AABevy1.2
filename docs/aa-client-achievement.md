# Original achievement UI

The client binds the original eight `x2ui/achievement` scripts to the read-only
compact database and received `SC1fc..200` achievement events. There are no
synthetic live completions or reward claims. The screenshot example encodes
explicit offline receipts and labels itself as a fixture.

`hud/achievement/catalog.rs` loads all 1,743 descriptors in actual SQLite row
order, 3,558 objective rows and kind9 character-record links. A missing character
record still contributes to the completion threshold; only resolved kind9
`value1` fields populate the dense objective-ID table. English names come from
`localized_texts`. The available database has no English achievement summary or
description translations: their original Korean text is retained. These fields
contain no `@` formatter tokens, so the native39435050 formatter leaves this
catalog's literals unchanged. Runtime-context token expansion is not implemented.

The shared `aa-net::Achievements` cache admits received batches with the native
50-row storage limit and first-wins semantics. Scalar receipts require a known
descriptor and received initial-config byte7 bit7. The same bit supplies the
original `X2Player:GetFeatureSet().achievement`; an unknown feature configuration
does not inherit the fake UI's enabled flag. Catalog binding happens before
network polling when the background database is ready. Earlier scalar receipts
are not replayed retroactively.

The native getter/table contract follows390598f0: f32 numeric conversion, signed
amounts, unsigned IDs/rewards, zero-progress defaults and six-field local-time
dates. Unknown descriptors still return current/complete/tracing, plus a date
when a received unknown-ID row has a nonzero timestamp. Timestamp0 omits the
date. Invalid nonzero CRT inputs retain the executed native sentinel1899/0/-1;
valid inputs use the runtime's local timezone and daylight-saving rules.
Icon lookup returns the native `Game\ui\icon\` prefix and `_bad.dds` fallback;
a child without an icon inherits its parent's icon when that parent exists.

Category counts and filters use native active/hidden/completed rules. All and
incomplete rows use own/parent priority and ID ordering; completions compare
raw timestamps as unsigned64-bit values descending; progress ratios divide
as f32 before category tie-breaking. All38 real category/all+incomplete results
are checked against executions of the original3900ef30 list helper and full
native sort. The source agent also checked the actual category comparator graph
for cycles. Malformed custom zero-threshold progress catalogs are explicitly
refused: the original non-strict comparator and MSVC partition permutations are
not replaced with a guessed order. Such rows do not occur in this catalog.

Local tracing has six ordered slots, zero holes, duplicate rejection and no
invented catalog/feature/completion guard. Removing the first matching slot
preserves holes; removing0 can find an existing empty slot and still emit the
native update. Recent completions retain duplicates, remove only the first
matching marker, and clear with an update even when already empty. An admitted
nonzero completion writes progress, appends its recent marker, emits
`COMPLETE_ACHIEVEMENT` while it remains traced, removes the first tracing slot,
emits an extra update when found, then emits the outer update. No receipt is
folded twice to produce these callbacks.

Character/connection changes clear the app-owned transient view to prevent data
from leaking to another character. Native tracing belongs to type5 UI data.
The adapter consumes the lane's genuine pushed responses; it does not invent a
type5 request. Hydration requires the active selected character, although the
original callback ignored the response's character ID. Six child values use
the executed native decimal conversion, including wrapping overflow and ignored
trailing text. The first case-sensitive `tracing_achievement` child is updated
in place, preserving other subtrees, duplicates and sibling order.

Trace changes update that local tree immediately. A valid original UI leave
request encodes the tree and queues `CSSaveUIData(type5)` before `CSLeaveWorld`.
The first `version` attribute becomes `1`, or is appended when absent. Neither
the socket write nor the subsequent leave response is a save acknowledgement.
The selected character must also match the actual HUD spawn identity. No save
is attempted before an admissible response establishes the complete tree.

The optional native Data codec refuses unsupported comments/metadata/block
strings atomically, so those opaque responses cannot be partially rewritten.
Positive lengths must equal the UTF-8 byte count or byte count plus the implied
NUL; a UTF-16-unit count can otherwise truncate valid later nodes in the original
parser. Nonpositive lengths preserve the previous tree. Empty payload/length1
is an explicit safe app empty-state boundary: the original parser's observed
partial garbage node is not reproduced. Unsupported or unsafe responses disable
saving until a complete supported response arrives. The backend's opaque
receipts remain untouched.

Live achievement settlement and save/re-entry are unverified while the shared
Bevy lane's game connections stall. Login/world-list availability is not treated
as successful in-world validation.

## Checks

The full client suite passed237 tests/37 existing or optional probes ignored
(25.29s) with the UI owner's native automatic row geometry. This includes all38
executed list results, native comparator goldens, all1,743 getter rows,
invalid/local dates, completion callback ordering, safe type5 hydration,
sibling/version preservation and captured save-before-leave command ordering.
The separate original-script physical tracing/remove/re-add, completion, detail
and Back probe passed (2.69s). It waits for the original opening scale animation
before checking exact mouse hits.

All four1920x1080 original-script pages were captured and inspected under
`shots/hud/achievement/`: list463.7?509.8FPS, overview401.8?474.8FPS,
tracing413.7?477.0FPS and detail448.3?530.1FPS. The native440px six-row list
uses73px rows with the two-pixel remainder unused; no fixture height override
is present. The list page is selected through its original script function, so
the left navigation retains the Summary highlight in that fixture capture.
There were no warning/error log entries, and every QA process closed itself.
The missing English summary/description data and live boundaries above remain
open; no matching real-client achievement reference capture exists locally.

The same main build reached the live `Lane-bevy` world list at294.5?414.8FPS,
with no warning/error entries. This verifies login/world-select only. The
verified playable alias is `target/debug/aa-client-play.exe`, copied from
`aa-client-20261018-achievements.exe` after confirming no client instance was
running. SHA256: `72AFD8DFECB752A08EC0A70141E4A2E76D594A4376E8190F2FC4B664C8ECBB26`.

The reproducible visual example uses `aa_dev::default_plugins` and `DevPlugin`:

```powershell
cargo run -p aa-client --example achievement -- --size 1920x1080 --shot shots/hud/achievement/list.png --shot-after 8
# Optional --overall, --tracing or --detail selects another original-script page.
```

Native-source execution artifacts and reproducible oracle scripts are under
`data/extract/aa-net/native-achievement-*`; the checked-in client list/comparator
fixtures preserve their provenance. See `docs/aa-net.md` for packet/cache source
addresses, bounded native faults and the independent wire tests.
