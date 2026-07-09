//! the game inline text-markup law (the `§` colour / `£` icon / `$var$` / `[data]`
//! escape codes embedded in localisation strings).
//!
//! Source of truth is FILE-derived, not fitted:
//!
//! * The `§<code>` colour table is the shipped `interface/core.gfx`
//!   `bitmapfonts = { textcolors = { ... } }` block (see `core.gfx:796`,
//!   comment *"Global text colors used in the entire game, for use in loc files
//!   ex: \"§G Text Text§!\""*). Codes are CASE-SENSITIVE (`b` = black vs
//!   `B` = blue, `g` = grey vs `G` = green). `§!` resets to the text box's
//!   base colour. This replaces the previous hard-coded (and wrong) colour map
//!   in `font.rs::colorize_hoi4_text`.
//! * `£<key>` / `£<key>|<frame>` is an inline sprite/icon token (resolved to a
//!   `GFX_` sprite by the caller). It is tokenised out of the text run so it
//!   never leaks as literal glyphs and never corrupts measured text width.
//! * `$<key>$` is a variable / nested-localisation substitution.
//! * `[<expr>]` is an engine-evaluated scripted-localisation data function
//!   (computed-not-stored in a read-only replay; the caller badges it, never
//!   fabricates a value).
//!
//! The tokeniser here does the lexical split only. Resolving `$…$`/`[…]` to a
//! value is the caller's job (loc lookup where resolvable, else an explicit
//! unavailable badge) — this module never invents a value.

use crate::assets::AssetResolver;
use crate::paradox_text::{block_items, parse_file, scalar_u32, Value};
use crate::Result;
use std::collections::BTreeMap;

/// `§` U+00A7 SECTION SIGN — the colour escape lead byte.
pub const SECTION: char = '\u{00a7}';
/// `£` U+00A3 POUND SIGN — the inline-icon lead byte.
pub const ICON: char = '\u{00a3}';
/// `Â` U+00C2 — the mojibake byte that precedes `§` when a UTF-8 `0xC2 0xA7`
/// section sign is decoded as Latin-1. Tolerated so double-decoded strings
/// still colour correctly.
const MOJIBAKE_LEAD: char = '\u{00c2}';

/// The `§<code>` → RGB table, parsed from `interface/core.gfx`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextColorTable {
    colors: BTreeMap<char, [u8; 3]>,
}

impl TextColorTable {
    /// Parse the `bitmapfonts.textcolors` block out of an already-parsed
    /// `core.gfx` root value. Case is preserved (the Clausewitz parser keeps
    /// raw key text), so `b`/`B` and `g`/`G` stay distinct.
    pub fn from_core_gfx_value(root: &Value) -> Self {
        let mut colors = BTreeMap::new();
        let Some(bitmapfonts) = root.first_value_ci("bitmapfonts") else {
            return Self { colors };
        };
        let Some(textcolors) = bitmapfonts.first_value_ci("textcolors") else {
            return Self { colors };
        };
        for entry in textcolors.fields() {
            let Some(key) = entry.key.as_deref() else {
                continue;
            };
            let mut chars = key.chars();
            let (Some(code), None) = (chars.next(), chars.next()) else {
                continue; // colour codes are single characters
            };
            let components: Vec<u8> = block_items(&entry.value)
                .filter_map(|item| scalar_u32(item))
                .map(|value| value.min(255) as u8)
                .collect();
            if let [r, g, b] = components[..] {
                colors.insert(code, [r, g, b]);
            }
        }
        Self { colors }
    }

    /// Load + parse `interface/core.gfx` through the mod-aware resolver.
    pub fn load(resolver: &AssetResolver) -> Result<Self> {
        let resolved = resolver.resolve_file("interface/core.gfx")?;
        let doc = parse_file(&resolved.path)?;
        Ok(Self::from_core_gfx_value(&doc.root))
    }

    /// RGB for a colour code, or `None` if the code is not in the shipped
    /// table (unknown codes leave the current colour unchanged, matching the
    /// engine which simply consumes the escape).
    pub fn get(&self, code: char) -> Option<[u8; 3]> {
        self.colors.get(&code).copied()
    }

    pub fn len(&self) -> usize {
        self.colors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }
}

/// One lexical token of a marked-up localisation string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// A run of literal text carrying the active colour (`None` = base colour).
    Text { text: String, color: Option<[u8; 3]> },
    /// `£key` / `£key|frame` inline icon.
    Icon { key: String, frame: Option<u32> },
    /// `$key$` variable / nested-loc substitution request.
    Variable { key: String },
    /// `[expr]` engine data-function (computed-not-stored).
    DataFunction { expr: String },
}

/// Tokenise a marked-up string. `table` resolves `§` codes; pass an empty
/// table to strip colour without tinting. Adjacent text with the same colour
/// is merged into one run.
pub fn parse(text: &str, table: &TextColorTable) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current: Option<[u8; 3]> = None;
    let mut buf = String::new();
    let mut chars = text.chars().peekable();

    macro_rules! flush {
        () => {
            if !buf.is_empty() {
                tokens.push(Token::Text {
                    text: std::mem::take(&mut buf),
                    color: current,
                });
            }
        };
    }

    while let Some(ch) = chars.next() {
        // Section sign, optionally preceded by the Â mojibake byte.
        if ch == SECTION || (ch == MOJIBAKE_LEAD && chars.peek() == Some(&SECTION)) {
            if ch == MOJIBAKE_LEAD {
                chars.next(); // consume the real §
            }
            flush!();
            // `§[expr]` — a scripted-localisation data function that evaluates
            // to a colour at runtime. Unresolvable in a read-only replay, so we
            // consume the whole bracket and leave the colour unchanged rather
            // than fabricate a tint.
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                }
                continue;
            }
            let Some(code) = chars.next() else { break };
            if code == '!' {
                current = None;
            } else if let Some(rgb) = table.get(code) {
                current = Some(rgb);
            }
            // unknown single-char code: consumed, colour unchanged
            continue;
        }

        if ch == ICON {
            flush!();
            let mut key = String::new();
            while let Some(&next) = chars.peek() {
                if next.is_ascii_alphanumeric() || next == '_' {
                    key.push(next);
                    chars.next();
                } else {
                    break;
                }
            }
            let mut frame = None;
            if chars.peek() == Some(&'|') {
                chars.next();
                let mut digits = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_digit() {
                        digits.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                frame = digits.parse().ok();
            }
            if !key.is_empty() {
                tokens.push(Token::Icon { key, frame });
            }
            continue;
        }

        if ch == '$' {
            let mut key = String::new();
            let mut closed = false;
            while let Some(next) = chars.next() {
                if next == '$' {
                    closed = true;
                    break;
                }
                key.push(next);
            }
            if closed {
                flush!();
                tokens.push(Token::Variable { key });
            } else {
                // Unterminated: treat the `$` literally.
                buf.push('$');
                buf.push_str(&key);
            }
            continue;
        }

        if ch == '[' {
            let mut expr = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == ']' {
                    closed = true;
                    break;
                }
                expr.push(next);
            }
            if closed {
                flush!();
                tokens.push(Token::DataFunction { expr });
            } else {
                buf.push('[');
                buf.push_str(&expr);
            }
            continue;
        }

        buf.push(ch);
    }
    flush!();
    tokens
}

/// Per-character colour vector + the visible (icon/variable/data stripped) text,
/// for the glyph renderer. `Variable`/`DataFunction` tokens are dropped from the
/// visible text (the caller substitutes/badges them before this stage if it has
/// a value); `Icon` tokens are dropped (drawn separately as sprites).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorRuns {
    pub text: String,
    pub colors: Vec<[u8; 3]>,
}

/// Flatten a token stream to visible glyph text + per-glyph colour, given the
/// text box's base colour for the `None`/reset state.
pub fn color_runs(tokens: &[Token], base: [u8; 3]) -> ColorRuns {
    let mut out = ColorRuns::default();
    for token in tokens {
        if let Token::Text { text, color } = token {
            let rgb = color.unwrap_or(base);
            for ch in text.chars() {
                out.text.push(ch);
                out.colors.push(rgb);
            }
        }
    }
    out
}

/// Remove ONLY `§` colour sequences (`§X`, `§!`, `§[expr]`, and the `Â§`
/// mojibake), leaving `£`/`$…$`/`[…]` intact. This is what callers that still
/// need the variable/icon delimiters (e.g. localisation-key detection) want.
pub fn strip_section_codes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == SECTION || (ch == MOJIBAKE_LEAD && chars.peek() == Some(&SECTION)) {
            if ch == MOJIBAKE_LEAD {
                chars.next();
            }
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                }
            } else {
                chars.next(); // the single code char (incl. '!')
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// Remove all markup, returning plain visible text (no tint). Icons, variables,
/// and data functions are dropped. Uses an empty table (colours are irrelevant
/// when stripping).
pub fn strip(text: &str) -> String {
    let empty = TextColorTable::default();
    parse(text, &empty)
        .into_iter()
        .filter_map(|token| match token {
            Token::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;

    fn table() -> TextColorTable {
        // Mirror of the shipped interface/core.gfx textcolors block.
        let doc = parse_str(
            r#"
            bitmapfonts = {
              textcolors = {
                G = { 0 159 3 }
                R = { 255 50 50 }
                Y = { 255 189 0 }
                b = { 0 0 0 }
                B = { 0 0 255 }
                g = { 176 176 176 }
              }
            }
            "#,
        );
        TextColorTable::from_core_gfx_value(&doc.root)
    }

    #[test]
    fn color_codes_are_case_sensitive() {
        let table = table();
        assert_eq!(table.get('G'), Some([0, 159, 3]));
        assert_eq!(table.get('g'), Some([176, 176, 176]));
        assert_eq!(table.get('b'), Some([0, 0, 0]));
        assert_eq!(table.get('B'), Some([0, 0, 255]));
    }

    #[test]
    fn parses_colour_runs_and_reset() {
        let table = table();
        let tokens = parse("\u{a7}GFull\u{a7}! 74%", &table);
        assert_eq!(
            tokens,
            vec![
                Token::Text {
                    text: "Full".to_string(),
                    color: Some([0, 159, 3])
                },
                Token::Text {
                    text: " 74%".to_string(),
                    color: None
                },
            ]
        );
    }

    #[test]
    fn colour_runs_flatten_per_char() {
        let table = table();
        let tokens = parse("\u{a7}RAB\u{a7}!C", &table);
        let runs = color_runs(&tokens, [221, 214, 190]);
        assert_eq!(runs.text, "ABC");
        assert_eq!(
            runs.colors,
            vec![[255, 50, 50], [255, 50, 50], [221, 214, 190]]
        );
    }

    #[test]
    fn tolerates_mojibake_section_sign() {
        let table = table();
        let tokens = parse("\u{c2}\u{a7}Gx", &table);
        assert_eq!(
            tokens,
            vec![Token::Text {
                text: "x".to_string(),
                color: Some([0, 159, 3])
            }]
        );
    }

    #[test]
    fn tokenises_icon_with_optional_frame() {
        let table = table();
        assert_eq!(
            parse("a\u{a3}political_power b", &table),
            vec![
                Token::Text { text: "a".to_string(), color: None },
                Token::Icon { key: "political_power".to_string(), frame: None },
                Token::Text { text: " b".to_string(), color: None },
            ]
        );
        assert_eq!(
            parse("\u{a3}GFX_flag|3", &table),
            vec![Token::Icon { key: "GFX_flag".to_string(), frame: Some(3) }]
        );
    }

    #[test]
    fn tokenises_variable_and_data_function() {
        let table = table();
        assert_eq!(
            parse("Owner: $KEY$ [GetName]", &table),
            vec![
                Token::Text { text: "Owner: ".to_string(), color: None },
                Token::Variable { key: "KEY".to_string() },
                Token::Text { text: " ".to_string(), color: None },
                Token::DataFunction { expr: "GetName".to_string() },
            ]
        );
    }

    #[test]
    fn dynamic_bracket_colour_is_consumed_not_leaked() {
        let table = table();
        // §[expr] evaluates to a colour at runtime; unresolvable here, so it
        // must vanish from the visible text and leave the colour unchanged.
        let tokens = parse("a\u{a7}[GetColorTier1]b\u{a7}!c", &table);
        assert!(tokens.iter().all(|t| matches!(t, Token::Text { color: None, .. })));
        let runs = color_runs(&tokens, [10, 20, 30]);
        assert_eq!(runs.text, "abc");
        assert_eq!(runs.colors, vec![[10, 20, 30]; 3]);
    }

    #[test]
    fn strip_removes_all_markup() {
        assert_eq!(strip("\u{a7}Y12\u{a7}! / \u{a7}R0\u{a7}!"), "12 / 0");
        assert_eq!(strip("x\u{a3}icon$V$[f]y"), "xy");
    }

    #[test]
    fn unterminated_dollar_is_literal() {
        let table = table();
        assert_eq!(
            parse("price $5", &table),
            vec![Token::Text { text: "price $5".to_string(), color: None }]
        );
    }
}
