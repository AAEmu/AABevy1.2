//! The post pass shaders compile and validate without a window.
//!
//! Bevy 0.20 turns a `.wesl` shader into WGSL with `wesl::compile` (imports and conditional translation on, the
//! pipeline's shader defs as the feature flags) and hands the WGSL to wgpu, whose validator is naga's. A shader that
//! does not compile shows only when the pipeline is first built, in a running client. This test runs the same two steps
//! on the sources in `src/`: `post.wesl` with and without `MULTISAMPLED` (the two pipelines `post.rs` builds) and
//! `post_bloom.wesl`.
//!
//! The two modules `post.wesl` imports live inside Bevy's crates (`bevy_render`'s `view.wesl`,
//! `bevy_core_pipeline`'s `fullscreen_vertex_shader.wesl`); the stand-ins below are those definitions as of Bevy
//! 0.20.0, cut to the types. A newer Bevy that renames a field the shader reads shows here only after the stand-in is
//! brought up to date.

use std::borrow::Cow;

use wesl::syntax::{ModulePath, PathOrigin};
use wesl::resolver::VirtualResolver;
use wesl::{CompileOptions, Feature};

/// `bevy_render-0.20.0/src/view.wesl`, the types only.
const BEVY_RENDER_VIEW: &str = r#"
struct ColorGrading {
    balance: mat3x3<f32>,
    saturation: vec3<f32>,
    contrast: vec3<f32>,
    gamma: vec3<f32>,
    gain: vec3<f32>,
    lift: vec3<f32>,
    midtone_range: vec2<f32>,
    exposure: f32,
    hue: f32,
    post_saturation: f32,
}

struct View {
    clip_from_world: mat4x4<f32>,
    unjittered_clip_from_world: mat4x4<f32>,
    world_from_clip: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    view_from_world: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    view_from_clip: mat4x4<f32>,
    world_position: vec3<f32>,
    exposure: f32,
    viewport: vec4<f32>,
    main_pass_viewport: vec4<f32>,
    frustum: array<vec4<f32>, 6>,
    lod_view_world_position: vec3<f32>,
    color_grading: ColorGrading,
    mip_bias: f32,
    frame_count: u32,
};
"#;

/// `bevy_core_pipeline-0.20.0/src/fullscreen_vertex_shader.wesl`.
const BEVY_FULLSCREEN: &str = r#"
struct FullscreenVertexOutput {
    @builtin(position)
    position: vec4<f32>,
    @location(0)
    uv: vec2<f32>,
};

@vertex
fn fullscreen_vertex_shader(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOutput {
    let uv = vec2<f32>(f32(vertex_index >> 1u), f32(vertex_index & 1u)) * 2.0;
    let clip_position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    return FullscreenVertexOutput(clip_position, uv);
}
"#;

fn package(name: &str, components: &[&str]) -> ModulePath {
    ModulePath { origin: PathOrigin::Package(name.to_string()), components: components.iter().map(|c| c.to_string()).collect() }
}

/// What the shader loader does with one pipeline: WESL to WGSL, then the validator. Returns the entry points.
fn compile(name: &str, source: &str, defs: &[&str]) -> Vec<String> {
    let mut resolver = VirtualResolver::new();
    let main = ModulePath { origin: PathOrigin::Absolute, components: vec![name.to_string()] };
    resolver.add_module(main.clone(), Cow::Owned(source.to_string()));
    resolver.add_module(package("bevy_render", &["view"]), Cow::Borrowed(BEVY_RENDER_VIEW));
    resolver.add_module(package("bevy_core_pipeline", &["fullscreen_vertex_shader"]), Cow::Borrowed(BEVY_FULLSCREEN));

    // as `bevy_shader::shader_cache`: imports and conditional translation on, visibility off, each def a flag
    let mut options = CompileOptions { imports: true, condcomp: true, visibility: false, ..Default::default() };
    for def in defs {
        options.features.flags.insert(def.to_string(), Feature::Enable);
    }
    let compiled = wesl::compile(&main, &options, &resolver).unwrap_or_else(|e| panic!("{name} {defs:?}: WESL does not compile:\n{}", e.diagnostic().render_plain()));
    let wgsl = compiled.to_string();

    let module = naga::front::wgsl::parse_str(&wgsl).unwrap_or_else(|e| panic!("{name} {defs:?}: the WGSL does not parse:\n{}\n--- WGSL ---\n{wgsl}", e.emit_to_string(&wgsl)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name} {defs:?}: the WGSL does not validate:\n{}\n--- WGSL ---\n{wgsl}", e.emit_to_string(&wgsl)));
    module.entry_points.iter().map(|e| e.name.clone()).collect()
}

#[test]
fn the_post_pass_compiles_and_validates() {
    let source = include_str!("../src/post.wesl");
    for defs in [&[][..], &["MULTISAMPLED"][..]] {
        let mut entries = compile("post", source, defs);
        entries.sort();
        assert_eq!(
            entries,
            [
                "adapt_fragment",
                "bright_fragment",
                "dof_blur_h_fragment",
                "dof_blur_v_fragment",
                "dof_down_fragment",
                "fragment",
                "lum_fragment",
                "shafts_blur1_fragment",
                "shafts_blur2_fragment",
                "shafts_blur3_fragment",
                "shafts_mask_fragment"
            ],
            "{defs:?}"
        );
    }
}

#[test]
fn the_bloom_chain_compiles_and_validates() {
    let mut entries = compile("post_bloom", include_str!("../src/post_bloom.wesl"), &[]);
    entries.sort();
    assert_eq!(entries, ["downsample", "upsample"]);
}

#[test]
fn a_broken_shader_is_caught() {
    // the check itself: an undefined name must fail the compile, not pass
    let broken = "@fragment\nfn fragment() -> @location(0) vec4<f32> { return vec4<f32>(no_such_name); }\n";
    let result = std::panic::catch_unwind(|| compile("broken", broken, &[]));
    assert!(result.is_err());
}

/// The member names of the WESL struct `Post` and of the Rust struct `PostUniform` (all `vec4<f32>` / `Vec4`), in order.
fn uniform_members() -> (Vec<String>, Vec<String>) {
    let wesl = include_str!("../src/post.wesl");
    let body = &wesl[wesl.find("struct Post {").unwrap()..];
    let body = &body[..body.find("\n}").unwrap()];
    let shader: Vec<String> = body.lines().filter_map(|l| l.trim().strip_suffix(": vec4<f32>,").map(str::to_string)).collect();
    let rust = include_str!("../src/post.rs");
    let body = &rust[rust.find("pub struct PostUniform {").unwrap()..];
    let body = &body[..body.find("\n}").unwrap()];
    let fields: Vec<String> = body.lines().filter_map(|l| l.trim().strip_prefix("pub ").and_then(|l| l.strip_suffix(": Vec4,")).map(str::to_string)).collect();
    (shader, fields)
}

#[test]
fn the_post_struct_is_the_rust_uniform_member_for_member() {
    use bevy::render::render_resource::ShaderType;
    let (shader, rust) = uniform_members();
    assert_eq!(shader, rust, "the WESL `Post` and `PostUniform` list the same members in the same order");
    assert!(rust.ends_with(&["dof".to_string(), "dof_rest".to_string()]), "the depth of field's two rows end both");
    assert_eq!(rust.len() as u64 * 16, aa_world::post::PostUniform::min_size().get(), "every member is one 16 byte vector: the span is the struct's size");
}
