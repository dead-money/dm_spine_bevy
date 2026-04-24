// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

//! Interactive browser for the example rigs that ship in upstream
//! `spine-runtimes/examples/`. Cycle through rigs / animations / skins with
//! the keyboard; the camera live-fits to the visible AABB of the current
//! pose.
//!
//! ## Controls
//!
//! - **Space** / **Shift+Space** — next / previous rig
//! - **N** / **Shift+N** — next / previous animation in the current rig
//! - **S** / **Shift+S** — next / previous skin in the current rig
//! - **R** — reset the current animation to time zero
//! - **+** / **-** — speed up / slow down playback (clamped 0.1× – 4×)
//! - **Esc** — quit
//!
//! ## Asset root
//!
//! The example does not bundle the Spine sample art — that lives in the
//! upstream [`EsotericSoftware/spine-runtimes`] repo and is licensed
//! separately. The browser looks for the `examples/` directory in this
//! order:
//!
//! 1. `--assets <path>` on the command line
//! 2. `SPINE_EXAMPLES_DIR` environment variable
//! 3. `../spine-runtimes/examples` (sibling clone, what other examples assume)
//! 4. `./spine-runtimes/examples` (cwd)
//!
//! If none exist, the example prints a clone command and exits cleanly.
//!
//! [`EsotericSoftware/spine-runtimes`]: https://github.com/EsotericSoftware/spine-runtimes

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;

use dm_spine_bevy::{SpinePlugin, SpineSet, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};

/// One discoverable rig variant.
#[derive(Clone, Debug)]
struct RigEntry {
    /// Display label, e.g. `"spineboy / spineboy-pro"`.
    label: String,
    /// Path inside the asset root to the `.skel` file, e.g.
    /// `"spineboy/export/spineboy-pro.skel"`.
    skel_relpath: String,
    /// Path inside the asset root to the `.atlas` file. PMA variant
    /// preferred; falls back to non-PMA if no `-pma.atlas` is present.
    atlas_relpath: String,
}

#[derive(Resource)]
struct Browser {
    rigs: Vec<RigEntry>,
    current_rig: usize,
    /// Animation names for the currently-loaded rig. Refreshed once the
    /// asset finishes loading.
    animations: Vec<String>,
    current_animation: usize,
    /// Skin names for the currently-loaded rig. Always has at least one
    /// entry (`"default"`) once populated.
    skins: Vec<String>,
    current_skin: usize,
    time_scale: f32,
    /// Entity holding the active `SpineSkeleton`. Despawned and respawned
    /// when the user cycles rigs.
    skeleton_entity: Option<Entity>,
    /// Current camera params (smoothed every frame toward `target_view`).
    current_view: View,
    /// Most recent aggregate AABB of the visible pose, expanded by a
    /// margin. Updated each frame.
    target_view: View,
    /// Set when the active rig changes; used by the metadata-refresh
    /// system to re-pull animations / skins after the new asset loads.
    metadata_dirty: bool,
    /// CLI-supplied animation name to start playing on first metadata
    /// refresh (overrides the default of `animations[0]`). Cleared once
    /// applied so subsequent rig changes use the default.
    initial_anim: Option<String>,
    /// CLI-supplied skin name to apply on first metadata refresh.
    initial_skin: Option<String>,
}

#[derive(Clone, Copy, Debug)]
struct View {
    center: Vec2,
    /// Vertical world-units visible at the desired margin. Camera projection
    /// scales to fit this; horizontal extent follows aspect ratio.
    height: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            center: Vec2::new(0.0, 200.0),
            height: 800.0,
        }
    }
}

#[derive(Component)]
struct HudText;

const VIEW_MARGIN: f32 = 1.15;
/// Per-frame lerp factor for camera smoothing. 0.15 = ~10 frame settle.
const VIEW_LERP: f32 = 0.15;
const MIN_VIEW_HEIGHT: f32 = 100.0;

/// Parsed command-line arguments. Long forms only — `--key value` or `--key=value`.
#[derive(Default, Debug, Clone)]
struct Cli {
    /// Asset root containing `<rig>/export/...`.
    assets: Option<PathBuf>,
    /// Substring filter to pre-select a rig at startup. Matches the
    /// label, e.g. `--rig spineboy-pro` or `--rig celestial`.
    rig: Option<String>,
    /// Animation name to start playing once the rig loads. Falls back
    /// to the first animation if absent or unknown.
    anim: Option<String>,
    /// Initial skin name (overrides `default`).
    skin: Option<String>,
    /// Window dimensions in physical pixels. Useful when recording so
    /// the captured frames have a known size.
    window_width: Option<u32>,
    window_height: Option<u32>,
}

fn parse_cli() -> Cli {
    let mut cli = Cli::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let (key, value) = if let Some((k, v)) = a.split_once('=') {
            (k.to_string(), Some(v.to_string()))
        } else {
            (a, args.next())
        };
        match key.as_str() {
            "--assets" => cli.assets = value.map(PathBuf::from),
            "--rig" => cli.rig = value,
            "--anim" => cli.anim = value,
            "--skin" => cli.skin = value,
            "--width" => cli.window_width = value.and_then(|v| v.parse().ok()),
            "--height" => cli.window_height = value.and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    cli
}

fn main() -> ExitCode {
    let cli = parse_cli();

    let asset_root = match resolve_asset_root(cli.assets.clone()) {
        Ok(p) => p,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(1);
        }
    };

    let rigs = discover_rigs(&asset_root);
    if rigs.is_empty() {
        eprintln!(
            "spine_browser: no rigs found under {}.\n\
             Expected structure: <root>/<rig>/export/<rig>.skel + <rig>.atlas",
            asset_root.display()
        );
        return ExitCode::from(1);
    }
    eprintln!(
        "spine_browser: discovered {} rig(s) under {}",
        rigs.len(),
        asset_root.display()
    );

    // Apply --rig substring jump.
    let initial_rig = cli
        .rig
        .as_ref()
        .and_then(|needle| rigs.iter().position(|r| r.label.contains(needle.as_str())))
        .unwrap_or(0);
    if let Some(needle) = &cli.rig
        && initial_rig == 0
        && !rigs[0].label.contains(needle.as_str())
    {
        eprintln!(
            "spine_browser: --rig {needle:?} matched no rig; starting at {}",
            rigs[0].label
        );
    }

    // Build the window plugin with optional fixed resolution.
    let mut window_plugin = WindowPlugin::default();
    if let (Some(w), Some(h)) = (cli.window_width, cli.window_height) {
        window_plugin.primary_window = Some(Window {
            resolution: WindowResolution::new(w, h),
            title: "spine_browser".into(),
            resizable: false,
            ..Default::default()
        });
    }

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_root.to_string_lossy().into_owned(),
                ..Default::default()
            })
            .set(ImagePlugin::default_nearest())
            .set(window_plugin),
    )
    .add_plugins(SpinePlugin)
    .insert_resource(Browser {
        rigs,
        current_rig: initial_rig,
        animations: Vec::new(),
        current_animation: 0,
        skins: Vec::new(),
        current_skin: 0,
        time_scale: 1.0,
        skeleton_entity: None,
        current_view: View::default(),
        target_view: View::default(),
        metadata_dirty: true,
        initial_anim: cli.anim,
        initial_skin: cli.skin,
    })
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            handle_input,
            refresh_metadata.after(SpineSet::Init),
            live_fit_camera.after(SpineSet::Tick),
            update_hud,
        ),
    );

    // Optional single-shot screenshot.
    if let Ok(path) = std::env::var("SPINE_BROWSER_SCREENSHOT") {
        let frames: u32 = std::env::var("SPINE_BROWSER_SCREENSHOT_FRAMES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        app.insert_resource(ScreenshotConfig {
            path,
            trigger_frame: frames,
            current_frame: 0,
            taken: false,
        });
        app.add_systems(Update, screenshot_driver);
    }

    // Optional frame-sequence recording for assembling animated GIFs.
    if let Ok(dir) = std::env::var("SPINE_BROWSER_RECORD_DIR") {
        let dir = PathBuf::from(dir);
        let frames: u32 = std::env::var("SPINE_BROWSER_RECORD_FRAMES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(90);
        let warmup: u32 = std::env::var("SPINE_BROWSER_RECORD_WARMUP")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);
        std::fs::create_dir_all(&dir).expect("create record dir");
        app.insert_resource(RecordConfig {
            out_dir: dir,
            target_frames: frames,
            warmup_frames: warmup,
            current_frame: 0,
            captured: 0,
            done_frame: None,
        });
        app.add_systems(Update, record_driver);
    }

    app.run();

    ExitCode::SUCCESS
}

#[derive(Resource)]
struct ScreenshotConfig {
    path: String,
    trigger_frame: u32,
    current_frame: u32,
    taken: bool,
}

fn screenshot_driver(
    mut commands: Commands,
    mut cfg: ResMut<ScreenshotConfig>,
    mut exit: MessageWriter<AppExit>,
) {
    cfg.current_frame += 1;
    const EXIT_GRACE_FRAMES: u32 = 30;
    if cfg.taken {
        if cfg.current_frame >= cfg.trigger_frame + EXIT_GRACE_FRAMES {
            exit.write(AppExit::Success);
        }
        return;
    }
    if cfg.current_frame >= cfg.trigger_frame {
        let path = cfg.path.clone();
        info!("spine_browser: capturing screenshot to {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        cfg.taken = true;
    }
}

/// Frame-sequence record driver: writes `frame_NNNN.png` into `out_dir`
/// for `target_frames` frames after a `warmup_frames` settle period (lets
/// the camera live-fit converge before recording starts). Exits after a
/// grace period so async PNG writes flush.
#[derive(Resource)]
struct RecordConfig {
    out_dir: PathBuf,
    target_frames: u32,
    warmup_frames: u32,
    current_frame: u32,
    captured: u32,
    done_frame: Option<u32>,
}

fn record_driver(
    mut commands: Commands,
    mut cfg: ResMut<RecordConfig>,
    mut exit: MessageWriter<AppExit>,
) {
    cfg.current_frame += 1;
    const EXIT_GRACE_FRAMES: u32 = 60;

    if let Some(done) = cfg.done_frame {
        if cfg.current_frame >= done + EXIT_GRACE_FRAMES {
            exit.write(AppExit::Success);
        }
        return;
    }

    if cfg.current_frame <= cfg.warmup_frames {
        return;
    }

    let path = cfg
        .out_dir
        .join(format!("frame_{:04}.png", cfg.captured));
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    cfg.captured += 1;

    if cfg.captured >= cfg.target_frames {
        info!(
            "spine_browser: captured {} frames into {}",
            cfg.captured,
            cfg.out_dir.display()
        );
        cfg.done_frame = Some(cfg.current_frame);
    }
}

// ---- Asset-root + rig discovery ------------------------------------------

fn resolve_asset_root(cli_assets: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(p) = cli_assets {
        return validate_root(p);
    }
    if let Ok(p) = std::env::var("SPINE_EXAMPLES_DIR") {
        return validate_root(PathBuf::from(p));
    }
    for fallback in ["../spine-runtimes/examples", "./spine-runtimes/examples"] {
        let p = PathBuf::from(fallback);
        if p.is_dir() {
            return Ok(p);
        }
    }
    Err(MISSING_ASSETS_HELP.to_string())
}

fn validate_root(p: PathBuf) -> Result<PathBuf, String> {
    if p.is_dir() {
        Ok(p)
    } else {
        Err(format!(
            "spine_browser: --assets path {} does not exist or is not a directory",
            p.display()
        ))
    }
}

const MISSING_ASSETS_HELP: &str = "spine_browser: could not find Spine example rigs.

Pass an asset root via one of:
  --assets <path>
  SPINE_EXAMPLES_DIR=<path>
  ./spine-runtimes/examples
  ../spine-runtimes/examples

The expected source is the upstream spine-runtimes repo:
  git clone https://github.com/EsotericSoftware/spine-runtimes ../spine-runtimes

(Spine example art is licensed separately from this crate and is not
bundled.)";

/// Walk `<root>/<rig>/export/` and emit one [`RigEntry`] per `.skel` file,
/// pairing it with the closest matching atlas (PMA preferred).
fn discover_rigs(root: &Path) -> Vec<RigEntry> {
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
/// preferred. Tries (in order): `<base>-pma.atlas`, `<base>.atlas`, where
/// `<base>` is the stem with trailing `-pro`/`-ess`/`-ios` stripped.
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

// ---- Setup + spawn -------------------------------------------------------

fn setup(mut commands: Commands, mut browser: ResMut<Browser>, asset_server: Res<AssetServer>) {
    commands.spawn((
        Camera2d,
        Transform::from_translation(browser.current_view.center.extend(0.0)),
    ));
    commands.spawn((
        HudText,
        Text::new("loading…"),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: px(8),
            left: px(12),
            ..default()
        },
    ));
    spawn_current_rig(&mut commands, &mut browser, &asset_server);
}

fn spawn_current_rig(
    commands: &mut Commands,
    browser: &mut Browser,
    asset_server: &AssetServer,
) {
    if let Some(prev) = browser.skeleton_entity.take() {
        commands.entity(prev).despawn();
    }
    let rig = browser.rigs[browser.current_rig].clone();
    let atlas = rig.atlas_relpath.clone();
    let handle: Handle<SpineSkeletonAsset> = asset_server.load_with_settings(
        rig.skel_relpath.clone(),
        move |s: &mut SpineSkeletonLoaderSettings| {
            s.atlas_path = Some(atlas.clone());
        },
    );
    let mut sk = SpineSkeleton::new(handle);
    sk.time_scale = browser.time_scale;
    let entity = commands.spawn(sk).id();
    browser.skeleton_entity = Some(entity);
    browser.metadata_dirty = true;
    browser.animations.clear();
    browser.skins.clear();
}

// ---- Input ---------------------------------------------------------------

fn handle_input(
    mut commands: Commands,
    mut browser: ResMut<Browser>,
    keys: Res<ButtonInput<KeyCode>>,
    mut exit: MessageWriter<AppExit>,
    asset_server: Res<AssetServer>,
    mut sk_query: Query<&mut SpineSkeleton>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
        return;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    // Rig cycling.
    if keys.just_pressed(KeyCode::Space) {
        let n = browser.rigs.len();
        browser.current_rig = if shift {
            (browser.current_rig + n - 1) % n
        } else {
            (browser.current_rig + 1) % n
        };
        browser.current_animation = 0;
        browser.current_skin = 0;
        spawn_current_rig(&mut commands, &mut browser, &asset_server);
        return;
    }

    // Animation cycling — needs the skeleton to be initialised.
    if keys.just_pressed(KeyCode::KeyN) && !browser.animations.is_empty() {
        let n = browser.animations.len();
        browser.current_animation = if shift {
            (browser.current_animation + n - 1) % n
        } else {
            (browser.current_animation + 1) % n
        };
        if let Some(entity) = browser.skeleton_entity
            && let Ok(mut sk) = sk_query.get_mut(entity)
        {
            let anim = browser.animations[browser.current_animation].clone();
            sk.play(0, anim, true);
        }
        return;
    }

    // Skin cycling.
    if keys.just_pressed(KeyCode::KeyS) && !browser.skins.is_empty() {
        let n = browser.skins.len();
        browser.current_skin = if shift {
            (browser.current_skin + n - 1) % n
        } else {
            (browser.current_skin + 1) % n
        };
        if let Some(entity) = browser.skeleton_entity
            && let Ok(mut sk) = sk_query.get_mut(entity)
        {
            let skin = browser.skins[browser.current_skin].clone();
            if let Err(err) = sk.set_skin(skin.clone()) {
                warn!("spine_browser: set_skin({skin:?}) failed: {err:?}");
            } else if !browser.animations.is_empty() {
                // Re-trigger the current animation so the new skin's
                // attachments end up in their animated state, not setup.
                let anim = browser.animations[browser.current_animation].clone();
                sk.play(0, anim, true);
            }
        }
        return;
    }

    // Reset current animation.
    if keys.just_pressed(KeyCode::KeyR)
        && let Some(entity) = browser.skeleton_entity
        && let Ok(mut sk) = sk_query.get_mut(entity)
        && !browser.animations.is_empty()
    {
        let anim = browser.animations[browser.current_animation].clone();
        sk.play(0, anim, true);
        return;
    }

    // Time-scale tweaks. NumpadAdd / NumpadSubtract cover keyboards without
    // bare +/- keys; the un-shifted Equal key reads as '=' and is treated
    // as +. Minus is the dash key.
    let inc = keys.just_pressed(KeyCode::NumpadAdd) || keys.just_pressed(KeyCode::Equal);
    let dec = keys.just_pressed(KeyCode::NumpadSubtract) || keys.just_pressed(KeyCode::Minus);
    if inc {
        browser.time_scale = (browser.time_scale * 1.25).clamp(0.1, 4.0);
        push_time_scale(&browser, &mut sk_query);
    } else if dec {
        browser.time_scale = (browser.time_scale / 1.25).clamp(0.1, 4.0);
        push_time_scale(&browser, &mut sk_query);
    }
}

fn push_time_scale(browser: &Browser, sk_query: &mut Query<&mut SpineSkeleton>) {
    if let Some(entity) = browser.skeleton_entity
        && let Ok(mut sk) = sk_query.get_mut(entity)
    {
        sk.time_scale = browser.time_scale;
    }
}

// ---- Metadata refresh ----------------------------------------------------

fn refresh_metadata(mut browser: ResMut<Browser>, mut sk_query: Query<&mut SpineSkeleton>) {
    if !browser.metadata_dirty {
        return;
    }
    let Some(entity) = browser.skeleton_entity else {
        return;
    };
    let Ok(mut sk) = sk_query.get_mut(entity) else {
        return;
    };
    let (Some(anims), Some(skins)) = (sk.available_animations(), sk.available_skins()) else {
        return;
    };

    let anims: Vec<String> = anims.iter().map(|s| (*s).to_string()).collect();
    let skins: Vec<String> = skins.iter().map(|s| (*s).to_string()).collect();

    // Honor `--anim` / `--skin` on first metadata refresh; fall back to
    // index-0 if the requested name doesn't exist on the loaded rig.
    if let Some(want) = browser.initial_anim.take()
        && let Some(idx) = anims.iter().position(|n| n == &want)
    {
        browser.current_animation = idx;
    }
    if let Some(want) = browser.initial_skin.take()
        && let Some(idx) = skins.iter().position(|n| n == &want)
    {
        browser.current_skin = idx;
    }

    browser.current_animation = browser.current_animation.min(anims.len().saturating_sub(1));
    browser.current_skin = browser.current_skin.min(skins.len().saturating_sub(1));
    browser.metadata_dirty = false;

    if browser.current_skin > 0
        && let Some(skin) = skins.get(browser.current_skin)
        && let Err(err) = sk.set_skin(skin.clone())
    {
        warn!("spine_browser: initial set_skin({skin:?}) failed: {err:?}");
    }

    // Auto-play the chosen animation so the rig is moving when it appears.
    if let Some(anim) = anims.get(browser.current_animation).cloned() {
        sk.play(0, anim, true);
    }

    browser.animations = anims;
    browser.skins = skins;
}

// ---- Live-fit camera -----------------------------------------------------

fn live_fit_camera(
    time: Res<Time>,
    mut browser: ResMut<Browser>,
    sk_query: Query<&SpineSkeleton>,
    windows: Query<&Window>,
    mut camera_q: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
) {
    // Update target from this frame's render commands, if available.
    if let Some(entity) = browser.skeleton_entity
        && let Ok(sk) = sk_query.get(entity)
        && let Some(state) = &sk.state
    {
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
        if have_any {
            let center = Vec2::new((xmin + xmax) * 0.5, (ymin + ymax) * 0.5);
            let height = ((ymax - ymin) * VIEW_MARGIN).max(MIN_VIEW_HEIGHT);
            // Account for window aspect: if the rig is wider than tall,
            // bumping height by the inverse aspect keeps it horizontally
            // in-frame too.
            let aspect = windows
                .iter()
                .next()
                .map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
            let needed_for_width = (xmax - xmin) * VIEW_MARGIN / aspect;
            browser.target_view = View {
                center,
                height: height.max(needed_for_width).max(MIN_VIEW_HEIGHT),
            };
        }
    }

    // Smooth current toward target.
    let dt = time.delta_secs().clamp(0.0, 1.0 / 30.0);
    let alpha = 1.0 - (1.0 - VIEW_LERP).powf(dt * 60.0);
    browser.current_view.center = browser
        .current_view
        .center
        .lerp(browser.target_view.center, alpha);
    browser.current_view.height = lerp_f32(
        browser.current_view.height,
        browser.target_view.height,
        alpha,
    );

    // Apply to the camera.
    let view = browser.current_view;
    if let Ok((mut transform, mut projection)) = camera_q.single_mut() {
        transform.translation.x = view.center.x;
        transform.translation.y = view.center.y;
        if let Projection::Orthographic(ortho) = &mut *projection {
            // ScalingMode::FixedVertical maps the projection's vertical
            // span to `view.height` world units, regardless of window
            // size. Width auto-fits the aspect.
            ortho.scaling_mode = bevy::camera::ScalingMode::FixedVertical {
                viewport_height: view.height,
            };
            ortho.scale = 1.0;
        }
    }
}

fn lerp_f32(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ---- HUD -----------------------------------------------------------------

fn update_hud(browser: Res<Browser>, mut text_q: Query<&mut Text, With<HudText>>) {
    let Ok(mut text) = text_q.single_mut() else {
        return;
    };
    let rig_label = &browser.rigs[browser.current_rig].label;
    let rig_count = browser.rigs.len();
    let anim_label = browser
        .animations
        .get(browser.current_animation)
        .map_or("(loading)", String::as_str);
    let skin_label = browser
        .skins
        .get(browser.current_skin)
        .map_or("(loading)", String::as_str);
    text.0 = format!(
        "rig {}/{}: {rig_label}\nanim {}/{}: {anim_label}\nskin {}/{}: {skin_label}\nspeed: {:.2}x\n\n[Space] rig  [Shift+Space] prev  [N] anim  [S] skin  [R] reset  [+/-] speed  [Esc] quit",
        browser.current_rig + 1,
        rig_count,
        browser.current_animation + 1,
        browser.animations.len(),
        browser.current_skin + 1,
        browser.skins.len(),
        browser.time_scale,
    );
}
