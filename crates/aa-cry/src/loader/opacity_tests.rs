use super::*;
use crate::material::{Material, Texture};
use bevy::asset::AssetPlugin;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler};

const PATH: &str = "textures/p_effects/base_will_pattern.dds";

fn texture(attributes: &str) -> Texture {
    Material::parse(&format!(r#"<Material><Textures><Texture Map="Opacity" File="{PATH}" {attributes}/></Textures></Material>"#))
        .unwrap().textures.remove(0)
}

fn loader(images: &Assets<Image>) -> CryLoader {
    CryLoader::new(
        images.get_handle_provider(),
        Assets::<Mesh>::default().get_handle_provider(),
        Assets::<StandardMaterial>::default().get_handle_provider(),
        Assets::<SkinnedMeshInverseBindposes>::default().get_handle_provider(),
        false,
    )
}

fn finish(prep: Prep<'_>, images: &mut Assets<Image>) {
    for pending in prep.into_pending() {
        if let Pending::Shared(Shared::Image(handle, slot)) = pending
            && let Some(image) = lock(&slot).take()
        {
            images.insert(handle.id(), image).unwrap();
        }
    }
}

#[test]
fn opacity_variants_separate_samplers_encoding_and_pending_delivery() {
    if !crate::pak_available() {
        return;
    }
    let mut images = Assets::<Image>::default();
    let loader = loader(&images);
    let clamp = texture(r#"IsTileU="0" IsTileV="0""#);
    let point = texture(r#"Filter="0" TexType="0""#);
    let linear = texture(r#"Filter="1""#);
    let mut abandoned = Prep::new(&loader);
    let c = abandoned.opacity_texture(&clamp).unwrap();
    let p = abandoned.opacity_texture(&point).unwrap();
    drop(abandoned); // a later preparation must still deliver the shared pending slots
    let mut prep = Prep::new(&loader);
    let old = prep.texture(PATH, false).unwrap();
    let colour = prep.texture(PATH, true).unwrap();
    assert_eq!(prep.opacity_texture(&texture("")).unwrap(), old);
    assert_eq!(
        prep.opacity_texture(&texture(r#"Filter="-1" TexType="0""#))
            .unwrap(),
        old
    );
    assert_eq!(
        prep.opacity_texture(&texture(r#"Filter="7" IsTileV="0""#))
            .unwrap(),
        old
    );
    let mut alias = clamp.clone();
    alias.file = format!("GAME/{}", PATH.to_ascii_uppercase());
    assert_eq!(prep.opacity_texture(&alias).unwrap(), c);
    assert_eq!(prep.opacity_texture(&point).unwrap(), p);
    let l = prep.opacity_texture(&linear).unwrap();
    for (a, b) in [(&old, &colour), (&old, &c), (&old, &p), (&c, &p), (&p, &l)] {
        assert_ne!(a, b);
    }
    finish(prep, &mut images);
    let original = images.get(&old).unwrap();
    let ImageSampler::Descriptor(original_sampler) = &original.sampler else {
        panic!()
    };
    assert_eq!(original_sampler.address_mode_u, ImageAddressMode::Repeat);
    assert_eq!(original_sampler.anisotropy_clamp, 4);
    for h in [&c, &p, &l] {
        let variant = images.get(h).unwrap();
        assert_eq!(
            variant.data, original.data,
            "each variant has the complete original mip payload"
        );
        assert_eq!(
            variant.texture_descriptor.mip_level_count,
            original.texture_descriptor.mip_level_count
        );
        assert_eq!(variant.asset_usage, original.asset_usage);
        assert_eq!(
            variant.texture_descriptor.format,
            original.texture_descriptor.format
        );
    }
    let ImageSampler::Descriptor(clamp_sampler) = &images.get(&c).unwrap().sampler else {
        panic!()
    };
    assert_eq!(clamp_sampler.address_mode_u, ImageAddressMode::ClampToEdge);
    assert_eq!(clamp_sampler.address_mode_v, ImageAddressMode::ClampToEdge);
    let ImageSampler::Descriptor(point_sampler) = &images.get(&p).unwrap().sampler else {
        panic!()
    };
    assert_eq!(point_sampler.min_filter, ImageFilterMode::Nearest);
    assert_eq!(point_sampler.lod_max_clamp, 0.0);
    // Simulate render-only CPU data retirement. Resolving the cached variant must not
    // depend on retained pixels in the original image or recreate a drained pending slot.
    images.get_mut(&old).unwrap().data = None;
    let mut late = Prep::new(&loader);
    assert_eq!(late.opacity_texture(&clamp), Some(c.clone()));
    finish(late, &mut images);
    assert!(images.get(&old).unwrap().data.is_none());
    assert!(images.get(&c).unwrap().data.is_some());
}

#[test]
fn opacity_variant_handles_follow_existing_cache_and_consumer_retirement() {
    if !crate::pak_available() {
        return;
    }
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
        .init_asset::<Image>();
    let loader = loader(app.world().resource::<Assets<Image>>());
    let mut prep = Prep::new(&loader);
    let original = prep.texture(PATH, false).unwrap();
    let variant = prep.opacity_texture(&texture(r#"Filter="0""#)).unwrap();
    let id = variant.id();
    finish(prep, &mut app.world_mut().resource_mut::<Assets<Image>>());
    drop(variant);
    app.update();
    assert!(
        app.world().resource::<Assets<Image>>().contains(id),
        "cache owns its strong handle like original images"
    );
    let mut prep = Prep::new(&loader);
    let consumer = prep.opacity_texture(&texture(r#"Filter="0""#)).unwrap();
    finish(prep, &mut app.world_mut().resource_mut::<Assets<Image>>());
    drop(loader);
    app.update();
    assert!(
        app.world().resource::<Assets<Image>>().contains(id),
        "consumer survives loader retirement"
    );
    drop(consumer);
    for _ in 0..3 {
        app.update();
    }
    let images = app.world().resource::<Assets<Image>>();
    assert!(
        !images.contains(id),
        "variant retires after the cache and last consumer release it"
    );
    assert!(
        images.contains(original.id()),
        "another consumer's original image remains alive"
    );
}
