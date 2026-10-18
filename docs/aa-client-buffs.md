# Original buff getters and self-buff cancellation

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

The count projection uses rules2/3 to select wrapping absolute
charged values, rule4 selects runtime stacks, and other rules select1. Only
positive selected counts are exposed, after native signed f32 conversion.
Zero/negative counts omit the field rather than displaying a fabricated1.
The static loader retains the actual stack_rule_id; no charge-to-stack mutation
or inferred history is added. Twelve executed original numeric branch goldens
pass alongside the prior six buff/catalog/control checks. The original-stage
badge/SCBuffUpdated check also passes: actual Insulating Lens1011 shows x5 from
charge−5, Inspire127 omits a zero stack, and rule1 Bulwark Ballad1000 remains1
despite received stack77. Signed updates clear/restore x3 and clear nonpositive
counts while all three icons remain. All eight focused checks pass in 3.28s.
The full copied client suite passes 283 checks / 38 existing optional probes
ignored in 62.57s (`data/extract/aa-client/trial/counts-full-tests.log`). Main and
the count example build. Both 1920x1080 captures in `shots/hud/buff-counts/`
are inspected: the original x5 charge badge at 328.6–463.0 FPS, and real lane
world select at 336.0–399.2 FPS. Both processes self-closed without runtime
warnings, errors or panics. The count example has no network session.

The latest inspected copy is `target/debug/aa-client-20261018-buff-counts.exe`;
the idle playable alias is refreshed from it. This newer build also includes
native ordinary-slot edge artwork, factual buff-history invalidation on decode
loss, and signed heightmap projection/normal frames. Subsequent completion-effect
or other worker changes require another build.
Current main-copy/playable SHA256:
`9DE983213ED1BE99446FB33AFF8285196F04E32E8265083C37192709FD4A3297`.

At the numeric-count milestone, the getter's broader flag representation was
still separate. The subsequent adapter below closes argument and field shapes;
full tick/stat, native actor-frame time advancement and caster-dependent
description formatting remain separate.

The getter now follows the executed original UnitBuff/UnitDeBuff argument and
optional-field contracts. Unit aliases require a Lua string, and ordinals use
strict numbers converted through native f32/CVTTSS2SI and wrapping subtraction.
Omitted flags select all (-1); explicit nil or wrong types select0. Missing
actors, ordinals and descriptors return zero Lua values. A missing descriptor
still occupies its physical Good/Bad position and count, preserving cancellation
mapping. Metadata category follows the static descriptor's kind, independently
of its received wire group, and uses original localization group33.

Flags1/2/4/8/16 select description/mine/duration/timeLeft/stack. Only exactly-1
adds name, path, buff_id, category and tipType in this app's script environment.
The mine field compares the received caster persistent ID with the actual local
character ID. Duration retains unsigned wire bits, including0, then rounds
through f32; timeLeft is absent for zero duration. Stack selection retains the
prior signed charge/rule behavior. The existing description provider still runs
even when its result is not exposed, as the native formatter does.

All11 focused buff checks pass in4.14s. Three added original-stage checks cover
default/explicit flags, malformed arguments and zero-result arity, physical
missing-descriptor ordinals, static versus wire kind, unsigned duration extremes,
f32 projection and omitted time/count fields. A physical pointer hover opens the
unchanged original buff tooltip (tooltip window7); its time-only refresh retains
the full name/description metadata, and pointer leave closes it. The aa-dev
buffs fixture adds --tooltip using the toolkit's normal virtual pointer path.

The existing HUD Instant countdown is explicitly retained as a compatibility
clock; this slice does not claim the original actor-frame elapsed producer.
Existing token expansion, local-stat/caster providers and unit alias resolution
also retain their documented limits. The archived native formatter/parser/number
oracles establish further source contracts but do not make this app's entire
description provider equivalent or port original unsafe buffer behavior.

The full copied client test executable passes286 checks /38 existing optional
probes ignored in27.36s (data/extract/aa-client/trial/getters-full-tests.log).
Main and buffs example build. Three1920x1080 captures under
shots/hud/buff-getters/ are inspected: original Inspire tooltip with category,
formatted description and live time-only refresh (296.3-324.7 FPS), signed charge
badge (279.2-363.2), and actual lane world select (244.0-310.6). All processes
self-close without script errors/WARN/ERROR/panics. The tooltip's authored brown
frame, category/title/icon and highlighted text are compared with
reference/ingame/tooltip_buff/real.png; the reference contains Patron and a real
world scene, while this explicit fixture contains Inspire over the HUD-only
background. No same-buff pixel equivalence or live world-entry claim is made.

Latest inspected main is target/debug/aa-client-20261018-buff-getters.exe;
target/debug/aa-client-play.exe is refreshed only after checking the alias is
idle. Both SHA256:
8FC2D3187828B755DCFECDBE969B1A61CF6F0AEFA6865145576E0D0BA33551EE.
This build includes native ordinary-slot completion artwork, BC1 terrain normal
XYZ preservation and SC09d/e socketing result readers. Subsequent SC09c/f, raw
cooldown setter and triangle layer-weight work requires another build.
