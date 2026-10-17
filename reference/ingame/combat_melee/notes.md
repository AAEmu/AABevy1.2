# ingame/combat_melee: basic melee auto attack on an Angry Rat

Key **3** (action bar slot 3, `Melee Attack`, skill id **2**, icon `icon_skill_basic01`, 300 ms cooldown); pressed with the rat at 0.94 m.
16 frames from 0.34 s before to 3.9 s after the key press. One press starts the attack and the client keeps repeating it (the server
log shows two `StartSkill: Id 2` within the same second; the character swings about once a second until the target dies). The rat had
148 HP before and 87 after the burst (hits of 19 and 22, white damage numbers over the character).

What the frames show: 1 to 2 idle with the target selected (red target ring, the rat's red name plate and HP bar, the target frame in the
HUD top-centre); 3 to 4 the sword arm comes up and the shield swings onto the arm (weapon draw; "Battle Start!" in red over the head from
frame 4); 5 to 16 the swings: the sword is raised, comes down in a horizontal cut, a hit number floats up, the rat's HP bar drops, the
rat flinches. A press while the weapon is sheathed starts with the draw; `combat_death_2` shows the same swings with the weapon already out.

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
