//! Cheap rebuilds of the preview character while a slider or colour moves.
//!
//! `CharacterDesc::resolve` costs 50-70 ms (database lookups, the `.cdf` files of body, face and
//! hair from the pak), nearly all of it for the structure of the character: which skeleton, which
//! parts, which skin and hair material. That only changes with race, gender, hair style, hair
//! colour and skin colour. Everything the customization screens edit besides those (face sliders,
//! decals, normal map, lip / iris / eyebrow / deco colours) only changes the face texture bake and
//! the morph, which [`customization_parts`] recomputes in well under a millisecond. The cache keeps
//! the resolved structure and refills those fields (`resolve_matches_cache` in the tests keeps the
//! copy of aa-anim's `face_customization` honest).

use aa_anim::db::GameDb;
use aa_anim::face::{FaceLayer, abgr};
use aa_anim::{CharacterAssets, CharacterDesc, Gender, Race};

type Key = (Race, Gender, Option<u32>, Option<u32>, u32);

fn key(desc: &CharacterDesc) -> Key {
    (desc.race, desc.gender, desc.hair, desc.hair_color, desc.skin)
}

/// `(face layers, lip mask + colour, face normal map + weight, iris colour)` of a customization.
type Parts = (Vec<FaceLayer>, Option<(String, [u8; 3])>, Option<(String, f32)>, Option<[u8; 3]>);

/// What `CharacterDesc::resolve` derives from the customization (aa-anim `face_customization`).
pub fn customization_parts(desc: &CharacterDesc, db: &GameDb) -> Parts {
    let c = &desc.customization;
    let face_dir = format!("objects/characters/{}/{}/face", desc.race.folder(), desc.gender.folder());
    let prefix = format!("{}_{}", desc.race.prefix(), desc.gender.letter());
    let mut layers: Vec<(u32, FaceLayer)> = Vec::new();
    for d in &c.decals {
        let Ok(asset) = db.face_decal(d.asset_id) else { continue };
        let tint = match asset.category {
            4 if c.eyebrow_color != 0 => Some(abgr(c.eyebrow_color)),
            5 if c.deco_color != 0 => Some(abgr(c.deco_color)),
            _ => None,
        };
        let (mx, my) = if asset.movable { (d.move_x, d.move_y) } else { (0, 0) };
        layers.push((
            asset.category,
            FaceLayer {
                path: asset.path,
                x: (asset.x + mx) as f32,
                y: (asset.y + my) as f32,
                scale: if d.scale > 0.0 { d.scale } else { 1.0 },
                rotate: d.rotate,
                weight: if d.weight > 0.0 { d.weight } else { 1.0 },
                tint,
            },
        ));
    }
    // drawing order: tattoo, makeup, scar, eyebrow, deco
    let rank = |cat: u32| match cat {
        2 => 0,
        3 => 1,
        1 => 2,
        4 => 3,
        _ => 4,
    };
    layers.sort_by_key(|(cat, _)| rank(*cat));
    let lips = (c.lip_color != 0).then(|| (format!("{face_dir}/{prefix}_face_mask_new.dds"), abgr(c.lip_color)));
    let normal = (c.normal_map != 0)
        .then(|| db.face_normal_map(c.normal_map).ok())
        .flatten()
        .filter(|(n, _)| !n.is_empty())
        .map(|(n, _)| (n, c.normal_map_weight));
    let pupil = (c.left_pupil_color != 0).then(|| abgr(c.left_pupil_color));
    (layers.into_iter().map(|(_, l)| l).collect(), lips, normal, pupil)
}

/// The last resolved structure of the preview character.
#[derive(Default)]
pub struct PreviewCache {
    base: Option<(Key, CharacterAssets)>,
}

impl PreviewCache {
    /// The assets of `desc`: the cached structure with the customization refilled, or a full
    /// resolve when race, gender, hair, hair colour or skin changed.
    pub fn assets(&mut self, desc: &CharacterDesc, db: &GameDb) -> aa_anim::Result<CharacterAssets> {
        let k = key(desc);
        if let Some((cached, base)) = &self.base {
            if *cached == k {
                let mut a = base.clone();
                let (layers, lips, normal, pupil) = customization_parts(desc, db);
                a.desc = desc.clone();
                a.face_layers = layers;
                a.face_lips = lips;
                a.face_normal = normal;
                a.pupil_color = pupil;
                return Ok(a);
            }
        }
        let a = desc.resolve(db)?;
        self.base = Some((k, a.clone()));
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::login::create::look::Customizer;

    #[test]
    #[ignore = "timing probe, needs the game data"]
    fn cached_cost() {
        let cz = Customizer::open(Race::Nuian, Gender::Male).unwrap();
        let mut cache = PreviewCache::default();
        let desc = cz.desc();
        for i in 0..5 {
            let t = web_time::Instant::now();
            let a = cache.assets(&desc, &cz.db).unwrap();
            println!("assets {i}: {:.2} ms, {} layers", t.elapsed().as_secs_f32() * 1000.0, a.face_layers.len());
        }
        let t = web_time::Instant::now();
        let _ = customization_parts(&desc, &cz.db);
        println!("customization_parts: {:.2} ms", t.elapsed().as_secs_f32() * 1000.0);
    }

    /// The cached path gives the same assets as a full resolve, for looks that touch every field the
    /// cache refills.
    #[test]
    #[ignore = "needs the game data"]
    fn resolve_matches_cache() {
        for (race, gender) in [(Race::Nuian, Gender::Male), (Race::Elf, Gender::Female), (Race::Ferre, Gender::Male)] {
            let mut cz = Customizer::open(race, gender).unwrap();
            let mut cache = PreviewCache::default();
            let d0 = cz.desc();
            cache.assets(&d0, &cz.db).unwrap();
            // a look with decals of every kind, colours, wrinkles and different sliders
            cz.look.tattoo = 2;
            cz.look.makeup = (3, 0.6);
            cz.look.eyebrow = 4;
            cz.look.deco = (2, 0.7);
            cz.look.scar.index = 2;
            cz.look.scar.x = 20.0;
            cz.look.scar.scale = 1.5;
            cz.look.wrinkle = (2, 0.5);
            cz.look.eyebrow_color = 0xFF2040A0;
            cz.look.deco_color = 0xFF10F0A0;
            cz.look.lip_color = 0xFF3030D0;
            cz.look.pupil_color = 0xFFD0A020;
            cz.look.modifier[5] = 40;
            let d1 = cz.desc();
            let cached = cache.assets(&d1, &cz.db).unwrap();
            let full = d1.resolve(&cz.db).unwrap();
            assert_eq!(format!("{:?}", cached.face_layers), format!("{:?}", full.face_layers), "{race:?} {gender:?} layers");
            assert_eq!(cached.face_lips, full.face_lips, "{race:?} {gender:?} lips");
            assert_eq!(cached.face_normal, full.face_normal, "{race:?} {gender:?} normal");
            assert_eq!(cached.pupil_color, full.pupil_color, "{race:?} {gender:?} pupil");
            assert_eq!(cached.desc, full.desc);
            assert_eq!(cached.parts.len(), full.parts.len());
            assert!(!full.face_layers.is_empty());
        }
    }
}
