# Original bounty HUD adapter

The trial API has a bounty adapter over received Bounties:
GetBountyData returns the original two results, with fifteen-column rows ordered
by unsigned persistent ID. IDs/levels use unsigned f32; zone/counts use signed
f32; displayed money uses signed low32 bits. First and second verdict counters
retain native column13 not-guilty/column14 guilty despite the lane's reversed
labels. Raw amounts remain untouched in the received cache.

Race and faction use the same checked native providers as the trial records.
Location column7 remains empty until the actual spatial registry is available;
the received zone ID is not a substitute for the native current-world query.

RequestSetBountyMoney and SendBountyUpdate require actual Lua argument types,
retain native f32/CVTT conversions and first-NUL raw names, return no Lua values,
and capture the actual selected character. They add no invented cash, permission,
trial-role or pending checks. Session transport still requires its genuine
selected header and in-world state. Requests do not mutate received money.

Each complete-list callback exposes its own received map, including duplicate
and empty complete receipts. Callbacks queued during loading retain their
original snapshots. Existing actor updates fold the actual unit spawn/name/
removal receipts in order, so the callback uses the name and local flag that
existed at that receipt. An absent actor does not produce a callback. Selected
header money and runtime actor bounty money remain independent.

The adapter forwards native TOGGLE_BOUNTY_BULLETIN, SET_BOUNTY,
SET_BOUNTY_DONE and BOUNTY_PAID names/arguments. Original chat consumes Done/Paid.
No SET_BOUNTY listener is present in the inspected original1.2 usertrial/chat
scripts; ShowSetupBountyWindow exists separately. The adapter does not invent
that event-to-window connection. The original empty bulletin and standalone
setup controls are checked directly as well as through received-event observers.

Nonempty original bulletin rendering is unresolved. Executing the unchanged
bytecode with a real nonempty getter table fails at bounty_bullet.alb:115:
`attempt to index global 'listCtrl' (a nil value)`. FillData also reads lowercase
`bountyRecords` and `index`; its button reads `curSelected` and `bountyRecords`,
while the file defines uppercase local BOUNTY_RECORDS/CUR_SELECTED. This is an
executed bytecode failure, not just a decompiler discrepancy. Native global
providers are being audited before any compatibility repair. This adapter does
not supply invented globals or claim a working nonempty bulletin.

The active bevy lane's CS0ea returns an empty bulletin. CS11f/120 payment
handlers are absent. Natural bulletin interaction still requires the actual
native outer interaction eligibility; a known skill20558 alone is insufficient.
No automatic navigation, payment, permission or settlement is synthesized.

Five focused bounty checks and the six earlier trial checks pass. Full client
268 passed/38 existing optional probes ignored in93.60s. The original empty
bulletin display and standalone setup screenshots are inspected under
shots/hud/bounty/empty.png and setup.png. They show the authored Wanted poster,
headers, original money fields and setup controls; the empty bulletin's offer
button is not claimed as a working selection action. FPS was331.3-418.9 and
330.4-352.0 respectively. No native bounty reference capture is available, so
no pixel-difference claim is made.

The same main build's live world list is inspected at229.6-286.5FPS, showing
Lane-bevy Up. All three processes self-close without WARN/ERROR/panic. Reproduce
with data/extract/aa-client/trial/qa-bounty.ps1, or the existing trial example
with --bounty-empty / --bounty-setup and the usual --shot arguments. The latter
is deliberately a standalone setup-function probe, not a new event listener.

Checked main: target/debug/aa-client-20261018-bounty.exe, SHA256
4f4b9c159cd1eacb3048c1320358a416f33d94860e0b5f4fea1b3e2e81f70439.
The playable aa-client-play.exe alias has the same bytes, refreshed after
checking that it was not running. This build predates the in-progress native
dual-texture cooldown material; later peer work requires a fresh integrated
build. Live in-world bounty settlement remains unverified.
