//! Development flags for unattended runs.
//!
//! * `--pos x,y[,z]` / `--yaw DEG`  start the player there (game coordinates, yaw counter-clockwise
//!   from north) instead of where the server saved it; the first movement packet moves the server
//!   side character too. `--yaw` alone only turns it.
//! * `--walk START:END[:KEYS]`  press KEYS (default `W`, any of `WASDQE` and `J` for jump, `S`hift is
//!   `H`) from START to END seconds after the player appeared. Repeatable.
//! * `--hour H`, `--zone ID`  time of day (aa-world's `WorldClock`); without `--hour` the hour follows the server's game clock.
//! * `--hair-color ID`  `hair_colors.id` to use instead of the one the server saved.
//! * `--hide-part N`  hide the N-th part of the player character (0 = body, 1 = face, ...)
//! * `--cam-yaw DEG`, `--cam-pitch DEG`, `--cam-dist M`  the starting camera orbit.
//! * `--leave-after S`  go back to the character list S seconds after the player appeared (client side only).
//! * `--no-prewarm`  do not compile the world's pipelines on the character list (`prewarm`).
//! * `--start-after S`  press Start S seconds after the character list came up (every visit; `--goto` presses it itself
//!   after 2 s).
//! * `--leave-world S`  ask the server to go back to the character list S seconds after the game screen came up
//!   (the real leave flow); `--again` presses Start when the list is back (see `flow`).

use bevy::prelude::*;

#[derive(Resource, Default, Debug, Clone)]
pub struct DevOptions {
    pub pos: Option<Vec3>,
    pub yaw: Option<f32>,
    pub walks: Vec<Walk>,
    pub hour: Option<f32>,
    pub zone: Option<u16>,
    pub cam_yaw: Option<f32>,
    pub cam_pitch: Option<f32>,
    pub cam_dist: Option<f32>,
    pub hide_part: Option<usize>,
    pub hair_color: Option<u32>,
    pub leave_after: Option<f32>,
    pub leave_world: Option<f32>,
    pub again: bool,
    pub no_prewarm: bool,
    pub start_after: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct Walk {
    pub start: f32,
    pub end: f32,
    pub keys: String,
}

impl Walk {
    pub fn pressed(&self, t: f32, key: char) -> bool {
        t >= self.start && t < self.end && self.keys.contains(key)
    }
}

impl DevOptions {
    pub fn from_args() -> DevOptions {
        let args: Vec<String> = std::env::args().collect();
        let all = |name: &str| -> Vec<String> {
            args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1).cloned()).collect()
        };
        let one = |name: &str| all(name).into_iter().next();
        let num = |name: &str| one(name).and_then(|v| v.parse::<f32>().ok());
        let pos = one("--pos").and_then(|v| {
            let n: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            (n.len() >= 2).then(|| Vec3::new(n[0], n[1], n.get(2).copied().unwrap_or(f32::NAN)))
        });
        let walks = all("--walk")
            .iter()
            .filter_map(|w| {
                let mut it = w.split(':');
                Some(Walk {
                    start: it.next()?.parse().ok()?,
                    end: it.next()?.parse().ok()?,
                    keys: it.next().unwrap_or("W").to_ascii_uppercase(),
                })
            })
            .collect();
        DevOptions {
            pos,
            yaw: num("--yaw").map(f32::to_radians),
            walks,
            hour: num("--hour"),
            zone: num("--zone").map(|z| z as u16),
            cam_yaw: num("--cam-yaw").map(f32::to_radians),
            cam_pitch: num("--cam-pitch").map(f32::to_radians),
            cam_dist: num("--cam-dist"),
            hide_part: num("--hide-part").map(|n| n as usize),
            hair_color: one("--hair-color").and_then(|v| v.parse().ok()),
            leave_after: num("--leave-after"),
            leave_world: num("--leave-world"),
            again: args.iter().any(|a| a == "--again"),
            no_prewarm: args.iter().any(|a| a == "--no-prewarm"),
            start_after: num("--start-after"),
        }
    }

    /// Is the scripted key held `t` seconds after the player appeared?
    pub fn scripted(&self, t: f32, key: char) -> bool {
        self.walks.iter().any(|w| w.pressed(t, key))
    }
}
