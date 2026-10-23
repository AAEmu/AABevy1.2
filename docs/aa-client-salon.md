# aa-client: the beauty salon (in-game re-customize)

Code: `crates/aa-client/src/hud/salon.rs` + `hud/salon.lua` (client), `crates/aa-net/src/salon.rs` (wire). The original window
(`x2ui/customizing/beautyshop/*.alb`) runs unchanged; this module is the native side it was written against.

## Protocol (server: `systems/services/beauty_salon.rs`)

| packet | opcode | body |
|---|---|---|
| `CSEnterBeautySalon` | 0x12e | empty |
| `CSExitBeautySalon` | 0x12c | empty |
| `CSBeautyshopData` | 0x12d | `u32` hair item template, `ModelParams` |
| `SCToggleBeautyshopResponse` | 0x1f1 | `u8` (1 in the salon, 0 out) |
| `SCCharacterGenderAndModelModified` | 0x1f3 | `u32` character id, `u32` hair item, `ModelParams`, `u32` 0 |

Paying takes one Salon Certificate (item 30811, "미용 이용권") from the bag; the server puts the buff `InBeautySalon` (6117) on
the player while in, applies the data and answers the modified packet to everyone around. The client then sends
`CSExitBeautySalon` (as the original does) and gets the toggle (0).

## How the original enters it (reversed, `x2game.dll`, image base 0x38FF0000)

* The salon helper NPC (`npcs` 13640, `npc_interaction_set_id` 12) offers the skill 22780 "Enter the beauty salon" (set 12 also
  holds 23469). Its right click is `CSStartInteraction`; `SCNpcInteractionSkillList` carries the skills. **Only the first skill
  of the list is dispatched** (`OnNpcInteractionSkillList` `FUN_391d8f80` -> `393be850` / `393be280` / `393bf310` / `393bf500`
  / `393c1280` / `393c0ca0`); there is no choice list in the Lua UI (`npc_interaction*.lua` only draws the function buttons).
* For the salon skill the native raises `X2DialogManager:RequestTaskDialog(DLG_TASK_CONFIRM_ENTER_BEAUTYSHOP = 46, {bypass})`
  (`FUN_393bfbd0`, vtable 399d3544; text `enter_beautyshop_body`, `..._bypass` when `bypass`). Its Ok handler `393bfab0`
  casts skill 22780 on the NPC, or - for the cash shop's `X2InGameShop:EnterBeautyShop` (`390608e0`, the *bypass* path) -
  sends `CSEnterBeautySalon` directly. `GenderTransfer` (`39060910`, `393bfa00`) is the sibling.
* `SCToggleBeautyshopResponse` -> `FUN_391e2160`: (1) enter `3931ef40` (the camera fades, then `TOGGLE_BEAUTYSHOP(true)`, event
  580 / 0x244), (0) leave `3931f0a0` (`TOGGLE_BEAUTYSHOP(false)`).
* The window (`CreateBeautyShopWindow`) is a full screen `modelview` widget with the character on a painted background
  (`ui/beautyshop/bg.dds`, FOV 20) plus the create screen's plastic (face parts) and cosmetic (hair, colours, decals) panels
  in "beauty shop style". Confirm opens the details window (which changes to apply, the certificate count `has/need`), its
  Confirm calls `PayBeautyShop`; Cancel -> `X2Customizer:TerminateBeautyShop`.

## What is built here

* `aa-net::salon`: `SalonCommand {Enter, Exit, Apply}` / `SalonEvent {Toggled, Modified}`, wired as `Command::Salon` /
  `Event::Salon`.
* `salon.lua` makes the pieces the window needs: `X2Customizer` (the counts, face targets, `GetBeautyShopConfigInfo`,
  `TerminateBeautyShop`), the `modelview` methods (`Get/SetCustomizing*`, `PayBeautyShop`, `ResetBeautyShop`,
  `IsModifierChanged`, `SetModifierToModel`, `RemoveEquipSlot(ES_COSPLAY)`, `ZoomInOutBeautyShop`, ...), put on the widget
  class table, and `SALON_CONFIRM()` (the confirm dialog).
* `salon.rs` reuses the character-create state (`login::create`: `Customizer`, `Look`, the `CC` natives over a `Shared`, the
  `PreviewCache`) as the edited look, built from the player's `ModelParams` (`look_of`, the inverse of
  `Customizer::create_request`); the hair list also holds the beauty-shop-only styles that have textures in this client. The 3D
  scene is the login stage (`login::stage3d`, render layer 1, the create screen's step 1 / step 2 cameras blended by the wheel
  zoom, the same lights) with the backdrop as a quad in front of the stage camera; the world camera is parked and the HUD's
  top-level windows hidden while it is up. The stage camera takes the world camera's HDR / MSAA settings (the UI cameras must
  agree with the 3D camera, `hud/cameras.rs`).
* After the paid change the player's spawn data is updated and `spawn_player` builds the model again where it stands.
* `SALON.talk(npc_obj_id)` (`CSStartInteraction`), `SALON.bypass()` (`CSEnterBeautySalon`), `SALON.cast()` are exposed for the
  script runner (`lua SALON.bypass()`).

## Not done

Paid hairstyle changes are blocked on a server defect when the character has no existing hair item:
`apply_beauty_salon` only calls `replace_hair` when `old_hair.is_some_and(...)`. A live request for hair
24128 on Salonbevy (id 14) consumed a certificate but returned hair 0. The client preserves that received
result; it must not pretend the requested style was accepted. Native camera/lighting parity, all races,
physical payment controls and remote-player appearance checks still need validation.

## Recovery verification (2026-10-23)

The original model-view method installation used `UIParent:CreateChildWidget`, which the root widget
does not support. The recovered code uses `CreateWidget`; fresh captures now show populated hairstyle,
colour and preset grids, and selecting hair 3 changes the actual 3D preview. The original Lua's
`GetCurrentCosmeticItem` and the nonempty icon path are asserted during the live script. Enter/edit/cancel
passes against the bevy lane; the paid-change/reentry check correctly fails on the server defect above.

Inspected captures: `shots/aa-client-salon/1_entered.png` (before the method-installation fix),
`shots/resume/salon/entered.png`, `edited.png` and `after-cancel.png`. The live offscreen run logs roughly
25–42 FPS while editing, with no errors, one allowed existing `tab_language` Lua error, and renderer/decal
warnings. Scripts and logs are `data/local/resume/resume-salon*.txt` / `data/resume-salon*.log`. These images validate
the window and preview, not original-client lighting parity.
