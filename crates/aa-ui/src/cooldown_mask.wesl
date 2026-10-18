// Original game/shaders/hwscripts/cryfx/x2ui.cfx, image-plus-mask path.
import bevy_sprite_render::mesh2d::vertex_output::VertexOutput;
import bevy_render::color_operations::{linear_to_srgb, srgb_to_linear};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> image_uv: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var<uniform> mask_uv: vec4<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var image_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var image_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var mask_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var mask_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let image_pos = image_uv.xy + (image_uv.zw - image_uv.xy) * in.uv;
    let mask_pos = mask_uv.xy + (mask_uv.zw - mask_uv.xy) * in.uv;
    let image = textureSample(image_texture, image_sampler, image_pos);
    let mask = textureSample(mask_texture, mask_sampler, mask_pos);
    // The UI's source colors and additive mask operate in sRGB space. Bevy's
    // sRGB textures and vertex colors arrive linear, and the target expects linear.
    var color = vec4<f32>(linear_to_srgb(image.rgb), image.a);
    @if(VERTEX_COLORS)
    color = color * vec4<f32>(linear_to_srgb(in.color.rgb), in.color.a);
    color = vec4<f32>(
        clamp(color.rgb + linear_to_srgb(mask.rgb) * 2.0 - 1.0, vec3<f32>(0.0), vec3<f32>(1.0)),
        color.a * mask.a,
    );
    return vec4<f32>(srgb_to_linear(color.rgb), color.a);
}
