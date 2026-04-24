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

//! 3D sibling of `spineboy_walk`: loads spineboy-pro, plays the `walk`
//! animation, and renders it through the 3D `Material` pipeline instead
//! of the 2D `Material2d` pipeline. A Camera3d with perspective
//! projection orbits a checkered ground plane so the rig clearly sits in
//! a 3D scene rather than a flat sprite canvas.
//!
//! Run from `dm_spine_bevy/`:
//!
//! ```bash
//! cargo run --example spineboy_walk_3d
//! ```
//!
//! Controls: none — the camera orbits on its own. Esc to quit.
//!
//! For CI / visual-regression use, set `SPINE_SCREENSHOT_3D` to a path to
//! drop a PNG after `SPINE_SCREENSHOT_3D_FRAMES` (default 90) frames and
//! exit.

use std::f32::consts::TAU;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use dm_spine_bevy::{
    SpinePlugin, SpineRender3d, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings,
};

mod common;

// Spineboy is authored ~400 units tall with origin at the feet. Scale
// the whole skeleton down to ~2 units tall in the 3D scene — roughly
// human-height next to the 20×20 ground plane, large enough to read
// clearly against the perspective camera.
const SKELETON_SCALE: f32 = 0.005;
/// How far the orbiting camera sits from the skeleton. Tuned so the rig
/// fills a comfortable fraction of the frame without clipping.
const CAMERA_RADIUS: f32 = 6.0;
/// Camera height above the ground. Puts the eye a bit above the top of
/// the rig so the ground plane is visible and the scene reads as 3D.
const CAMERA_HEIGHT: f32 = 3.0;
/// Camera look-at target — midway up the skeleton so the rig is framed.
const CAMERA_TARGET: Vec3 = Vec3::new(0.0, 1.2, 0.0);

/// Tag the camera rig we orbit each frame.
#[derive(Component)]
struct OrbitCamera;

fn main() {
    let asset_root = common::resolve_asset_root(None)
        .expect("spineboy_walk_3d: clone https://github.com/EsotericSoftware/spine-runtimes alongside this repo");

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_root.to_string_lossy().into_owned(),
                ..Default::default()
            })
            .set(ImagePlugin::default_nearest()),
    )
    .add_plugins(SpinePlugin)
    .add_systems(Startup, setup)
    .add_systems(Update, orbit_camera);

    if let Some(cfg) =
        common::ScreenshotConfig::from_env("SPINE_SCREENSHOT_3D", "SPINE_SCREENSHOT_3D_FRAMES", 90)
    {
        common::install_screenshot_driver(&mut app, cfg);
    }

    app.run();
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Camera starts slightly off-axis so the first frame already reads
    // as 3D (not a flat elevation). `orbit_camera` sweeps it around the
    // Y axis at a slow rate each frame.
    let start = Vec3::new(CAMERA_RADIUS * 0.5, CAMERA_HEIGHT, CAMERA_RADIUS * 0.87);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(start).looking_at(CAMERA_TARGET, Vec3::Y),
        OrbitCamera,
    ));

    // Single directional light. The SpineMaterial3d is unlit — Spine
    // color channels already carry authored lighting — but a real 3D
    // scene has lights, and the light plus the ground shading sell the
    // "this is a 3D scene" cue the example exists for.
    commands.spawn((
        DirectionalLight {
            illuminance: 6_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Ground plane + subtle ambient light so the scene reads as 3D.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(20.0, 20.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.25, 0.27, 0.3),
            perceptual_roughness: 0.9,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
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

    // The Spine runtime emits positions in the skeleton's local XY
    // plane. No extra rotation needed — the skeleton's +Y already points
    // up the way a 3D scene expects. Scale alone maps authored pixels to
    // scene units.
    commands.spawn((
        SpineSkeleton::new(skel_handle).with_initial_animation(0, "walk", true),
        SpineRender3d,
        Transform::from_xyz(0.0, 0.0, 0.0).with_scale(Vec3::splat(SKELETON_SCALE)),
    ));
}

fn orbit_camera(time: Res<Time>, mut q: Query<&mut Transform, With<OrbitCamera>>) {
    let Ok(mut tf) = q.single_mut() else {
        return;
    };
    // Slow ~18-second full revolution. Starting offset matches the
    // initial transform so the first frame of orbit continues smoothly.
    let angle = (time.elapsed_secs() * 0.35 + TAU / 6.0) % TAU;
    tf.translation = Vec3::new(
        angle.sin() * CAMERA_RADIUS,
        CAMERA_HEIGHT,
        angle.cos() * CAMERA_RADIUS,
    );
    tf.look_at(CAMERA_TARGET, Vec3::Y);
}
