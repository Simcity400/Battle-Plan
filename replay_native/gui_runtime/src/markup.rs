//! MARKUP EXPANSION + GLYPH-MEASURED WORD WRAP (M5 foundation 3b).
//!
//! the game localized strings carry three markup forms that must be *expanded* before
//! the text is measured and laid out — measuring the raw markup would be wrong by
//! the width of every control code and variable name:
//!
//!   * `$VAR$`     — a variable substitution (`$STATE$` -> "Brandenburg"). The
//!                   value comes from a caller-supplied table; an unknown var is
//!                   left as its literal name (never blank) per the loc contract.
//!   * `§Ccolour`  — a colour-control run: `§Y` opens yellow, `§!` closes back to
//!                   the default. The control codes occupy NO horizontal space, so
//!                   they are stripped from the measured/displayed glyph stream but
//!                   recorded as per-span COLOURS so the draw pass can tint runs.
//!   * `£icon`     — an inline icon token (`£GFX_state` or `£oil`). It renders as a
//!                   sprite roughly one line tall; for measurement we reserve a
//!                   fixed icon advance (the glyph stream gets a placeholder).
//!
//! The critical Codex requirement (VERTICAL_SLICE_SPEC §3.1): **wrap is computed
//! from glyph metrics AFTER expansion** — i.e. the wrapper measures the EXPANDED,
//! markup-stripped text with the real BMFont `measure()`, never the source string.
//! [`expand`] does the expansion; [`wrap`] does the glyph-measured wrap; the two
//! are deliberately separate so a test can prove the order.

use std::collections::HashMap;

use assets::BmFont;

/// One contiguous run of display text that shares a single colour. The `text` is
/// already markup-stripped (no `§`/`£`/`$`), ready to measure and draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    /// Markup-stripped, variable-substituted display text.
    pub text: String,
    /// RGBA tint for this run (the default text colour, or a `§C` colour).
    pub color: [f32; 4],
    /// If this span is an inline `£icon`, the GFX sprite name; else None. An icon
    /// span's `text` is a single placeholder space used only for measurement.
    pub icon: Option<String>,
}

/// The fully expanded text: an ordered list of coloured spans + the concatenated
/// plain display string (markup-stripped) for whole-string measurement.
#[derive(Debug, Clone, Default)]
pub struct Expanded {
    pub spans: Vec<Span>,
    /// The concatenation of every span's `text` — the EXACTLY measurable display
    /// string (post-expansion). This is what [`wrap`] measures, never the source.
    pub plain: String,
}

/// The the game `§C` colour palette (the subset that appears in UI strings). Unknown
/// codes fall back to the default colour so an unrecognised run never vanishes.
fn color_code(c: char, default: [f32; 4]) -> [f32; 4] {
    match c {
        'Y' => [0.93, 0.82, 0.27, 1.0], // yellow (highlight)
        'G' => [0.40, 0.78, 0.36, 1.0], // green (good)
        'R' => [0.86, 0.27, 0.24, 1.0], // red (bad)
        'B' => [0.36, 0.55, 0.90, 1.0], // blue
        'W' => [0.96, 0.96, 0.94, 1.0], // white
        'O' => [0.92, 0.58, 0.22, 1.0], // orange
        'T' => [0.45, 0.78, 0.85, 1.0], // teal
        'b' => [0.16, 0.16, 0.18, 1.0], // black-ish
        '!' => default,                 // reset to default
        _ => default,
    }
}

/// The pixel advance reserved for an inline `£icon` during measurement (icons are
/// drawn ~ one line tall; the game reserves roughly a square per icon).
pub const ICON_ADVANCE: f32 = 18.0;

/// EXPAND a localized source string: substitute `$VAR$`, apply `§C` colour runs,
/// and pull out `£icon` tokens — producing measurable, markup-stripped spans.
///
/// `vars` maps a bare variable name (`STATE`, no `$`) to its substituted value;
/// an unknown variable is left as its literal name. `default` is the base colour.
pub fn expand(src: &str, vars: &HashMap<String, String>, default: [f32; 4]) -> Expanded {
    // 1) variable substitution first (so a $VAR$ value can itself be plain text).
    let substituted = substitute_vars(src, vars);

    // 2) walk the substituted text, splitting on §C colour codes and £icon tokens.
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut cur = String::new();
    let mut color = default;

    let chars: Vec<char> = substituted.chars().collect();
    let mut i = 0;
    let push_run = |cur: &mut String, spans: &mut Vec<Span>, color: [f32; 4]| {
        if !cur.is_empty() {
            spans.push(Span {
                text: std::mem::take(cur),
                color,
                icon: None,
            });
        }
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '§' && i + 1 < chars.len() {
            // colour control: close the current run, switch colour, skip 2 chars.
            push_run(&mut cur, &mut spans, color);
            color = color_code(chars[i + 1], default);
            i += 2;
            continue;
        }
        if c == '£' {
            // inline icon token: read the icon name up to a delimiter.
            push_run(&mut cur, &mut spans, color);
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let name: String = chars[i + 1..j].iter().collect();
            // a single space stands in for the icon in the measurable stream.
            plain.push(' ');
            spans.push(Span {
                text: " ".to_string(),
                color,
                icon: Some(name),
            });
            i = j;
            continue;
        }
        cur.push(c);
        plain.push(c);
        i += 1;
    }
    push_run(&mut cur, &mut spans, color);

    Expanded { spans, plain }
}

/// Substitute `$VAR$` tokens. `$$` is a literal `$`. Unknown vars stay literal.
fn substitute_vars(src: &str, vars: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(src.len());
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() && chars[i + 1] == '$' {
                out.push('$');
                i += 2;
                continue;
            }
            // find the closing '$'
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == '$') {
                let name: String = chars[i + 1..i + 1 + close].iter().collect();
                // the game vars may carry a |formatter suffix ($VAR|2$); strip it.
                let bare = name.split('|').next().unwrap_or(&name);
                match vars.get(bare) {
                    Some(v) => out.push_str(v),
                    None => {
                        // unknown var: keep the literal name (never blank).
                        out.push_str(bare);
                    }
                }
                i = i + 1 + close + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// One wrapped display line: its spans (coloured runs / icons), already fitted to
/// the wrap width, plus the measured pixel width of the line.
#[derive(Debug, Clone, Default)]
pub struct WrappedLine {
    pub spans: Vec<Span>,
    pub width: f32,
}

/// The result of wrapping: the lines + the line height to advance between them.
#[derive(Debug, Clone, Default)]
pub struct Wrapped {
    pub lines: Vec<WrappedLine>,
    pub line_height: f32,
}

/// Measure the display width of a markup-stripped fragment with the real BMFont,
/// reserving [`ICON_ADVANCE`] for any icon placeholder. This is THE measurement
/// the wrap is built on — glyph metrics, post-expansion.
fn measure_fragment(font: &BmFont, frag: &str, is_icon: bool) -> f32 {
    if is_icon {
        ICON_ADVANCE
    } else {
        font.measure(frag) as f32
    }
}

/// WORD-WRAP the expanded text to `max_width` pixels, measuring every candidate
/// line with the BMFont's glyph metrics (AFTER expansion). Words are split on
/// ASCII spaces; a single word longer than `max_width` is hard-broken by glyphs.
/// Colour spans are preserved across wrap points, and a separator space is emitted
/// ONLY where the SOURCE had one (so a colour boundary mid-word, `a§Yb§!`, measures
/// as `ab`, not `a b` — Codex (b)).
///
/// Width guarantees (Codex (b), honest scope): for the normal case (each token and
/// the inter-word space fit the budget) every produced line is `<= max_width`. Two
/// DEGENERATE inputs are kept ON THEIR OWN LINE rather than dropped or looped, so
/// they may exceed the budget: (1) a single glyph whose advance alone is wider than
/// `max_width`; (2) an icon when `max_width < ICON_ADVANCE`. These are pathological
/// (a panel column is always many glyphs wide) and the alternative (dropping the
/// glyph/icon) would be a worse, lossy fabrication.
///
/// This is the Codex-required "wrap computed from glyph metrics AFTER expansion":
/// the input is an [`Expanded`] (already markup-stripped + var-substituted), and
/// every width here comes from [`BmFont::measure`], never from a raw char count.
pub fn wrap(exp: &Expanded, font: &BmFont, max_width: f32) -> Wrapped {
    let line_height = font.line_height.max(1) as f32;
    // Flatten the spans into a stream of (word, color, icon) tokens. We split each
    // text span on ASCII spaces, keeping per-word colour; icon spans are atomic.
    #[derive(Clone)]
    struct Tok {
        text: String,
        color: [f32; 4],
        icon: Option<String>,
        /// trailing space after this token in the source (a wrap opportunity).
        space_after: bool,
    }
    let mut toks: Vec<Tok> = Vec::new();
    for sp in &exp.spans {
        if sp.icon.is_some() {
            toks.push(Tok {
                text: sp.text.clone(),
                color: sp.color,
                icon: sp.icon.clone(),
                space_after: false,
            });
            continue;
        }
        // split on spaces, recording where a space was (a wrap point).
        let mut rest = sp.text.as_str();
        while !rest.is_empty() {
            match rest.find(' ') {
                Some(p) => {
                    let word = &rest[..p];
                    if !word.is_empty() {
                        toks.push(Tok {
                            text: word.to_string(),
                            color: sp.color,
                            icon: None,
                            space_after: true,
                        });
                    } else if let Some(last) = toks.last_mut() {
                        last.space_after = true;
                    }
                    rest = &rest[p + 1..];
                }
                None => {
                    toks.push(Tok {
                        text: rest.to_string(),
                        color: sp.color,
                        icon: None,
                        space_after: false,
                    });
                    rest = "";
                }
            }
        }
    }

    let space_w = font.measure(" ") as f32;
    let mut lines: Vec<WrappedLine> = Vec::new();
    let mut cur: Vec<Span> = Vec::new();
    let mut cur_w = 0.0f32;

    let flush = |cur: &mut Vec<Span>, cur_w: &mut f32, lines: &mut Vec<WrappedLine>| {
        if !cur.is_empty() {
            lines.push(WrappedLine {
                spans: std::mem::take(cur),
                width: *cur_w,
            });
            *cur_w = 0.0;
        }
    };

    // `prev_space` = did the SOURCE have a space between the previous token and
    // this one? Only then do we emit a separator (Codex (b): the synthetic-space
    // bug — `a§Yb§!` must measure/draw as `ab`, not `a b`. We track the previous
    // token's `space_after` instead of always inserting a space when `cur` is
    // non-empty).
    let mut prev_space = false;
    for tok in &toks {
        let tok_w = measure_fragment(font, &tok.text, tok.icon.is_some());
        // the separator only exists when the source had a space here.
        let sep = if !cur.is_empty() && prev_space {
            space_w
        } else {
            0.0
        };
        if !cur.is_empty() && cur_w + sep + tok_w > max_width {
            // wrap before this token (drop the trailing separator at the break).
            flush(&mut cur, &mut cur_w, &mut lines);
        }
        // a single token wider than the line: hard-break it by glyphs (text only).
        if tok.icon.is_none() && tok_w > max_width && cur.is_empty() {
            let mut piece = String::new();
            let mut piece_w = 0.0f32;
            for ch in tok.text.chars() {
                let cw = font.measure(&ch.to_string()) as f32;
                // emit the piece before a glyph that would overflow — UNLESS the
                // piece is empty (a single glyph wider than the line: keep it on
                // its own line rather than loop forever / drop it).
                if piece_w + cw > max_width && !piece.is_empty() {
                    lines.push(WrappedLine {
                        spans: vec![Span {
                            text: std::mem::take(&mut piece),
                            color: tok.color,
                            icon: None,
                        }],
                        width: piece_w,
                    });
                    piece_w = 0.0;
                }
                piece.push(ch);
                piece_w += cw;
            }
            if !piece.is_empty() {
                cur.push(Span {
                    text: piece,
                    color: tok.color,
                    icon: None,
                });
                cur_w += piece_w;
            }
        } else {
            // emit a separating space ONLY when the source had one (prev_space).
            if !cur.is_empty() && prev_space {
                if let Some(last) = cur.last_mut() {
                    if last.icon.is_none() && last.color == tok.color {
                        last.text.push(' ');
                    } else {
                        cur.push(Span {
                            text: " ".to_string(),
                            color: tok.color,
                            icon: None,
                        });
                    }
                }
                cur_w += space_w;
            }
            cur.push(Span {
                text: tok.text.clone(),
                color: tok.color,
                icon: tok.icon.clone(),
            });
            cur_w += tok_w;
        }
        prev_space = tok.space_after;
    }
    flush(&mut cur, &mut cur_w, &mut lines);

    Wrapped { lines, line_height }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_font() -> BmFont {
        // a deterministic monospace-ish test font: every printable glyph advances
        // 10px, space advances 6px, line height 16. Built directly (no atlas).
        let mut glyphs = std::collections::HashMap::new();
        for cp in 32u32..=126 {
            let adv = if cp == 32 { 6 } else { 10 };
            glyphs.insert(
                cp,
                assets::bmfont::Glyph {
                    id: cp,
                    page: 0,
                    x: 0,
                    y: 0,
                    width: if cp == 32 { 0 } else { 8 },
                    height: 10,
                    xoffset: 0,
                    yoffset: 0,
                    xadvance: adv,
                    chnl: 15,
                },
            );
        }
        BmFont {
            face: "test".into(),
            size: 14,
            line_height: 16,
            base: 12,
            scale_w: 256,
            scale_h: 256,
            page_files: Default::default(),
            page_paths: Default::default(),
            glyphs,
            kernings: Default::default(),
        }
    }

    #[test]
    fn expand_substitutes_vars_and_strips_colour() {
        let mut vars = HashMap::new();
        vars.insert("STATE".to_string(), "Brandenburg".to_string());
        let exp = expand("Capital of §Y$STATE$§! region", &vars, [1.0, 1.0, 1.0, 1.0]);
        // the plain (measurable) text has NO markup and the var is substituted.
        assert_eq!(exp.plain, "Capital of Brandenburg region");
        // there is a yellow span carrying exactly "Brandenburg".
        let yellow = exp
            .spans
            .iter()
            .find(|s| s.text == "Brandenburg")
            .expect("brandenburg span");
        assert_eq!(yellow.color, [0.93, 0.82, 0.27, 1.0]);
    }

    #[test]
    fn unknown_var_is_left_literal_never_blank() {
        let exp = expand("see $MISSINGVAR$ here", &HashMap::new(), [1.0; 4]);
        assert_eq!(exp.plain, "see MISSINGVAR here");
    }

    #[test]
    fn icon_token_becomes_icon_span_with_placeholder() {
        let exp = expand("oil £oil here", &HashMap::new(), [1.0; 4]);
        let icon = exp.spans.iter().find(|s| s.icon.is_some()).unwrap();
        assert_eq!(icon.icon.as_deref(), Some("oil"));
        // the measurable plain stream is markup-stripped (no £) and reserves a
        // single placeholder space for the icon glyph.
        assert!(!exp.plain.contains('£'));
        assert!(exp.plain.starts_with("oil"));
        assert!(exp.plain.trim_end().ends_with("here"));
    }

    #[test]
    fn colour_boundary_with_no_source_space_does_not_insert_a_space() {
        // Codex (b): `a§Yb§!c` is the single word "abc" with a yellow 'b'. The wrap
        // must NOT synthesize separators across the colour boundaries (no source
        // space) — the line width must equal measure("abc"), not measure("a b c").
        let font = sample_font();
        let exp = expand("a§Yb§!c", &HashMap::new(), [1.0; 4]);
        let wrapped = wrap(&exp, &font, 1000.0);
        assert_eq!(wrapped.lines.len(), 1);
        // concatenated display text across the coloured spans is exactly "abc".
        let text: String = wrapped.lines[0]
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(text, "abc");
        // width = 3 glyphs * 10px = 30px (NO inserted spaces).
        assert_eq!(wrapped.lines[0].width, font.measure("abc") as f32);
        // and a REAL source space IS honoured: "a b" measures with the space.
        let exp2 = expand("a §Yb§!", &HashMap::new(), [1.0; 4]);
        let w2 = wrap(&exp2, &font, 1000.0);
        let t2: String = w2.lines[0].spans.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(t2, "a b");
        assert_eq!(w2.lines[0].width, font.measure("a b") as f32);
    }

    #[test]
    fn wrap_uses_glyph_measure_after_expansion_not_raw_len() {
        // The source has colour markup; the wrap MUST be driven by the expanded,
        // markup-stripped glyph width, not the raw source length. "§Y...§!" adds
        // 4 source chars that must NOT count toward the wrap.
        let font = sample_font();
        let mut vars = HashMap::new();
        vars.insert("N".to_string(), "Brandenburg".to_string());
        let exp = expand(
            "§YBrandenburg§! is the capital region here",
            &vars,
            [1.0; 4],
        );
        // each non-space glyph = 10px, space = 6px. width budget 120px.
        let wrapped = wrap(&exp, &font, 120.0);
        // every produced line must actually be <= the budget (proves measurement).
        for l in &wrapped.lines {
            assert!(l.width <= 120.0 + 0.01, "line {:?} exceeds budget", l.width);
        }
        // it wrapped into more than one line (the text is wider than 120px).
        assert!(wrapped.lines.len() >= 2);
        // the FIRST line's measured width equals the BMFont measure of its text —
        // proving the wrap used glyph metrics, not character counts.
        let first_text: String = wrapped.lines[0]
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(wrapped.lines[0].width, font.measure(&first_text) as f32);
        // colour survived: "Brandenburg" stays yellow somewhere in the output.
        let has_yellow = wrapped
            .lines
            .iter()
            .flat_map(|l| &l.spans)
            .any(|s| s.text.contains("Brandenburg") && s.color == [0.93, 0.82, 0.27, 1.0]);
        assert!(has_yellow);
    }

    #[test]
    fn long_word_hard_breaks_so_it_never_overflows() {
        let font = sample_font();
        let exp = expand(
            "Supercalifragilisticexpialidocious",
            &HashMap::new(),
            [1.0; 4],
        );
        let wrapped = wrap(&exp, &font, 100.0);
        assert!(wrapped.lines.len() >= 2);
        for l in &wrapped.lines {
            assert!(l.width <= 100.0 + 0.01);
        }
    }
}
