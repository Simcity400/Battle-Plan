use crate::assets::AssetResolver;
use crate::font::strip_color_codes;
use crate::paradox_text::{decode_text, destructive_event_count, diagnose_text_decode};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Localisation {
    pub entries: BTreeMap<String, String>,
    /// The sole effective-VFS definition file for each unambiguous key.
    ///
    /// `source_files` proves only that a corpus was loaded. It cannot prove
    /// which file the engine would select when two admitted files define the
    /// same key. Such keys are excluded from this map and tracked below.
    pub source_by_key: BTreeMap<String, String>,
    /// Keys for which more than one definition exists and the exact engine
    /// precedence is unproven. Such keys remain available to research callers
    /// in `entries`, but production consumers must refuse them.
    pub ambiguous_sources_by_key: BTreeMap<String, Vec<String>>,
    /// True only when every parsed key is classified as either uniquely
    /// sourced or explicitly ambiguous. It does not claim a winner for an
    /// ambiguous key.
    pub provenance_complete: bool,
    pub source_files: Vec<String>,
    pub parse_failures: Vec<String>,
    /// Files whose BYTES were destroyed at decode, with event counts.
    ///
    /// A separate axis from `parse_failures`, and invisible to it: an invalid
    /// byte inside a quoted value is replaced by U+FFFD and the resulting
    /// `KEY: "value"` line is still perfectly well formed, so
    /// `parse_english_yml` reports nothing while the displayed string is not
    /// the string the file contained. Measured: the shipped corpus produces
    /// none of these across all 10 languages, so a non-empty list really does
    /// mean localisation bytes were lost.
    pub decode_lossy_files: Vec<String>,
}

impl Localisation {
    pub fn load_english(resolver: &AssetResolver) -> Result<Self> {
        Self::load_language(resolver, "l_english")
    }

    /// Load exactly the playback viewer's selected language from the admitted
    /// effective VFS. There is no English fallback: if that language's files
    /// were not admitted by the portable content capability, the resulting
    /// catalog is empty and the product caller must fail closed.
    pub fn load_language(resolver: &AssetResolver, language: &str) -> Result<Self> {
        let language = normalise_language(language)?;
        let directory = language
            .strip_prefix("l_")
            .expect("normalise_language always returns an l_ prefix");
        let mut files =
            resolver.list_files_recursive(&format!("localisation/{directory}"), Some("yml"))?;
        files.sort_by(|left, right| {
            portable_content_manifest::authored_ui::engine_recursive_path_cmp(
                &left.rel_path,
                &right.rel_path,
            )
        });
        let mut loc = Self::default();
        for file in files {
            match fs::read(&file.path) {
                Ok(bytes) => {
                    let decode_events = destructive_event_count(&diagnose_text_decode(&bytes));
                    if decode_events != 0 {
                        loc.decode_lossy_files.push(format!(
                            "{}: {decode_events} destructive decode events",
                            file.rel_path
                        ));
                    }
                    let text = decode_text(&bytes);
                    // R11 C8: `parse_failures` could only ever hold `fs::read`
                    // errors, because parsing had no failure channel at all —
                    // a truncated or damaged .yml produced missing/corrupted
                    // entries and an empty failure list. Line-level damage is
                    // now recorded per file.
                    let mut file_entries = BTreeMap::new();
                    let (failures, duplicate_keys) =
                        parse_language_yml_with_duplicates(&text, &language, &mut file_entries);
                    for failure in failures {
                        loc.parse_failures
                            .push(format!("{}: {failure}", file.rel_path));
                    }
                    for (key, value) in file_entries {
                        let duplicate_in_file = duplicate_keys.contains(&key);
                        if duplicate_in_file {
                            let prior = loc.source_by_key.remove(&key);
                            let sources =
                                loc.ambiguous_sources_by_key.entry(key.clone()).or_default();
                            if let Some(prior) = prior {
                                sources.push(prior);
                            }
                            if !sources.contains(&file.rel_path) {
                                sources.push(file.rel_path.clone());
                            }
                        } else if let Some(sources) =
                            loc.ambiguous_sources_by_key.get_mut(&key)
                        {
                            if !sources.contains(&file.rel_path) {
                                sources.push(file.rel_path.clone());
                            }
                        } else if let Some(prior) =
                            loc.source_by_key.insert(key.clone(), file.rel_path.clone())
                        {
                            loc.source_by_key.remove(&key);
                            loc.ambiguous_sources_by_key
                                .insert(key.clone(), vec![prior, file.rel_path.clone()]);
                        }
                        loc.entries.insert(key.clone(), value);
                    }
                    loc.source_files.push(file.rel_path);
                }
                Err(err) => loc
                    .parse_failures
                    .push(format!("{}: {}", file.rel_path, err)),
            }
        }
        // Preserve the engine parse order. Sorting here used to erase the
        // authenticated case-sensitive sequence after duplicate-key winners
        // had already been selected.
        loc.parse_failures.sort();
        loc.decode_lossy_files.sort();
        for sources in loc.ambiguous_sources_by_key.values_mut() {
            sources.sort();
            sources.dedup();
        }
        loc.provenance_complete = loc.entries.keys().all(|key| {
            loc.source_by_key.contains_key(key)
                ^ loc.ambiguous_sources_by_key.contains_key(key)
        });
        Ok(loc)
    }

    /// Product loader for one viewer-selected language. This couples language
    /// selection to the portable content capability and refuses empty,
    /// unreadable, malformed, or byte-damaged catalogs in one place so panel
    /// assemblers cannot accidentally treat them as an English-fallback case.
    pub fn load_admitted_language(resolver: &AssetResolver, language: &str) -> Result<Self> {
        if !resolver.has_portable_content_admission() {
            return Err(crate::GameUiError::missing(
                "product localisation requires a portable content-admission resolver",
            ));
        }
        let localisation = Self::load_language(resolver, language)?;
        if localisation.source_files.is_empty() || localisation.entries.is_empty() {
            return Err(crate::GameUiError::missing(format!(
                "admitted content has no localisation entries for {language:?}"
            )));
        }
        if !localisation.parse_failures.is_empty() || !localisation.decode_lossy_files.is_empty() {
            return Err(crate::GameUiError::parse(format!(
                "selected-language localisation is incomplete: parse={:?}, decode={:?}",
                localisation.parse_failures, localisation.decode_lossy_files
            )));
        }
        if !localisation.provenance_complete {
            return Err(crate::GameUiError::missing(
                "selected-language localisation has an unclassified key provenance",
            ));
        }
        for source in localisation.source_by_key.values() {
            if resolver.admitted_content_key(source).is_none() {
                return Err(crate::GameUiError::missing(format!(
                    "unique localisation definition {source:?} is absent from the portable content admission"
                )));
            }
        }
        for sources in localisation.ambiguous_sources_by_key.values() {
            for source in sources {
                if resolver.admitted_content_key(source).is_none() {
                    return Err(crate::GameUiError::missing(format!(
                        "ambiguous localisation definition {source:?} is absent from the portable content admission"
                    )));
                }
            }
        }
        Ok(localisation)
    }

    /// Return a production-safe value for `key`.
    ///
    /// `entries` deliberately retains one provisional value for research and
    /// diagnostics, but callers using the ordinary lookup API must never
    /// receive that value when more than one definition exists and the
    /// engine's effective-VFS precedence has not been proven.
    pub fn get(&self, key: &str) -> Option<&str> {
        if self.ambiguous_sources_by_key.contains_key(key) {
            return None;
        }
        self.entries.get(key).map(String::as_str)
    }

    pub fn source_file_for_key(&self, key: &str) -> Option<&str> {
        self.source_by_key.get(key).map(String::as_str)
    }

    pub fn ambiguous_sources_for_key(&self, key: &str) -> Option<&[String]> {
        self.ambiguous_sources_by_key.get(key).map(Vec::as_slice)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

fn normalise_language(language: &str) -> Result<String> {
    let language = language.trim().to_ascii_lowercase();
    let language = if language.starts_with("l_") {
        language
    } else {
        format!("l_{language}")
    };
    let suffix = language
        .strip_prefix("l_")
        .expect("normalised language has an l_ prefix");
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(crate::GameUiError::parse(
            "playback localisation language is not a safe Clausewitz language identity",
        ));
    }
    Ok(language)
}

/// Parse one `l_english` localisation file into `out`, returning one
/// diagnostic per line that could not be interpreted as a `KEY: "value"` pair.
///
/// Skipping a damaged line is the right rendering behaviour (the rest of the
/// file is still good), but skipping it *silently* is what made
/// `Localisation::parse_failures` structurally incapable of reporting a parse
/// failure. The diagnostics are the report; the caller decides what to do.
#[must_use = "line-level localisation damage must be recorded, not dropped"]
pub fn parse_english_yml(text: &str, out: &mut BTreeMap<String, String>) -> Vec<String> {
    let mut duplicates = BTreeSet::new();
    parse_localisation_rows(text, Some("l_english"), out, &mut duplicates)
}

/// Parse one Clausewitz localisation file for exactly `language`.
///
/// Files whose header names another language, and malformed or missing first
/// content headers, are reported rather than silently ignored or interpreted
/// as localisation keys.
#[must_use = "line-level localisation damage must be recorded, not dropped"]
pub fn parse_language_yml(
    text: &str,
    language: &str,
    out: &mut BTreeMap<String, String>,
) -> Vec<String> {
    parse_language_yml_with_duplicates(text, language, out).0
}

pub(crate) fn parse_language_yml_with_duplicates(
    text: &str,
    language: &str,
    out: &mut BTreeMap<String, String>,
) -> (Vec<String>, BTreeSet<String>) {
    let expected = match normalise_language(language) {
        Ok(language) => language,
        Err(error) => return (vec![error.message], BTreeSet::new()),
    };
    let mut content_lines = text.lines().filter(|raw_line| {
        let line = strip_comment(raw_line).trim();
        !line.is_empty()
    });
    let Some(header_line) = content_lines.next() else {
        return (
            vec![format!("missing {expected} localisation header")],
            BTreeSet::new(),
        );
    };
    let header = strip_comment(header_line).trim().trim_end_matches(':');
    if !header.eq_ignore_ascii_case(&expected) {
        return (
            vec![format!(
                "localisation header {header:?} does not match selected language {expected:?}"
            )],
            BTreeSet::new(),
        );
    }
    for raw_line in content_lines {
        let line = strip_comment(raw_line).trim();
        if is_language_header(line) && !line.trim_end_matches(':').eq_ignore_ascii_case(&expected) {
            return (
                vec![format!(
                    "localisation file mixes selected language {expected:?} with header {:?}",
                    line.trim_end_matches(':')
                )],
                BTreeSet::new(),
            );
        }
    }
    let mut duplicates = BTreeSet::new();
    let failures = parse_localisation_rows(text, Some(&expected), out, &mut duplicates);
    (failures, duplicates)
}

fn is_language_header(line: &str) -> bool {
    let language = line.to_ascii_lowercase();
    let Some(language) = language.strip_suffix(':') else {
        return false;
    };
    let Some(suffix) = language.strip_prefix("l_") else {
        return false;
    };
    !suffix.is_empty()
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn parse_localisation_rows(
    text: &str,
    accepted_header: Option<&str>,
    out: &mut BTreeMap<String, String>,
    duplicate_keys: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut failures = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if accepted_header
            .is_some_and(|header| line.trim_end_matches(':').eq_ignore_ascii_case(header))
        {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            failures.push(format!(
                "line {}: no key/value separator in {:?}",
                index + 1,
                truncate_for_report(line)
            ));
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            failures.push(format!("line {}: empty localisation key", index + 1));
            continue;
        }
        let (value, unterminated) = parse_localised_value(rest.trim());
        if unterminated {
            failures.push(format!(
                "line {}: unterminated quoted value for key {key}",
                index + 1
            ));
        }
        if out.insert(key.to_string(), value).is_some() {
            duplicate_keys.insert(key.to_string());
        }
    }
    failures
}

fn truncate_for_report(line: &str) -> String {
    let mut out: String = line.chars().take(60).collect();
    if out.chars().count() < line.chars().count() {
        out.push('…');
    }
    out
}

pub fn is_localisation_key(text: &str) -> bool {
    let text = strip_color_codes(text).trim().trim_matches('$').to_string();
    text.len() > 1
        && !text.contains(char::is_whitespace)
        && text
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.'))
        && text.chars().any(|ch| ch.is_ascii_uppercase())
}

fn strip_comment(line: &str) -> &str {
    let mut escaped = false;
    let mut quoted = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..idx],
            _ => {}
        }
    }
    line
}

/// Returns the value and whether a quoted value ran off the end of the line
/// (accepted, because the visible text is still the best available reading,
/// but reported so the damage is never invisible).
fn parse_localised_value(rest: &str) -> (String, bool) {
    let rest = rest
        .trim_start()
        .trim_start_matches(|ch: char| ch.is_ascii_digit())
        .trim_start();
    if let Some((value, terminated)) = parse_quoted(rest) {
        return (value, !terminated);
    }
    (rest.trim().to_string(), false)
}

fn parse_quoted(text: &str) -> Option<(String, bool)> {
    let mut chars = text.chars();
    if chars.next()? != '"' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for ch in chars {
        if escaped {
            out.push(match ch {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some((out, true)),
            other => out.push(other),
        }
    }
    Some((out, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "game_ui_xui06_{label}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after Unix epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn parses_header_version_and_color_escapes() {
        let mut entries = BTreeMap::new();
        let failures = parse_english_yml(
            "l_english:\n MAIN_MENU:0 \"Main menu\"\n SAMPLE_TT:0 \"§YHeader§!\\nBody # not comment\" # comment\n",
            &mut entries,
        );
        assert!(failures.is_empty(), "{failures:?}");
        let localisation = Localisation {
            entries,
            ..Localisation::default()
        };
        assert_eq!(localisation.get("MAIN_MENU"), Some("Main menu"));
        assert_eq!(
            localisation.get("SAMPLE_TT"),
            Some("§YHeader§!\nBody # not comment")
        );
    }

    #[test]
    fn line_level_damage_is_reported_not_silently_skipped() {
        let mut entries = BTreeMap::new();
        let failures = parse_english_yml(
            "l_english:\n GOOD:0 \"kept\"\n this line has no separator\n :0 \"no key\"\n OPEN:0 \"never closed\n",
            &mut entries,
        );
        assert_eq!(entries.get("GOOD").map(String::as_str), Some("kept"));
        // The salvageable value is still kept; the damage is still reported.
        assert_eq!(
            entries.get("OPEN").map(String::as_str),
            Some("never closed")
        );
        assert_eq!(failures.len(), 3, "{failures:?}");
        assert!(
            failures[0].contains("no key/value separator"),
            "{failures:?}"
        );
        assert!(
            failures[1].contains("empty localisation key"),
            "{failures:?}"
        );
        assert!(
            failures[2].contains("unterminated quoted value"),
            "{failures:?}"
        );
    }

    #[test]
    fn selected_language_header_is_exact_and_has_no_english_fallback() {
        let mut entries = BTreeMap::new();
        assert!(parse_language_yml(
            "l_french:\n HELLO:0 \"Bonjour\"\n",
            "l_french",
            &mut entries,
        )
        .is_empty());
        assert_eq!(entries.get("HELLO").map(String::as_str), Some("Bonjour"));

        let mut mismatched = BTreeMap::new();
        let failures = parse_language_yml(
            "l_english:\n HELLO:0 \"Hello\"\n",
            "l_french",
            &mut mismatched,
        );
        assert_eq!(failures.len(), 1);
        assert!(mismatched.is_empty());

        let mut mixed = BTreeMap::new();
        let failures = parse_language_yml(
            "l_french:\n HELLO:0 \"Bonjour\"\nl_english:\n HELLO:0 \"Hello\"\n",
            "l_french",
            &mut mixed,
        );
        assert_eq!(failures.len(), 1);
        assert!(mixed.is_empty());

        let failures = parse_language_yml(
            "l_french:\n HELLO:0 \"Bonjour\"\nL_ENGLISH:\n HELLO:0 \"Hello\"\n",
            "l_french",
            &mut mixed,
        );
        assert_eq!(failures.len(), 1);
        assert!(mixed.is_empty());
    }

    #[test]
    fn cross_file_and_same_file_duplicates_are_ambiguous_per_key() {
        let root = temp_root("loc_winner");
        let directory = root.join("localisation/english");
        fs::create_dir_all(&directory).expect("create localisation fixture");
        fs::write(
            directory.join("a_l_english.yml"),
            "l_english:\n DUPLICATE:0 \"first\"\n SAME_FILE:0 \"one\"\n SAME_FILE:1 \"two\"\n ONLY_A:0 \"a\"\n",
        )
        .expect("write first localisation file");
        fs::write(
            directory.join("b_l_english.yml"),
            "l_english:\n DUPLICATE:0 \"winner\"\n",
        )
        .expect("write second localisation file");

        let resolver = AssetResolver::new(vec![root.clone()]);
        let localisation =
            Localisation::load_english(&resolver).expect("load synthetic localisation");
        assert_eq!(localisation.get("DUPLICATE"), None);
        assert_eq!(
            localisation.entries.get("DUPLICATE").map(String::as_str),
            Some("winner")
        );
        assert_eq!(localisation.source_file_for_key("DUPLICATE"), None);
        assert_eq!(
            localisation.ambiguous_sources_for_key("DUPLICATE"),
            Some(
                [
                    "localisation/english/a_l_english.yml".to_string(),
                    "localisation/english/b_l_english.yml".to_string()
                ]
                .as_slice()
            )
        );
        assert_eq!(localisation.source_file_for_key("SAME_FILE"), None);
        assert_eq!(
            localisation.ambiguous_sources_for_key("SAME_FILE"),
            Some(["localisation/english/a_l_english.yml".to_string()].as_slice())
        );
        assert_eq!(
            localisation.source_file_for_key("ONLY_A"),
            Some("localisation/english/a_l_english.yml")
        );
        assert!(localisation.provenance_complete);
        assert_eq!(
            localisation.entries.len(),
            localisation.source_by_key.len() + localisation.ambiguous_sources_by_key.len()
        );
        fs::remove_dir_all(root).expect("remove localisation fixture");
    }

    #[test]
    fn detects_raw_localisation_keys() {
        assert!(is_localisation_key("MAP_MODE_CLICK_INFO"));
        assert!(is_localisation_key("$MAIN_MENU$"));
        assert!(!is_localisation_key("Main menu"));
    }

    #[test]
    fn duplicate_diagnostic_winner_uses_authenticated_case_sensitive_engine_order() {
        let root = std::env::temp_dir().join(format!(
            "game_ui_loc_engine_order_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory: PathBuf = root.join("localisation/english");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("Z.yml"),
            b"l_english:\n DUPLICATE:0 \"uppercase-first\"\n",
        )
        .unwrap();
        fs::write(
            directory.join("a.yml"),
            b"l_english:\n DUPLICATE:0 \"lowercase-last\"\n",
        )
        .unwrap();
        let localisation =
            Localisation::load_language(&AssetResolver::new(vec![root.clone()]), "l_english")
                .unwrap();
        assert_eq!(
            localisation.source_files,
            ["localisation/english/Z.yml", "localisation/english/a.yml"]
        );
        assert_eq!(
            localisation.entries.get("DUPLICATE").map(String::as_str),
            Some("lowercase-last")
        );
        assert_eq!(
            localisation.get("DUPLICATE"),
            None,
            "production lookup must still refuse an unproven duplicate winner"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
