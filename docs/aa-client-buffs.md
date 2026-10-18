# Original self-buff cancellation

The client binds the unchanged original player-frame right-button callback to
`X2Ability:CancelPlayerBuff`. Its argument is the one-based native Good-list
ordinal. The selected record supplies the received instance index, including
valid zero and unsigned maximum values. It is not a buff template ID.

Known player and target icon lists now use `Buffs.native`, preserving native
repeat folding and stable group order. Raw lane lists remain the display
fallback while native history is unknown; cancellation is refused in that
state. A displayed row must still match the current native ordinal and instance.
The queued request captures the real character/runtime actor, template, index
and receipt lifetime; changed ownership, replacement, rekey or disconnect drops
it before the typed writer. The session independently checks its actual selected
header, in-world state and self actor.

The number adapter follows successful numeric ScriptAnyValue type4, f32
CVTTSS2SI and wrapping subtraction. Wrong types, missing/out-of-range rows and
missing descriptors return zero Lua results silently. The actual static
`buffs.system` flag emits localized error224, `CANNOT_REMOVE_SYSTEM_BUFF`, and
sends nothing. Generic `can_not_remove` or owner-only flags are not substitutes.
No optimistic list erase occurs: local successful-write events leave the icon
in place, and genuine `BuffRemoved` updates the received manager and display.

Native method/lookup/constructor source and seven executed oracle cases are
archived under `data/extract/aa-net/native-remove-buff-*20261018.*`; the owning
network docs retain exact addresses, framing and explicit external mocks. The
network manager also preserves native group capacity, first-index stable erase,
descriptor admission and unknown-history boundaries. Full native tick/frame
lifetime, stats invalidation and automatic reason9 removal remain separate work.

All five focused checks pass in 2.18s: numeric/result/system contracts including
valid instance0, folded display/rekey and receipt-only removal, unknown/stale
mapping and ownership refusal, and the actual HUD scheduler/typed writer with
disconnect/selection capture disposal, and actual original physical left/right
clicks, localized system-buff warning, folded indices and stable icon shift after
an explicit removal receipt. The fixture obtains the original local player
frame through `ADDON:GetContent(UIC_PLAYER_UNITFRAME)`; its uppercase alias
exists only in the fixture. The full copied client test executable passes
281 checks / 38 existing optional probes ignored in 57.26s; its log is
`data/extract/aa-client/trial/buffs-full-tests.log`. Main and the aa-dev `buffs`
example build. Five 1920x1080 captures under `shots/hud/buffs/` are inspected:
folded icons (303.2–340.6 FPS), system icon refusal (273.5), received removal
(320.5–375.5), live world select (243.7–336.8), and `system-visible.png` with the
original red "Can't cancel this effect." text (487.8 FPS log sample). All five
processes self-closed without warnings, errors or panics. The extra warning shot
uses a one-second delay because the original message has a two-second lifetime;
the original scripts and production timing are unchanged.

Fresh game-entry attempts remain separate evidence: HUD-only `bevysol61hud`
receives an empty character list, while the previously documented
`bevycreate`/`Bevylook1` attempt times out waiting for the game server at
2026-10-18 16:51:24 UTC, before any character selection/spawn. Both logs are
retained in that capture directory; no world-entry image or live removal is
claimed. No server service was restarted or changed.

The inspected main copy is `target/debug/aa-client-20261018-buffs.exe`, and
`target/debug/aa-client-play.exe` is refreshed after verifying the alias is not
running. It includes this cancellation adapter and native independent terrain
normal UVs. The later factual decode-loss native-history invalidation needs the
next build; no unbuilt change is attributed to this copy.
The main copy and playable alias SHA256 is
`505EBE6AAB2B00127C0BC9602169BB0735D9C6DD5F3CF7DD25273B46E07F817D`.

The aa-dev `buffs` example uses real Inspire127, Experimental Glider1029 and
Bulwark Ballad1000 descriptors over explicit received fixtures. It has no network
session. `--remove` physically clicks the original first icon, checks the typed
capture and then applies a separately explicit removal receipt; `--system`
physically clicks the system-buff icon. Neither mode claims a server settlement.

This slice retains the earlier tooltip flag/stack representation. Native getter
tracing identifies a separate follow-up: stack rules2/3 use positive wrapping
absolute charged values, rule4 uses a positive runtime stack, other rules expose1,
and the field is omitted without bit0x10 or a positive selected count. That must
be integrated with the original omitted/default numeric flag behavior rather
than merely removing the old display clamp. Full tick/stat and metadata flag
parity is not claimed by this cancellation milestone.
