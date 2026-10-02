//! Text shared by combat windows and ordinary-map hovers. Call sites own native keys and scopes.
use super::{Chunk, INLINE_FLAG, text_icon};
use dashboard::scripted_text::stored_text;
use installed_content::gui::native::fonts::TextColors;
use installed_content::localisation::{Argument as Arg, Localisation, Piece, pieces};

// One declaration supplies the producer's keys and the preload inventory.
macro_rules! tooltip_keys {
    ($($key:ident = $text:literal),* $(,)?) => {
        $(const $key: &str = $text;)*
        pub(super) const TOOLTIP_KEYS: &[&str] = &[$($key),*];
    };
}
pub(crate) use tooltip_keys;

/// CState::display_name (1409CC380): the custom name (+0x38), else the owner
/// name (+0xE8), else the template's. set_state_name (140310B50) stores the
/// localised text of its argument as the custom name; SetOwner (1409D1710,
/// also on load) has 1409D3770 store the localised `<owner original tag or
/// tag>_<template name key>` as the owner name, or clear it when the key is
/// missing. Both are [`stored_text`]: never localised again, nor rebuilt from
/// today's owner.
pub(crate) fn state_name(
    loc: &Localisation,
    recorded: &str,
    id: u32,
    custom: &[u8],
    owner: &[u8],
    template: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    for (bytes, what) in [
        (custom, "the custom name (CState+0x38)"),
        (owner, "the owner name (CState+0xE8)"),
    ] {
        if !bytes.is_empty() {
            return stored_text(loc, recorded, bytes, &format!("state {id}: {what}"));
        }
    }
    template()
}

pub(crate) fn localise(
    loc: &Localisation,
    key: &str,
    args: &[(&str, Arg<'_>)],
) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

/// EnsureTrailingNewlines@1424B6660 adds only missing trailing newlines.
pub(crate) fn ensure_newlines(text: &mut String, count: usize) {
    if !text.is_empty() {
        let have = text.bytes().rev().take_while(|&b| b == b'\n').count();
        for _ in have..count {
            text.push('\n');
        }
    }
}

/// 140BBA780: qualify with the simulation tag before trying the plain key.
pub(crate) fn country_localised(
    loc: &Localisation,
    tag: Result<&str, &str>,
    key: &str,
) -> Result<String, String> {
    let tag = tag.map_err(|reason| {
        format!("{key}'s name needs its country's TAG_{key} key, but {reason}")
    })?;
    let qualified = format!("{tag}_{key}");
    localise(
        loc,
        if loc.get(&qualified).is_some() {
            &qualified
        } else {
            key
        },
        &[],
    )
}

/// 141580360 preserves signed integer limits, including an invalid interval.
pub(crate) fn value_limit(
    loc: &Localisation,
    min: Option<i32>,
    max: Option<i32>,
) -> Result<String, String> {
    let key = match (min, max) {
        (None, None) => return Ok(String::new()),
        (Some(_), None) => "value_limit_min",
        (None, Some(_)) => "value_limit_max",
        (Some(a), Some(b)) if b < a => "value_limit_invalid",
        (Some(a), Some(b)) if a == b => "value_limit_exact",
        _ => "value_limit_min_max",
    };
    let mut args = Vec::new();
    if let Some(n) = min {
        args.push(("MIN", Arg::Int32(n)));
    }
    if let Some(n) = max {
        args.push(("MAX", Arg::Int32(n)));
    }
    localise(loc, key, &args)
}

/// The colour state of one draw of text: CBitmapFont::draw (14228B2F0)
/// empties its colour stack before the text, and each colour code goes
/// through CBitmapFont::ApplyTextColorCode (142287650).
pub(crate) struct Colours {
    /// The font's own colour (+0x80), which `!` sets on an unnested stack.
    base: [f32; 4],
    current: [f32; 4],
    stack: Vec<[f32; 4]>,
}

impl Colours {
    pub(crate) fn new(base: [f32; 4]) -> Self {
        Self {
            base,
            current: base,
            stack: Vec::new(),
        }
    }

    /// 142287650: `!` over more than one pushed colour pops back to the
    /// previous one. Otherwise the code's colour (`!`: the font's) becomes
    /// current and is pushed; a code with no colour is logged and changes
    /// nothing.
    fn apply(&mut self, code: char, table: &TextColors) {
        if code == '!' && self.stack.len() > 1 {
            self.stack.pop();
            self.current = self.stack[self.stack.len() - 1];
            return;
        }
        let colour = if code == '!' {
            self.base
        } else if let Some(colour) = u8::try_from(code).ok().and_then(|code| table.get(&code)) {
            *colour
        } else {
            return;
        };
        self.current = colour;
        self.stack.push(colour);
    }
}

/// One line of rich text as the game's text layout reads it (1422894D0,
/// through [`pieces`]): colour codes through `colours`, text icons and the
/// viewer's resolved inline flags ([`INLINE_FLAG`]).
pub(crate) fn chunks(assets: &super::Assets, raw: &str, colours: &mut Colours) -> Vec<Chunk> {
    chunk_runs(raw, &assets.text_colors, colours, |name| {
        text_icon(assets, name).map(|(sprite, frame)| (Box::new(sprite.into_owned()), frame))
    })
}

fn chunk_runs(
    raw: &str,
    colors: &TextColors,
    colours: &mut Colours,
    mut icon: impl FnMut(&str) -> Option<(Box<super::Sprite>, i64)>,
) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut run = String::new();
    let flush = |run: &mut String, chunks: &mut Vec<Chunk>, color: [f32; 4]| {
        if !run.is_empty() {
            chunks.push(Chunk::Text(std::mem::take(run), color));
        }
    };
    for (index, mut segment) in raw.split(INLINE_FLAG).enumerate() {
        if index > 0 {
            let end = segment
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(segment.len());
            flush(&mut run, &mut chunks, colours.current);
            chunks.push(Chunk::Flag(segment[..end].to_owned()));
            segment = &segment[end..];
        }
        for piece in pieces(segment) {
            match piece {
                Piece::Text(text) => run.push_str(text),
                Piece::Colour(code) => {
                    flush(&mut run, &mut chunks, colours.current);
                    colours.apply(code, colors);
                }
                Piece::Icon(name) => {
                    flush(&mut run, &mut chunks, colours.current);
                    // A frame outside the sprite is refused when bound
                    // (binding::displayable).
                    if let Some((sprite, frame)) = icon(name)
                        && let Ok(frame) = u32::try_from(frame)
                    {
                        chunks.push(Chunk::Icon(sprite, frame));
                    }
                }
                // Refused when bound (binding::displayable).
                Piece::Flag(_) | Piece::Control14 | Piece::Control1A(_) => (),
            }
        }
    }
    flush(&mut run, &mut chunks, colours.current);
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_state_names_precede_the_template_in_the_recording_language_only() {
        let mut loc = Localisation::default();
        loc.language = "english".into();
        let name = |custom: &[u8], owner: &[u8], recorded| {
            state_name(&loc, recorded, 7, custom, owner, || Ok("template".into()))
        };
        // The stored bytes are drawn as they are, never localised again.
        assert_eq!(
            name(b"$literal$", b"owner", "english").as_deref(),
            Ok("$literal$")
        );
        assert_eq!(name(b"", b"owner", "english").as_deref(), Ok("owner"));
        assert_eq!(name(b"", b"", "english").as_deref(), Ok("template"));
        assert_eq!(name(b"", b"", "german").as_deref(), Ok("template"));
        assert_eq!(
            name(b"", b"Bohmen", "german"),
            Err("state 7: the owner name (CState+0xE8) is text the game stored in german without its key, and the viewer's language is english".into())
        );
        assert!(
            name(b"Sudetenland", b"", "german")
                .unwrap_err()
                .contains("custom name (CState+0x38)")
        );
        assert_eq!(
            name(&[255], b"owner", "english"),
            Err("state 7: the custom name (CState+0x38) is not UTF-8".into())
        );
    }

    /// 142287650 pops to the previous colour, and `!` on a stack of one
    /// pushes the font's own colour, so the next `!` returns to the first.
    #[test]
    fn colour_codes_nest_as_the_font_draws_them() {
        let colors = [(b'Y', [1., 1., 0., 1.]), (b'R', [1., 0., 0., 1.])]
            .into_iter()
            .collect();
        let (base, yellow, red) = ([1.; 4], [1., 1., 0., 1.], [1., 0., 0., 1.]);
        let runs = |text: &str| {
            let mut colours = Colours::new(base);
            chunk_runs(text, &colors, &mut colours, |_| None)
                .into_iter()
                .map(|chunk| match chunk {
                    Chunk::Text(text, colour) => (text, colour),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
        };
        let run = |text: &str, colour| (text.to_owned(), colour);
        assert_eq!(
            runs("a\u{11}Yb\u{11}Rc\u{11}!d\u{11}!e"),
            [
                run("a", base),
                run("b", yellow),
                run("c", red),
                run("d", yellow),
                run("e", base)
            ]
        );
        assert_eq!(
            runs("\u{11}Ya\u{11}!b\u{11}!c\u{11}Zd"),
            [
                run("a", yellow),
                run("b", base),
                run("c", yellow),
                run("d", yellow)
            ]
        );
    }

    #[test]
    fn shared_chunks_keep_literal_markup_colour_across_lines_and_resolved_flags() {
        let loc: Localisation = [(
            "line".into(),
            "§Ycoloured\ncontinued§! §§Y ££icon @@tag".into(),
        )]
        .into_iter()
        .collect();
        let raw = localise(&loc, "line", &[]).unwrap();
        let colors = [(b'Y', [1., 1., 0., 1.])].into_iter().collect();
        let base = [1.; 4];
        let yellow = [1., 1., 0., 1.];
        let mut color = Colours::new(base);
        let mut lines = raw.split('\n');
        let first = chunk_runs(lines.next().unwrap(), &colors, &mut color, |_| None);
        assert!(matches!(&first[..], [Chunk::Text(s, c)] if s == "coloured" && *c == yellow));
        let last = chunk_runs(lines.next().unwrap(), &colors, &mut color, |_| None);
        assert!(matches!(&last[..], [Chunk::Text(s, c), Chunk::Text(t, b)]
            if s == "continued" && *c == yellow && t == " §Y £icon @tag" && *b == base));
        let flags = chunk_runs(
            &format!("{INLINE_FLAG}SYN_test tail"),
            &colors,
            &mut color,
            |_| None,
        );
        assert!(
            matches!(&flags[..], [Chunk::Flag(key), Chunk::Text(t, _)] if key == "SYN_test" && t == " tail")
        );
        assert!(
            super::super::binding::validate(&super::super::binding::NoIcons, "\u{12}SYN_test")
                .unwrap_err()
                .contains("inline flag")
        );
    }
}
