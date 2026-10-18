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
