# ingame/mirage_isle: real 1.2 client on Mirage Isle (`arche_mall_world`, zone 260)

Question: why does the remake wash Mirage Isle out in white? Answer from the real client: the island is clean, with no visible
fog even over the sea. The zone's `time_of_day.xml` layer 0 (global density 5, white fog x 10) is not what the client
draws: a time of day **area** covering the island replaces it.

## Files

| File | What |
|---|---|
| `real.png`, `ui.json`, `camera.json` | Spawn view after entering through the portal, HUD on (a tutorial popup over the floor). |
| `real_noui.png`, `camera_noui.json` | Same view, UI hidden (`screenshotmode` action). |
| `real_turn1..3.png`, `camera_turn1..3.json` | UI hidden, character turned left with `a` (900 ms each): camera yaw 56.4, 112.9, 166.3 deg. `turn3` looks over the sea and the beach tents. |

All shots: 1920x1080, `cl_fov` default (reported FOV 57 deg), camera 1.73 m above the player, pitch 0. `e_time_of_day` 5.07 to 5.18
(the server clock; Mirage's `lighting.xml` `DawnTime` is 371 = 6:11).

## Coordinates

Client-local = world - 3072 (3 cells) in x and y: camera (608.5, 1496.2, 157.8) is world (3680.5, 4568.2), the server's
`arche_mall_world` spawn (3680.5, 4572.2, 156) for the player. Same view in the remake:

```
cargo run -p aa-world --example fly -- --world arche_mall_world --camera-json reference/ingame/mirage_isle/camera_turn3.json \
  --shift 3072,3072,0 --hour 5.18 --subject --shot shots/aa-world/mirage/turn3.png --shot-after 15
```

## The time of day the client uses here

`ss_debug_ui 4` (System state CVar) prints the client's time of day inputs. In Hellswamp:
`TOD Zone Weight: (161) w_hell_swamp_1 (normal, sunny) : 1.0000`, `(248) w_hell_swamp_2 (normal, sunny) : 0.9540`.
On Mirage Isle: `Area TOD [0] : Entity(11, tod_area_shopping_land), weight(1.0000), priority(0)` and no zone line, i.e. the
AreaShape `tod_area_shopping_land` (Group 14, `cells/003_003/client/entities.xml`, 23 points around the island) takes the whole
weight and zone 260's own file gets `1 - 1 = 0`. Its curves: green horizon (0.24, 0.81, 0.24 x 0.2) under a navy zenith,
fog layer densities 0.01 / 0.001, fog colour x 0.1 to 0.24. So the player (the area manager's subject) is inside the area in
ordinary play: aa-client marks its local player with `aa_world::AreaToDSubject`.

## How it was set up

1. Lane `bevy` servers (login 27237), account `bevylive1`, character Livebevyone (GM). `capture.py launch` with `AA_USER=bevylive1`.
2. `/move 7484.4 9256.8 262` next to the Hellswamp Mirage Isle Portal (doodad 4894 at 7483.3, 9258.0), then the
   `do_interaction_1` action (`client_action`) entered the system instance (zone key 260) through the loading screen.
3. Logging out inside the instance saves the main world return point; Livebevyone's row was restored afterwards
   (`zone_id 179`, `x 15580.8`, `y 15386.6`, `z 126.245`, `roll 0`).

Notes: `System.SetCVar('e_time_of_day', h)` does not stick (the engine rewrites it every frame) and the lane's
`/time set hour=13` answers "Parameter hour is required" although `/echo` shows `hour=13` arriving intact, so only the server
hour was captured. The large studded door left of the spawn is a server doodad (not in the cell data).

## Differences that remain (remake with `--subject`, same cameras)

`crates/aa-world/tools/lighting_fit.py render --tag T` renders these views and compares named boxes (`docs/aa-world.md`, Fit harness).

* Fog: gone, as in the real client.
* Floor: the blue fill light `Light_205` (radius 15, 6 m above the spawn) lights it. With the local lights in the ambient's unit it is
  0.4 to 0.5 stops darker than the real floor and blue grey (it was 1.9 to 2.1 stops darker and yellow green).
* Walls beside the braziers: 1.1 to 1.2 stops too bright (Bevy's inverse-square near field against the engine's flat falloff).
* Sky: navy overhead with a green band at the horizon (the engine's night sky gradient, `Night sky: Zenith shift`); high sky +0.35 to
  +0.6 stops (it was +1.6 to +3.2 and green). The clouds are brighter but still about 2 stops darker and greener than the dense white
  clouds of the captures.
* Sea: real bright turquoise, remake grey: the reflection of the pale horizon dominates, more body colour only adds blue.
