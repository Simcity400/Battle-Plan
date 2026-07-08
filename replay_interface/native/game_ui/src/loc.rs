use crate::assets::AssetResolver;
use crate::font::strip_color_codes;
use crate::paradox_text::decode_text;
use crate::Result;
use std::collections::BTreeMap;
use std::fs;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Localisation {
    pub entries: BTreeMap<String, String>,
    pub source_files: Vec<String>,
    pub parse_failures: Vec<String>,
}

impl Localisation {
    pub fn load_english(resolver: &AssetResolver) -> Result<Self> {
        let files = resolver.list_files_recursive("localisation/english", Some("yml"))?;
        let mut loc = Self::default();
        for file in files {
            match fs::read(&file.path) {
                Ok(bytes) => {
                    let text = decode_text(&bytes);
                    parse_english_yml(&text, &mut loc.entries);
                    loc.source_files.push(file.rel_path);
                }
                Err(err) => loc
                    .parse_failures
                    .push(format!("{}: {}", file.rel_path, err)),
            }
        }
        loc.source_files.sort();
        loc.parse_failures.sort();
        Ok(loc)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn parse_english_yml(text: &str, out: &mut BTreeMap<String, String>) {
    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() || line.eq_ignore_ascii_case("l_english:") {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let value = parse_localised_value(rest.trim());
        out.insert(key.to_string(), value);
    }
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

fn parse_localised_value(rest: &str) -> String {
    let rest = rest
        .trim_start()
        .trim_start_matches(|ch: char| ch.is_ascii_digit())
        .trim_start();
    if let Some(value) = parse_quoted(rest) {
        return value;
    }
    rest.trim().to_string()
}

fn parse_quoted(text: &str) -> Option<String> {
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
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_header_version_and_color_escapes() {
        let mut entries = BTreeMap::new();
        parse_english_yml(
            "l_english:\n MAIN_MENU:0 \"Main menu\"\n SAMPLE_TT:0 \"§YHeader§!\\nBody # not comment\" # comment\n",
            &mut entries,
        );
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
    fn detects_raw_localisation_keys() {
        assert!(is_localisation_key("MAP_MODE_CLICK_INFO"));
        assert!(is_localisation_key("$MAIN_MENU$"));
        assert!(!is_localisation_key("Main menu"));
    }
}
