// Exact native lookup spacing, SI_BendInfo and main branch from modificatorvt.cfi.
// Input positions are still Cry-local; world_from_local includes Cry->Bevy once.
fn lookup(index: i32) -> f32 {
    return sin(f32(index & 1023) * 0.3519061505794525 * 3.1415927410125732 * 0.0055555556900799274);
}
fn bend_info(p: vec4<f32>, world: vec3<f32>, seconds: f32) -> vec4<f32> {
    let frequency = 0.4 / (p.w + 1.0) + 0.2;
    let indices = vec2<i32>((vec2<f32>(frequency, frequency * 1.125) * seconds + world.xy * 0.125) * 1024.0);
    let waves = vec2<f32>(lookup(indices.x), lookup(indices.y)) * (p.xy * 0.008 - p.yx * 0.002);
    return vec4<f32>(p.z * (p.xy + waves * 50.0), p.z * length(p.xy) * 2.0, length(p.xy + waves) * 0.3);
}
fn triangle(x: vec4<f32>) -> vec4<f32> { return abs(fract(x + 0.5) * 2.0 - 1.0); }
// _DetailBending: leaf-edge flutter along the vertex normal (R) and branch bob (1-B), phase from G.
// `d` = frequency, leaf amplitude, branch amplitude, phase (material PublicParams); zero frequency = off.
// `c` = vertex R,G,B and the quantised normal xy (see vegetation_color/ao.rs), so every pass agrees.
fn detail(position: vec3<f32>, world: vec3<f32>, speed_in: f32, seconds: f32, grass: bool, d: vec4<f32>, c: vec4<f32>) -> vec3<f32> {
    if d.x == 0.0 { return position; }
    var speed = speed_in;
    if grass { speed *= position.z; }
    let nx = floor(c.w / 256.0);
    let normal_xy = vec2<f32>(nx, c.w - nx * 256.0) / 255.0 * 2.0 - 1.0;
    let branch_phase = c.y + 2.0 * (world.x + world.y + world.z);
    let vertex_phase = (position.x + position.y + position.z) * (d.w + branch_phase);
    let t = seconds + vec4<f32>(vertex_phase, vertex_phase, branch_phase, branch_phase);
    let waves = triangle((fract(t * vec4<f32>(1.975, 0.793, 0.375, 0.193)) * 2.0 - 1.0) * speed * d.x);
    let sum = vec2<f32>(waves.x + waves.y, waves.z + waves.w);
    return position + vec3<f32>(sum.x * c.x * d.y * normal_xy, sum.y * (1.0 - c.z) * d.z);
}
fn bend(position_in: vec3<f32>, world_from_local: mat4x4<f32>, p: vec4<f32>, seconds: f32, grass: bool, high_quality: bool, d: vec4<f32>, c: vec4<f32>) -> vec3<f32> {
    if p.z == 0.0 { return position_in; }
    let translation = world_from_local[3].xyz;
    let world = vec3<f32>(translation.x, -translation.z, translation.y);
    let info = bend_info(p, world, seconds);
    let position = detail(position_in, world, info.w, seconds, grass, d, c);
    // Native row-vector multiplication by the native 3x3, retaining authored uniform scale/tilt.
    let bevy_direction = vec3<f32>(info.x, info.z, -info.y);
    var xy = vec2<f32>(dot(bevy_direction,world_from_local[0].xyz),dot(bevy_direction,world_from_local[1].xyz));
    if high_quality {
        var speed = info.w;
        if grass { speed *= position_in.z; }
        let phase = (seconds + length(world) * 2.0) * 2.0;
        let wave_in = (fract(phase * vec4<f32>(0.95,0.45793,0.913,0.5793) * 0.1) * 2.0 - 1.0) * 0.7 * speed;
        let tri = triangle(wave_in);
        let wave = tri * tri * (3.0 - 2.0 * tri);
        xy += 0.3333 * clamp(xy.x + xy.y,0.0,1.0) * vec2<f32>(wave.x + wave.y,wave.z + wave.w);
    }
    xy *= 0.015;
    var factor = position.z * info.z + 1.0;
    factor *= factor;
    factor = factor * factor - factor;
    let moved = position + vec3<f32>(xy * factor,0.0);
    if dot(moved,moved) == 0.0 { return vec3<f32>(0.0); }
    return normalize(moved) * length(position);
}
