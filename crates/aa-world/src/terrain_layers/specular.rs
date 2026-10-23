//! Gloss admission preserves linear RGB and alpha without displacing diffuse/normal slots.
use super::*;

pub(super) fn load_gloss(path: &str) -> Option<Layer> {
    let bytes = aa_cry::read(path).ok()?;
    prepare_gloss(&Dds::parse(&bytes).ok()?)
}

fn prepare_gloss(dds: &Dds<'_>) -> Option<Layer> {
    let kind = Kind::Gloss;
    if dds.faces != 1
        || dds.depth != 1
        || dds.width != dds.height
        || !dds.width.is_power_of_two()
        || !matches!(dds.format, DdsFormat::Bc1 | DdsFormat::Bc3)
        || dds.width < 256
    {
        return None;
    }
    let copies = dds.width == 256;
    let skip = if copies {
        0
    } else {
        (dds.width / kind.px()).trailing_zeros()
    };
    let mips = kind.mips() - u32::from(copies);
    if dds.mips < skip + mips {
        return None;
    }
    let mut data = Vec::with_capacity(kind.layer_bytes());
    let mut level = Vec::new();
    for mip in skip..skip + mips {
        let src = &dds.data[dds.surface_range(0, mip)];
        level.clear();
        if dds.format == DdsFormat::Bc3 {
            level.extend_from_slice(src);
        } else {
            // BC1's RGB block is unchanged; opaque alpha is exact for these gloss maps.
            for block in src.as_chunks::<8>().0 {
                let a = u16::from_le_bytes([block[0], block[1]]);
                let b = u16::from_le_bytes([block[2], block[3]]);
                // BC3 always uses four RGB colours, BC1 can use three plus black.
                // Only admit a byte-preserving promotion when every used colour agrees.
                if a <= b {
                    let bits = u32::from_le_bytes(block[4..8].try_into().unwrap());
                    for i in 0..16 {
                        let index = (bits >> (2 * i)) & 3;
                        if (index == 2 && a != b) || (index == 3 && (a != 0 || b != 0)) {
                            return None;
                        }
                    }
                }
                level.extend_from_slice(&[255, 255, 0, 0, 0, 0, 0, 0]);
                level.extend_from_slice(block);
            }
        }
        if copies {
            let row = ((dds.width >> mip).max(4) / 4) as usize * 16;
            for _ in 0..2 {
                for r in level.chunks(row) {
                    data.extend_from_slice(r);
                    data.extend_from_slice(r);
                }
            }
        } else {
            data.extend_from_slice(&level);
        }
    }
    if copies {
        data.extend_from_slice(&level);
    }
    debug_assert_eq!(data.len(), kind.layer_bytes());
    Some(Layer {
        kind,
        copies,
        ati2_swapped: false,
        bc1_normal: false,
        data,
    })
}

pub(super) fn update_light(
    env: Option<Res<crate::sky::Environment>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let Some(env) = env else { return; };
    let multiplier = env.sample.as_ref().map_or(1.0, |e| e.sun_specular);
    let changed: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.params.specular_light.x != multiplier)
        .map(|(id, _)| id)
        .collect();
    for id in changed {
        materials
            .get_mut(id)
            .unwrap()
            .extension
            .params
            .specular_light
            .x = multiplier;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gloss_arrays_preserve_blocks_and_overflow_independently() {
        if !crate::pak_available() {
            return;
        }
        let path = "textures/aa_terrain/artificial/tr_af_tile_801a_sp.dds";
        let bytes = aa_cry::read(path).unwrap();
        let dds = Dds::parse(&bytes).unwrap();
        let layer = prepare_gloss(&dds).unwrap();
        let skip = (dds.width / 512).trailing_zeros();
        let source = &dds.data[dds.surface_range(0, skip)];
        if dds.format == DdsFormat::Bc1 {
            assert_eq!(&layer.data[8..16], &source[..8]);
            assert_eq!(&layer.data[..8], &[255, 255, 0, 0, 0, 0, 0, 0]);
        } else {
            assert_eq!(&layer.data[..source.len()], source);
        }
        let mut counts = [0; LAYER_ARRAYS];
        counts[Kind::Gloss as usize] = MAX_LAYERS;
        assert_eq!(
            place(&mut counts, Kind::Gloss, true).map(|(k, t)| (k, t.code())),
            Some((Kind::GlossMore, 3000.0))
        );
        counts[Kind::GlossMore as usize] = MAX_LAYERS;
        assert!(place(&mut counts, Kind::Gloss, false).is_none());
        assert_eq!(&counts[..5], &[0; 5]);
    }

    #[test]
    fn three_colour_bc1_cannot_silently_change_authored_gloss() {
        if !crate::pak_available() { return; }
        let bytes = aa_cry::read("textures/aa_terrain/artificial/tr_af_pavingstone_001c_sp.dds").unwrap();
        let mut dds = Dds::parse(&bytes).unwrap();
        assert_eq!(dds.format, DdsFormat::Bc1);
        assert_eq!(dds.width, 256);
        let mut blocks = dds.data.to_vec();
        // Selector2 is the 3-colour midpoint; BC3 would instead use 2/3 of endpoint0.
        blocks[..8].copy_from_slice(&[0,0,255,255,2,0,0,0]);
        dds.data = &blocks;
        assert!(prepare_gloss(&dds).is_none());
    }

    #[test]
    fn authored_specular_and_matte_layers_reach_the_cell_table() {
        if !crate::pak_available() {
            return;
        }
        let built = LayerLibrary::build("main_world", [(13, 10), (15, 15)].into_iter()).unwrap();
        let mut positive = 0;
        let mut zero = 0;
        for m in &built.library.mats {
            for p in m.proj.iter().flatten() {
                if p.specular.xyz().max_element() > 0.0 {
                    positive += 1;
                } else {
                    zero += 1;
                }
            }
        }
        assert!(positive > 0 && zero > 0);
        for cell in [(13, 10), (15, 15)] {
            let params = built.library.params(cell, Vec3::ONE).unwrap();
            for (sid, (mat, _)) in built.library.cells[&cell].iter().enumerate() {
                let Some(m) = built.library.mats.get(*mat as usize) else {
                    continue;
                };
                for (p, projection) in m.proj.iter().enumerate() {
                    if let Some(projection) = projection {
                        assert_eq!(params.specular[(sid * 3 + p) * 2], projection.specular);
                        assert_eq!(
                            params.specular[(sid * 3 + p) * 2 + 1].x,
                            projection.gloss.map_or(-1.0, Tex::code)
                        );
                    }
                }
            }
        }
        let matte =
            load_material("materials/aa_terrain/artificial/tr_af_pavingstone_003a").unwrap();
        assert_eq!(matte[2].as_ref().unwrap().specular.xyz(), Vec3::ZERO);
        let plain =
            load_material("materials/aa_terrain/canyon/tr_canyon_mineralrock3d_709a").unwrap();
        assert!(
            plain[0].as_ref().unwrap().specular.x > 0.0
                && plain[0].as_ref().unwrap().gloss.is_none()
        );
        let unsupported = load_material("materials/aa_terrain/mix/river_mud_wet").unwrap();
        assert_eq!(unsupported[2].as_ref().unwrap().specular, Vec4::ZERO);
    }
}
