// Atmosphere fog + the engine's bloom and film curve, see `post.rs` and `post_bloom.rs`.
//
// Input: the HDR scene (already multiplied by the view exposure by the material shaders) and the main pass depth.
// Entry points: `bright_fragment` (the engine's bright pass at half resolution, run before the blur chain of
// `post_bloom.wesl`) and `fragment` (fog, bloom, grade, film curve; output: display linear colour, the render
// target does the sRGB encoding).

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
    // bloom: x brightness scale (`r_HDRBrightLevel / 0.18`), y threshold, z offset (the engine's `HDRBrightPassFilterPS`)
    bloom: vec4<f32>,
    // bloom: rgb strength added to the exposed scene (`BloomColor x BloomMul x r_HDRBloomMul x 3 x 0.5` of `FilmicMapping`)
    bloom_color: vec4<f32>,
    // sun shafts: x strength added to the exposed scene (0 = off), y length of the rays (fraction of the way to the sun),
    // z attenuation with the distance from the sun
    shafts: vec4<f32>,
    // sun shafts: rgb colour of the rays
    shafts_color: vec4<f32>,
    // eye adaptation: x blend per frame towards a brighter scene, y strength (0 = off), z darkest, w brightest exposure (stops)
    adapt: vec4<f32>,
    // eye adaptation: x log2 luminance of the calibrated exposure, y luminance cap, z blend per frame towards a darker scene
    adapt2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> post: Post;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_multisampled_2d;
@else
@group(0) @binding(2) var depth_texture: texture_depth_2d;
@group(0) @binding(3) var color_texture: texture_2d<f32>;
// eye adaptation state: 1x1, r = log2 of the adapted luminance relative to the calibrated one
@group(0) @binding(4) var adapt_texture: texture_2d<f32>;
// the blurred bright pass (mip 0 of the bloom chain); the blur passes of the shafts and `adapt_fragment` read their input here
@group(0) @binding(5) var bloom_texture: texture_2d<f32>;
@group(0) @binding(6) var bloom_sampler: sampler;
// the blurred sun shafts, only the final pass reads them
@group(0) @binding(7) var shafts_texture: texture_2d<f32>;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The exposure multiplier of the time of day times the eye's adaptation to the scene (a dark scene brightens, a bright one dims).
fn exposure() -> f32 {
    var gain = 1.0;
    if post.adapt.y > 0.0 {
        let state = textureLoad(adapt_texture, vec2<i32>(0, 0), 0).r;
        gain = exp2(clamp(-state * post.adapt.y, post.adapt.z, post.adapt.w));
    }
    return post.grade.w * gain;
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

// The scene colour of a full resolution pixel after the under water fog and the atmosphere fog (HDR, before the exposure).
fn scene_color(pixel: vec2<i32>, uv: vec2<f32>) -> vec3<f32> {
    var color = textureLoad(color_texture, pixel, 0).rgb;

    // ---- under water: the engine's `OceanOutof` / `WaterVolumeOutof` fog ----
    let under_water = post.under.w > 0.5;
    if under_water {
        let depth = textureLoad(depth_texture, pixel, 0);
        let ndc = vec3<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), max(depth, 1e-9));
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
        let ndc = vec3<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), max(depth, 1e-9));
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
    return color;
}

// The engine's bright pass (`HDRBrightPassFilterPS`) of an exposed scene colour: `v = c x level / key - threshold`, then
// `v / (offset + v)`, so the bloom of a pixel saturates (the sun disc blooms like a white wall).
fn bright(c_in: vec3<f32>) -> vec3<f32> {
    // Refraction effects add signed differences to the HDR target: a negative (or NaN) channel is no light at all
    let c = clamp(select(vec3<f32>(0.0), c_in, c_in == c_in), vec3<f32>(0.0), vec3<f32>(1.0e4));
    // The engine thresholds every channel; here the luminance decides and the colour of the pixel is kept (a blue sky has a blue
    // channel above the threshold long before its luminance gets there, and would paint a blue fringe on the foliage in front)
    let v = max(luminance(c) * post.bloom.x - post.bloom.y, 0.0);
    let tint = c / max(max(c.r, max(c.g, c.b)), 1e-4);
    return tint * (v / (post.bloom.z + v));
}

// Half resolution bright pass: the mean of the bright pass of the four full resolution pixels of a block (the engine's
// merged variant: no fireflies from single pixels). The target is half the size of the view.
@fragment
fn bright_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(view.viewport.zw);
    let base = vec2<i32>(in.position.xy) * 2;
    var sum = vec3<f32>(0.0);
    let ex = exposure();
    for (var i = 0; i < 4; i++) {
        let pixel = min(base + vec2<i32>(i & 1, i >> 1), size - vec2<i32>(1));
        let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.zw;
        let c = scene_color(pixel, uv) * ex;
        sum += bright(c);
    }
    return vec4<f32>(sum * 0.25, 1.0);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let color = scene_color(pixel, in.uv);

    // ---- bloom, grade and film curve ----
    let key = 0.18;
    // (a NaN from a refraction effect's signed colour must not reach the film curve)
    var c = select(vec3<f32>(0.0), color, color == color) * exposure();
    // the engine adds the bloom to the exposed scene before the grading (`exposure * scene + bloom * 0.5`)
    c = c + textureSampleLevel(bloom_texture, bloom_sampler, in.uv, 0.0).rgb * post.bloom_color.rgb;
    // sun shafts: the radially blurred sky around the sun, weaker with the distance from it
    if post.shafts.x > 0.0 {
        let sun = sun_screen();
        let aspect = view.viewport.z / view.viewport.w;
        let d = length((in.uv - sun.xy) * vec2<f32>(aspect, 1.0));
        let shafts = textureSampleLevel(shafts_texture, bloom_sampler, in.uv, 0.0).rgb;
        c = c + shafts * post.shafts_color.rgb * (post.shafts.x * exp(-d * post.shafts.z * 0.4));
    }
    c = vec3<f32>(key) + post.grade.y * (c - vec3<f32>(key));
    c = c * post.balance.rgb;
    let l = luminance(c);
    c = max(vec3<f32>(l) + post.grade.x * (c - vec3<f32>(l)), vec3<f32>(0.0));
    var g = film_curve(c);
    // levels: the black point
    g = clamp((g - vec3<f32>(post.grade.z)) / (1.0 - post.grade.z), vec3<f32>(0.0), vec3<f32>(1.0));
    // debug (AA_TUNE adapt_debug=1): the adaptation state as a grey, linear 0.5 + state / 8
    if post.adapt2.w > 0.5 {
        return vec4<f32>(vec3<f32>(0.5 + textureLoad(adapt_texture, vec2<i32>(0, 0), 0).r / 8.0), 1.0);
    }
    // gamma encoded -> linear: the swap chain applies the sRGB curve
    return vec4<f32>(pow(g, vec3<f32>(2.2)), 1.0);
}

// ---- sun shafts (`post_shafts.rs`) ----

// The sun on the screen: xy uv, z how much the view faces it (0 = away, 1 = towards).
fn sun_screen() -> vec3<f32> {
    let clip = view.clip_from_world * vec4<f32>(view.world_position + post.sun_dir.xyz * 1000.0, 1.0);
    let ndc = clip.xy / max(abs(clip.w), 1e-4) * sign(clip.w);
    let uv = ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let facing = select(0.0, smoothstep(0.0, 0.5, 1.0 - 0.5 * length(ndc)), clip.w > 0.0);
    return vec3<f32>(uv, facing);
}

// Quarter resolution mask: the sky (depth at or beyond the dome) weighted by how close it is to the sun, in exposed colour.
// Everything in front of the sky (trees, buildings, hills) is black, so the blur draws their shadows as rays.
@fragment
fn shafts_mask_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(view.viewport.zw);
    let base = vec2<i32>(in.position.xy) * 4;
    let ex = exposure();
    let sky_radius = post.fog_params.w;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let pixel = min(base + vec2<i32>(1 + 2 * (i & 1), 1 + 2 * (i >> 1)), size - vec2<i32>(1));
        let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.zw;
        let depth = textureLoad(depth_texture, pixel, 0);
        let ndc = vec3<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), max(depth, 1e-9));
        let wp = view.world_from_clip * vec4<f32>(ndc, 1.0);
        let to = wp.xyz / wp.w - view.world_position;
        let dist = length(to);
        if dist >= sky_radius * 0.9 {
            // (the direction from the near plane: a point at infinity has no reliable position)
            let near = view.world_from_clip * vec4<f32>(ndc.xy, 1.0, 1.0);
            let dir = normalize(near.xyz / near.w - view.world_position);
            let near_sun = pow(max(dot(dir, post.sun_dir.xyz), 0.0), 24.0);
            let c = clamp(textureLoad(color_texture, pixel, 0).rgb * ex, vec3<f32>(0.0), vec3<f32>(3.0));
            sum += c * near_sun;
        }
    }
    return vec4<f32>(sum * 0.25 * sun_screen().z, 1.0);
}

// Radial blur of the input (the bloom slot) towards the sun: 12 samples over `reach` of the way. The first pass blurs a
// twelfth of the final length, the second spreads that over all of it.
fn shafts_blur(uv: vec2<f32>, reach: f32) -> vec4<f32> {
    let to_sun = sun_screen().xy - uv;
    // (an interleaved offset per pixel hides the stepping)
    let jitter = fract(52.9829189 * fract(dot(uv * view.viewport.zw, vec2<f32>(0.06711056, 0.00583715))));
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 12; i++) {
        let t = (f32(i) + jitter) / 12.0 * reach;
        sum += textureSampleLevel(bloom_texture, bloom_sampler, uv + to_sun * t, 0.0).rgb;
    }
    return vec4<f32>(sum / 12.0, 1.0);
}

@fragment
fn shafts_blur1_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return shafts_blur(in.uv, post.shafts.y / 12.0);
}

@fragment
fn shafts_blur2_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return shafts_blur(in.uv, post.shafts.y);
}

// ---- eye adaptation (`post_adapt.rs`) ----

const LUM_CELLS = vec2<i32>(32, 18);

// 32 x 18 cells of the exposed scene: the mean log2 luminance of a 6 x 6 grid of pixels in each (the sun and its glow capped).
@fragment
fn lum_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let cell = vec2<f32>(in.position.xy);
    var sum = 0.0;
    for (var i = 0; i < 36; i++) {
        let offset = (vec2<f32>(f32(i % 6), f32(i / 6)) + vec2<f32>(0.5)) / 6.0;
        let uv = (cell + offset) / vec2<f32>(LUM_CELLS);
        let pixel = min(vec2<i32>(uv * view.viewport.zw), vec2<i32>(view.viewport.zw) - vec2<i32>(1));
        let lum = luminance(max(scene_color(pixel, uv), vec3<f32>(0.0))) * post.grade.w;
        sum += log2(clamp(lum, 1.0e-3, post.adapt2.y));
    }
    return vec4<f32>(sum / 36.0, 0.0, 0.0, 1.0);
}

// The centre weighted mean of the cells, as a state relative to the calibrated exposure, eased from the previous state.
@fragment
fn adapt_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var sum = 0.0;
    var weights = 0.0;
    for (var y = 0; y < LUM_CELLS.y; y++) {
        for (var x = 0; x < LUM_CELLS.x; x++) {
            let p = (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / vec2<f32>(LUM_CELLS) - vec2<f32>(0.5);
            let w = exp(-dot(p, p) * 5.0);
            sum += textureLoad(bloom_texture, vec2<i32>(x, y), 0).r * w;
            weights += w;
        }
    }
    let target_state = sum / weights - post.adapt2.x;
    let previous = textureLoad(adapt_texture, vec2<i32>(0, 0), 0).r;
    let blend = select(post.adapt2.z, post.adapt.x, target_state > previous);
    return vec4<f32>(clamp(mix(previous, target_state, blend), -8.0, 8.0), 0.0, 0.0, 1.0);
}
