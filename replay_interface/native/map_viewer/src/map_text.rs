
use crate::*;
use std::collections::BTreeMap;

pub const LOCALISATION_ROOT: &str = "localisation";
pub const DRAW_COUNTRY_NAMES_CUTOFF_KEY: &str = "DRAW_COUNTRY_NAMES_CUTOFF";

pub fn localisation_table_from_resolver(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<BTreeMap<String, String>> {
    let rel_dir = format!("{LOCALISATION_ROOT}/{language}");
    let files = resolver.resolve_dir_files(&rel_dir, Some("yml"))?;
    let mut table = BTreeMap::new();
    for (rel, _) in files {
        let bytes = resolver.read_required_bytes(&rel, "map-text localisation")?;
        let text = String::from_utf8_lossy(&bytes);
        merge_localisation_source(&mut table, &text, &rel);
    }
    Ok(table)
}

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

pub fn merge_localisation_source_exact(
    table: &mut BTreeMap<String, String>,
    source: &str,
    origin: &str,
    expected_header: &str,
) -> Result<()> {
    let mut saw_header = false;
    for (line_index, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if !saw_header {
            if !line.eq_ignore_ascii_case(expected_header) {
                return Err(format!(
                    "production localisation {origin}:{} header {line:?} is not {expected_header:?}",
                    line_index + 1
                )
                .into());
            }
            saw_header = true;
            continue;
        }
        let colon = line.find(':').ok_or_else(|| {
            format!(
                "production localisation {origin}:{} has no key separator",
                line_index + 1
            )
        })?;
        let key = line[..colon].trim();
        if key.is_empty() || key.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(format!(
                "production localisation {origin}:{} has an invalid key",
                line_index + 1
            )
            .into());
        }
        let mut rest = line[colon + 1..].trim_start();
        rest = rest.trim_start_matches(|character: char| character.is_ascii_digit());
        rest = rest.trim_start();
        let quoted = rest.strip_prefix('"').ok_or_else(|| {
            format!(
                "production localisation {origin}:{} key {key:?} is not quoted",
                line_index + 1
            )
        })?;
        let closing = closing_quote_index(quoted).ok_or_else(|| {
            format!(
                "production localisation {origin}:{} key {key:?} has no closing quote, or text after the value that is neither a comment nor part of it",
                line_index + 1
            )
        })?;
        if table
            .insert(key.to_string(), quoted[..closing].to_string())
            .is_some()
        {
            return Err(format!(
                "production localisation key {key:?} has multiple effective definitions"
            )
            .into());
        }
    }
    if !saw_header {
        return Err(format!("production localisation {origin} has no language header").into());
    }
    Ok(())
}

/// `NGraphics.DRAW_COUNTRY_NAMES_CUTOFF` from 00_graphics.lua (vanilla
/// 260.0): the camera distance below which the game stops drawing country
/// names.
pub fn draw_country_names_cutoff_from_resolver(resolver: &GameFileResolver) -> Result<f32> {
    let (sources, _) = crate::xmap09_lighting_law::effective_define_sources(resolver, "effective graphics defines")?;
    let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;
    evaluated.scalar("NGraphics", DRAW_COUNTRY_NAMES_CUTOFF_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::fs;
    use std::path::Path;

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
    fn production_localisation_rejects_duplicate_and_malformed_rows() {
        let mut table = BTreeMap::new();
        merge_localisation_source_exact(
            &mut table,
            "\u{feff}l_english:\n GER:0 \"Germany\"\n",
            "a.yml",
            "l_english:",
        )
        .expect("first exact source");
        assert!(merge_localisation_source_exact(
            &mut table,
            "l_english:\n GER:0 \"German Empire\"\n",
            "b.yml",
            "l_english:",
        )
        .expect_err("duplicate effective definition must fail closed")
        .to_string()
        .contains("multiple effective definitions"));
        assert!(merge_localisation_source_exact(
            &mut BTreeMap::new(),
            "l_english:\n malformed row\n",
            "bad.yml",
            "l_english:",
        )
        .expect_err("malformed semantic input must not be skipped")
        .to_string()
        .contains("no key separator"));
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
    fn admitted_localisation_reads_refuse_a_tampered_snapshot() {
        let root = temp_test_dir("map_text_loc_admitted_snapshot");
        let vanilla = root.join("vanilla");
        write_text(
            &vanilla,
            "localisation/english/countries_l_english.yml",
            VANILLA_LOC,
        );
        let resolver = GameFileResolver::vanilla(vanilla.clone())
            .expect("resolver")
            .bind_all_test_content(&"11".repeat(32))
            .expect("admission");
        let table = localisation_table_from_resolver(&resolver, "english").expect("admitted loc");
        assert_eq!(table.get("GER").map(String::as_str), Some("Germany"));
        let (_, snapshot) = resolver
            .resolve_dir_files("localisation/english", Some("yml"))
            .expect("admitted listing")
            .remove(0);
        assert!(!snapshot.starts_with(&vanilla), "admitted listing names the live file");
        crate::test_support::overwrite_readonly(&snapshot, b"l_english:\n GER:0 \"Tampered\"\n");
        let error = localisation_table_from_resolver(&resolver, "english")
            .expect_err("tampered snapshot")
            .to_string();
        assert!(error.contains("immutable admitted content snapshot changed"), "{error}");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn cutoff_parses_from_defines_fixture() {
        let root = temp_test_dir("map_text_cutoff");
        let vanilla = root.join("vanilla");
        write_text(
            &vanilla,
            BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE,
            "NDefines = {\n\tNGraphics = {\n\t\tDRAW_COUNTRY_NAMES_CUTOFF = 260,\n\t},\n}\n",
        );
        let resolver = GameFileResolver::vanilla(vanilla).expect("resolver");
        let cutoff = draw_country_names_cutoff_from_resolver(&resolver).expect("cutoff");
        assert_eq!(cutoff, 260.0);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_localisation_and_cutoff() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
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


pub(crate) fn closing_quote_index(quoted: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in quoted.char_indices() {
        if character == '"' && !escaped {
            let trailing = quoted[index + 1..].trim();
            if trailing.is_empty() || trailing.starts_with('#') {
                // FIRST qualifying candidate, not the last: a real trailing
                // comment can itself end in a quoted phrase
                // (`key: "value" # see "this"`), and taking the last candidate
                // swallowed the comment into the value. The vanilla
                // inner-quote line is unaffected because only its final quote
                // qualifies at all -- every earlier one leaves prose behind.
                return Some(index);
            }
        }
        escaped = character == '\\' && !escaped;
        if character != '\\' {
            escaped = false;
        }
    }
    None
}


#[cfg(test)]
mod value_terminator_tests {
    use super::closing_quote_index;

    /// Shipped vanilla (`aat_events_l_english.yml:1886`) puts an unescaped
    /// quoted phrase inside an event title, and a real trailing comment can
    /// itself end in a quoted phrase. One rule has to get both right.
    ///
    /// kills: returning the FIRST unescaped quote regardless of what follows
    /// truncates the vanilla title; returning the LAST qualifying candidate
    /// swallows the comment into the value (Codex review round 1); dropping the
    /// `#` clause makes the commented line refuse.
    #[test]
    fn unescaped_inner_quotes_are_a_value_not_a_syntax_error() {
        // The vanilla line, with the opening quote already stripped.
        let vanilla = r#"[ICE.GetName] Declares itself "Protector of the North Sea Passage""#;
        let end = closing_quote_index(vanilla).expect("vanilla title terminates");
        assert_eq!(
            &vanilla[..end],
            r#"[ICE.GetName] Declares itself "Protector of the North Sea Passage"#
        );

        let commented = r#"value" # see "this""#;
        let end = closing_quote_index(commented).expect("commented line terminates");
        assert_eq!(&commented[..end], "value");

        let plain = r#"value""#;
        assert_eq!(&plain[..closing_quote_index(plain).unwrap()], "value");

        assert_eq!(closing_quote_index("no closing quote at all"), None);
    }
}
