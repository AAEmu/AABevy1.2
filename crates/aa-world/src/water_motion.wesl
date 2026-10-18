// Original WaterSurfaceVS Wave0.xy/Wave0.wz/Wave1.xy/Wave1.wz ordering.
fn coordinates(uv: vec2<f32>, drift: vec2<f32>, no_flow: bool) -> mat4x2<f32> {
    if no_flow {
        return mat4x2<f32>(uv*2.0+drift*2.0,uv*3.0-drift*2.0,uv*4.0+drift*2.0,uv*5.0-drift*2.0);
    }
    return mat4x2<f32>(uv*4.0+drift*2.0,uv*2.0+drift*3.0,uv*8.0+drift*2.0,uv*16.0+drift*3.0);
}
