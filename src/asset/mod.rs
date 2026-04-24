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

//! Bevy asset types + loaders for Spine `.atlas`, `.skel`, and `.json` files.

pub mod atlas_loader;
pub mod skel_json_loader;
pub mod skel_loader;

pub use atlas_loader::{SpineAtlasAsset, SpineAtlasLoader, SpineAtlasLoaderError};
pub use skel_json_loader::{
    SpineSkeletonJsonLoader, SpineSkeletonJsonLoaderError, SpineSkeletonJsonLoaderSettings,
};
pub use skel_loader::{
    SpineSkeletonAsset, SpineSkeletonLoader, SpineSkeletonLoaderError, SpineSkeletonLoaderSettings,
};

use bevy::asset::{AssetPath, ParseAssetPathError};
use thiserror::Error;

/// Shared atlas-path derivation used by every skeleton loader. Strips common
/// rig-suffix variants (`-pro`, `-ess`, `-ios`) off the stem and appends
/// `.atlas`, or returns the `override` path verbatim. Returns an
/// [`AtlasDeriveError`] that loader-specific error enums can `From`-convert.
pub(crate) fn derive_atlas_path(
    skel_path: &AssetPath<'static>,
    override_path: Option<&str>,
) -> Result<AssetPath<'static>, AtlasDeriveError> {
    if let Some(p) = override_path {
        return Ok(AssetPath::parse(p).clone_owned());
    }

    let stem = skel_path
        .path()
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| AtlasDeriveError::BadStem(skel_path.to_string()))?;

    let base = ["-pro", "-ess", "-ios"]
        .into_iter()
        .find_map(|suffix| stem.strip_suffix(suffix))
        .unwrap_or(stem);

    let atlas_name = format!("{base}.atlas");
    Ok(skel_path.resolve_embed(&atlas_name)?)
}

#[derive(Debug, Error)]
pub(crate) enum AtlasDeriveError {
    #[error("could not derive atlas path from skeleton path {0:?}")]
    BadStem(String),
    #[error("asset path parse error: {0}")]
    Parse(#[from] ParseAssetPathError),
}
