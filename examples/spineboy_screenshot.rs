// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

//! Non-interactive sibling of `spineboy_walk`. Spawns the same scene,
//! waits for the asset to finish loading + a handful of tick-and-render
//! frames, then screenshots the window and exits. Useful for CI / visual
//! comparisons in environments where a human isn't watching.
//!
//! ```bash
//! cargo run --example spineboy_screenshot
//! ls spineboy_screenshot.png
//! ```
//!
//! Environment:
//! - `SPINE_SCREENSHOT` — output PNG path (default `spineboy_screenshot.png`).
//! - `SPINE_SCREENSHOT_FRAMES` — frame number to shoot on (default `60`).

use bevy::asset::AssetPlugin;
use bevy::prelude::*;

use dm_spine_bevy::{SpinePlugin, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};

mod common;

fn main() {
    let asset_root = common::resolve_asset_root(None)
        .expect("spineboy_screenshot: clone https://github.com/EsotericSoftware/spine-runtimes alongside this repo");

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
    .add_systems(Startup, setup);

    // This example exists *for* the screenshot, so always install with
    // a sensible default path if the env var is unset.
    let cfg = common::ScreenshotConfig::from_env("SPINE_SCREENSHOT", "SPINE_SCREENSHOT_FRAMES", 60)
        .unwrap_or(common::ScreenshotConfig {
            path: "spineboy_screenshot.png".to_string(),
            trigger_frame: 60,
        });
    common::install_screenshot_driver(&mut app, cfg);

    app.run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn((Camera2d, Transform::from_xyz(0.0, 200.0, 0.0)));

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
