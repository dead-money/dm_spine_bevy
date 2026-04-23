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

//! Bevy 0.18 integration for `dm_spine_runtime`. See `PHASE_7_PLAN.md` in the
//! runtime crate for the full architecture.
//!
//! Add [`SpinePlugin`] to your `App` to register asset loaders for `.atlas`
//! and `.skel` files. Later sub-phases will add components, tick systems, and
//! a PMA-aware `Material2d`.

use bevy::asset::AssetApp;
use bevy::prelude::*;
use bevy::sprite_render::Material2dPlugin;

pub mod asset;
pub mod components;
pub mod material;
pub mod mesh;
pub mod systems;

pub use asset::{
    SpineAtlasAsset, SpineAtlasLoader, SpineAtlasLoaderError, SpineSkeletonAsset,
    SpineSkeletonLoader, SpineSkeletonLoaderError, SpineSkeletonLoaderSettings,
};
pub use components::{PendingAnimation, SpineSkeleton, SpineSkeletonState};
pub use material::{SpineBlendMode, SpineColors, SpineMaterial, SpineMaterialKey};
pub use mesh::build_spine_meshes;
pub use systems::{
    SpineKeyframeEvent, SpineSet, SpineStateEvent, drain_spine_events,
    initialize_spine_skeletons, tick_spine_skeletons,
};

#[derive(Default)]
pub struct SpinePlugin;

impl Plugin for SpinePlugin {
    fn build(&self, app: &mut App) {
        material::spine_material::register_spine_shader(app);

        app.init_asset::<SpineAtlasAsset>()
            .init_asset::<SpineSkeletonAsset>()
            .init_asset_loader::<SpineAtlasLoader>()
            .init_asset_loader::<SpineSkeletonLoader>()
            .add_plugins(Material2dPlugin::<SpineMaterial>::default())
            .add_message::<SpineStateEvent>()
            .add_message::<SpineKeyframeEvent>()
            .configure_sets(
                Update,
                (
                    SpineSet::Init,
                    SpineSet::Tick,
                    SpineSet::BuildMeshes,
                    SpineSet::Events,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    initialize_spine_skeletons.in_set(SpineSet::Init),
                    tick_spine_skeletons.in_set(SpineSet::Tick),
                    build_spine_meshes.in_set(SpineSet::BuildMeshes),
                    drain_spine_events.in_set(SpineSet::Events),
                ),
            );
    }
}
