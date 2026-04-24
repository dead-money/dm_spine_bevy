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

use dm_spine_runtime::load::{AtlasAttachmentLoader, JsonError, SkeletonJson};

use crate::asset::atlas_loader::SpineAtlasAsset;
use crate::asset::skel_loader::SpineSkeletonAsset;

/// Per-load overrides for the JSON skeleton loader. When `atlas_path` is
/// `None` (the default), the loader derives the atlas path from the
/// skeleton's filename stem — the same convention used by the binary
/// `.skel` loader (`spineboy-pro.json` -> `spineboy.atlas`).
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SpineSkeletonJsonLoaderSettings {
    /// Absolute asset path of the atlas, or `None` to auto-derive.
    pub atlas_path: Option<String>,
    /// Uniform scale applied to vertex coordinates at load time. `None` keeps
    /// the skeleton's native scale. Forwarded to `SkeletonJson::with_scale`.
    pub scale: Option<f32>,
}

/// Bevy asset loader for `.json` skeleton files. Loads the companion atlas
/// the same way the `.skel` loader does and yields a [`SpineSkeletonAsset`]
/// — the asset type is shared so both formats plug into the rest of the
/// pipeline identically.
#[derive(Default, TypePath)]
pub struct SpineSkeletonJsonLoader;

#[derive(Debug, Error)]
pub enum SpineSkeletonJsonLoaderError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not derive atlas path from skeleton path {0:?}")]
    AtlasPathDerivation(String),
    #[error("failed to load companion atlas {0:?}: {1}")]
    AtlasLoad(String, String),
    #[error("json skeleton parse error: {0}")]
    Parse(#[from] JsonError),
    #[error("asset path parse error: {0}")]
    Path(#[from] bevy::asset::ParseAssetPathError),
}

impl From<super::AtlasDeriveError> for SpineSkeletonJsonLoaderError {
    fn from(e: super::AtlasDeriveError) -> Self {
        match e {
            super::AtlasDeriveError::BadStem(s) => Self::AtlasPathDerivation(s),
            super::AtlasDeriveError::Parse(e) => Self::Path(e),
        }
    }
}

impl AssetLoader for SpineSkeletonJsonLoader {
    type Asset = SpineSkeletonAsset;
    type Settings = SpineSkeletonJsonLoaderSettings;
    type Error = SpineSkeletonJsonLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        let atlas_path = resolve_atlas_path(load_context.path(), settings.atlas_path.as_deref())?;

        let atlas_handle: Handle<SpineAtlasAsset> = load_context.load(atlas_path.clone());

        let loaded_atlas = load_context
            .loader()
            .immediate()
            .load::<SpineAtlasAsset>(atlas_path.clone())
            .await
            .map_err(|e| {
                SpineSkeletonJsonLoaderError::AtlasLoad(atlas_path.to_string(), e.to_string())
            })?;
        let atlas_asset = loaded_atlas.get();
        let mut attachment_loader = AtlasAttachmentLoader::new(&atlas_asset.atlas);

        let mut json = SkeletonJson::with_loader(&mut attachment_loader);
        if let Some(scale) = settings.scale {
            json = json.with_scale(scale);
        }
        let data = json.read_slice(&bytes)?;

        Ok(SpineSkeletonAsset {
            data: Arc::new(data),
            atlas: atlas_handle,
        })
    }

    fn extensions(&self) -> &[&str] {
        &["json"]
    }
}

fn resolve_atlas_path(
    json_path: &AssetPath<'static>,
    override_path: Option<&str>,
) -> Result<AssetPath<'static>, SpineSkeletonJsonLoaderError> {
    super::derive_atlas_path(json_path, override_path).map_err(Into::into)
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
        let j = make_path("rigs/spineboy/export/spineboy-pro.json");
        let atlas = resolve_atlas_path(&j, None).unwrap();
        assert_eq!(
            atlas.path().to_str(),
            Some("rigs/spineboy/export/spineboy.atlas")
        );
    }

    #[test]
    fn honours_override() {
        let j = make_path("rigs/spineboy-pro.json");
        let atlas = resolve_atlas_path(&j, Some("packs/hero.atlas")).unwrap();
        assert_eq!(atlas.path().to_str(), Some("packs/hero.atlas"));
    }
}
