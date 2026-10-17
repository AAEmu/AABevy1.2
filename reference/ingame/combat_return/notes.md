# ingame/combat_return: Recall (Return), 10 s cast, then the zone reload

Key **SHIFT-9** (action bar slot 21, `Recall`, skill id **10594**, icon `icon_skill_basic04`, 10 s cast, 30 min cooldown, `start_anim` 108
`all_co_sk_buff_cast_mental`, fire anim 100, fx group 288), pressed with no target selected. One `StartSkill: Id 10594` in the server log.
Frames were scheduled at about 1, 5 and 9 s (plus a few more); the screenshot hook plus a busy box delayed them, the real times are in
`frames.json` (`target_s` = asked for, `t_done_s` = file written).

* real_02 (0.8 s) and real_03 (1.6 s): the cast has started: a blue cast bar labelled "Recall" appears above the action bar (centre bottom,
  about x 790 to 1130, y 920 to 940) and fills slowly; the character stands in the sword-ready pose.
* real_07 (5.65 s): half way: the cast bar is half full and a **purple-white glowing sphere with lightning-like strands is around the left hand**
  (the FORM effect); the character holds the pose (the loop of `all_co_sk_buff_cast_mental`).
* real_11 (9.95 s) and real_12 (10.9 s): the bar is nearly full / full, the sphere is the same.
* **The fire is a zone change** to the return point: from real_13 (13.55 s) on the loading screen (ship at sea, "ArcheAge: Craft. Claim. Conquer.",
  gauge); real_13 to real_19 (13.5 to 17.7 s) are all loading screens. Loading took about 1 minute here (the return point is the Nuian spawn, the same pond).
* real_20 to real_22: after the reload, at the arrival (the weapon sheathed again, default camera behind the player, bow on the back). Their times
  (about 169 s after the key press) are approximate (file times).

## Setup and camera (same for every combat_* folder)

* Real 1.2 client (r208022), lane `bevy`, account `clienttest`, character **Refone** (Nuian male, level 1, Battlerage; one-hand sword and
  shield equipped, bow on the back), 1920x1080, UI scale 1.0, HUD visible. Standing in the pond at the Nuian spawn (client-local
  position about (4311..4314, 4112..4118, 126.2); the Angry Rat spawner is 3 m from the spawn point). The character was put there by
  its DB row (server position 15578, 15382.1, 126.245) while logged out; the DB row was restored afterwards.
* **Camera: fixed, side-on.** The camera action `right_camera` (hotkey Home) snaps the camera to the character's side: camera yaw =
  player yaw - 90 deg, pitch 0, roll 0, **4.0 m** horizontal from the player, **1.734 m** above the player's feet (4.36 m to the
  feet), looking at the player, who stays centred on screen (x about 960); the character and the rat face screen-left (the camera
  stands on the character's left side). The default third-person camera has the same distance (4.0 m behind) and height, so this is
  the client's own distance. The coordinator asked for 5 to 6 m: the camera distance could not be changed from outside (the
  `zoom_in` / `zoom_out` actions and every `cl_*` / `cam_*` CVar I could find did nothing, the distance stayed 4.000 m), so the lens
  was widened instead: **`cl_fov` 75 = 71.25 deg** (default 45 = 42.75 deg; both are `System.GetViewCameraFov()` in degrees).
  At 4 m with 42.75 deg the ground at the target's depth falls off the bottom edge, with 71.25 deg the player, the target and the HUD all fit.
* `camera.json` has the exact numbers: `before_key_press` (or `before_burst`) and `after_burst`, each with the view camera `cam`
  (position), `ang` (pitch, roll, yaw degrees), `fov` (radians), `tod` (`e_time_of_day`), the `player` position and yaw (`pyaw`,
  radians), `camera_to_player_xy`, `camera_height_above_player_feet`, `camera_yaw_minus_player_yaw_deg` and `nearest_mob` (class,
  client-local position, xy distance). The camera yaw follows the character when it turns: it stays at -90 deg unless the character spins
  hard (after the melee burst it read -147 deg because the character turned to the rat while the camera kept its yaw).
* Time of day: `e_time_of_day` is in `camera.json` (it follows the server clock, 2.7 h to 3.8 h in these captures; it cannot be forced).
* **Frames** are real screenshots of the DX10 back buffer (the driver's DXGI hook, `aaemu-clientdriver screenshot`), taken strictly
  one after the other as fast as the hook allows (0.25 to 0.5 s each, 0.3 s typical; the shared box was sometimes slower, see the gaps).
  `frames.json` lists for every `real_NN.png` the time the request was made (`t_req_s`) and the time the file was written
  (`t_done_s`), in seconds after the key press was sent (0.0 = the `client_input` call, which returned after `key_press_returned_s`);
  the picture is the first presented frame after the request, so its true time is about `t_req_s` + 0.05 s. `skill_id`, the key and
  the server's `StartSkill` lines (`server_log`) are in `frames.json` too.
