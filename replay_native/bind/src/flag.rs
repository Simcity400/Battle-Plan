//! FLAG resolution — the authentic the game country-flag selection.
//!
//! the game stores per-country flags as `gfx/flags/<TAG>_<ideology>.tga` (and a
//! `gfx/flags/medium/<TAG>_<ideology>.tga` / `gfx/flags/small/...` set). The
//! "ideology-tinted flag" of the binding map is NOT a colour applied to a generic
//! shield — it is the IDEOLOGY-SPECIFIC FLAG IMAGE selected by the country's
//! `politics.ruling_party` (e.g. GER + `fascism` -> `GER_fascism.tga`). Selecting
//! the real file is faithful; fabricating a tint would not be.
//!
//! Resolution order (medium size, to match the topbar `GFX_shield_medium` slot),
//! each tried through the game resolver (mod overlay wins, alt extensions):
//!   1. `gfx/flags/medium/<TAG>_<ideology>.tga`
//!   2. `gfx/flags/medium/<TAG>.tga`                (ideology-less fallback)
//!   3. `gfx/flags/<TAG>_<ideology>.tga`            (large, if no medium)
//!   4. `gfx/flags/<TAG>.tga`
//! If none resolve, the flag is reported MISSING and the element keeps its
//! original `GFX_shield_medium` chrome (no fabricated flag).

use assets::resolver::GameResolver;
use std::path::PathBuf;

/// Resolves a country flag file from a TAG + ruling-party ideology.
pub struct FlagResolver<'a> {
    resolver: &'a GameResolver,
}

/// The outcome of resolving a flag.
#[derive(Debug, Clone)]
pub struct ResolvedFlag {
    /// The resolved on-disk flag image (`.tga`/`.dds`), ready to upload.
    pub path: PathBuf,
    /// The TAG it was resolved for.
    pub tag: String,
    /// The ideology used to pick the file (`None` = ideology-less fallback hit).
    pub ideology: Option<String>,
}

impl<'a> FlagResolver<'a> {
    pub fn new(resolver: &'a GameResolver) -> FlagResolver<'a> {
        FlagResolver { resolver }
    }

    /// Resolve the flag for `tag` with the optional `ideology` (ruling party).
    /// Returns `None` only if no flag file exists at all.
    pub fn resolve(&self, tag: &str, ideology: Option<&str>) -> Option<ResolvedFlag> {
        let mut candidates: Vec<(String, Option<String>)> = Vec::new();
        if let Some(id) = ideology {
            candidates.push((format!("gfx/flags/medium/{tag}_{id}.tga"), Some(id.to_string())));
        }
        candidates.push((format!("gfx/flags/medium/{tag}.tga"), None));
        if let Some(id) = ideology {
            candidates.push((format!("gfx/flags/{tag}_{id}.tga"), Some(id.to_string())));
        }
        candidates.push((format!("gfx/flags/{tag}.tga"), None));

        for (rel, ideo) in candidates {
            if let Some(path) = self.resolver.resolve(&rel) {
                return Some(ResolvedFlag {
                    path,
                    tag: tag.to_string(),
                    ideology: ideo,
                });
            }
        }
        None
    }
}
