// Spine Runtimes License Agreement
// Last updated April 5, 2025. Replaces all prior versions.
//
// Copyright (c) 2013-2025, Esoteric Software LLC
//
// See LICENSE for full terms.

//! Stress test: spawn N skeletons in a grid, run them all at full speed,
//! report frame budget + per-stage timing in the HUD. Use the arrow keys
//! to scale N up and down at runtime to find the cliff on your machine.
//!
//! ## What it actually measures
//!
//! Each frame Bevy runs:
//!
//! - `SpineSet::Init` — first-frame asset hookup (cheap once warm)
//! - `SpineSet::Tick` — animation update + apply + world transforms +
//!   render-command emission. Linear in N × bones-per-rig.
//! - `SpineSet::BuildMeshes` — convert command stream to Bevy `Mesh` +
//!   `MeshMaterial2d` per command. Linear in N × commands-per-rig + per-vertex.
//! - GPU pipeline (mesh extract, draw calls). Linear in commands.
//!
//! The HUD reports the first three plus overall fps/frame so you can see
//! which side starts hurting first.
//!
//! ## Controls
//!
//! - **`]`** / **`[`**: step the population to the next / previous perfect
//!   square (1 → 4 → 9 → 16 → …). Grid stays centered on the origin.
//! - **R**: reset to the initial count
//! - **Esc**: quit
//!
//! ## CLI / env
//!
//! - `--rig <substring>`: pick the rig by label substring (default `spineboy-pro`).
//! - `--anim <name>`: animation name to play. Default: spineboy's `run`,
//!   falling back to the first declared animation if absent.
//! - `--count <N>`: initial skeleton count (default 1).
//! - `--csv <path>`: append `frame,count,fps,tick_ms,build_ms` rows to
//!   the path each frame for offline analysis.
//! - `--width <w>` / `--height <h>`: window resolution.
//! - `--assets <path>` / `SPINE_EXAMPLES_DIR`: same resolution chain as
//!   `spine_browser`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use bevy::asset::AssetPlugin;
use bevy::diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};

use dm_spine_bevy::{SpinePlugin, SpineSet, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};
use dm_spine_runtime::skeleton::Physics;

mod common;
use common::RigEntry;

/// Diagnostic path for the per-frame `SpineSet::Tick` duration.
const TICK_MS: DiagnosticPath = DiagnosticPath::const_new("dm_spine_bevy/tick_ms");
/// Diagnostic path for the per-frame `SpineSet::BuildMeshes` duration.
const BUILD_MS: DiagnosticPath = DiagnosticPath::const_new("dm_spine_bevy/build_meshes_ms");

/// Default initial count. The example always presents a perfect-square
/// grid centered on the origin, growing in N² steps.
const DEFAULT_COUNT: usize = 1;
const GRID_CELL_PADDING: f32 = 1.05;

/// Round `count` down to the nearest perfect square's root.
fn grid_root_of(count: usize) -> usize {
    (count as f32).sqrt().round().max(1.0) as usize
}

#[derive(Default, Debug, Clone)]
struct Cli {
    assets: Option<PathBuf>,
    rig: Option<String>,
    anim: Option<String>,
    count: Option<usize>,
    csv: Option<PathBuf>,
    width: Option<u32>,
    height: Option<u32>,
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
            "--count" => cli.count = value.and_then(|v| v.parse().ok()),
            "--csv" => cli.csv = value.map(PathBuf::from),
            "--width" => cli.width = value.and_then(|v| v.parse().ok()),
            "--height" => cli.height = value.and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    cli
}


#[derive(Resource)]
struct StressConfig {
    rig: RigEntry,
    anim_override: Option<String>,
    initial_count: usize,
    /// Per-instance grid spacing in world units. Computed once after the
    /// first skeleton's bounds settle, then reused.
    cell_size: Option<Vec2>,
    /// Centroid of the rig's bounding box in attachment-local space.
    /// Subtracted from each grid cell's center to place the *visible body*
    /// at the cell center rather than the bone origin (which sits at the
    /// rig's feet for spineboy-style rigs).
    bbox_centroid: Vec2,
}

#[derive(Resource, Default)]
struct StressState {
    /// Currently spawned skeleton entities.
    spawned: Vec<Entity>,
    /// Number we're trying to converge to (may differ from `spawned.len()`
    /// transiently as spawn / despawn batches process across frames).
    target_count: usize,
    /// Counter used to seed deterministic per-instance jitter.
    spawn_counter: u64,
}

#[derive(Resource, Default)]
struct StageTimers {
    tick_start: Option<Instant>,
    build_start: Option<Instant>,
}

#[derive(Resource)]
struct CsvWriter {
    file: std::fs::File,
    frame: u64,
}

#[derive(Component)]
struct HudText;

/// Marker so we can identify skeletons that still need their per-instance
/// time-offset randomization applied (once `SpineSkeletonState` exists).
#[derive(Component)]
struct NeedsTimeOffset(f32);

fn main() -> ExitCode {
    let cli = parse_cli();

    let asset_root = match common::resolve_asset_root(cli.assets.clone()) {
        Ok(p) => p,
        Err(message) => {
            eprintln!("spine_stress: {message}");
            return ExitCode::from(1);
        }
    };
    let rigs = common::discover_rigs(&asset_root);
    if rigs.is_empty() {
        eprintln!(
            "spine_stress: no rigs found under {}",
            asset_root.display()
        );
        return ExitCode::from(1);
    }

    let want = cli.rig.as_deref().unwrap_or("spineboy-pro");
    let Some(rig) = rigs.iter().find(|r| r.label.contains(want)).cloned() else {
        eprintln!(
            "spine_stress: no rig matching {want:?}. Available labels:\n{}",
            rigs.iter().map(|r| format!("  - {}", r.label)).collect::<Vec<_>>().join("\n")
        );
        return ExitCode::from(1);
    };
    eprintln!("spine_stress: using rig {}", rig.label);

    let initial_count = cli.count.unwrap_or(DEFAULT_COUNT).max(1);

    // Vsync would mask the real cost — this is a stress test, not a
    // production loop. Lower-bound FPS is more useful than smooth.
    let resolution = match (cli.width, cli.height) {
        (Some(w), Some(h)) => WindowResolution::new(w, h),
        _ => WindowResolution::default(),
    };
    let window_plugin = WindowPlugin {
        primary_window: Some(Window {
            resolution,
            title: "spine_stress".into(),
            present_mode: PresentMode::Immediate,
            resizable: true,
            ..Default::default()
        }),
        ..Default::default()
    };

    let csv_writer = cli.csv.as_ref().map(|p| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(p)
            .expect("open --csv path");
        writeln!(file, "frame,count,fps,tick_ms,build_ms").unwrap();
        CsvWriter { file, frame: 0 }
    });

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
    .add_plugins(FrameTimeDiagnosticsPlugin::default())
    .insert_resource(StressConfig {
        rig,
        // Default to spineboy's `run` so the demo looks like the
        // stampede the stress test wants. CLI --anim wins.
        anim_override: cli.anim.or_else(|| Some("run".to_string())),
        initial_count,
        cell_size: None,
        bbox_centroid: Vec2::ZERO,
    })
    .insert_resource(StressState {
        target_count: initial_count,
        ..Default::default()
    })
    .insert_resource(StageTimers::default())
    .add_systems(Startup, (register_diagnostics, setup));

    if let Some(w) = csv_writer {
        app.insert_resource(w);
        app.add_systems(Update, write_csv_row);
    }

    // Optional one-shot screenshot for docs / smoke verification.
    if let Some(cfg) = common::ScreenshotConfig::from_env(
        "SPINE_STRESS_SCREENSHOT",
        "SPINE_STRESS_SCREENSHOT_FRAMES",
        120,
    ) {
        common::install_screenshot_driver(&mut app, cfg);
    }

    app
        .add_systems(
            Update,
            (
                handle_input,
                seed_time_offsets,
                measure_cell_size,
                converge_population,
                update_hud,
                mark_tick_start.before(SpineSet::Tick),
                mark_tick_end_and_build_start
                    .after(SpineSet::Tick)
                    .before(SpineSet::BuildMeshes),
                mark_build_end.after(SpineSet::BuildMeshes),
            ),
        )
        .run();

    ExitCode::SUCCESS
}

fn register_diagnostics(mut store: ResMut<DiagnosticsStore>) {
    store.add(Diagnostic::new(TICK_MS).with_suffix("ms").with_smoothing_factor(0.85));
    store.add(
        Diagnostic::new(BUILD_MS)
            .with_suffix("ms")
            .with_smoothing_factor(0.85),
    );
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2d);
    commands.spawn((
        HudText,
        Text::new("warming up…"),
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
}

// ---- Population control --------------------------------------------------

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    cfg: Res<StressConfig>,
    mut state: ResMut<StressState>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
        return;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        let root = grid_root_of(state.target_count);
        state.target_count = (root + 1).pow(2);
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        let root = grid_root_of(state.target_count);
        let next = root.saturating_sub(1).max(1);
        state.target_count = next.pow(2);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        state.target_count = cfg.initial_count;
    }
}

/// Spawn the first skeleton at the origin so we can measure its bounds and
/// derive a sensible per-cell grid size before laying out the rest. The
/// rest of the population converges once we have a measurement.
fn measure_cell_size(
    mut cfg: ResMut<StressConfig>,
    sk_query: Query<&SpineSkeleton>,
    state: Res<StressState>,
) {
    if cfg.cell_size.is_some() {
        return;
    }
    let Some(probe) = state.spawned.first() else {
        return;
    };
    let Ok(sk) = sk_query.get(*probe) else {
        return;
    };
    let Some(state_inner) = sk.state.as_ref() else {
        return;
    };
    let mut have_any = false;
    let mut xmin = f32::INFINITY;
    let mut xmax = f32::NEG_INFINITY;
    let mut ymin = f32::INFINITY;
    let mut ymax = f32::NEG_INFINITY;
    for cmd in state_inner.renderer.commands() {
        if let Some((cxmin, cxmax, cymin, cymax)) = cmd.position_bounds() {
            xmin = xmin.min(cxmin);
            xmax = xmax.max(cxmax);
            ymin = ymin.min(cymin);
            ymax = ymax.max(cymax);
            have_any = true;
        }
    }
    if have_any {
        let w = ((xmax - xmin) * GRID_CELL_PADDING).max(64.0);
        let h = ((ymax - ymin) * GRID_CELL_PADDING).max(64.0);
        cfg.cell_size = Some(Vec2::new(w, h));
        cfg.bbox_centroid = Vec2::new((xmin + xmax) * 0.5, (ymin + ymax) * 0.5);
    }
}

fn converge_population(
    mut commands: Commands,
    cfg: Res<StressConfig>,
    mut state: ResMut<StressState>,
    asset_server: Res<AssetServer>,
    cameras: Query<&mut Projection, With<Camera2d>>,
    mut camera_transforms: Query<&mut Transform, With<Camera2d>>,
    mut transforms: Query<&mut Transform, (With<SpineSkeleton>, Without<Camera2d>)>,
) {
    // Always have at least one skeleton spawned so cell-size measurement
    // can converge.
    if state.spawned.is_empty() {
        spawn_one(&mut commands, &cfg, &mut state, &asset_server, Vec2::ZERO);
    }
    let cell = cfg.cell_size.unwrap_or(Vec2::splat(300.0));
    let centroid = cfg.bbox_centroid;

    // Reposition every existing entity into the slot it would occupy at
    // the current target_count. Cheap (one Transform write per skeleton)
    // and means resizes flow into the new grid immediately.
    for (idx, &entity) in state.spawned.iter().enumerate() {
        let pos = grid_position(idx, state.target_count, cell) - centroid;
        if let Ok(mut t) = transforms.get_mut(entity) {
            t.translation.x = pos.x;
            t.translation.y = pos.y;
        }
    }

    // Adjust population toward target. Spawn / despawn in batches of up
    // to 64 per frame to avoid huge frame-time spikes.
    const BATCH: usize = 64;
    let current = state.spawned.len();
    if current < state.target_count {
        let to_spawn = (state.target_count - current).min(BATCH);
        for _ in 0..to_spawn {
            let idx = state.spawned.len();
            let pos = grid_position(idx, state.target_count, cell) - centroid;
            spawn_one(&mut commands, &cfg, &mut state, &asset_server, pos);
        }
    } else if current > state.target_count {
        let to_remove = (current - state.target_count).min(BATCH);
        let drain_start = state.spawned.len() - to_remove;
        for entity in state.spawned.drain(drain_start..) {
            commands.entity(entity).despawn();
        }
    }

    // The grid is centered on the origin, so the camera stays put.
    // Just refit the orthographic projection so the full grid is visible
    // with a margin once the population settles.
    if state.spawned.len() == state.target_count && state.target_count > 0 {
        let root = grid_root_of(state.target_count);
        let view_height = cell.y * root as f32 * 1.1;
        for mut t in camera_transforms.iter_mut() {
            t.translation.x = 0.0;
            t.translation.y = 0.0;
        }
        for mut p in cameras {
            if let Projection::Orthographic(ortho) = &mut *p {
                ortho.scaling_mode = bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: view_height,
                };
                ortho.scale = 1.0;
            }
        }
    }
}

fn spawn_one(
    commands: &mut Commands,
    cfg: &StressConfig,
    state: &mut StressState,
    asset_server: &AssetServer,
    pos: Vec2,
) {
    let atlas = cfg.rig.atlas_relpath.clone();
    let handle: Handle<SpineSkeletonAsset> = asset_server.load_with_settings(
        cfg.rig.skel_relpath.clone(),
        move |s: &mut SpineSkeletonLoaderSettings| {
            s.atlas_path = Some(atlas.clone());
        },
    );
    let mut sk = SpineSkeleton::new(handle);
    sk.physics = Physics::Update;
    if let Some(name) = &cfg.anim_override {
        sk = sk.with_initial_animation(0, name.clone(), true);
    }
    // Deterministic per-instance time offset so all skeletons aren't in
    // lockstep — defeats GPU coherence and inflates fps artificially.
    let offset = jitter(state.spawn_counter) * 5.0;
    state.spawn_counter += 1;
    let entity = commands
        .spawn((sk, Transform::from_translation(pos.extend(0.0)), NeedsTimeOffset(offset)))
        .id();
    state.spawned.push(entity);
}

/// Once a freshly-spawned skeleton has its `SpineSkeletonState`, advance
/// its internal animation clock by the marker's offset and remove the
/// marker. Without this, every spineboy walks in perfect sync.
fn seed_time_offsets(
    mut commands: Commands,
    mut q: Query<(Entity, &mut SpineSkeleton, &NeedsTimeOffset)>,
) {
    for (entity, mut sk, NeedsTimeOffset(offset)) in &mut q {
        // Need state to exist *and* an animation to have been applied
        // (otherwise the offset has nothing to act on).
        let ready = sk
            .state
            .as_ref()
            .is_some_and(|st| !st.animation_state.skeleton_data().animations.is_empty());
        if !ready {
            continue;
        }
        // The pending animation may have failed (e.g. the rig doesn't
        // have the requested name); fall back to the first declared
        // animation so something is always playing.
        if let Some(state) = sk.state.as_mut()
            && state.animation_state.current(0).is_none()
            && let Some(anim) = state.animation_state.skeleton_data().animations.first()
        {
            let name = anim.name.clone();
            sk.play(0, name, true);
        }
        if let Some(state) = sk.state.as_mut() {
            state.animation_state.update(*offset);
        }
        commands.entity(entity).remove::<NeedsTimeOffset>();
    }
}

/// Place `index` (0..total) in a square grid centered on the origin.
/// `total` is rounded up to the next perfect square so each row has the
/// same column count and the grid stays symmetric. Excess slots in the
/// final row are simply unused.
fn grid_position(index: usize, total: usize, cell: Vec2) -> Vec2 {
    let cols = grid_root_of(total);
    let row = index / cols;
    let col = index % cols;
    let x = (col as f32 - (cols as f32 - 1.0) * 0.5) * cell.x;
    let y = -((row as f32 - (cols as f32 - 1.0) * 0.5) * cell.y);
    Vec2::new(x, y)
}

// ---- Stage timing --------------------------------------------------------

fn mark_tick_start(mut t: ResMut<StageTimers>) {
    t.tick_start = Some(Instant::now());
}

/// One system between Tick and BuildMeshes: closes out tick timing,
/// opens build timing. Combined to avoid two unordered ResMut systems
/// touching `StageTimers` in the same gap.
fn mark_tick_end_and_build_start(mut t: ResMut<StageTimers>, mut diagnostics: Diagnostics) {
    let now = Instant::now();
    if let Some(start) = t.tick_start.take() {
        let ms = (now - start).as_secs_f64() * 1000.0;
        diagnostics.add_measurement(&TICK_MS, || ms);
    }
    t.build_start = Some(now);
}

fn mark_build_end(mut t: ResMut<StageTimers>, mut diagnostics: Diagnostics) {
    if let Some(start) = t.build_start.take() {
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        diagnostics.add_measurement(&BUILD_MS, || ms);
    }
}

// ---- HUD + CSV -----------------------------------------------------------

fn update_hud(
    state: Res<StressState>,
    cfg: Res<StressConfig>,
    diagnostics: Res<DiagnosticsStore>,
    mut text_q: Query<&mut Text, With<HudText>>,
) {
    let Ok(mut text) = text_q.single_mut() else {
        return;
    };
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);
    let tick_ms = diagnostics
        .get(&TICK_MS)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);
    let build_ms = diagnostics
        .get(&BUILD_MS)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);

    let root = grid_root_of(state.target_count);
    let header = format!(
        "rig: {}\ncount: {} ({}x{} target)\nfps: {fps:6.1}\nframe: {frame_ms:5.2} ms\ntick:  {tick_ms:5.2} ms\nbuild: {build_ms:5.2} ms\n\n[ ] ] grow / shrink one perfect square  [R] reset  [Esc] quit",
        cfg.rig.label,
        state.spawned.len(),
        root,
        root,
    );
    text.0 = header;
}

fn write_csv_row(
    mut writer: ResMut<CsvWriter>,
    state: Res<StressState>,
    diagnostics: Res<DiagnosticsStore>,
) {
    writer.frame += 1;
    let frame = writer.frame;
    let count = state.spawned.len();
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);
    let tick_ms = diagnostics
        .get(&TICK_MS)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);
    let build_ms = diagnostics
        .get(&BUILD_MS)
        .and_then(Diagnostic::smoothed)
        .unwrap_or(0.0);
    let _ = writeln!(
        writer.file,
        "{frame},{count},{fps:.2},{tick_ms:.3},{build_ms:.3}",
    );
}

// ---- Misc ----------------------------------------------------------------

/// Cheap deterministic 0..1 jitter from a counter. Not statistically
/// pretty; enough to scramble per-instance time offsets so the population
/// isn't in lockstep.
fn jitter(seed: u64) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0xC6BC_2796_31E1_F4D5);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    ((x as u32) as f32) / (u32::MAX as f32)
}

