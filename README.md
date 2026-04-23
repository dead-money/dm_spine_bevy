# dm_spine_bevy

Bevy 0.18 integration for [`dm_spine_runtime`](https://github.com/dead-money/dm_spine_runtime), the native Rust port of the [Spine](https://esotericsoftware.com/) 4.2 runtime.

This crate is the thin layer that maps the runtime's renderer-agnostic `RenderCommand` stream onto Bevy 2D meshes, materials, and systems. The runtime itself carries no GPU or windowing dependency.

**Status:** Early Phase 7 — the full load / tick / render path works end-to-end (`examples/spineboy_walk`), but polish, docs, and extended example coverage are ongoing. Not yet production-ready.

## Important: Spine Editor license required

This crate inherits the license obligations of `dm_spine_runtime`, which is a derivative of the official [spine-runtimes](https://github.com/EsotericSoftware/spine-runtimes) C++ runtime.

- **End users need a Spine Editor license.** If you ship software that uses this crate, each of your users must hold their own [Spine Editor license](https://esotericsoftware.com/spine-purchase). Same obligation as every official Spine runtime.
- **Copyright and license notices must be preserved.** Every source file carries the Esoteric Software copyright block; the `LICENSE` file reproduces the Spine Runtimes License verbatim.

Consult the [Spine licensing page](https://esotericsoftware.com/spine-purchase) or contact Esoteric Software directly if your use case is in doubt.

## Compatibility

- Bevy 0.18.x
- Spine 4.2 binary `.skel` + `.atlas` files (PMA atlases expected — see [Atlas expectations](#atlas-expectations))

## Quick start

Add to your `Cargo.toml`:

```toml
[dependencies]
bevy = "0.18"
dm_spine_runtime = { path = "../dm_spine_runtime" }
dm_spine_bevy = { path = "../dm_spine_bevy" }
```

Register the plugin and spawn a skeleton:

```rust
use bevy::prelude::*;
use dm_spine_bevy::{SpinePlugin, SpineSkeleton, SpineSkeletonAsset, SpineSkeletonLoaderSettings};

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(SpinePlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2d);

    let skel: Handle<SpineSkeletonAsset> = asset_server.load_with_settings(
        "spineboy/export/spineboy-pro.skel",
        |s: &mut SpineSkeletonLoaderSettings| {
            s.atlas_path = Some("spineboy/export/spineboy-pma.atlas".into());
        },
    );

    commands.spawn(SpineSkeleton::new(skel).with_initial_animation(0, "walk", true));
}
```

See `examples/spineboy_walk.rs` for a runnable version.

## Public API surface

- `SpinePlugin` — registers asset loaders, `Material2dPlugin`, the three system sets, and a pair of `Message` types for animation events.
- `SpineAtlasAsset` / `SpineSkeletonAsset` — asset wrappers. `SpineAtlasAsset::pages: Vec<Handle<Image>>` is the `TextureId(u32) → Handle<Image>` resolution table, index-parallel with the parsed atlas's pages.
- `SpineSkeleton` — per-instance component. Holds the asset handle, lazily-constructed runtime state, `time_scale`, `physics` mode, `paused`, and a pending-animation slot for spawn-time playback.
- `SpineMaterial` — PMA-aware `Material2d` with per-blend-mode pipeline specialization (Normal / Additive / Multiply / Screen).
- `SpineSet` — three ordered stages in `Update`: `Init` → `Tick` → `BuildMeshes` → `Events`.
- `SpineStateEvent` / `SpineKeyframeEvent` — drained from each skeleton's lifecycle + keyframe streams every frame.

## Atlas expectations

The built-in material assumes **premultiplied-alpha** textures and applies the PMA blending equations (`ONE, ONE_MINUS_SRC_ALPHA` for Normal, etc.). Most Spine exports ship `*-pma.atlas` / `*-pma.png` variants next to the straight-alpha pair — prefer those.

For straight-alpha atlases you currently have two options:

1. Override the atlas path on load (as in the quick-start above) to point at the PMA variant.
2. Pre-multiply your PNGs as part of your asset pipeline before they enter `assets/`.

A future sub-phase will auto-detect the atlas's `pma:` flag and premultiply at image-load time if needed.

## Examples

All examples live under `examples/` and run from the crate root. The default asset-path setting expects the sibling `spine-runtimes/examples/` directory to exist — see the runtime crate's README for how to check that out.

- `spineboy_walk` — opens a window with spineboy-pro looping the `walk` animation.
- `spineboy_screenshot` — headless-friendly sibling. Runs N frames then writes a PNG via Bevy's `Screenshot` API. `SPINE_SCREENSHOT`, `SPINE_SCREENSHOT_FRAMES` env vars configure output.

More examples (mesh-heavy rigs, skins, clipping, bounds queries) land as Phase 7 progresses.

## Commands

- `cargo check --all-targets` — fast type-check.
- `cargo test` — unit + integration (`tests/plugin_registers.rs` exercises the plugin under a minimal headless app).
- `cargo clippy --all-targets` — lint.
- `cargo run --example spineboy_walk` — first visual.

## Architecture notes

- One `RenderCommand` maps to one child entity (parented to the `SpineSkeleton` entity) with its own `Mesh2d` + `MeshMaterial2d<SpineMaterial>`. Meshes are mutated in place each frame via `Assets::get_mut`.
- Draw order is preserved by giving each child a small per-index Z offset; `Transparent2d` sorts back-to-front, and the runtime already emits commands in the correct order.
- `SkeletonData` is `Arc`-shared across instances. Spawning multiple copies of the same rig is an `Arc::clone`, not a deep copy.
- The runtime's adjacency batcher is preserved: adjacent commands sharing texture + blend + color merge into one draw call, so spineboy-pro is typically rendered as a single mesh.

## Related crates

- [`dm_spine_runtime`](https://github.com/dead-money/dm_spine_runtime) — the renderer-agnostic core this crate depends on. Data loaders, skeleton pose, animation state, constraints, clipping, bounds, render-command emission.

## License

Distributed under the [Spine Runtimes License](./LICENSE). See that file for the full text.
