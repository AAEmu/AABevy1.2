// Atmosphere fog + the engine's bloom and film curve, see `post.rs` and `post_bloom.rs`.
//
// Input: the HDR scene (already multiplied by the view exposure by the material shaders) and the main pass depth.
// Entry points: `bright_fragment` (the engine's bright pass at half resolution, run before the blur chain of
// `post_bloom.wesl`), `dof_down_fragment`, `dof_blur_h_fragment` and `dof_blur_v_fragment` (the depth of field's half
// resolution copy, `post_dof.rs`) and `fragment` (depth of field, fog, bloom, grade, film curve; output: display linear colour,
// the render target does the sRGB encoding).

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;

struct Post {
    // rgb: fog colour near the ground, pre-exposed; a: global density
    fog_color: vec4<f32>,
    // rgb: fog colour high up, pre-exposed
    fog_top: vec4<f32>,
    // bottom height, top height, bottom density, top density (world Y, metres)
    fog_layer: vec4<f32>,
    // unused, extinction per metre at density 1, maximum fog amount, sky radius (m)
    fog_params: vec4<f32>,
    // the engine's `vfRampParams`: x 1 / (ramp end - ramp start), y -ramp start x that, z ramp influence, w 1 - influence
    fog_ramp: vec4<f32>,
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
    // under water: rgb colour of the deep water at the surface (pre-exposed),
    // private a: 1 volume Outof, -1 source-qualified sea, 0 legacy ocean fallback
    under_color: vec4<f32>,
    // bloom: x brightness scale (`r_HDRBrightLevel / 0.18`), y threshold, z offset (the engine's `HDRBrightPassFilterPS`)
    bloom: vec4<f32>,
    // bloom: rgb strength added to the exposed scene (`BloomColor x BloomMul x r_HDRBloomMul x 3 x 0.5` of `FilmicMapping`)
    bloom_color: vec4<f32>,
    // radial motion blur: x reach towards the centre (0 = off), yz the centre (uv)
    motion: vec4<f32>,
    // HDR shafts: x RaysAmount * tuning gain, y finite-projection bridge scale,
    // z clamped RaysAttenuation, w world view limit for the depth bridge (metres)
    shafts: vec4<f32>,
    // sun shafts: rgb colour of the rays
    shafts_color: vec4<f32>,
    // eye adaptation: x blend per frame towards a brighter scene, y strength (0 = off), z darkest, w brightest exposure (stops)
    adapt: vec4<f32>,
    // eye adaptation: x log2 luminance of the calibrated exposure, y luminance cap, z blend per frame towards a darker scene
    adapt2: vec4<f32>,
    // colour grading (`AdjustLevels`): x 1 / gamma, y max input (0..1); the min input is grade.z
    levels: vec4<f32>,
    // colour grading (`ApplyPhotoFilter`): rgb filter colour (display), w density (0 = off)
    photo: vec4<f32>,
    // colour grading (`AdjustColor`): x saturation, y contrast, z brightness (1 = neutral), w 1 when the grading is on
    color: vec4<f32>,
    // depth of field the application asks for (a quest talk): x the blur of the half resolution copy, a gaussian's sigma in its texels
    // (0 = off), y the focus distance (m), z half the focus range, w the blur amount (`depth_of_field.rs`, `post_dof.rs`)
    dof: vec4<f32>,
    // the time of day's depth of field at rest, the same four numbers (x 0 = none); `dof` wins while its x is above 0
    dof_rest: vec4<f32>,
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
// the depth of field's blurred half resolution copy, premultiplied (rgb x w, w); only the final pass reads it
@group(0) @binding(8) var dof_texture: texture_2d<f32>;

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The exposure multiplier of the time of day times the eye's adaptation to the scene (a dark scene brightens, a bright one dims).
// The engine's `FilmicMapping`: `exposure = clamp(key / lerp(r_EyeAdaptationBase, luminance, r_EyeAdaptationFactor), 0.36, 2.8)`
// with key = base = 0.18 and factor 0.4 (`cryrenderd3d10.dll` defaults); `state` is log2(luminance / calibrated luminance), so the
// calibrated look is exposure 1: `1 / (0.6 + 0.4 2^state)`. post.adapt.y scales the state (1 = the engine's).
const EYE_FACTOR = 0.4;
fn exposure() -> f32 {
    var gain = 1.0;
    if post.adapt.y > 0.0 {
        let state = textureLoad(adapt_texture, vec2<i32>(0, 0), 0).r * post.adapt.y;
        gain = clamp(1.0 / (1.0 - EYE_FACTOR + EYE_FACTOR * exp2(state)), exp2(post.adapt.z), exp2(post.adapt.w));
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
        if post.under_color.a > 0.5 {
            // Original WaterVolumeOutofPS: eye-to-opaque depth, limited by
            // the surface plane. No ocean scatter, receiver-depth term or
            // synthetic angular window. The native upload converts density
            // to log2(e) before exp2, equivalent to this natural exponential.
            var volume_path = dist;
            if dir.y > 0.0 { volume_path = min(volume_path, depth_cam / dir.y); }
            color = mix(post.under_color.rgb, color, exp(-k * volume_path));
        } else {
        // Retain the calibrated sea path/scatter. Original OceanOutofPS has no
        // angular mask: only the exact qualified tag bypasses the old cone.
        // This is not a port of its distinct scatter integral or shallow colour.
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
        if post.under_color.a == -1.0 { color = seen; }
        else { color = mix(far_scatter, seen, window); }
        }
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
            let y0 = view.world_position.y;
            // the engine's `ComputeVolumetricFogInternal` (`volumetricfog.cfi`): the layer's optical depth from the eye, the fog
            // eased in from `Ramp start` to `Ramp end` as much as `Ramp influence` says
            let od = height_integral(y0, dir.y, dist, hb, k);
            let tau = post.fog_params.y * post.fog_color.a * db * max(od, 0.0);
            var r = clamp(dist * post.fog_ramp.x + post.fog_ramp.y, 0.0, 1.0);
            r = r * (2.0 - r) * post.fog_ramp.z + post.fog_ramp.w;
            var amount = (1.0 - exp(-tau)) * r * post.fog_params.z;
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

// ---- depth of field (`depth_of_field.rs`, `post_dof.rs`): 1.2's composite blends each pixel towards a blurred half resolution copy of
// the scene by its own depth; the application's focus (a quest talk) while it is on, else the time of day's (x 0: neither, no pass and
// no sample) ----

fn dof_params() -> vec4<f32> {
    return select(post.dof_rest, post.dof, post.dof.x > 0.0);
}

// The linear depth of a pixel in metres (along the view; the far plane and the sky, depth 0 of the reversed buffer, are far away).
fn dof_depth(pixel: vec2<i32>) -> f32 {
    let depth = textureLoad(depth_texture, pixel, 0);
    if depth <= 1e-9 {
        return 1.0e6;
    }
    let v = view.view_from_clip * vec4<f32>(0.0, 0.0, depth, 1.0);
    return -v.z / max(abs(v.w), 1e-9) * sign(v.w);
}

// How much of the copy a pixel at linear depth z takes (`Focus::composite_weight`, 1.2's `HDRToneMapSample`): far = saturate((z - d) / (r / 2))
// from the pixel's own depth, near = saturate(2 b saturate((d - z) / (r / 2))), their sum held to 1.
fn dof_weight(z: f32, dof: vec4<f32>) -> f32 {
    let half_range = max(dof.z, 1e-3);
    let far = clamp((z - dof.y) / half_range, 0.0, 1.0);
    let near = clamp(2.0 * dof.w * clamp((dof.y - z) / half_range, 0.0, 1.0), 0.0, 1.0);
    return min(far + near, 1.0);
}

// The copy: the fogged scene box-filtered to half resolution (1.2's `texToTexParams`: the four pixels of the block), each pixel weighted by
// how much of the copy it takes, premultiplied, so what is in focus adds nothing to the blur around it.
@fragment
fn dof_down_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dof = dof_params();
    let size = vec2<i32>(view.viewport.zw);
    let base = vec2<i32>(in.position.xy) * 2;
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let pixel = min(base + vec2<i32>(i & 1, i >> 1), size - vec2<i32>(1));
        let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.zw;
        let raw = scene_color(pixel, uv);
        let c = clamp(select(vec3<f32>(0.0), raw, raw == raw), vec3<f32>(0.0), vec3<f32>(6.0e4));
        let w = max(dof_weight(dof_depth(pixel), dof), 1e-3);
        sum += vec4<f32>(c * w, w);
    }
    return sum * 0.25;
}

// One direction of the separable gaussian of the premultiplied copy (`depth_of_field::gaussian_taps`): sigma `dof.x` texels, reach three
// sigma held to 24, neighbouring texels merged in pairs into one bilinear tap. The input is read through the bloom slot.
fn dof_gauss(uv: vec2<f32>, axis: vec2<f32>) -> vec4<f32> {
    let sigma = max(dof_params().x, 1e-3);
    let reach = i32(clamp(ceil(3.0 * sigma), 1.0, 24.0));
    let k2 = 1.0 / (2.0 * sigma * sigma);
    let step = axis / vec2<f32>(textureDimensions(bloom_texture));
    var sum = textureSampleLevel(bloom_texture, bloom_sampler, uv, 0.0);
    var total = 1.0;
    for (var k = 1; k <= reach; k += 2) {
        let a = exp(-f32(k * k) * k2);
        let b = select(0.0, exp(-f32((k + 1) * (k + 1)) * k2), k < reach);
        let w = a + b;
        let o = (f32(k) * a + f32(k + 1) * b) / w;
        sum += w * (textureSampleLevel(bloom_texture, bloom_sampler, uv + step * o, 0.0)
            + textureSampleLevel(bloom_texture, bloom_sampler, uv - step * o, 0.0));
        total += 2.0 * w;
    }
    return sum / total;
}

@fragment
fn dof_blur_h_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return dof_gauss(in.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn dof_blur_v_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return dof_gauss(in.uv, vec2<f32>(0.0, 1.0));
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    var color = scene_color(pixel, in.uv);

    // ---- depth of field: the blurred copy blended in by the pixel's own depth (off at x 0: no sample) ----
    let dof = dof_params();
    if dof.x > 0.0 {
        let m = dof_weight(dof_depth(pixel), dof);
        if m > 0.0 {
            let copy = textureSampleLevel(dof_texture, bloom_sampler, in.uv, 0.0);
            // a drawn copy has an alpha of 1e-3 or more; 0 is the cleared texture of a copy whose pipelines are still compiling
            if copy.a > 1e-5 {
                color = mix(color, copy.rgb / copy.a, m);
            }
        }
    }

    // ---- radial motion blur (off at 0: no extra samples) ----
    // the raw scene is sampled towards the centre, the centre itself (the character) stays sharp; the fog of the pixel is
    // kept and the samples add their own colour's difference to it
    if post.motion.x > 0.0 {
        let size = view.viewport.zw;
        let d = in.uv - post.motion.yz;
        let reach = post.motion.x * smoothstep(0.08, 0.6, length(d * vec2<f32>(size.x / size.y, 1.0)));
        if reach > 1e-4 {
            let raw = textureLoad(color_texture, pixel, 0).rgb;
            var sum = vec3<f32>(0.0);
            for (var i = 1; i <= 6; i++) {
                let uv = in.uv - d * (reach * f32(i) / 6.0);
                let p = clamp(vec2<i32>(uv * size), vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
                sum += textureLoad(color_texture, p, 0).rgb;
            }
            let blurred = (raw + sum) / 7.0;
            color = max(color + (blurred - raw), vec3<f32>(0.0));
        }
    }

    // ---- bloom, grade and film curve ----
    let key = 0.18;
    // (a NaN from a refraction effect's signed colour must not reach the film curve)
    var c = select(vec3<f32>(0.0), color, color == color) * exposure();
    // the engine adds the bloom to the exposed scene before the grading (`exposure * scene + bloom * 0.5`)
    c = c + textureSampleLevel(bloom_texture, bloom_sampler, in.uv, 0.0).rgb * post.bloom_color.rgb;
    c = vec3<f32>(key) + post.grade.y * (c - vec3<f32>(key));
    c = c * post.balance.rgb;
    let l = luminance(c);
    c = max(vec3<f32>(l) + post.grade.x * (c - vec3<f32>(l)), vec3<f32>(0.0));
    var g = clamp(film_curve(c), vec3<f32>(0.0), vec3<f32>(1.0));
    // HDRFinalScenePS: screen blend on the gamma-encoded film result, before grading.
    if post.shafts.x > 0.0 {
        let shafts = textureSampleLevel(shafts_texture, bloom_sampler, in.uv, 0.0).rgb;
        g += shafts * post.shafts_color.rgb * post.shafts.x * (vec3<f32>(1.0) - g);
    }
    g = color_grade(g);
    // debug (AA_TUNE adapt_debug=1): the adaptation state as a grey, linear 0.5 + state / 8
    if post.adapt2.w > 0.5 {
        return vec4<f32>(vec3<f32>(0.5 + textureLoad(adapt_texture, vec2<i32>(0, 0), 0).r / 8.0), 1.0);
    }
    // gamma encoded -> linear: the swap chain applies the sRGB curve
    return vec4<f32>(pow(g, vec3<f32>(2.2)), 1.0);
}

// The colour grading effect of the time of day on the tone mapped (gamma encoded) colour, in the engine's order: `AdjustLevels`,
// `ApplyPhotoFilter`, `AdjustColor` (`posteffectsgame.cfx`, technique `ColorGrading`). The levels' black point acts alone when
// the grading is off (the earlier look).
fn color_grade(g_in: vec3<f32>) -> vec3<f32> {
    let lo = post.grade.z;
    var g = g_in;
    if post.color.w < 0.5 {
        return clamp((g - vec3<f32>(lo)) / (1.0 - lo), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // levels: (c 255 - min) / (max - min), then ^ (1 / gamma)
    g = pow(max((g - vec3<f32>(lo)) / max(post.levels.y - lo, 1e-3), vec3<f32>(0.0)), vec3<f32>(post.levels.x));
    // photo filter: the colour curve black -> filter colour (at luminance 0.5) -> white, blended in by luminance x density
    let lum = dot(g, vec3<f32>(0.33, 0.59, 0.11));
    var curve = mix(vec3<f32>(0.0), post.photo.rgb, saturate(lum * 2.0));
    curve = mix(curve, vec3<f32>(1.0), saturate(lum - 0.5) * 2.0);
    g = mix(g, curve, saturate(lum * post.photo.w));
    // colour matrix: brightness, contrast, saturation
    g = g * post.color.z;
    g = (g - vec3<f32>(0.5)) * post.color.y + vec3<f32>(0.5);
    let l = dot(g, vec3<f32>(0.299, 0.587, 0.114));
    g = vec3<f32>(l) + post.color.x * (g - vec3<f32>(l));
    return clamp(g, vec3<f32>(0.0), vec3<f32>(1.0));
}

// ---- sun shafts (`post_shafts.rs`) ----

// SetSunDir (Cry3DEngine 30126C30) stores normalized direction * 7500;
// GenerateSunShafts (3815D290) projects that absolute position * 1000.
// Infinite reversed-Z clip.z is NOT the native signed forward projection.
fn sun_screen() -> vec3<f32> {
    let clip = view.clip_from_world * vec4<f32>(post.sun_dir.xyz * 7500000.0, 1.0);
    let uv = vec2<f32>(clip.x + clip.w, clip.w - clip.y) * 0.5 / (clip.w + 1e-6);
    let forward = -(view.view_from_world * vec4<f32>(post.sun_dir.xyz, 0.0)).z;
    return vec3<f32>(uv, forward * post.shafts.y);
}

// Native scene Z is axial distance / camera far (CommonZPass.cfi), not scene
// brightness or a binary sky mask. Reconstruct it from Bevy's reversed depth.
fn shaft_depth(uv: vec2<f32>) -> f32 {
    let pixel = clamp(vec2<i32>(uv * view.viewport.zw), vec2<i32>(0), vec2<i32>(view.viewport.zw) - vec2<i32>(1));
    let depth = textureLoad(depth_texture, pixel, 0);
    let vp = view.view_from_clip * vec4<f32>(0.0, 0.0, max(depth, 1e-9), 1.0);
    return saturate(-vp.z / vp.w / max(post.shafts.w, 1e-3));
}

@fragment
fn shafts_mask_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = 1.0 / view.viewport.zw;
    let d = (shaft_depth(in.uv + vec2<f32>(0.0, 1.0) * pixel)
           + shaft_depth(in.uv + vec2<f32>(1.0, 0.65) * pixel)
           + shaft_depth(in.uv + vec2<f32>(-1.0, 0.65) * pixel)) * 0.333;
    return vec4<f32>(d, d, d, 1.0 - d);
}

// SunShaftsGenPS: eight weighted taps. The native reaches are constants and the
// signed facing and radial attenuation apply on EACH pass, not in the final blend.
fn shafts_blur(uv: vec2<f32>, reach: f32) -> vec4<f32> {
    let sun = sun_screen();
    let to_sun = sun.xy - uv;
    let aspect = 1.333 * view.viewport.w / view.viewport.z;
    let sun_dist = saturate(sun.z) * saturate(1.0 - saturate(length(to_sun * vec2<f32>(1.0, aspect)) * post.shafts.z));
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < 8; i++) {
        let at = uv + to_sun * (reach * sun.z * f32(i));
        sum += textureSampleLevel(bloom_texture, bloom_sampler, at, 0.0) * (1.0 - f32(i) / 8.0);
    }
    let blurred = sum * 0.25;
    return vec4<f32>(blurred.rgb * sun_dist, blurred.a + 1.0 - saturate(sun.z * 0.1 + 0.9));
}

@fragment
fn shafts_blur1_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return shafts_blur(in.uv, 0.1);
}

@fragment
fn shafts_blur2_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return shafts_blur(in.uv, 0.025);
}

@fragment
fn shafts_blur3_fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return shafts_blur(in.uv, 0.01);
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
