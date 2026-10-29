// The sky dome: a procedural horizon -> zenith gradient with sun glow and disc, covered by the zone's sky box
// textures the way the engine's `Sky` shader blends them (sky.cfx). See `sky_material.rs` for the layout.
//
// Colours are HDR and already multiplied by the view exposure (CPU side).

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
};

struct SkyParams {
    // rgb: colour at the horizon, w: exponent of the elevation -> zenith blend
    horizon: vec4<f32>,
    // rgb: colour at the zenith
    zenith: vec4<f32>,
    // xyz: direction towards the sun, w: sun visibility 0..1
    sun_dir: vec4<f32>,
    // rgb: glow colour, w: disc radiance multiplier
    sun_color: vec4<f32>,
    // rgb: shading colour of the sky box from the sky (`ShadeColorFromSky * SkyColorMultiplier`), w: opacity of the box
    box_sky: vec4<f32>,
    // rgb: shading colour from the sun, w: attenuation
    box_sun: vec4<f32>,
    // x: alpha saturation, y: cos(angle), z: sin(angle) of the box rotation about the vertical, w: 1 when textures are bound
    box_params: vec4<f32>,
    // x: e-fold height of the gradient (degrees of elevation, 0: use the exponent in `horizon.w`), y: phase gain,
    // z: weight of the engine's night sky gradient (the sun below the horizon), w: its zenith shift
    shape: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> sky: SkyParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var box_12: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var box_12_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var box_34: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var box_34_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var box_5: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var box_5_sampler: sampler;

// The sky box: four side faces (north, east, south, west going clockwise), each stored as its upper half (1024 x 512,
// `box_12` holds the first two, the second one flipped vertically; `box_34` the others) and the top face `box_5`.
// `d` is a direction in the box frame, y up, y >= 0.
fn sky_box(d: vec3<f32>) -> vec4<f32> {
    let ax = abs(d.x);
    let az = abs(d.z);
    if d.y >= ax && d.y >= az {
        // top: north at the bottom of the image, east at the right
        let uv = vec2<f32>((d.x / d.y + 1.0) * 0.5, (1.0 - d.z / d.y) * 0.5);
        return textureSampleLevel(box_5, box_5_sampler, uv, 0.0);
    }
    var s: f32;
    var t: f32;
    var face: u32;
    if az >= ax {
        t = d.y / az;
        if d.z < 0.0 {
            face = 0u;
            s = d.x / az;
        } else {
            face = 2u;
            s = -d.x / az;
        }
    } else {
        t = d.y / ax;
        if d.x > 0.0 {
            face = 1u;
            s = d.z / ax;
        } else {
            face = 3u;
            s = -d.z / ax;
        }
    }
    let u = (s + 1.0) * 0.5;
    // rows of the half face: 0 at the top edge (t = 1) .. 1 at the horizon (t = 0)
    let r = (1.0 - t) * 0.5;
    switch face {
        case 0u: { return textureSampleLevel(box_12, box_12_sampler, vec2<f32>(u, r), 0.0); }
        case 1u: { return textureSampleLevel(box_12, box_12_sampler, vec2<f32>(u, 1.0 - r), 0.0); }
        case 2u: { return textureSampleLevel(box_34, box_34_sampler, vec2<f32>(u, r), 0.0); }
        default: { return textureSampleLevel(box_34, box_34_sampler, vec2<f32>(u, 1.0 - r), 0.0); }
    }
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let d = normalize(in.world_position.xyz - view.world_position);
    let h = d.y;
    let e = clamp(h, 0.0, 1.0);
    var col: vec3<f32>;
    let cos_sun = dot(d, sky.sun_dir.xyz);
    if sky.shape.x > 0.0 {
        // the sky light falls off with the elevation, and is brighter towards the sun (Rayleigh phase, normalised to the
        // side of the sky, 100 degrees from the sun)
        let elevation = degrees(asin(e));
        col = mix(sky.zenith.rgb, sky.horizon.rgb, exp(-elevation / sky.shape.x));
        let phase = (1.0 + cos_sun * cos_sun) / 1.03;
        col = col * (1.0 + sky.shape.y * (phase - 1.0));
    } else {
        col = mix(sky.horizon.rgb, sky.zenith.rgb, pow(e, sky.horizon.w));
    }
    if sky.shape.z > 0.0 {
        // the engine's night sky gradient (`skyhdr.cfx`, `saturate(z * shift.x + shift.y)` eased): the horizon colour from
        // 0.1 below the horizon (the engine's minimum gradient), the zenith colour from the height `Night sky: Zenith shift`
        // up. The engine adds the scattered sun light on top, which the blend above stands for by day; at night it is this.
        let s = max(sky.shape.w + 0.1, 0.01);
        var gr = saturate((h + 0.1) / s);
        gr = gr * (2.0 - gr);
        col = mix(col, mix(sky.horizon.rgb, sky.zenith.rgb, gr), sky.shape.z);
    }

    // sun: a wide glow, a halo and the disc
    let mu = max(cos_sun, 0.0);
    let glow = pow(mu, 6.0) * 0.12 + pow(mu, 48.0) * 0.35 + pow(mu, 1200.0) * 2.0;
    col = col + sky.sun_color.rgb * glow * sky.sun_dir.w;
    let disc = smoothstep(0.99985, 0.99995, mu);
    col = col + sky.sun_color.rgb * sky.sun_color.w * disc * sky.sun_dir.w;

    // Native 380C3E40 adds the atmosphere after stars/moons; the alpha Sky box
    // is dispatched later by 38179EB0. Keep the existing combined RGB exactly,
    // and output its box opacity: ONE/INV_SRC_ALPHA gives G*(1-A)+box*A+C*(1-A).
    var box_alpha = 0.0;
    // sky box textures
    if h > 0.0 && sky.box_params.w > 0.5 {
        let c = sky.box_params.y;
        let sn = sky.box_params.z;
        let local = vec3<f32>(d.x * c - d.z * sn, d.y, d.x * sn + d.z * c);
        let t = sky_box(local);
        // the engine reads the red channel as it is stored (gamma space)
        let opacity = pow(t.x, 1.0 / 2.2);
        let density = opacity * 8.0;
        let k = exp2(-sky.box_sun.w * density);
        let shade = mix(sky.box_sky.rgb, sky.box_sun.rgb, k);
        let a = t.a * sky.box_sky.w * pow(opacity, sky.box_params.x);
        // faint near the horizon where the haze takes over
        let fade = smoothstep(0.0, 0.08, h);
        box_alpha = clamp(a * fade, 0.0, 1.0);
        col = mix(col, t.rgb * shade, box_alpha);
    }
    return vec4<f32>(col, box_alpha);
}
