# Time of day: from the server's game clock to the sky

Date: 2026-10-17. Read-only research (no cargo, no process touched, `<client>` read only). Question: why do the real-client captures show
full daylight at `e_time_of_day` 19.17 h (spawn) and 2.7 to 4.3 h (combat) while zone 179's time-of-day splines are night at those hours?

## Answer in one paragraph

Nothing is remapped between the server and the engine: the capture hour **is** the server's game hour. The remap happens inside the
engine, and only for the colour splines. The engine's time of day manager (`SetTime(hour)`) stores the raw hour (sun path, `e_time_of_day`, HUD clock, NPC
schedules all use it) but hands every zone layer a **warped** hour that moves the world's real sunrise/sunset (`lighting.xml`
`DawnTime` / `DuskTime`, main_world 2:00 and 22:00) onto the splines' authored 6:00 and 18:00. The layer then reads its splines at
`warped / 24`. For main_world this turns 12:00 to 19:03 into a constant spline hour 12.0 (noon), and 2.7 to 4.3 h into 6.7 to 7.9 (morning).
So `aa-world` has to evaluate the splines at `spline_hour(hour)` instead of `hour`, and keep the raw hour for everything else.

## 1. The mapping, with evidence

### 1.1 Server to client (exact)

| Step | What | Where |
|---|---|---|
| Clock | starts at 12:00 when the server boots; every 10 s it adds `TICK_DELAY * 10` = 59.9976 game seconds (6 game seconds per real second, 0.0016666 h/s; a day is 4 real hours); wraps at 24 h. Not configurable: constants, no config key | `<aaemu-src>\rust\crates\aaemu-game\src\managers\time.rs:13-15, 57, 121-138, 141-155` |
| Join | `SCDetailedTimeOfDay` (0x0f1) on `CSSpawnCharacter`: `time` (h), `speed` 0.0016666, `start` 0, `end` 24 | `time.rs:96`, `packets\world.rs:583-600`, `world\session.rs:628` |
| Every 10 s | `SCTimeOfDay` (0x0f0): `time` (h, f32). A per-character override exists only for the storm-ship buff (2.0 h), not relevant | `time.rs:135-137`, `packets\world.rs:602-616`, `opcodes.rs:765-766` |
| Client state | `aa_net::state::TimeOfDay { hour, speed, received }`, `hour_now()` extrapolates at `speed`; `start`/`end` are skipped | `crates/aa-net/src/game.rs:742-749`, `state/world.rs:11-30, 66-69` |
| GM | `time set hour=H [minute=M]` sets the clock; it does not push, clients see it at the next 10 s tick | `...\systems\services\commands\sub.rs:391-440` |

`sv_timeofdaylength / start / enable` in the client's cvar lists (`scriptsbin/network/cvars.txt`) are CryEngine game-rules leftovers; the AAEmu
server never sends them.

### 1.2 Server hour = `e_time_of_day` = HUD clock (verified numerically)

`rs-game.log` line 1: the server started `2026-10-16 14:31:06`, one run, no restart since. Predicting the capture hour as
`12 + 0.0016666 h/s x (file write time - 14:31:06)` mod 24 for every `camera*.json`:

| Capture | Recorded `e_time_of_day` | Predicted | Diff (real s) |
|---|---|---|---|
| spawn (10-16 15:42:49) | 19.166 | 19.173 | -4 |
| character_info / bag / worldmap (23:18 to 23:19) | 16.682 / 16.739 / 16.785 | 16.691 / 16.748 / 16.794 | -5 |
| options_keybinds (23:28:57) | 17.774 | 17.783 | -5 |
| royal_palace (10-17 02:06 to 02:11) | 9.563 / 9.987 / 9.998 | 9.574 / 9.995 / 10.005 | -5 to -6 |
| combat_* (08:58 to 09:14) | 2.678 .. 4.305 | 2.686 .. 4.332 | -10 to -24 |

All within the delay between the cvar read and the file write, over 17.5 hours of wall time: no offset, no scale, no separate clock. The
HUD agrees: `clock.lua` (`data/extract/aa-lua/decompiled/x2ui/hud/clock.lua:19-76`) places the sun / moon from `X2Time:GetGameTime()` with
`minute = 210 .. 1230` (3:30 to 20:30) as day. Inverting the sun drawable offsets in `reference/ingame/*/ui.json` (`clockBar.sun`
`eoffset.x` 1877.9, 1858.9, 1867.8) gives 19.09, 16.68, 17.77 h, equal to the recorded `e_time_of_day`. Moreover
`reference/ingame/combat_melee/real_05.png` shows the HUD bar at night (moon, 2.68 h) over a fully lit scene: direct proof that the 3D
splines are not read at the raw hour.

### 1.3 Inside the engine (1.2 client binary, `<client>\bin32\cry3dengine.dll`, base 0x30000000)

Read with capstone (the in-game `x2game.dll` is encrypted on disk, so the packet handler itself was not found; see 4). The x64 dedicated
build's decompile (`<re-notes>\zone\out\cry3dengine_dedicate.c`, 10.x, only used as a readable cross-reference) has the same functions.

* `0x300A46B0` = the manager's `SetTime(hour, force)` (vtable `0x3025B518`, slot 5; dedicated `sub_300ADD60`, .c:120746):
  `this->time = hour; *(3dEngine + 0x694) = hour` (that field is the `e_time_of_day` cvar, registered at `0x3000FC7E`), then for every layer in
  the manager's buffer (max 10, `tod buffer full`, `0x300A4270`, one layer per loaded zone, filled by
  `SeamlessSystem::UpdateTimeOfDay` in `crysystem.dll`) calls `layer->SetTime(vtbl[14](hour))`, then `Update`.
* `vtbl[14]` = `0x300A49D0` (dedicated `sub_300AE3A0`, .c:121000) is the warp, `vtbl[13]` = `0x300A48C0` its inverse. Both use
  `p = (engine[0x310] + engine[0x314]) / 2` = `DawnTime / 60` (hours), written by the `lighting.xml` parser at `0x30140700..0x30140890`
  from `<Lighting DawnTime DawnDuration DuskTime DuskDuration>` (dedicated .c:239849+). Dusk is not read: the second half assumes the mirror
  image, `24 - p`.
* `layer->SetTime(x)` = `0x300A4170`: `layer.time = x`, then `Update`; the layer `Update` (`0x300A4C10`) reads its splines at
  `layer.time x (1/24)` (`0x300A4CB8`, constant `0x3025B05C` = 0.0416667).
* The sun direction (`CTimeOfDay::Update`, dedicated `sub_300AECE0`, .c:121823) reads `this->time` = the **raw** hour:
  `(hour + 12) / 24 turn`. Only an editor flag of a layer (x64 build `layer + 0x4E`) replaces it; never set in game. This is what
  `aa_world::env::sun_direction` implements and what the palace captures confirmed (docs/aa-world.md: best lit-patch match 123 / 42 at
  9.56 h and 132 / 44 at 9.998 h against the formula's 124.0 / 42.4 and 131.8 / 44.4).

The warp, per half day `u` in 0..1 (all values in hours, `bend(a) = (a/12 - 0.5) * -4` if `a/12 < 1`, else `w = a/12 - 1`,
`(2 - 4w) / (4w^2 - 4w)`; the engine clamps the quadratic at 0):

```
h < 12:  k = bend(p);       u = h / 12;        y = (sqrt((1-k)^2 + 4 k u) + k - 1) / (2 k)        (inverse of k u^2 + (1-k) u)
         spline_hour = max(y, 0) * 12
h >= 12: k = bend(24 - p);  u = (h - 12) / 12; y = k u^2 + (1-k) u
         spline_hour = (max(y, 0) + 1) * 12
k == 0 (p = 6) is the identity.
```

`p` per world (`worlds/<w>/lighting.xml`): main_world `DawnTime 119` (p = 1.9833; dawn 2:00, dusk 22:00); login2 and almost every instance
`360` (identity: create step 3's 6.17 h and 18.03 h are real spline hours); instance_prologue 315; instance_library 211; arche_mall 371.

main_world, game hour to spline hour (script `data/extract/research-tod/warp.py`):

| Game hour | 0 | 1 | 2 | 2.7 | 3.8 | 4.3 | 6 | 9.56 | 10 | 12..19.05 | 19.17 | 20 | 21 | 22 | 23 | 24 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Spline hour | 3.04 | 4.88 | 6.02 | 6.67 | 7.55 | 7.91 | 9.01 | 10.90 | 11.11 | 12.00 | 12.17 | 13.53 | 15.54 | 17.96 | 20.78 | 24 |

Sunrise (sun at the horizon, raw 2:00) lands on spline 6.0 and sunset (22:00) on 18.0, which is the design intent. The flat noon from 12:00 to
19:03 is the engine's own clamp (the second-half quadratic is negative for `u < 0.59`); it is reproduced here as it is and the captures
agree (character_info 16.68, options 17.77 and spawn 19.17 look the same).

Zone 179 sun colour, `data/extract/research-tod/tod_eval.py`: read at the raw hour it is night blue (lum 0.34) at 2.7 to 4.3 h and 19.17 h, orange
at 16.68 and dark red at 17.77; read at the warped hour it is dawn warm (0.93 0.83 0.60) at 2.7 h rising to white (0.98 0.91 0.79) at 4.3 h,
and white (1.00 0.96 0.92, x 6.2) at 16.68, 17.77 and 19.17. That is what the captures show.

## 2. Is the capture hour metadata trustworthy?

Yes: `e_time_of_day` in every `camera*.json` is the server game hour to within 0.02 h (about 6 to 10 s), see 1.2; the README note "follows the server
clock" is exact. The `tod` values in the combat captures are the same quantity. What was misleading is only the assumption that the spline hour
equals it. Real hours and the spline hour the picture corresponds to (main_world):

| Capture | Game hour | Spline hour | Raw sun (az / el) |
|---|---|---|---|
| spawn | 19.166 | 12.16 | 287.5 / 13.2 |
| character_info, bag, worldmap_zone | 16.682, 16.739, 16.785 | 12.00 | 255 / 29 |
| options_keybinds | 17.774 | 12.00 | 269.5 / 21.9 |
| royal_palace | 9.563, 9.987, 9.998 | 10.90, 11.1 | 124 / 42, 132 / 44 |
| combat_melee, death, triple_slash | 2.68, 2.69, 2.82 | 6.65, 6.66, 6.77 | 28.5 / 2.4 .. 30.4 / 2.9 |
| combat_kick, return, death_2 | 3.59, 3.76, 4.32 | 7.39, 7.52, 7.92 | 40.5 / 6.4 .. 49.8 / 10.2 |
| create step 3 (login2, identity) | 6.17, 18.03 | same | per login2 lighting |
| server select / char select | 12.04 to 12.13, 13 | not from the server (no game connection yet) | |

The aa-world "calibration hour 10" is a spline hour (game 7.8 h); the spawn is at spline 12.16 and the day colours barely move between 9 and 15, so
the existing tuning stays roughly valid, but `AtmosphereTuning` and the terrain tint were fitted without the warp (and with the sun of hour 10, not
of 19.17): re-fit at `--hour 19.166` after the change.

## 3. Recommendation

`sync_clock` (`crates/aa-client/src/ingame/mod.rs:133-146`) needs no formula: keep `clock.hour = WorldInfo::hour_now()` (raw). The raw
hour is also what the HUD clock (`hud/api.rs:389-392`), the sound areas' `on_at` (`aa-world/src/objects/sounds.rs:267`), the sun path and the
NPC / doodad schedules want. The warp belongs in `aa-world`, on the spline lookup only (aa-world's owner; I did not touch the crate):

* `Lighting` gets `dawn_hour = DawnTime / 60` (default 355 / 60, the engine's default; `Lighting::parse` already reads the file, `sky.rs:338`).
* `TimeOfDay::sample` (`env.rs:294`) changes `let t = hour / 24.0;` to `let t = spline_hour(hour, lighting.dawn_hour) / 24.0;`.
  `sun_direction(hour, lighting)` (`env.rs:325`) keeps the raw hour. `Environment.hour` (`sky.rs:482`) keeps the raw hour.

```rust
/// The hour the zone's time of day splines are read at for the game hour `hour`
/// (the time of day manager's `SetTime` -> `vtbl[14]`, cry3dengine.dll 0x300A49D0). `dawn` = lighting.xml `DawnTime / 60`
/// (main_world 119 -> 1.9833; 6.0 is the identity, login2 and most instances).
pub fn spline_hour(hour: f32, dawn: f32) -> f32 {
    // bend of the half day quadratic y = k u^2 + (1 - k) u; `anchor` in hours
    fn bend(anchor: f32) -> f32 {
        let v = anchor / 12.0;
        if v >= 1.0 {
            let w = v - 1.0;
            (2.0 - 4.0 * w) / (4.0 * w * w - 4.0 * w)
        } else {
            (v - 0.5) * -4.0
        }
    }
    let h = hour.rem_euclid(24.0);
    if h < 12.0 {
        let (u, k) = (h / 12.0, bend(dawn));
        let y = if k.abs() < 1e-6 { u } else { (((1.0 - k).powi(2) + 4.0 * k * u).sqrt() + k - 1.0) * 0.5 / k };
        y.max(0.0) * 12.0
    } else {
        let (u, k) = ((h - 12.0) / 12.0, bend(24.0 - dawn));
        let y = if k.abs() < 1e-6 { u } else { u * u * k + (1.0 - k) * u };
        (y.max(0.0) + 1.0) * 12.0
    }
}
```

Test vectors (dawn = 119.0 / 60.0): `0 -> 3.04`, `2.0 -> 6.02`, `2.7 -> 6.67`, `9.56 -> 10.90`, `12 -> 12.0`, `16.68 -> 12.0`, `19.17 -> 12.17`,
`22 -> 17.96`; `dawn = 6.0`: `h -> h` for every h.

The server needs to send nothing new: hour, speed and start / end already arrive, and dawn / dusk are client data. Server-side notes only:
the clock is 12:00 after every server restart (so the sky jumps to noon), the day length is a constant, and `time set` is visible only
after the next 10 s push (a push on `set` would make GM-driven captures instant).

## 4. What is unproven

* The packet handler: where `SCTimeOfDay` / `SCDetailedTimeOfDay` call `SetTime` / `SetAdvancedInfo` in `x2game.dll` was not located (the file on disk is
  encrypted, IDA was not connected). It is inferred: `e_time_of_day` (written by `SetTime`) equals the server hour to a few seconds. The client also
  advances the clock itself between pushes (recorded hours are not multiples of the 10 s tick), consistent with `speed` feeding
  `e_time_of_day_speed` (`0x300A4650`, `0x3000DE10`); `hour_now()` extrapolation matches.
* The plateau and the compression of 19:00 to 22:00 into spline 12 to 18 come from the 1.2 code as read, not from a day sweep. A sweep (below)
  would confirm it.
* The sun direction at the spawn and combat hours is not verified (the palace verifies it at 9.56 to 10.0 h only). The code says raw, i.e. az 288 / el 13
  at the spawn, but `docs/aa-world.md` ("Known gaps") reads the spawn picture as lit from the north-north-east, and the combat fit uses
  `sun_az=290, sun_el=15`, while the raw sun at the combat hours is az 29 to 50, el 2 to 10. Either those readings of the diffuse pictures are off or the
  sun path has another input I did not find. Check `fly --hour 19.166` against `reference/ingame/spawn/real.png` after the warp is in.
* Layer choice: `SeamlessSystem::UpdateEnv` logs `relic layer %s->%s, weather layer %s->%s, layer weight`; aa-world uses layer 0 always. Zone blending by layer weight
  across zone borders is not modelled.

Cheap verification (needs the lane server and a GM account): with the real client in game, run `time set hour=H` for H in 2, 3, 5, 8, 12, 15, 19,
20, 21, 22 and 23 (wait for the next 10 s push), capture a fixed view with `reference/tools/capture.py screen --camera`, and compare with
`fly --hour H` after the change. Expect: identical look 12 to 19, a fast turn from orange to night between 20 and 22, dawn at 2 to 3.
