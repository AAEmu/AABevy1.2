// Original WaterSurfaceVS Wave0.xy/Wave0.wz/Wave1.xy/Wave1.wz ordering.
fn coordinates(uv: vec2<f32>, drift: vec2<f32>, no_flow: bool) -> mat4x2<f32> {
    if no_flow {
        return mat4x2<f32>(uv*2.0+drift*2.0,uv*3.0-drift*2.0,uv*4.0+drift*2.0,uv*5.0-drift*2.0);
    }
    return mat4x2<f32>(uv*4.0+drift*2.0,uv*2.0+drift*3.0,uv*8.0+drift*2.0,uv*16.0+drift*3.0);
}

// Native WaterVolumeInto and WaterSurface opacity inputs, interpolated separately.
// x: linear authored fog weight; y: surface alpha after native vertex near fade.
fn endpoint_weights(color: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(color.r,sqrt(clamp(color.a,0.0,1.0))*color.r);
}

// WaterSurfacePS keeps the full normalized bump for sun shading. It has no distance
// attenuation; the non-authored sea retains its existing flattening approximation.
fn sun_normal(wave: vec3<f32>, distance: f32, native: bool) -> vec3<f32> {
    if native { return wave; }
    return normalize(mix(vec3<f32>(0.0,1.0,0.0),wave,1.0-smoothstep(60.0,260.0,distance)));
}

// Native Cry XY -> Bevy XZ. Fresnel uses this UNNORMALIZED value; reflection alone
// normalizes it (watersurface.cfx420-448). The legacy sea normalizes for both uses.
fn bump_normal(sun: vec3<f32>, native: bool) -> vec3<f32> {
    if native { return sun * vec3<f32>(0.15,1.0,0.15); }
    return mix(vec3<f32>(0.0,1.0,0.0),sun,0.2);
}

// WaterSurfacePS offsets viewport-normalized HPosToScreenTC by compressed Cry XY
// * .1 (full bump XY * .015), without a thickness fade. Bevy's depth/scene copies
// cover the full physical target, so translate the delta and POINT/CLAMP bounds.
fn refraction_uv(pixel: vec2<f32>, bump: vec3<f32>, viewport: vec4<f32>, size: vec2<f32>) -> vec2<f32> {
    let delta=bump.xz*vec2<f32>(1.0,-1.0)*0.1;
    return clamp((pixel+delta*viewport.zw)/size,
        (viewport.xy+vec2<f32>(0.5))/size,
        (viewport.xy+viewport.zw-vec2<f32>(0.5))/size);
}

// Native surface viewZ/far < sampled scene viewZ/far. Both ordinary projection
// types are reversed-Z in Bevy: equality/foreground reject, clear depth 0 accepts.
fn refraction_behind(surface: f32, scene: f32) -> bool {
    return scene < surface;
}
