//! Parses every cell of both worlds (heightmap, cover, placement, water). Slow (about a minute), so ignored:
//! `cargo test -p aa-world --test all_cells -- --ignored --nocapture`

use std::collections::BTreeSet;
use std::sync::Mutex;

use aa_world::cover::Cover;
use aa_world::heightmap::CellHeights;
use aa_world::placement::load_cell_objects;
use aa_world::water::water_mesh;
use aa_world::world::cell_dir;

fn cells_of(world: &str) -> Vec<(i32, i32)> {
    let prefix = format!("worlds/{world}/cells/");
    let mut set = BTreeSet::new();
    for name in aa_pak::Pak::global().list(&prefix) {
        // names come back with the pak's `game/` root
        let rest = name.split_once("/cells/").map_or(name, |(_, r)| r);
        let cell = rest.split('/').next().unwrap_or("");
        if let Some((x, y)) = cell.split_once('_') {
            if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                set.insert((x, y));
            }
        }
    }
    set.into_iter().collect()
}

#[test]
#[ignore]
fn every_cell_parses() {
    if !aa_world::pak_available() {
        return;
    }
    let pak = aa_pak::Pak::global();
    for world in ["login2", "main_world"] {
        let cells = cells_of(world);
        assert!(!cells.is_empty(), "{world}");
        let problems = Mutex::new(Vec::new());
        let stats = Mutex::new((0usize, 0usize, 0usize, 0usize, 0usize)); // heightmaps, covers, brushes, vegetation, water
        let next = Mutex::new(0usize);
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| loop {
                    let i = {
                        let mut n = next.lock().unwrap();
                        *n += 1;
                        *n - 1
                    };
                    let Some(&(x, y)) = cells.get(i) else { break };
                    let dir = cell_dir(world, x, y);
                    let mut local = (0, 0, 0, 0, 0);
                    if let Ok(b) = pak.read(&format!("{dir}/client/terrain/heightmap.dat")) {
                        match CellHeights::parse(&b) {
                            Ok(h) => {
                                local.0 = 1;
                                if !(h.min_z.is_finite() && h.max_z.is_finite() && h.max_z >= h.min_z) {
                                    problems.lock().unwrap().push(format!("{world} {x}_{y}: heights {} {}", h.min_z, h.max_z));
                                }
                            }
                            Err(e) => problems.lock().unwrap().push(format!("{world} {x}_{y}: heightmap {e}")),
                        }
                    }
                    if let Ok(b) = pak.read(&format!("{dir}/client/terrain/cover.ctc")) {
                        match Cover::parse(&b) {
                            Ok(c) => {
                                local.1 = 1;
                                let a = c.atlas(0);
                                if a.data.is_empty() {
                                    problems.lock().unwrap().push(format!("{world} {x}_{y}: empty cover atlas"));
                                }
                            }
                            Err(e) => problems.lock().unwrap().push(format!("{world} {x}_{y}: cover {e}")),
                        }
                    }
                    let o = load_cell_objects(world, x, y);
                    local.2 = o.brushes.len();
                    local.3 = o.vegetation.len();
                    local.4 = o.water.len();
                    if let Some(w) = o.water.iter().find(|w| w.points.len() < 3 || !w.surface.is_finite()) {
                        problems.lock().unwrap().push(format!("{world} {x}_{y}: bad water {:?}", w.bbox));
                    }
                    let _ = water_mesh(&o.water);
                    let mut t = stats.lock().unwrap();
                    t.0 += local.0;
                    t.1 += local.1;
                    t.2 += local.2;
                    t.3 += local.3;
                    t.4 += local.4;
                });
            }
        });
        let t = stats.into_inner().unwrap();
        println!("{world}: {} cells, {} heightmaps, {} covers, {} brushes, {} vegetation, {} water volumes", cells.len(), t.0, t.1, t.2, t.3, t.4);
        let problems = problems.into_inner().unwrap();
        for p in problems.iter().take(30) {
            println!("PROBLEM {p}");
        }
        assert!(problems.is_empty(), "{} problems in {world}", problems.len());
    }
}
