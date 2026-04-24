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

//! First visual example: loads spineboy-pro, plays the `walk` animation on
//! track 0. Points the asset root at the upstream `spine-runtimes/examples`
//! directory so we don't duplicate rig binaries into this crate.
//!
//! Run from `dm_spine_bevy/`:
//!
//! ```bash
//! cargo run --example spineboy_walk
//! ```

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use dm_spine_bevy::{SpinePlugin, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};

fn main() {
    // Bevy resolves AssetPlugin::file_path relative to the executable's
    // directory at runtime, not the project root, so canonicalize first.
    let asset_root = std::path::PathBuf::from("../spine-runtimes/examples")
        .canonicalize()
        .expect("spine-runtimes/examples must exist as a sibling clone");

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: asset_root.to_string_lossy().into_owned(),
                    ..Default::default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugins(SpinePlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((
        Camera2d,
        // Spineboy is tall and positioned with origin at foot. Shift the
        // camera up so the full rig sits in frame.
        Transform::from_xyz(0.0, 200.0, 0.0),
    ));

    // Override the atlas path to the PMA variant — the shader assumes
    // premultiplied-alpha textures, which the stock `spineboy.atlas` is
    // not. `-pma.png` ships pre-multiplied.
    let skel_handle: Handle<SpineSkeletonAsset> = asset_server.load_with_settings(
        "spineboy/export/spineboy-pro.skel",
        |settings: &mut SpineSkeletonLoaderSettings| {
            settings.atlas_path = Some("spineboy/export/spineboy-pma.atlas".to_string());
        },
    );

    commands.spawn((
        SpineSkeleton::new(skel_handle).with_initial_animation(0, "walk", true),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}
