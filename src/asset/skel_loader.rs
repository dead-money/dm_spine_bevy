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

use bevy::asset::{AssetLoader, AssetPath, LoadContext, io::Reader};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use dm_spine_runtime::data::SkeletonData;
use dm_spine_runtime::load::{AtlasAttachmentLoader, BinaryError, SkeletonBinary};

use crate::asset::atlas_loader::SpineAtlasAsset;

/// Spine skeleton asset wrapping the shared `Arc<SkeletonData>` and a handle
/// to the atlas it was loaded against. Instances clone the `Arc` cheaply.
#[derive(Asset, TypePath, Debug)]
pub struct SpineSkeletonAsset {
    /// Immutable parsed skeleton data. Shared across all `Skeleton` instances
    /// spawned from this asset.
    pub data: Arc<SkeletonData>,
    /// Handle to the atlas used during attachment resolution. The Bevy-side
    /// renderer uses this to pull `Vec<Handle<Image>>` for page lookups.
    pub atlas: Handle<SpineAtlasAsset>,
}

/// Per-load overrides for the skeleton loader. When `atlas_path` is `None`
/// (the default), the loader derives the atlas path from the skeleton's
/// filename stem — stripping trailing `-pro`/`-ess`/`-ios` suffixes where
/// present, then appending `.atlas`. This matches the naming convention used
/// by every rig under `spine-runtimes/examples/`.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SpineSkeletonLoaderSettings {
    /// Absolute asset path of the atlas, or `None` to auto-derive.
    pub atlas_path: Option<String>,
    /// Uniform scale applied to vertex coordinates at load time. `None` keeps
    /// the skeleton's native scale. Forwarded to `SkeletonBinary::with_scale`.
    pub scale: Option<f32>,
}

/// Bevy asset loader for `.skel` files. Loads the companion atlas
/// (resolved via [`SpineSkeletonLoaderSettings::atlas_path`] or derived
/// from the skeleton's filename stem), runs the binary skeleton parser
/// against it, and yields a [`SpineSkeletonAsset`].
#[derive(Default, TypePath)]
pub struct SpineSkeletonLoader;

#[derive(Debug, Error)]
pub enum SpineSkeletonLoaderError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not derive atlas path from skeleton path {0:?}")]
    AtlasPathDerivation(String),
    #[error("failed to load companion atlas {0:?}: {1}")]
    AtlasLoad(String, String),
    #[error("binary skeleton parse error: {0}")]
    Parse(#[from] BinaryError),
    #[error("asset path parse error: {0}")]
    Path(#[from] bevy::asset::ParseAssetPathError),
}

impl AssetLoader for SpineSkeletonLoader {
    type Asset = SpineSkeletonAsset;
    type Settings = SpineSkeletonLoaderSettings;
    type Error = SpineSkeletonLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        let atlas_path = resolve_atlas_path(load_context.path(), settings.atlas_path.as_deref())?;

        // Two loads on the same path: the `immediate` load gives us the owned
        // atlas value (needed to run the attachment loader here, synchronously),
        // and the `deferred` load yields a `Handle<SpineAtlasAsset>` we stash on
        // the asset for downstream texture-handle resolution. The asset server
        // dedupes these — same path, one load.
        let atlas_handle: Handle<SpineAtlasAsset> = load_context.load(atlas_path.clone());

        let loaded_atlas = load_context
            .loader()
            .immediate()
            .load::<SpineAtlasAsset>(atlas_path.clone())
            .await
            .map_err(|e| {
                SpineSkeletonLoaderError::AtlasLoad(atlas_path.to_string(), e.to_string())
            })?;
        let atlas_asset = loaded_atlas.get();
        let mut attachment_loader = AtlasAttachmentLoader::new(&atlas_asset.atlas);

        let mut binary = SkeletonBinary::with_loader(&mut attachment_loader);
        if let Some(scale) = settings.scale {
            binary = binary.with_scale(scale);
        }
        let data = binary.read(&bytes)?;

        Ok(SpineSkeletonAsset {
            data: Arc::new(data),
            atlas: atlas_handle,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["skel"]
    }
}

/// Derive the atlas asset path for a `.skel` path, honouring an explicit
/// override. Strip common rig-suffix variants (`-pro`, `-ess`, `-ios`) before
/// appending `.atlas`: `spineboy-pro.skel` -> `spineboy.atlas`.
fn resolve_atlas_path(
    skel_path: &AssetPath<'static>,
    override_path: Option<&str>,
) -> Result<AssetPath<'static>, SpineSkeletonLoaderError> {
    if let Some(p) = override_path {
        return Ok(AssetPath::parse(p).clone_owned());
    }

    let stem = skel_path
        .path()
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| SpineSkeletonLoaderError::AtlasPathDerivation(skel_path.to_string()))?;

    // Strip common export suffixes. These are the only variants shipped under
    // spine-runtimes/examples. Users with custom suffixes should pass
    // `SpineSkeletonLoaderSettings::atlas_path`.
    let base = ["-pro", "-ess", "-ios"]
        .into_iter()
        .find_map(|suffix| stem.strip_suffix(suffix))
        .unwrap_or(stem);

    let atlas_name = format!("{base}.atlas");
    Ok(skel_path.resolve_embed(&atlas_name)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_path(p: &str) -> AssetPath<'static> {
        AssetPath::from(PathBuf::from(p))
    }

    #[test]
    fn strips_pro_suffix() {
        let skel = make_path("rigs/spineboy/export/spineboy-pro.skel");
        let atlas = resolve_atlas_path(&skel, None).unwrap();
        assert_eq!(atlas.path().to_str(), Some("rigs/spineboy/export/spineboy.atlas"));
    }

    #[test]
    fn strips_ess_suffix() {
        let skel = make_path("rigs/spineboy-ess.skel");
        let atlas = resolve_atlas_path(&skel, None).unwrap();
        assert_eq!(atlas.path().to_str(), Some("rigs/spineboy.atlas"));
    }

    #[test]
    fn keeps_unsuffixed_stem() {
        let skel = make_path("rigs/raptor.skel");
        let atlas = resolve_atlas_path(&skel, None).unwrap();
        assert_eq!(atlas.path().to_str(), Some("rigs/raptor.atlas"));
    }

    #[test]
    fn honours_override() {
        let skel = make_path("rigs/spineboy-pro.skel");
        let atlas = resolve_atlas_path(&skel, Some("packs/hero.atlas")).unwrap();
        assert_eq!(atlas.path().to_str(), Some("packs/hero.atlas"));
    }
}
