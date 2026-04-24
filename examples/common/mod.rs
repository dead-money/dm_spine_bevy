// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

//! Helpers shared by the Bevy crate's examples. Cargo can't directly link
//! a single helper crate from `examples/`, so each example does
//! `mod common;` to pull this in.
//!
//! What lives here:
//!
//! - [`resolve_asset_root`] — `--assets` / `$SPINE_EXAMPLES_DIR` /
//!   sibling-clone fallback chain, plus canonicalisation so `AssetPlugin`
//!   doesn't resolve relative to the binary's directory.
//! - [`discover_rigs`] — walks `<root>/<rig>/export/*.skel` and pairs each
//!   skeleton with its preferred (PMA-first) atlas.
//! - [`aggregate_bounds`] — `RenderCommand`-stream AABB used by both
//!   `spine_browser` (live-fit camera) and `spine_stress` (cell sizing).
//! - [`install_screenshot_driver`] — opt-in headless screenshot mode used
//!   for CI / docs.

#![allow(dead_code)] // Each example uses a subset of the helpers.

use std::path::{Path, PathBuf};

use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use dm_spine_bevy::SpineSkeletonState;

// ---- Asset root + rig discovery ------------------------------------------

/// One discoverable rig variant. `skel_relpath` and `atlas_relpath` are
/// relative to the resolved asset root and ready to hand to
/// `AssetServer::load_with_settings`.
#[derive(Clone, Debug)]
pub struct RigEntry {
    pub label: String,
    pub skel_relpath: String,
    pub atlas_relpath: String,
}

/// Resolve the asset root from (in order): an explicit `--assets <path>`
/// from the caller's CLI, the `SPINE_EXAMPLES_DIR` env var, or a
/// sibling clone of the upstream `spine-runtimes/examples` directory.
/// The returned path is always canonicalised; `AssetPlugin::file_path`
/// is interpreted relative to the binary at runtime, so a non-canonical
/// "../spine-runtimes/examples" would resolve from `target/release/examples/`.
///
/// On failure, returns a generic message; the caller is expected to
/// prepend its own program-name prefix.
pub fn resolve_asset_root(cli_assets: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = cli_assets {
        return validate_root(p);
    }
    if let Ok(p) = std::env::var("SPINE_EXAMPLES_DIR") {
        return validate_root(PathBuf::from(p));
    }
    for fallback in ["../spine-runtimes/examples", "./spine-runtimes/examples"] {
        let p = PathBuf::from(fallback);
        if p.is_dir() {
            return validate_root(p);
        }
    }
    Err(MISSING_ASSETS_HELP.to_string())
}

fn validate_root(p: PathBuf) -> Result<PathBuf, String> {
    if !p.is_dir() {
        return Err(format!(
            "asset path {} does not exist or is not a directory",
            p.display()
        ));
    }
    p.canonicalize()
        .map_err(|e| format!("cannot canonicalize asset path {}: {e}", p.display()))
}

const MISSING_ASSETS_HELP: &str = "Could not find Spine example rigs.

Pass an asset root via one of:
  --assets <path>
  SPINE_EXAMPLES_DIR=<path>
  ./spine-runtimes/examples
  ../spine-runtimes/examples

The expected source is the upstream spine-runtimes repo:
  git clone https://github.com/EsotericSoftware/spine-runtimes ../spine-runtimes

(Spine example art is licensed separately from this crate and is not bundled.)";

/// Walk `<root>/<rig>/export/` and yield one [`RigEntry`] per `.skel`
/// file paired with the closest matching atlas (PMA preferred).
pub fn discover_rigs(root: &Path) -> Vec<RigEntry> {
    let mut out = Vec::new();
    let Ok(rig_dirs) = std::fs::read_dir(root) else {
        return out;
    };
    for rig_dir in rig_dirs.flatten() {
        let rig_dir = rig_dir.path();
        if !rig_dir.is_dir() {
            continue;
        }
        let export = rig_dir.join("export");
        if !export.is_dir() {
            continue;
        }
        let rig_name = rig_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let skels: Vec<PathBuf> = std::fs::read_dir(&export)
            .map(|it| {
                it.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "skel"))
                    .collect()
            })
            .unwrap_or_default();
        for skel in skels {
            let stem = skel
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            let Some(atlas) = pick_atlas(&export, &stem) else {
                continue;
            };
            out.push(RigEntry {
                label: format!("{rig_name} / {stem}"),
                skel_relpath: relpath(root, &skel),
                atlas_relpath: relpath(root, &atlas),
            });
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

/// Find the best atlas for a skeleton with the given stem. PMA variant
/// preferred. Tries `<base>-pma.atlas` then `<base>.atlas`, where `<base>`
/// is the stem with trailing `-pro`/`-ess`/`-ios` stripped.
fn pick_atlas(export: &Path, skel_stem: &str) -> Option<PathBuf> {
    let base = ["-pro", "-ess", "-ios"]
        .into_iter()
        .find_map(|sfx| skel_stem.strip_suffix(sfx))
        .unwrap_or(skel_stem);
    for candidate in [format!("{base}-pma.atlas"), format!("{base}.atlas")] {
        let p = export.join(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn relpath(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

// ---- Render-command bounds -----------------------------------------------

/// Aggregate AABB over the skeleton's most recent `RenderCommand` stream
/// (`(min, max)`). Returns `None` for skeletons with no visible geometry
/// — typically because the asset hasn't loaded yet or every command in
/// the frame is empty.
pub fn aggregate_bounds(state: &SpineSkeletonState) -> Option<(Vec2, Vec2)> {
    let mut have_any = false;
    let mut xmin = f32::INFINITY;
    let mut xmax = f32::NEG_INFINITY;
    let mut ymin = f32::INFINITY;
    let mut ymax = f32::NEG_INFINITY;
    for cmd in state.renderer.commands() {
        if let Some((cxmin, cxmax, cymin, cymax)) = cmd.position_bounds() {
            xmin = xmin.min(cxmin);
            xmax = xmax.max(cxmax);
            ymin = ymin.min(cymin);
            ymax = ymax.max(cymax);
            have_any = true;
        }
    }
    have_any.then_some((Vec2::new(xmin, ymin), Vec2::new(xmax, ymax)))
}

// ---- Headless screenshot driver ------------------------------------------

#[derive(Resource)]
struct ScreenshotState {
    path: String,
    trigger: u32,
    current: u32,
    taken: bool,
}

/// Resolved configuration for [`install_screenshot_driver`]. Build via
/// [`ScreenshotConfig::from_env`] for env-var-driven examples, or
/// directly when the example wants to default a value.
pub struct ScreenshotConfig {
    pub path: String,
    pub trigger_frame: u32,
}

impl ScreenshotConfig {
    /// Read `(env_path, env_frames)` and return `Some(config)` if the
    /// path env var is set; otherwise `None`. `env_frames` is parsed as
    /// `u32`, falling back to `default_frames` on missing / unparseable.
    pub fn from_env(env_path: &str, env_frames: &str, default_frames: u32) -> Option<Self> {
        let path = std::env::var(env_path).ok()?;
        let trigger_frame = std::env::var(env_frames)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_frames);
        Some(Self {
            path,
            trigger_frame,
        })
    }
}

/// Schedule a one-shot window screenshot at frame `cfg.trigger_frame`
/// to `cfg.path`, then exit the app after a short grace period for the
/// async PNG write to finish.
pub fn install_screenshot_driver(app: &mut App, cfg: ScreenshotConfig) {
    app.insert_resource(ScreenshotState {
        path: cfg.path,
        trigger: cfg.trigger_frame,
        current: 0,
        taken: false,
    });
    app.add_systems(Update, screenshot_driver_system);
}

fn screenshot_driver_system(
    mut commands: Commands,
    mut state: ResMut<ScreenshotState>,
    mut exit: MessageWriter<AppExit>,
) {
    state.current += 1;
    const GRACE: u32 = 30;
    if state.taken {
        if state.current >= state.trigger + GRACE {
            exit.write(AppExit::Success);
        }
        return;
    }
    if state.current >= state.trigger {
        let path = state.path.clone();
        info!("dm_spine_bevy example: screenshot to {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        state.taken = true;
    }
}
