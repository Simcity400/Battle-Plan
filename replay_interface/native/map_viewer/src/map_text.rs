//! Map-label text sources (OFF-4): localisation strings and the
//! country-name zoom cutoff, resolver-routed per MOD_COMPATIBILITY.md.
//!
//! This backs the replay app's EXPLICITLY-LABELED FALLBACK label renderer
//! (`--map-labels-fallback`): the engine's curved label layout is a known
//! [ENGINE-LAW] gap (GAME_FILES_ATLAS_APPENDIX "Map label curving/layout"),
//! so the replay draws straight game-font labels at computed anchors until
//! that law is reconstructed.
//!
//! Sources:
//! * `localisation/<language>/*.yml` — UTF-8 (BOM) files of
//!   `KEY:VERSION "text"` lines under an `l_<language>:` header. Country
//!   display names key off the plain tag (`GER: "Germany"`); ideology
//!   variants (`GER_fascism`) need game state the recording does not carry
//!   yet and stay out of the fallback.
//! * `common/defines/00_graphics.lua` `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF`
//!   (vanilla 260) — "Cutoff for drawing country names on the map"; the
//!   only shipped numeric law tying country-name visibility to camera
//!   distance.
//!
//! Merge law note: the engine's exact cross-file localisation precedence
//! (replace dirs, key version numbers) is NOT reproduced here; files merge
//! in sorted rel-path order with later files overriding duplicate keys.
//! That is sufficient for tag -> display-name lookups and is documented as
//! part of the fallback.

use crate::*;
use std::collections::BTreeMap;

pub const LOCALISATION_ROOT: &str = "localisation";
pub const DRAW_COUNTRY_NAMES_CUTOFF_KEY: &str = "DRAW_COUNTRY_NAMES_CUTOFF";

/// Load and merge every `localisation/<language>/*.yml` through the
/// resolver (mod overrides win file-by-file; mod-added files merge in).
pub fn localisation_table_from_resolver(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<BTreeMap<String, String>> {
    let rel_dir = format!("{LOCALISATION_ROOT}/{language}");
    let files = resolver.resolve_dir_files(&rel_dir, Some("yml"))?;
    let mut table = BTreeMap::new();
    for (rel, path) in files {
        let bytes =
            std::fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        merge_localisation_source(&mut table, &text, &rel);
    }
    Ok(table)
}

/// Parse one localisation yml source into `table` (later keys override).
pub fn merge_localisation_source(
    table: &mut BTreeMap<String, String>,
    source: &str,
    _origin: &str,
) {
    for raw_line in source.lines() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let key = line[..colon].trim();
        if key.is_empty() || key.starts_with("l_") {
            continue;
        }
        let rest = line[colon + 1..].trim_start();
        // Optional version number between ':' and the quoted text.
        let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
        let rest = rest.trim_start();
        let Some(unquoted) = rest.strip_prefix('"') else {
            continue;
        };
        let Some(end) = unquoted.rfind('"') else {
            continue;
        };
        table.insert(key.to_string(), unquoted[..end].to_string());
    }
}

/// `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF` from 00_graphics.lua (vanilla
/// 260.0): the camera distance below which the game stops drawing country
/// names.
pub fn draw_country_names_cutoff_from_resolver(resolver: &GameFileResolver) -> Result<f32> {
    let path = resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let bytes = std::fs::read(&path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let source = latin1_bytes_to_string(&bytes);
    parse_lua_number_define(&source, DRAW_COUNTRY_NAMES_CUTOFF_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::{Path, PathBuf};

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = rel
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    const VANILLA_LOC: &str = "\u{feff}l_english:\n GER:0 \"Germany\"\n FRA:0 \"France\"\n # comment\n GER_fascism: \"German Reich\"\n";

    #[test]
    fn parses_yml_lines_with_versions_and_bom() {
        let mut table = BTreeMap::new();
        merge_localisation_source(&mut table, VANILLA_LOC, "test");
        assert_eq!(table.get("GER").map(String::as_str), Some("Germany"));
        assert_eq!(table.get("FRA").map(String::as_str), Some("France"));
        assert_eq!(
            table.get("GER_fascism").map(String::as_str),
            Some("German Reich")
        );
        assert!(!table.contains_key("l_english"));
    }

    #[test]
    fn localisation_from_resolver_uses_mod_override() {
        let root = temp_test_dir("map_text_loc_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "localisation/english/countries_l_english.yml",
            VANILLA_LOC,
        );
        write_text(
            &mod_root,
            "localisation/english/countries_l_english.yml",
            "l_english:\n GER:0 \"Deutsches Kaiserreich\"\n",
        );

        let resolver =
            GameFileResolver::with_mod_paths(vanilla, vec![mod_root.clone()]).expect("resolver");
        let table = localisation_table_from_resolver(&resolver, "english").expect("modded loc");
        assert_eq!(
            table.get("GER").map(String::as_str),
            Some("Deutsches Kaiserreich"),
            "mod layer must win"
        );
        // The whole vanilla file was overridden, so FRA vanished with it —
        // Paradox file-level override semantics.
        assert_eq!(table.get("FRA"), None);
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn localisation_from_resolver_merges_mod_added_files() {
        let root = temp_test_dir("map_text_loc_mod_added");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "localisation/english/countries_l_english.yml",
            VANILLA_LOC,
        );
        write_text(
            &mod_root,
            "localisation/english/zz_more_l_english.yml",
            "l_english:\n KRB: \"Carib Republic\"\n",
        );

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let table = localisation_table_from_resolver(&resolver, "english").expect("merged loc");
        assert_eq!(table.get("GER").map(String::as_str), Some("Germany"));
        assert_eq!(table.get("KRB").map(String::as_str), Some("Carib Republic"));
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn localisation_replace_path_excludes_vanilla_dir() {
        let root = temp_test_dir("map_text_loc_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "localisation/english/countries_l_english.yml",
            VANILLA_LOC,
        );
        write_text(
            &mod_root,
            "descriptor.mod",
            "name=\"ReplaceLoc\"\nreplace_path=\"localisation/english\"\n",
        );

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let table = localisation_table_from_resolver(&resolver, "english").expect("replaced loc");
        assert!(table.is_empty(), "replace_path must drop the vanilla file");
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cutoff_parses_from_defines_fixture() {
        let root = temp_test_dir("map_text_cutoff");
        let vanilla = root.join("vanilla");
        write_text(
            &vanilla,
            BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
            "NDefines_Graphics = {\n\tNGraphics = {\n\t\tDRAW_COUNTRY_NAMES_CUTOFF = 260,\n\t},\n}\n",
        );
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let cutoff = draw_country_names_cutoff_from_resolver(&resolver).expect("cutoff");
        assert_eq!(cutoff, 260.0);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn gate_vanilla_localisation_and_cutoff() {
        let game_dir = PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla map_text gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let table =
            localisation_table_from_resolver(&resolver, "english").expect("vanilla localisation");
        assert!(
            table.len() > 10_000,
            "vanilla english localisation should be large, got {}",
            table.len()
        );
        assert_eq!(table.get("GER").map(String::as_str), Some("Germany"));
        assert_eq!(table.get("FRA").map(String::as_str), Some("France"));
        let cutoff = draw_country_names_cutoff_from_resolver(&resolver).expect("cutoff");
        assert_eq!(cutoff, 260.0, "vanilla DRAW_COUNTRY_NAMES_CUTOFF");
        println!(
            "gate: vanilla localisation {} keys, DRAW_COUNTRY_NAMES_CUTOFF {cutoff}",
            table.len()
        );
    }
}
