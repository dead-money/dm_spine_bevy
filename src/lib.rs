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

//! Bevy 0.18 integration for [`dm_spine_runtime`].
//!
//! # Quick start
//!
//! ```no_run
//! use bevy::prelude::*;
//! use dm_spine_bevy::{SpinePlugin, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};
//!
//! fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
//!     commands.spawn(Camera2d);
//!     let skel: Handle<SpineSkeletonAsset> = asset_server.load_with_settings(
//!         "spineboy/export/spineboy-pro.skel",
//!         |s: &mut SpineSkeletonLoaderSettings| {
//!             s.atlas_path = Some("spineboy/export/spineboy-pma.atlas".into());
//!         },
//!     );
//!     commands.spawn(SpineSkeleton::new(skel).with_initial_animation(0, "walk", true));
//! }
//!
//! App::new()
//!     .add_plugins(DefaultPlugins)
//!     .add_plugins(SpinePlugin)
//!     .add_systems(Startup, setup)
//!     .run();
//! ```
//!
//! # How it fits together
//!
//! [`SpinePlugin`] registers asset loaders, the [`SpineMaterial`]
//! `Material2d` plugin, two `Message` types for animation events, and four
//! ordered system stages:
//!
//! 1. [`SpineSet::Init`] — observes assets that finished loading and
//!    builds the per-instance runtime state.
//! 2. [`SpineSet::Tick`] — advances animation time, applies timelines,
//!    re-integrates world transforms, emits the per-frame
//!    `RenderCommand` stream into each skeleton's renderer. Parallel
//!    over skeletons.
//! 3. [`SpineSet::BuildMeshes`] — converts that command stream into
//!    Bevy `Mesh` + `MeshMaterial2d<SpineMaterial>` children.
//! 4. [`SpineSet::Events`] — drains lifecycle + keyframe events into
//!    [`SpineStateEvent`] / [`SpineKeyframeEvent`] writers.
//!
//! User systems can `.before(SpineSet::Tick)` to mutate `time_scale` or
//! queue animations on the same frame they take effect.
//!
//! # Atlas expectations
//!
//! The shipped material assumes premultiplied-alpha textures. Spine
//! exports usually ship `*-pma.atlas` / `*-pma.png` variants alongside
//! straight-alpha pairs; prefer the PMA variant via
//! [`SpineSkeletonLoaderSettings::atlas_path`].
//!
//! [`dm_spine_runtime`]: https://github.com/dead-money/dm_spine_runtime

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
    SpineSkeletonJsonLoader, SpineSkeletonJsonLoaderError, SpineSkeletonJsonLoaderSettings,
    SpineSkeletonLoader, SpineSkeletonLoaderError, SpineSkeletonLoaderSettings,
};
pub use components::{PendingAnimation, SpineSkeleton, SpineSkeletonState};
pub use material::{SpineBlendMode, SpineColors, SpineMaterial, SpineMaterialKey};
pub use mesh::build_spine_meshes;
pub use systems::{
    SpineInitialized, SpineKeyframeEvent, SpineSet, SpineStateEvent, drain_spine_events,
    initialize_spine_skeletons, tick_spine_skeletons,
};

/// Bevy plugin entry point. Register once during `App` setup; spawns
/// [`SpineSkeleton`] components afterward to bring rigs to life.
#[derive(Default)]
pub struct SpinePlugin;

impl Plugin for SpinePlugin {
    fn build(&self, app: &mut App) {
        material::spine_material::register_spine_shader(app);

        app.init_asset::<SpineAtlasAsset>()
            .init_asset::<SpineSkeletonAsset>()
            .init_asset_loader::<SpineAtlasLoader>()
            .init_asset_loader::<SpineSkeletonLoader>()
            .init_asset_loader::<SpineSkeletonJsonLoader>()
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
