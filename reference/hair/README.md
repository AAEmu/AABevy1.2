# reference/hair: real 1.2 client hair close-ups

Real client (r208022), 1920x1080, own account `refhair` (not clienttest / bevytest / bevycreate), lane `bevy`. Taken for the hair shader:
the character-create appearance step (step 2, UI panels hidden, camera zoomed in once with `ZoomInCustomizeCamera`) and in game.
`HairColor` / `HairStyle` below are the indices the client UI uses (`X2CustomizingUnit:SetCustomizingHair(style)` / `SetCustomizingHairColor(index)`),
read back with `GetCustomHair()`. Colour index n is the `hair_colors` row with `comment = customNN` of the race/gender (`hair_material`
`..._hcNN.mtl`, `hair_base_color_r/g/b` in `compact.sqlite3`); every race/gender has 60 colours (`GetNumCustomHairColor()` = 60).
Hair styles: Nuian m 21 (+bald), Nuian f 22, Elf m 18, Elf f 22, Firran m 10, Firran f 14, Harani m 22, Harani f 22 (`GetNumCustomHair()`).

## Index sheets (jpg, every style / colour at a glance)

* `index_styles_<race>_<gender>.jpg`: all hair styles in order, numbers in the corner = style index (colour 22, dark red-brown, so shape reads).
* `index_colors_nuian_male_style03.jpg`: all 60 colours on one style (number = colour index). 4 = silver-grey, 7 = blond, 22 = dark red-brown (the "brown" below), 25 = red, 47 = black.
  The other races use the same index order with their own `hcNN` materials; the colour names below are by eye.

## Character create (`<race>_<gender>_<colour>_style<NN>.png`)

Front view, head and shoulders, bright painted backdrop behind the head. The backdrop is the light source you see against the hair:
judge rim and thin loose strands on the long styles (e.g. Nuian m blond 7, Nuian f silver 19).
The create scene cannot be orbited from outside, so the "against the light" shots are the ones where the hair silhouette is in front of the bright sky patch of the backdrop;
the real back-lit shots are the in game ones.

| file | race gender | HairStyle | HairColor |
|---|---|---|---|
| `elf_female_black_style17.png` | elf female | 17 | 47 (black) |
| `elf_female_blond_style20.png` | elf female | 20 | 7 (blond) |
| `elf_female_red_style11.png` | elf female | 11 | 25 (red) |
| `elf_male_black_style14.png` | elf male | 14 | 47 (black) |
| `elf_male_silver_style10.png` | elf male | 10 | 4 (silver) |
| `elf_male_blond_style03.png` | elf male | 3 | 7 (blond) |
| `elf_male_red_style08.png` | elf male | 8 | 25 (red) |
| `firran_female_black_style04.png` | firran female | 4 | 47 (black) |
| `firran_female_blond_style06.png` | firran female | 6 | 7 (blond) |
| `firran_female_red_style14.png` | firran female | 14 | 25 (red) |
| `firran_male_black_style02.png` | firran male | 2 | 47 (black) |
| `firran_male_silver_style05.png` | firran male | 5 | 4 (silver) |
| `firran_male_brown_style06.png` | firran male | 6 | 22 (brown) |
| `harani_female_black_style09.png` | harani female | 9 | 47 (black) |
| `harani_female_brown_style20.png` | harani female | 20 | 22 (brown) |
| `harani_female_red_style14.png` | harani female | 14 | 25 (red) |
| `harani_male_black_style18.png` | harani male | 18 | 47 (black) |
| `harani_male_blond_style17.png` | harani male | 17 | 7 (blond) |
| `harani_male_silver_style09.png` | harani male | 9 | 4 (silver) |
| `nuian_female_black_style10.png` | nuian female | 10 | 47 (black) |
| `nuian_female_blond_style20.png` | nuian female | 20 | 7 (blond) |
| `nuian_female_brown_style17.png` | nuian female | 17 | 22 (brown) |
| `nuian_female_silver_style19.png` | nuian female | 19 | 4 (silver) |
| `nuian_male_black_style09.png` | nuian male | 9 | 47 (black) |
| `nuian_male_blond_style07.png` | nuian male | 7 | 7 (blond) |
| `nuian_male_silver_style20.png` | nuian male | 20 | 4 (silver) |
| `nuian_male_red_style08.png` | nuian male | 8 | 25 (red) |

Long hair: Nuian m 9, 7, 20; Nuian f 10, 17 (pigtail curls), 19, 20; Elf m 10, 14; Elf f 17, 20; Firran m 2, 5 (dreadlocks), 6 (afro); Firran f 4, 14; Harani m 18, 17, 9; Harani f 9, 20, 14 (curls).

## In game (`ingame_*.png`)

Character Hairrefb (Nuian male, HairStyle 7, HairColor 7 = blond, jacket and bow on the back), standing in the pond at the Nuian spawn (client position about (4314, 4117, 126), Solzreed). HUD hidden with the
`screenshotmode` hotkey action, telephoto lens (`cl_fov` 16 to 22, a 4 m camera made close-ups) and the camera snapped with `front_camera` / `right_camera` / `back_camera` (a close head and shoulders at about 2 m equivalent).
The game time cannot be forced (`e_time_of_day` snaps back), so I waited for it: the shots are at the real game hours listed.

| file | when | what |
|---|---|---|
| `ingame_morning_backlit_front.png`, `_side.png`, `_back.png` | backlit_front 7.37 h, backlit_side 7.39 h, backlit_back 7.40 h (low morning sun) | sun roughly behind or beside the character (the water sparkle is in the background); hair against sky and foliage |
| `ingame_noon_sun_front.png`, `_side.png`, `_back.png` | sun_front 11.49 h, sun_side 11.49 h, sun_back 11.51 h | sun high, the character in the sun, dark foliage behind |
| `ingame_shade_side.png`, `_back.png`, `ingame_shade_front.png` | front 11.95 h, side 11.97 h, back 11.97 h | hair in the shade of the bank / overcast, same character, sky behind. `ingame_shade_front.png` has a lily leaf over the chest, the face and hair are clear |

Per-shot camera position, angles, FOV and `e_time_of_day` were logged while taking them (the values in the table above); `create_shots.json` has the style and colour of each create shot.

## Notes

* No hair-specific detail is available from the UI side: textures, specular and alpha cannot be read back (`GetId`/drawable getters are empty), so these are pictures only.
* The create background is a painted 2D image; its hot spots are not the 3D sun. For translucency use the in game back-lit shots.
* Accounts / characters created for this: account `refhair`, characters `Hairrefa` (Nuian male, style 3 colour 6) and `Hairrefb` in the lane `bevy` DB. Tools: `reference/tools/hair.py` (not committed here).
