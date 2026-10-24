# Appearance conversion and existing-character refresh

The client uses the original fusion and Skinize windows over received item records.
Conversion, Cleanup and Skinize submit the guarded `aa-net::ItemLookRequest` operations;
only subsequent inventory receipts change flags, counts or image templates.

## Original entry and item targeting

An NPC with the catalog's `look_converter` flag offers skill **23478** in the existing
service list. The selected service emits the original
`NPC_INTERACTION_START("look_converter", "", npcId)` event. Original Lua opens interaction
window **16**, named `changeItemLook`, and existing distance/window checks close it.
The retained native map at `393c17a6..393c17cf` binds the runtime scalar at `39ac79ec`
to `look_converter`; the pinned read-only runtime dump stores 23478 there. This is
static source/data correspondence, not a native live-NPC execution claim.

Fusion uses the original 13 `X2ItemLookConverter` methods, ordered catalog recipes,
physical BAG keys and exact item IDs. The shared pending set preserves explicit
unlock behavior; it is not a reference count. Unknown native BAG history retains
the older local shop/mail lock fallback. Tooltip construction retains raw grades,
detail tags and the distinction between base gameplay properties and image visuals.

Using Fusion Alembic or the Cleanup consumable runs existing item admission before
entering private item-target mode. It does not send an ordinary unit-target cast.
Skinize opens original task 19 with exactly `itemInfo`, `scrollType` and
`scrollGrade`; its target unlocks before either confirmation callback. Cleanup
submits inline. Both paths capture complete source/target records and recheck them
before transport. Task 19, the held drag cursor and item targeting have separate
lifetimes.

The per-release toolkit callback runs after each completed GUI release branch.
Item targeting checks the post-layout GUI hit, retains ownership after a target
callback clears the mode, and handles both held-item drop paths. Retained ownership
prevents the same input from falling through to item destruction or world selection.
Fresh targeting survives its entering release and ages only on an eligible UI
frame. Existing reticles share modal ownership; orbit, continuous movement and
independent task dialogs retain their existing behavior. The mapped `open_config`
action performs the same cancellation sequence as physical Escape.

The private admission cache refreshes changed inventory/action/skill state and the
own buff slice before original callbacks. It does not retain an ECS `World` pointer
or introduce a second production mouse state. Lua callbacks run after HUD borrows
are released. Discrete world actions follow release coordination in the Update
schedule; the full normal-login and in-game plugin schedule is covered by a smoke
test.

Native evidence and boundaries:

- [Converter APIs and wire contract](aa-net.md), including guarded item captures.
- Ignored original UI research:
  `data/extract/aa-ui/look-conversion-20261023/CONTRACT.md` and its `followup/` reports.
- Root static NPC proof:
  `data/extract/aa-client/look-npc-entry-20261024/provenance.json`.
- The protected original planner `39396e20`, complete physical native input ordering
  and inactive enchant membership remain outside the executed boundary. Application
  admission is not a successful execution of that planner.

## Refreshing an existing actor

Own equipment comes from complete native item records; remote equipment comes from
the received unit state. A visual key includes the character identity/customization
and each equipment slot's base template, effective image and received dye. Grade,
wear and stack changes do not rebuild identical geometry.

At most two preparation jobs run concurrently. Results must still match the actor,
source generation and visual key immediately before application. Failed keys retain
the previous complete appearance and retry only after their source changes. Remote
initial builds record the equipment snapshot actually prepared, allowing a later
receipt to trigger refresh rather than incorrectly acknowledging unseen geometry.

`aa_anim::character::replace_character_parts` commits prepared body/facial parts on
the same model root and shared bones. Animator or parked `Asleep` state, facial
clocks, layers and root placement survive. Carried weapons are replaced separately;
gameplay wield classification comes from the base item. Replacement applies cached
shadow policy, reconciles staff-side state, attaches weapons before held-state
updates, and discovers/retires costume payloads in the same update.

Nonzero received dyes resolve through `GameDb::dye_color`. An unknown color fails
preparation and retains the old appearance; zero keeps the authored default.
The character replacement/facial contract and its independent validation are in
[aa-anim.md](aa-anim.md).

## Reproduction and validation

The offline examples use `aa_dev` and original assets:

```powershell
cargo run --locked -p aa-client --example look_conversion -- --case skinize-task --size 1280x720 --shot shots/aa-client/skinize.png --shot-after 8
cargo run --locked -p aa-client --example look_conversion -- --case confirm --size 1280x720 --shot shots/aa-client/fusion.png --shot-after 8
cargo run --locked -p aa-client --example gear_refresh -- --before --size 1280x720 --shot shots/aa-client/gear-before.png --shot-after 12
cargo run --locked -p aa-client --example gear_refresh -- --size 1280x720 --shot shots/aa-client/gear-after.png --shot-after 12
```

The two gear controls receive identical updates at a fixed camera and pose; only
refresh application is disabled in the before control. The inspected final pair
differs at **22,040 pixels** and checks actor/model-root, parked-animation and facial
continuity. It shows a hood/image weapon, received red-to-purple dye and costume
payload retirement. Shared-machine FPS was **256.4–294.7 before** and
**270.5–412.3 after**; this is not an isolated performance comparison.

Final full client validation passed **702 tests**, with 67 existing ignored tests.
Post-style checks passed all targets plus 28 appearance, five gear, Escape and full
plugin-schedule regressions. The merchant/NPC-service group passes all ten tests,
including original Lua open/close behavior. Adding the catalog flag exposed an
outdated in-memory merchant test schema; it was updated, and the full suite then
passed. Final logs are `appearance-client-full5-20261024.log` and
`appearance-npc-store-tests3-20261024.log` under `data/local/resume/`.
The NPC-aware main client also built successfully; its immutable live-QA copy and
hash are recorded in `appearance-final-npc-caller-evidence-20261024.json` there.

The single client Clippy attempt failed because the existing `skill_coverage`
example lacked its `flash` module. That include was repaired, new-code style
suggestions were resolved, and all-target checking/focused tests passed afterward.
Clippy was not rerun, so no successful final Clippy result is claimed.

Ignored logs, hashes and image measurements are retained in
`data/local/resume/appearance-{input,final-fixture,final-caller}-evidence-20261024.json`.
All four final UI/gear captures exited successfully with clean shader/runtime logs
and were inspected. These fixtures supply received records; they do not prove a
server operation, native graph/FIFO behavior or complete native scene parity.
The isolated server and real-client appearance validation remain a separate step.

## Renderer boundaries

The nested `LodIllumMaterial` controls pass 18 exact full/half-size image equalities,
and the rebuilt offline sea caller renders successfully. These are regression
checks, not native pass-correctness evidence. The original pass-routing audit shows
that the current 15 masked Illum rows require diffuse-alpha depth/normal coverage,
successful-Z-dependent forward selection and separate motion coverage. Their
historical composed-alpha main/prepass equality does not close that work.

Raw `Texture.attributes` preservation is complete. Sampler resolution, renderer
pass integration and the vegetation local-wind cache boundary remain held for
their separately documented prerequisites. Global pipeline counts do not certify
coherent screenshot readiness.
