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

Backpack under a costume: the options script `x2ui/option/interface_option.lua` has `ShowMyCosplay` default 1 and
`ShowMyBackPackWithCosplay` default **0** (`X2Player:ShowCosplay` / `ShowBackPackWithCosplay`), so `weapons::carried` hides the
backpack while a costume (wire slot 27) is worn.

**Fishing rods** (27308..27312, 27367..27370, 29702, 30584; holdable 17 = `2h_blunt_staff`, pose 4, `worn_scale` 0.8, offset
(0, -0.77, 0.05)). Nothing is missing in the data: the grip (cork handle, reel) is the model origin, the rod is 3.1 m and points
along +Y. The hold is the animation: the fishing clips `fist_fishing_idle`, `fist_fishing_action_r/l` and
`fist_pos_stn_fishing_idle_rand_*` (and `animations/item/interaction/fishpole_a/*` for the rod itself) key both hands onto the rod
(`h_rod_fishing_left.png`: the right hand holds the handle, the left the reel, the rod points forward and up; `AA_EQUIP_CLIP=
fist_fishing_idle AA_EQUIP_DRAW=1` of the example). In the frozen idle the rod hangs from the hand at the side, which is what the
sweep calls `weapon_off_body` / `bad_size` (bbox centre 1.1 m from the hand of a 3 m rod): expected, not a placement bug. The client
has no fishing flow, so no code plays the clips; sheathed a rod hangs like a staff (`item_staff_l`).

**1H spears on `item_side_l`** (5729, 21488, 21792): single axis rotation (0, 0, 0.5), so the rotation order changes nothing. The
13 to 15 cm was a bind-pose artefact of the sweep: the relaxed idle clip moves the item bones (`item_side_l` from (0.027, -0.001,
0.186) to (0.149, 0.156, 0.185) in the pelvis frame, `item_back_r` 12 cm, the staff and bow bones 2 cm) and the sweep placed the
weapons in the bind pose. In the idle pose the shaft touches the hip (`i_spear_left.png`); no offset is missing. The sweep now
places weapons in the idle pose.

Not changed: the `.cdf` effect attachments (enchant glow) are not drawn; the sweep's `no_material` for the four 41_* `.cdf` items
(27334, 27338, 27340, 27344) is a sweep artefact (their `Material` is in the cdf, which `model_files` passes on).

## Equip sweep, weapon slots, before and after

`cargo run -p aa-anim --example equip_sweep --release -- --analyze --slot <slot>` for the nine weapon slots, all 8 models
(findings in `data/extract/aa-anim/equip_sweep*/`, git-ignored). The sweep's weapon rules now mirror `weapons.rs` (`item_back_r`,
ZYX, `.cdf` blade resolution) and place the weapons in the idle pose against the idle body. Findings (item x model):

| category | before (bind pose, old rules) | after, new rules, bind pose | after, new rules, idle pose |
|---|---|---|---|
| `sheath_horizontal` | 3557 (449 items) | 6 (1) | 22 (3 shields, `item_arm_l`) |
| `floating` | 553 (99) | 171 (25) | 169 (58, thin swords 13 to 15 cm, NPC props) |
| `weapon_cdf` | 200 (21) | 8 (1: `real_torch.cdf`, the torch has no mesh) | 8 (1) |
| `weapon_off_body` | 199 (20) | 151 (21) | 113 (20: fishing rods in the hand, the NPC telescope) |
| `bad_size` | 631 (49) | 239 (32) | 173 (18: rods, NPC 3.5 m props) |

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
`front`, `AA_EQUIP_DRAW=1` puts the weapons in the hands, `AA_EQUIP_CLIP=<cal clip>` / `AA_EQUIP_TIME=<s>` play and hold a clip
(`fist_fishing_idle`). The figures are built with `appearance::describe` / `resolve` and
`weapons::carried` / `attach_carried`, as the game does, and frozen in the relaxed idle (frame 0.7 s).

Frames in `shots/aa-client/weapons-fix/` (git-ignored): `a_back` 2H sword / staff / mace / spear / axes from behind; `b_left`,
`b_front` sword + shield + bow, axe, staff, 2H sword and mace; `c_left`, `c_back` more 1H axes, 1H spears, 2H axe / mace /
`.cdf` blade (18663), fishing rod; `d_pack_left`, `d_pack_back` backpacks (the last column of the first run drew the deployed
`glider_bx`, now skipped); `e_tail_back`, `e_tail_front` ferre tails and beard; `f_cdf_hand` the `.cdf` blade (18661), the Gorgon
shield (30060), the 2H blade, the dragon staff (28905) and mace (35524) and mirrored off hand swords in the hands;
`g_ingame_2h` the game screen with the 2H sword (`AA_DEV_MAINHAND=5660`) on the back. Before: the sweep sheets
`shots/aa-anim/equip/twohand_nu_m_a_idle_back.jpg` and `sword_main_nu_m_a_idle_side.jpg`.
