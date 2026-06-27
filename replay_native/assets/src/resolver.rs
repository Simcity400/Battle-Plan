//! Game-path resolver — mirrors `game_assets/extract.py`'s rule: a (read-only)
//! vanilla install plus an optional mod overlay, with the mod overriding vanilla
//! by relative path. The install path is supplied at RUNTIME (CLI arg or env),
//! never baked into source, so no personal path reaches a committed file.
//!
//! Precedence, highest first:
//!   1. explicit argument            (e.g. CLI `--vanilla`)
//!   2. `BATTLE_PLAN_VANILLA` env var
//!   3. the documented Steam default (the same string extract.py uses)

use std::path::{Path, PathBuf};

/// The documented default install location (identical to extract.py's
/// DEFAULT_VANILLA). This is a generic, non-personal Steam library path; the
/// user may override it via `--vanilla` / `BATTLE_PLAN_VANILLA`.
pub const DEFAULT_VANILLA: &str =
    r"C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV";

#[derive(Debug, Clone)]
pub struct GameResolver {
    pub vanilla: PathBuf,
    pub mod_root: Option<PathBuf>,
}

impl GameResolver {
    /// Build from an optional explicit vanilla path; falls back to env then the
    /// documented default. `mod_root` is optional (mod overlay).
    pub fn new(vanilla: Option<&str>, mod_root: Option<&str>) -> GameResolver {
        let vanilla = vanilla
            .map(|s| s.to_string())
            .or_else(|| std::env::var("BATTLE_PLAN_VANILLA").or_else(|_| std::env::var("HOI4_VANILLA")).ok())
            .unwrap_or_else(|| DEFAULT_VANILLA.to_string());
        GameResolver {
            vanilla: PathBuf::from(vanilla),
            mod_root: mod_root
                .map(|s| s.to_string())
                .or_else(|| std::env::var("BATTLE_PLAN_MOD").or_else(|_| std::env::var("HOI4_MOD")).ok())
                .map(PathBuf::from),
        }
    }

    pub fn vanilla_exists(&self) -> bool {
        self.vanilla.is_dir()
    }

    /// Resolve a relative game path (e.g. "gfx/interface/foo.dds"), mod overlay
    /// winning over vanilla. Tries alternate image extensions like extract.py.
    pub fn resolve(&self, rel: &str) -> Option<PathBuf> {
        let rel = rel.replace('\\', "/");
        let rel = rel.trim_start_matches('/');
        // direct hit (mod first)
        for base in self.bases() {
            let p = base.join(rel);
            if p.is_file() {
                return Some(p);
            }
        }
        // alternate extension fallback (.dds <-> .tga <-> .png)
        if let Some(dot) = rel.rfind('.') {
            let stem = &rel[..dot];
            for ext in [".dds", ".tga", ".png"] {
                for base in self.bases() {
                    let p = base.join(format!("{stem}{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
        None
    }

    fn bases(&self) -> Vec<&Path> {
        let mut v = Vec::new();
        if let Some(m) = &self.mod_root {
            v.push(m.as_path());
        }
        v.push(self.vanilla.as_path());
        v
    }
}
