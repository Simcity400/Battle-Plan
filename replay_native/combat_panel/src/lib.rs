//! `combat_panel` — STAGE 4 the COMBAT-PANEL ADAPTER (plan §C.2 + §G +
//! BINDING_MAP_diplomacy_combat.md).
//!
//! The flagship interaction: a clickable BUBBLE at each active combat's
//! `location_province` on the map; clicking a bubble opens the read-only COMBAT
//! PANEL (landcombat view) showing the ROSTER AGGREGATION — attacker/defender side
//! identity + location + duration + lifecycle — with EVERY battle-DETAIL field
//! badged UNAVAILABLE ("—"), never fabricated.
//!
//! Pipeline:
//!   1. [`domain`]  — pull `combats[]` + the province→controller table from a
//!                    snapshot, preserving Absent/Null/Measured at the boundary.
//!   2. [`model`]   — `combats[]` entry → [`model::CombatVM`] roster aggregation,
//!                    with the no-fabrication honesty grades (Codex-refuted).
//!   3. [`bubbles`] — province centroid (from the id raster) → screen bubble +
//!                    the click→bubble pick.
//!   4. [`view`]    — draw the bubbles + the stacked combat panel (reusing the
//!                    `render` primitives), with the "UNAVAILABLE — " badging.
//!
//! The shell wiring (bubble click → open panel, close control → close) is a small
//! drill registry mirrored from the app's `drill` module, kept here so the adapter
//! is self-contained + testable; the `app` shell registers the same rule.

pub mod bubbles;
pub mod domain;
pub mod drill;
pub mod model;
pub mod view;

pub use bubbles::{layout_bubbles, pick_bubble, Bubble, MapCamera, ProvinceCentroids};
pub use domain::CombatDomain;
pub use drill::{CombatDrill, CombatHit, PanelAction};
pub use model::{aggregate_combats, CombatVM, Field, Side, SideRoster, SideTag};

use std::collections::HashMap;
use std::path::Path;

use assets::resolver::GameResolver;

/// Static province-id → display NAME, loaded from `game_assets` map artifacts +
/// localisation. Province names in the game are the STATE's name in practice (a
/// province belongs to a state; the in-game battle title uses the state/region
/// name). We resolve province → state (`province_to_state.json`) → state name_key
/// (`state_meta.json`) → localised string. Missing at any hop → `None` (the panel
/// shows the exact province id instead — still honest).
#[derive(Debug, Clone, Default)]
pub struct ProvinceNames {
    /// province id -> resolved display name (already localised when possible).
    name: HashMap<i64, String>,
}

impl ProvinceNames {
    /// Build from the map_pipeline artifacts dir + a [`gui_runtime::loc::Loc`].
    /// `artifacts_dir` holds `province_to_state.json` + `state_meta.json`.
    pub fn load(artifacts_dir: &Path, loc: &gui_runtime::loc::Loc) -> ProvinceNames {
        let mut name = HashMap::new();
        let p2s = read_json_object(&artifacts_dir.join("province_to_state.json"));
        let sm = read_json_object(&artifacts_dir.join("state_meta.json"));
        // state id -> localised name
        let mut state_name: HashMap<u64, String> = HashMap::new();
        for (sid, v) in &sm {
            if let Ok(id) = sid.parse::<u64>() {
                if let Some(key) = v.get("name_key").and_then(|n| n.as_str()) {
                    let resolved = loc.resolve(key);
                    // loc.resolve returns the raw key when missing; we keep that
                    // (still informative, never blank — per the loc no-blank contract).
                    state_name.insert(id, resolved);
                }
            }
        }
        for (pid, v) in &p2s {
            if let (Ok(p), Some(sid)) = (pid.parse::<i64>(), v.as_u64()) {
                if let Some(n) = state_name.get(&sid) {
                    name.insert(p, n.clone());
                }
            }
        }
        ProvinceNames { name }
    }

    pub fn get(&self, province: i64) -> Option<String> {
        self.name.get(&province).cloned()
    }
    pub fn len(&self) -> usize {
        self.name.len()
    }
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }
}

/// Resolve a country flag image for a TAG, optionally tinted by ruling-party
/// `ideology` (the authentic the game ideology-specific flag, e.g. `ITA_fascism`).
/// Tries, in order: the live game install (`gfx/flags/...tga`, via the resolver),
/// then the extracted `game_assets/assets/flags/*.png` set (ideology-specific
/// first, then `<TAG>__medium` / `<TAG>`). `None` if no flag file exists at all —
/// the panel then shows the accent-coloured holding-tag slot (never a fabricated
/// flag, but the TAG itself stays exact/derived text).
pub fn flag_path(
    resolver: &GameResolver,
    extracted_flags_dir: Option<&Path>,
    tag: &str,
    ideology: Option<&str>,
) -> Option<std::path::PathBuf> {
    // (1) live install via resolver (mod overlay wins). Ideology-specific first.
    let mut rels: Vec<String> = Vec::new();
    if let Some(id) = ideology {
        rels.push(format!("gfx/flags/medium/{tag}_{id}.tga"));
    }
    rels.push(format!("gfx/flags/medium/{tag}.tga"));
    if let Some(id) = ideology {
        rels.push(format!("gfx/flags/{tag}_{id}.tga"));
    }
    rels.push(format!("gfx/flags/{tag}.tga"));
    for rel in &rels {
        if let Some(p) = resolver.resolve(rel) {
            return Some(p);
        }
    }
    // (2) extracted PNG flags (the game_assets set another agent regenerates).
    if let Some(dir) = extracted_flags_dir {
        let mut names: Vec<String> = Vec::new();
        if let Some(id) = ideology {
            names.push(format!("{tag}_{id}__medium.png"));
            names.push(format!("{tag}_{id}.png"));
        }
        names.push(format!("{tag}__medium.png"));
        names.push(format!("{tag}.png"));
        names.push(format!("{tag}__small.png"));
        for n in &names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Read a JSON file expected to be a top-level object; returns an empty map on any
/// IO/parse failure (callers degrade to "—", never fabricate).
fn read_json_object(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|txt| serde_json::from_str::<serde_json::Value>(&txt).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn province_names_empty_when_artifacts_missing() {
        let loc = gui_runtime::loc::Loc::new();
        let names = ProvinceNames::load(Path::new("/no/such/dir"), &loc);
        assert!(names.is_empty());
        assert!(names.get(12740).is_none());
    }
}
