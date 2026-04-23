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

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::sprite_render::MeshMaterial2d;

use dm_spine_runtime::render::RenderCommand;

use crate::asset::SpineAtlasAsset;
use crate::components::SpineSkeleton;
use crate::material::{SpineBlendMode, SpineColors, SpineMaterial};

/// Small per-command Z offset applied to child mesh transforms so the
/// `Transparent2d` phase preserves the runtime's back-to-front command
/// order. Commands emitted later by `SkeletonRenderer::commands()` sit in
/// front of earlier ones.
const Z_OFFSET_PER_COMMAND: f32 = 0.001;

/// Builds (and rebuilds) Bevy meshes + materials for every initialized
/// skeleton each frame. Runs in [`crate::SpineSet::BuildMeshes`], after
/// the tick system populated `SkeletonRenderer`'s internal command buffer.
///
/// Strategy: each skeleton owns a parallel `Vec<Entity>` /
/// `Vec<Handle<Mesh>>` / `Vec<Handle<SpineMaterial>>`, one slot per
/// `RenderCommand`. Meshes and materials are mutated in place via
/// `Assets::get_mut`; new child entities are spawned only when the command
/// count grows. Children past `cmds.len()` are hidden rather than
/// despawned so growth/shrink/growth doesn't churn entities.
#[allow(clippy::too_many_arguments)]
pub fn build_spine_meshes(
    mut commands: Commands,
    mut query: Query<(Entity, &mut SpineSkeleton)>,
    atlases: Res<Assets<SpineAtlasAsset>>,
    skeleton_assets: Res<Assets<crate::asset::SpineSkeletonAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SpineMaterial>>,
    mut child_vis: Query<&mut Visibility>,
) {
    for (entity, mut sk) in &mut query {
        let Some(skel_asset) = skeleton_assets.get(&sk.asset) else {
            continue;
        };
        let Some(atlas) = atlases.get(&skel_asset.atlas) else {
            continue;
        };
        let Some(state) = sk.state.as_mut() else {
            continue;
        };

        let cmd_count = state.renderer.commands().len();
        grow_child_buffers(
            &mut commands,
            entity,
            state,
            cmd_count,
            &mut meshes,
            &mut materials,
        );

        // `commands()` returns &[RenderCommand]; we need the slice twice
        // (once for the for-loop, once for the shrink loop) but re-borrow
        // mutably in between via Assets::get_mut — so copy the length up
        // top and borrow the slice only inside the loop iterations.
        for i in 0..cmd_count {
            let cmd = &state.renderer.commands()[i];
            let tex = atlas
                .pages
                .get(cmd.texture.0 as usize)
                .cloned()
                .unwrap_or_default();

            if let Some(mesh) = meshes.get_mut(&state.meshes[i]) {
                write_mesh_from_command(mesh, cmd);
            }
            if let Some(mat) = materials.get_mut(&state.materials[i]) {
                mat.texture = tex;
                mat.colors = SpineColors {
                    light: unpack_argb(cmd.colors.first().copied().unwrap_or(0xffff_ffff)),
                    dark: unpack_argb(cmd.dark_colors.first().copied().unwrap_or(0xff00_0000)),
                };
                mat.blend_mode = SpineBlendMode::from(cmd.blend_mode);
            }
            if let Ok(mut vis) = child_vis.get_mut(state.children[i]) {
                *vis = Visibility::Visible;
            }
        }

        // Hide any trailing children that have no command this frame.
        for &child in &state.children[cmd_count..] {
            if let Ok(mut vis) = child_vis.get_mut(child) {
                *vis = Visibility::Hidden;
            }
        }
    }
}

/// Ensure `state.meshes` / `state.materials` / `state.children` have at
/// least `cmd_count` entries, spawning new child entities for any new
/// slots. Existing slots are left alone.
fn grow_child_buffers(
    commands: &mut Commands,
    parent: Entity,
    state: &mut crate::components::SpineSkeletonState,
    cmd_count: usize,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<SpineMaterial>,
) {
    while state.meshes.len() < cmd_count {
        let i = state.meshes.len();
        let mesh_handle = meshes.add(empty_mesh());
        let material_handle = materials.add(SpineMaterial::default());
        let z = (i as f32) * Z_OFFSET_PER_COMMAND;
        let child = commands
            .spawn((
                Mesh2d(mesh_handle.clone()),
                MeshMaterial2d(material_handle.clone()),
                Transform::from_xyz(0.0, 0.0, z),
                Visibility::Hidden,
                ChildOf(parent),
            ))
            .id();
        state.meshes.push(mesh_handle);
        state.materials.push(material_handle);
        state.children.push(child);
    }
}

fn empty_mesh() -> Mesh {
    // We mutate these meshes every frame via Assets::get_mut, so they must
    // stay resident in the main world. `RENDER_WORLD` alone causes the
    // mesh to be extracted and dropped from the main world after frame 1,
    // which trips `Mesh::insert_attribute` next frame.
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
}

/// Convert a [`RenderCommand`]'s interleaved position/uv buffers + index
/// list into mesh attributes, overwriting whatever was there.
pub(crate) fn write_mesh_from_command(mesh: &mut Mesh, cmd: &RenderCommand) {
    let n = cmd.num_vertices();
    let mut positions = Vec::with_capacity(n);
    let mut uvs = Vec::with_capacity(n);
    for i in 0..n {
        positions.push([cmd.positions[i * 2], cmd.positions[i * 2 + 1], 0.0]);
        uvs.push([cmd.uvs[i * 2], cmd.uvs[i * 2 + 1]]);
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U16(cmd.indices.clone()));
}

/// Unpack a spine-runtime `0xAARRGGBB` color into a `[0..1]^4` RGBA Vec4.
/// The light-color channel is already pre-multiplied by alpha on the CPU
/// side (see `pack_color` in `dm_spine_runtime::render`).
pub(crate) fn unpack_argb(v: u32) -> Vec4 {
    let a = ((v >> 24) & 0xff) as f32 / 255.0;
    let r = ((v >> 16) & 0xff) as f32 / 255.0;
    let g = ((v >> 8) & 0xff) as f32 / 255.0;
    let b = (v & 0xff) as f32 / 255.0;
    Vec4::new(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_white_opaque() {
        assert_eq!(unpack_argb(0xffff_ffff), Vec4::splat(1.0));
    }

    #[test]
    fn unpacks_red_half_alpha_matches_runtime_pack() {
        // Runtime packs `pack_color(1, 0, 0, 0.5) == 0x7fff_0000`; 0.5 * 255
        // truncates to 127 = 0x7f on the C++ side, so the round trip is
        // 127/255 = 0.4980...
        let v = unpack_argb(0x7fff_0000);
        assert!((v.x - 1.0).abs() < 1e-6);
        assert!(v.y.abs() < 1e-6);
        assert!(v.z.abs() < 1e-6);
        assert!((v.w - 127.0 / 255.0).abs() < 1e-6);
    }
}
