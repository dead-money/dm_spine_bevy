# Performance evaluation + improvement roadmap

Written 2026-04-23 against `dm_spine_bevy` commit `1665ff3` (post-simplify pass: parallel `tick_spine_skeletons`, `SpineInitialized` filter, `attribute_mut`-based mesh reuse).

Reference hardware: NVIDIA RTX 4090 + Intel i9-14900K (24 cores), Linux, X11/Vulkan windowed.

---

## What we know empirically

From `cargo run --release --example spine_stress -- --csv stress.csv` sweeping perfect-square counts:

| N    | tick     | build    | total spine | frame budget left | FPS  | likely bottleneck above this point |
|------|----------|----------|-------------|--------------------|------|-------------------------------------|
| 1    | 0.07 ms  | 0.02 ms  | 0.09 ms     | 16.58 ms           | 657  | GPU & schedule overhead             |
| 50   | 0.19 ms  | 0.07 ms  | 0.25 ms     | 16.42 ms           | 570  | GPU (lots of headroom)              |
| 200  | 0.44 ms  | 0.19 ms  | 0.63 ms     | 16.04 ms           | 310  | GPU draw calls                      |
| 500  | 0.93 ms  | 0.46 ms  | 1.39 ms     | 15.28 ms           | 156  | render extract + draw calls         |
| 1000 | 1.72 ms  | 1.03 ms  | 2.75 ms     | 13.92 ms           | 84   | render extract + draw calls         |

The cliff above ~500 instances is **not** on the runtime CPU side. Spine work scales near-linearly to <2 ms at 1000 with parallel tick. What's eating the 16.67 ms vsync budget at 1000 instances is everything *between* `BuildMeshes` and the wgpu submit: render extraction of 1000 mesh assets, GPU bind-group switches (one per material, currently one material per command per skeleton), and 1000+ draw calls.

---

## Tier 0 — verify before optimizing anything

These aren't optimizations; they're the questions you must answer first or you'll waste effort optimizing the wrong thing.

- **Profile a real game scene, not the stress test.** The stress test runs 1000 identical spineboys with identical animations. A real Dead Money game scene has 5–50 spineboys with different anims, mixed with sprites, particle systems, UI, audio, and physics. The bottleneck shape may be entirely different. **Step 1 of any perf work is "tell me what your real frame looks like."**
- **Pick a target.** "Sustains 60 fps with ≤ X ms of budget for spine" lets us walk away when we hit it. Without a target, we'll keep optimizing forever.
- **Wire perf regression detection into CI.** The CSV-export mode in `spine_stress` is a foundation. A CI job that fails when p99 frame time at N=200 exceeds Y ms catches accidental regressions before they reach the game.

---

## Tier 1 — measurement infrastructure (~half session each)

Worth doing before guessing further about optimizations. Each surfaces real signal we'd otherwise speculate about.

### 1. Frame-time histogram in `spine_stress`

Currently we report `Diagnostic::smoothed()`. Add a `--histogram` mode that records every frame time to the CSV and prints **percentiles (p50 / p95 / p99 / p99.9)** on exit. A smooth 60 fps average masks bad outliers; p99 > 16.67 ms means we drop frames even if average is fine. Critical for game-feel evaluation, where occasional 33 ms hitches are far worse than a steady 17 ms.

### 2. Per-stage diagnostics inside the bevy crate proper

The `mark_*_start` / `mark_*_end` timing systems in `spine_stress` only run in that example. Move them behind a feature gate (e.g. `dm_spine_bevy/profile`) so any consumer can flip them on. Costs ~80 LOC and gives every game using the crate the same per-stage visibility we currently get from the stress harness.

### 3. `puffin` or `tracy` integration

Bevy 0.18 ships `bevy_dev_tools` with optional tracing. Add a feature flag that enables it and pipes the four `SpineSet` stages as named scopes. Pays for itself the first time we need to see a flame graph of GPU upload vs render extract vs prepare vs draw.

### 4. GPU-side measurement

wgpu's `Timestamp` query feature gives draw-call-level GPU timings. Bevy 0.18 surfaces it via `RenderDiagnosticsPlugin`. Integrate and report `gpu_ms` alongside our two CPU stages. **Until we have this, "the bottleneck is GPU" is just inference** from the gap between `tick + build` and `frame_time`.

---

## Tier 2 — likely wins from theory + measurements (~1 session each)

Candidates to profile first. All target the build → draw path because that's where 14 ms of the 16 ms budget went at N=1000.

### 5. Material deduplication via `HashMap<MaterialKey, Handle<SpineMaterial>>`

Currently every render-command slot in every skeleton allocates its own `SpineMaterial` via `materials.add(SpineMaterial::default())`. Spineboy at 1000 instances = 1000 distinct materials, all with identical (texture, blend, white-tint, black-dark). One material handle would let Bevy's `Transparent2d` phase batch many meshes into one draw call.

This is **likely the single biggest GPU-side win available without changing the architecture**. Cheap experiment to run. Key risk: the current per-skeleton-per-command material asset means freeing one skeleton frees its materials cleanly; a shared cache needs careful refcounting.

**Open question to verify before doing this:** does Bevy 0.18 actually batch `Mesh2d` draws automatically when material handles match? If yes, this is a 10× improvement. If no, we need a custom render command (Tier 3 #9).

### 6. Mesh-asset dedup for static meshes

For any skeleton in pure setup pose with no Deform timelines active, the per-frame mesh content is identical across instances. Could share one `Handle<Mesh>` across all of them and only allocate a unique handle when the mesh actually deforms.

Niche win — most game scenarios have all skeletons animating — but cheap to add. Pairs naturally with #5.

### 7. Skip `BuildMeshes` for paused / off-screen skeletons

We already have `SpineSkeleton::paused`. Extend with a frustum-culling mechanism that reads viewport bounds + per-skeleton AABB (we have `aggregate_bounds` in `examples/common/`; could promote to the crate proper) and short-circuits the mesh-build stage for off-screen instances. Real games rarely render every spawned entity simultaneously.

### 8. Single-mesh-per-skeleton when batched

The runtime's adjacency batcher already merges most spineboy slots into one command. We currently produce one `Mesh2d` per command (so spineboy = one mesh). For rigs with multiple commands (like dragon, 14 commands per frame), consolidating into a single Mesh2d with grouped index ranges + instanced draws would help. Complex; defer until measurement says it matters.

---

## Tier 3 — bigger architectural moves (only if Tier 2 isn't enough)

### 9. Custom render command + manual draw batching

Bypass `Material2d` and write a `RenderCommand` for the wgpu queue directly. Lets us batch all skeletons' geometry into a few large draw calls keyed only on (texture, blend mode). Removes the per-instance `Mesh2d` overhead entirely.

This is what production Spine integrations (e.g. Unity's `SkeletonRenderer` URP, Unreal's SpineWidget) do. Bigger lift than Tier 2 but well-trodden ground; only worth it if we hit a hard wall after Tier 2.

### 10. GPU skinning

Per-skeleton bone matrices uploaded once per frame; vertex shader does the skinning instead of CPU. Kills `update_world_transform` + per-vertex CPU work entirely. Fundamental architecture change to the runtime crate. Defer until we truly need 5000+ skeletons.

### 11. Multi-skeleton parallel draw via instancing

When the same atlas + same skeleton-data is used by N instances and animations are limited to a known small set, GPU-side animation evaluation becomes possible. Even bigger lift than #10.

---

## Recommended order

1. **Tier 0 first.** Capture a real-game-scene profile + set a target. Without these, the rest is shadow boxing.
2. **Tier 1 #4 (GPU-side timestamps)** — only one session, and it's the missing data point. Replaces inference with measurement.
3. **Tier 2 #5 (material dedup)** — most likely to convert the GPU bottleneck into headroom. Cheap experiment.
4. **Stop here unless Dead Money's actual game scene hits the wall.** Tier 3 is real engineering effort for a problem we may not have. (Calibration: this entire crate, including the runtime port and the Bevy integration, was built in a single 11.5-hour AI-pair-programming session — so "weeks of effort" here means person-days, not calendar months. The decision criterion is still "do we need it?", not "can we afford it?".)

---

## What's already done (for context)

These are the optimizations already landed; not in the roadmap because they're shipped.

- **Parallel `tick_spine_skeletons`** via `par_iter_mut` — ~5× speedup at scale (10.4 ms → 2.7 ms total spine cost at N=1000).
- **`SpineInitialized` marker + `Without<>` filter** — `initialize_spine_skeletons` no longer scans steady-state skeletons.
- **In-place mesh attribute reuse** in `write_mesh_from_command` — eliminates per-frame `Vec` allocations for positions / UVs / indices.
- **`SkeletonRenderer::commands()` accessor** in the runtime crate — lets the build stage read the tick stage's output without re-rendering.
- **Adjacency batcher** in the runtime — merges adjacent same-(texture, blend, color) commands into single batched draws. Already running; spineboy = one batched command per frame.

The `spine_stress` example with `--csv` flag is the standing benchmark harness for any of the above.
