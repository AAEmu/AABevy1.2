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
