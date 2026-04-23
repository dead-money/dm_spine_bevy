// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

// Mesh2d-compatible shader for dm_spine_bevy. Per-material uniform carries
// the slot's premultiplied light color and its tint-black dark color; the
// runtime batcher guarantees every vertex in one RenderCommand shares the
// same colors, so we pass them as uniforms rather than per-vertex attributes.

#import bevy_sprite::{
    mesh2d_functions as mesh_functions,
    mesh2d_vertex_output::VertexOutput,
}

struct SpineColors {
    light: vec4<f32>,
    dark: vec4<f32>,
}

@group(2) @binding(0) var<uniform> colors: SpineColors;
@group(2) @binding(1) var spine_texture: texture_2d<f32>;
@group(2) @binding(2) var spine_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    out.world_position = mesh_functions::mesh2d_position_local_to_world(
        world_from_local, vec4<f32>(v.position, 1.0));
    out.position = mesh_functions::mesh2d_position_world_to_clip(out.world_position);
    out.world_normal = vec3<f32>(0.0, 0.0, 1.0);
    out.uv = v.uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let sample = textureSample(spine_texture, spine_sampler, in.uv);
    // Spine tint-black: ((sample.rgb - 1) * dark.rgb + sample.rgb) * light.rgb
    // Atlases are PMA, so `sample.rgb` is already pre-multiplied by `sample.a`.
    // `colors.light` is also pre-multiplied by `colors.light.a` on the CPU
    // side (the spine runtime packs it this way).
    let rgb = ((sample.rgb - vec3<f32>(1.0)) * colors.dark.rgb + sample.rgb) * colors.light.rgb;
    let a = sample.a * colors.light.a;
    return vec4<f32>(rgb, a);
}
