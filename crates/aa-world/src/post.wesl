// Atmosphere fog + the engine's film curve, see `post.rs`.
//
// Input: the HDR scene (already multiplied by the view exposure by the material shaders, bloom added) and the
// main pass depth. Output: display linear colour (the render target does the sRGB encoding).

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;

struct Post {
    // rgb: fog colour near the ground, pre-exposed; a: global density
    fog_color: vec4<f32>,
    // rgb: fog colour high up, pre-exposed
    fog_top: vec4<f32>,
    // bottom height, top height, bottom density, top density (world Y, metres)
    fog_layer: vec4<f32>,
    // ramp start (m), extinction per metre at density 1, maximum fog amount, sky radius (m)
    fog_params: vec4<f32>,
    // xyz: direction towards the sun, w: glow exponent
    sun_dir: vec4<f32>,
    // rgb: sun glow colour added to the fog towards the sun, pre-exposed
    sun_glow: vec4<f32>,
    // film curve: shoulder, midtones, toe, whitepoint
    film: vec4<f32>,
    // saturation, contrast, levels minimum, exposure multiplier
    grade: vec4<f32>,
    // rgb: colour balance, w: 1 when the fog is on
    balance: vec4<f32>,
    // under water: x water level (world Y), y extinction per metre, z in-scattering gain, w 1 when the camera is below the surface
    under: vec4<f32>,
    // under water: rgb colour of the deep water at the surface (pre-exposed)
    under_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> post: Post;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_multisampled_2d;
@else
@group(0) @binding(2) var depth_texture: texture_depth_2d;
@group(0) @binding(3) var color_texture: texture_2d<f32>;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// integral of exp(-k (y0 + dir_y s - hb)) for s in 0..len
fn height_integral(y0: f32, dir_y: f32, len: f32, hb: f32, k: f32) -> f32 {
    let base = exp(-k * (y0 - hb));
    let a = k * dir_y;
    if abs(a) < 1e-5 {
        return base * len;
    }
    return base * (1.0 - exp(-a * len)) / a;
}

// The engine's HDR "filmic" curve (postprocess.cfx FilmicMapping): (x (a 6.2 x + b 0.5)) / (x (a 6.2 x + 1.7) + c 0.06)
// divided by the same value at the white point. The result is gamma encoded.
fn film_curve(x: vec3<f32>) -> vec3<f32> {
    let a = post.film.x;
    let b = post.film.y;
    let c = post.film.z;
    let w = post.film.w;
    let num = x * (a * 6.2 * x + vec3<f32>(b * 0.5));
    let den = x * (a * 6.2 * x + vec3<f32>(1.7)) + vec3<f32>(c * 0.06);
    let white = (w * (a * 6.2 * w + b * 0.5)) / (w * (a * 6.2 * w + 1.7) + c * 0.06);
    return (num / den) / white;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    var color = textureLoad(color_texture, pixel, 0).rgb;

    // ---- under water: the engine's `OceanOutof` / `WaterVolumeOutof` fog ----
    let under_water = post.under.w > 0.5;
    if under_water {
        let depth = textureLoad(depth_texture, pixel, 0);
        let ndc = vec3<f32>(in.uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), max(depth, 1e-9));
        let wp = view.world_from_clip * vec4<f32>(ndc, 1.0);
        let world = wp.xyz / wp.w;
        let to = world - view.world_position;
        let dist = length(to);
        let dir = to / max(dist, 1e-4);
        let level = post.under.x;
        let k = post.under.y;
        let depth_cam = max(level - view.world_position.y, 0.0);
        // a ray that reaches the surface before the scene leaves the water: through the window of the surface (Snell's cone,
        // 49 degrees around the vertical) it sees what is above, outside it the surface is a mirror of the water below
        var path = dist;
        var seen_depth = max(level - world.y, 0.0);
        var window = 1.0;
        let to_surface = depth_cam / max(dir.y, 1e-3);
        if dir.y > 1e-3 && to_surface < dist {
            path = to_surface;
            seen_depth = 0.0;
            window = smoothstep(0.62, 0.72, dir.y);
        }
        // light scattered towards the eye along the path (the sunlight dims with the depth), the scene dims with the path
        // and with its own depth
        let kt = k * (dir.y - 1.0);
        let along = select((exp(kt * path) - 1.0) / kt, path, abs(kt) < 1e-6);
        let scatter = post.under_color.rgb * (exp(-k * depth_cam) * along * k * post.under.z);
        let far_along = select((exp(kt * 20000.0) - 1.0) / kt, 20000.0, abs(kt) < 1e-6);
        let far_scatter = post.under_color.rgb * (exp(-k * depth_cam) * far_along * k * post.under.z);
        let seen = color * exp(-k * (path + seen_depth)) + scatter;
        color = mix(far_scatter, seen, window);
    }

    // ---- fog ----
    if post.balance.w > 0.5 && !under_water {
        let depth = textureLoad(depth_texture, pixel, 0);
        let ndc = vec3<f32>(in.uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), max(depth, 1e-9));
        let wp = view.world_from_clip * vec4<f32>(ndc, 1.0);
        let world = wp.xyz / wp.w;
        let to = world - view.world_position;
        let dist = length(to);
        let dir = to / max(dist, 1e-4);
        // the sky dome and everything beyond it carries its own haze
        let sky_radius = post.fog_params.w;
        if dist < sky_radius * 0.9 {
            let hb = post.fog_layer.x;
            let ht = max(post.fog_layer.y, hb + 1.0);
            let db = max(post.fog_layer.z, 1e-3);
            let dt = max(post.fog_layer.w, 1e-3);
            let k = log(db / dt) / (ht - hb);
            let ramp = post.fog_params.x;
            let y0 = view.world_position.y;
            let near = min(ramp, dist);
            let od = height_integral(y0, dir.y, dist, hb, k) - height_integral(y0, dir.y, near, hb, k);
            let tau = post.fog_params.y * post.fog_color.a * db * max(od, 0.0);
            var amount = (1.0 - exp(-tau)) * post.fog_params.z;
            // colour: low fog near the ground, thin high fog above the layer
            let hit_y = world.y;
            let fog_t = clamp((hit_y - hb) / (ht - hb), 0.0, 1.0);
            var fog_rgb = mix(post.fog_color.rgb, post.fog_top.rgb, fog_t);
            let sun = max(dot(dir, post.sun_dir.xyz), 0.0);
            fog_rgb = fog_rgb + post.sun_glow.rgb * pow(sun, post.sun_dir.w);
            color = mix(color, fog_rgb, amount);
        }
    }

    // ---- grade and film curve ----
    let key = 0.18;
    var c = color * post.grade.w;
    c = vec3<f32>(key) + post.grade.y * (c - vec3<f32>(key));
    c = c * post.balance.rgb;
    let l = luminance(c);
    c = max(vec3<f32>(l) + post.grade.x * (c - vec3<f32>(l)), vec3<f32>(0.0));
    var g = film_curve(c);
    // levels: the black point
    g = clamp((g - vec3<f32>(post.grade.z)) / (1.0 - post.grade.z), vec3<f32>(0.0), vec3<f32>(1.0));
    // gamma encoded -> linear: the swap chain applies the sRGB curve
    return vec4<f32>(pow(g, vec3<f32>(2.2)), 1.0);
}
