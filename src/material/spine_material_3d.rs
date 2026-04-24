// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// Integration of the Spine Runtimes into software or otherwise creating
// derivative works of the Spine Runtimes is permitted under the terms and
// conditions of Section 2 of the Spine Editor License Agreement:
// http://esotericsoftware.com/spine-editor-license
//
// Otherwise, it is permitted to integrate the Spine Runtimes into software
// or otherwise create derivative works of the Spine Runtimes (collectively,
// "Products"), provided that each user of the Products must obtain their own
// Spine Editor license and redistribution of the Products in any form must
// include this license and copyright notice.
//
// THE SPINE RUNTIMES ARE PROVIDED BY ESOTERIC SOFTWARE LLC "AS IS" AND ANY
// EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
// WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL ESOTERIC SOFTWARE LLC BE LIABLE FOR ANY
// DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
// (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES,
// BUSINESS INTERRUPTION, OR LOSS OF USE, DATA, OR PROFITS) HOWEVER CAUSED AND
// ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF
// THE SPINE RUNTIMES, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! 3D (`Material`) flavor of the Spine material. Spawned by
//! [`crate::mesh::build_spine_meshes_3d`] for skeletons tagged with
//! [`crate::components::SpineRender3d`]. The fragment math (tint-black over
//! a PMA atlas sample) is identical to the 2D material; the differences
//! are the trait impl, the WGSL imports, and the vertex-stage plumbing.
//!
//! Deliberately unlit: Spine's light/dark color channels already bake
//! authored lighting, and atlas samples are premultiplied by their own
//! alpha. Layering PBR lighting on top would double-count both.

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use crate::material::shared::{SpineBlendMode, SpineColors, SpineMaterialKey};

/// 3D material emitted by [`crate::mesh::build_spine_meshes_3d`] for each
/// batched `RenderCommand` on entities tagged with
/// [`crate::components::SpineRender3d`].
///
/// Mirrors [`crate::SpineMaterial`] at the bind-group level — same layout,
/// same uniform, same texture slot, same specialization key — so the build
/// system can share color/texture/blend update code across both backends.
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(SpineMaterialKey)]
pub struct SpineMaterial3d {
    #[uniform(0)]
    pub colors: SpineColors,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    pub blend_mode: SpineBlendMode,
}

impl Default for SpineMaterial3d {
    fn default() -> Self {
        Self {
            colors: SpineColors::default(),
            texture: Handle::default(),
            blend_mode: SpineBlendMode::Normal,
        }
    }
}

impl From<&SpineMaterial3d> for SpineMaterialKey {
    fn from(m: &SpineMaterial3d) -> Self {
        Self {
            blend_mode: m.blend_mode,
        }
    }
}

const SHADER_ASSET_PATH: &str = "embedded://dm_spine_bevy/material/spine_3d.wgsl";

impl Material for SpineMaterial3d {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    // Spine geometry is translucent / hand-authored; the shadow and depth
    // prepasses would discard fragments we want rendered and sort poorly
    // against the main transparent pass. Turn both off.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let blend = key.bind_group_data.blend_mode.blend_state();
        if let Some(fragment) = descriptor.fragment.as_mut()
            && let Some(Some(target)) = fragment.targets.first_mut().map(Option::as_mut)
        {
            target.blend = Some(blend);
        }
        // Spine slot Z-offsets (see `Z_OFFSET_PER_COMMAND` in mesh.rs) are
        // smaller than typical depth precision; let the transparent pass
        // sort by camera distance and disable depth writes so slots layer
        // correctly regardless of submission order.
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }
        // Spine meshes are single-sided and can wind either way depending
        // on skeleton flips, skin swaps, and bone chains crossing over
        // themselves. The 2D (`Material2d`) pipeline doesn't cull; match
        // that here so the rig stays visible from both sides as the
        // camera orbits.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Register the 3D WGSL shader with Bevy's embedded asset source. Called by
/// [`crate::SpinePlugin`] during `build`.
pub(crate) fn register_spine_shader_3d(app: &mut App) {
    embedded_asset!(app, "spine_3d.wgsl");
}
