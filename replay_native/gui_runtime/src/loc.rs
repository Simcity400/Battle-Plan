//! Minimal the game localisation layer.
//!
//! the game loc files (`localisation/english/*_l_english.yml`) are pseudo-YAML:
//! ```text
//! l_english:
//!  KEY: "value"
//!  KEY_WITH_VERSION:0 "value"      # the `:N` is a loc version, ignored
//!  REF: "see $OTHER_KEY$ here"      # $VAR$ left literal until M3 binding
//! ```
//! Files are UTF-8 with a leading BOM. We parse just enough to resolve a text
//! element's loc KEY to its localized STRING. A key that is not a real loc key
//! (e.g. a literal `"12/12/12"`) resolves to itself; a MISSING key resolves to
//! the raw key (never blank), per the no-blank contract.

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct Loc {
    map: HashMap<String, String>,
}

impl Loc {
    pub fn new() -> Loc {
        Loc {
            map: HashMap::new(),
        }
    }

    /// Number of loaded keys.
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Parse one loc file's text and fold its keys in. The FIRST definition of a
    /// key wins (matches the game's load-order-first-wins for a given file set; the
    /// caller controls file order — mod before vanilla).
    pub fn add_yml(&mut self, text: &str) {
        for raw in text.lines() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Skip the language header (`l_english:` with no value).
            if line.ends_with(':') && !line.contains('"') {
                continue;
            }
            if let Some((key, value)) = parse_line(line) {
                self.map.entry(key).or_insert(value);
            }
        }
    }

    /// Load every `*.yml` under a localisation directory (non-recursive is enough
    /// for `localisation/english/`, but we also descend one level).
    pub fn add_dir(&mut self, dir: &Path) -> std::io::Result<usize> {
        let mut n = 0;
        if !dir.is_dir() {
            return Ok(0);
        }
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d)? {
                let entry = entry?;
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().and_then(|e| e.to_str()) == Some("yml") {
                    if let Ok(text) = std::fs::read_to_string(&p) {
                        let before = self.map.len();
                        self.add_yml(&text);
                        n += self.map.len() - before;
                    }
                }
            }
        }
        Ok(n)
    }

    /// Resolve a `.gui` text token: if it is a known loc KEY, return its value;
    /// otherwise return the token verbatim (a literal label or a missing key —
    /// never blank). `$VAR$` substitution is deferred to M3, so any `$…$` in the
    /// resolved value is left literal.
    pub fn resolve(&self, key: &str) -> String {
        if let Some(v) = self.map.get(key) {
            return v.clone();
        }
        key.to_string()
    }
}

/// Parse a single `KEY[:N] "value"` line. Returns (key, unescaped value).
fn parse_line(line: &str) -> Option<(String, String)> {
    // key is up to the first ':'
    let colon = line.find(':')?;
    let key = line[..colon].trim().to_string();
    if key.is_empty() {
        return None;
    }
    let rest = &line[colon + 1..];
    // optional version number then the first quote
    let q0 = rest.find('"')?;
    let after = &rest[q0 + 1..];
    // value runs to the last quote on the line (values may contain escaped \")
    let q1 = after.rfind('"')?;
    let value = &after[..q1];
    Some((key, unescape(value)))
}

/// Minimal unescape: `\n` -> newline, `\"` -> quote. Leaves loc markup
/// (`$VAR$`, `§Y…§!`, `£icon`) untouched — the markup engine handles those.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('n') => {
                    out.push('\n');
                    chars.next();
                }
                Some('"') => {
                    out.push('"');
                    chars.next();
                }
                Some('\\') => {
                    out.push('\\');
                    chars.next();
                }
                _ => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_keys_and_resolves() {
        let mut loc = Loc::new();
        loc.add_yml("\u{feff}l_english:\n LOBBY: \"Lobby\"\n MAIN_MENU:0 \"Menu\"\n");
        assert_eq!(loc.resolve("LOBBY"), "Lobby");
        assert_eq!(loc.resolve("MAIN_MENU"), "Menu");
        // missing key -> raw key, never blank
        assert_eq!(loc.resolve("NOPE_KEY"), "NOPE_KEY");
        // a literal (not a key) -> itself
        assert_eq!(loc.resolve("12/12/12"), "12/12/12");
    }

    #[test]
    fn first_definition_wins() {
        let mut loc = Loc::new();
        loc.add_yml(" K: \"first\"\n");
        loc.add_yml(" K: \"second\"\n");
        assert_eq!(loc.resolve("K"), "first");
    }

    #[test]
    fn value_with_inner_quotes_and_var() {
        let mut loc = Loc::new();
        loc.add_yml(" Q: \"say \\\"hi\\\" $X$\"\n");
        assert_eq!(loc.resolve("Q"), "say \"hi\" $X$");
    }
}
