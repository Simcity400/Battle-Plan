//! Text shared by combat windows and ordinary-map hovers. Call sites own native keys and scopes.
use dashboard_text::scripted_text::stored_text;
use installed_content_localisation::{
    localisation::Argument, localisation::Argument as Arg, localisation::Localisation,
    localisation::Piece, localisation::pieces,
};
use replay_data::rows::CountryRow;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tooltip {
    pub title: String,
    pub body: String,
    /// Line name and why its input/text cannot be reproduced; no guessed
    /// text stands in for it. Counts are occurrences, not unique reasons.
    pub refused: BTreeMap<String, usize>,
}

impl Tooltip {
    pub fn refuse(&mut self, line: &str, reason: String) {
        *self.refused.entry(format!("{line}: {reason}")).or_default() += 1;
    }
    pub fn line(&mut self, body: bool, gap: usize, name: &str, value: Result<String, String>) {
        match value {
            Ok(value) => {
                let out = if body {
                    &mut self.body
                } else {
                    &mut self.title
                };
                if !value.is_empty() {
                    ensure_newlines(out, gap);
                    out.push_str(&value);
                }
            }
            Err(reason) => self.refuse(name, reason),
        }
    }
}

/// The sprite frame count and the selected frame of a text icon, or `None`
/// for an icon the content lacks.
pub trait TextIcons {
    fn icon(&self, name: &str) -> Option<(u32, i64)>;
}
/// Content with no text icons, for the text builders' tests.
#[cfg(any(test, feature = "test-support"))]
pub struct NoIcons;
#[cfg(any(test, feature = "test-support"))]
impl TextIcons for NoIcons {
    fn icon(&self, _: &str) -> Option<(u32, i64)> {
        None
    }
}
/// Text drawn by the combat walk: glyphs, colour codes and text icons
/// (`native_text`). An authored inline flag, the 0x14 and 0x1A controls and an icon frame
/// outside its sprite are refused by name. An icon the content lacks is no
/// refusal: the game draws nothing for it.
pub fn displayable(icons: &(impl TextIcons + ?Sized), text: String) -> Result<String, String> {
    validate(icons, &text)?;
    Ok(text)
}

pub fn validate(icons: &(impl TextIcons + ?Sized), text: &str) -> Result<(), String> {
    validate_inner(icons, text, false)
}

/// Carrier country text is painted with the hour's native flag source in
/// the host tooltip draw. Other text bindings retain their flag refusal.
pub fn displayable_flags(
    icons: &(impl TextIcons + ?Sized),
    text: String,
) -> Result<String, String> {
    validate_inner(icons, &text, true)?;
    Ok(text)
}

fn validate_inner(
    icons: &(impl TextIcons + ?Sized),
    text: &str,
    flags: bool,
) -> Result<(), String> {
    for piece in pieces(text) {
        match piece {
            Piece::Text(_) | Piece::Colour(_) => (),
            Piece::Icon(name) => {
                if let Some((frames, frame)) = icons.icon(name)
                    && !(0..i64::from(frames)).contains(&frame)
                {
                    return Err(format!(
                        "{}, which selects frame index {frame} of a {frames}-frame sprite, outside it",
                        piece.describe()
                    ));
                }
            }
            Piece::Flag(_) if flags => (),
            Piece::Flag(_) | Piece::Control14 | Piece::Control1A(_) => {
                return Err(format!(
                    "{}, which the viewer does not draw",
                    piece.describe()
                ));
            }
        }
    }
    Ok(())
}
/// A key the executable names, through the game's localiser with
/// its arguments. A key the content lacks
/// is drawn as its own bytes; output the localiser or the native text layout
/// cannot produce as the game does is refused by name.
pub fn builtin(
    icons: &(impl TextIcons + ?Sized),
    localisation: &Localisation,
    key: &str,
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    let text = localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|refusal| refusal.to_string())?;
    displayable(icons, text).map_err(|reason| format!("localisation {key} holds {reason}"))
}
/// A recorded localisation token through the game's localiser with no
/// arguments: a key the content lacks is drawn as its own bytes,
/// an empty one as nothing.
pub fn token(
    icons: &(impl TextIcons + ?Sized),
    localisation: &Localisation,
    token: &[u8],
) -> Result<String, String> {
    token_with(icons, localisation, token, &[])
}
/// [`token`] with the call site's native arguments, for a content key such
/// as a modifier name with its `$ARG$`.
pub fn token_with(
    icons: &(impl TextIcons + ?Sized),
    localisation: &Localisation,
    token: &[u8],
    arguments: &[(&str, Argument<'_>)],
) -> Result<String, String> {
    let text = localisation
        .localise_with(token, arguments)
        .map_err(|refusal| refusal.to_string())?;
    displayable(icons, text).map_err(|reason| {
        format!(
            "localisation {} holds {reason}",
            String::from_utf8_lossy(installed_content_core::parser::c_string(token))
        )
    })
}
// Native keys used by the tooltip producers.
#[macro_export]
macro_rules! tooltip_keys {
    ($($key:ident = $text:literal),* $(,)?) => {
        $(const $key: &str = $text;)*
    };
}
pub use crate::tooltip_keys;

/// State display naming uses the stored custom name, else owner name, else
/// template name. A name change stores localised text. Owner changes, including
/// load, store the owner-tag-qualified template name or clear it if absent.
/// Both are [`stored_text`], never localised again or rebuilt from the current owner.
pub fn state_name(
    loc: &Localisation,
    recorded: &str,
    id: u32,
    custom: &[u8],
    owner: &[u8],
    template: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    for (bytes, what) in [(custom, "the custom name"), (owner, "the owner name")] {
        if !bytes.is_empty() {
            return stored_text(loc, recorded, bytes, &format!("state {id}: {what}"));
        }
    }
    template()
}

pub fn localise(loc: &Localisation, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    loc.localise_with(key.as_bytes(), args)
        .map_err(|e| e.to_string())
}

/// trailing-newline padding adds only missing trailing newlines.
pub fn ensure_newlines(text: &mut String, count: usize) {
    if !text.is_empty() {
        let have = text.bytes().rev().take_while(|&b| b == b'\n').count();
        for _ in have..count {
            text.push('\n');
        }
    }
}

/// qualify with the simulation tag before trying the plain key.
pub fn country_localised(
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

/// The raw simulation tag for the viewing canonical slot. Combat text uses
/// this tag for viewing-country localisation, never a cosmetic alias.
pub fn viewing_tag<'a>(
    countries: &'a [CountryRow],
    vision: &crate::map::perspective::Vision,
) -> Result<&'a str, &'static str> {
    countries
        .iter()
        .find(|country| country.canonical_slot == vision.ours())
        .ok_or("the viewing country has no Countries row")
        .and_then(|country| {
            std::str::from_utf8(&country.tag).map_err(|_| "the viewing country's tag is not UTF-8")
        })
}

/// the game preserves signed integer limits, including an invalid interval.
pub fn value_limit(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refused_lines_are_named_and_counted_without_hiding_other_lines() {
        let mut tooltip = Tooltip::default();
        tooltip.line(false, 2, "one", Ok("first\n".into()));
        for _ in 0..2 {
            tooltip.line(false, 2, "two", Err("absent operand".into()));
        }
        tooltip.line(false, 2, "three", Ok("last".into()));
        assert_eq!(tooltip.title, "first\n\nlast");
        assert_eq!(tooltip.refused["two: absent operand"], 2);
    }

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
            Err("state 7: the owner name is text the game stored in german without its key, and the viewer's language is english".into())
        );
        assert!(
            name(b"Sudetenland", b"", "german")
                .unwrap_err()
                .contains("custom name")
        );
        assert_eq!(
            name(&[255], b"owner", "english"),
            Err("state 7: the custom name is not UTF-8".into())
        );
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    struct Icons;
    impl TextIcons for Icons {
        fn icon(&self, name: &str) -> Option<(u32, i64)> {
            name.starts_with("strip").then(|| {
                (
                    3,
                    i64::from(crate::map::native_text::icon_lookup(name).1.unwrap_or(0)),
                )
            })
        }
    }

    #[test]
    fn carrier_flags_facade_preserves_other_text_validation() {
        let flag = "\u{12}SYN\u{11}HCountry\u{11}!:\n".to_owned();
        assert!(
            displayable(&Icons, flag.clone())
                .unwrap_err()
                .contains("inline flag")
        );
        assert_eq!(displayable_flags(&Icons, flag.clone()), Ok(flag));
        for text in ["\u{13}strip|4 ", "\u{14}", "\u{1a}x"] {
            assert_eq!(
                displayable_flags(&Icons, text.into()),
                displayable(&Icons, text.into()),
            );
            assert!(displayable_flags(&Icons, text.into()).is_err());
        }
        assert_eq!(
            displayable_flags(&Icons, "\u{13}missing ".into()),
            Ok("\u{13}missing ".into())
        );
    }

    /// Keys go through the game's localiser as the content stores them: a
    /// missing key, whether the executable names it or it is a recorded
    /// token, is drawn as its bytes; unsupported controls are refused by name.
    #[test]
    fn localised_text_is_native_and_refused_by_name() {
        // Authored text, stored as the loader stores it.
        let localisation: Localisation = [
            ("BATTLE_ATTACK", "§YAngriff§! $NESTED$"),
            ("NESTED", "[x] 100§§"),
            ("SKILL", "$SKILL|Y$"),
            ("FLAG", "@GER"),
            ("ICON", "£strip|3 £strip|4 £missing"),
            ("SPACE", "a¤b"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        assert_eq!(
            builtin(&Icons, &localisation, "BATTLE_ATTACK", &[]).as_deref(),
            Ok("\u{11}YAngriff\u{11}! [x] 100§")
        );
        assert_eq!(
            builtin(
                &Icons,
                &localisation,
                "SKILL",
                &[("SKILL", Argument::Int32(4))]
            )
            .as_deref(),
            Ok("\u{11}Y4\u{11}!")
        );
        assert_eq!(
            builtin(&Icons, &localisation, "BUTTON_DISMISS", &[]).as_deref(),
            Ok("BUTTON_DISMISS")
        );
        assert_eq!(
            token(&Icons, &localisation, b"BUTTON_DISMISS\0x").as_deref(),
            Ok("BUTTON_DISMISS")
        );
        assert_eq!(token(&Icons, &localisation, b"").as_deref(), Ok(""));
        assert_eq!(
            token(&Icons, &localisation, b"FLAG"),
            Err("localisation FLAG holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw".into())
        );
        assert_eq!(
            token(&Icons, &localisation, b"ICON"),
            Err("localisation ICON holds the icon 0x13 \"strip|4\" (an authored `£`), which selects frame index 3 of a 3-frame sprite, outside it".into())
        );
        assert!(
            token(&Icons, &localisation, b"SPACE")
                .unwrap_err()
                .contains("0x14")
        );
    }
}

/// country-specific short-name localisation checks TAG_key_short, key_short,
/// then country-specific localisation for the ordinary key.
pub fn wing_short_name(loc: &Localisation, tag: &str, key: &str) -> Result<String, String> {
    let short = format!("{key}_short");
    let qualified = format!("{tag}_{short}");
    let selected = if loc.get(&qualified).is_some() {
        qualified
    } else if loc.get(&short).is_some() {
        short
    } else {
        return country_localised(loc, Ok(tag), key);
    };
    localise(loc, &selected, &[])
}

#[cfg(test)]
mod wing_name_tests {
    use super::*;
    #[test]
    fn wing_short_name_prefers_country_then_generic_then_ordinary_key() {
        let fixture = |country, generic| {
            let mut entries = vec![("fighter", "Ordinary")];
            if country {
                entries.push(("TAG_fighter_short", "Country short"));
            }
            if generic {
                entries.push(("fighter_short", "Generic short"));
            }
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect::<Localisation>()
        };
        assert_eq!(
            wing_short_name(&fixture(true, true), "TAG", "fighter").unwrap(),
            "Country short"
        );
        assert_eq!(
            wing_short_name(&fixture(false, true), "TAG", "fighter").unwrap(),
            "Generic short"
        );
        assert_eq!(
            wing_short_name(&fixture(false, false), "TAG", "fighter").unwrap(),
            "Ordinary"
        );
    }
}
