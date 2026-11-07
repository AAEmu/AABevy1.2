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
No SET_BOUNTY listener is present in the inspected original 1.2 usertrial/chat
scripts, and the popup-menu proc is dead code; ShowSetupBountyWindow exists
separately. The Bevy shim supplies that event-to-window connection; the adapter
itself invents nothing. The original empty bulletin, the standalone setup
function and the received-event observers are checked directly.

Nonempty original bulletin rendering is repaired in the Bevy client only. Executing the unchanged
bytecode with a real nonempty getter table fails at bounty_bullet.alb:115:
`attempt to index global 'listCtrl' (a nil value)`. The disassembly
(`dis/bounty_bullet.txt`, proto `main.0.4` pc15/pc20/pc21) reads the nil globals `listCtrl`,
`bountyRecords` and `index`; the loop itself iterates the uppercase `BOUNTY_RECORDS` upvalue.
The reward button (proto `main.0.1` pc0/pc3) reads the unset globals `curSelected` and
`bountyRecords`, while the file defines uppercase local `CUR_SELECTED`/`BOUNTY_RECORDS`. This is an
executed bytecode failure, not a decompiler discrepancy. The aa-ui audit checks all 961 original
bytecode files: no script writes `listCtrl`, `bountyRecords` or `curSelected`. The native child-binding
oracle writes the parent table field; pagelist also stores `frame.listCtrl`. Global `index` has
unrelated writers and is not globally absent. The screen is loaded and reachable, making this an
original asset defect rather than an unused-screen or missing-toolkit-global assumption. See
`docs/aa-ui.md`. The stock 1.2 client is unchanged and still fails as above.

The Bevy repair is `crates/aa-client/src/hud/trial/bounty.lua`, installed lazily and idempotently
from `bounty.rs` before `TOGGLE_BOUNTY_BULLETIN`/`SET_BOUNTY` and from the trial fixture:
`userTrial`'s metatable `__newindex` intercepts only the `bountyBulletWnd` assignment (an existing
metatable is chained, the value is rawset) and patches each new window. The `FillLeftFrame` wrapper
keeps the selection and records the original already receives and binds exactly the two globals the
reward handler reads (`curSelected`, `bountyRecords`); nothing else is invented. `FillData` is
replaced with the original's page-list behaviour on the real widgets: `DeleteAllDatas`,
`listCtrl:ClearSelection`, then one `pageListCtrl:InsertData(row, column, tostring(record[field]))`
per record with columns 1..5 = record[6], [2], [3], [7], [5] (`dis` pc15-69). The LCCIT_STRING key
column gets the same per-cell setter the LCCIT_WINDOW columns receive, because the original FillData
call that would have written it also used the nil `listCtrl` global. The original FillData ignored
its page argument and the PageListCtrl's `UpdatePage` does the paging, so the shim loops over all
records too. UIParent's `SET_BOUNTY` handler calls `ShowSetupBountyWindow(id, name, money)`, matching
the native event's argument order; the shipped scripts have no SET_BOUNTY listener. The original
FillLeftFrame body indexes its argument with its own CUR_SELECTED upvalue; the wrapper passes a view
that resolves every key to the selected record, so the detail card and the reward button follow the
same row.

The popup-menu entry (`popup_menu_proc` proto `main.18`) stays out of scope: its closure is held in a
local that no later closure captures, it is stored in no global, and no portrait/menu entry reaches
it.

`LocalizeUiText` keeps the native number types for record fields 11/13. The original 1.2 engine logs
`Wrong parameter type ... parameter 4` for those two arguments (log-only, `CSS:8090-8105`); the Bevy
`LocalizeUiText` converts Lua numbers with `lua_tostring`, so the history labels fill without a
script error. The native types are not changed.

Seven focused bounty checks pass, including the nonempty rows, the selected-row reward request,
`SET_BOUNTY_DONE` money refresh, hide/reopen re-patching and `SET_BOUNTY` opening the setup window.
The trial filter runs 18 passed/1 ignored. The new rows capture is `shots/hud/bounty/rows.png`,
inspected at **484.2-511.3 FPS**: it shows the authored Wanted poster, both received rows
(7 / Wanted One / 52 / blank / 12345 and 12 / Wanted Two / 55 / blank / 987654), the original detail
card and "Offer a Reward". The trial example with `--bounty-rows` self-closes without WARN/ERROR or
panic. The earlier empty/setup captures remain local-only; no native bounty reference capture is
available, so no pixel-difference claim is made. Live in-world bounty settlement remains unverified.
