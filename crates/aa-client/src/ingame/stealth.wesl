// CryEngine's cloak material layer (`MTL_LAYER_CLOAK`, `game/shaders/hwscripts/cryfx/cloaklayer.cfx`, see `stealth.rs`), as one additive pass.
//
// The two passes of the original:
//  * CloakRefrationPS (alpha blend): the scene behind the model, refracted (offset by the normal and the scanlines), plus a colour
//    from the 256 x 1 palette `cloak_palette.dds`, looked up at `interlation.y * (1 - N.E)` and scaled by `(1 - N.E)`: black in the
//    middle of a face turned to the camera, dark red -> magenta -> violet -> blue -> white towards the rim.
//  * CloakSparksPS (additive): blue sparks from three 3D noise lookups through `cloak_sparks.dds`, squared, `* SparksColor`
//    (0.3, 0.3, 1.0), modulated by the scanlines.
// The scene refraction is not reproduced (Bevy has no scene colour texture for this): the model is see-through and the palette colour is
// added on top, which is what the refraction pass amounts to when the refracted pixel is the pixel behind. The palette and the noise
// are procedural.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
};

struct CloakParams {
    // x: palette gain, y: sparks gain, z: unused, w: unused
    values: vec4<f32>,
}
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> cloak: CloakParams;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

// cloak_palette.dds sampled at `t` (0..1): the key colours of the file (0..255 values), linear between them.
fn palette(t: f32) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0) * 255.0;
    var a = vec3<f32>(0.0);
    var b = vec3<f32>(0.0);
    var f = 0.0;
    if x < 80.0 {
        a = vec3<f32>(0.0, 0.0, 0.0); b = vec3<f32>(58.0, 0.0, 0.0); f = x / 80.0;
    } else if x < 128.0 {
        a = vec3<f32>(58.0, 0.0, 0.0); b = vec3<f32>(59.0, 8.0, 32.0); f = (x - 80.0) / 48.0;
    } else if x < 160.0 {
        a = vec3<f32>(59.0, 8.0, 32.0); b = vec3<f32>(50.0, 18.0, 74.0); f = (x - 128.0) / 32.0;
    } else if x < 192.0 {
        a = vec3<f32>(50.0, 18.0, 74.0); b = vec3<f32>(37.0, 30.0, 122.0); f = (x - 160.0) / 32.0;
    } else if x < 208.0 {
        a = vec3<f32>(37.0, 30.0, 122.0); b = vec3<f32>(30.0, 76.0, 246.0); f = (x - 192.0) / 16.0;
    } else if x < 224.0 {
        a = vec3<f32>(30.0, 76.0, 246.0); b = vec3<f32>(78.0, 127.0, 255.0); f = (x - 208.0) / 16.0;
    } else {
        a = vec3<f32>(78.0, 127.0, 255.0); b = vec3<f32>(166.0, 192.0, 255.0); f = min((x - 224.0) / 16.0, 1.0);
    }
    return mix(a, b, f) / 255.0;
}

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// Value noise in 0..1 (Noise3D.dds stand in).
fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let n000 = hash3(i);
    let n100 = hash3(i + vec3<f32>(1.0, 0.0, 0.0));
    let n010 = hash3(i + vec3<f32>(0.0, 1.0, 0.0));
    let n110 = hash3(i + vec3<f32>(1.0, 1.0, 0.0));
    let n001 = hash3(i + vec3<f32>(0.0, 0.0, 1.0));
    let n101 = hash3(i + vec3<f32>(1.0, 0.0, 1.0));
    let n011 = hash3(i + vec3<f32>(0.0, 1.0, 1.0));
    let n111 = hash3(i + vec3<f32>(1.0, 1.0, 1.0));
    return mix(mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y), mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y), u.z);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let e = normalize(view.world_position - in.world_position.xyz);
    let n_dot_e = saturate(dot(e, n));

    // the scanlines: every fourth pixel row of the screen, a triangle wave (`cloak_interlation.dds`: y 0.8 .. 0.95, w 0 .. 1)
    let row = floor(in.position.y) * 0.25 + fract(globals.time * 7.0) * 0.5;
    let wave = abs(fract(row) * 2.0 - 1.0);
    let inter_y = 0.8 + 0.15 * wave;

    // pass 1: the palette colour at the grazing angle
    let look = inter_y * saturate(1.0 - n_dot_e);
    let rim = srgb_to_linear(palette(look)) * saturate(1.0 - n_dot_e) * cloak.values.x;

    // pass 2: the sparks, three noise lookups (each through the spark map in the original) of the position and time
    let t = globals.time * 0.4;
    let p = in.world_position.xyz * 6.0;
    var s = noise3(p + vec3<f32>(0.0, t * 2.0, 0.0));
    s = noise3(p + s * 0.2 + vec3<f32>(t, 0.0, t));
    s = noise3(p + s * 0.4 + vec3<f32>(0.0, t * 1.5, t));
    let spark = pow(saturate(s * 2.2 - 1.05), 2.0);
    let sparks = vec3<f32>(0.3, 0.3, 1.0) * spark * spark * (0.4 + 0.6 * wave) * cloak.values.y;

    // Bevy's Add material uses premultiplied blending: alpha 0 keeps the destination and adds the colour
    return vec4<f32>(rim + sparks, 0.0);
}
