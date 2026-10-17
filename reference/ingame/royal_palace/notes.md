# ingame/royal_palace: real 1.2 client at Aanetwalker's spawn spot (Two Crowns, "Royal Palace")

Question: is the cobblestone terrain with mounds in the courtyard in front of the player right? Answer from the real
client: yes, the same mounds are there. `real.png` has the same ridge and the same dark cobbled slopes, in the same
places, as `shots/aa-fx/ingame.png`. What differs is only lighting and HUD (see "Differences").

## Files

| File | What |
|---|---|
| `real.png`, `ui.json`, `camera.json` | Camera behind the player, same heading as the aa-fx shot (camera yaw -90.7 deg, facing +X client-local). FOV 42.75 deg (`cl_fov` 45). |
| `real_turned90.png`, `ui_turned90.json`, `camera_turned90.json` | Turned 90 deg to the left (player and camera yaw 0, facing +Y), same FOV and same standing spot. |
| `real_turned90_wide.png`, `ui_turned90_wide.json`, `camera_turned90_wide.json` | Same view, wider lens: `cl_fov` 75 (reported FOV 71.25 deg). |

Each `camera*.json` has the view camera (`pos`, `angles_deg` = pitch, roll, yaw, `fov_deg`), CVars (`e_time_of_day`, `cl_fov`),
and the player entity (client-local position, yaw in radians, `terrain_height`).

## Values

Aanetwalker's saved row in the lane DB (`lane_bevy_game.characters`, id 4, account `aanet2`), server coordinates:
`zone_id 186`, `x 13637`, `y 10449`, `z 238.676`, `yaw 0`, `pitch 0`, `roll -1.5708`. The server stores the heading in
the **`roll`** column (`yaw` stays 0); -1.5708 rad is -90 deg.

| Shot | Player (client-local) | Player yaw | Camera pos | Camera yaw | FOV | `e_time_of_day` |
|---|---|---|---|---|---|---|
| `real` | 4421.0, 4305.0, 238.676 (on the terrain) | -1.583 rad (-90.7 deg) | 4416.99, 4305.05, 240.41 | -90.71 deg | 42.75 | 9.563 |
| `real_turned90` | 4421.0, 4305.0, 238.676 | 0 | 4421.0, 4300.99, 240.41 | 0 | 42.75 | 9.998 |
| `real_turned90_wide` | same | 0 | same | 0 | 71.25 | 9.987 |

Client-local coordinates are not server coordinates (the seamless map origin moves): server (13637, 10449) is client (4421, 4305).
The default camera sits 4 m behind the player and 1.73 m above the terrain, pitch 0, roll 0; its yaw equals the player's yaw
(client yaw -90.71 deg although the DB heading is exactly -90 deg: the client reads back 0.7 deg off, as at spawn in `ingame/spawn`).
Zone text "Two Crowns", HUD zone name "Ezna" (sub zone) at this spot; the roadmap shows "Royal Palace" next to the player marker.

## How it was set up

1. Read Aanetwalker's row (private MySQL 3317, lane DB `lane_bevy_game`, read only; never logged in as `aanet2`).
2. Refone (account `clienttest`, character id 3, logged out) got the same `world_id`, `zone_id`, `x`, `y`, `z`, `yaw`, `pitch`
   and `roll` copied from that row by one `UPDATE ... JOIN` (exact float copies). No GM command was needed.
   Original Refone row, restored afterwards: `zone_id 179`, `x 15578`, `y 15382.1`, `z 126.245`, `yaw 0`, `pitch 0`, `roll 2.74889`.
3. Own client launched as in the README (`capture.py launch`, window hidden, `to-create`, Start on Refone), waited until the
   picture settled (`capture.py settle`), then `capture.py screen ingame/royal_palace --camera`.
4. The turned shot: the camera of the real client cannot be turned from outside (the view camera is rewritten every frame, `SetViewCameraAngles`
   and `X2Camera:SetUnitCameraAngles` do not stick, `client_pointer` cannot hold a button, Q/E/A/D strafe). So the client was closed,
   the DB row set to the same spot with `roll 0` (+90 deg), and the character entered again.
   The wide shot is `System.SetCVar('cl_fov', 75)` in the System Lua state (45 restored right after).
5. `e_time_of_day` follows the server clock and cannot be forced with `System.SetCVar` (it snaps back). The three shots were taken
   at 9.56 h to 10.00 h game time.

## Differences to the aa-fx shot

* Character: Refone (blond, brown jacket) instead of Aanetwalker; no tutorial popup, three buffs instead of five.
* Lighting: the real cobbles look darker and bluer at game time 9.6 h and the plaza is partly in shadow; the aa-fx shot is brighter and
  browner (its time of day is not known here). Terrain shape, ridge position, tree positions and wall layout match.
