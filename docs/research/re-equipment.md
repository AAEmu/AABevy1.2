# x2game RE: equipment questions (costume slots, attachment rotation, heel)

Source: Ghidra 12.1.4 on `D:\aa-re\client12\x2game_unpacked.dll` (image base `0x38FF0000`, full auto-analysis, project in
`data/extract/ghidra/proj`). Follow-up to `docs/research/equipment-attach.md` (6b07470).

## (a) Armour slots a costume clears: `0, 2, 4, 5, 6, 8`

The table at `0x39AC7358` (`x2game+0xAD7358`) sits in `.data`, which is still encrypted in the unpacked file. It was read
**read-only** from a running 1.2 client (`ReadProcessMemory`, pid of a test client in `D:\aaemu-testclient\AA1.2-ext`; its
`.text` bytes at `0x39316C10` are identical to the unpacked DLL, so it is the same x2game build):

```
x2game+0xAD7340: 4 5 6 7 8 27 | 0x39AC7358: 0 2 4 5 6 8 | 3619 1846 ...
```

The loop in `39316BF0` (`39316C10..39316C54`: `mov eax,[edi+0x39AC7358]`, `edi += 4`, `cmp edi, 0x18`) walks exactly these
6 dwords (each is an index into the per-slot `0xC0`-byte equip array, `slot*0xC0 + unit`). In the wire numbering of
equipment-attach.md that is:

| value | slot |
|---|---|
| 0 | head |
| 2 | chest |
| 4 | legs |
| 5 | hands |
| 6 | feet |
| 8 | back (cloak) |

So the guess 2..=7 was wrong: a costume hides the **helmet and the cloak** too, and keeps **waist (3) and arms (7)** (and
underwear 13/14). Rule to implement: when slot 27 has a drawable costume, do not draw slots 0, 2, 4, 5, 6, 8 (plus the nude
body, see 4.2 of equipment-attach.md); re-dress the same six when it comes off.

(The table just before it, `x2game+0xAD7340` = `4 5 6 7 8 27` after unknown leading entries, is a different list; not used
by `39316BF0`.)

## (b) `attachment_offset_rot_*` -> rotation: Rz·Ry·Rx (glam `EulerRot::ZYX`)

`item_assets` loader `396C89D0` (`SELECT id, allow_mirror, attachment_offset_pos_x..z, attachment_offset_rot_x..z,
default_anim, detail, heel_offset_height, ...`) fills a 0x48-byte record: `+4` allow_mirror, `+8/+C/+10` pos, `+14/+18/+1C`
rot (degrees), `+20` default_anim, `+24` detail, `+28` heel_offset_height, `+2C` hinge_damping, `+30` hinge_idx, `+34`
hinge_limit, `+38` model_id, `+3C` more_asset_id, `+40` name_tag_offset_height, `+44` path.

Two consumers build the quaternion inline, both the same formula:

* `3935A800` (per equip slot `param_2`, asset = `394E58A0(unit + 0x4B8 + slot*0xC0, model)`): `attachment->vfunc+0x34(QuatT)`.
* `3935EA10` (weapons / held items, only when `(detail & 0x7E) != 0`): same, with a mirror (below) and a scale.

With `cx = cos(rx/2)`, `sx = sin(rx/2)` (rx in degrees · 0.017453292) etc.:

```
w = cx*cy*cz + sx*sy*sz
x = sx*cy*cz - cx*sy*sz
y = cx*sy*cz + sx*cy*sz
z = cx*cy*sz - sx*sy*cz
```

This is CryEngine `Quat::SetRotationXYZ(Ang3)` = `qz * qy * qx` (X applied first, then Y, then Z), i.e. `R = Rz·Ry·Rx`.
In glam: `Quat::from_euler(EulerRot::ZYX, rz, ry, rx)` (radians). The proposal in equipment-attach.md 4.7 is confirmed.
Translation is `(pos_x, pos_y, pos_z)` unchanged (metres).

Mirror in `3935EA10`: for the off-hand slot (`slot == 16`) when `394E2840()` is true (the mirror condition, presumably
`allow_mirror`), `sign = -1` and the client uses `pos_x * sign`, `rot_y * sign`, `rot_z * sign` (rot_x unchanged): a
mirror across the attachment's X plane. Otherwise sign = 1.

## (c) `heel_offset_height`: camera only, no model lift found

Update 2026-10-23: the two item fields are the costume (`+0x1C` of the `equip_pack_cloths` record = `cosplay_id`)
and the shoes (`+0x4C` = `shoes_id`, twelve fields on in the loader's column order), and the camera rises by exactly
the heel difference, i.e. it follows a head that stands `heel` higher. aa-anim lifts the model root by the heel
(costume first, then shoes); see `docs/aa-anim.md`, "Heel".

The only reader of record `+0x28` is `39311310(model, itemA, itemB)`: the heel of `itemA`'s asset for that model, else of
`itemB`'s, else 0 (assets via `394E5630(item, model)`). It has exactly two callers, both cameras:

* `39413F40` (the "profileshots" portrait renderer: `fist_ac_stage_idle`, `e_character_back_light`, `r_ColorGrading` ...):
  `heel = 39311310(model, itemA, itemB)` (two item slots of a local copy of the look); the camera matrix translation z gets `+= heel - heel_of_a_second
  look` (`397A7360` -> `39696100`, its items at `+0x1C` / `+0x4C`, 0 when absent).
* `392BD640` (a character camera, called from `392BE110`): the same, `pfVar5[0xB]` (translation z of the camera matrix)
  `+= heel(self items @+0x1700 / +0x740) - heel(second look)`.

So x2game uses the heel only to raise the camera / portrait target by the heel height (minus that of the look it is
compared with). No code in x2game lifts the character model or its root by it; whether CryAnimation's ground alignment
compensates was not checked (it has no `heel_offset` input). Which slots the two item fields are (costume first, then shoes
is the likely order) was not confirmed.
