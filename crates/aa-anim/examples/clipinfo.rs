//! Prints the tracks of clips: `cargo run -p aa-anim --example clipinfo -- <skeleton .chr> <clip .caf>...`
//! (bones driven, their depth, first-key root position): which part of the body a clip animates.

use aa_anim::{Clip, pose::Rig};
use aa_cry::Skeleton;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(skeleton) = args.first() else {
        eprintln!("usage: clipinfo <skeleton .chr> <clip .caf>...");
        return;
    };
    let rig = Rig::new(Skeleton::load(skeleton).expect("skeleton"));
    println!("{skeleton}: {} bones", rig.len());
    for path in &args[1..] {
        let clip = match Clip::load(path) {
            Ok(c) => c,
            Err(e) => {
                println!("{path}: {e}");
                continue;
            }
        };
        let binding = rig.bind_clip(&clip);
        let mut names: Vec<String> = binding.pairs.iter().map(|&(b, _)| rig.skeleton.bones[b as usize].name.clone()).collect();
        names.sort();
        println!("{path}: {:.2} s, {} tracks, {} bound", clip.duration, clip.tracks.len(), binding.pairs.len());
        println!("  {}", names.join(", "));
    }
}
