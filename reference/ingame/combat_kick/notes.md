# ingame/combat_kick: Kick (a second Battlerage skill) on an Angry Rat

The character has no Charge. The next Battlerage skill on the bar is **Kick** (skill id **14495**, action bar slot 14, key **SHIFT-2**, icon
`icon_skill_fight22`, no cooldown, range 2 m, fire anim 389, plot 326). The other bar entries are Dash (16287, slot 13), Dance (14152, slot 20),
Recall (10594, slot 21), Play Instrument (14900, slot 22) and the not usable Use Glider / Drop Trade Pack. 8 frames, 0.0 to 3.1 s after the key
press, one `StartSkill: Id 14495`; the rat's HP went 137 to 125 (one hit of **12**). The character's HP read 393 of 720 at the start (the rat
had bitten it in an earlier try), visible in the HUD HP bar.
What the frames show (look at the character, not at a leg): 1 (0 s) idle, shield up; 2 (0.37 s) the sword is held low in front; 3 (0.88 s) the
sword arm swings up and back with a white blade glow and the body steps forward; 4 (1.35 s) the character has lunged to the left, towards the rat
(it ends near the left edge of the 4 m camera frame; the character moves about 1 m during this skill), the yellow damage number 12 floats up over
it; 5 (1.8 s) the number is still rising and the character is back at its place, the shield comes forward again; 6 to 8 back to the stance. The
name "Kick" and the 12 damage are from the client; I did not look into the animation name, in these frames it reads as a lunge with the sword,
not as a leg kick.

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
