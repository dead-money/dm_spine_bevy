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

use bevy::prelude::*;

use dm_spine_runtime::animation::{AnimationState, Event};
use dm_spine_runtime::render::SkeletonRenderer;
use dm_spine_runtime::skeleton::{Physics, Skeleton, SkinNotFound};

use crate::asset::SpineSkeletonAsset;
use crate::material::SpineMaterial;

/// Per-instance Spine skeleton. Owns a [`Handle<SpineSkeletonAsset>`] plus
/// lazily-constructed runtime state ([`Skeleton`] + [`AnimationState`] +
/// [`SkeletonRenderer`]).
///
/// Spawn with [`SpineSkeleton::new`] and — optionally — chain
/// [`SpineSkeleton::with_initial_animation`] to start a track-0 animation
/// the frame the asset finishes loading.
#[derive(Component)]
#[require(Transform, Visibility)]
pub struct SpineSkeleton {
    /// Asset handle. Kept strong — the asset (and its `Arc<SkeletonData>`)
    /// stays alive as long as this component exists.
    pub asset: Handle<SpineSkeletonAsset>,
    /// Runtime state. `None` until [`crate::systems::initialize_spine_skeletons`]
    /// observes the asset finishing load and constructs it.
    pub state: Option<SpineSkeletonState>,
    /// Playback speed multiplier. Applied to `Time::delta_secs` each tick.
    pub time_scale: f32,
    /// Physics-integration mode forwarded to
    /// [`Skeleton::update_world_transform`]. Defaults to
    /// [`Physics::Update`].
    pub physics: Physics,
    /// When `true`, the tick system skips `update` / `apply` /
    /// `update_world_transform` / `render`.
    pub paused: bool,
    /// Animation to start on track 0 once the asset finishes loading.
    /// Overwritten on every successful init. Use [`SpineSkeleton::play`] for
    /// post-init playback.
    pub pending_animation: Option<PendingAnimation>,
    /// Skin to activate once the asset finishes loading. Applied after the
    /// pending animation, before the first tick. Use [`SpineSkeleton::set_skin`]
    /// for post-init skin changes.
    pub pending_skin: Option<String>,
}

/// Runtime state owned by a [`SpineSkeleton`] once its asset has loaded.
/// Constructed by [`crate::systems::initialize_spine_skeletons`]; everything
/// here is per-instance (no shared mutable state across skeletons).
pub struct SpineSkeletonState {
    /// Pose state — bone transforms, slot attachments, draw order.
    pub skeleton: Skeleton,
    /// Multi-track animation playback. Mutate directly via
    /// [`SpineSkeleton::animation_state_mut`] for queueing / mixing
    /// beyond what [`SpineSkeleton::play`] covers.
    pub animation_state: AnimationState,
    /// Owns scratch buffers + the most recent `RenderCommand` stream.
    /// Read via `renderer.commands()` in `SpineSet::BuildMeshes`.
    pub renderer: SkeletonRenderer,
    /// Reusable per-frame event buffer. Cleared and refilled each tick;
    /// drained into Bevy messages by [`crate::systems::drain_spine_events`].
    pub events: Vec<Event>,
    /// Mesh asset per `RenderCommand` slot. Sized up lazily as the
    /// frame-to-frame command count grows; the vec is index-parallel
    /// with `materials` and `children`. Populated by
    /// [`crate::mesh::build_spine_meshes`].
    pub meshes: Vec<Handle<Mesh>>,
    /// Material asset per `RenderCommand` slot. Index-parallel with
    /// `meshes`.
    pub materials: Vec<Handle<SpineMaterial>>,
    /// Child entity per `RenderCommand` slot. Index-parallel with
    /// `meshes`. Excess entities (after the command count shrinks) are
    /// hidden rather than despawned so re-growth reuses them.
    pub children: Vec<Entity>,
}

/// Deferred animation request stored on [`SpineSkeleton::pending_animation`]
/// when [`SpineSkeleton::play`] is called before the asset has loaded.
#[derive(Clone, Debug)]
pub struct PendingAnimation {
    /// Track index inside `AnimationState` (track 0 is the primary).
    pub track: usize,
    /// Animation name as declared in `SkeletonData::animations`.
    pub name: String,
    /// `true` to loop, `false` to play once.
    pub looping: bool,
}

impl SpineSkeleton {
    /// Construct a new skeleton component pointing at `asset`. Defaults:
    /// `time_scale = 1.0`, `physics = Physics::Update`, unpaused, no pending
    /// animation.
    #[must_use]
    pub fn new(asset: Handle<SpineSkeletonAsset>) -> Self {
        Self {
            asset,
            state: None,
            time_scale: 1.0,
            physics: Physics::Update,
            paused: false,
            pending_animation: None,
            pending_skin: None,
        }
    }

    /// Builder-style setter: queue `name` (looped or not) to start on track
    /// `track` the first frame the asset is available.
    #[must_use]
    pub fn with_initial_animation(
        mut self,
        track: usize,
        name: impl Into<String>,
        looping: bool,
    ) -> Self {
        self.pending_animation = Some(PendingAnimation {
            track,
            name: name.into(),
            looping,
        });
        self
    }

    /// Play `name` on `track`. If the asset has loaded, dispatches immediately
    /// via [`AnimationState::set_animation_by_name`] — failures are logged
    /// and swallowed. Otherwise queues the request, overwriting any prior
    /// pending animation.
    pub fn play(&mut self, track: usize, name: impl Into<String>, looping: bool) {
        let pending = PendingAnimation {
            track,
            name: name.into(),
            looping,
        };
        if let Some(state) = self.state.as_mut() {
            if let Err(err) = state.animation_state.set_animation_by_name(
                pending.track,
                &pending.name,
                pending.looping,
            ) {
                warn!(
                    "dm_spine_bevy: set_animation_by_name({}, {:?}, {}) failed: {err:?}",
                    pending.track, pending.name, pending.looping
                );
            }
        } else {
            self.pending_animation = Some(pending);
        }
    }

    /// Borrow the inner [`AnimationState`] once init has run. Returns `None`
    /// while the asset is still loading.
    #[must_use]
    pub fn animation_state(&self) -> Option<&AnimationState> {
        self.state.as_ref().map(|s| &s.animation_state)
    }

    /// Mutable variant of [`Self::animation_state`] for callers that need
    /// direct control (queuing, track clearing, mix tweaks).
    pub fn animation_state_mut(&mut self) -> Option<&mut AnimationState> {
        self.state.as_mut().map(|s| &mut s.animation_state)
    }

    /// Borrow the inner [`Skeleton`] once init has run.
    #[must_use]
    pub fn skeleton(&self) -> Option<&Skeleton> {
        self.state.as_ref().map(|s| &s.skeleton)
    }

    /// Mutable variant of [`Self::skeleton`] for hand-authored pose tweaks
    /// between ticks (constraint overrides, bone-follow gameplay code, etc.).
    pub fn skeleton_mut(&mut self) -> Option<&mut Skeleton> {
        self.state.as_mut().map(|s| &mut s.skeleton)
    }

    /// Builder-style setter: queue `name` to be activated as the skeleton's
    /// skin once the asset finishes loading. Use [`SpineSkeleton::set_skin`]
    /// for post-init changes.
    #[must_use]
    pub fn with_initial_skin(mut self, name: impl Into<String>) -> Self {
        self.pending_skin = Some(name.into());
        self
    }

    /// Activate skin `name`. If the asset has loaded, dispatches immediately
    /// via [`Skeleton::set_skin_by_name`] and re-resolves slot attachments
    /// against the new skin. Otherwise queues the request, overwriting any
    /// prior pending skin.
    ///
    /// # Errors
    /// Returns [`SkinNotFound`] if the skin name is unknown to the loaded
    /// asset. When queuing (asset not yet loaded), returns `Ok(())` — the
    /// request might still fail at init time, in which case it's logged and
    /// swallowed.
    pub fn set_skin(&mut self, name: impl Into<String>) -> Result<(), SkinNotFound> {
        let name = name.into();
        if let Some(state) = self.state.as_mut() {
            state.skeleton.set_skin_by_name(&name)?;
            state.skeleton.set_slots_to_setup_pose();
            Ok(())
        } else {
            self.pending_skin = Some(name);
            Ok(())
        }
    }

    /// Names of every animation declared by the loaded `SkeletonData`.
    /// Returns `None` while the asset is still loading.
    #[must_use]
    pub fn available_animations(&self) -> Option<Vec<&str>> {
        let skel = self.skeleton()?;
        Some(skel.data().animations.iter().map(|a| a.name.as_str()).collect())
    }

    /// Names of every skin declared by the loaded `SkeletonData`. The
    /// implicit `default` skin is always present at index 0.
    /// Returns `None` while the asset is still loading.
    #[must_use]
    pub fn available_skins(&self) -> Option<Vec<&str>> {
        let skel = self.skeleton()?;
        Some(skel.data().skins.iter().map(|s| s.name.as_str()).collect())
    }
}
