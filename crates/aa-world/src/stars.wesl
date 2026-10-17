// Native catalogue sprite math from the client's stars.cfx. Position is a direction, so the
// camera translation is excluded; clip-space expansion keeps a star five pixels wide.
import bevy_pbr::render::mesh_view_bindings::view;

struct StarParams { values: vec4<f32>, }
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> stars: StarParams;

struct Input {
    @location(0) direction: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color_mag: vec4<f32>,
}
struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vertex(input: Input) -> Output {
    var out: Output;
    var clip = view.clip_from_world * vec4<f32>(input.direction, 0.0);
    let viewport = view.viewport.zw;
    let size = stars.values.z * min(1.0, min(viewport.x / 1280.0, viewport.y / 720.0));
    clip = vec4<f32>(clip.xy + (input.uv * 2.0 - 1.0) * size / viewport * clip.w, 0.0, clip.w);
    out.clip = clip;
    out.uv = input.uv;
    let native_pos = vec3<f32>(input.direction.x, -input.direction.z, input.direction.y);
    let table = array<vec2<f32>, 8>(
        vec2<f32>(0.897907815, -0.347608525), vec2<f32>(0.550299290, 0.273586675),
        vec2<f32>(0.823885965, 0.098853070), vec2<f32>(0.922739035, -0.122108860),
        vec2<f32>(0.800630175, -0.088956800), vec2<f32>(0.711673375, 0.158864420),
        vec2<f32>(0.870537795, 0.085484560), vec2<f32>(0.956022355, -0.058114540)
    );
    let hash = fract(native_pos.xy * 256.0);
    let index = fract(hash.x + (hash.y + 1.0) * stars.values.y) * 8.0;
    let pair = table[min(u32(index), 7u)];
    let flicker = pair.x + fract(index) * pair.y;
    let magnitude = 6.5 + input.color_mag.w * (-1.44 - 6.5);
    let brightness = flicker * pow(5.0, (-magnitude - 1.44) / 2.5);
    out.color = vec4<f32>(brightness * input.color_mag.rgb, brightness);
    return out;
}

@fragment
fn fragment(input: Output) -> @location(0) vec4<f32> {
    if stars.values.x <= 0.0 { discard; }
    let distance = 3.5 * (input.uv - 0.5);
    let scale = exp(-dot(distance, distance));
    let color = stars.values.x * (input.color.rgb * scale + 5.0 * input.color.a * pow(scale, 10.0));
    // Bevy's Add material uses premultiplied blending; zero alpha keeps the destination
    // intact, matching the native ONE/ONE blend instead of darkening the sprite rectangle.
    return vec4<f32>(color, 0.0);
}
