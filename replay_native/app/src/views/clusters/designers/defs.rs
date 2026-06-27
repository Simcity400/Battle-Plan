//! The static DEF catalog for the designer cluster: equipment MODULE / ARCHETYPE /
//! UPGRADE definitions, read READ-ONLY from `game_defs/out/catalogs.json`
//! (`catalogs.equipment_modules` / `.equipment_archetypes` / `.equipment_upgrades`).
//!
//! These DEFs supply the module/upgrade NAMES + ICONS + per-module STAT CONTRIBUTIONS
//! that the snapshot's `designs.equipment_variants[V].modules{slot:mod}` /
//! `upgrades{name:lvl}` reference by TOKEN. They are DERIVED def data (a token→name/icon/
//! contribution lookup), NOT a fabricated value: a token absent from the catalog renders
//! its raw token (derived) and an `Unavailable` icon — never an invented stat.
//!
//! NOTE on the binding map's stale file refs: it names
//! `equipment_modules.json`/`equipment_archetypes.json`/`equipment_upgrades.json` which do
//! NOT exist as separate files — the data lives inside `catalogs.json["catalogs"][...]`.
//! Stat values in the catalog are STRINGS (e.g. `"soft_attack":"2"`); they are surfaced
//! verbatim as derived text (no numeric coercion that could hide a parse).
//!
//! Resolution is READ-ONLY + retry-tolerant of the concurrent extractor: a missing/
//! unparseable catalog yields an EMPTY catalog (every lookup → None → the caller badges/
//! falls back to the token), never a panic and never a fabricated entry.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde_json::Value;

/// The loaded static DEF catalog (modules / archetypes / upgrades). Built once and
/// memoised; an absent catalog is an EMPTY catalog (all lookups None).
#[derive(Debug, Default)]
pub struct DefCatalog {
    /// module token → (display name, icon GFX, [(stat, value-string)] contributions).
    modules: HashMap<String, ModuleDef>,
    /// archetype token → display name (the `id` humanised, or its loc — kept as the id).
    archetypes: HashMap<String, String>,
    /// upgrade token → display name.
    upgrades: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
struct ModuleDef {
    name: String,
    icon: Option<String>,
    contributions: Vec<(String, String)>,
}

static CATALOG: OnceLock<DefCatalog> = OnceLock::new();

impl DefCatalog {
    /// The process-wide catalog (loaded once from `catalogs.json`, memoised). READ-ONLY.
    pub fn get() -> &'static DefCatalog {
        CATALOG.get_or_init(DefCatalog::load)
    }

    /// Load from `game_defs/out/catalogs.json` (sibling of `replay_native`). A miss → an
    /// empty catalog (never a panic, never a fabricated entry).
    fn load() -> DefCatalog {
        let parsed = Self::catalog_path()
            .and_then(|p| std::fs::read_to_string(&p).ok())
            .and_then(|txt| Self::parse(&txt));
        // a missing/unparseable catalog → the EMPTY catalog (a no-data fallback for static
        // DEF assets, not a snapshot value; every lookup then returns None).
        match parsed {
            Some(c) => c,
            None => DefCatalog::default(),
        }
    }

    /// `<repo>/game_defs/out/catalogs.json`, derived from the crate dir
    /// (`replay_native/app` → up 2 → `replay_native` → up 1 → repo root → `game_defs/out`).
    fn catalog_path() -> Option<PathBuf> {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let replay_native = crate_dir.parent()?; // app -> replay_native
        let repo = replay_native.parent()?; // replay_native -> repo root
        Some(repo.join("game_defs").join("out").join("catalogs.json"))
    }

    /// Parse the `catalogs` object out of the file text.
    fn parse(txt: &str) -> Option<DefCatalog> {
        let v: Value = serde_json::from_str(txt).ok()?;
        let cats = v.get("catalogs")?.as_object()?;
        let mut out = DefCatalog::default();

        if let Some(mods) = cats.get("equipment_modules").and_then(Value::as_object) {
            for (token, def) in mods {
                let name = humanise(token);
                let icon = def.get("icon").and_then(Value::as_str).map(str::to_string);
                let mut contributions = Vec::new();
                for stat_key in ["add_stats", "multiply_stats", "add_average_stats"] {
                    if let Some(stats) = def.get(stat_key).and_then(Value::as_object) {
                        let mut keys: Vec<&String> = stats.keys().collect();
                        keys.sort();
                        for k in keys {
                            if let Some(s) = stat_value_string(&stats[k]) {
                                contributions.push((k.clone(), s));
                            }
                        }
                    }
                }
                out.modules.insert(
                    token.clone(),
                    ModuleDef {
                        name,
                        icon,
                        contributions,
                    },
                );
            }
        }
        if let Some(arch) = cats.get("equipment_archetypes").and_then(Value::as_object) {
            for token in arch.keys() {
                out.archetypes.insert(token.clone(), humanise(token));
            }
        }
        if let Some(ups) = cats.get("equipment_upgrades").and_then(Value::as_object) {
            for token in ups.keys() {
                out.upgrades.insert(token.clone(), humanise(token));
            }
        }
        Some(out)
    }

    /// A module's display name (humanised token), if known.
    pub fn module_name(&self, token: &str) -> Option<String> {
        self.modules.get(token).map(|m| m.name.clone())
    }
    /// A module's icon GFX name, if the DEF carries one.
    pub fn module_icon(&self, token: &str) -> Option<String> {
        self.modules.get(token).and_then(|m| m.icon.clone())
    }
    /// A module's per-stat contributions `[(stat, value-string)]` (derived def text). An
    /// unknown token has NO contributions (an empty list — the caller then shows no stat
    /// rows; not a fabricated value).
    pub fn module_contributions(&self, token: &str) -> Vec<(String, String)> {
        match self.modules.get(token) {
            Some(m) => m.contributions.clone(),
            None => Vec::new(),
        }
    }
    /// An archetype's display name (humanised token), if known.
    pub fn archetype_name(&self, token: &str) -> Option<String> {
        self.archetypes.get(token).cloned()
    }
    /// An upgrade's display name (humanised token), if known.
    pub fn upgrade_name(&self, token: &str) -> Option<String> {
        self.upgrades.get(token).cloned()
    }

    /// Build a catalog directly from a `catalogs.json`-shaped value (TESTS only).
    #[cfg(test)]
    pub fn from_value(v: &Value) -> DefCatalog {
        match Self::parse(&v.to_string()) {
            Some(c) => c,
            None => DefCatalog::default(),
        }
    }
}

/// Surface a JSON stat value as a string verbatim (the catalog stores them as strings; a
/// number is rendered too). Non-scalar → None (skipped, never fabricated).
fn stat_value_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Turn a snake_case DEF token into a Title-Cased display label (a presentation-only
/// transform — it never invents a value, just formats an existing token).
fn humanise(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for (i, word) in token.split('_').enumerate() {
        if word.is_empty() {
            continue;
        }
        if i > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        token.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> DefCatalog {
        DefCatalog::from_value(&json!({
            "catalogs": {
                "equipment_modules": {
                    "main_battery_2": {
                        "icon": "GFX_ship_battery",
                        "add_stats": {"naval_attack": "20", "build_cost_ic": "5"}
                    },
                    "no_icon_mod": { "add_stats": {"defense": "3"} }
                },
                "equipment_archetypes": { "medium_tank_chassis_1": {} },
                "equipment_upgrades": { "tank_armor_upgrade": {"fields": {"max_level": "5"}} }
            }
        }))
    }

    #[test]
    fn module_lookup_name_icon_contributions() {
        let c = sample();
        assert_eq!(
            c.module_name("main_battery_2").as_deref(),
            Some("Main Battery 2")
        );
        assert_eq!(
            c.module_icon("main_battery_2").as_deref(),
            Some("GFX_ship_battery")
        );
        let contribs = c.module_contributions("main_battery_2");
        // sorted keys: build_cost_ic, naval_attack.
        assert_eq!(contribs[0], ("build_cost_ic".to_string(), "5".to_string()));
        assert_eq!(contribs[1], ("naval_attack".to_string(), "20".to_string()));
    }

    #[test]
    fn missing_icon_is_none_not_fabricated() {
        let c = sample();
        assert_eq!(c.module_name("no_icon_mod").as_deref(), Some("No Icon Mod"));
        assert_eq!(c.module_icon("no_icon_mod"), None);
    }

    #[test]
    fn unknown_token_returns_none() {
        let c = sample();
        assert_eq!(c.module_name("ghost"), None);
        assert_eq!(c.module_icon("ghost"), None);
        assert!(c.module_contributions("ghost").is_empty());
        assert_eq!(c.archetype_name("ghost"), None);
        assert_eq!(c.upgrade_name("ghost"), None);
    }

    #[test]
    fn archetype_and_upgrade_names() {
        let c = sample();
        assert_eq!(
            c.archetype_name("medium_tank_chassis_1").as_deref(),
            Some("Medium Tank Chassis 1")
        );
        assert_eq!(
            c.upgrade_name("tank_armor_upgrade").as_deref(),
            Some("Tank Armor Upgrade")
        );
    }

    #[test]
    fn empty_catalog_never_panics() {
        let c = DefCatalog::default();
        assert_eq!(c.module_name("x"), None);
        assert!(c.module_contributions("x").is_empty());
    }
}
