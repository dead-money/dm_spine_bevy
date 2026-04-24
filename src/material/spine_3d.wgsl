// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

// 3D (`Material`) sibling of `spine.wgsl`. Same fragment math (tint-black
// over a PMA atlas sample); differs from the 2D shader only in which
// import paths are used for the vertex transformation and in the
// VertexOutput layout. Unlit by design — Spine light/dark channels
// already bake authored tint, so no PBR lighting here.

#import bevy_pbr::{
    mesh_functions as mesh_functions,
    forward_io::VertexOutput,
    view_transformations::position_world_to_clip,
}

struct SpineColors {
    light: vec4<f32>,
    dark: vec4<f32>,
}

// Bevy's 3D `Material` pipeline places the user material bind group at
// `MATERIAL_BIND_GROUP` (3 in Bevy 0.18), with groups 0/1/2 reserved for
// view / mesh / global bindings. The specializer injects the index as a
// shader-def, so `@group(#{MATERIAL_BIND_GROUP})` is resolved at compile
// time. The 2D sibling shader still hardcodes `@group(2)` because the
// `Material2d` pipeline uses a different layout.
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> colors: SpineColors;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var spine_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var spine_sampler: sampler;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vertex(v: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let world_from_local = mesh_functions::get_world_from_local(v.instance_index);
    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local, vec4<f32>(v.position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
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
