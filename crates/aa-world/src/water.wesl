import aa_world::water_motion::{coordinates,foam_coordinates,endpoint_weights,sun_normal,bump_normal,refraction_uv,refraction_behind};
// Water volumes and the sea, see `water_material.rs`.
//
// Rendered in the transmissive phase: the opaque scene of the frame is available as `view_transmission_texture`
// and the depth prepass gives the scene depth. Colours are HDR, already multiplied by the view exposure (CPU side).
//
// The engine's `Watersurface` shader and the volume fog of the water volume, in that order:
//
//   body       = mix(fog colour, refracted scene, exp(-density * thickness))       (volume fog, constant colour)
//   colour     = mix(body, reflection_amount * reflection, transparency * fresnel)  (surface)
//              + sun glint + procedural foam (`water_foam.dds`)
//   reflection = the environment cube map of the material times sun + sky (the engine's), or the analytic sky (the sea);
//                optionally the scene where a screen space ray march finds it

import bevy_pbr::render::{
    forward_io::{Vertex,VertexOutput,decompress_vertex},
    mesh_functions,
    view_transformations::position_world_to_clip,
    mesh_view_bindings::{view, globals},
    view_transformations::depth_ndc_to_view_z,
};
@if(DEPTH_PREPASS) import bevy_pbr::prepass::utils::prepass_depth;
@if(VIEW_TRANSMISSION_TEXTURE) import bevy_pbr::render::mesh_view_bindings::{view_transmission_texture, view_transmission_sampler};

struct WaterParams {
    // rgb: colour of deep water (pre-exposed), a: absorption per metre
    body: vec4<f32>,
    // x: fresnel bias (reflectance at normal incidence), y: reflection weight (the material's TransparencyRatio times
    // a gain), z: bump scale, w: sun shine exponent
    refl: vec4<f32>,
    // x: big waves scale, y: small waves scale, z: speed, w: texture repeats per metre
    waves: vec4<f32>,
    // rgb: sun colour times the material's sun multiplier (pre-exposed), a: soft shore factor
    sun: vec4<f32>,
    // xyz: direction towards the sun, w: sun visibility
    sun_dir: vec4<f32>,
    // rgb: sky colour at the horizon (pre-exposed), w: exponent of the blend towards the zenith
    sky_h: vec4<f32>,
    // rgb: sky colour at the zenith
    sky_z: vec4<f32>,
    // x: screen space reflections on (1) / off (0), y: refraction strength, z: reflection roughness (spread of the rays),
    // w: the material's ReflectionAmount (scales the reflected colour)
    misc: vec4<f32>,
    // x: foam intersection factor (how deep the foam reaches), y: waves speed (drifts the foam), z: mip level of the environment
    // cube map lookup; private qualified sea w: source SoftIntersectionFactor
    foam: vec4<f32>,
    // rgb: foam colour: (sun + sky) times the material's foam multiplier (pre-exposed); zero = no foam
    foam_col: vec4<f32>,
    // rgb: the engine's `sun + sky` colour (pre-exposed), a: 1 = the reflection is the environment cube map times it
    // (ponds, lakes, rivers), 0 = the analytic sky (the sea)
    env_lit: vec4<f32>,
    // authored mesh UV / NO_FLOW / diagnostic time (-1 live) / output (1 wave, 2 alpha,
    // 3 sun, 4 reflection, 5 NdotI/Fresnel, 6 compressed bump; +10 old-normal diagnostics,
    // 8 injected cube-sampler ray; -1 old softness, -2 old normal stages, -3 old refraction).
    // Private source-qualified sea: x=-1, y=RefractionBumpScale; not authored NO_FLOW.
    motion: vec4<f32>,
    // sea only (`water.cfx` FOAM): FoamSoftIntersectionFactor, FoamCoverFactor, FoamCoverOffset, FoamCoverBlendWeight
    ocean_foam: vec4<f32>,
    // sea only: x foam repeats per metre, yz foam drift per second, w gloss map repeats per metre
    ocean_uv: vec4<f32>,
    // sea only: rgb sun colour x saturate(FoamAmount) (pre-exposed), a: 1 = ocean foam on
    ocean_sun: vec4<f32>,
    // sea only: rgb sky colour x saturate(FoamAmount) (pre-exposed)
    ocean_sky: vec4<f32>,
    // sea only (deferredcaustics.cfx): rgb sun colour x 0.5 x CausticIntensity (pre-exposed, zero = off), a: CausticDepth
    caustic_col: vec4<f32>,
    // sea only: x CausticsTilling, y caustics distance; private qualified sea zw: source FresnelBias/Power
    caustics: vec4<f32>,
    // sea only: the swell (vertex displacement): x peak height in metres (0 = flat), y repeat length of the base wave (m),
    // z speed factor, w wind direction (radians, Cry XY)
    ocean_wave: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> water: WaterParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var normal_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var normal_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var foam_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var foam_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var env_map: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var env_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(7) var environment_2d: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(8) var environment_2d_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(9) var gloss_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(10) var gloss_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(11) var caustics_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(12) var caustics_map_sampler: sampler;

// Matches water_material::AUTHORED_WATER_CUBE: actual authored static cube,
// ENVIRONMENT_MAP+BUMP_MAP, no REALTIME_REFLECTIONS, successful source load.
const AUTHORED_WATER_CUBE: f32 = 2.0;
// Same source gate, loaded ordinary 2D DDS: native TT3_TCUBE is absent.
const AUTHORED_WATER_ENVIRONMENT_2D: f32 = 3.0;

// The sea's swell: four sine waves around the wind direction with the wavelengths `y / 1, 2, 4, 8` (the base wave is the repeat
// length of the engine's FFT texture, `water.cfx` `ApplyVertexDisplacement`) and a spectrum falling with the frequency; deep
// water dispersion `w = sqrt(g k)`. Returns the height in x and the slopes dh/dx, dh/dz in yz. Each wave fades out with the
// distance at which the polar grid (2.45 % cell size) no longer samples it (4 cells per wavelength), and all of them within
// 2 m of the camera (`fDisplaceAtten = saturate(dist x 0.5)^2`).
fn swell(xz: vec2<f32>, time: f32, dist: f32) -> vec3<f32> {
    let wind = vec2<f32>(cos(water.ocean_wave.w), -sin(water.ocean_wave.w));
    var h = 0.0;
    var slope = vec2<f32>(0.0);
    var weights = array<f32, 4>(0.45, 0.30, 0.18, 0.07);
    var angles = array<f32, 4>(0.0, 0.45, -0.55, 1.0);
    for (var i = 0; i < 4; i++) {
        let wavelength = water.ocean_wave.y / f32(1 << u32(i));
        let k = 6.2831853 / wavelength;
        let c = cos(angles[i]);
        let s = sin(angles[i]);
        let dir = vec2<f32>(wind.x * c - wind.y * s, wind.x * s + wind.y * c);
        let phase = k * dot(dir, xz) - sqrt(9.81 * k) * water.ocean_wave.z * time + f32(i) * 1.7;
        let fade = 1.0 - smoothstep(0.6, 1.0, dist / (wavelength * 10.0));
        let amp = water.ocean_wave.x * weights[i] * fade;
        h += amp * sin(phase);
        slope += dir * (amp * k * cos(phase));
    }
    let near = clamp(dist * 0.5, 0.0, 1.0);
    let atten = near * near;
    return vec3<f32>(h * atten, slope * atten);
}

// Static authored water and ocean meshes use the standard Bevy world transform.
// Native WaterSurfaceVS carries endpoint alpha twice: RGB untouched and A near-faded.
@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let vertex=decompress_vertex(input,input.instance_index);
    let matrix=mesh_functions::get_world_from_local(input.instance_index);
    var out: VertexOutput;
    out.world_position=mesh_functions::mesh_position_local_to_world(matrix,vec4<f32>(vertex.position,1.0));
    if water.ocean_wave.x > 0.0 {
        out.world_position.y += swell(out.world_position.xz, globals.time, distance(out.world_position.xz, view.world_position.xz)).x;
    }
    out.position=position_world_to_clip(out.world_position.xyz);
    out.world_normal=mesh_functions::mesh_normal_local_to_world(vertex.normal,input.instance_index);
    @if(VERTEX_UVS_A) {out.uv=vertex.uv;}
    @if(VERTEX_UVS_B) {out.uv_b=vertex.uv_b;}
    @if(VERTEX_TANGENTS) {out.world_tangent=mesh_functions::mesh_tangent_local_to_world(matrix,vertex.tangent,input.instance_index);}
    @if(VERTEX_COLORS) {
        out.color=vertex.color;
        if water.motion.x > 0.5 {
            let a=vertex.color.a;
            out.color=vec4<f32>(a,a,a,a*(1.0-clamp(1.0/out.position.w,0.0,1.0)));
        }
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {out.instance_index=input.instance_index;}
    @if(VISIBILITY_RANGE_DITHER) {out.visibility_range_dither=mesh_functions::get_visibility_range_dither_level(input.instance_index,matrix[3]);}
    return out;
}

// Depth of the opaque scene at a pixel (reverse z, 0 = far / nothing drawn); 0 without a depth prepass.
@if(DEPTH_PREPASS)
fn scene_depth(pixel: vec2<f32>) -> f32 {
    return prepass_depth(vec4<f32>(pixel, 0.0, 0.0), 0u);
}
@else
fn scene_depth(pixel: vec2<f32>) -> f32 {
    return 0.0;
}

// The opaque scene of this frame at a screen uv; the body colour without the transmission copy.
@if(VIEW_TRANSMISSION_TEXTURE)
fn scene_color(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(view_transmission_texture, view_transmission_sampler, uv, 0.0).rgb;
}
@else
fn scene_color(uv: vec2<f32>) -> vec3<f32> {
    return water.body.rgb;
}

// Native screenMapSampler is POINT/CLAMP; Bevy's shared transmission sampler is
// linear. The copy already contains linear HDR colour, so no second sRGB decode.
@if(VIEW_TRANSMISSION_TEXTURE)
fn scene_color_point(uv: vec2<f32>) -> vec3<f32> {
    let size=vec2<i32>(textureDimensions(view_transmission_texture));
    let pixel=clamp(vec2<i32>(uv*vec2<f32>(size)),vec2<i32>(0),size-vec2<i32>(1));
    return textureLoad(view_transmission_texture,pixel,0).rgb;
}
@else
fn scene_color_point(uv: vec2<f32>) -> vec3<f32> {
    return water.body.rgb;
}

@if(VIEW_TRANSMISSION_TEXTURE)
fn target_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(view_transmission_texture));
}
@else
fn target_size() -> vec2<f32> {
    return view.viewport.zw;
}

fn sky_color(d: vec3<f32>) -> vec3<f32> {
    let e = clamp(d.y, 0.0, 1.0);
    var col = mix(water.sky_h.rgb, water.sky_z.rgb, pow(e, water.sky_h.w));
    let mu = max(dot(d, water.sun_dir.xyz), 0.0);
    col = col + water.sun.rgb * (pow(mu, 6.0) * 0.02 + pow(mu, 48.0) * 0.05) * water.sun_dir.w;
    return col;
}

// The four layers of the bump map the engine adds up: two big ones and two small ones, drifting.
fn wave_normal(p: vec2<f32>, mesh_uv: vec2<f32>, time: f32) -> vec3<f32> {
    let native = water.motion.x > 0.5;
    let uv = select(p * water.waves.w, mesh_uv, native);
    let sample_time = select(time,water.motion.z,water.motion.z >= 0.0);
    let drift = vec2<f32>(sample_time * water.waves.z * select(0.012,0.0125,native),0.0);
    let taps = coordinates(uv,drift,native && water.motion.y > 0.5);
    let a = textureSample(normal_map,normal_map_sampler,taps[0]).xy;
    let b = textureSample(normal_map,normal_map_sampler,taps[1]).xy;
    let c = textureSample(normal_map,normal_map_sampler,taps[2]).xy;
    let d = textureSample(normal_map,normal_map_sampler,taps[3]).xy;
    let low = (a + b) * water.waves.x - vec2<f32>(water.waves.x);
    let high = (c + d) * water.waves.y - vec2<f32>(water.waves.y);
    let t = (low + high) * water.refl.z;
    // texture x -> world +X, texture y -> cry +Y = bevy -Z
    var normal = vec3<f32>(t.x, 1.0, -t.y);
    if water.ocean_wave.x > 0.0 {
        // the swell tilts the surface: the normal of the height field is (-dh/dx, 1, -dh/dz)
        let s = swell(p, globals.time, distance(p, view.world_position.xz));
        normal = vec3<f32>(normal.x - s.y, 1.0, normal.z - s.z);
    }
    return normalize(normal);
}

// The ocean's deferred caustics (`deferredcaustics.cfx` DeferredCasticsPS) at a sea bed point `bed` (Bevy world):
// rgb is added light, a darkens the bed (the forward pass blends ONE / ONE_MINUS_SRC_ALPHA). Computed in Cry axes.
// Only below the ocean level `level`: from 2.5 m down (soft) to CausticDepth (fading over its last 2 m).
fn ocean_caustics(bed: vec3<f32>, level: f32, eye_dist: f32, time: f32) -> vec4<f32> {
    let pos = vec3<f32>(bed.x, -bed.z, bed.y);
    let sun = normalize(vec3<f32>(water.sun_dir.x, -water.sun_dir.z, water.sun_dir.y));
    // mLightView: the coordinates seen from the sun
    let forward = -sun;
    let side = normalize(select(cross(vec3<f32>(0.0, 0.0, 1.0), forward), vec3<f32>(1.0, 0.0, 0.0), abs(forward.z) > 0.999));
    let top = cross(forward, side);
    let proj = vec2<f32>(dot(side, pos), dot(top, pos)) * water.caustics.x;
    // vAnimParams, from the forward CommonCausticsPass (g_VS_AnimGenParams.w = time / 8)
    let t = time * 0.125;
    let anim = vec4<f32>(0.06 * t, 0.05 * t, 0.1 * t, -0.11 * t);
    let wave_tc = proj.xyxy * vec4<f32>(0.00024, 0.00024, 0.0002, 0.0002) + anim.xxyy;
    let temp = proj.xyxy * vec4<f32>(0.005, 0.005, 0.02, 0.02);
    let tc0 = temp.xyyx + anim.zzww;
    let tc1 = temp.zwwz + anim.zzww;
    let raw = textureSample(normal_map, normal_map_sampler, wave_tc.xy).xy + textureSample(normal_map, normal_map_sampler, wave_tc.wz).xy;
    let wave = raw * 0.04 - vec2<f32>(0.04);
    let low = textureSample(normal_map, normal_map_sampler, tc0.xy + wave).xy + textureSample(normal_map, normal_map_sampler, tc0.wz + wave).xy;
    let high = textureSample(normal_map, normal_map_sampler, tc1.xy + wave).xy + textureSample(normal_map, normal_map_sampler, tc1.wz + wave).xy;
    let n = normalize(vec3<f32>(low + high * 2.0 - vec2<f32>(3.0), 1.0));
    let r = refract(sun, n, 0.9).xy;
    let g = r - n.xy * 0.05;
    let b = r - n.xy * 0.1;
    let orig = vec3<f32>(
        textureSample(caustics_map, caustics_map_sampler, r * 0.5 + vec2<f32>(0.5)).x,
        textureSample(caustics_map, caustics_map_sampler, g * 0.5 + vec2<f32>(0.5)).x,
        textureSample(caustics_map, caustics_map_sampler, b * 0.5 + vec2<f32>(0.5)).x);
    // pow(c, 16) emulated as in the shader
    var c = clamp(5.65 * orig - vec3<f32>(4.66), vec3<f32>(0.0), vec3<f32>(1.0));
    c = c * c * clamp(dot(n, sun), 0.0, 1.0) * water.caustic_col.rgb;
    let darken = clamp(dot(vec3<f32>(1.0) - orig, vec3<f32>(0.25)), 0.0, 1.0);
    var atten = clamp(1.0 - exp(-(level - 2.5 - pos.z) * 2.0), 0.0, 1.0);
    atten = atten * clamp(water.caustics.y / (0.075 * eye_dist * eye_dist), 0.0, 1.0);
    let depth = water.caustic_col.a;
    atten = atten * (1.0 - smoothstep(level - depth + 2.0, level - depth, pos.z));
    return vec4<f32>(c, darken) * atten;
}

fn linear_depth(ndc_z: f32) -> f32 {
    return -depth_ndc_to_view_z(max(ndc_z, 1e-9));
}

// Screen space reflection: returns the uv of the hit (x < 0 when nothing was hit).
fn trace(p: vec3<f32>, dir: vec3<f32>, pixel: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    var t = 0.6;
    var prev_t = 0.0;
    for (var i = 0; i < 16; i = i + 1) {
        let clip = view.clip_from_world * vec4<f32>(p + dir * t, 1.0);
        if clip.w <= 0.0 {
            break;
        }
        let ndc = clip.xyz / clip.w;
        let suv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
        if suv.x < 0.0 || suv.x > 1.0 || suv.y < 0.0 || suv.y > 1.0 {
            break;
        }
        let sd = scene_depth(suv * size);
        if sd > ndc.z && sd > 0.0 {
            // behind the scene: a hit when it is not far behind (thin objects let the ray pass)
            if linear_depth(ndc.z) - linear_depth(sd) < 0.6 + t * 0.15 {
                // refine between the last two samples
                let tm = 0.5 * (t + prev_t);
                let cm = view.clip_from_world * vec4<f32>(p + dir * tm, 1.0);
                if cm.w > 0.0 {
                    let nm = cm.xyz / cm.w;
                    let um = nm.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
                    if um.x > 0.0 && um.x < 1.0 && um.y > 0.0 && um.y < 1.0 && scene_depth(um * size) > nm.z {
                        return um;
                    }
                }
                return suv;
            }
        }
        prev_t = t;
        t = t * 1.3 + 0.3;
    }
    return vec2<f32>(-1.0);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let p = in.world_position.xyz;
    let inspect_mode = select(water.motion.w,water.motion.w-10.0,water.motion.w>=10.0);
    // Isolated native/legacy foam-projection controls. Injected Cry XY avoids
    // moving the real authored meshes or rounding large world coordinates in a camera.
    if water.motion.w == 30.0 || water.motion.w == 31.0 {
        let taps=foam_coordinates(water.sky_h.xy,water.motion.z,water.foam.y,water.motion.x>0.5);
        let uv=select(taps[0],taps[1],water.motion.w==31.0);
        return vec4<f32>(fract(uv),0.2,1.0);
    }
    var endpoints=vec2<f32>(1.0);
    @if(VERTEX_COLORS) {
        if water.motion.x > 0.5 {endpoints=endpoint_weights(in.color);}
    }
    if inspect_mode > 1.5 && inspect_mode < 2.5 {
        return vec4<f32>(endpoints,0.0,1.0);
    }
    if inspect_mode > 0.5 && inspect_mode < 1.5 {
        return vec4<f32>(wave_normal(p.xz,in.uv,globals.time)*0.5+0.5,1.0);
    }
    let to_eye = view.world_position - p;
    let dist = length(to_eye);
    let v = to_eye / max(dist, 1e-4);
    let frag = in.position;
    let size = target_size();
    let pixel = frag.xy;

    let up = vec3<f32>(0.0, 1.0, 0.0);
    let native_normals = water.motion.x > 0.5 && water.motion.w != -2.0 && water.motion.w < 10.0;
    let n = sun_normal(wave_normal(p.xz,in.uv,globals.time),dist,native_normals);
    let bump = bump_normal(n,native_normals);
    let nr = normalize(bump);
    // Exact private tag plus bounded source bundle, depth AND colour copy.
    // Normal modes 0/1/2/3 and missing inputs keep their old branches.
    // Qualified foam uses this bundle on both sides; refraction/Fresnel only
    // use it below. The sea's generated normals are still approximated.
    var qualified_ocean = false;
    @if(DEPTH_PREPASS) {
        @if(VIEW_TRANSMISSION_TEXTURE) {
            qualified_ocean = water.motion.x == -1.0
                && water.motion.y >= 0.0 && water.motion.y <= 1.0
                && water.foam.w >= 0.0 && water.foam.w <= 1000.0
                && water.caustics.z >= 0.0 && water.caustics.z <= 1.0
                && water.caustics.w >= 0.0 && water.caustics.w <= 5.0;
        }
    }
    let ocean_underside = qualified_ocean && view.world_position.y < p.y;
    // Native Fresnel deliberately uses the compressed, unnormalized bump.
    var ndotv = clamp(abs(dot(v, select(nr,bump,native_normals))), 0.0, 1.0);
    let f0 = water.refl.x;
    var fresnel = clamp((f0 + (1.0 - f0) * pow(1.0 - ndotv, 5.0)) * water.refl.y, 0.0, 1.0);
    if ocean_underside {
        // water.cfx: dot(full vNormal,view)*sign(view.z), then GetFresnel.
        // Negative signed incidence saturates the internal reflection to one;
        // abs(dot) is a different shader's convention (WaterSurface).
        ndotv = dot(v,n) * sign(v.y);
        fresnel = clamp(water.caustics.z + (1.0-water.caustics.z)*pow(1.0-ndotv,water.caustics.w),0.0,1.0);
    }
    if inspect_mode == 6.0 { return vec4<f32>(bump*0.5+vec3<f32>(0.5),1.0); }
    if inspect_mode == 5.0 { return vec4<f32>(ndotv,fresnel,0.0,1.0); }
    if inspect_mode == 4.0 { return vec4<f32>(nr*0.5+vec3<f32>(0.5),1.0); }
    if inspect_mode == 3.0 { return vec4<f32>(n*0.5+vec3<f32>(0.5),1.0); }
    // WaterSurfacePS is two-sided: its abs(NdotI) and refraction depth mask
    // also run inside water. The separately qualified WaterPS sea uses its own
    // source refraction below; unqualified/missing-input fallbacks still discard.
    var underside_supported = false;
    @if(DEPTH_PREPASS) {
        @if(VIEW_TRANSMISSION_TEXTURE) { underside_supported = water.motion.x > 0.5; }
    }
    let authored_underside = underside_supported && view.world_position.y < p.y;
    if view.world_position.y < p.y && !authored_underside && !ocean_underside { discard; }
    // the foam is looked up unconditionally (uniform control flow), see below
    let native_foam=water.motion.x>0.5;
    let foam_time=select(globals.time,water.motion.z,native_foam && water.motion.z>=0.0);
    let foam_taps=foam_coordinates(vec2<f32>(p.x,-p.z),foam_time,water.foam.y,native_foam && water.motion.w != -5.0);
    let foam_bump = vec2<f32>(n.x, -n.z) * (0.15 * 0.075);
    let foam_uv_a = foam_taps[0] + foam_bump;
    let foam_uv_b = foam_taps[1] + foam_bump;
    let foam_sum = textureSample(foam_map, foam_map_sampler, foam_uv_a).rgb + textureSample(foam_map, foam_map_sampler, foam_uv_b).rgb;

    // ---- under the surface: thickness of the water along the view ray, refracted scene ----
    var thickness = 4.0;
    let scene_ndc = scene_depth(pixel);
    if scene_ndc > 0.0 {
        let forward = -view.world_from_view[2].xyz;
        let along = max(dot(-v, forward), 0.05);
        thickness = clamp((linear_depth(scene_ndc) - linear_depth(frag.z)) / along, 0.0, 60.0);
    } else {
        thickness = 60.0;
    }
    var native_refraction=false;
    @if(DEPTH_PREPASS) {
        @if(VIEW_TRANSMISSION_TEXTURE) {
            native_refraction=water.motion.x>0.5 && water.motion.w != -3.0;
        }
    }
    var uv = pixel / size;
    if native_refraction {
        let shifted=refraction_uv(pixel,bump,view.viewport,size);
        if refraction_behind(frag.z,scene_depth(shifted*size)) {uv=shifted;}
    } else if ocean_underside {
        // Non-tessellated D3D10 WaterPS: axial-depth softness, full Cry XY
        // normal, source RefractionBumpScale and VS screenProj.z attenuation.
        // Do not reuse WaterSurface's compressed constant .1 offset or the
        // calibrated misc.y. Strict reverse-Z mask rejects equality/foreground.
        var behind = 1.0e4;
        if scene_ndc > 0.0 { behind = max(linear_depth(scene_ndc)-linear_depth(frag.z),0.0); }
        let soft_intersect = clamp(water.foam.w*behind,0.0,1.0);
        let depth_scale = clamp(1.0-0.15*sqrt(clamp(1.0/frag.w,0.0,1.0)),0.0,1.0);
        let delta = vec2<f32>(n.x,-n.z)*water.motion.y*soft_intersect*depth_scale;
        let shifted = clamp((pixel+delta*view.viewport.zw)/size,
            (view.viewport.xy+vec2<f32>(0.5))/size,
            (view.viewport.xy+view.viewport.zw-vec2<f32>(0.5))/size);
        if refraction_behind(frag.z,scene_depth(shifted*size)) {uv=shifted;}
    } else {
        let shift = n.xz * vec2<f32>(1.0, -1.0) * water.misc.y * clamp(thickness, 0.0, 1.0);
        let shifted = uv + shift;
        if scene_ndc <= 0.0 || scene_depth(shifted * size) <= frag.z * 1.0001 {
            uv = shifted;
        }
    }
    var refracted: vec3<f32>;
    if native_refraction || ocean_underside {refracted=scene_color_point(uv);} else {refracted=scene_color(uv);}
    // Sea bed caustics (looked up unconditionally: uniform control flow for the implicit derivatives)
    let bed = p - v * thickness;
    let caustic = ocean_caustics(bed, p.y, length(view.world_position - bed), globals.time);
    if water.caustic_col.a > 0.0 && scene_ndc > 0.0 && thickness < 59.0 {
        refracted = refracted * (1.0 - caustic.a) + caustic.rgb;
    }
    // WaterVolumeInto uses 1-a+T*a, independently of the surface's nonlinear fade.
    // The separate native Into fog only draws from above. The underwater
    // eye-to-surface volume Outof is applied by the post pass, once.
    let transmit = select(1.0-endpoints.x+exp(-water.body.a*thickness)*endpoints.x,1.0,authored_underside);
    let fogged_background=mix(water.body.rgb,scene_color(pixel/size),transmit);
    var colour = mix(water.body.rgb, refracted, transmit);

    // ---- reflection: the scene where the ray hits it, the sky elsewhere (two rays spread vertically: the surface is rough) ----
    var r = reflect(-v, nr);
    // Isolated original-DDS cube sampling controls inject a known reflected Bevy ray.
    if inspect_mode == 8.0 { r = water.sky_h.xyz; }
    // Varying signed Cry XY controls verify implicit mip derivatives.
    if inspect_mode == 9.0 { r = vec3<f32>(in.uv.x,0.0,-in.uv.y); }
    let folded_rdir = normalize(vec3<f32>(r.x, abs(r.y), r.z));
    // The native cube sampler consumes the reflected Cry vector without folding its Z.
    // The analytic sky, optional SSR and every fallback keep their existing hemisphere.
    let native_cube = water.motion.x == AUTHORED_WATER_CUBE && water.misc.x <= 0.5;
    let rdir = select(folded_rdir,r,native_cube);
    // the engine reflects a fixed cube map (Cry axes: x east, y north, z up) lit by the sun and the sky colour
    let cube = textureSampleLevel(env_map, env_map_sampler, vec3<f32>(rdir.x, -rdir.z, rdir.y), water.foam.z).rgb * water.env_lit.rgb;
    var environment = cube;
    if water.motion.x == AUTHORED_WATER_ENVIRONMENT_2D && water.misc.x <= 0.5 && water.motion.w != -4.0 {
        // WaterSurfacePS441: signed reflected Cry XY, no remap/fold. Native
        // tex2D uses implicit mip derivatives and the ordinary default SRV.
        environment = textureSample(environment_2d,environment_2d_sampler,vec2<f32>(r.x,-r.z)).rgb * water.env_lit.rgb;
    }
    if inspect_mode == 8.0 || inspect_mode == 9.0 { return vec4<f32>(environment,1.0); }
    let sky = mix(sky_color(folded_rdir), environment, water.env_lit.a);
    var refl = sky;
    if water.misc.x > 0.5 && dist < 160.0 {
        // rough water smears the reflection vertically: two rays above and below the mirror direction
        let side = normalize(cross(folded_rdir, up));
        let top = cross(side, folded_rdir);
        var sum = vec3<f32>(0.0);
        for (var k = 0; k < 2; k = k + 1) {
            let dir = normalize(folded_rdir + top * (water.misc.z * (f32(k) * 2.0 - 1.0)));
            let hit = trace(p, vec3<f32>(dir.x, abs(dir.y), dir.z), pixel, size);
            var c = sky;
            if hit.x >= 0.0 {
                let edge = min(min(hit.x, 1.0 - hit.x), min(hit.y, 1.0 - hit.y));
                c = mix(sky, scene_color(hit), smoothstep(0.0, 0.08, edge));
            }
            sum = sum + c;
        }
        refl = sum * 0.5;
    }

    // transparent where the water is thin
    let soft = clamp(water.sun.a * thickness, 0.0, 1.0);
    // Native WaterSurfacePS applies softIntersect once to final alpha. The sea keeps
    // its existing shading path; negative diagnostic mode is the isolated old control.
    let native_soft = water.motion.x > 0.5 && water.motion.w != -1.0;
    let contribution_soft = select(soft,1.0,native_soft);
    colour = mix(colour, refl * water.misc.w, fresnel * contribution_soft);

    // sun glint on the ripples
    let rs = reflect(-v, n);
    let glint = pow(clamp(dot(rs, water.sun_dir.xyz), 0.0, 1.0), water.refl.w);
    // Native selfshadowK = viewVec.z < 0 ? 0 : 1.
    colour = colour + water.sun.rgb * glint * water.sun_dir.w * contribution_soft * select(1.0,0.0,authored_underside);

    // procedural foam: the engine's two drifting lookups of the foam texture, strongest where the water is shallow
    // (around legs and objects, at the shore) but not where it vanishes
    let foam_lum = clamp(dot(foam_sum, vec3<f32>(0.333)) - 1.0, 0.0, 1.0);
    let foam_reach = clamp(water.foam.x * (0.2 / max(foam_lum, 1e-3)) * thickness, 0.0, 1.0);
    var foam = 1.0 - foam_reach;
    foam = foam * smoothstep(0.2, 0.7, soft * foam);
    colour = colour + water.foam_col.rgb * foam * foam_lum * contribution_soft;

    // The sea's own foam (`water.cfx` WaterPS, FOAM): one red-channel lookup of `waterfoam.dds` perturbed by the bump,
    // masked by the gloss map and by the linear depth of the water under the surface (shore and shallows, sparse flecks
    // on deep water), lit by sun x N.L + sky; added after the Fresnel blend.
    let sea_xy = vec2<f32>(p.x, -p.z);
    let sea_foam_tex = textureSample(foam_map, foam_map_sampler, sea_xy * water.ocean_uv.x + water.ocean_uv.yz * globals.time + vec2<f32>(n.x, -n.z) * 0.1).x;
    let sea_gloss = textureSample(gloss_map, gloss_map_sampler, sea_xy * water.ocean_uv.w).x * 0.7 + 0.3;
    if water.ocean_sun.a > 0.5 {
        var under = 1.0e4;
        if scene_ndc > 0.0 { under = max(linear_depth(scene_ndc) - linear_depth(frag.z), 0.0); }
        let fsi = clamp(water.ocean_foam.x * under, 0.0, 1.0) + water.ocean_foam.y;
        var sea_foam = sea_foam_tex * clamp(-fsi * (1.0 + (sea_gloss * 2.0 - 1.0)) + sea_foam_tex, 0.0, 1.0);
        sea_foam = sea_foam * clamp(fsi - pow(clamp(fsi - water.ocean_foam.z, 0.0, 1.0), water.ocean_foam.w), 0.0, 1.0);
        let ndotl = clamp(dot(n, water.sun_dir.xyz), 0.0, 1.0) * water.sun_dir.w;
        if qualified_ocean {
            // WaterPS1497-1502 keeps fFoam scalar: the light multiply takes
            // red, then adds the result equally to RGB. Both camera sides
            // also use saturate(screenProj.w-.33); fragment W is reciprocal
            // clip W, including the native orthographic clip W=1 case.
            // Reflection-derived shadow and receiver selection remain open.
            let light = water.ocean_sun.r * ndotl + water.ocean_sky.r;
            let camera_soft = clamp(1.0 / frag.w - 0.33, 0.0, 1.0);
            colour = colour + vec3<f32>(sea_foam * light * camera_soft);
        } else {
            colour = colour + sea_foam * (water.ocean_sun.rgb * ndotl + water.ocean_sky.rgb);
        }
    }

    // Native surface SRC_ALPHA blending, folded into the combined fog/surface pass.
    // Other full-shading and separate-pass boundaries remain explicit.
    let surface_alpha=endpoints.y*select(1.0,soft,native_soft);
    return vec4<f32>(mix(fogged_background,colour,surface_alpha),1.0);
}
