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

use std::sync::Arc;

use bevy::prelude::*;

use dm_spine_runtime::animation::{
    AnimationState, AnimationStateData, Event as SpineEvent, state::StateEvent,
};
use dm_spine_runtime::render::SkeletonRenderer;
use dm_spine_runtime::skeleton::{Physics, Skeleton};

use crate::asset::SpineSkeletonAsset;
use crate::components::{SpineSkeleton, SpineSkeletonState};

/// Stages run each frame on a [`SpineSkeleton`]. Gameplay code ordering
/// itself `.before(SpineSet::Tick)` can mutate `time_scale` / queue
/// animations on the same frame they take effect.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpineSet {
    /// First-frame construction of [`SpineSkeletonState`] once the asset
    /// finishes loading.
    Init,
    /// Advance animation time, apply timelines, recompute world transforms,
    /// rebuild the render-command stream.
    Tick,
    /// Consume the frame's render commands into Bevy meshes + materials.
    /// Wired in Phase 7c.
    BuildMeshes,
    /// Drain per-entry lifecycle + keyframe events into Bevy [`Event`]
    /// writers. Runs after [`Self::Tick`].
    Events,
}

/// Watches [`SpineSkeleton`] entities whose `state` is still `None` and the
/// asset they point at has arrived. Constructs the runtime state, runs a
/// first `update_cache` / `set_to_setup_pose`, and kicks off any
/// `pending_animation`.
pub fn initialize_spine_skeletons(
    mut query: Query<&mut SpineSkeleton>,
    assets: Res<Assets<SpineSkeletonAsset>>,
) {
    for mut sk in &mut query {
        if sk.state.is_some() {
            continue;
        }
        let Some(asset) = assets.get(&sk.asset) else {
            continue;
        };

        let data = Arc::clone(&asset.data);
        let mut skeleton = Skeleton::new(Arc::clone(&data));
        skeleton.update_cache();
        skeleton.set_to_setup_pose();
        // First-pass world transform with no physics stepping — matches the
        // Phase 0-6 end-to-end flow used by `tests/render_smoke.rs`.
        skeleton.update_world_transform(Physics::None);

        let state_data = Arc::new(AnimationStateData::new(Arc::clone(&data)));
        let mut animation_state = AnimationState::new(state_data);

        if let Some(pending) = sk.pending_animation.take()
            && let Err(err) = animation_state.set_animation_by_name(
                pending.track,
                &pending.name,
                pending.looping,
            )
        {
            warn!(
                "dm_spine_bevy: pending animation {:?} on track {} failed: {err:?}",
                pending.name, pending.track
            );
        }

        sk.state = Some(SpineSkeletonState {
            skeleton,
            animation_state,
            renderer: SkeletonRenderer::new(),
            events: Vec::new(),
        });
    }
}

/// Advance one frame on every initialized skeleton: update animation state,
/// apply timelines, re-integrate world transforms, emit the frame's render
/// commands into the internal buffer on [`SkeletonRenderer`]. Read commands
/// via `state.renderer.commands()` in [`SpineSet::BuildMeshes`].
pub fn tick_spine_skeletons(time: Res<Time>, mut query: Query<&mut SpineSkeleton>) {
    let base_dt = time.delta_secs();
    for mut sk in &mut query {
        if sk.paused {
            continue;
        }
        let scale = sk.time_scale;
        let physics = sk.physics;
        let Some(state) = sk.state.as_mut() else {
            continue;
        };

        let dt = base_dt * scale;
        state.animation_state.update(dt);
        state.events.clear();
        state
            .animation_state
            .apply(&mut state.skeleton, &mut state.events);
        state.skeleton.update_world_transform(physics);
        let _ = state.renderer.render(&state.skeleton);
    }
}

/// One lifecycle / keyframe event pulled off a [`SpineSkeleton`] after the
/// tick system ran. Carries the source entity so listeners that span
/// multiple skeletons can disambiguate. Bevy 0.18 renamed the plain
/// buffered-event type to `Message`; this is a `Message` despite the
/// historical `Event` suffix in the name.
#[derive(Message, Debug, Clone)]
pub struct SpineStateEvent {
    pub entity: Entity,
    pub event: StateEvent,
}

/// One animation keyframe event (spine-cpp `Event`) pulled off the per-frame
/// event buffer. Fired alongside [`SpineStateEvent`] with
/// `StateEvent::kind == EventType::Event`, but split out for consumers that
/// only care about keyframes.
#[derive(Message, Debug, Clone)]
pub struct SpineKeyframeEvent {
    pub entity: Entity,
    pub event: SpineEvent,
}

/// Drain per-skeleton events into Bevy's message system. Runs in
/// [`SpineSet::Events`], after [`SpineSet::Tick`].
pub fn drain_spine_events(
    mut query: Query<(Entity, &mut SpineSkeleton)>,
    mut state_writer: MessageWriter<SpineStateEvent>,
    mut keyframe_writer: MessageWriter<SpineKeyframeEvent>,
) {
    for (entity, mut sk) in &mut query {
        let Some(state) = sk.state.as_mut() else {
            continue;
        };
        for event in state.animation_state.drain_events() {
            state_writer.write(SpineStateEvent {
                entity,
                event: event.clone(),
            });
        }
        for event in state.events.drain(..) {
            keyframe_writer.write(SpineKeyframeEvent { entity, event });
        }
    }
}
