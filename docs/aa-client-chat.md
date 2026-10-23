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
preserving the separate method field. AAUI.ChatMethod resolves its retained raw
tab ID through the current global model, saves that owner's method before the
selector display callback, and preserves its selected index. Unknown raw IDs
still display through aa-ui but do not write extras. Repeated method choices and
restoration use the same native save path. Tab append/update/remove use aa-ui's
source-verified lifecycle helpers, retaining other widgets, histories, handlers
and message scroll state. A newly allocated reusedID clears its old history.

GetChatOption exposes overAlpha/leaveAlpha; UpdateChatOption copies existing
values, replaces only numeric fields, saves locally, then refreshes native
owner-layer alpha. Raw options remain unclamped. Missing/wrong-kind fields
retain their old values; updating an unchanged pair still performs the native
idle reset. Message content stays fully opaque; metadata background alpha is
stored while the original message background DRAW alpha is1. Its background
uses the global dynamic .5/.7 pair; tab/divider decorations retain metadata alpha.

The eight window lock flags are saved independently as the locks key. Both
native IsLockedChatWindowByChatTabId and LockChatWindowByTabId resolve the raw
tab's owner rather than treating a tab ID as a window ID. Malformed numeric
arguments return zero values; valid unknown raw IDs return one false. The
setter accepts a boolean or a numeric nonzero flag, changes memory first,
applies AllowTabSwitch(!locked) and UseResizing(!locked) to an actual live window,
then returns the local save result. An absent live owner therefore returns false
after changing its in-memory byte, without writing the file; a later valid save
includes all eight bytes. Invalid/missing saved arrays reset all eight to false.
Restored flags apply once to each live widget before the toolkit frame; explicit
unchanged unlocks still cancel the native warning. The app JSON is its own storage
representation, with no native eight-byte binary import claim.

Locking preserves normal tab selection and right-click menus. An actual registered
drag attempt arms the original3000ms notification with400ms blink intervals and
prevents moving/resizing. It does not show the warning on an ordinary click.
Unlocked first-tab dragging uses the toolkit's native generic window movement;
other-tab splitting and reparenting remain open.

Native source evidence includes record parser39458870, manager record lookup
39503d20, validator39503df0, add39507e00, save-tabs39504680,
save-extra395044d0, local key39286690, selection395139b4..bb,
option update39508b90 and default/load395072c0. Complete native metadata,
lifecycle/selection and alpha execution artifacts are archived under
`data/extract/aa-ui/native-chat-*`; checked-in native_defaults.json preserves
palette/filter provenance and PE SHA256.

Remaining boundaries: provider-specific extra name filtering at39507e18,
window geometry persistence and other-tab splitting/
tearing off require their remaining toolkit/source integration.
Original physical right-click context dispatch is implemented in aa-ui, including
the protected rawID0 clear action without changing selection. Legacy fallbacks for the remaining
methods are not counted as implemented. Character-scoped writes preserve other
settings and do not commit pending graphics/options edits.

## Checks

All seven focused configuration checks pass, including physical original
Save/Cancel/Reset, complete palette restart, signed character keys, raw float
bits and six-character file-size admission. The restart check also types /p
through the original input, verifies method4 and selected3 on disk and in the
fresh stage, and proves unknown retained raw IDs leave saved bytes unchanged.
These seven checks pass again after the method hook integration. aa-ui separately
passes30 selected input/method checks,12 executed native method groups and an
inspected restored-input capture at517.4-566.5 FPS.
With lock integration, all nine configuration checks pass. They include the
original context Lock/Unlock buttons, blocked real dragging and the source-correct
first warning blink, persisted flags on a fresh stage, numeric/bool adapters and
the missing-live-window save edge. Full client251 passed/38 optional probes
ignored in70.48s. aa-ui separately passes37 selected lock/drag/resize/chat tests
and15 focused bool-adapter checks.
The strict native palette parser
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
At that milestone the same checked bytes were installed as target/debug/aa-client-play.exe
after verifying no client was running; the later crime build supersedes that alias
(see aa-client-crime.md). Full client245 passed/37 optional probes
ignored in57.34s; the separate original equipment-cooldown probe passed in2.05s.
The reproducible example is:

```powershell
cargo run -p aa-client --example chat_config -- --size 1920x1080 --shot shots/hud/chat-config/options.png --shot-after 8
# --style shows the original font/background/opacity page; --chat shows chat alone.
# --locked uses the original context Lock action, then periodically attempts a real drag.
```

The later lock milestone has five inspected captures in shots/hud/chat-locks:
locked306.4-450.4 FPS, its visible warning phase252.7-492.6, the corrected dishonor
panel398.1-505.0, jury tooltip258.4-281.0 and live world list360.2-412.1. Each
self-closes without WARN/ERROR/panic. The blink is sampled at7.4s for the warning
capture; its off phase at8s is also retained. Main
target/debug/aa-client-20261018-locks.exe has SHA256
6f8a31022ccdad799604d2282f0a75b887ab395b281d9fd64d74514991a3e400 and
supersedes the playable alias after checking no client is running.

## Native full reset and independent extra loading

InitChatWindow now follows3945b33d ->394581f0 ->395083e0 ->39507410:
retire every actual chat window through aa-ui, clear shared history, delete the
current world/character's tabs/extra/locks/options, reload the manager model, then
call original OnResetAllChatWindow with zero arguments. The API returns zero Lua
values and ignores the callback's return value. The original callback recreates
the default All/Party/Trade window at430x350 with its bottom-left anchor. Identity
and the current option-default font survive; locks and opacity return to their
native defaults. Other characters/worlds, future unknown record keys and pending
option edits remain intact. A local save error is logged and reset continues;
the current process sees deleted keys, while a failed file replacement can leave
the old records on the next process's disk load. Missing/failed script callbacks
do not undo window retirement.
The four keys are absent before the callback. Creating the original window then
restores method0 through395107d0 ->39510390 and saves default extra info even
when the method is unchanged; that extra key is expected to reappear afterward.

The original All-tab settings exposes Reset Chat. Other tabs hide that control
and expose only Reset Tab, so physical integration checks open rawID0 rather
than overriding hidden controls. Direct programmatic reset of a separately
attached open top-level popup retains the documented native observer/geometry
lifetime boundary; destruction does not invent OnHide. The real settings flow
dismisses the original context popup on its outside press.

Native395072c0 loads tab data, options, locks and extras independently. In
particular, failed/missing tab data builds defaults through39507060 without
saving tabs, then still attempts extra loading at3950739c. The app now restores
saved selection/input method even if a user has only changed these defaults and
there is no tabs key. Invalid extra rows fall back to window0 selected0/method0
without discarding valid tab metadata; an admitted empty extra list stays empty.

The reset example is the existing aa-dev chat_config example with --reset.
Native toolkit execution provenance and its14-group reset oracle are in
docs/aa-ui.md and data/extract/aa-ui/native-chat-reset-*.

All12 configuration checks pass, including the real original All-tab reset
button, four-key deletion observed before callback, recreated default extra,
history clearing, font/identity retention, other-world/character/future-key and
pending-option preservation, fresh-stage defaults, and missing/failed callback
zero-result behavior. Full client255 passed/38 optional probes ignored in46.56s.
The period oracle and updated365-day guild-age check also pass in that suite.

Three inspected aa-dev captures in shots/hud/chat-reset self-close without
runtime warnings/errors: original reset at1920x1080 runs398.0-456.6 FPS,
1280x720 runs277.0-392.0, and the live world list269.2-325.4. Both reset images
show only fresh messages and the restored All/Party/Trade tabs. They establish
local original-script behavior; live in-world settlement remains unverified.
The reproducible runner is data/extract/aa-client/chat/reset-qa.ps1.

Checked main: target/debug/aa-client-20261018-reset.exe, SHA256
6d90dcca0da162bb4700fcaa417cad16c2de1ba9d13c5136936e96f7c23afba3.
The playable aa-client-play.exe alias now has the same bytes, refreshed after
confirming no client process was running. The trial HUD bridge remains pending.

The subsequent forced-input integration calls the source-backed prepare helper
only when HUD.chat_open opens the actual chat edit, before prefill and focus.
Pending method11 is consumed there; clearing pending with -6 leaves the visible
method unchanged. Full Reset Chat clears pending at the manager reload point,
before original window recreation. All13 configuration checks pass, including
original chat-open prefill, persisted method11, clear-without-switch and reset
with a previously pending trial method. This is an input integration check;
the trial receipt producer is validated with the separate trial HUD bridge.

## Slash commands beyond the channel switches

The toolkit's chat input swallows the channel commands it knows (`/s`, `/p`, `/w Name`, `/1`...; see
`docs/aa-ui.md`). A line that still starts with a slash when it reaches `AAUI.ChatSend` goes through
`HUD.slash` (`hud/chat/commands.rs` + `commands.lua`) before it becomes a chat line (or an emote, `api.rs`).

The words are the client's own table, which the game database carries as `slash_commands` (171 rows; the
words are lower-cased, Korean and English spellings alike), `chat_commands` (action id -> chat type) and
`slash_functions` (the ids, with Korean descriptions). A `ChatCommand` row names a channel: `/ex` guild (7),
`/fa` family (9), `/co` raid leaders (10), `/tr` trial (11), `/pl` play (13), `/u` alliance (14), `/n` nation
(6), `/r` reply (-5) and the ones the toolkit has. A `SlashFunction` row names a function:

| id | words | does |
|---|---|---|
| 1 / 26 / 23 | `/invite`, `/partyinvite` `/pinvite`, `/raidinvite` `/rainvite` | `X2Team:InviteToTeam(name, isParty)`; without a name the targeted player |
| 2 / 27 / 24 | `/leave`, `/partyleave`, `/raidleave` | `X2Team:LeaveTeam()` (the party/raid ones only in that kind of team) |
| 3 / 28 / 25 | `/kick`, `/partykick`, `/raidkick` | `X2Team:KickTeamMemberByName` |
| 4 / 5 / 6 | `/ccreate`, `/cjoin`, `/cleave` | `CSJoinUserChatChannel` (create / join, name and password), `CSLeaveChatChannel` (the channel's identity, remembered from `ChatChannelJoined` in `HUD.user_channels`) |
| 7 | `/roll [sides]` | `CSRollDice` (100 sides by default); the answer `SCDiceValue` is the original `CHAT_DICE_VALUE` line (`msg_dice_value`) |
| 12 / 13 / 41 | `/exinvite`, `/exleave`, `/expedition_kick` | `X2Faction:InviteToExpedition` / `LeaveExpedition` / `KickFromExpedition` |
| 15 / 16 / 42 / 43 | `/block`, `/unblock`, and the Korean words of friend add / delete (the table has no English ones) | `X2Friend:BlockUser` / `UnblockUser` / `AddFriend` / `DeleteFriend` |
| 18 / 19 / 21 / 22 | `/addfamily Name Title`, `/family_leave`, `/family_kick`, `/familytitle Name Title` | `X2Family:Invite` / `Leave` / `Kick` / `ChangeTitle`; a missing title shows `locale.family.invite_syntax` / `title_syntax` |
| 40 | `/report Name` | the skill of the row (20256) on the named unit in view |
| 46 | `/target Name` | selects the unit of that name in view |
| 8, 9, 17, 29-39, 44, 45, 47-51 | web cast, journal, family list, languages, jury, watch, play music, newbie guild, nation | no effect here (swallowed, not sent as chat) |

Teams, friends, family and guild use the same `X2*` methods as the original windows, so their checks and
answers are the windows'. `/r text` answers the last received whisper (`HUD.last_whisper`, set from the
incoming whisper channel); with none, nothing is sent. Unknown `/words` are still sent as chat lines (emotes
first: `api.rs`). `ui_texts` mention `/invite`, `/report Targetname`, `/addfamily name title`, `/familytitle
name title` (the English client spells the family functions that way).

Open: the exact native usage/error texts of each function and the behaviour of the language and watch
commands (not in the database); see the native executor notes below if they were recovered.

## Saved window positions: `X2:Get/Set/SaveCharacterUiData`

Every window with `AddUISaveHandlerByKey(key, window)` (`x2ui/ui_management.lua`) saves
`ui_bound_<key> = { bound = {x, y, width, height}, screenResolution = {x, y} }` through
`X2:SetCharacterUiData` when the toolkit calls its `OnMovedPosition` (a window moved or resized with the pointer;
`aa-ui` `stop_moving`) and applies it again at the next `OnScale` / `ApplyLastWindowBound` (the chat windows are
`chatWindow[N]`). `uiOptionDatas`, the action bar layout and similar keys use the same store.
`hud/chat/ui_data.rs` keeps the values in the account settings file under the character's key (`chat[<world>:<id>].ui`),
Lua tables of numbers, strings and booleans as JSON (integer keys of non-sequence tables are tagged), written two seconds
after the last change and when the game ends. The quest notifier's reference position of the old fake remains the answer
for that one key when nothing was saved.

Recovery audit (2026-10-23): switching owners now flushes a pending UI-data save under the old owner's key,
clears its in-memory values, and loads only the newly selected owner's record. A disconnect clears the bound
owner in the same way. The regression checks two characters with different bag positions and returning to
the first character; no environment variables or shared account files are changed by the check.
