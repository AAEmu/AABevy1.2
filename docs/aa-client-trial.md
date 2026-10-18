# Original trial HUD

The client bridge installs X2Trial.GetCrimeData, GetTrialType,
GetCrimeRecordsByPage, ConfirmCrimeRecords, CancelTrial and ChooseVerdict over
aa-net's received Trials. Task6 jury invitations, Task7 prison/trial choices and
Task8 final-statement choices retain their original received character/trial
captures. The unchanged original Lua decorators own buttons and layout.

Each callback temporarily exposes the snapshot taken at its native dispatch
point. Queued callbacks replay in order when the HUD becomes available, before
new receipts. This preserves Status before admission and Beautyshop before role
assignment. Tasks still perform native rejection and close callbacks during HUD
loading; missing visual decorators retain the accepted task for a later retry.
Other subsystem caches retain the existing independent HUD tracking; exact
mixed-subsystem callback ordering remains a broader integration boundary.
Session changes retire captures and queued replies. Replies are typed requests;
they do not acknowledge a choice or mutate received trial roles/counts.

Task7 duplicates return silently before creating another choice. Task6 rejects
the incoming invitation on ignore-option75 or an existing Task6, sending false
with the incoming captured trial ID. Task8 duplicates retain the existing task;
its false choice sends nothing. Closing pending6/7 runs their false callbacks
in that order, before the terminal trial Lua events. RulingClosed, Canceled and
final-statement closure use their distinct source behavior.

Task6 acceptance uses the login-start flag and the active main_world layer.
Native ConnectToServer39440bd0 sets gEnv+0x96 before its connection attempt;
the normal and developer shortcut login producers now record that start before
queuing Login. This flag is independent of success. The HUD checks its actual
in-game WorldInfo/WorldConfig agreement and exact main_world name. Preloaded
world metadata on the login screen cannot admit the invitation. Unknown or
other layers force a decline and error544. Unimplemented world/layer transitions
remain outside this checked presentation path.

The native 19-key crime table keeps signed f32 conversions except unsigned
sentence bits. Race keys use category47, factions use received expedition names
then actual static faction descriptors, and strings stop at their first NUL.
Kind counts refresh on DefendantData176 rather than on each record append177.
Records preserve insertion order in ten-row pages with native column7 doodad
name and column8 skill name. Column10 uses the local calendar and category102
year_to_minute. Unknown/uninitialized native fields are omitted; opening the
records/ruling UI is refused until its required received fields are coherent.

Record column9 remains empty: the actual current-world Group18 registry and its
production loader are not yet supplied. The existing HUD map-area heuristics
are unsuitable for this native 3D query. Invalid CRT dates, native uninitialized
output domains, oversized localized static buffers and unsupported layer
transitions remain explicit boundaries. Raw network fields remain retained.

Trial local ticks sample Unix seconds explicitly after the received packet
pump. Getters do not advance clocks. Native period decomposition is shared with
the already checked original period helper. Forced chat method11 is delivered
before SummonJury/Defendant adoption; the exact RulingClosed/local AudienceLeft
producers clear pending with -6. Canceled does not clear it. Actual chat open
consumes the pending method through the separately checked chat input bridge.

The isolated aa-dev example is:

```powershell
cargo run -p aa-client --example trial -- --size 1920x1080 --shot shots/hud/trial/records.png --shot-after 8
# --verdict, --waiting and --invitation show the other original controls.
```

These are explicit receipt fixtures, not live trial settlement. The shared lane
game-connection stall still prevents live in-world validation. No live trial
choices or bounty payments were sent by these checks.

All six focused client checks pass: callback snapshots/replay, exact getter
keys and pages, weak native controls and stale-session refusal, the real HUD
schedule/typed writer, original Task6/7/8 controls and visual retry, and original
records/verdict/waiting buttons plus the running original time label. The task
fixture supplies the actual static database before building the full HUD, as
the skill window waits for that initializer. Deferred invitation admission,
option changes before visual retry, duplicate captures, forced outside-world
decline, phase5 closure and same-ID re-entry are retained in this check.

The first four inspected1920x1080 captures in shots/hud/trial show records,
verdicts, waiting and invitations without warnings/errors. Observed FPS ranges
under concurrent compilation were207.3-336.4,78.4-278.7,145.2-337.0 and
313.7-354.5 respectively. All four processes self-closed. That first example
build shows static clock indicators; the subsequent example adds an explicit
fixture clock for the original time labels. No native trial-screen reference
capture exists in reference/ingame, so no pixel-difference claim is made.
