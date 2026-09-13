//! Localised strings from the layered `localisation/*.yml` files.
//!
//! Values keep everything the game stores in them: the `§Y...§!` colour codes,
//! the `$VARIABLE$` and `[Scope.Function]` placeholders and the literal `\n`
//! escape. Only `\"` and `\\` are decoded, the same two escapes the Clausewitz
//! text parser decodes, so nothing else is silently rewritten here.

use std::{collections::BTreeMap, io};

use crate::{ResolvedLayers, content::normalize_relative};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Every localised string of one language, with later layers already applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Localisation {
    pub language: String,
    entries: BTreeMap<String, String>,
    /// `<file>:<line>` for each line that is neither blank, a comment, a
    /// language header nor a parsable entry. No shipped file produces one; a
    /// mod's broken line is recorded rather than failing the whole language.
    pub unparsed: Vec<String>,
}

impl Localisation {
    /// The localised text for a key, or `None` when no layer defines it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Number of distinct keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no layer defined any key of this language.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every key and value in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

/// One parsed file: its entries in file order and the lines it could not read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedFile {
    pub entries: Vec<(String, String)>,
    /// One-based line numbers that are neither blank, a comment, a language
    /// header nor a parsable entry.
    pub unparsed_lines: Vec<u32>,
}

/// Splits ` KEY:0 "text"` into its key and the text that follows the colon.
fn split_entry(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let (key, rest) = trimmed.split_once(':')?;
    if key.is_empty() || key.starts_with('#') || key.contains(char::is_whitespace) {
        return None;
    }
    // The version number after the colon is optional and carries no meaning
    // for a reader; the game only uses it to flag stale translations.
    let rest = rest.trim_start_matches(|character: char| character.is_ascii_digit());
    Some((key, rest.trim_start_matches([' ', '\t'])))
}

/// Reads the quoted value, decoding `\"` and `\\` and keeping every other byte.
///
/// The value ends at the first closing quote after which nothing but a comment
/// remains, because shipped strings contain bare quotes (`Declares itself
/// "Protector of the North Sea Passage"`) and shipped comments contain them too.
fn quoted(rest: &str) -> Option<String> {
    let body = rest.strip_prefix('"')?;
    let mut text = String::new();
    let mut characters = body.char_indices();
    while let Some((index, character)) = characters.next() {
        match character {
            '\\' => match characters.next() {
                Some((_, escaped @ ('"' | '\\'))) => text.push(escaped),
                Some((_, other)) => {
                    text.push('\\');
                    text.push(other);
                }
                None => text.push('\\'),
            },
            '"' => {
                let tail = body[index + 1..].trim_start();
                if tail.is_empty() || tail.starts_with('#') {
                    return Some(text);
                }
                text.push('"');
            }
            other => text.push(other),
        }
    }
    None
}

/// True for a bare `l_<language>:` header line, which may switch language.
fn header_language(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let (language, rest) = trimmed.strip_prefix("l_")?.split_once(':')?;
    if language.is_empty()
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        || !(rest.trim().is_empty() || rest.trim_start().starts_with('#'))
    {
        return None;
    }
    Some(language)
}

/// Parses one `.yml` file, keeping only the entries under `l_<language>:`.
///
/// A file may carry several language headers; `localisation/languages.yml`
/// does. Keys before any header belong to no language and are unparsed.
pub fn parse_file(text: &str, language: &str) -> ParsedFile {
    let mut parsed = ParsedFile::default();
    let mut current = None;
    for (index, line) in text.lines().enumerate() {
        let number = index as u32 + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(found) = header_language(line) {
            current = Some(found.to_owned());
            continue;
        }
        let Some((key, rest)) = split_entry(line) else {
            parsed.unparsed_lines.push(number);
            continue;
        };
        let Some(value) = quoted(rest) else {
            parsed.unparsed_lines.push(number);
            continue;
        };
        if current.as_deref() == Some(language) {
            parsed.entries.push((key.to_owned(), value));
        }
    }
    parsed
}

/// Sorting rank of one localisation file: `replace/` beats the base folder.
fn replace_rank(path: &str) -> u8 {
    u8::from(
        path.split('/')
            .any(|part| part.eq_ignore_ascii_case("replace")),
    )
}

/// Reads every localisation file of one language across the resolved layers.
///
/// Precedence, highest last: the layer's load order, then `replace/` over the
/// base folder within a layer, then the path in byte order. Within one file the
/// last definition of a key wins. Only the layer order is established; the
/// `replace/` and alphabetical rules are this reader's choice and research R2
/// has not yet confirmed them against the engine.
pub fn read(layers: &ResolvedLayers, language: &str) -> io::Result<Localisation> {
    if language.is_empty()
        || !language
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return Err(invalid(format!("unsupported language name {language:?}")));
    }
    let mut files = Vec::new();
    for path in layers.paths() {
        let lower = path.to_ascii_lowercase();
        if !lower.starts_with("localisation/") || !lower.ends_with(".yml") {
            continue;
        }
        let layer = layers.layer_index(path).unwrap_or(0);
        files.push((layer, replace_rank(&lower), path.to_owned()));
    }
    files.sort();
    let mut localisation = Localisation {
        language: language.to_owned(),
        ..Localisation::default()
    };
    for (_, _, path) in files {
        let bytes = layers.read(&normalize_relative(&path)?)?;
        let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
            .map_err(|_| invalid(format!("{path} is not UTF-8")))?;
        let parsed = parse_file(text, language);
        for (key, value) in parsed.entries {
            localisation.entries.insert(key, value);
        }
        for line in parsed.unparsed_lines {
            localisation.unparsed.push(format!("{path}:{line}"));
        }
    }
    Ok(localisation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_keep_colour_codes_variables_and_the_literal_newline_escape() {
        let parsed = parse_file(
            "l_english:\n GER_fascism:0 \"§YGerman§! Reich\"\n \
             A: \"$OVERLORDADJ$ Zone\\nsecond\"\n B:12 \"x\"\nNOSPACE: \"y\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            vec![
                ("GER_fascism".into(), "§YGerman§! Reich".to_owned()),
                ("A".into(), "$OVERLORDADJ$ Zone\\nsecond".to_owned()),
                ("B".into(), "x".to_owned()),
                ("NOSPACE".into(), "y".to_owned()),
            ]
        );
        assert!(parsed.unparsed_lines.is_empty());
    }

    #[test]
    fn a_value_may_hold_bare_quotes_and_a_comment_may_hold_them_too() {
        let parsed = parse_file(
            "l_english:\n \
             A: \"Declares itself \"Protector of the North Sea\"\"\n \
             B: \"Elect King Anastasia.\" #monarchs are called \"King\"\n \
             C: \"#\"\n D: \"say \\\"hi\\\" now\"\n",
            "english",
        );
        assert_eq!(
            parsed.entries,
            vec![
                (
                    "A".into(),
                    "Declares itself \"Protector of the North Sea\"".to_owned()
                ),
                ("B".into(), "Elect King Anastasia.".to_owned()),
                ("C".into(), "#".to_owned()),
                ("D".into(), "say \"hi\" now".to_owned()),
            ]
        );
    }

    #[test]
    fn only_the_requested_language_is_kept_across_several_headers() {
        let text = "# leading comment\nl_english:\n A: \"one\"\nl_french:\n A: \"un\"\n\
                    l_english:\n B: \"two\"\n";
        assert_eq!(
            parse_file(text, "english").entries,
            vec![
                ("A".into(), "one".to_owned()),
                ("B".into(), "two".to_owned())
            ]
        );
        assert_eq!(
            parse_file(text, "french").entries,
            vec![("A".into(), "un".to_owned())]
        );
        assert!(parse_file(text, "german").entries.is_empty());
    }

    #[test]
    fn unreadable_lines_are_counted_instead_of_failing_the_language() {
        let parsed = parse_file(
            "l_english:\n A: \"unterminated\n continued.\"\n B: \"good\"\n C: bare\n",
            "english",
        );
        assert_eq!(parsed.entries, vec![("B".into(), "good".to_owned())]);
        assert_eq!(parsed.unparsed_lines, vec![2, 3, 5]);
        // Keys before any header belong to no language.
        let orphan = parse_file(" A: \"x\"\nl_english:\n B: \"y\"\n", "english");
        assert_eq!(orphan.entries, vec![("B".into(), "y".to_owned())]);
    }

    #[test]
    fn replace_outranks_the_base_folder_and_paths_break_ties_in_order() {
        assert_eq!(replace_rank("localisation/english/a_l_english.yml"), 0);
        assert_eq!(replace_rank("localisation/replace/english/a.yml"), 1);
        assert_eq!(replace_rank("localisation/Replace/a.yml"), 1);
        assert_eq!(replace_rank("localisation/replaced/a.yml"), 0);
        let mut files = [
            (1_usize, 0_u8, "localisation/english/b.yml".to_owned()),
            (0, 1, "localisation/replace/english/a.yml".to_owned()),
            (1, 1, "localisation/replace/english/a.yml".to_owned()),
            (1, 0, "localisation/english/a.yml".to_owned()),
        ];
        files.sort();
        assert_eq!(
            files.iter().map(|file| file.2.as_str()).collect::<Vec<_>>(),
            vec![
                "localisation/replace/english/a.yml",
                "localisation/english/a.yml",
                "localisation/english/b.yml",
                "localisation/replace/english/a.yml",
            ]
        );
    }

    #[test]
    fn a_later_definition_wins_and_lookups_report_missing_keys() {
        let mut localisation = Localisation {
            language: "english".into(),
            ..Localisation::default()
        };
        for (key, value) in parse_file("l_english:\n A: \"one\"\n A: \"two\"\n", "english").entries
        {
            localisation.entries.insert(key, value);
        }
        assert_eq!(localisation.get("A"), Some("two"));
        assert_eq!(localisation.get("B"), None);
        assert_eq!(localisation.len(), 1);
        assert!(!localisation.is_empty());
        assert_eq!(localisation.iter().collect::<Vec<_>>(), vec![("A", "two")]);
    }

    #[test]
    fn an_unusable_language_name_is_refused_before_any_file_is_read() {
        let layers = ResolvedLayers::default();
        for language in ["", "English", "en-gb", "../x"] {
            assert!(read(&layers, language).is_err(), "{language}");
        }
        assert!(read(&layers, "english").unwrap().is_empty());
    }
}
