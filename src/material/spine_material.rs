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

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

use dm_spine_runtime::data::BlendMode;

/// Per-material uniform carrying the slot's light + dark tint as
/// premultiplied RGBA. Matches the `SpineColors` struct in `spine.wgsl`
/// at `@group(2) @binding(0)`.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct SpineColors {
    /// Light tint, premultiplied-alpha. The runtime produces this
    /// premultiplied already (color packing in `RenderCommand::colors`).
    pub light: Vec4,
    /// Tint-black (`darkColor` in spine-cpp). Alpha byte is always `0xff`
    /// on the CPU side; the shader never reads `dark.a`.
    pub dark: Vec4,
}

/// 2D material emitted by [`crate::mesh`] for each batched `RenderCommand`.
/// Pairs an atlas-page texture with the slot's premultiplied colors and a
/// blend mode.
///
/// `blend_mode` is promoted into [`SpineMaterialKey`] so the
/// `Material2d` pipeline specializer caches one pipeline per mode (four
/// permutations total).
#[derive(Asset, AsBindGroup, TypePath, Clone, Debug)]
#[bind_group_data(SpineMaterialKey)]
pub struct SpineMaterial {
    /// Per-slot light + dark tint, shared across every vertex of one
    /// command (the runtime's adjacency batcher only merges commands
    /// with identical colors).
    #[uniform(0)]
    pub colors: SpineColors,
    /// Atlas page the command samples from. Resolved by the mesh-build
    /// system from `SpineAtlasAsset::pages` keyed by `RenderCommand::texture`.
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    /// Spine blend mode. Not a bind-group field — copied into
    /// [`SpineMaterialKey`] for pipeline specialization.
    pub blend_mode: SpineBlendMode,
}

impl Default for SpineMaterial {
    fn default() -> Self {
        Self {
            colors: SpineColors::default(),
            texture: Handle::default(),
            blend_mode: SpineBlendMode::Normal,
        }
    }
}

/// Bevy-side mirror of `dm_spine_runtime::data::BlendMode`. Lives in the
/// plugin crate so the runtime crate doesn't take a `bevy` dep.
#[repr(u8)]
#[derive(Copy, Clone, Hash, Eq, PartialEq, Default, Debug)]
pub enum SpineBlendMode {
    #[default]
    Normal,
    Additive,
    Multiply,
    Screen,
}

impl From<BlendMode> for SpineBlendMode {
    fn from(mode: BlendMode) -> Self {
        match mode {
            BlendMode::Normal => Self::Normal,
            BlendMode::Additive => Self::Additive,
            BlendMode::Multiply => Self::Multiply,
            BlendMode::Screen => Self::Screen,
        }
    }
}

impl SpineBlendMode {
    /// wgpu blend-state for PMA atlases, by Spine blend mode. Ported from
    /// `spine-cpp/src/spine/SkeletonRenderer.cpp`.
    ///
    /// All four modes use `BlendOperation::Add` for both color and alpha.
    #[must_use]
    pub fn blend_state(self) -> BlendState {
        let color = match self {
            Self::Normal => BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            Self::Additive => BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::One,
                operation: BlendOperation::Add,
            },
            Self::Multiply => BlendComponent {
                src_factor: BlendFactor::Dst,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            Self::Screen => BlendComponent {
                src_factor: BlendFactor::One,
                dst_factor: BlendFactor::OneMinusSrc,
                operation: BlendOperation::Add,
            },
        };
        let alpha = match self {
            Self::Normal | Self::Additive => color,
            Self::Multiply => BlendComponent {
                src_factor: BlendFactor::OneMinusSrcAlpha,
                dst_factor: BlendFactor::OneMinusSrcAlpha,
                operation: BlendOperation::Add,
            },
            Self::Screen => BlendComponent {
                src_factor: BlendFactor::OneMinusSrc,
                dst_factor: BlendFactor::OneMinusSrc,
                operation: BlendOperation::Add,
            },
        };
        BlendState { color, alpha }
    }
}

/// Specialization key: one pipeline per blend mode.
#[repr(C)]
#[derive(Copy, Clone, Hash, Eq, PartialEq, Debug)]
pub struct SpineMaterialKey {
    pub blend_mode: SpineBlendMode,
}

impl From<&SpineMaterial> for SpineMaterialKey {
    fn from(m: &SpineMaterial) -> Self {
        Self {
            blend_mode: m.blend_mode,
        }
    }
}

const SHADER_ASSET_PATH: &str = "embedded://dm_spine_bevy/material/spine.wgsl";

impl Material2d for SpineMaterial {
    fn vertex_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn fragment_shader() -> ShaderRef {
        SHADER_ASSET_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let blend = key.bind_group_data.blend_mode.blend_state();
        if let Some(fragment) = descriptor.fragment.as_mut()
            && let Some(Some(target)) = fragment.targets.first_mut().map(Option::as_mut)
        {
            target.blend = Some(blend);
        }
        Ok(())
    }
}

/// Register the WGSL shader with Bevy's embedded asset source. Called by
/// [`crate::SpinePlugin`] during `build`. Downstream, `Material2d::fragment_shader`
/// returns the `embedded://...` path and the asset server resolves it
/// lazily — so this registration doesn't require any render plugins to be
/// installed yet.
pub(crate) fn register_spine_shader(app: &mut App) {
    embedded_asset!(app, "spine.wgsl");
}
