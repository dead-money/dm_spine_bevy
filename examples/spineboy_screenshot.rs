// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

//! Non-interactive sibling of `spineboy_walk`. Spawns the same scene, waits
//! for the asset to finish loading + a handful of tick-and-render frames,
//! then screenshots the window and exits. Useful for driving CI / visual
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
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use dm_spine_bevy::{SpinePlugin, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};

#[derive(Resource)]
struct ScreenshotConfig {
    path: String,
    trigger_frame: u32,
    current_frame: u32,
    taken: bool,
}

fn main() {
    let path = std::env::var("SPINE_SCREENSHOT")
        .unwrap_or_else(|_| "spineboy_screenshot.png".to_string());
    let trigger_frame = std::env::var("SPINE_SCREENSHOT_FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: "../spine-runtimes/examples".to_string(),
                    ..Default::default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugins(SpinePlugin)
        .insert_resource(ScreenshotConfig {
            path,
            trigger_frame,
            current_frame: 0,
            taken: false,
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (tick_frame, capture_and_exit).chain())
        .run();
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

fn tick_frame(mut cfg: ResMut<ScreenshotConfig>) {
    cfg.current_frame += 1;
}

fn capture_and_exit(
    mut commands: Commands,
    mut cfg: ResMut<ScreenshotConfig>,
    mut exit: MessageWriter<AppExit>,
) {
    // The screenshot crosses from main world to render world and back
    // asynchronously, so we can't exit on the same frame we trigger. Give
    // ourselves a generous grace period (30 frames ≈ 0.5s at 60Hz) before
    // shutting down.
    const EXIT_GRACE_FRAMES: u32 = 30;

    if cfg.taken {
        if cfg.current_frame >= cfg.trigger_frame + EXIT_GRACE_FRAMES {
            exit.write(AppExit::Success);
        }
        return;
    }
    if cfg.current_frame >= cfg.trigger_frame {
        let path = cfg.path.clone();
        info!("dm_spine_bevy: capturing screenshot to {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        cfg.taken = true;
    }
}
