# aa-client: rigid equipment (weapons, backpacks, tail, beard)

Follow-up to `docs/research/equipment-attach.md` (causes 3, 6, 7, 10), `equipment-sweep.md` and `re-equipment.md` (Ghidra of
x2game). Code: `crates/aa-client/src/ingame/weapons.rs`, `data.rs` (`Db::weapon`, `Db::backpack`), `appearance.rs`
(`extra_body_parts`), `prepare.rs`. Check tool: `cargo run -p aa-client --example equipment` (below).

## What changed

| cause | change |
|---|---|
| 3 two-handed weapons on the backpack bone | `sheath_bone`: two-handed weapons (and the old fallback arm) hang on `item_back_r` (x2game sheath code 103; the relaxed / run clips key `item_back_r`, the holster clip is `twohand_ac_holster_back_r`). `item_back_c` is the backpack / glider bone now. Staves stay on `item_staff_l` and go to `item_side_l` after a weapon skill, the bow on `item_bow_l`, the shield on `item_arm_l`. A 2H sword now runs from over the right shoulder to the left hip, as in the real client; before it stuck 1.3 m out of the back (the sweep's `sheath_horizontal`, 3,557 findings, mostly this and cause 7) |
| 6 `.cdf` weapons invisible | `weapons::model_files(path)`: a `.cdf` is unwrapped. A `real_torch.chr` carrier (`NoDraw` material) is skipped and its `CA_BONE` object attachment (`sword` / `item` / `weapon`, with the `_enchant` material for the 1H / 2H blades) is the blade; a `.cdf` whose `Model` is a real mesh (`shield_gorgon_002`, `sword_1h_59_1`, `bow_59_1`) is drawn as that model with its `Material`. Several pieces hang under one parent entity. `prepare.rs` prewarms the same files. A dangling `.cdf` (`sword_2h_46_8.cdf` is not in the pak) falls back to the `.cgf` of the same name. The effect attachments (enchant glow, `wpnfx_hasla_*`) are not drawn (no persistent weapon effect hook yet); `real_torch.cdf` (the torch item: light and fire are entity bindings) draws nothing |
| 7 rotation order | `attachment_rotation`: `Quat::from_euler(EulerRot::ZYX, rz, ry, rx)` = `Rz * Ry * Rx`, X first, as x2game's `3935A800` / `3935EA10` do (Ghidra, `re-equipment.md` (b)). `AA_DEV_EULER_XYZ=1` brings back the old order for comparison |
| mirror | `item_assets.allow_mirror` (14 assets, 1H swords / maces / axes): the off hand copy flips `pos_x`, `rot_y`, `rot_z` (x2game `3935EA10`, slot 16) |
| 10 slots never drawn | Backpack (wire slot 26): `Db::backpack` reads `item_backpacks` (`asset_id`, or `asset2_id` = the stowed look of a glider, `glider_b_back.cgf`) with the `item_assets` offset; hung on `item_back_c` as `Grip::Pack`, never in a hand. The 5 gliders with no stowed look (`glider_bx.chr`, only the 6 m flying canopy) draw nothing. Tail (ferre `fe_m_tail01`, `fe_f_tail`) and beard (`fe_m_beard01`): `appearance::resolve` adds them as extra skin parts (`item_body_parts`, model must match); without the tail slot a ferre body has a hole at the tail root. Glasses: no row in `item_body_parts` / `item_armors` has a glasses visual, nothing to draw |

Not changed: backpacks are drawn also under a costume (x2game has `ShowBackPackWithCosplay`; its default was not read);
the fishing rods (27308..: 3.2 m props, `staff` pose) still hang as a staff; the sweep's `floating` for 1H spears on `item_side_l`
(the authored offset) is unchanged.

## Rotation order: evidence

The decompile of both consumers builds the quaternion with the CryEngine `SetRotationXYZ` formula (`w = cx cy cz + sx sy sz` ...),
i.e. X first. No real-client capture of an item with two or more non-zero angles was available (the running real client was
another session's, and `reference/ingame/combat_*` carries a sword with a single axis); the check is by look:
`shots/aa-client/weapons-fix/rot_xyz_left.png` (old order) against `rot_zyx_left.png` (new). The 1H axe `axe_1h_01_1` (5961, rot
(-157, 180, -1)) and `axe_1h_36_3` (13855, (-160, 180, -1)) lie horizontal and stick out of the hip with XYZ (what the sweep
reports as `sheath_horizontal`), and hang along the thigh head up with ZYX; the 2H spear 26302 (0, 180, 2.5) and the pick 2667 move
too.

## The `equipment` example

```
AA_EQUIP="race=nu,g=m,main=5660,label=2H sword;race=fe,g=m,tail=562,beard=564" AA_EQUIP_VIEW=back \
  cargo run -p aa-client --example equipment -- --shot shots/aa-client/weapons-fix/x.png --shot-after 8
```

Figures are separated by `;`, each a `key=value` list: `race` (nu el ha fe), `g` (m f), `main off ranged music back cloak tail
beard glasses chest legs feet hands head cosplay` = item template id, `label`. `AA_EQUIP_VIEW` is `back` / `left` / `right` /
`front`, `AA_EQUIP_DRAW=1` puts the weapons in the hands. The figures are built with `appearance::describe` / `resolve` and
`weapons::carried` / `attach_carried`, as the game does, and frozen in the relaxed idle (frame 0.7 s).

Frames in `shots/aa-client/weapons-fix/` (git-ignored): `a_back` 2H sword / staff / mace / spear / axes from behind; `b_left`,
`b_front` sword + shield + bow, axe, staff, 2H sword and mace; `c_left`, `c_back` more 1H axes, 1H spears, 2H axe / mace /
`.cdf` blade (18663), fishing rod; `d_pack_left`, `d_pack_back` backpacks (the last column of the first run drew the deployed
`glider_bx`, now skipped); `e_tail_back`, `e_tail_front` ferre tails and beard; `f_cdf_hand` the `.cdf` blade (18661), the Gorgon
shield (30060), the 2H blade, the dragon staff (28905) and mace (35524) and mirrored off hand swords in the hands;
`g_ingame_2h` the game screen with the 2H sword (`AA_DEV_MAINHAND=5660`) on the back. Before: the sweep sheets
`shots/aa-anim/equip/twohand_nu_m_a_idle_back.jpg` and `sword_main_nu_m_a_idle_side.jpg`.
