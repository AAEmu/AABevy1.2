import aa_world::water_motion::{coordinates,endpoint_weights,sun_normal,bump_normal,refraction_uv,refraction_behind};
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
    // x: foam intersection factor (how deep the foam reaches), y: waves speed (drifts the foam), zw: unused
    foam: vec4<f32>,
    // rgb: foam colour: (sun + sky) times the material's foam multiplier (pre-exposed); zero = no foam
    foam_col: vec4<f32>,
    // rgb: the engine's `sun + sky` colour (pre-exposed), a: 1 = the reflection is the environment cube map times it
    // (ponds, lakes, rivers), 0 = the analytic sky (the sea)
    env_lit: vec4<f32>,
    // authored mesh UV / NO_FLOW / diagnostic time (-1 live) / output (1 wave, 2 alpha,
    // 3 sun, 4 reflection, 5 NdotI/Fresnel, 6 compressed bump; +10 old-normal diagnostics,
    // 8 injected cube-sampler ray; -1 old softness, -2 old normal stages, -3 old refraction)
    motion: vec4<f32>,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> water: WaterParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var normal_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var normal_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var foam_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var foam_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var env_map: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var env_map_sampler: sampler;

// Matches water_material::AUTHORED_WATER_CUBE: actual authored static cube,
// ENVIRONMENT_MAP+BUMP_MAP, no REALTIME_REFLECTIONS, successful source load.
const AUTHORED_WATER_CUBE: f32 = 2.0;

// Static authored water and ocean meshes use the standard Bevy world transform.
// Native WaterSurfaceVS carries endpoint alpha twice: RGB untouched and A near-faded.
@vertex
fn vertex(input: Vertex) -> VertexOutput {
    let vertex=decompress_vertex(input,input.instance_index);
    let matrix=mesh_functions::get_world_from_local(input.instance_index);
    var out: VertexOutput;
    out.world_position=mesh_functions::mesh_position_local_to_world(matrix,vec4<f32>(vertex.position,1.0));
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
    return normalize(vec3<f32>(t.x, 1.0, -t.y));
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
    // Native Fresnel deliberately uses the compressed, unnormalized bump.
    let ndotv = clamp(abs(dot(v, select(nr,bump,native_normals))), 0.0, 1.0);
    let f0 = water.refl.x;
    let fresnel = clamp((f0 + (1.0 - f0) * pow(1.0 - ndotv, 5.0)) * water.refl.y, 0.0, 1.0);
    if inspect_mode == 6.0 { return vec4<f32>(bump*0.5+vec3<f32>(0.5),1.0); }
    if inspect_mode == 5.0 { return vec4<f32>(ndotv,fresnel,0.0,1.0); }
    if inspect_mode == 4.0 { return vec4<f32>(nr*0.5+vec3<f32>(0.5),1.0); }
    if inspect_mode == 3.0 { return vec4<f32>(n*0.5+vec3<f32>(0.5),1.0); }
    // From below, the surface remains the post pass's responsibility.
    if view.world_position.y < p.y { discard; }
    // the foam is looked up unconditionally (uniform control flow), see below
    let foam_anim = fract(globals.time * 0.025 * water.foam.y);
    let foam_bump = vec2<f32>(n.x, -n.z) * (0.15 * 0.075);
    let foam_uv_a = vec2<f32>(p.x, -p.z) * 0.3197 - vec2<f32>(foam_anim) + foam_bump;
    let foam_uv_b = vec2<f32>(-p.z, p.x) * 0.2797 + vec2<f32>(foam_anim) + foam_bump;
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
    } else {
        let shift = n.xz * vec2<f32>(1.0, -1.0) * water.misc.y * clamp(thickness, 0.0, 1.0);
        let shifted = uv + shift;
        if scene_ndc <= 0.0 || scene_depth(shifted * size) <= frag.z * 1.0001 {
            uv = shifted;
        }
    }
    var refracted: vec3<f32>;
    if native_refraction {refracted=scene_color_point(uv);} else {refracted=scene_color(uv);}
    // WaterVolumeInto uses 1-a+T*a, independently of the surface's nonlinear fade.
    let transmit = 1.0-endpoints.x+exp(-water.body.a*thickness)*endpoints.x;
    let fogged_background=mix(water.body.rgb,scene_color(pixel/size),transmit);
    var colour = mix(water.body.rgb, refracted, transmit);

    // ---- reflection: the scene where the ray hits it, the sky elsewhere (two rays spread vertically: the surface is rough) ----
    var r = reflect(-v, nr);
    // Isolated original-DDS cube sampling controls inject a known reflected Bevy ray.
    if inspect_mode == 8.0 { r = water.sky_h.xyz; }
    let folded_rdir = normalize(vec3<f32>(r.x, abs(r.y), r.z));
    // The native cube sampler consumes the reflected Cry vector without folding its Z.
    // The analytic sky, optional SSR and every fallback keep their existing hemisphere.
    let native_cube = water.motion.x == AUTHORED_WATER_CUBE && water.misc.x <= 0.5;
    let rdir = select(folded_rdir,r,native_cube);
    // the engine reflects a fixed cube map (Cry axes: x east, y north, z up) lit by the sun and the sky colour
    let cube = textureSample(env_map, env_map_sampler, vec3<f32>(rdir.x, -rdir.z, rdir.y)).rgb * water.env_lit.rgb;
    if inspect_mode == 8.0 { return vec4<f32>(cube,1.0); }
    let sky = mix(sky_color(folded_rdir), cube, water.env_lit.a);
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
    colour = colour + water.sun.rgb * glint * water.sun_dir.w * contribution_soft;

    // procedural foam: the engine's two drifting lookups of the foam texture, strongest where the water is shallow
    // (around legs and objects, at the shore) but not where it vanishes
    let foam_lum = clamp(dot(foam_sum, vec3<f32>(0.333)) - 1.0, 0.0, 1.0);
    let foam_reach = clamp(water.foam.x * (0.2 / max(foam_lum, 1e-3)) * thickness, 0.0, 1.0);
    var foam = 1.0 - foam_reach;
    foam = foam * smoothstep(0.2, 0.7, soft * foam);
    colour = colour + water.foam_col.rgb * foam * foam_lum * contribution_soft;

    // Native surface SRC_ALPHA blending, folded into the combined fog/surface pass.
    // Other full-shading and separate-pass boundaries remain explicit.
    let surface_alpha=endpoints.y*select(1.0,soft,native_soft);
    return vec4<f32>(mix(fogged_background,colour,surface_alpha),1.0);
}
