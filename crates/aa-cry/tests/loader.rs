//! `prepare_model` on a worker thread + `finish_model` on the main thread, the model hooks, and
//! the rule that cube maps are never bound to 2D slots (skipped when the client is missing).

use std::sync::Arc;

use aa_cry::scene::{CryAssets, CryPlugin, LoadedModel, MaterialHook, ModelOptions, VertexHook};
use bevy::asset::AssetPlugin;
use bevy::ecs::system::SystemState;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;
use bevy::render::render_resource::TextureViewDimension;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<Image>()
        .init_asset::<StandardMaterial>()
        .init_asset::<SkinnedMeshInverseBindposes>()
        .add_plugins(CryPlugin);
    app
}

fn all_textures(m: &StandardMaterial) -> Vec<&Handle<Image>> {
    [&m.base_color_texture, &m.emissive_texture, &m.metallic_roughness_texture, &m.normal_map_texture, &m.occlusion_texture]
        .into_iter()
        .flatten()
        .collect()
}

/// Every handle of the model points at an inserted asset, and no 2D slot holds a cube map.
fn check_inserted(cry: &CryAssets, loaded: &LoadedModel) -> usize {
    let mut parts = 0;
    for part in loaded.parts.iter().flat_map(|(_, p)| p) {
        parts += 1;
        assert!(cry.meshes.get(&part.mesh).is_some(), "mesh missing");
        let m = cry.materials.get(&part.material).expect("material missing");
        for t in all_textures(m) {
            let img = cry.images.get(t).expect("texture missing");
            let cube = img.texture_view_descriptor.as_ref().is_some_and(|d| d.dimension == Some(TextureViewDimension::Cube));
            assert!(!cube, "cube map in a 2D slot");
        }
    }
    parts
}

#[test]
fn prepare_on_a_worker_finish_on_main() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let loader = state.get_mut(app.world_mut()).unwrap().loader();
    let path = "objects/env/02_harihara/001_hdeco/hdeco_table_h.cgf";
    let prepared = std::thread::spawn(move || loader.prepare_model(path, &ModelOptions::default())).join().unwrap().expect("prepare");
    // handles are usable before anything is inserted
    assert!(!prepared.loaded().parts.is_empty());
    let mut cry = state.get_mut(app.world_mut()).unwrap();
    assert!(cry.meshes.is_empty());
    let loaded = cry.finish_model(prepared);
    assert!(check_inserted(&cry, &loaded) > 0);
    // the synchronous API shares the cache with the loader
    let again = cry.load_model(path, &ModelOptions::default()).unwrap();
    assert!(Arc::ptr_eq(&loaded, &again));
}

#[test]
fn hooks_edit_materials_and_vertices() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let mut cry = state.get_mut(app.world_mut()).unwrap();
    let path = "objects/env/02_harihara/001_hdeco/hdeco_table_h.cgf";
    let plain = cry.load_model(path, &ModelOptions::default()).unwrap();
    let options = ModelOptions {
        material_hook: Some(MaterialHook::new(|_, m| m.base_color = Color::srgb(1.0, 0.0, 0.0))),
        vertex_hook: Some(VertexHook::new(|mesh| mesh.positions.iter_mut().for_each(|p| p.z += 10.0))),
        ..default()
    };
    let hooked = cry.load_model(path, &options).unwrap();
    assert!(!Arc::ptr_eq(&plain, &hooked), "hooked loads are not shared");
    let part = &hooked.parts[0].1[0];
    assert_eq!(cry.materials.get(&part.material).unwrap().base_color, Color::srgb(1.0, 0.0, 0.0));
    let plain_part = &plain.parts[0].1[0];
    assert_ne!(cry.materials.get(&plain_part.material).unwrap().base_color, Color::srgb(1.0, 0.0, 0.0));
    assert!(part.aabb.center.z > plain_part.aabb.center.z + 9.0, "vertex hook applied");
}

#[test]
fn original_alpha_glow_subset_keeps_source_rgba_in_basic_and_full() {
    if !aa_cry::pak_available() { return; }
    let mut app = app();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let mut cry = state.get_mut(app.world_mut()).unwrap();
    let path = "objects/env/03_origin_continent/001_odeco/odeco_plants_d01.cgf";
    for shading in [aa_cry::Shading::Basic, aa_cry::Shading::Full] {
        let loaded = cry.load_model(path, &ModelOptions { material: Some("objects/env/03_origin_continent/001_odeco/odeco_plants_d1.mtl".into()), keep_cpu_mesh: true, shading, ..default() }).unwrap();
        let mut checked = 0;
        let mut partial = 0;
        for (node, parts) in &loaded.parts {
            let source = &loaded.model.meshes[loaded.model.nodes[*node].mesh.unwrap()];
            let Some(subset) = source.subsets.iter().find(|s| s.material_id == 1) else { continue; };
            let target = cry.texture("objects/env/03_origin_continent/001_odeco/odeco_plants_d_trunk_df.dds", true);
            for part in parts {
                if cry.materials.get(&part.material).unwrap().base_color_texture != target { continue; }
                let mesh = cry.meshes.get(&part.mesh).unwrap();
                let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("missing RGBA") };
                let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
                let mut order = Vec::new();
                let mut seen = vec![false; source.positions.len()];
                for &i in &source.indices[subset.first_index as usize..(subset.first_index + subset.index_count) as usize] {
                    if !std::mem::replace(&mut seen[i as usize],true) { order.push(i as usize); }
                }
                for ((color, position), index) in colors.iter().zip(positions).zip(order) {
                    assert_eq!(*position, source.positions[index].to_array());
                    assert_eq!(*color, source.colors[index].map(|c| c as f32 / 255.0));
                    partial += usize::from(color[3] > 0.0 && color[3] < 1.0);
                    checked += 1;
                }
            }
        }
        assert!(checked > 100);
        assert!(partial > 0, "fixture must exercise authored nontrivial alpha");
    }
}

#[test]
fn copy_suffix_and_cubes() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let mut cry = state.get_mut(app.world_mut()).unwrap();
    // `... df copy.dds` is not in the pak; the original is
    let loader = cry.loader();
    let fixed = loader.resolve_texture("objects/characters/elf/female/face/face01/el_f_face01_df copy.dds");
    assert_eq!(fixed.as_deref(), Some("objects/characters/elf/female/face/face01/el_f_face01_df.dds"));
    // cube maps are environment maps, not 2D textures
    assert!(cry.texture("textures/cubemaps/beach_blur2.dds", true).is_none());
    assert!(cry.cube_texture("textures/cubemaps/beach_blur2.dds").is_some());
}

#[test]
fn stored_material_extensions_are_case_insensitive() {
    if !aa_cry::pak_available() {
        return;
    }
    // This bridge has no same-name .mtl: only its stored material name can resolve it.
    let path = "objects/env/02_harihara/05_si/si_woodbridge_01.cgf";
    let mut model = aa_cry::Model::load(path).unwrap();
    assert!(!aa_pak::Pak::global().exists(&path.replace(".cgf", ".mtl")));
    let expected = aa_cry::scene::find_mtl(path, &model).expect("bridge material");
    let name = aa_pak::normalize(model.material_name.as_deref().unwrap());
    model.material_name = Some(format!("{}.MTL", name.trim_end_matches(".mtl").to_ascii_uppercase()));
    assert_eq!(aa_cry::scene::find_mtl(path, &model), Some(expected));
}

#[test]
fn palace_material_overrides_preserve_raw_vertex_rgb_in_basic_and_full_models() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let mut cry = state.get_mut(app.world_mut()).unwrap();
    let path = "objects/env/01_nuia/01_sol/castle_nuia_001.cgf";
    let material = "objects/env/01_nuia/01_sol/castle_nuia_001_twocrowns.mtl";
    let mtl = cry.mtl(material).unwrap();
    for shading in [aa_cry::Shading::Basic, aa_cry::Shading::Full] {
        let loaded = cry.load_model(path, &ModelOptions { material: Some(material.into()), keep_cpu_mesh: true, shading, ..default() }).unwrap();
        let mut colored = 0;
        let mut nonwhite = 0;
        for (node, parts) in &loaded.parts {
            let source = &loaded.model.meshes[loaded.model.nodes[*node].mesh.unwrap()];
            // All subsets of this sample draw, so order is unchanged.
            assert_eq!(parts.len(), source.subsets.len());
            for (part, subset) in parts.iter().zip(&source.subsets) {
                let expected = mtl.sub(subset.material_id as usize).is_some_and(aa_cry::shade::illum::uses_vertex_color);
                let mesh = cry.meshes.get(&part.mesh).unwrap();
                if !expected {
                    assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
                    continue;
                }
                colored += 1;
                let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!("missing Illum colours") };
                let mut order = Vec::new();
                let mut seen = vec![false; source.positions.len()];
                for &index in &source.indices[subset.first_index as usize..(subset.first_index + subset.index_count) as usize] {
                    if !std::mem::replace(&mut seen[index as usize], true) { order.push(index); }
                }
                assert_eq!(colors.len(), order.len());
                for (color, index) in colors.iter().zip(order) {
                    let raw = source.colors[index as usize];
                    assert_eq!(*color, [raw[0] as f32 / 255.0, raw[1] as f32 / 255.0, raw[2] as f32 / 255.0, 1.0]);
                    nonwhite += usize::from(raw[..3] != [255; 3]);
                }
                assert!(cry.materials.get(&part.material).unwrap().lightmap_exposure >= 3.0e6);
            }
        }
        assert!(colored > 0 && nonwhite > 0, "sample must exercise baked occlusion");
    }
}

#[test]
fn native_illum_and_stage_cubemaps_keep_their_distinct_sampler_encodings() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut app = app();
    let mut state = SystemState::<CryAssets>::new(app.world_mut());
    let mut cry = state.get_mut(app.world_mut()).unwrap();
    let stage = cry.cube_texture("textures/cubemaps/beach_blur2.dds").unwrap();
    assert!(cry.images.get(&stage).unwrap().texture_descriptor.format.is_srgb());
    let loaded = cry.load_model("objects/characters/nuian/female/costume/nu_f_sk_saint001/nu_f_sk_saint001.chr", &ModelOptions {
        shading: aa_cry::Shading::Full,
        ..default()
    }).unwrap();
    assert!(check_inserted(&cry, &loaded) > 0);
    assert!(cry.images.iter().any(|(_, image)| {
        image.texture_view_descriptor.as_ref().is_some_and(|view| view.dimension == Some(TextureViewDimension::Cube))
            && !image.texture_descriptor.format.is_srgb()
    }), "native Illum must not reuse the stage's sRGB cube handle");
}
